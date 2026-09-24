//! A clone of Erica Synths' Sample Drum -- a dual-channel sample
//! player/slicer eurorack module, closely following its real manual
//! (encoder/menu layout, feature names, behavior) as far as this
//! sim's architecture allows:
//!
//! - **Two independent channels**, switched with one selector (the
//!   real module's physical 1/2 switch) -- Sample/Slice/Envelope/FX
//!   menus all edit whichever channel is currently selected, exactly
//!   as the manual describes ("use the switch to select the channel
//!   and upload samples; ... all other settings are set individually
//!   by sample in the corresponding channel").
//! - **4 real play modes** (Forward, Forward Loop, Backward, Backward
//!   Loop) with real Start/Loop/End point behavior: a loop plays
//!   Start->End once, then repeats Loop->End (mirrored for Backward:
//!   End->Start once, then repeats End->Loop) -- not just a label,
//!   see `DrumVoice::render`. Looping is orthogonal to slicing (the
//!   manual doesn't disable one for the other): with slicing active,
//!   a loop mode loops the *current* slice's own region until the
//!   next trigger jumps it to a different slice -- this reads as an
//!   always-running loop that a trigger just re-aims, never silence.
//! - **A real waveform + slice-cut view** (see `draw_slice_view`/
//!   `source_waveform_and_slices`) -- every slice boundary is drawn
//!   as a tick over the actual sample waveform, with the slice the
//!   next trigger will fire highlighted, so slicing is something you
//!   can see, not just a number.
//! - **Automatic slicing** (1-32 slices, Linear or real Zero-Crossing
//!   snapping -- see `snap_to_zero_crossing`) with the manual's own
//!   FWD/BKW/RND/NONE/CV step modes (see `SLICE_STEP_NAMES`) -- CV
//!   lets an external CV (any ModBus source, e.g. a Pam's channel)
//!   pick which slice plays, still gated by an incoming trigger,
//!   exactly as the manual describes it. Slices are advanced by the
//!   two manual trigger buttons (TRIG1/TRIG2, mapped to grid pads 0/1
//!   here), an internal clock (`Selection::AutoClock`/`AutoRate`), or
//!   a real external trigger patched into ModBus (see
//!   `ChannelParams::ext_trig`) -- Pam's Square/Euclidean channels
//!   routed there clock this exactly like a patch cable into the
//!   module's TRIG jack would. Combined with RND (or CV) step mode,
//!   this is the classic "chop a break and randomize which slice
//!   plays each hit" jungle/drum-and-bass technique -- see
//!   `SampleDrumProcessor::process`'s trigger handling.
//! - **A real Attack-Hold-Decay envelope** per channel (Short/Mid/Long
//!   range presets), gating every trigger.
//! - **One insert FX per channel** (Delay, Reverb, Lowpass, Highpass,
//!   Bitcrush, Fold, Drive), each real DSP with its own 2 parameters
//!   + Mix, matching the manual's ROOM/DAMP/MIX-style per-effect
//!   layout.
//! - **Presets**: save/recall a channel's full sample+slice+envelope+
//!   FX setup, by sample *name* (not index -- see sequencer.rs's Kit
//!   feature, which this mirrors) so a preset still loads correctly
//!   even if the sample library has changed since it was saved.
//!
//! Deliberately not implemented (redundant for a self-contained
//! digital sim with no physical CV jacks, SD card, or hardware to
//! calibrate): the literal CV Assign menu (attenuators, voltage
//! ranges, auto-calibration) -- this sim's existing uniform
//! modulation system (every key knob already takes an external
//! `modbus.rs` input from any other app) covers the same real
//! function without a separate assignment UI. Also skipped: manual
//! per-slice-point dragging (automatic slicing covers the real
//! workflow), SD card/RAM/firmware-update/SINGLE-DOUBLE-project
//! mechanics (no removable storage to simulate), the envelope's
//! "Relative" range mode, and Attack/Decay curve-shape morphing
//! (fixed exponential, same simplification this sim's other
//! envelopes already make).

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
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NUM_CHANNELS: usize = 2;
const MAX_SLICES: usize = 32;
const PLAY_MODE_NAMES: [&str; 4] = ["Forward", "Fwd Loop", "Backward", "Bwd Loop"];
const SLICE_MODE_NAMES: [&str; 2] = ["Linear", "Zero X"];
/// Matches the real module's own slice playback-order names exactly
/// (see its manual's Slice menu): FWD/BKW step sequentially through
/// slices one per trigger, RND picks one at random each trigger, NONE
/// always plays slice 1, and CV lets an external CV (here, any
/// `ModBus` source patched to `ChannelParams::ext_slice_cv`, e.g. a
/// Pam's channel) pick which slice plays -- still requires an
/// incoming trigger to actually fire it, exactly as the manual notes
/// ("CV setting ... still need the incoming trigger to actually play
/// back the CV defined slice").
const SLICE_STEP_NAMES: [&str; 5] = ["FWD", "BKW", "RND", "NONE", "CV"];
const ENV_RANGE_NAMES: [&str; 3] = ["Short", "Mid", "Long"];
/// Max Attack/Hold/Decay time in seconds per range preset -- matches
/// the manual's own SHORT (1s)/MID (3s)/LONG (10s) figures.
const ENV_RANGE_MAX_SECONDS: [f32; 3] = [1.0, 3.0, 10.0];
const FX_NAMES: [&str; 8] = ["None", "Delay", "Reverb", "Lowpass", "Highpass", "Bitcrush", "Fold", "Drive"];
const SAMPLES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/samples");
const NUM_PRESETS: usize = 8;
const PRESETS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/presets/sample_drum");

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.01).clamp(min, max);
    value.set(next);
}

fn truncate_display(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max).collect::<String>())
    }
}

// --- Sample library (flat, no pack/type tree -- this app's own
// Source browsing is a single flat list like Tonestack's, not a
// drill-down like Sequencer's tracks use) ---

struct SampleSlot {
    name: String,
    path: std::path::PathBuf,
    decoded: std::sync::OnceLock<(Vec<f32>, f32)>,
}

impl SampleSlot {
    fn new(path: std::path::PathBuf) -> Self {
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string();
        Self { name, path, decoded: std::sync::OnceLock::new() }
    }

    fn decoded(&self) -> (&[f32], f32) {
        let (data, rate) = self.decoded.get_or_init(|| match decode_wav(&self.path) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("sample_drum: couldn't load {}: {e}", self.path.display());
                (Vec::new(), 44100.0)
            }
        });
        (data.as_slice(), *rate)
    }
}

fn decode_wav(path: &Path) -> Result<(Vec<f32>, f32), Box<dyn std::error::Error>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let max = (1i64 << spec.bits_per_sample.saturating_sub(1).max(1)).max(1) as f32;
            reader.samples::<i32>().collect::<Result<Vec<i32>, _>>()?.into_iter().map(|s| s as f32 / max).collect()
        }
    };
    let mono = if channels > 1 {
        raw.chunks(channels).map(|c| c.iter().sum::<f32>() / channels as f32).collect()
    } else {
        raw
    };
    Ok((mono, spec.sample_rate as f32))
}

/// Recursively finds every .wav under `dir`, sorted depth-first by
/// path for a stable, predictable browse order -- no pack/type tree
/// (see module doc comment), just one flat list.
fn scan_samples(dir: &Path) -> Vec<SampleSlot> {
    let mut out = Vec::new();
    scan_dir(dir, &mut out);
    out
}

fn scan_dir(dir: &Path, out: &mut Vec<SampleSlot>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            scan_dir(&path, out);
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("wav")) {
            out.push(SampleSlot::new(path));
        }
    }
}

// --- Real DSP: envelope, voice, FX -------------------------------

/// A real Attack-Hold-Decay envelope -- see the module doc comment
/// for why "Relative" range and curve-shape morphing are skipped.
#[derive(Default)]
struct Envelope {
    state: u8, // 0 idle, 1 attack, 2 hold, 3 decay
    level: f32,
    elapsed: f32,
}

impl Envelope {
    fn trigger(&mut self) {
        self.state = 1;
        self.elapsed = 0.0;
        self.level = 0.0;
    }

    fn tick(&mut self, attack_s: f32, hold_s: f32, decay_s: f32, dt: f32) -> f32 {
        match self.state {
            1 => {
                self.elapsed += dt;
                self.level = (self.elapsed / attack_s.max(0.001)).min(1.0);
                if self.elapsed >= attack_s {
                    self.state = 2;
                    self.elapsed = 0.0;
                    self.level = 1.0;
                }
            }
            2 => {
                self.level = 1.0;
                self.elapsed += dt;
                if self.elapsed >= hold_s {
                    self.state = 3;
                    self.elapsed = 0.0;
                }
            }
            3 => {
                self.elapsed += dt;
                self.level = (1.0 - self.elapsed / decay_s.max(0.001)).max(0.0);
                if self.elapsed >= decay_s {
                    self.state = 0;
                    self.level = 0.0;
                }
            }
            _ => self.level = 0.0,
        }
        self.level
    }

    fn active(&self) -> bool {
        self.state != 0
    }
}

/// One channel's sample-playback voice -- real Start/Loop/End
/// behavior for all 4 play modes (see module doc comment), envelope-
/// gated, with the same linear-interpolation resampling every other
/// sample-based app in this build uses.
#[derive(Default)]
struct DrumVoice {
    pos: f32,
    dir: f32,
    lower: f32,
    upper: f32,
    loop_target: f32,
    looping: bool,
    playing: bool,
    env: Envelope,
}

