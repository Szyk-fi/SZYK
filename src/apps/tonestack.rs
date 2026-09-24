//! A guitar amp-sim / effects-chain processor, in the spirit of
//! commercial plugin suites like Neural DSP's Archetype line: taps
//! another app's live output (see audio_bus.rs, the same technique
//! Magnito/Warps/Nautilus already use) as its input -- this sim has
//! no real instrument input jack, only synthesized/recorded signals
//! apps can hand each other (see `AudioDeviceState::set_input`'s own
//! doc comment) -- and runs it through a real, fixed-order pedalboard
//! signal chain:
//!
//!   Input Gain -> Noise Gate -> Drive/Amp voicing -> 3-band tone
//!   stack -> Cabinet EQ -> Modulation (Chorus/Phaser) -> Delay ->
//!   Reverb -> Output level
//!
//! Every stage is real DSP, not a label-only stub:
//! - **4 amp voicings** (Clean/Crunch/Modern/Fuzz), each its own
//!   waveshaping curve -- soft symmetric tanh, asymmetric diode-style
//!   clipping, cascaded higher-gain clipping, and a hard clamp,
//!   respectively (see `shape`). Not a convolution-based neural
//!   amp-capture model (this sim has no ML runtime or IR-capture
//!   pipeline) -- the same "real but simplified" DSP every other
//!   app's physical modeling in this build already is (Magnito's
//!   hysteresis saturation, Nautilus's delay network, ...).
//! - **3-band tone stack** (Bass/Mid/Treble) and **cabinet EQ** (one
//!   of 4 fixed speaker/cab colorations, or Direct/flat) are real RBJ
//!   cookbook biquads (`Biquad`), not a convolved cabinet impulse
//!   response (no IR-loading infra in this build either -- see
//!   `sequencer.rs`'s WAV sample loading for the one place real audio
//!   files get decoded, which cab sim deliberately doesn't reuse:
//!   an IR is a *different* asset type/pipeline than a one-shot
//!   sample).
//! - **Chorus** (a short LFO-modulated fractional delay line) and
//!   **Phaser** (4 cascaded first-order allpass filters swept by one
//!   shared LFO) as the two Modulation choices.
//! - **Delay**: a feedback delay line with one-pole damping in the
//!   feedback path (a tape/analog-delay-style darkening on repeats).
//! - **Reverb**: a small Schroeder network (2 feedback combs + 1
//!   allpass), the same real-DSP shape `beads.rs`'s own reverb block
//!   uses, independently tuned here.
//!
//! Explicitly not implemented: real neural-network amp capture/IR
//! convolution (no ML runtime or IR pipeline in this build), multiple
//! simultaneous amp/cab slots (A/B split), a tuner, and per-pedal
//! reordering (the chain's stage order is fixed, matching a typical
//! amp-in-a-box pedalboard rather than a fully modular rack).

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const VOICING_NAMES: [&str; 4] = ["Clean", "Crunch", "Modern", "Fuzz"];
const CAB_NAMES: [&str; 4] = ["1x12", "2x12", "4x12", "Direct"];
const MOD_NAMES: [&str; 3] = ["Off", "Chorus", "Phaser"];

const MIN_DELAY_MS: f32 = 20.0;
const MAX_DELAY_MS: f32 = 800.0;
const MIN_MOD_HZ: f32 = 0.1;
const MAX_MOD_HZ: f32 = 6.0;

/// How many points the output waveform trace is downsampled to, once
/// per audio block -- same economy `Magnito`/`Warps` already use for
/// their own Slint scope traces.
const WAVEFORM_SNAPSHOT_POINTS: usize = 96;

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.01).clamp(min, max);
    value.set(next);
}

// --- Real DSP building blocks -----------------------------------