impl DrumVoice {
    /// `lower_frac`/`upper_frac`/`loop_frac` are 0..1 of the sample's
    /// total length (already resolved to whichever slice is playing,
    /// if slicing is active -- see `SampleDrumProcessor::process`).
    /// `backward` picks which end playback starts from; `looping`
    /// makes it wrap between `loop_frac` and the far end instead of
    /// stopping there.
    fn trigger(&mut self, lower_frac: f32, upper_frac: f32, loop_frac: f32, backward: bool, looping: bool, len: usize) {
        let len_f = (len.max(2) - 1) as f32;
        let lower = lower_frac.clamp(0.0, 1.0) * len_f;
        let upper = (upper_frac.clamp(0.0, 1.0) * len_f).max(lower + 1.0).min(len_f);
        self.lower = lower;
        self.upper = upper;
        self.loop_target = (loop_frac.clamp(0.0, 1.0) * len_f).clamp(lower, upper);
        self.dir = if backward { -1.0 } else { 1.0 };
        self.pos = if backward { upper } else { lower };
        self.looping = looping;
        self.playing = true;
        self.env.trigger();
    }

    fn render(&mut self, data: &[f32], native_rate: f32, device_rate: f32, semitones: i32, attack: f32, hold: f32, decay: f32, sample_rate: f32) -> f32 {
        if !self.playing || data.len() < 2 {
            return 0.0;
        }
        let rate = (native_rate / device_rate) * 2f32.powf(semitones as f32 / 12.0);
        // `pos` is float math driven by several independently-clamped
        // fractions; never trust it to stay in range on its own --
        // clamp the cast every time (see sequencer.rs's own
        // Track Rate / delay-line bugs this exact pattern was added
        // to fix).
        let i = (self.pos as usize).min(data.len() - 2);
        let frac = (self.pos - i as f32).clamp(0.0, 1.0);
        let sample = data[i] + (data[i + 1] - data[i]) * frac;

        let env_gain = self.env.tick(attack, hold, decay, 1.0 / sample_rate);
        if !self.env.active() {
            self.playing = false;
        }

        self.pos += rate.max(0.01) * self.dir;
        if self.dir > 0.0 && self.pos >= self.upper {
            if self.looping {
                self.pos = self.loop_target + (self.pos - self.upper);
            } else {
                self.playing = false;
            }
        } else if self.dir < 0.0 && self.pos <= self.lower {
            if self.looping {
                self.pos = self.loop_target - (self.lower - self.pos);
            } else {
                self.playing = false;
            }
        }
        sample * env_gain
    }
}

/// Finds the sample-frame position nearest `frac` (0..1 of `data`'s
/// length) where the waveform actually crosses zero, searching a
/// small window either side -- real zero-crossing detection (not a
/// label), same idea the manual's "ZC" slicing mode describes: it
/// minimizes clicks when a slice boundary lands mid-waveform.
fn snap_to_zero_crossing(data: &[f32], frac: f32) -> f32 {
    let len = data.len();
    if len < 4 {
        return frac;
    }
    let len_f = (len - 1) as f32;
    let center = ((frac.clamp(0.0, 1.0) * len_f) as usize).min(len - 2);
    let window = (len / 200).clamp(4, 2000);
    let lo = center.saturating_sub(window);
    let hi = (center + window).min(len - 2);
    let mut best = center;
    let mut best_dist = usize::MAX;
    for i in lo..=hi {
        if (data[i] >= 0.0) != (data[i + 1] >= 0.0) {
            let dist = i.abs_diff(center);
            if dist < best_dist {
                best_dist = dist;
                best = i;
            }
        }
    }
    best as f32 / len_f
}

/// Peak amplitude (0..1) per bucket, `width` buckets spanning the
/// whole of `data` -- a fast, allocation-light waveform overview for
/// display (not audio-accurate, just "what does this sample look
/// like"), same idea sequencer.rs's own waveform previews use.
fn downsample_peaks(data: &[f32], width: usize) -> Vec<f32> {
    if data.is_empty() || width == 0 {
        return Vec::new();
    }
    let bucket = (data.len() / width).max(1);
    (0..width)
        .map(|i| {
            let lo = i * bucket;
            let hi = (lo + bucket).min(data.len());
            data.get(lo..hi).map(|s| s.iter().fold(0.0f32, |m, v| m.max(v.abs()))).unwrap_or(0.0).min(1.0)
        })
        .collect()
}

/// The `[lower, upper)` fraction bounds (0..1 of the whole sample)
/// slice `step` covers, given `start`/`end` (the sample screen's own
/// Start/End, which bound the whole sliced region) split into
/// `num_slices` equal parts -- optionally snapped to real zero
/// crossings. `num_slices <= 1` returns `(start, end)` unchanged (no
/// slicing).
fn slice_bounds(data: &[f32], start: f32, end: f32, num_slices: usize, zero_cross: bool, step: usize) -> (f32, f32) {
    let num_slices = num_slices.max(1);
    if num_slices <= 1 {
        return (start, end);
    }
    let span = (end - start).max(0.0);
    let slice_span = span / num_slices as f32;
    let step = step % num_slices;
    let mut lower = start + slice_span * step as f32;
    let mut upper = lower + slice_span;
    if zero_cross {
        lower = snap_to_zero_crossing(data, lower);
        upper = snap_to_zero_crossing(data, upper);
    }
    (lower, upper)
}

// --- FX: a compact biquad (lowpass/highpass), a feedback delay, a
// small Schroeder reverb, a bitcrusher and a wavefolder -- same
// "real but simplified" DSP standard every other effect app in this
// build already holds itself to (see e.g. tonestack.rs's own copies
// of these same building blocks; each app keeps its own rather than
// sharing one module, same existing convention). ---

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

    fn set_lowpass(&mut self, freq: f32, sample_rate: f32, q: f32) {
        let w0 = std::f32::consts::TAU * (freq / sample_rate).clamp(0.001, 0.49);
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 - cos_w0) / 2.0) / a0;
        self.b1 = (1.0 - cos_w0) / a0;
        self.b2 = self.b0;
        self.a1 = (-2.0 * cos_w0) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn set_highpass(&mut self, freq: f32, sample_rate: f32, q: f32) {
        let w0 = std::f32::consts::TAU * (freq / sample_rate).clamp(0.001, 0.49);
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 + cos_w0) / 2.0) / a0;
        self.b1 = (-(1.0 + cos_w0)) / a0;
        self.b2 = self.b0;
        self.a1 = (-2.0 * cos_w0) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }
}

#[derive(Default)]
struct DelayFx {
    buf: Vec<f32>,
    write: usize,
    damp_state: f32,
}

impl DelayFx {
    fn ensure_capacity(&mut self, sample_rate: f32) {
        let needed = (1.01 * sample_rate) as usize; // up to ~1s delay
        if self.buf.len() != needed {
            self.buf = vec![0.0; needed.max(8)];
            self.write = 0;
        }
    }

    fn process(&mut self, x: f32, time_ms: f32, feedback: f32, sample_rate: f32) -> f32 {
        self.ensure_capacity(sample_rate);
        let len = self.buf.len();
        let delay_samples = (time_ms.clamp(10.0, 1000.0) * 0.001 * sample_rate).clamp(1.0, (len - 2) as f32);
        let read_pos = (self.write as f32 - delay_samples).rem_euclid(len as f32);
        let i0 = (read_pos as usize).min(len - 1);
        let i1 = (i0 + 1) % len;
        let frac = (read_pos - i0 as f32).clamp(0.0, 1.0);
        let wet = self.buf[i0] + (self.buf[i1] - self.buf[i0]) * frac;
        self.damp_state += (wet - self.damp_state) * 0.35;
        self.buf[self.write] = x + self.damp_state * feedback.clamp(0.0, 0.95);
        self.write = (self.write + 1) % len;
        wet
    }
}

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
        let len_a = ((0.0311 * sample_rate) as usize).max(4);
        let len_b = ((0.0379 * sample_rate) as usize).max(4);
        let len_ap = ((0.0047 * sample_rate) as usize).max(4);
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

    fn process(&mut self, x: f32, room: f32, damp: f32, sample_rate: f32) -> f32 {
        self.ensure_capacity(sample_rate);
        let fb = 0.55 + room.clamp(0.0, 1.0) * 0.4;
        let a_out = self.comb_a[self.pos_a];
        self.comb_a[self.pos_a] = x + a_out * fb * (1.0 - damp.clamp(0.0, 1.0) * 0.3);
        self.pos_a = (self.pos_a + 1) % self.comb_a.len();
        let b_out = self.comb_b[self.pos_b];
        self.comb_b[self.pos_b] = x + b_out * fb * (1.0 - damp.clamp(0.0, 1.0) * 0.3);
        self.pos_b = (self.pos_b + 1) % self.comb_b.len();
        let combined = (a_out + b_out) * 0.5;
        let ap_out = self.allpass[self.pos_ap];
        let ap_in = combined + ap_out * 0.5;
        self.allpass[self.pos_ap] = ap_in;
        self.pos_ap = (self.pos_ap + 1) % self.allpass.len();
        ap_out - ap_in * 0.5
    }
}

#[derive(Default)]
struct Bitcrush {
    held: f32,
    counter: u32,
}

impl Bitcrush {
    fn process(&mut self, x: f32, bits: f32, rate_div: u32) -> f32 {
        if self.counter == 0 {
            let levels = 2f32.powf(bits.clamp(1.0, 16.0));
            self.held = (x * levels).round() / levels;
        }
        self.counter = (self.counter + 1) % rate_div.max(1);
        self.held
    }
}

fn fold_fx(x: f32, amount: f32) -> f32 {
    let mut y = x * (1.0 + amount.clamp(0.0, 1.0) * 8.0);
    for _ in 0..6 {
        if y > 1.0 {
            y = 2.0 - y;
        } else if y < -1.0 {
            y = -2.0 - y;
        } else {
            break;
        }
    }
    y
}