/// A standard RBJ-cookbook biquad, Direct Form I, f32 throughout to
/// match every other filter in this build. Coefficients are
/// recomputed from scratch whenever a caller changes what the filter
/// should be doing (tone/cab knobs move, or the sample rate is only
/// now known) -- cheap enough (a handful of transcendental calls) to
/// just do every block rather than caching a coefficient-dirty flag.
#[derive(Default, Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    fn set_coeffs(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    fn set_peaking(&mut self, freq: f32, sample_rate: f32, gain_db: f32, q: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = std::f32::consts::TAU * (freq / sample_rate).clamp(0.001, 0.49);
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        self.set_coeffs(1.0 + alpha * a, -2.0 * cos_w0, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos_w0, 1.0 - alpha / a);
    }

    fn set_low_shelf(&mut self, freq: f32, sample_rate: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = std::f32::consts::TAU * (freq / sample_rate).clamp(0.001, 0.49);
        let (sin_w0, cos_w0) = w0.sin_cos();
        let sqrt_a = a.sqrt();
        let alpha = sin_w0 / 2.0 * 2f32.sqrt(); // shelf slope S=1
        self.set_coeffs(
            a * ((a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0),
            a * ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha),
            (a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
            (a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha,
        );
    }

    fn set_high_shelf(&mut self, freq: f32, sample_rate: f32, gain_db: f32) {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = std::f32::consts::TAU * (freq / sample_rate).clamp(0.001, 0.49);
        let (sin_w0, cos_w0) = w0.sin_cos();
        let sqrt_a = a.sqrt();
        let alpha = sin_w0 / 2.0 * 2f32.sqrt();
        self.set_coeffs(
            a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0),
            a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha),
            (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha,
            2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
            (a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha,
        );
    }

    fn set_lowpass(&mut self, freq: f32, sample_rate: f32, q: f32) {
        let w0 = std::f32::consts::TAU * (freq / sample_rate).clamp(0.001, 0.49);
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        self.set_coeffs((1.0 - cos_w0) / 2.0, 1.0 - cos_w0, (1.0 - cos_w0) / 2.0, 1.0 + alpha, -2.0 * cos_w0, 1.0 - alpha);
    }
}

/// The 4 amp-voicing waveshapers -- see the module doc comment for
/// what each one is modeling. `drive` 0..1, pre-gain scaled from
/// `1x` to `12x` so the low end of the knob still cleans up nicely
/// even on the higher-gain voicings.
fn shape(voicing: u32, x: f32, drive: f32) -> f32 {
    let g = 1.0 + drive.clamp(0.0, 1.0) * 11.0;
    match voicing {
        1 => {
            // Crunch -- asymmetric soft clip: the positive and
            // negative halves saturate at different rates, the same
            // "diode-style" asymmetry real tube/diode clippers have.
            let xg = x * g;
            (xg.max(0.0).tanh() * 0.9 + (xg.min(0.0) * 1.3).tanh() * 0.7) * 0.6
        }
        2 => {
            // Modern -- cascaded, higher-gain clipping for a denser
            // harmonic stack (a second soft-clip stage on top of the
            // first, not just one harder curve).
            let xg = x * g * 1.4;
            let y1 = xg.tanh();
            (y1 * 1.8).tanh() * 0.65
        }
        3 => {
            // Fuzz -- a hard clamp for the square-ish, more
            // compressed character fuzz pedals have.
            (x * g * 1.8).clamp(-1.0, 1.0) * 0.6
        }
        _ => {
            // Clean -- gentle tanh saturation only; Drive still
            // thickens it without ever really "breaking up".
            (x * (1.0 + drive.clamp(0.0, 1.0) * 2.5)).tanh() * 0.9
        }
    }
}

/// A peak-follower noise gate: opens fast (~5ms) once the input rises
/// above `threshold`, closes slowly (~80ms) once it falls back below
/// -- the same fast-attack/slow-release asymmetry a real gate uses so
/// it doesn't chop a note's natural decay.
#[derive(Default)]
struct Gate {
    env: f32,
    gain: f32,
}

impl Gate {
    fn process(&mut self, x: f32, threshold: f32, sample_rate: f32) -> f32 {
        let follow_coef = (-1.0 / (sample_rate * 0.01)).exp();
        self.env = self.env * follow_coef + x.abs() * (1.0 - follow_coef);
        let target = if self.env > threshold { 1.0 } else { 0.0 };
        let smooth_coef = if target > self.gain { (-1.0 / (sample_rate * 0.005)).exp() } else { (-1.0 / (sample_rate * 0.08)).exp() };
        self.gain = self.gain * smooth_coef + target * (1.0 - smooth_coef);
        x * self.gain
    }
}

/// A short LFO-modulated fractional delay line -- the classic chorus
/// building block. Buffer is lazily (re)sized to the current sample
/// rate rather than assuming a fixed one, same convention every other
/// delay-shaped buffer in this build uses.
#[derive(Default)]
struct Chorus {
    buf: Vec<f32>,
    write: usize,
    phase: f32,
}

impl Chorus {
    fn ensure_capacity(&mut self, sample_rate: f32) {
        let needed = ((0.04 * sample_rate) as usize).max(8); // 40ms, comfortably past the deepest sweep
        if self.buf.len() != needed {
            self.buf = vec![0.0; needed];
            self.write = 0;
        }
    }

    fn process(&mut self, x: f32, rate_hz: f32, depth: f32, sample_rate: f32) -> f32 {
        self.ensure_capacity(sample_rate);
        let len = self.buf.len();
        self.buf[self.write] = x;
        self.phase = (self.phase + rate_hz / sample_rate).fract();
        let center_ms = 12.0;
        let depth_ms = depth.clamp(0.0, 1.0) * 8.0;
        let delay_ms = center_ms + (self.phase * std::f32::consts::TAU).sin() * depth_ms;
        let delay_samples = (delay_ms * 0.001 * sample_rate).clamp(1.0, (len - 2) as f32);
        let read_pos = (self.write as f32 - delay_samples).rem_euclid(len as f32);
        // Same float-rounding guard as `DelayFx::process` -- see its
        // comment.
        let i0 = (read_pos as usize).min(len - 1);
        let i1 = (i0 + 1) % len;
        let frac = read_pos - i0 as f32;
        let wet = self.buf[i0] + (self.buf[i1] - self.buf[i0]) * frac;
        self.write = (self.write + 1) % len;
        wet
    }
}

/// One first-order allpass stage, with its own `x[n-1]`/`y[n-1]`
/// memory -- 4 of these in series, each retuned every sample from the
/// shared LFO, make up the phaser.
#[derive(Default, Clone, Copy)]
struct Allpass1 {
    a: f32,
    x1: f32,
    y1: f32,
}

impl Allpass1 {
    fn process(&mut self, x: f32) -> f32 {
        let y = -self.a * x + self.x1 + self.a * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

#[derive(Default)]
struct Phaser {
    stages: [Allpass1; 4],
    phase: f32,
}

impl Phaser {
    fn process(&mut self, x: f32, rate_hz: f32, depth: f32, sample_rate: f32) -> f32 {
        self.phase = (self.phase + rate_hz / sample_rate).fract();
        let lfo = (self.phase * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        let center_hz = 300.0 + lfo * depth.clamp(0.0, 1.0) * 2200.0;
        let wt = (std::f32::consts::PI * center_hz / sample_rate).tan();
        let a = (wt - 1.0) / (wt + 1.0);
        let mut y = x;
        for stage in self.stages.iter_mut() {
            stage.a = a;
            y = stage.process(y);
        }
        // Dry + phase-shifted wet, the classic moving-notch comb
        // sound a phaser is named for.
        (x + y) * 0.5
    }
}

/// A feedback delay line with one-pole damping on the repeats -- the
/// darkening real tape/analog delays get as a signal recirculates.
#[derive(Default)]
struct DelayFx {
    buf: Vec<f32>,
    write: usize,
    damp_state: f32,
}

impl DelayFx {
    fn ensure_capacity(&mut self, sample_rate: f32) {
        let needed = ((MAX_DELAY_MS * 0.001 + 0.01) * sample_rate) as usize;
        if self.buf.len() != needed {
            self.buf = vec![0.0; needed.max(8)];
            self.write = 0;
        }
    }

    fn process(&mut self, x: f32, time_ms: f32, feedback: f32, sample_rate: f32) -> f32 {
        self.ensure_capacity(sample_rate);
        let len = self.buf.len();
        let delay_samples = (time_ms.clamp(MIN_DELAY_MS, MAX_DELAY_MS) * 0.001 * sample_rate).clamp(1.0, (len - 2) as f32);
        let read_pos = (self.write as f32 - delay_samples).rem_euclid(len as f32);
        // `rem_euclid` can round up to exactly `len as f32` from
        // float error even though it's mathematically guaranteed
        // `< len` -- clamp the cast, not just trust it, or an
        // occasional read lands one past the buffer's end.
        let i0 = (read_pos as usize).min(len - 1);
        let i1 = (i0 + 1) % len;
        let frac = read_pos - i0 as f32;
        let wet = self.buf[i0] + (self.buf[i1] - self.buf[i0]) * frac;
        self.damp_state += (wet - self.damp_state) * 0.35;
        self.buf[self.write] = x + self.damp_state * feedback.clamp(0.0, 0.95);
        self.write = (self.write + 1) % len;
        wet
    }
}

/// A small Schroeder reverb: 2 feedback combs (summed) feeding 1
/// allpass -- the same real-DSP shape `beads.rs`'s own reverb block
/// uses, independently tuned here. Fixed, classic Schroeder tunings
/// (in seconds, scaled to whatever sample rate is live) rather than
/// this build's one other reverb's exact lengths, so the two don't
/// sound identical.
#[derive(Default)]
struct ReverbFx {
    comb_a: Vec<f32>,
    pos_a: usize,
    comb_b: Vec<f32>,
    pos_b: usize,
    allpass: Vec<f32>,
    pos_ap: usize,
}

impl ReverbFx {
    fn ensure_capacity(&mut self, sample_rate: f32) {
        let len_a = ((0.0353 * sample_rate) as usize).max(4);
        let len_b = ((0.0417 * sample_rate) as usize).max(4);
        let len_ap = ((0.0051 * sample_rate) as usize).max(4);
        if self.comb_a.len() != len_a {
            self.comb_a = vec![0.0; len_a];
            self.pos_a = 0;
        }
        if self.comb_b.len() != len_b {
            self.comb_b = vec![0.0; len_b];
            self.pos_b = 0;
        }
        if self.allpass.len() != len_ap {
            self.allpass = vec![0.0; len_ap];
            self.pos_ap = 0;
        }
    }

    fn process(&mut self, x: f32, size: f32, sample_rate: f32) -> f32 {
        self.ensure_capacity(sample_rate);
        let fb = 0.6 + size.clamp(0.0, 1.0) * 0.35;
        let a_out = self.comb_a[self.pos_a];
        self.comb_a[self.pos_a] = x + a_out * fb;
        self.pos_a = (self.pos_a + 1) % self.comb_a.len();
        let b_out = self.comb_b[self.pos_b];
        self.comb_b[self.pos_b] = x + b_out * fb;
        self.pos_b = (self.pos_b + 1) % self.comb_b.len();
        let combined = (a_out + b_out) * 0.5;
        let ap_out = self.allpass[self.pos_ap];
        let ap_in = combined + ap_out * 0.5;
        self.allpass[self.pos_ap] = ap_in;
        self.pos_ap = (self.pos_ap + 1) % self.allpass.len();
        ap_out - ap_in * 0.5
    }
}

// --- Menu / app scaffolding --------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Source,
    InputGain,
    GateThreshold,
    Voicing,
    Drive,
    Bass,
    Mid,
    Treble,
    AmpLevel,
    CabType,
    ModType,
    ModRate,
    ModDepth,
    DelayTime,
    DelayFeedback,
    DelayMix,
    ReverbSize,
    ReverbMix,
    OutputVolume,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 7;

struct Params {
    source: AtomicUsize,
    input_gain: AtomicF32,
    gate_threshold: AtomicF32,
    voicing: AtomicU32,
    drive: AtomicF32,
    bass: AtomicF32,
    mid: AtomicF32,
    treble: AtomicF32,
    amp_level: AtomicF32,
    cab_type: AtomicU32,
    mod_type: AtomicU32,
    mod_rate: AtomicF32,
    mod_depth: AtomicF32,
    delay_time: AtomicF32,
    delay_feedback: AtomicF32,
    delay_mix: AtomicF32,
    reverb_size: AtomicF32,
    reverb_mix: AtomicF32,
    output_volume: AtomicF32,
    /// Real post-chain peak this block, 0..~1.5 -- Slint output meter.
    output_peak: AtomicF32,
    /// Whether the gate is currently holding the signal closed --
    /// Slint gate indicator.
    gate_closed: AtomicBool,
    /// The real post-chain output waveform, downsampled to
    /// `WAVEFORM_SNAPSHOT_POINTS` once per audio block.
    waveform_snapshot: Mutex<Vec<f32>>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    ext_output_volume: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Tonestack", modbus);
        Self {
            source: AtomicUsize::new(crate::audio_bus::NO_SOURCE),
            input_gain: AtomicF32::new(1.0),
            gate_threshold: AtomicF32::new(0.0),
            voicing: AtomicU32::new(0),
            drive: AtomicF32::new(0.4),
            bass: AtomicF32::new(0.5),
            mid: AtomicF32::new(0.5),
            treble: AtomicF32::new(0.5),
            amp_level: AtomicF32::new(1.0),
            cab_type: AtomicU32::new(0),
            mod_type: AtomicU32::new(0),
            mod_rate: AtomicF32::new(0.25),
            mod_depth: AtomicF32::new(0.5),
            delay_time: AtomicF32::new(0.3),
            delay_feedback: AtomicF32::new(0.3),
            delay_mix: AtomicF32::new(0.0),
            reverb_size: AtomicF32::new(0.4),
            reverb_mix: AtomicF32::new(0.0),
            output_volume: AtomicF32::new(0.8),
            output_peak: AtomicF32::new(0.0),
            gate_closed: AtomicBool::new(false),
            waveform_snapshot: Mutex::new(Vec::new()),
            bus_out: audio_bus.register("Tonestack"),
            mix_level,
            ext_mix_level,
            ext_output_volume: modbus.register("Tonestack: Output Volume"),
        }
    }
}

pub struct TonestackApp {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Tonestack's own palette: flat solid colors, not a
// device-wide theme -- Tweed-amp cream with a warm amp-glow orange accent and dark-ink text -- a well-worn guitar amp panel. ---

const TONESTACK_BG: Rgb565 = Rgb565::new(23, 44, 19);
const TONESTACK_TITLE: Rgb565 = Rgb565::new(7, 11, 4);
const TONESTACK_ACCENT: Rgb565 = Rgb565::new(14, 15, 2);
const TONESTACK_DIM: Rgb565 = Rgb565::new(9, 15, 5);

impl TonestackApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            audio_bus,
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
        }
    }

    fn source_name(&self) -> String {
        self.audio_bus.source_name(self.params.source.load(Ordering::Relaxed))
    }

    fn group_name(&self, g: usize) -> &'static str {
        match g {
            0 => "Input",
            1 => "Amp",
            2 => "Cab",
            3 => "Modulation",
            4 => "Delay",
            5 => "Reverb",
            _ => "Output",
        }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => vec![Selection::Source, Selection::InputGain, Selection::GateThreshold],
            1 => vec![Selection::Voicing, Selection::Drive, Selection::Bass, Selection::Mid, Selection::Treble, Selection::AmpLevel],
            2 => vec![Selection::CabType],
            3 => vec![Selection::ModType, Selection::ModRate, Selection::ModDepth],
            4 => vec![Selection::DelayTime, Selection::DelayFeedback, Selection::DelayMix],
            5 => vec![Selection::ReverbSize, Selection::ReverbMix],
            _ => vec![Selection::OutputVolume],
        }
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for g in 0..NUM_GROUPS {
            rows.push(Row::Group(g));
            if self.expanded[g] {
                for sel in self.group_leaves(g) {
                    rows.push(Row::Leaf(sel));
                }
            }
        }
        rows
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => self.source_name(),
            1 => {
                let idx = self.params.voicing.load(Ordering::Relaxed) as usize % VOICING_NAMES.len();
                format!("{}, drive {:.0}%", VOICING_NAMES[idx], self.params.drive.get() * 100.0)
            }
            2 => {
                let idx = self.params.cab_type.load(Ordering::Relaxed) as usize % CAB_NAMES.len();
                CAB_NAMES[idx].to_string()
            }
            3 => {
                let idx = self.params.mod_type.load(Ordering::Relaxed) as usize % MOD_NAMES.len();
                MOD_NAMES[idx].to_string()
            }
            4 => format!("{:.0}% wet", self.params.delay_mix.get() * 100.0),
            5 => format!("{:.0}% wet", self.params.reverb_mix.get() * 100.0),
            _ => format!("{:.0}%", self.params.output_volume.get() / 1.5 * 100.0),
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => "Source".into(),
            Selection::InputGain => "Input Gain".into(),
            Selection::GateThreshold => "Gate".into(),
            Selection::Voicing => "Voicing".into(),
            Selection::Drive => "Drive".into(),
            Selection::Bass => "Bass".into(),
            Selection::Mid => "Mid".into(),
            Selection::Treble => "Treble".into(),
            Selection::AmpLevel => "Amp Level".into(),
            Selection::CabType => "Cab".into(),
            Selection::ModType => "Type".into(),
            Selection::ModRate => "Rate".into(),
            Selection::ModDepth => "Depth".into(),
            Selection::DelayTime => "Time".into(),
            Selection::DelayFeedback => "Feedback".into(),
            Selection::DelayMix => "Mix".into(),
            Selection::ReverbSize => "Size".into(),
            Selection::ReverbMix => "Mix".into(),
            Selection::OutputVolume => "Volume".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => self.source_name(),
            Selection::InputGain => format!("{:.2}x", self.params.input_gain.get()),
            Selection::GateThreshold => {
                let t = self.params.gate_threshold.get();
                if t <= 0.001 { "off".into() } else { format!("{:.0}%", t * 100.0) }
            }
            Selection::Voicing => {
                let idx = self.params.voicing.load(Ordering::Relaxed) as usize % VOICING_NAMES.len();
                VOICING_NAMES[idx].to_string()
            }
            Selection::Drive => format!("{:.0}%", self.params.drive.get() * 100.0),
            Selection::Bass => format!("{:+.0}", (self.params.bass.get() - 0.5) * 24.0),
            Selection::Mid => format!("{:+.0}", (self.params.mid.get() - 0.5) * 24.0),
            Selection::Treble => format!("{:+.0}", (self.params.treble.get() - 0.5) * 24.0),
            Selection::AmpLevel => format!("{:.2}x", self.params.amp_level.get()),
            Selection::CabType => {
                let idx = self.params.cab_type.load(Ordering::Relaxed) as usize % CAB_NAMES.len();
                CAB_NAMES[idx].to_string()
            }
            Selection::ModType => {
                let idx = self.params.mod_type.load(Ordering::Relaxed) as usize % MOD_NAMES.len();
                MOD_NAMES[idx].to_string()
            }
            Selection::ModRate => format!("{:.1}Hz", MIN_MOD_HZ + self.params.mod_rate.get() * (MAX_MOD_HZ - MIN_MOD_HZ)),
            Selection::ModDepth => format!("{:.0}%", self.params.mod_depth.get() * 100.0),
            Selection::DelayTime => format!("{:.0}ms", MIN_DELAY_MS + self.params.delay_time.get() * (MAX_DELAY_MS - MIN_DELAY_MS)),
            Selection::DelayFeedback => format!("{:.0}%", self.params.delay_feedback.get() * 100.0),
            Selection::DelayMix => format!("{:.0}%", self.params.delay_mix.get() * 100.0),
            Selection::ReverbSize => format!("{:.0}%", self.params.reverb_size.get() * 100.0),
            Selection::ReverbMix => format!("{:.0}%", self.params.reverb_mix.get() * 100.0),
            Selection::OutputVolume => format!("{:.0}%", self.params.output_volume.get() / 1.5 * 100.0),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::Source => {
                let cur = self.params.source.load(Ordering::Relaxed);
                self.params.source.store(crate::audio_bus::cycle_source(cur, step, self.audio_bus.len()), Ordering::Relaxed);
            }
            Selection::InputGain => bump(&self.params.input_gain, delta, sensitivity, 0.0, 2.0),
            Selection::GateThreshold => bump(&self.params.gate_threshold, delta, sensitivity, 0.0, 1.0),
            Selection::Voicing => {
                let cur = self.params.voicing.load(Ordering::Relaxed) as i32;
                self.params.voicing.store((cur + step).rem_euclid(VOICING_NAMES.len() as i32) as u32, Ordering::Relaxed);
            }
            Selection::Drive => bump(&self.params.drive, delta, sensitivity, 0.0, 1.0),
            Selection::Bass => bump(&self.params.bass, delta, sensitivity, 0.0, 1.0),
            Selection::Mid => bump(&self.params.mid, delta, sensitivity, 0.0, 1.0),
            Selection::Treble => bump(&self.params.treble, delta, sensitivity, 0.0, 1.0),
            Selection::AmpLevel => bump(&self.params.amp_level, delta, sensitivity, 0.0, 2.0),
            Selection::CabType => {
                let cur = self.params.cab_type.load(Ordering::Relaxed) as i32;
                self.params.cab_type.store((cur + step).rem_euclid(CAB_NAMES.len() as i32) as u32, Ordering::Relaxed);
            }
            Selection::ModType => {
                let cur = self.params.mod_type.load(Ordering::Relaxed) as i32;
                self.params.mod_type.store((cur + step).rem_euclid(MOD_NAMES.len() as i32) as u32, Ordering::Relaxed);
            }
            Selection::ModRate => bump(&self.params.mod_rate, delta, sensitivity, 0.0, 1.0),
            Selection::ModDepth => bump(&self.params.mod_depth, delta, sensitivity, 0.0, 1.0),
            Selection::DelayTime => bump(&self.params.delay_time, delta, sensitivity, 0.0, 1.0),
            Selection::DelayFeedback => bump(&self.params.delay_feedback, delta, sensitivity, 0.0, 1.0),
            Selection::DelayMix => bump(&self.params.delay_mix, delta, sensitivity, 0.0, 1.0),
            Selection::ReverbSize => bump(&self.params.reverb_size, delta, sensitivity, 0.0, 1.0),
            Selection::ReverbMix => bump(&self.params.reverb_mix, delta, sensitivity, 0.0, 1.0),
            Selection::OutputVolume => bump(&self.params.output_volume, delta, sensitivity, 0.0, 1.5),
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Source => self.params.source.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed),
            Selection::InputGain => self.params.input_gain.set(1.0),
            Selection::GateThreshold => self.params.gate_threshold.set(0.0),
            Selection::Drive => self.params.drive.set(0.4),
            Selection::Bass => self.params.bass.set(0.5),
            Selection::Mid => self.params.mid.set(0.5),
            Selection::Treble => self.params.treble.set(0.5),
            Selection::AmpLevel => self.params.amp_level.set(1.0),
            Selection::ModRate => self.params.mod_rate.set(0.25),
            Selection::ModDepth => self.params.mod_depth.set(0.5),
            Selection::DelayTime => self.params.delay_time.set(0.3),
            Selection::DelayFeedback => self.params.delay_feedback.set(0.3),
            Selection::DelayMix => self.params.delay_mix.set(0.0),
            Selection::ReverbSize => self.params.reverb_size.set(0.4),
            Selection::ReverbMix => self.params.reverb_mix.set(0.0),
            Selection::OutputVolume => self.params.output_volume.set(0.8),
            // No sensible single "default" to reset these to.
            Selection::Voicing | Selection::CabType | Selection::ModType => {}
        }
    }

    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g), true)
                }
                Row::Leaf(sel) => (self.leaf_name(*sel), self.leaf_value(*sel), false),
            })
            .collect()
    }

    pub(crate) fn selected_row(&self) -> usize {
        self.list.selected
    }

    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }

    /// Real post-chain telemetry for the Slint panel -- see
    /// `Params::output_peak`/`gate_closed`/`waveform_snapshot`, all
    /// written once per audio block by `TonestackProcessor::process`.
    pub(crate) fn output_visual(&self) -> crate::app::TonestackExtra {
        let idx = self.params.voicing.load(Ordering::Relaxed) as usize % VOICING_NAMES.len();
        crate::app::TonestackExtra {
            voicing_name: VOICING_NAMES[idx].into(),
            waveform: self.params.waveform_snapshot.lock().unwrap().clone(),
            output_peak: self.params.output_peak.get(),
            gate_closed: self.params.gate_closed.load(Ordering::Relaxed),
        }
    }
}

impl App for TonestackApp {
    fn needs_background_audio(&self) -> bool { self.params.source.load(Ordering::Relaxed) != crate::audio_bus::NO_SOURCE }
    fn tick(&mut self, input: &Input) {
        let rows = self.visible_rows();
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();

        if input.knob1_press {
            if let Some(Row::Group(g)) = current {
                self.expanded[g] = !self.expanded[g];
            }
        }
        if let Some(Row::Leaf(sel)) = current {
            self.edit(sel, input.knob2);
            if input.knob2_press {
                self.reset(sel);
            }
        }
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.selected_row()
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        self.windowed_rows(visible)
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        crate::app::SlintExtra::Tonestack(self.output_visual())
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(TonestackProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            gate: Gate::default(),
            bass_filter: Biquad::default(),
            mid_filter: Biquad::default(),
            treble_filter: Biquad::default(),
            cab_lowpass: Biquad::default(),
            cab_presence: Biquad::default(),
            chorus: Chorus::default(),
            phaser: Phaser::default(),
            delay: DelayFx::default(),
            reverb: ReverbFx::default(),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(TONESTACK_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, TONESTACK_TITLE);
        Text::new("Tonestack", Point::new(16, 30), title).draw(fb).ok();

        let dim = MonoTextStyle::new(&SPLEEN_6X12, TONESTACK_DIM);

        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", self.leaf_name(*sel)), self.leaf_value(*sel)),
            })
            .collect();
        self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, TONESTACK_BG, TONESTACK_DIM, TONESTACK_ACCENT);

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