fn drive_fx(x: f32, amount: f32) -> f32 {
    (x * (1.0 + amount.clamp(0.0, 1.0) * 9.0)).tanh()
}

// --- Presets: save/recall a channel's setup by sample name --------

#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct DrumPreset {
    sample: Option<String>,
    mode: u32,
    tune: i32,
    start: f32,
    loop_point: f32,
    end: f32,
    num_slices: usize,
    slice_mode: u32,
    slice_step: u32,
    #[serde(default)]
    auto_clock: bool,
    #[serde(default = "default_auto_rate_bpm")]
    auto_rate_bpm: f32,
    env_attack: f32,
    env_hold: f32,
    env_decay: f32,
    env_range: u32,
    fx_type: u32,
    fx_param1: f32,
    fx_param2: f32,
    fx_mix: f32,
    volume: f32,
}

fn default_auto_rate_bpm() -> f32 {
    120.0
}

fn preset_path(dir: &Path, slot: usize) -> std::path::PathBuf {
    dir.join(format!("preset_{}.toml", slot + 1))
}

// --- Menu / app scaffolding ---------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Channel,
    Preset,
    SavePreset,
    LoadPreset,
    Sample,
    Mode,
    Tune,
    Start,
    LoopPoint,
    End,
    NumSlices,
    SliceMode,
    SliceStep,
    ResetSlice,
    /// Self-triggers this channel at `AutoRate` instead of waiting
    /// for TRIG1/TRIG2 -- see the module doc comment for why this
    /// exists (the real module always needs an external clock
    /// patched into its trigger input for this exact "chop a break
    /// and randomize it" technique; this sim has no literal patch
    /// cables, so an internal clock stands in for one). Also what F3
    /// (Start/Stop) toggles for whichever channel is selected -- see
    /// `SampleDrumApp::running`.
    AutoClock,
    /// The loop's own tempo (a 1-bar/4-beat loop, per the manual's own
    /// suggested workflow) -- NOT a fixed absolute trigger rate. The
    /// actual per-slice trigger period divides this by `NumSlices`
    /// (see `SampleDrumProcessor::process`), so re-chopping the same
    /// loop from 4 to 16 slices keeps tiling the same bar seamlessly
    /// instead of leaving dead air between now-much-shorter clips.
    AutoRate,
    EnvAttack,
    EnvHold,
    EnvDecay,
    EnvRange,
    FxType,
    FxParam1,
    FxParam2,
    FxMix,
    Volume,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 6;

struct ChannelParams {
    sample: AtomicUsize,
    mode: AtomicU32,
    tune: AtomicI32,
    start: AtomicF32,
    loop_point: AtomicF32,
    end: AtomicF32,
    num_slices: AtomicUsize,
    slice_mode: AtomicU32,
    slice_step: AtomicU32,
    /// Self-triggers this channel at `auto_rate_bpm` instead of only
    /// responding to TRIG1/TRIG2 -- see `Selection::AutoClock`'s doc
    /// comment.
    auto_clock: AtomicBool,
    auto_rate_bpm: AtomicF32,
    /// One-shot "TRIG1/TRIG2 pressed" pulse, same convention every
    /// other trigger-style input in this build uses -- also set by
    /// the internal auto-clock (see `SampleDrumProcessor::process`),
    /// which is otherwise indistinguishable from a real trigger.
    trig_pending: AtomicBool,
    /// The slice index the *next* trigger will play for FWD/BKW
    /// stepping -- audio-thread-owned, but exposed for the UI's
    /// "current slice" display.
    step_index: AtomicUsize,
    env_attack: AtomicF32,
    env_hold: AtomicF32,
    env_decay: AtomicF32,
    env_range: AtomicU32,
    fx_type: AtomicU32,
    fx_param1: AtomicF32,
    fx_param2: AtomicF32,
    fx_mix: AtomicF32,
    volume: AtomicF32,
    ext_tune: Arc<AtomicF32>,
    ext_start: Arc<AtomicF32>,
    ext_end: Arc<AtomicF32>,
    /// Picks the slice in `SliceStep::CV` mode (0..1 of `num_slices`) --
    /// routed from any other app's ModBus output (Pam's, typically),
    /// same as every other `ext_*` field here.
    ext_slice_cv: Arc<AtomicF32>,
    /// A rising edge (crossing above 0.5) fires this channel exactly
    /// like a manual TRIG1/TRIG2 press -- lets Pam's (or anything else
    /// that writes a gate/pulse into ModBus) clock this channel the
    /// same way an external trigger patched into the real module's
    /// TRIG jack would. See `SampleDrumProcessor::process`'s edge
    /// detection.
    ext_trig: Arc<AtomicF32>,
    /// Real, live output waveform for this channel, downsampled once
    /// per audio block -- Slint panel only.
    waveform_snapshot: Mutex<Vec<f32>>,
    /// The source sample's own waveform (not the live output),
    /// downsampled for display, plus the Start/End-bounded slice
    /// boundaries -- recomputed only when the sample or slice layout
    /// actually changes (see `SampleDrumApp::refresh_source_waveform`),
    /// not every frame. UI/Slint panel only, never read on the audio
    /// thread.
    source_waveform_cache: Mutex<SourceWaveformCache>,
}

/// See `ChannelParams::source_waveform_cache`.
#[derive(Default, Clone)]
struct SourceWaveformCache {
    /// (sample index, start, end, num_slices, slice_mode) this cache
    /// was computed for -- recomputed whenever any of these change.
    key: (usize, u32, u32, usize, u32),
    waveform: Vec<f32>,
    slice_bounds: Vec<(f32, f32)>,
}

impl ChannelParams {
    fn new(ch: usize, modbus: &ModBus) -> Self {
        Self {
            sample: AtomicUsize::new(crate::audio_bus::NO_SOURCE),
            mode: AtomicU32::new(0),
            tune: AtomicI32::new(0),
            start: AtomicF32::new(0.0),
            loop_point: AtomicF32::new(0.0),
            end: AtomicF32::new(1.0),
            num_slices: AtomicUsize::new(1),
            slice_mode: AtomicU32::new(0),
            slice_step: AtomicU32::new(0),
            auto_clock: AtomicBool::new(false),
            auto_rate_bpm: AtomicF32::new(120.0),
            trig_pending: AtomicBool::new(false),
            step_index: AtomicUsize::new(0),
            env_attack: AtomicF32::new(0.0),
            env_hold: AtomicF32::new(0.0),
            env_decay: AtomicF32::new(0.3),
            env_range: AtomicU32::new(1),
            fx_type: AtomicU32::new(0),
            fx_param1: AtomicF32::new(0.3),
            fx_param2: AtomicF32::new(0.3),
            fx_mix: AtomicF32::new(0.0),
            volume: AtomicF32::new(0.8),
            ext_tune: modbus.register(format!("Sample Drum: Ch{} Tune", ch + 1)),
            ext_start: modbus.register(format!("Sample Drum: Ch{} Start", ch + 1)),
            ext_end: modbus.register(format!("Sample Drum: Ch{} End", ch + 1)),
            ext_slice_cv: modbus.register(format!("Sample Drum: Ch{} Slice CV", ch + 1)),
            ext_trig: modbus.register(format!("Sample Drum: Ch{} Trig", ch + 1)),
            waveform_snapshot: Mutex::new(Vec::new()),
            source_waveform_cache: Mutex::new(SourceWaveformCache::default()),
        }
    }
}