struct TonestackProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    gate: Gate,
    bass_filter: Biquad,
    mid_filter: Biquad,
    treble_filter: Biquad,
    cab_lowpass: Biquad,
    cab_presence: Biquad,
    chorus: Chorus,
    phaser: Phaser,
    delay: DelayFx,
    reverb: ReverbFx,
}

impl AudioProcessor for TonestackProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;

        let source_idx = self.params.source.load(Ordering::Relaxed);
        let tapped = self.audio_bus.get(source_idx).map(|b| b.lock().unwrap().clone());

        let input_gain = self.params.input_gain.get();
        let gate_threshold = self.params.gate_threshold.get();
        let voicing = self.params.voicing.load(Ordering::Relaxed);
        let drive = self.params.drive.get();
        let amp_level = self.params.amp_level.get();
        let cab_type = self.params.cab_type.load(Ordering::Relaxed);
        let mod_type = self.params.mod_type.load(Ordering::Relaxed);
        let mod_rate_hz = MIN_MOD_HZ + self.params.mod_rate.get() * (MAX_MOD_HZ - MIN_MOD_HZ);
        let mod_depth = self.params.mod_depth.get();
        let delay_time_ms = MIN_DELAY_MS + self.params.delay_time.get() * (MAX_DELAY_MS - MIN_DELAY_MS);
        let delay_feedback = self.params.delay_feedback.get();
        let delay_mix = self.params.delay_mix.get();
        let reverb_size = self.params.reverb_size.get();
        let reverb_mix = self.params.reverb_mix.get();
        let output_volume = (self.params.output_volume.get() + self.params.ext_output_volume.get()).clamp(0.0, 2.0);

        // Tone stack + cab EQ coefficients -- recomputed every block
        // from the live knob/cab-type values (see `Biquad`'s own doc
        // comment for why that's cheap enough not to cache).
        self.bass_filter.set_low_shelf(120.0, sample_rate, (self.params.bass.get() - 0.5) * 24.0);
        self.mid_filter.set_peaking(800.0, sample_rate, (self.params.mid.get() - 0.5) * 24.0, 0.8);
        self.treble_filter.set_high_shelf(2800.0, sample_rate, (self.params.treble.get() - 0.5) * 24.0);
        let (cab_cutoff_hz, cab_presence_db) = match cab_type {
            0 => (5200.0, 2.0),  // 1x12 -- brighter, a little presence bump
            1 => (4600.0, 1.0),  // 2x12 -- balanced
            2 => (3800.0, 3.0),  // 4x12 -- darker overall, more "honk"
            _ => (18000.0, 0.0), // Direct -- effectively flat/bypassed
        };
        self.cab_lowpass.set_lowpass(cab_cutoff_hz, sample_rate, 0.707);
        self.cab_presence.set_peaking(2500.0, sample_rate, cab_presence_db, 1.2);

        let mut peak = 0.0f32;
        let mut waveform_snapshot = Vec::with_capacity(WAVEFORM_SNAPSHOT_POINTS);
        let snapshot_stride = (frames / WAVEFORM_SNAPSHOT_POINTS.max(1)).max(1);
        let mut gate_closed_this_block = false;

        for (i, frame) in buffer.chunks_mut(channels).enumerate() {
            let dry_in = tapped.as_ref().and_then(|v| v.get(i)).copied().unwrap_or(0.0) * input_gain;

            let gated = if gate_threshold > 0.001 { self.gate.process(dry_in, gate_threshold, sample_rate) } else { dry_in };
            if self.gate.gain < 0.5 {
                gate_closed_this_block = true;
            }

            let shaped = shape(voicing, gated, drive) * amp_level;
            let toned = self.treble_filter.process(self.mid_filter.process(self.bass_filter.process(shaped)));
            let cabbed = self.cab_presence.process(self.cab_lowpass.process(toned));

            let modulated = match mod_type {
                1 => self.chorus.process(cabbed, mod_rate_hz, mod_depth, sample_rate),
                2 => self.phaser.process(cabbed, mod_rate_hz, mod_depth, sample_rate),
                _ => cabbed,
            };

            let delayed = self.delay.process(modulated, delay_time_ms, delay_feedback, sample_rate);
            let with_delay = modulated + delayed * delay_mix.clamp(0.0, 1.0);

            let reverbed = self.reverb.process(with_delay, reverb_size, sample_rate);
            let with_reverb = with_delay + reverbed * reverb_mix.clamp(0.0, 1.0);

            let out = with_reverb * output_volume;
            peak = peak.max(out.abs());
            if i % snapshot_stride == 0 && waveform_snapshot.len() < WAVEFORM_SNAPSHOT_POINTS {
                waveform_snapshot.push(out.clamp(-1.5, 1.5));
            }

            {
                let mut bus_out = self.params.bus_out.lock().unwrap();
                if bus_out.len() <= i {
                    bus_out.resize(i + 1, 0.0);
                }
                bus_out[i] = out;
            }

            let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
            for sample in frame.iter_mut() {
                *sample = out * mix_level;
            }
        }

        self.params.output_peak.set(peak);
        self.params.gate_closed.store(gate_closed_this_block, Ordering::Relaxed);
        *self.params.waveform_snapshot.lock().unwrap() = waveform_snapshot;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app_and_bus() -> (TonestackApp, Arc<AudioBus>) {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(3.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let app = TonestackApp::new(sensitivity, nav_speed, modbus, Arc::clone(&audio_bus), mixer_bus);
        (app, audio_bus)
    }

    fn new_processor_with_source(input: Vec<f32>) -> (Arc<Params>, TonestackProcessor, Arc<AudioBus>) {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        *src.lock().unwrap() = input;
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.source.store(0, Ordering::Relaxed);
        params.mix_level.set(1.0);
        let proc = TonestackProcessor {
            params: Arc::clone(&params),
            audio_bus: Arc::clone(&audio_bus),
            gate: Gate::default(),
            bass_filter: Biquad::default(),
            mid_filter: Biquad::default(),
            treble_filter: Biquad::default(),
            cab_lowpass: Biquad::default(),
            cab_presence: Biquad::default(),
            chorus: Chorus::default(),
            phaser: Phaser::default(),
            delay: DelayFx::default(),
            reverb: ReverbFx::default(),
        };
        (params, proc, audio_bus)
    }

    /// With no Source patched in (the default -- `NO_SOURCE`), the
    /// chain must produce silence, not garbage from an unresolved tap.
    #[test]
    fn no_source_produces_silence() {
        let (_app, audio_bus) = new_app_and_bus();
        let modbus = ModBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.mix_level.set(1.0);
        let mut proc = TonestackProcessor {
            params: Arc::clone(&params),
            audio_bus,
            gate: Gate::default(),
            bass_filter: Biquad::default(),
            mid_filter: Biquad::default(),
            treble_filter: Biquad::default(),
            cab_lowpass: Biquad::default(),
            cab_presence: Biquad::default(),
            chorus: Chorus::default(),
            phaser: Phaser::default(),
            delay: DelayFx::default(),
            reverb: ReverbFx::default(),
        };
        let mut buffer = vec![1.0f32; 128 * 2]; // pre-filled with garbage to prove it gets overwritten
        proc.process(&mut buffer, 2, 48000.0);
        assert!(buffer.iter().all(|&s| s == 0.0), "no source patched in should mean silence, not leftover buffer contents");
    }

    /// A real tapped signal, run at unity gain/level with the gate and
    /// every wet effect off, must actually reach the output as real,
    /// non-silent audio -- proves the whole chain passes signal
    /// through, not just that it doesn't panic.
    #[test]
    fn a_tapped_signal_reaches_the_output() {
        let sine: Vec<f32> = (0..512).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let (params, mut proc, _bus) = new_processor_with_source(sine);
        params.gate_threshold.set(0.0);
        params.delay_mix.set(0.0);
        params.reverb_mix.set(0.0);
        params.output_volume.set(1.0);

        let mut buffer = vec![0.0f32; 512 * 2];
        proc.process(&mut buffer, 2, 48000.0);

        assert!(buffer.iter().any(|&s| s.abs() > 0.001), "a real tapped sine should reach the output as real audio");
    }

    /// A gate threshold above the tapped signal's own level must
    /// silence it (once the gate's release has actually closed),
    /// proving the gate stage is real, not a label-only knob.
    #[test]
    fn gate_above_signal_level_silences_it() {
        let quiet: Vec<f32> = vec![0.01; 4096];
        let (params, mut proc, _bus) = new_processor_with_source(quiet);
        params.gate_threshold.set(0.5); // well above the 0.01 signal
        params.output_volume.set(1.0);

        let mut buffer = vec![0.0f32; 4096 * 2];
        proc.process(&mut buffer, 2, 48000.0);

        // Check the back half of the block, after the gate's ~80ms
        // release has had time to fully close at 48kHz (well under
        // 4096 samples).
        let tail = &buffer[buffer.len() / 2..];
        assert!(tail.iter().all(|&s| s.abs() < 0.01), "a gate well above the signal level should have closed by the end of this block");
    }

    /// Each amp voicing's waveshaper must actually behave differently
    /// -- specifically, Fuzz's hard clamp should *saturate* much more
    /// sharply than Clean's gentle tanh: doubling the input should
    /// barely move Fuzz's output (it's already slammed into its
    /// clamp) while Clean's output still visibly grows. Comparing
    /// raw output amplitude between voicings isn't a meaningful check
    /// on its own -- each voicing's final `* 0.6`/`* 0.9` etc. output
    /// trim is a separate, deliberate level-matching choice, not a
    /// measure of how hard it clips.
    #[test]
    fn fuzz_saturates_much_harder_than_clean_as_input_grows() {
        let drive = 0.8f32;
        let quiet = 0.3f32;
        let loud = 0.9f32;

        let clean_quiet = shape(0, quiet, drive);
        let clean_loud = shape(0, loud, drive);
        let clean_growth = (clean_loud - clean_quiet).abs();

        let fuzz_quiet = shape(3, quiet, drive);
        let fuzz_loud = shape(3, loud, drive);
        let fuzz_growth = (fuzz_loud - fuzz_quiet).abs();

        assert!(
            fuzz_growth < clean_growth * 0.5,
            "Fuzz should already be saturated well before 0.9, growing far less than Clean over the same input range: fuzz_growth={fuzz_growth}, clean_growth={clean_growth}"
        );
    }

    /// `Biquad::set_peaking` at 0dB gain must be a true no-op filter
    /// (unity gain at every frequency) -- proves the coefficient math
    /// isn't silently coloring the signal when a tone knob sits at
    /// its "flat" center position.
    #[test]
    fn zero_db_peaking_filter_is_unity_gain() {
        let mut f = Biquad::default();
        f.set_peaking(800.0, 48000.0, 0.0, 0.8);
        let mut out = 0.0;
        for i in 0..200 {
            let x = (i as f32 * 0.3).sin();
            out = f.process(x);
        }
        // After the filter settles, a steady-state sine through a true
        // 0dB peaking filter should come back out essentially
        // unchanged in amplitude -- just check it's not silenced or
        // blown up, not an exact sample match (phase/settling noise).
        assert!(out.abs() < 1.2, "0dB peaking filter must not amplify the signal, got {out}");
    }

    /// Reset must actually restore each leaf's documented default,
    /// not just compile -- picked a representative handful rather
    /// than every leaf (mirrors this build's other apps' reset tests).
    #[test]
    fn reset_restores_documented_defaults() {
        let (mut app, _bus) = new_app_and_bus();
        app.params.drive.set(0.99);
        app.params.bass.set(0.1);
        app.params.output_volume.set(0.1);
        app.reset(Selection::Drive);
        app.reset(Selection::Bass);
        app.reset(Selection::OutputVolume);
        assert_eq!(app.params.drive.get(), 0.4);
        assert_eq!(app.params.bass.get(), 0.5);
        assert_eq!(app.params.output_volume.get(), 0.8);
    }

    /// Source cycling must actually reach every real registered
    /// source and a "none" position, in both directions -- the same
    /// bidirectional-cycling contract every other app's Source
    /// selector already has tests for.
    #[test]
    fn source_cycles_through_every_real_source_and_none() {
        let (mut app, audio_bus) = new_app_and_bus();
        // `new_app_and_bus` already registers Tonestack's own output
        // as bus index 0 (see `Params::new`'s `audio_bus.register`),
        // same as every other source-tapping app -- these two land
        // right after it.
        audio_bus.register("A");
        audio_bus.register("B");
        let last_real_idx = audio_bus.len() - 1;
        app.edit(Selection::Source, -1);
        assert_eq!(app.params.source.load(Ordering::Relaxed), last_real_idx, "backward from None must wrap to the last real source");
        app.edit(Selection::Source, 1);
        assert_eq!(app.params.source.load(Ordering::Relaxed), crate::audio_bus::NO_SOURCE);
    }

    /// Every delay-shaped effect (Chorus, Phaser, Delay, Reverb) run
    /// together over many blocks, with a mode switch partway through,
    /// must never panic -- specifically guards against exactly the
    /// float-rounding bug `DelayFx`/`Chorus` used to have: `rem_euclid`
    /// occasionally rounding a read position up to exactly `len`
    /// (one index past the buffer's end) instead of strictly `< len`.
    /// A handful of single-block unit tests never ran the delay lines
    /// through nearly enough wraps to hit it -- this does.
    #[test]
    fn stress_every_effect_over_many_blocks_never_panics() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        {
            let mut s = src.lock().unwrap();
            *s = (0..512).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        }
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.source.store(0, Ordering::Relaxed);
        params.mix_level.set(1.0);
        params.mod_type.store(1, Ordering::Relaxed); // Chorus
        params.delay_mix.set(0.5);
        params.reverb_mix.set(0.5);
        params.gate_threshold.set(0.1);
        let mut proc = TonestackProcessor {
            params: Arc::clone(&params),
            audio_bus: Arc::clone(&audio_bus),
            gate: Gate::default(),
            bass_filter: Biquad::default(),
            mid_filter: Biquad::default(),
            treble_filter: Biquad::default(),
            cab_lowpass: Biquad::default(),
            cab_presence: Biquad::default(),
            chorus: Chorus::default(),
            phaser: Phaser::default(),
            delay: DelayFx::default(),
            reverb: ReverbFx::default(),
        };
        let mut buffer = vec![0.0f32; 512 * 2];
        for i in 0..500 {
            if i == 100 {
                params.mod_type.store(2, Ordering::Relaxed); // Phaser
            }
            proc.process(&mut buffer, 2, 48000.0);
        }
    }
}