struct Params {
    channel: AtomicUsize,
    channels: [ChannelParams; NUM_CHANNELS],
    samples: Vec<SampleSlot>,
    preset_slot: AtomicUsize,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let samples = scan_samples(Path::new(SAMPLES_DIR));
        let (mix_level, ext_mix_level) = mixer_bus.register("Sample Drum", modbus);
        Self {
            channel: AtomicUsize::new(0),
            channels: std::array::from_fn(|i| ChannelParams::new(i, modbus)),
            samples,
            preset_slot: AtomicUsize::new(0),
            bus_out: audio_bus.register("Sample Drum"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct SampleDrumApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    prev_grid: [bool; 16],
}

// --- Sample Drum's own palette: flat solid colors, not a
// device-wide theme -- Warm hardware-chassis grey with a bold sampler-pad orange -- gritty MPC/boom-bap hardware energy. ---

const SAMPLE_DRUM_BG: Rgb565 = Rgb565::new(4, 7, 3);
const SAMPLE_DRUM_TITLE: Rgb565 = Rgb565::new(29, 58, 28);
const SAMPLE_DRUM_ACCENT: Rgb565 = Rgb565::new(31, 26, 3);
const SAMPLE_DRUM_DIM: Rgb565 = Rgb565::new(15, 28, 13);

impl SampleDrumApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
            prev_grid: [false; 16],
        }
    }

    fn ch(&self) -> usize {
        self.params.channel.load(Ordering::Relaxed)
    }

    fn resolved_sample(&self, ch: usize) -> Option<usize> {
        let idx = self.params.channels[ch].sample.load(Ordering::Relaxed);
        if idx == crate::audio_bus::NO_SOURCE { None } else { Some(idx) }
    }

    fn sample_name(&self, ch: usize) -> String {
        match self.resolved_sample(ch).and_then(|i| self.params.samples.get(i)) {
            Some(slot) => truncate_display(&slot.name, 18),
            None => "(empty)".into(),
        }
    }

    fn warm_selected_sample(&self, ch: usize) {
        if let Some(slot) = self.resolved_sample(ch).and_then(|i| self.params.samples.get(i)) {
            slot.decoded();
        }
    }

    fn group_name(&self, g: usize) -> &'static str {
        match g {
            0 => "Global",
            1 => "Sample",
            2 => "Slice",
            3 => "Envelope",
            4 => "FX",
            _ => "Output",
        }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => vec![Selection::Channel, Selection::Preset, Selection::SavePreset, Selection::LoadPreset],
            1 => vec![Selection::Sample, Selection::Mode, Selection::Tune, Selection::Start, Selection::LoopPoint, Selection::End],
            2 => vec![Selection::NumSlices, Selection::SliceMode, Selection::SliceStep, Selection::ResetSlice, Selection::AutoClock, Selection::AutoRate],
            3 => vec![Selection::EnvAttack, Selection::EnvHold, Selection::EnvDecay, Selection::EnvRange],
            4 => vec![Selection::FxType, Selection::FxParam1, Selection::FxParam2, Selection::FxMix],
            _ => vec![Selection::Volume],
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
        let ch = self.ch();
        let c = &self.params.channels[ch];
        match g {
            0 => format!("channel {}", ch + 1),
            1 => self.sample_name(ch),
            2 => format!("{} {}", c.num_slices.load(Ordering::Relaxed), SLICE_STEP_NAMES[c.slice_step.load(Ordering::Relaxed) as usize % SLICE_STEP_NAMES.len()]),
            3 => format!("A{:.0}% H{:.0}% D{:.0}%", c.env_attack.get() * 100.0, c.env_hold.get() * 100.0, c.env_decay.get() * 100.0),
            4 => FX_NAMES[c.fx_type.load(Ordering::Relaxed) as usize % FX_NAMES.len()].to_string(),
            _ => format!("{:.0}%", c.volume.get() * 100.0),
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Channel => "Channel".into(),
            Selection::Preset => "Preset".into(),
            Selection::SavePreset => "Save Preset".into(),
            Selection::LoadPreset => "Load Preset".into(),
            Selection::Sample => "Sample".into(),
            Selection::Mode => "Mode".into(),
            Selection::Tune => "Tune".into(),
            Selection::Start => "Start".into(),
            Selection::LoopPoint => "Loop".into(),
            Selection::End => "End".into(),
            Selection::NumSlices => "Slices".into(),
            Selection::SliceMode => "Slice Mode".into(),
            Selection::SliceStep => "Step".into(),
            Selection::ResetSlice => "Reset To Slice 1".into(),
            Selection::AutoClock => "Auto Clock".into(),
            Selection::AutoRate => "Auto Rate".into(),
            Selection::EnvAttack => "Attack".into(),
            Selection::EnvHold => "Hold".into(),
            Selection::EnvDecay => "Decay".into(),
            Selection::EnvRange => "Range".into(),
            Selection::FxType => "FX".into(),
            Selection::FxParam1 => "Param 1".into(),
            Selection::FxParam2 => "Param 2".into(),
            Selection::FxMix => "Mix".into(),
            Selection::Volume => "Volume".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        let ch = self.ch();
        let c = &self.params.channels[ch];
        match sel {
            Selection::Channel => format!("{}", ch + 1),
            Selection::Preset => format!("{}", self.params.preset_slot.load(Ordering::Relaxed) + 1),
            Selection::SavePreset | Selection::LoadPreset => "press knob2".into(),
            Selection::Sample => self.sample_name(ch),
            Selection::Mode => PLAY_MODE_NAMES[c.mode.load(Ordering::Relaxed) as usize % 4].to_string(),
            Selection::Tune => format!("{:+}", c.tune.load(Ordering::Relaxed)),
            Selection::Start => format!("{:.0}%", c.start.get() * 100.0),
            Selection::LoopPoint => format!("{:.0}%", c.loop_point.get() * 100.0),
            Selection::End => format!("{:.0}%", c.end.get() * 100.0),
            Selection::NumSlices => format!("{}", c.num_slices.load(Ordering::Relaxed)),
            Selection::SliceMode => SLICE_MODE_NAMES[c.slice_mode.load(Ordering::Relaxed) as usize % 2].to_string(),
            Selection::SliceStep => SLICE_STEP_NAMES[c.slice_step.load(Ordering::Relaxed) as usize % SLICE_STEP_NAMES.len()].to_string(),
            Selection::ResetSlice => "press knob2".into(),
            Selection::AutoClock => if c.auto_clock.load(Ordering::Relaxed) { "ON".into() } else { "off".into() },
            Selection::AutoRate => format!("{:.0} BPM", c.auto_rate_bpm.get()),
            Selection::EnvAttack => format!("{:.0}%", c.env_attack.get() * 100.0),
            Selection::EnvHold => format!("{:.0}%", c.env_hold.get() * 100.0),
            Selection::EnvDecay => format!("{:.0}%", c.env_decay.get() * 100.0),
            Selection::EnvRange => ENV_RANGE_NAMES[c.env_range.load(Ordering::Relaxed) as usize % 3].to_string(),
            Selection::FxType => FX_NAMES[c.fx_type.load(Ordering::Relaxed) as usize % FX_NAMES.len()].to_string(),
            Selection::FxParam1 => format!("{:.0}%", c.fx_param1.get() * 100.0),
            Selection::FxParam2 => format!("{:.0}%", c.fx_param2.get() * 100.0),
            Selection::FxMix => format!("{:.0}%", c.fx_mix.get() * 100.0),
            Selection::Volume => format!("{:.0}%", c.volume.get() * 100.0),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        let ch = self.ch();
        let c = &self.params.channels[ch];
        match sel {
            Selection::Channel => {
                let next = (ch as i32 + step).rem_euclid(NUM_CHANNELS as i32) as usize;
                self.params.channel.store(next, Ordering::Relaxed);
            }
            Selection::Preset => {
                let cur = self.params.preset_slot.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(NUM_PRESETS as i32);
                self.params.preset_slot.store(next as usize, Ordering::Relaxed);
            }
            Selection::SavePreset | Selection::LoadPreset | Selection::ResetSlice => {} // press-only, see `reset`
            Selection::AutoClock => c.auto_clock.store(delta > 0, Ordering::Relaxed),
            Selection::AutoRate => {
                // A plain `bump` (scaled for 0..1-range knobs) would
                // take hundreds of ticks to move 1 BPM across this
                // 20..300 range -- same wider per-tick scale
                // sequencer.rs's own BPM edit uses.
                let next = (c.auto_rate_bpm.get() + accelerate(delta) * sensitivity * 2.0).clamp(20.0, 300.0);
                c.auto_rate_bpm.set(next);
            }
            Selection::Sample => {
                let cur = c.sample.load(Ordering::Relaxed);
                let next = crate::audio_bus::cycle_source(cur, step, self.params.samples.len());
                c.sample.store(next, Ordering::Relaxed);
                self.warm_selected_sample(ch);
            }
            Selection::Mode => {
                let cur = c.mode.load(Ordering::Relaxed) as i32;
                c.mode.store((cur + step).rem_euclid(4) as u32, Ordering::Relaxed);
            }
            Selection::Tune => {
                let cur = c.tune.load(Ordering::Relaxed);
                c.tune.store(cur + step, Ordering::Relaxed);
            }
            Selection::Start => bump(&c.start, delta, sensitivity, 0.0, 1.0),
            Selection::LoopPoint => bump(&c.loop_point, delta, sensitivity, 0.0, 1.0),
            Selection::End => bump(&c.end, delta, sensitivity, 0.0, 1.0),
            Selection::NumSlices => {
                let cur = c.num_slices.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(1, MAX_SLICES as i32);
                c.num_slices.store(next as usize, Ordering::Relaxed);
            }
            Selection::SliceMode => {
                let cur = c.slice_mode.load(Ordering::Relaxed) as i32;
                c.slice_mode.store((cur + step).rem_euclid(2) as u32, Ordering::Relaxed);
            }
            Selection::SliceStep => {
                let cur = c.slice_step.load(Ordering::Relaxed) as i32;
                c.slice_step.store((cur + step).rem_euclid(SLICE_STEP_NAMES.len() as i32) as u32, Ordering::Relaxed);
            }
            Selection::EnvAttack => bump(&c.env_attack, delta, sensitivity, 0.0, 1.0),
            Selection::EnvHold => bump(&c.env_hold, delta, sensitivity, 0.0, 1.0),
            Selection::EnvDecay => bump(&c.env_decay, delta, sensitivity, 0.0, 1.0),
            Selection::EnvRange => {
                let cur = c.env_range.load(Ordering::Relaxed) as i32;
                c.env_range.store((cur + step).rem_euclid(3) as u32, Ordering::Relaxed);
            }
            Selection::FxType => {
                let cur = c.fx_type.load(Ordering::Relaxed) as i32;
                c.fx_type.store((cur + step).rem_euclid(FX_NAMES.len() as i32) as u32, Ordering::Relaxed);
            }
            Selection::FxParam1 => bump(&c.fx_param1, delta, sensitivity, 0.0, 1.0),
            Selection::FxParam2 => bump(&c.fx_param2, delta, sensitivity, 0.0, 1.0),
            Selection::FxMix => bump(&c.fx_mix, delta, sensitivity, 0.0, 1.0),
            Selection::Volume => bump(&c.volume, delta, sensitivity, 0.0, 1.0),
        }
    }

    fn reset(&mut self, sel: Selection) {
        let ch = self.ch();
        let c = &self.params.channels[ch];
        match sel {
            Selection::Channel => self.params.channel.store(0, Ordering::Relaxed),
            Selection::Preset => self.params.preset_slot.store(0, Ordering::Relaxed),
            Selection::SavePreset => self.save_preset(),
            Selection::LoadPreset => self.load_preset(),
            Selection::Sample | Selection::Mode | Selection::SliceMode | Selection::FxType => {} // no sensible single default
            Selection::Tune => c.tune.store(0, Ordering::Relaxed),
            Selection::Start => c.start.set(0.0),
            Selection::LoopPoint => c.loop_point.set(0.0),
            Selection::End => c.end.set(1.0),
            Selection::NumSlices => c.num_slices.store(1, Ordering::Relaxed),
            Selection::SliceStep => c.slice_step.store(0, Ordering::Relaxed),
            Selection::ResetSlice => c.step_index.store(0, Ordering::Relaxed),
            Selection::AutoClock => c.auto_clock.store(false, Ordering::Relaxed),
            Selection::AutoRate => c.auto_rate_bpm.set(120.0),
            Selection::EnvAttack => c.env_attack.set(0.0),
            Selection::EnvHold => c.env_hold.set(0.0),
            Selection::EnvDecay => c.env_decay.set(0.3),
            Selection::EnvRange => c.env_range.store(1, Ordering::Relaxed),
            Selection::FxParam1 => c.fx_param1.set(0.3),
            Selection::FxParam2 => c.fx_param2.set(0.3),
            Selection::FxMix => c.fx_mix.set(0.0),
            Selection::Volume => c.volume.set(0.8),
        }
    }

    /// Writes the current channel's full setup to `Params::preset_slot`'s
    /// file. See `save_preset_to` for why `dir` is a parameter --
    /// tests use a scratch directory instead of the real project's.
    fn save_preset_to(&self, dir: &Path) {
        let ch = self.ch();
        let c = &self.params.channels[ch];
        let preset = DrumPreset {
            sample: self.resolved_sample(ch).and_then(|i| self.params.samples.get(i)).map(|s| s.name.clone()),
            mode: c.mode.load(Ordering::Relaxed),
            tune: c.tune.load(Ordering::Relaxed),
            start: c.start.get(),
            loop_point: c.loop_point.get(),
            end: c.end.get(),
            num_slices: c.num_slices.load(Ordering::Relaxed),
            slice_mode: c.slice_mode.load(Ordering::Relaxed),
            slice_step: c.slice_step.load(Ordering::Relaxed),
            auto_clock: c.auto_clock.load(Ordering::Relaxed),
            auto_rate_bpm: c.auto_rate_bpm.get(),
            env_attack: c.env_attack.get(),
            env_hold: c.env_hold.get(),
            env_decay: c.env_decay.get(),
            env_range: c.env_range.load(Ordering::Relaxed),
            fx_type: c.fx_type.load(Ordering::Relaxed),
            fx_param1: c.fx_param1.get(),
            fx_param2: c.fx_param2.get(),
            fx_mix: c.fx_mix.get(),
            volume: c.volume.get(),
        };
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("sample_drum: couldn't create {}: {e}", dir.display());
            return;
        }
        let path = preset_path(dir, self.params.preset_slot.load(Ordering::Relaxed));
        match toml::to_string_pretty(&preset) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    eprintln!("sample_drum: couldn't save {}: {e}", path.display());
                }
            }
            Err(e) => eprintln!("sample_drum: couldn't serialize preset: {e}"),
        }
    }

    fn save_preset(&self) {
        self.save_preset_to(Path::new(PRESETS_DIR));
    }

    /// Loads `Params::preset_slot`'s file over the current channel. A
    /// slot that's never been saved, or a sample name the current
    /// library no longer has, degrades gracefully rather than
    /// erroring -- same "fail closed" convention `sequencer.rs`'s Kit
    /// loading uses.
    fn load_preset_from(&self, dir: &Path) {
        let path = preset_path(dir, self.params.preset_slot.load(Ordering::Relaxed));
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("sample_drum: couldn't load {}: {e}", path.display());
                return;
            }
        };
        let preset: DrumPreset = match toml::from_str(&text) {
            Ok(preset) => preset,
            Err(e) => {
                eprintln!("sample_drum: couldn't parse {}: {e}", path.display());
                return;
            }
        };
        let ch = self.ch();
        let c = &self.params.channels[ch];
        let sample_idx = preset
            .sample
            .as_deref()
            .and_then(|name| self.params.samples.iter().position(|s| s.name == name))
            .unwrap_or(crate::audio_bus::NO_SOURCE);
        c.sample.store(sample_idx, Ordering::Relaxed);
        c.mode.store(preset.mode, Ordering::Relaxed);
        c.tune.store(preset.tune, Ordering::Relaxed);
        c.start.set(preset.start);
        c.loop_point.set(preset.loop_point);
        c.end.set(preset.end);
        c.num_slices.store(preset.num_slices.clamp(1, MAX_SLICES), Ordering::Relaxed);
        c.slice_mode.store(preset.slice_mode, Ordering::Relaxed);
        c.slice_step.store(preset.slice_step, Ordering::Relaxed);
        c.auto_clock.store(preset.auto_clock, Ordering::Relaxed);
        c.auto_rate_bpm.set(preset.auto_rate_bpm);
        c.env_attack.set(preset.env_attack);
        c.env_hold.set(preset.env_hold);
        c.env_decay.set(preset.env_decay);
        c.env_range.store(preset.env_range, Ordering::Relaxed);
        c.fx_type.store(preset.fx_type, Ordering::Relaxed);
        c.fx_param1.set(preset.fx_param1);
        c.fx_param2.set(preset.fx_param2);
        c.fx_mix.set(preset.fx_mix);
        c.volume.set(preset.volume);
        if sample_idx != crate::audio_bus::NO_SOURCE {
            self.warm_selected_sample(ch);
        }
    }

    fn load_preset(&self) {
        self.load_preset_from(Path::new(PRESETS_DIR));
    }

    /// The current channel's source-sample waveform (downsampled to
    /// `width` peaks) plus its slice cut points, recomputing only when
    /// the sample or slice layout has actually changed since the last
    /// call -- see `ChannelParams::source_waveform_cache`. This is
    /// what draws the "designate when the cuts are going to be,
    /// visually" slice view, both on the real device screen and in
    /// the Slint panel.
    fn source_waveform_and_slices(&self, ch: usize, width: usize) -> (Vec<f32>, Vec<(f32, f32)>) {
        let c = &self.params.channels[ch];
        let sample_idx = c.sample.load(Ordering::Relaxed);
        let start = (c.start.get() + c.ext_start.get()).clamp(0.0, 1.0);
        let end = (c.end.get() + c.ext_end.get()).clamp(0.0, 1.0);
        let num_slices = c.num_slices.load(Ordering::Relaxed);
        let slice_mode = c.slice_mode.load(Ordering::Relaxed);
        let key = (sample_idx, start.to_bits(), end.to_bits(), num_slices, slice_mode);

        let mut cache = c.source_waveform_cache.lock().unwrap();
        if cache.key != key {
            let Some(slot) = self.params.samples.get(sample_idx) else {
                *cache = SourceWaveformCache { key, waveform: Vec::new(), slice_bounds: Vec::new() };
                return (cache.waveform.clone(), cache.slice_bounds.clone());
            };
            let (data, _) = slot.decoded();
            let waveform = downsample_peaks(data, width);
            let slices = (0..num_slices.max(1)).map(|step| slice_bounds(data, start, end, num_slices, slice_mode == 1, step)).collect();
            *cache = SourceWaveformCache { key, waveform, slice_bounds: slices };
        }
        (cache.waveform.clone(), cache.slice_bounds.clone())
    }

    /// Renders the current channel's sample waveform with its slice
    /// cut points marked -- "designate when the cuts are going to be,
    /// visually", the thing this screen never had before. Sits in the
    /// gap between the menu list and the bottom hint line.
    fn draw_slice_view(&mut self, fb: &mut FrameBuffer) {
        let ch = self.ch();
        let (x0, y0, w, h) = (16i32, 292i32, (WIDTH as i32 - 32), 34i32);
        let (waveform, slices) = self.source_waveform_and_slices(ch, w as usize);

        Rectangle::new(Point::new(x0, y0), Size::new(w as u32, h as u32))
            .into_styled(PrimitiveStyle::with_stroke(SAMPLE_DRUM_DIM, 1))
            .draw(fb)
            .ok();

        if waveform.is_empty() {
            return;
        }

        let step_index = self.params.channels[ch].step_index.load(Ordering::Relaxed);
        let mid_y = y0 + h / 2;

        // The slice the next trigger will fire, highlighted first so
        // the waveform/tick marks draw on top of it.
        if let Some(&(lower, upper)) = slices.get(step_index) {
            let hx0 = x0 + (lower * w as f32) as i32;
            let hx1 = x0 + (upper * w as f32) as i32;
            Rectangle::new(Point::new(hx0, y0 + 1), Size::new((hx1 - hx0).max(1) as u32, (h - 2) as u32))
                .into_styled(PrimitiveStyle::with_fill(SAMPLE_DRUM_TITLE))
                .draw(fb)
                .ok();
        }

        for (i, &peak) in waveform.iter().enumerate() {
            let x = x0 + i as i32;
            let half = ((peak * (h as f32 / 2.0 - 1.0)) as i32).max(1);
            Line::new(Point::new(x, mid_y - half), Point::new(x, mid_y + half))
                .into_styled(PrimitiveStyle::with_stroke(SAMPLE_DRUM_ACCENT, 1))
                .draw(fb)
                .ok();
        }

        // One tick per slice boundary -- every slice's lower edge,
        // plus the final slice's own upper edge (the End point).
        for (i, &(lower, upper)) in slices.iter().enumerate() {
            let tx = x0 + (lower * w as f32) as i32;
            Line::new(Point::new(tx, y0), Point::new(tx, y0 + h)).into_styled(PrimitiveStyle::with_stroke(SAMPLE_DRUM_DIM, 1)).draw(fb).ok();
            if i == slices.len() - 1 {
                let ux = x0 + (upper * w as f32) as i32;
                Line::new(Point::new(ux, y0), Point::new(ux, y0 + h)).into_styled(PrimitiveStyle::with_stroke(SAMPLE_DRUM_DIM, 1)).draw(fb).ok();
            }
        }
    }
}

impl SampleDrumApp {
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

    pub(crate) fn output_visual(&self) -> crate::app::SampleDrumExtra {
        let ch = self.ch();
        let (source_waveform, slice_bounds) = self.source_waveform_and_slices(ch, 128);
        crate::app::SampleDrumExtra {
            channel: ch,
            sample_name: self.sample_name(ch),
            num_slices: self.params.channels[ch].num_slices.load(Ordering::Relaxed),
            step_index: self.params.channels[ch].step_index.load(Ordering::Relaxed),
            waveform: self.params.channels[ch].waveform_snapshot.lock().unwrap().clone(),
            source_waveform,
            slice_bounds,
        }
    }
}

impl App for SampleDrumApp {
    fn supports_pad_lock(&self) -> bool { true }

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

        // TRIG1/TRIG2 -- the real module's two manual trigger
        // buttons, mapped to the first two grid pads (this sim has no
        // dedicated trigger-button input beyond the grid). The rest
        // of the grid is unused, matching the real module's own
        // 2-trigger-button simplicity rather than inventing a use for
        // the other 14 pads.
        if input.grid[0] && !self.prev_grid[0] {
            self.params.channels[0].trig_pending.store(true, Ordering::Relaxed);
        }
        if input.grid[1] && !self.prev_grid[1] {
            self.params.channels[1].trig_pending.store(true, Ordering::Relaxed);
        }
        self.prev_grid = input.grid;
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
        crate::app::SlintExtra::SampleDrum(self.output_visual())
    }

    /// F3 (Start/Stop) starts/stops whichever channel is currently
    /// selected self-triggering at `AutoRate` -- same "whichever one
    /// you're actually looking at" convention Bloom's per-shape
    /// Running already uses, since Sample Drum's two channels are
    /// just as independent.
    fn running(&self) -> Option<bool> {
        Some(self.params.channels[self.ch()].auto_clock.load(Ordering::Relaxed))
    }

    fn toggle_running(&mut self) {
        let c = &self.params.channels[self.ch()];
        let cur = c.auto_clock.load(Ordering::Relaxed);
        c.auto_clock.store(!cur, Ordering::Relaxed);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(SampleDrumProcessor {
            params: Arc::clone(&self.params),
            voices: std::array::from_fn(|_| DrumVoice::default()),
            fx_biquad: std::array::from_fn(|_| Biquad::default()),
            fx_delay: std::array::from_fn(|_| DelayFx::default()),
            fx_reverb: std::array::from_fn(|_| ReverbFx::default()),
            fx_bitcrush: std::array::from_fn(|_| Bitcrush::default()),
            auto_clock_timer: [0.0; NUM_CHANNELS],
            prev_ext_trig: [false; NUM_CHANNELS],
            rng: 0x9E3779B9,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(SAMPLE_DRUM_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, SAMPLE_DRUM_TITLE);
        Text::new("Sample Drum", Point::new(16, 30), title).draw(fb).ok();

        let dim = MonoTextStyle::new(&SPLEEN_6X12, SAMPLE_DRUM_DIM);

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
        self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, SAMPLE_DRUM_BG, SAMPLE_DRUM_DIM, SAMPLE_DRUM_ACCENT);

        self.draw_slice_view(fb);

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

struct SampleDrumProcessor {
    params: Arc<Params>,
    voices: [DrumVoice; NUM_CHANNELS],
    fx_biquad: [Biquad; NUM_CHANNELS],
    fx_delay: [DelayFx; NUM_CHANNELS],
    fx_reverb: [ReverbFx; NUM_CHANNELS],
    fx_bitcrush: [Bitcrush; NUM_CHANNELS],
    /// Seconds remaining until `AutoClock`'s next self-trigger, per
    /// channel -- audio-thread-only, counts down every block and
    /// fires `trig_pending` itself on reaching zero. See
    /// `Selection::AutoClock`'s doc comment for why this exists.
    auto_clock_timer: [f32; NUM_CHANNELS],
    /// Last block's `ext_trig` gate state, per channel -- see
    /// `ChannelParams::ext_trig`'s doc comment for the rising-edge
    /// detection this drives.
    prev_ext_trig: [bool; NUM_CHANNELS],
    rng: u32,
}

impl SampleDrumProcessor {
    fn next_rand01(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }
}

impl AudioProcessor for SampleDrumProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        let mut mono_buf = vec![0.0f32; frames];

        for ch in 0..NUM_CHANNELS {
            // Drawn unconditionally (not just when RND stepping is
            // actually in use) so `c`'s borrow below never has to
            // span a call back into `self` -- see the borrow-checker
            // note this avoided.
            let rand01 = self.next_rand01();
            let c = &self.params.channels[ch];

            if c.auto_clock.load(Ordering::Relaxed) {
                let dt = frames as f32 / sample_rate;
                self.auto_clock_timer[ch] -= dt;
                if self.auto_clock_timer[ch] <= 0.0 {
                    c.trig_pending.store(true, Ordering::Relaxed);
                    // `AutoRate` is the *loop's* tempo (a 1-bar/4-beat
                    // loop, per the manual's own "prepare 1 bar loops,
                    // slice into 16, and trigger them on every clock
                    // tick to sync your loops to the BPM" workflow),
                    // not a fixed absolute trigger rate -- dividing by
                    // `num_slices` here is what makes re-chopping from
                    // 4 to 16 slices keep tiling the *same* bar
                    // seamlessly (4x as many, 4x shorter triggers)
                    // instead of leaving dead air between short clips
                    // (16 slices firing at the old 4-slice rate would
                    // each finish playing long before the next trigger
                    // arrived). At `num_slices == 4` this is exactly
                    // the previous `60.0 / bpm` -- unchanged.
                    let num_slices = c.num_slices.load(Ordering::Relaxed).max(1) as f32;
                    let period = (240.0 / c.auto_rate_bpm.get().max(1.0) / num_slices).max(0.02);
                    // `+=` (not `=`), so a block-length rounding
                    // remainder carries into the next period instead
                    // of quietly drifting the tempo late over time.
                    self.auto_clock_timer[ch] += period;
                }
            }

            // A rising edge on the external Trig CV (see
            // `ChannelParams::ext_trig`'s doc comment) fires this
            // channel exactly like a manual TRIG press -- lets Pam's
            // (Square/Euclidean shapes are gate-like, crossing this
            // 0.5 threshold on each pulse) clock Sample Drum the same
            // way patching a real external clock into the module's
            // TRIG jack would.
            let ext_trig_now = c.ext_trig.get() > 0.5;
            if ext_trig_now && !self.prev_ext_trig[ch] {
                c.trig_pending.store(true, Ordering::Relaxed);
            }
            self.prev_ext_trig[ch] = ext_trig_now;

            if c.trig_pending.swap(false, Ordering::Relaxed) {
                let sample_idx = c.sample.load(Ordering::Relaxed);
                if let Some(slot) = self.params.samples.get(sample_idx) {
                    let (data, _) = slot.decoded();
                    let start = (c.start.get() + c.ext_start.get()).clamp(0.0, 1.0);
                    let end = (c.end.get() + c.ext_end.get()).clamp(0.0, 1.0);
                    let num_slices = c.num_slices.load(Ordering::Relaxed);
                    let slice_step_mode = c.slice_step.load(Ordering::Relaxed);
                    let step = match slice_step_mode {
                        1 => (c.step_index.load(Ordering::Relaxed) + num_slices.max(1) - 1) % num_slices.max(1),
                        2 => (rand01 * num_slices.max(1) as f32) as usize % num_slices.max(1),
                        3 => 0,
                        4 => (c.ext_slice_cv.get().clamp(0.0, 1.0) * num_slices.max(1) as f32) as usize % num_slices.max(1),
                        _ => c.step_index.load(Ordering::Relaxed) % num_slices.max(1),
                    };
                    let (lower, upper) = slice_bounds(data, start, end, num_slices, c.slice_mode.load(Ordering::Relaxed) == 1, step);
                    let mode = c.mode.load(Ordering::Relaxed);
                    let backward = mode == 2 || mode == 3;
                    // Fwd Loop / Bwd Loop keep looping their region
                    // forever -- Forward/Backward are one-shot -- same
                    // meaning regardless of slicing (the manual treats
                    // Play Mode's loop-or-not as orthogonal to
                    // slicing). While sliced, each trigger re-fires
                    // `trigger()` on a (possibly new) slice, so a loop
                    // mode reads as "always running, trigger jumps the
                    // playhead to a different slice" -- it loops the
                    // *current* slice's own region (lower..upper, so
                    // `loop_target == lower`) until the next trigger
                    // arrives, rather than ever falling silent.
                    let looping = mode == 1 || mode == 3;
                    let loop_point = if num_slices <= 1 { c.loop_point.get() } else { lower };
                    self.voices[ch].trigger(lower, upper, loop_point, backward, looping, data.len());

                    // Advance the FWD/BKW step for next time -- RND
                    // and NONE ignore this state entirely.
                    if slice_step_mode == 0 {
                        c.step_index.store((step + 1) % num_slices.max(1), Ordering::Relaxed);
                    } else if slice_step_mode == 1 {
                        c.step_index.store(step, Ordering::Relaxed);
                    }
                }
            }

            let sample_idx = c.sample.load(Ordering::Relaxed);
            let Some(slot) = self.params.samples.get(sample_idx) else {
                *c.waveform_snapshot.lock().unwrap() = Vec::new();
                continue;
            };
            let (data, native_rate) = slot.decoded();
            let tune = c.tune.load(Ordering::Relaxed) + c.ext_tune.get().round() as i32;
            let range_max = ENV_RANGE_MAX_SECONDS[c.env_range.load(Ordering::Relaxed) as usize % 3];
            let attack = c.env_attack.get() * range_max;
            let hold = c.env_hold.get() * range_max;
            let decay = (c.env_decay.get() * range_max).max(0.02);
            let volume = c.volume.get();

            let fx_type = c.fx_type.load(Ordering::Relaxed);
            let fx_p1 = c.fx_param1.get();
            let fx_p2 = c.fx_param2.get();
            let fx_mix = c.fx_mix.get();

            let mut snapshot = Vec::with_capacity(96);
            let stride = (frames / 96).max(1);
            for (i, m) in mono_buf.iter_mut().enumerate() {
                let dry = self.voices[ch].render(data, native_rate, sample_rate, tune, attack, hold, decay, sample_rate);
                let wet = match fx_type {
                    1 => self.fx_delay[ch].process(dry, 60.0 + fx_p1 * 900.0, fx_p2, sample_rate),
                    2 => self.fx_reverb[ch].process(dry, fx_p1, fx_p2, sample_rate),
                    3 => {
                        self.fx_biquad[ch].set_lowpass(80.0 + fx_p1 * 8000.0, sample_rate, 0.5 + fx_p2 * 3.0);
                        self.fx_biquad[ch].process(dry)
                    }
                    4 => {
                        self.fx_biquad[ch].set_highpass(40.0 + fx_p1 * 6000.0, sample_rate, 0.5 + fx_p2 * 3.0);
                        self.fx_biquad[ch].process(dry)
                    }
                    5 => self.fx_bitcrush[ch].process(dry, 2.0 + (1.0 - fx_p1) * 14.0, 1 + (fx_p2 * 40.0) as u32),
                    6 => fold_fx(dry, fx_p1),
                    7 => drive_fx(dry, fx_p1),
                    _ => dry,
                };
                let out = (dry + (wet - dry) * fx_mix.clamp(0.0, 1.0)) * volume;
                *m += out;
                if i % stride == 0 && snapshot.len() < 96 {
                    snapshot.push(out.clamp(-1.5, 1.5));
                }
            }
            *c.waveform_snapshot.lock().unwrap() = snapshot;
        }

        {
            let mut bus_out = self.params.bus_out.lock().unwrap();
            bus_out.clear();
            bus_out.extend_from_slice(&mono_buf);
        }

        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        for (frame, sample) in buffer.chunks_mut(channels).zip(mono_buf.iter()) {
            for out in frame.iter_mut() {
                *out = *sample * mix_level;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> SampleDrumApp {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(3.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        SampleDrumApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus)
    }

    fn new_processor(params: Arc<Params>) -> SampleDrumProcessor {
        SampleDrumProcessor {
            params,
            voices: std::array::from_fn(|_| DrumVoice::default()),
            fx_biquad: std::array::from_fn(|_| Biquad::default()),
            fx_delay: std::array::from_fn(|_| DelayFx::default()),
            fx_reverb: std::array::from_fn(|_| ReverbFx::default()),
            fx_bitcrush: std::array::from_fn(|_| Bitcrush::default()),
            auto_clock_timer: [0.0; NUM_CHANNELS],
            prev_ext_trig: [false; NUM_CHANNELS],
            rng: 12345,
        }
    }

    #[test]
    fn no_sample_produces_silence() {
        let app = new_app();
        let mut proc = new_processor(Arc::clone(&app.params));
        let mut buffer = vec![1.0f32; 128 * 2];
        proc.process(&mut buffer, 2, 48000.0);
        assert!(buffer.iter().all(|&s| s == 0.0), "no sample assigned should mean silence, not leftover buffer contents");
    }

    #[test]
    fn triggering_a_real_sample_produces_output() {
        let app = new_app();
        assert!(!app.params.samples.is_empty(), "expected the real samples/ directory to have at least one sample for this test to mean anything");
        app.params.channels[0].sample.store(0, Ordering::Relaxed);
        app.params.channels[0].env_decay.set(1.0);
        app.params.channels[0].trig_pending.store(true, Ordering::Relaxed);
        app.params.mix_level.set(1.0);

        let mut proc = new_processor(Arc::clone(&app.params));
        let mut buffer = vec![0.0f32; 512 * 2];
        proc.process(&mut buffer, 2, 48000.0);

        assert!(buffer.iter().any(|&s| s != 0.0), "triggering a real sample should produce real audio output");
        assert!(!app.params.channels[0].trig_pending.load(Ordering::Relaxed), "trig_pending must be consumed (one-shot), not left set");
    }

    /// Auto Clock must actually self-trigger without any manual
    /// TRIG press -- the whole point of it (see `Selection::
    /// AutoClock`'s doc comment): combined with RND step mode, this
    /// is what lets a chopped break keep randomizing itself into new
    /// rhythms hands-free, the way a real patched-in external clock
    /// would on the real module.
    #[test]
    fn auto_clock_self_triggers_a_real_sample_with_no_manual_press() {
        let app = new_app();
        assert!(!app.params.samples.is_empty());
        let c = &app.params.channels[0];
        c.sample.store(0, Ordering::Relaxed);
        c.env_decay.set(1.0);
        c.auto_clock.store(true, Ordering::Relaxed);
        c.auto_rate_bpm.set(600.0); // fast enough that one block-worth of audio comfortably crosses a period
        app.params.mix_level.set(1.0);

        let mut proc = new_processor(Arc::clone(&app.params));
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut ever_sounded = false;
        for _ in 0..40 {
            proc.process(&mut buffer, 2, 48000.0);
            if buffer.iter().any(|&s| s != 0.0) {
                ever_sounded = true;
                break;
            }
        }
        assert!(ever_sounded, "Auto Clock should have self-triggered real audio output without any manual TRIG press");
    }

    /// Forward Loop must actually reach the loop-back behavior: play
    /// Start->End once, then keep repeating Loop->End -- not just
    /// stop at End like a plain one-shot would.
    #[test]
    fn forward_loop_repeats_between_loop_point_and_end_instead_of_stopping() {
        let data: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.1).sin()).collect();
        let mut voice = DrumVoice::default();
        voice.trigger(0.0, 1.0, 0.5, false, true, data.len());

        let mut max_pos = 0.0f32;
        for _ in 0..20000 {
            voice.render(&data, 48000.0, 48000.0, 0, 0.0, 0.0, 100.0, 48000.0); // huge decay so the envelope never ends this test early
            max_pos = max_pos.max(voice.pos);
            assert!(voice.playing, "a looping voice with an effectively infinite decay must never stop on its own");
        }
        // After many loops, position must stay bounded near the
        // sample's upper region -- if looping were broken (e.g. still
        // treating this as one-shot), `playing` would already be
        // false, caught above; this also checks it never runs past
        // the buffer.
        assert!(max_pos <= (data.len() - 1) as f32, "position must never exceed the buffer");
    }

    /// The float-rounding lesson from tonestack.rs/sequencer.rs's own
    /// delay-line bugs, generalized: run every FX type and every play
    /// mode over many blocks and confirm it never panics (an index-
    /// out-of-bounds from position math would show up here, not in a
    /// short single-block test).
    #[test]
    fn stress_every_fx_and_play_mode_over_many_blocks_never_panics() {
        let app = new_app();
        assert!(!app.params.samples.is_empty());
        app.params.channels[0].sample.store(0, Ordering::Relaxed);
        app.params.channels[0].num_slices.store(8, Ordering::Relaxed);
        app.params.channels[0].slice_mode.store(1, Ordering::Relaxed); // zero-crossing
        app.params.mix_level.set(1.0);

        let mut proc = new_processor(Arc::clone(&app.params));
        let mut buffer = vec![0.0f32; 256 * 2];
        for mode in 0..4u32 {
            app.params.channels[0].mode.store(mode, Ordering::Relaxed);
            for fx in 0..FX_NAMES.len() as u32 {
                app.params.channels[0].fx_type.store(fx, Ordering::Relaxed);
                app.params.channels[0].fx_mix.set(0.7);
                for step_mode in 0..SLICE_STEP_NAMES.len() as u32 {
                    app.params.channels[0].slice_step.store(step_mode, Ordering::Relaxed);
                    for _ in 0..40 {
                        app.params.channels[0].trig_pending.store(true, Ordering::Relaxed);
                        for _ in 0..5 {
                            proc.process(&mut buffer, 2, 48000.0);
                        }
                    }
                }
            }
        }
    }

    /// `slice_bounds` must split `[start, end]` into exactly
    /// `num_slices` equal, contiguous, non-overlapping pieces in
    /// Linear mode.
    #[test]
    fn slice_bounds_splits_evenly_in_linear_mode() {
        let data = vec![0.0f32; 100];
        let (l0, u0) = slice_bounds(&data, 0.0, 1.0, 4, false, 0);
        let (l1, u1) = slice_bounds(&data, 0.0, 1.0, 4, false, 1);
        assert!((l0 - 0.0).abs() < 1e-6 && (u0 - 0.25).abs() < 1e-6);
        assert!((l1 - 0.25).abs() < 1e-6 && (u1 - 0.5).abs() < 1e-6);
    }

    /// Saving then loading a preset must round-trip every field,
    /// matched by sample name, same convention `sequencer.rs`'s Kit
    /// feature already established (and the same reason: raw flat
    /// indices are fragile across library changes, names aren't).
    #[test]
    fn saving_and_loading_a_preset_round_trips_every_field() {
        let scratch_root = std::env::temp_dir().join(format!("portamax_sample_drum_preset_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch_root);

        let mut app = new_app();
        assert!(!app.params.samples.is_empty());
        let c = &app.params.channels[0];
        c.sample.store(0, Ordering::Relaxed);
        c.mode.store(1, Ordering::Relaxed);
        c.tune.store(-5, Ordering::Relaxed);
        c.start.set(0.1);
        c.loop_point.set(0.2);
        c.end.set(0.9);
        c.num_slices.store(6, Ordering::Relaxed);
        c.fx_type.store(3, Ordering::Relaxed);
        c.volume.set(0.42);

        app.save_preset_to(&scratch_root);

        c.sample.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed);
        c.mode.store(0, Ordering::Relaxed);
        c.tune.store(0, Ordering::Relaxed);
        c.start.set(0.0);
        c.num_slices.store(1, Ordering::Relaxed);
        c.fx_type.store(0, Ordering::Relaxed);
        c.volume.set(0.8);

        app.load_preset_from(&scratch_root);

        let c = &app.params.channels[0];
        assert_eq!(c.sample.load(Ordering::Relaxed), 0);
        assert_eq!(c.mode.load(Ordering::Relaxed), 1);
        assert_eq!(c.tune.load(Ordering::Relaxed), -5);
        assert_eq!(c.start.get(), 0.1);
        assert_eq!(c.loop_point.get(), 0.2);
        assert_eq!(c.end.get(), 0.9);
        assert_eq!(c.num_slices.load(Ordering::Relaxed), 6);
        assert_eq!(c.fx_type.load(Ordering::Relaxed), 3);
        assert_eq!(c.volume.get(), 0.42);

        let _ = std::fs::remove_dir_all(&scratch_root);
    }

    /// A loop mode must keep looping even while slicing is active --
    /// this used to be explicitly disabled; it's now the whole point
    /// of "play the sample in a loop while I can see/trigger slices".
    #[test]
    fn loop_mode_keeps_looping_a_slice_instead_of_stopping() {
        let app = new_app();
        assert!(!app.params.samples.is_empty());
        let c = &app.params.channels[0];
        c.sample.store(0, Ordering::Relaxed);
        c.mode.store(1, Ordering::Relaxed); // Fwd Loop
        c.num_slices.store(4, Ordering::Relaxed);
        c.env_decay.set(1.0);
        app.params.mix_level.set(1.0);

        let mut proc = new_processor(Arc::clone(&app.params));
        c.trig_pending.store(true, Ordering::Relaxed);
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..200 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        assert!(proc.voices[0].playing, "a looping slice must never stop on its own while no new trigger has arrived");
    }

    /// The bug this fixes: Backward (mode 2, one-shot) used to loop
    /// forever when unsliced, and Backward Loop (mode 3) used to never
    /// loop at all -- `looping` must track the mode's own name, not an
    /// accidental direction/slicing coincidence.
    #[test]
    fn backward_loop_actually_loops_and_plain_backward_does_not() {
        let data: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.1).sin()).collect();

        let mut looped = DrumVoice::default();
        looped.trigger(0.0, 1.0, 0.5, true, true, data.len()); // Backward Loop
        for _ in 0..20000 {
            looped.render(&data, 48000.0, 48000.0, 0, 0.0, 0.0, 100.0, 48000.0);
        }
        assert!(looped.playing, "Backward Loop must keep looping");

        let mut one_shot = DrumVoice::default();
        one_shot.trigger(0.0, 1.0, 0.5, true, false, data.len()); // plain Backward
        for _ in 0..20000 {
            one_shot.render(&data, 48000.0, 48000.0, 0, 0.0, 0.0, 100.0, 48000.0);
        }
        assert!(!one_shot.playing, "plain Backward must stop at its lower bound instead of looping forever");
    }

    /// CV step mode must pick the slice the routed CV points at, not
    /// just advance/repeat like FWD/NONE would.
    #[test]
    fn cv_step_mode_selects_the_slice_the_routed_cv_points_at() {
        let app = new_app();
        assert!(!app.params.samples.is_empty());
        let c = &app.params.channels[0];
        c.sample.store(0, Ordering::Relaxed);
        c.num_slices.store(4, Ordering::Relaxed);
        c.slice_step.store(4, Ordering::Relaxed); // CV
        c.env_decay.set(1.0);
        c.ext_slice_cv.set(0.9); // should land on the last slice (index 3 of 4)
        app.params.mix_level.set(1.0);

        let mut proc = new_processor(Arc::clone(&app.params));
        c.trig_pending.store(true, Ordering::Relaxed);
        let mut buffer = vec![0.0f32; 64 * 2];
        proc.process(&mut buffer, 2, 48000.0);

        let voice = &proc.voices[0];
        let (data, _) = app.params.samples[0].decoded();
        let (expected_lower, expected_upper) = slice_bounds(data, 0.0, 1.0, 4, false, 3);
        let len_f = (data.len().max(2) - 1) as f32;
        assert!((voice.lower - expected_lower * len_f).abs() < 1.0, "CV=0.9 over 4 slices should trigger slice index 3, got lower={}", voice.lower);
        let _ = expected_upper;
    }

    /// F3 (Start/Stop) must reflect and control whichever channel is
    /// currently selected, not always channel 0.
    #[test]
    fn f3_running_toggles_auto_clock_on_the_selected_channel() {
        let mut app = new_app();
        assert_eq!(app.running(), Some(false));
        app.toggle_running();
        assert_eq!(app.running(), Some(true));
        assert!(app.params.channels[0].auto_clock.load(Ordering::Relaxed));
        assert!(!app.params.channels[1].auto_clock.load(Ordering::Relaxed), "toggling channel 0's Start/Stop must not affect channel 1");

        app.params.channel.store(1, Ordering::Relaxed);
        assert_eq!(app.running(), Some(false), "channel 1 was never toggled, so F3 must report it as stopped");
        app.toggle_running();
        assert!(app.params.channels[1].auto_clock.load(Ordering::Relaxed));
    }

    /// The whole point of scaling AutoClock's period by `NumSlices`
    /// (see `SampleDrumProcessor::process`): re-chopping the same loop
    /// from 4 to 16 slices must trigger roughly 4x as often in the
    /// same wall-clock time, so the same bar keeps tiling seamlessly
    /// end to end instead of leaving dead air between now-much-
    /// shorter clips.
    #[test]
    fn auto_clock_rate_scales_with_slice_count_so_16_reacts_like_4() {
        // `trig_pending` gets consumed within the same `process()`
        // call that sets it, so it can never be observed from the
        // outside -- count real trigger events instead by watching
        // `step_index` (FWD stepping) change, which happens exactly
        // once per trigger and always differs from the previous value
        // when `num_slices >= 2`.
        fn count_triggers(num_slices: usize, blocks: usize) -> u32 {
            let app = new_app();
            assert!(!app.params.samples.is_empty());
            let c = &app.params.channels[0];
            c.sample.store(0, Ordering::Relaxed);
            c.auto_clock.store(true, Ordering::Relaxed);
            c.auto_rate_bpm.set(120.0);
            c.num_slices.store(num_slices, Ordering::Relaxed);
            let mut proc = new_processor(Arc::clone(&app.params));
            let mut buffer = vec![0.0f32; 128 * 2];
            let mut triggers = 0;
            let mut prev_step = c.step_index.load(Ordering::Relaxed);
            for _ in 0..blocks {
                proc.process(&mut buffer, 2, 48000.0);
                let cur_step = app.params.channels[0].step_index.load(Ordering::Relaxed);
                if cur_step != prev_step {
                    triggers += 1;
                    prev_step = cur_step;
                }
            }
            triggers
        }

        let at_4 = count_triggers(4, 4000);
        let at_16 = count_triggers(16, 4000);
        assert!(at_4 > 0 && at_16 > 0, "expected both slice counts to trigger at all over this many blocks");
        let ratio = at_16 as f32 / at_4 as f32;
        assert!((ratio - 4.0).abs() < 0.5, "16 slices should trigger ~4x as often as 4 slices at the same AutoRate/BPM, got ratio {ratio} ({at_16} vs {at_4})");
    }

    /// A rising edge on `ext_trig` (a Pam's channel or anything else
    /// routed via ModBus) must fire this channel exactly like a manual
    /// TRIG press -- this is what makes it "triggerable by Pam's and
    /// the like".
    #[test]
    fn external_trig_cv_rising_edge_fires_a_trigger() {
        let app = new_app();
        assert!(!app.params.samples.is_empty());
        let c = &app.params.channels[0];
        c.sample.store(0, Ordering::Relaxed);
        c.env_decay.set(1.0);
        app.params.mix_level.set(1.0);

        let mut proc = new_processor(Arc::clone(&app.params));
        let mut buffer = vec![0.0f32; 128 * 2];

        c.ext_trig.set(0.0);
        proc.process(&mut buffer, 2, 48000.0);
        assert!(buffer.iter().all(|&s| s == 0.0), "a low ext_trig must not fire anything");

        c.ext_trig.set(1.0); // rising edge
        proc.process(&mut buffer, 2, 48000.0);
        assert!(buffer.iter().any(|&s| s != 0.0), "a rising edge on ext_trig should fire a trigger and produce audio, same as a manual TRIG press");
    }

    #[test]
    fn loading_a_never_saved_preset_slot_does_not_panic_or_change_state() {
        let scratch_root = std::env::temp_dir().join(format!("portamax_sample_drum_missing_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch_root);
        let mut app = new_app();
        app.params.channels[0].tune.store(7, Ordering::Relaxed);

        app.load_preset_from(&scratch_root); // nothing was ever saved here

        assert_eq!(app.params.channels[0].tune.load(Ordering::Relaxed), 7, "a missing preset file must leave the channel untouched");
    }
}
