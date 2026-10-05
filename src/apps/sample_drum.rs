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
//! workflow) and SD card/RAM/firmware-update/SINGLE-DOUBLE-project
//! mechanics (no removable storage to simulate).
//!
//! Also here: fine tuning in cents, pitched playback from notes (see
//! `SampleDrumApp::play_notes`), the envelope's Relative range and A/D curve
//! shapes, and a short fade on a retriggered voice so hits don't click.

use crate::app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes, Throw};
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
use embedded_graphics::pixelcolor::Rgb565;
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
const ENV_RANGE_NAMES: [&str; 4] = ["Short", "Mid", "Long", "Relative"];
const NOTE_TARGET_NAMES: [&str; 3] = ["Ch 1", "Ch 2", "Both"];
/// The note that plays a sample at its own pitch (C4).
const ROOT_NOTE: i32 = 60;
/// Loop lengths, in bars, the Loop Bars row steps through.
const LOOP_BARS: [u32; 4] = [1, 2, 4, 8];

/// A tempo written into a sample's name ("..._174_AmenBreak"), if any.
fn tempo_in_name(name: &str) -> Option<f32> {
    name.split(|c: char| !c.is_ascii_digit()).filter_map(|t| t.parse::<u32>().ok()).find(|t| (60..=240).contains(t)).map(|t| t as f32)
}

/// How many bars a loop of `seconds` most likely is: from a tempo in its name when
/// there is one, otherwise whichever length puts it nearest the project tempo.
fn guess_loop_bars(name: &str, seconds: f32, project_bpm: f32, beats_per_bar: f32) -> u32 {
    let bpm_of = |bars: u32| bars as f32 * beats_per_bar * 60.0 / seconds.max(0.01);
    let aim = tempo_in_name(name).unwrap_or(project_bpm);
    *LOOP_BARS.iter().min_by(|a, b| (bpm_of(**a) - aim).abs().total_cmp(&(bpm_of(**b) - aim).abs())).unwrap_or(&1)
}
/// Max Attack/Hold/Decay time in seconds per range preset -- matches
/// the manual's own SHORT (1s)/MID (3s)/LONG (10s) figures.
const ENV_RANGE_MAX_SECONDS: [f32; 4] = [1.0, 3.0, 10.0, 1.0];
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

/// A real Attack-Hold-Decay envelope: Short/Mid/Long time ranges or a Relative
/// one scaled to the region, with the manual's curve-shape morphing.
#[derive(Default)]
struct Envelope {
    state: u8, // 0 idle, 1 attack, 2 hold, 3 decay
    level: f32,
    elapsed: f32,
    /// The level a decay starts from -- 1.0 after a full hold, lower when a
    /// note-off cuts the envelope short.
    decay_from: f32,
}

/// Curve exponent for an envelope shape in -1..1. 0 is linear; the manual's
/// -50 (exponential, convex) and +50 (logarithmic, concave) ends are 4 and 0.25.
fn shape_exponent(shape: f32) -> f32 {
    2f32.powf(-shape.clamp(-1.0, 1.0) * 2.0)
}

impl Envelope {
    fn trigger(&mut self) {
        self.state = 1;
        self.elapsed = 0.0;
        self.level = 0.0;
        self.decay_from = 1.0;
    }

    /// A note-off: skip straight to the decay, from wherever the envelope is.
    fn release(&mut self) {
        if self.state == 1 || self.state == 2 {
            self.decay_from = self.level;
            self.state = 3;
            self.elapsed = 0.0;
        }
    }

    fn tick(&mut self, attack_s: f32, hold_s: f32, decay_s: f32, a_shape: f32, d_shape: f32, dt: f32) -> f32 {
        match self.state {
            1 => {
                self.elapsed += dt;
                let x = (self.elapsed / attack_s.max(0.001)).min(1.0);
                self.level = x.powf(shape_exponent(a_shape));
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
                    self.decay_from = 1.0;
                }
            }
            3 => {
                self.elapsed += dt;
                let x = (1.0 - self.elapsed / decay_s.max(0.001)).max(0.0);
                self.level = self.decay_from * x.powf(shape_exponent(d_shape));
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

/// Where a voice is reading its sample, and how it moves: the position, the
/// direction, the playing region, and the loop point it wraps to.
#[derive(Default, Clone, Copy)]
struct Head {
    pos: f32,
    dir: f32,
    lower: f32,
    upper: f32,
    loop_target: f32,
    looping: bool,
}

impl Head {
    fn read(&self, data: &[f32]) -> f32 {
        // `pos` is float math driven by several independently-clamped
        // fractions; never trust it to stay in range on its own --
        // clamp the cast every time.
        let i = (self.pos as usize).min(data.len() - 2);
        let frac = (self.pos - i as f32).clamp(0.0, 1.0);
        data[i] + (data[i + 1] - data[i]) * frac
    }

    /// Moves one output sample on; false when a one-shot ran off its region.
    fn advance(&mut self, rate: f32) -> bool {
        self.pos += rate.max(0.01) * self.dir;
        if self.dir > 0.0 && self.pos >= self.upper {
            if self.looping {
                self.pos = self.loop_target + (self.pos - self.upper);
            } else {
                return false;
            }
        } else if self.dir < 0.0 && self.pos <= self.lower {
            if self.looping {
                self.pos = self.loop_target - (self.lower - self.pos);
            } else {
                return false;
            }
        }
        true
    }
}

/// How long the cut-off tail of a retriggered voice takes to fade, in
/// samples (~2 ms): a hard cut mid-waveform is a click.
const RETRIGGER_FADE: f32 = 96.0;

/// One channel's sample-playback voice -- real Start/Loop/End
/// behavior for all 4 play modes (see module doc comment), envelope-
/// gated, with the same linear-interpolation resampling every other
/// sample-based app in this build uses. A retrigger lets the old playhead
/// fade out under the new one instead of cutting it.
#[derive(Default)]
struct DrumVoice {
    head: Head,
    ghost: Head,
    ghost_gain: f32,
    ghost_env: f32,
    /// The new playhead's fade-in after a retrigger (1 when it started from silence).
    onset: f32,
    playing: bool,
    env: Envelope,
    /// Length of the region being played, in sample frames -- what the
    /// envelope's Relative range scales to.
    region: f32,
}

impl DrumVoice {
    /// `lower_frac`/`upper_frac`/`loop_frac` are 0..1 of the sample's
    /// total length (already resolved to whichever slice is playing,
    /// if slicing is active -- see `SampleDrumProcessor::process`).
    /// `backward` picks which end playback starts from; `looping`
    /// makes it wrap between `loop_frac` and the far end instead of
    /// stopping there.
    fn trigger(&mut self, lower_frac: f32, upper_frac: f32, loop_frac: f32, backward: bool, looping: bool, len: usize) {
        self.onset = 1.0;
        if self.playing {
            self.onset = 0.0;
            self.ghost = self.head;
            self.ghost_gain = 1.0;
            self.ghost_env = self.env.level;
        }
        let len_f = (len.max(2) - 1) as f32;
        let lower = lower_frac.clamp(0.0, 1.0) * len_f;
        let upper = (upper_frac.clamp(0.0, 1.0) * len_f).max(lower + 1.0).min(len_f);
        self.region = upper - lower;
        self.head = Head {
            lower,
            upper,
            loop_target: (loop_frac.clamp(0.0, 1.0) * len_f).clamp(lower, upper),
            dir: if backward { -1.0 } else { 1.0 },
            pos: if backward { upper } else { lower },
            looping,
        };
        self.playing = true;
        self.env.trigger();
    }

    /// Note-off: a looping voice fades out through its decay.
    fn release(&mut self) {
        if self.playing {
            self.env.release();
        }
    }

    /// `rel_len_s`, when set, is what the envelope's times are fractions of
    /// (its Relative range); otherwise they're the seconds in `env`.
    fn render(&mut self, data: &[f32], native_rate: f32, device_rate: f32, semitones: f32, env: EnvSettings, sample_rate: f32) -> f32 {
        if (!self.playing && self.ghost_gain <= 0.0) || data.len() < 2 {
            return 0.0;
        }
        let rate = (native_rate / device_rate) * 2f32.powf(semitones / 12.0);
        let mut out = 0.0;
        if self.ghost_gain > 0.0 {
            out += self.head_ghost(data, rate);
        }
        if self.playing {
            let (attack, hold, decay) = match env.relative {
                true => {
                    let len_s = self.region / (rate.max(0.01) * sample_rate);
                    (env.attack * len_s, env.hold * len_s, env.decay * len_s)
                }
                false => (env.attack, env.hold, env.decay),
            };
            let gain = self.env.tick(attack, hold, decay, env.a_shape, env.d_shape, 1.0 / sample_rate);
            out += self.head.read(data) * gain * self.onset;
            self.onset = (self.onset + 1.0 / RETRIGGER_FADE).min(1.0);
            if !self.env.active() || !self.head.advance(rate) {
                self.playing = false;
            }
        }
        out
    }

    fn head_ghost(&mut self, data: &[f32], rate: f32) -> f32 {
        let out = self.ghost.read(data) * self.ghost_gain * self.ghost_env;
        self.ghost_gain -= 1.0 / RETRIGGER_FADE;
        if !self.ghost.advance(rate) {
            self.ghost_gain = 0.0;
        }
        out
    }
}

/// One block's envelope settings for a channel, resolved to seconds (or, for
/// the Relative range, to fractions of the region).
#[derive(Clone, Copy)]
struct EnvSettings {
    attack: f32,
    hold: f32,
    decay: f32,
    a_shape: f32,
    d_shape: f32,
    relative: bool,
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
    #[serde(default)]
    fine: i32,
    #[serde(default)]
    tempo_match: bool,
    #[serde(default = "default_one")]
    loop_bars: u32,
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
    #[serde(default)]
    env_ashape: f32,
    #[serde(default)]
    env_dshape: f32,
    fx_type: u32,
    fx_param1: f32,
    fx_param2: f32,
    fx_mix: f32,
    volume: f32,
}

fn default_one() -> u32 {
    1
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
    NotesTo,
    Sample,
    Mode,
    Tune,
    FineTune,
    TempoMatch,
    LoopBars,
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
    EnvAShape,
    EnvDShape,
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
    /// Fine tune in cents (the real module's TRG2/SHIFT + tune encoder).
    fine: AtomicI32,
    /// Pitch of the last note played into this channel, in semitones from C4 --
    /// held like a keyboard's 1V/oct CV, so later triggers keep it.
    note_semi: AtomicF32,
    note_gain: AtomicF32,
    /// Plays the sample faster or slower so its loop (`loop_bars` long) fits the
    /// project tempo: speed-matching, like turning a record's pitch control, so
    /// pitch moves with it. Auto Clock then fires on the project's bar grid.
    tempo_match: AtomicBool,
    loop_bars: AtomicU32,
    /// A note-off arrived: looping voices fade out.
    release_pending: AtomicBool,
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
    /// A slice a pad asked for (`usize::MAX`: none). The next trigger plays that
    /// slice instead of stepping.
    slice_request: AtomicUsize,
    /// The slice most recently played, for the pads' lights.
    last_slice: AtomicUsize,
    /// The slice index the *next* trigger will play for FWD/BKW
    /// stepping -- audio-thread-owned, but exposed for the UI's
    /// "current slice" display.
    step_index: AtomicUsize,
    env_attack: AtomicF32,
    env_hold: AtomicF32,
    env_decay: AtomicF32,
    env_range: AtomicU32,
    env_ashape: AtomicF32,
    env_dshape: AtomicF32,
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
            fine: AtomicI32::new(0),
            note_semi: AtomicF32::new(0.0),
            note_gain: AtomicF32::new(1.0),
            tempo_match: AtomicBool::new(false),
            loop_bars: AtomicU32::new(1),
            release_pending: AtomicBool::new(false),
            start: AtomicF32::new(0.0),
            loop_point: AtomicF32::new(0.0),
            end: AtomicF32::new(1.0),
            num_slices: AtomicUsize::new(1),
            slice_mode: AtomicU32::new(0),
            slice_step: AtomicU32::new(0),
            auto_clock: AtomicBool::new(false),
            auto_rate_bpm: AtomicF32::new(120.0),
            trig_pending: AtomicBool::new(false),
            slice_request: AtomicUsize::new(usize::MAX),
            last_slice: AtomicUsize::new(usize::MAX),
            step_index: AtomicUsize::new(0),
            env_attack: AtomicF32::new(0.0),
            env_hold: AtomicF32::new(0.0),
            env_decay: AtomicF32::new(0.3),
            env_range: AtomicU32::new(1),
            env_ashape: AtomicF32::new(0.0),
            env_dshape: AtomicF32::new(0.0),
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
    /// Which channel(s) incoming notes play: 0 = 1, 1 = 2, 2 = both.
    note_target: AtomicU32,
    /// The shared transport: its tempo is what Tempo Match fits samples to.
    clock: Arc<crate::clock::Clock>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let samples = scan_samples(Path::new(SAMPLES_DIR));
        let (mix_level, ext_mix_level) = mixer_bus.register("Sample Drum", modbus);
        let channels: [ChannelParams; NUM_CHANNELS] = std::array::from_fn(|i| ChannelParams::new(i, modbus));
        // Open with a sample on each channel, so the first pad press makes a sound.
        for (i, c) in channels.iter().enumerate() {
            if !samples.is_empty() {
                c.sample.store(i.min(samples.len() - 1), Ordering::Relaxed);
            }
        }
        Self {
            channel: AtomicUsize::new(0),
            channels,
            samples,
            preset_slot: AtomicUsize::new(0),
            note_target: AtomicU32::new(0),
            clock: crate::clock::Clock::shared(),
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
    prev_keys: [u8; 128],
    /// The shared play view (play_kit.rs).
    kit: PlayKit,
}

/// The play view's controls, most important first. Like every menu row
/// here they act on the selected channel. The slice window (Start/End)
/// and the envelope's decay are what shape a chopped break most, so they
/// lead; FX and the auto-clock tempo follow; the stepped choices (sample,
/// slicing, play mode) sit on the upper Controls pads.
const CONTROLS: [(Selection, &str); 16] = [
    (Selection::Start, "Start"),
    (Selection::End, "End"),
    (Selection::EnvDecay, "Decay"),
    (Selection::FxMix, "FX Mix"),
    (Selection::FxParam1, "FX Param 1"),
    (Selection::FxParam2, "FX Param 2"),
    (Selection::AutoRate, "Auto Rate"),
    (Selection::Volume, "Volume"),
    (Selection::Sample, "Sample"),
    (Selection::NumSlices, "Slices"),
    (Selection::SliceStep, "Step"),
    (Selection::FxType, "FX"),
    (Selection::Mode, "Mode"),
    (Selection::Tune, "Tune"),
    (Selection::EnvAttack, "Attack"),
    (Selection::Channel, "Channel"),
];
const C_DECAY: usize = 2;
const C_FX_MIX: usize = 3;
const C_FX_P1: usize = 4;
const C_SAMPLE: usize = 8;
const C_SLICES: usize = 9;
const C_STEP: usize = 10;
const C_MODE: usize = 12;
const C_TUNE: usize = 13;
const C_ATTACK: usize = 14;
const C_CHANNEL: usize = 15;
/// The menu's Tune row is unbounded; this only bounds the dial's
/// position and what moments/throws can set (two octaves each way
/// covers every musically useful repitch of a drum hit).
const TUNE_SPAN: i32 = 24;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "sample_drum",
        // TRIG1/TRIG2 on pads 0/1, exactly as before.
        // TRIG: the module's two trigger buttons. SLICES: one pad per slice of the
        // selected channel, so a chopped break is played like a drum kit (SLICES
        // 17+ is the second page, for up to 32 slices).
        layers: vec![Layer::Native(0, "TRIG"), Layer::Native(1, "SLICES"), Layer::Native(2, "SLICES 17+"), Layer::Controls, Layer::Moments, Layer::Throws],
        hero: vec![[0, 1], [2, 3], [4, 5], [6, 7]],
        // Flipping through samples is what a sample player's encoder
        // does most.
        browse: Some(C_SAMPLE),
        // Stick X scrubs where the slice window starts (the classic
        // "scan through a break" move), Y sweeps the insert FX's first
        // parameter (cutoff, delay time, drive...). Hands: decay (gate
        // the hits open) and how wet the FX is.
        routes: Routes { stick_x: Some(0), stick_y: Some(C_FX_P1), hand_l: Some(C_DECAY), hand_r: Some(C_FX_MIX) },
        // Most alive with the auto clock running (F3): hold a pad to bend
        // the chopped loop, let go and it snaps back.
        throws: vec![
            // SLICE_STEP_NAMES index 2 = RND, of 5 choices.
            Throw { control: C_STEP, to: 0.5, label: "RANDOM" },
            Throw { control: C_DECAY, to: 0.03, label: "CHOKE" },
            Throw { control: C_DECAY, to: 1.0, label: "OPEN" },
            Throw { control: C_FX_MIX, to: 1.0, label: "FX WET" },
            // -24 + 0.75 * 48 = +12 st; 0.25 = -12 st.
            Throw { control: C_TUNE, to: 0.75, label: "OCT UP" },
            Throw { control: C_TUNE, to: 0.25, label: "OCT DOWN" },
            // PLAY_MODE_NAMES index 2 = Backward, of 4.
            Throw { control: C_MODE, to: 2.0 / 3.0, label: "BACKWARD" },
            Throw { control: C_FX_P1, to: 0.0, label: "FX P1 MIN" },
        ],
        // Notes play the sample at pitch (see `SampleDrumApp::play_notes`)
        // instead of landing on the two trigger pads.
        midi_to_pads: false,
        own_expression: false,
    }
}

// --- Sample Drum's own palette: flat solid colors, not a
// device-wide theme -- Warm hardware-chassis grey with a bold sampler-pad orange -- gritty MPC/boom-bap hardware energy. ---

const SAMPLE_DRUM_BG: Rgb565 = Rgb565::new(4, 7, 3);
const SAMPLE_DRUM_TITLE: Rgb565 = Rgb565::new(29, 58, 28);
const SAMPLE_DRUM_ACCENT: Rgb565 = Rgb565::new(31, 26, 3);
const SAMPLE_DRUM_DIM: Rgb565 = Rgb565::new(15, 28, 13);

impl SampleDrumApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        let app = Self {
            params,
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
            prev_grid: [false; 16],
            prev_keys: [0; 128],
            kit: PlayKit::new(kit_config(), !cfg!(test)),
        };
        for ch in 0..NUM_CHANNELS {
            app.guess_bars(ch);
        }
        app
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

    /// Notes from a keyboard, a sequencer or the note bus play the sample at
    /// pitch, like a 1V/oct CV and a trigger patched in: C4 is the sample's own
    /// pitch, velocity sets the level, and the channel(s) it goes to is the
    /// "Notes Play" row. Drum samples ignore the note-off; looping ones fade out
    /// through their decay when the key is let go.
    fn play_notes(&mut self, keys: &[u8; 128]) {
        let target = self.params.note_target.load(Ordering::Relaxed) % 3;
        let channels: &[usize] = match target {
            0 => &[0],
            1 => &[1],
            _ => &[0, 1],
        };
        let mut released = false;
        let mut newest: Option<usize> = None;
        for n in 0..128 {
            let (now, before) = (keys[n], self.prev_keys[n]);
            if now > 0 && before == 0 {
                newest = Some(n);
            } else if now == 0 && before > 0 {
                released = true;
            }
        }
        if let Some(n) = newest {
            let vel = keys[n] as f32 / 127.0;
            for &ch in channels {
                let c = &self.params.channels[ch];
                c.note_semi.set((n as i32 - ROOT_NOTE) as f32);
                // A curve that keeps soft notes audible but still plays dynamics.
                c.note_gain.set(0.25 + 0.75 * vel);
                c.trig_pending.store(true, Ordering::Relaxed);
            }
        }
        if released && keys.iter().all(|&k| k == 0) {
            for &ch in channels {
                self.params.channels[ch].release_pending.store(true, Ordering::Relaxed);
            }
        }
        self.prev_keys = *keys;
    }

    /// Plays slice `index` of channel `ch` now (a SLICES pad). A pad past the last
    /// slice does nothing.
    fn trigger_slice(&self, ch: usize, index: usize) {
        let c = &self.params.channels[ch];
        if index < c.num_slices.load(Ordering::Relaxed).max(1) {
            c.slice_request.store(index, Ordering::Relaxed);
            c.trig_pending.store(true, Ordering::Relaxed);
        }
    }

    /// Sets Loop Bars from the channel's sample: its length against a tempo in its
    /// name, or the project tempo.
    fn guess_bars(&self, ch: usize) {
        if let Some(slot) = self.resolved_sample(ch).and_then(|i| self.params.samples.get(i)) {
            let (data, rate) = slot.decoded();
            let bars = guess_loop_bars(&slot.name, data.len() as f32 / rate.max(1.0), self.params.clock.bpm(), self.params.clock.bar_beats() as f32);
            self.params.channels[ch].loop_bars.store(bars, Ordering::Relaxed);
            // Only a sample that says its tempo is assumed to be a loop; a one-shot
            // (a piano note, a kick) must not be sped up to fit the bar. Either way
            // the row can be switched.
            self.params.channels[ch].tempo_match.store(tempo_in_name(&slot.name).is_some(), Ordering::Relaxed);
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
            0 => vec![Selection::Channel, Selection::NotesTo, Selection::Preset, Selection::SavePreset, Selection::LoadPreset],
            1 => vec![Selection::Sample, Selection::Mode, Selection::Tune, Selection::FineTune, Selection::TempoMatch, Selection::LoopBars, Selection::Start, Selection::LoopPoint, Selection::End],
            2 => vec![Selection::NumSlices, Selection::SliceMode, Selection::SliceStep, Selection::ResetSlice, Selection::AutoClock, Selection::AutoRate],
            3 => vec![Selection::EnvAttack, Selection::EnvHold, Selection::EnvDecay, Selection::EnvRange, Selection::EnvAShape, Selection::EnvDShape],
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
            Selection::FineTune => "Fine Tune".into(),
            Selection::TempoMatch => "Tempo Match".into(),
            Selection::LoopBars => "Loop Bars".into(),
            Selection::NotesTo => "Notes Play".into(),
            Selection::EnvAShape => "Attack Shape".into(),
            Selection::EnvDShape => "Decay Shape".into(),
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
            Selection::SavePreset | Selection::LoadPreset => "hold SELECT".into(),
            Selection::Sample => self.sample_name(ch),
            Selection::Mode => PLAY_MODE_NAMES[c.mode.load(Ordering::Relaxed) as usize % 4].to_string(),
            Selection::Tune => format!("{:+} st", c.tune.load(Ordering::Relaxed)),
            Selection::FineTune => format!("{:+} ct", c.fine.load(Ordering::Relaxed)),
            Selection::TempoMatch => if c.tempo_match.load(Ordering::Relaxed) { format!("to {:.0} BPM", self.params.clock.bpm()) } else { "off".into() },
            Selection::LoopBars => format!("{}", c.loop_bars.load(Ordering::Relaxed)),
            Selection::NotesTo => NOTE_TARGET_NAMES[self.params.note_target.load(Ordering::Relaxed) as usize % 3].to_string(),
            Selection::EnvAShape => format!("{:+.0}", c.env_ashape.get() * 50.0),
            Selection::EnvDShape => format!("{:+.0}", c.env_dshape.get() * 50.0),
            Selection::Start => format!("{:.0}%", c.start.get() * 100.0),
            Selection::LoopPoint => format!("{:.0}%", c.loop_point.get() * 100.0),
            Selection::End => format!("{:.0}%", c.end.get() * 100.0),
            Selection::NumSlices => format!("{}", c.num_slices.load(Ordering::Relaxed)),
            Selection::SliceMode => SLICE_MODE_NAMES[c.slice_mode.load(Ordering::Relaxed) as usize % 2].to_string(),
            Selection::SliceStep => SLICE_STEP_NAMES[c.slice_step.load(Ordering::Relaxed) as usize % SLICE_STEP_NAMES.len()].to_string(),
            Selection::ResetSlice => "hold SELECT".into(),
            Selection::AutoClock => if c.auto_clock.load(Ordering::Relaxed) { "ON".into() } else { "off".into() },
            Selection::AutoRate => format!("{:.0} BPM", c.auto_rate_bpm.get()),
            Selection::EnvAttack => format!("{:.0}%", c.env_attack.get() * 100.0),
            Selection::EnvHold => format!("{:.0}%", c.env_hold.get() * 100.0),
            Selection::EnvDecay => format!("{:.0}%", c.env_decay.get() * 100.0),
            Selection::EnvRange => ENV_RANGE_NAMES[c.env_range.load(Ordering::Relaxed) as usize % 4].to_string(),
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
                self.guess_bars(ch);
            }
            Selection::Mode => {
                let cur = c.mode.load(Ordering::Relaxed) as i32;
                c.mode.store((cur + step).rem_euclid(4) as u32, Ordering::Relaxed);
            }
            Selection::Tune => {
                let cur = c.tune.load(Ordering::Relaxed);
                c.tune.store(cur + step, Ordering::Relaxed);
            }
            Selection::TempoMatch => c.tempo_match.store(delta > 0, Ordering::Relaxed),
            Selection::LoopBars => {
                let cur = LOOP_BARS.iter().position(|b| *b == c.loop_bars.load(Ordering::Relaxed)).unwrap_or(0) as i32;
                c.loop_bars.store(LOOP_BARS[(cur + step).clamp(0, LOOP_BARS.len() as i32 - 1) as usize], Ordering::Relaxed);
            }
            Selection::FineTune => {
                let cur = c.fine.load(Ordering::Relaxed);
                c.fine.store((cur + step * 5 * delta.abs().min(4)).clamp(-100, 100), Ordering::Relaxed);
            }
            Selection::NotesTo => {
                let cur = self.params.note_target.load(Ordering::Relaxed) as i32;
                self.params.note_target.store((cur + step).rem_euclid(3) as u32, Ordering::Relaxed);
            }
            Selection::EnvAShape => bump(&c.env_ashape, delta, sensitivity, -1.0, 1.0),
            Selection::EnvDShape => bump(&c.env_dshape, delta, sensitivity, -1.0, 1.0),
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
                let next = (cur + step).rem_euclid(4) as u32;
                c.env_range.store(next, Ordering::Relaxed);
                if next == 3 {
                    // The manual's Relative defaults: no attack, hold the whole
                    // region, no decay -- the sample plays as recorded.
                    c.env_attack.set(0.0);
                    c.env_hold.set(1.0);
                    c.env_decay.set(0.0);
                }
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
            Selection::FineTune => c.fine.store(0, Ordering::Relaxed),
            Selection::TempoMatch => c.tempo_match.store(false, Ordering::Relaxed),
            Selection::LoopBars => self.guess_bars(ch),
            Selection::NotesTo => self.params.note_target.store(0, Ordering::Relaxed),
            Selection::EnvAShape => c.env_ashape.set(0.0),
            Selection::EnvDShape => c.env_dshape.set(0.0),
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
            fine: c.fine.load(Ordering::Relaxed),
            tempo_match: c.tempo_match.load(Ordering::Relaxed),
            loop_bars: c.loop_bars.load(Ordering::Relaxed),
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
            env_ashape: c.env_ashape.get(),
            env_dshape: c.env_dshape.get(),
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
        c.fine.store(preset.fine.clamp(-100, 100), Ordering::Relaxed);
        c.tempo_match.store(preset.tempo_match, Ordering::Relaxed);
        c.loop_bars.store(if LOOP_BARS.contains(&preset.loop_bars) { preset.loop_bars } else { 1 }, Ordering::Relaxed);
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
        c.env_range.store(preset.env_range % 4, Ordering::Relaxed);
        c.env_ashape.set(preset.env_ashape.clamp(-1.0, 1.0));
        c.env_dshape.set(preset.env_dshape.clamp(-1.0, 1.0));
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

impl SampleDrumApp {
    /// Storage for the controls that have an atomic Knob can describe
    /// (the selected channel's); Sample, Slices and Channel are
    /// AtomicUsize and handled by hand in `kit_norm`/`kit_set_norm`.
    fn knob(&self, i: usize) -> Knob<'_> {
        let c = &self.params.channels[self.ch()];
        match CONTROLS[i % 16].0 {
            Selection::Start => Knob::F(&c.start, 0.0, 1.0),
            Selection::End => Knob::F(&c.end, 0.0, 1.0),
            Selection::EnvDecay => Knob::F(&c.env_decay, 0.0, 1.0),
            Selection::FxMix => Knob::F(&c.fx_mix, 0.0, 1.0),
            Selection::FxParam1 => Knob::F(&c.fx_param1, 0.0, 1.0),
            Selection::FxParam2 => Knob::F(&c.fx_param2, 0.0, 1.0),
            Selection::AutoRate => Knob::F(&c.auto_rate_bpm, 20.0, 300.0),
            Selection::Volume => Knob::F(&c.volume, 0.0, 1.0),
            Selection::SliceStep => Knob::U(&c.slice_step, SLICE_STEP_NAMES.len() as u32),
            Selection::FxType => Knob::U(&c.fx_type, FX_NAMES.len() as u32),
            Selection::Mode => Knob::U(&c.mode, PLAY_MODE_NAMES.len() as u32),
            Selection::Tune => Knob::I(&c.tune, -TUNE_SPAN, TUNE_SPAN),
            Selection::EnvAttack => Knob::F(&c.env_attack, 0.0, 1.0),
            _ => Knob::None,
        }
    }
}

impl PlayHost for SampleDrumApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS.len()
    }
    fn kit_label(&self, i: usize) -> String {
        CONTROLS[i % 16].1.to_string()
    }
    fn kit_value(&self, i: usize) -> String {
        self.leaf_value(CONTROLS[i % 16].0)
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        let ch = self.ch();
        let c = &self.params.channels[ch];
        match i % 16 {
            C_SAMPLE => {
                // No sample loaded has no position, so a moment taken
                // then leaves whatever sample is loaded at recall alone.
                let n = self.params.samples.len();
                self.resolved_sample(ch).filter(|&idx| idx < n).map(|idx| if n > 1 { idx as f32 / (n - 1) as f32 } else { 0.0 })
            }
            C_SLICES => Some((c.num_slices.load(Ordering::Relaxed).clamp(1, MAX_SLICES) - 1) as f32 / (MAX_SLICES - 1) as f32),
            C_CHANNEL => Some(ch as f32 / (NUM_CHANNELS - 1) as f32),
            _ => self.knob(i).norm(),
        }
    }
    fn kit_stepped(&self, i: usize) -> bool {
        i >= C_SAMPLE && i != C_ATTACK
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit(CONTROLS[i % 16].0, delta);
    }
    fn kit_reset(&mut self, i: usize) {
        self.reset(CONTROLS[i % 16].0);
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        let ch = self.ch();
        let c = &self.params.channels[ch];
        match i % 16 {
            C_SAMPLE => {
                let n = self.params.samples.len();
                if n > 0 {
                    c.sample.store((v * (n - 1) as f32).round() as usize, Ordering::Relaxed);
                    self.warm_selected_sample(ch);
                }
            }
            C_SLICES => c.num_slices.store(1 + (v * (MAX_SLICES - 1) as f32).round() as usize, Ordering::Relaxed),
            C_CHANNEL => self.params.channel.store((v * (NUM_CHANNELS - 1) as f32).round() as usize, Ordering::Relaxed),
            _ => self.knob(i).set(v),
        }
    }
    /// A moment is one channel's sound, recalled onto whichever channel
    /// is selected -- so the channel selector itself is left out (it
    /// would otherwise switch channels halfway through the recall).
    fn kit_snapshot(&self) -> serde_json::Value {
        serde_json::Value::Array(
            (0..CONTROLS.len())
                .map(|i| if i == C_CHANNEL { serde_json::Value::Null } else { self.kit_norm(i).map_or(serde_json::Value::Null, |v| serde_json::json!(v)) })
                .collect(),
        )
    }
    fn kit_line(&self) -> String {
        let ch = self.ch();
        let c = &self.params.channels[ch];
        let slices = c.num_slices.load(Ordering::Relaxed).max(1);
        format!("CH{} slice {}/{}  {}", ch + 1, c.step_index.load(Ordering::Relaxed).min(slices - 1) + 1, slices, self.sample_name(ch))
    }
    fn kit_pad_label(&self, layer: u8, pad: usize) -> String {
        if layer > 0 {
            let index = (layer as usize - 1) * 16 + pad;
            return if index < self.params.channels[self.ch()].num_slices.load(Ordering::Relaxed).max(1) { format!("{}", index + 1) } else { String::new() };
        }
        match pad {
            0 => "TRIG 1".into(),
            1 => "TRIG 2".into(),
            _ => String::new(),
        }
    }
    /// TRIG pads lit, yellow while that channel's auto clock is firing
    /// it on its own; the other 14 pads stay dark, as before.
    fn kit_pad_color(&self, layer: u8, pad: usize, held: bool) -> crate::led_output::PadColor {
        use crate::led_output::PadColor;
        if layer > 0 {
            let c = &self.params.channels[self.ch()];
            let index = (layer as usize - 1) * 16 + pad;
            return if index >= c.num_slices.load(Ordering::Relaxed).max(1) {
                PadColor::Off
            } else if held {
                PadColor::Green
            } else if c.last_slice.load(Ordering::Relaxed) == index {
                PadColor::Yellow
            } else {
                PadColor::Blue
            };
        }
        if pad >= NUM_CHANNELS {
            PadColor::Off
        } else if held {
            PadColor::Green
        } else if self.params.channels[pad].auto_clock.load(Ordering::Relaxed) {
            PadColor::Yellow
        } else {
            PadColor::Blue
        }
    }
}

impl App for SampleDrumApp {
    fn supports_pad_lock(&self) -> bool { true }
    fn play_surface(&self) -> bool {
        true
    }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (!self.kit.menu).then(|| self.kit.column(self))
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }
    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
    }
    fn grid_led_overlay(&self) -> [crate::led_output::PadColor; 16] {
        self.kit.led_overlay(self)
    }

    fn tick(&mut self, input: &Input) {
        // The play view takes the knobs and D-pad first; in the menu they
        // pass straight through. Pads reach TRIG1/TRIG2 only on TRIG.
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let input = &step.input;
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
        match step.native {
            Some(1) | Some(2) => {
                let page = (step.native.unwrap_or(1) as usize - 1) * 16;
                let ch = self.ch();
                for pad in 0..16 {
                    if input.grid[pad] && !self.prev_grid[pad] {
                        self.trigger_slice(ch, page + pad);
                    }
                }
            }
            _ => {
                if input.grid[0] && !self.prev_grid[0] {
                    self.params.channels[0].trig_pending.store(true, Ordering::Relaxed);
                }
                if input.grid[1] && !self.prev_grid[1] {
                    self.params.channels[1].trig_pending.store(true, Ordering::Relaxed);
                }
            }
        }
        self.prev_grid = input.grid;
        self.play_notes(&input.midi_keys.0);
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
            mono: Vec::new(),
            snapshot: Vec::new(),
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
        if self.kit.menu {
            self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, SAMPLE_DRUM_BG, SAMPLE_DRUM_DIM, SAMPLE_DRUM_ACCENT);
        } else if let Some(col) = self.play_column() {
            // Ends above the slice view at y = 292.
            let pal = kit::draw::Palette { bg: SAMPLE_DRUM_BG, ink: SAMPLE_DRUM_TITLE, accent: SAMPLE_DRUM_ACCENT, dim: SAMPLE_DRUM_DIM, faint: Rgb565::new(7, 12, 6) };
            kit::draw::column(fb, &col, 16, 40, 350, 248, pal);
        }

        self.draw_slice_view(fb);

        let hint = match rows.get(self.list.selected) {
            _ if !self.kit.menu => "L/R: slice/env/FX   U/D: sample   F2: pads   F3: auto clock   R1: menu".to_string(),
            Some(Row::Group(_)) => "up/down: browse   SELECT: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("left/right: change {}   hold SELECT: reset", self.leaf_name(*sel)),
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
    /// Reused every block so the audio thread allocates nothing after the first.
    mono: Vec<f32>,
    snapshot: Vec<f32>,
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
        let mut mono_buf = std::mem::take(&mut self.mono);
        mono_buf.clear();
        mono_buf.resize(frames, 0.0);

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
                    // With Tempo Match the loop is `loop_bars` long at the project
                    // tempo; otherwise it is one bar at Auto Rate.
                    let loop_s = if c.tempo_match.load(Ordering::Relaxed) {
                        c.loop_bars.load(Ordering::Relaxed).max(1) as f32 * self.params.clock.bar_beats() as f32 * 60.0 / self.params.clock.bpm().max(1.0)
                    } else {
                        240.0 / c.auto_rate_bpm.get().max(1.0)
                    };
                    let period = (loop_s / num_slices).max(0.02);
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

            if c.release_pending.swap(false, Ordering::Relaxed) && c.mode.load(Ordering::Relaxed) % 2 == 1 {
                self.voices[ch].release();
            }

            if c.trig_pending.swap(false, Ordering::Relaxed) {
                let sample_idx = c.sample.load(Ordering::Relaxed);
                if let Some(slot) = self.params.samples.get(sample_idx) {
                    let (data, _) = slot.decoded();
                    let start = (c.start.get() + c.ext_start.get()).clamp(0.0, 1.0);
                    let end = (c.end.get() + c.ext_end.get()).clamp(0.0, 1.0);
                    let num_slices = c.num_slices.load(Ordering::Relaxed);
                    let slice_step_mode = c.slice_step.load(Ordering::Relaxed);
                    let requested = c.slice_request.swap(usize::MAX, Ordering::Relaxed);
                    let step = match slice_step_mode {
                        _ if requested != usize::MAX => requested % num_slices.max(1),
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
                    c.last_slice.store(step, Ordering::Relaxed);
                    if requested != usize::MAX {
                        // A pad chose this slice: the clock's own order carries on as it was.
                    } else if slice_step_mode == 0 {
                        c.step_index.store((step + 1) % num_slices.max(1), Ordering::Relaxed);
                    } else if slice_step_mode == 1 {
                        c.step_index.store(step, Ordering::Relaxed);
                    }
                }
            }

            let sample_idx = c.sample.load(Ordering::Relaxed);
            let Some(slot) = self.params.samples.get(sample_idx) else {
                if let Ok(mut shared) = c.waveform_snapshot.try_lock() {
                    shared.clear();
                }
                continue;
            };
            let (data, native_rate) = slot.decoded();
            // Tempo Match: the sample's loop (`loop_bars` bars) should last as long as
            // that many bars at the project tempo, so play it faster or slower by
            // the ratio, expressed in semitones like every other pitch offset here.
            let match_semitones = if c.tempo_match.load(Ordering::Relaxed) {
                let loop_s = data.len() as f32 / native_rate.max(1.0);
                let want_s = c.loop_bars.load(Ordering::Relaxed).max(1) as f32 * self.params.clock.bar_beats() as f32 * 60.0 / self.params.clock.bpm().max(1.0);
                12.0 * (loop_s / want_s.max(0.01)).clamp(0.25, 4.0).log2()
            } else {
                0.0
            };
            let semitones = match_semitones + c.tune.load(Ordering::Relaxed) as f32 + c.fine.load(Ordering::Relaxed) as f32 / 100.0 + c.ext_tune.get() + c.note_semi.get();
            let range = c.env_range.load(Ordering::Relaxed) as usize % 4;
            let range_max = ENV_RANGE_MAX_SECONDS[range];
            let env = EnvSettings {
                attack: c.env_attack.get() * range_max,
                hold: c.env_hold.get() * range_max,
                decay: if range == 3 { c.env_decay.get() * range_max } else { (c.env_decay.get() * range_max).max(0.02) },
                a_shape: c.env_ashape.get(),
                d_shape: c.env_dshape.get(),
                relative: range == 3,
            };
            let volume = c.volume.get() * c.note_gain.get();

            let fx_type = c.fx_type.load(Ordering::Relaxed);
            let fx_p1 = c.fx_param1.get();
            let fx_p2 = c.fx_param2.get();
            let fx_mix = c.fx_mix.get();

            // Filter coefficients depend only on this block's knobs, so they are
            // set once here rather than for every sample.
            match fx_type {
                3 => self.fx_biquad[ch].set_lowpass(80.0 + fx_p1 * 8000.0, sample_rate, 0.5 + fx_p2 * 3.0),
                4 => self.fx_biquad[ch].set_highpass(40.0 + fx_p1 * 6000.0, sample_rate, 0.5 + fx_p2 * 3.0),
                _ => {}
            }
            let mut snapshot = std::mem::take(&mut self.snapshot);
            snapshot.clear();
            let stride = (frames / 96).max(1);
            for (i, m) in mono_buf.iter_mut().enumerate() {
                let dry = self.voices[ch].render(data, native_rate, sample_rate, semitones, env, sample_rate);
                let wet = match fx_type {
                    1 => self.fx_delay[ch].process(dry, 60.0 + fx_p1 * 900.0, fx_p2, sample_rate),
                    2 => self.fx_reverb[ch].process(dry, fx_p1, fx_p2, sample_rate),
                    3 | 4 => self.fx_biquad[ch].process(dry),
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
            // The UI only reads this for a scope; never wait for it on the audio thread.
            if let Ok(mut shared) = c.waveform_snapshot.try_lock() {
                shared.clear();
                shared.extend_from_slice(&snapshot);
            }
            self.snapshot = snapshot;
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
        self.mono = mono_buf;
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
        let app = SampleDrumApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);
        // Tests that measure pitch or timing want the sample at its own speed.
        for c in &app.params.channels {
            c.tempo_match.store(false, Ordering::Relaxed);
        }
        app
    }

    /// An envelope so long it never ends a test early.
    fn long_env() -> EnvSettings {
        EnvSettings { attack: 0.0, hold: 0.0, decay: 100.0, a_shape: 0.0, d_shape: 0.0, relative: false }
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
            mono: Vec::new(),
            snapshot: Vec::new(),
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
            voice.render(&data, 48000.0, 48000.0, 0.0, long_env(), 48000.0); // huge decay so the envelope never ends this test early
            max_pos = max_pos.max(voice.head.pos);
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
            looped.render(&data, 48000.0, 48000.0, 0.0, long_env(), 48000.0);
        }
        assert!(looped.playing, "Backward Loop must keep looping");

        let mut one_shot = DrumVoice::default();
        one_shot.trigger(0.0, 1.0, 0.5, true, false, data.len()); // plain Backward
        for _ in 0..20000 {
            one_shot.render(&data, 48000.0, 48000.0, 0.0, long_env(), 48000.0);
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
        assert!((voice.head.lower - expected_lower * len_f).abs() < 1.0, "CV=0.9 over 4 slices should trigger slice index 3, got lower={}", voice.head.lower);
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

    #[test]
    fn opens_playable_knob_1_moves_the_slice_start_and_r1_opens_the_menu() {
        let mut app = new_app();
        assert!(app.play_column().is_some(), "play view first");
        let start = app.params.channels[0].start.get();
        app.tick(&Input { knob1: 3, ..Default::default() });
        assert!(app.params.channels[0].start.get() > start, "knob 1 is Start on the play view");
        app.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(app.play_column().is_none(), "R1 opens the full menu");
    }

    /// Needs no sample: TRIG only arms the channel's one-shot trigger,
    /// which the processor consumes (tested above with real samples).
    #[test]
    fn trig_layer_pads_still_fire_trig1_and_trig2() {
        let mut app = new_app();
        assert_eq!(app.kit.layer_label(), "TRIG");
        app.tick(&Input { grid: std::array::from_fn(|i| i == 0), ..Default::default() });
        assert!(app.params.channels[0].trig_pending.load(Ordering::Relaxed), "pad 0 is TRIG1");
        assert!(!app.params.channels[1].trig_pending.load(Ordering::Relaxed));
        app.tick(&Input { grid: std::array::from_fn(|i| i == 1), ..Default::default() });
        assert!(app.params.channels[1].trig_pending.load(Ordering::Relaxed), "pad 1 is TRIG2");
    }

    fn held(note: usize, vel: u8) -> [u8; 128] {
        let mut k = [0u8; 128];
        k[note] = vel;
        k
    }

    /// A note plays the sample at pitch: C4 is the sample's own speed, an octave
    /// up reads it twice as fast.
    #[test]
    fn a_note_plays_the_sample_at_its_pitch() {
        let mut speeds = Vec::new();
        for note in [60, 72, 48] {
            let mut app = new_app();
            let mut proc = new_processor(Arc::clone(&app.params));
            app.params.channels[0].env_decay.set(1.0);
            app.params.channels[0].env_range.store(2, Ordering::Relaxed);
            app.play_notes(&held(note, 100));
            let mut buffer = vec![0.0f32; 480 * 2];
            proc.process(&mut buffer, 2, 48000.0);
            let (_, native) = app.params.samples[0].decoded();
            speeds.push(proc.voices[0].head.pos / (480.0 * native / 48000.0));
        }
        assert!((speeds[0] - 1.0).abs() < 0.05, "C4 plays at the sample's own pitch: {}", speeds[0]);
        assert!((speeds[1] - 2.0).abs() < 0.1, "C5 an octave up: {}", speeds[1]);
        assert!((speeds[2] - 0.5).abs() < 0.05, "C3 an octave down: {}", speeds[2]);
    }

    #[test]
    fn velocity_sets_the_level_and_the_target_row_picks_the_channel() {
        let peak = |vel: u8, target: u32| {
            let mut app = new_app();
            let mut proc = new_processor(Arc::clone(&app.params));
            app.params.note_target.store(target, Ordering::Relaxed);
            app.params.channels[1].env_decay.set(1.0);
            app.params.channels[0].env_decay.set(1.0);
            app.play_notes(&held(60, vel));
            let mut buffer = vec![0.0f32; 480 * 2];
            proc.process(&mut buffer, 2, 48000.0);
            (buffer.iter().fold(0.0f32, |m, v| m.max(v.abs())), proc.voices[0].playing, proc.voices[1].playing)
        };
        let (soft, ..) = peak(20, 0);
        let (loud, ..) = peak(127, 0);
        assert!(loud > soft * 1.5, "harder keys are louder: {soft} vs {loud}");
        assert_eq!(peak(100, 0).1 as u8 * 2 + peak(100, 0).2 as u8, 2, "Ch 1 only");
        assert_eq!(peak(100, 1).1 as u8 * 2 + peak(100, 1).2 as u8, 1, "Ch 2 only");
        assert_eq!(peak(100, 2).1 as u8 * 2 + peak(100, 2).2 as u8, 3, "both");
    }

    /// A looping voice goes on until the key is let go, then fades through its decay.
    #[test]
    fn letting_go_of_a_key_releases_a_looping_voice_only() {
        for (mode, stops) in [(1u32, true), (0u32, false)] {
            let mut app = new_app();
            let mut proc = new_processor(Arc::clone(&app.params));
            let c = &app.params.channels[0];
            c.mode.store(mode, Ordering::Relaxed);
            c.env_range.store(0, Ordering::Relaxed);
            c.env_hold.set(1.0);
            c.env_decay.set(0.05);
            app.play_notes(&held(60, 100));
            let mut buffer = vec![0.0f32; 480 * 2];
            proc.process(&mut buffer, 2, 48000.0);
            assert!(proc.voices[0].playing);
            app.play_notes(&[0u8; 128]);
            for _ in 0..20 {
                proc.process(&mut buffer, 2, 48000.0);
            }
            assert_eq!(!proc.voices[0].playing, stops, "mode {mode}: release only matters for loops");
        }
    }

    /// Relative range: the times are fractions of the region, so a sample
    /// held at 100% plays whole and one with 50% hold and no decay is cut at
    /// half.
    #[test]
    fn the_relative_envelope_scales_to_the_region() {
        let data: Vec<f32> = vec![0.5; 48000];
        let play = |hold: f32| {
            let mut v = DrumVoice::default();
            v.trigger(0.0, 1.0, 0.0, false, false, data.len());
            let env = EnvSettings { attack: 0.0, hold, decay: 0.0, a_shape: 0.0, d_shape: 0.0, relative: true };
            (0..60000).take_while(|_| { v.render(&data, 48000.0, 48000.0, 0.0, env, 48000.0); v.playing }).count()
        };
        let half = play(0.5);
        assert!((23000..25500).contains(&half), "half the region: {half}");
        let hole = play(1.0);
        assert!(hole > 47000, "the whole region: {hole}");
    }

    #[test]
    fn envelope_shapes_bend_the_curves() {
        let level_at = |shape: f32, state_attack: bool| {
            let mut e = Envelope::default();
            e.trigger();
            if state_attack {
                for _ in 0..500 { e.tick(1.0, 0.0, 1.0, shape, 0.0, 0.001); }
            } else {
                e.state = 3;
                for _ in 0..500 { e.tick(1.0, 0.0, 1.0, 0.0, shape, 0.001); }
            }
            e.level
        };
        // Halfway through: linear 0.5, logarithmic attack faster, exponential slower.
        assert!((level_at(0.0, true) - 0.5).abs() < 0.02);
        assert!(level_at(1.0, true) > 0.7 && level_at(-1.0, true) < 0.3);
        // Decay: exponential drops faster than linear, logarithmic stays up.
        assert!(level_at(-1.0, false) < 0.2 && level_at(1.0, false) > 0.7);
    }

    /// Cutting a playing voice mid-waveform used to click; the old playhead now
    /// fades under the new one, so the output never jumps between samples.
    #[test]
    fn a_retrigger_does_not_click() {
        let data: Vec<f32> = (0..9600).map(|i| (i as f32 * 0.05).sin() * 0.9).collect();
        let mut v = DrumVoice::default();
        v.trigger(0.0, 1.0, 0.0, false, false, data.len());
        let env = EnvSettings { attack: 0.0, hold: 5.0, decay: 1.0, a_shape: 0.0, d_shape: 0.0, relative: false };
        let mut prev = 0.0;
        let mut worst = 0.0f32;
        for i in 0..4000 {
            if i == 2000 {
                // Retrigger from a different place in the sample.
                v.trigger(0.3, 1.0, 0.3, false, false, data.len());
            }
            let x = v.render(&data, 48000.0, 48000.0, 0.0, env, 48000.0);
            if i > 1 {
                worst = worst.max((x - prev).abs());
            }
            prev = x;
        }
        assert!(worst < 0.2, "largest step between samples across the retrigger: {worst}");
    }

    #[test]
    fn fine_tune_is_in_cents() {
        let mut app = new_app();
        let mut proc = new_processor(Arc::clone(&app.params));
        app.params.channels[0].env_decay.set(1.0);
        app.params.channels[0].env_range.store(2, Ordering::Relaxed);
        app.params.channels[0].fine.store(100, Ordering::Relaxed); // +100 cents = one semitone
        app.params.channels[0].trig_pending.store(true, Ordering::Relaxed);
        let mut buffer = vec![0.0f32; 480 * 2];
        proc.process(&mut buffer, 2, 48000.0);
        let (_, native) = app.params.samples[0].decoded();
        let speed = proc.voices[0].head.pos / (480.0 * native / 48000.0);
        assert!((speed - 2f32.powf(1.0 / 12.0)).abs() < 0.02, "+100 ct is a semitone: {speed}");
    }

    /// A SLICES pad plays its own slice, whatever the step order is, and a pad
    /// past the last slice does nothing.
    #[test]
    fn slice_pads_play_their_own_slice() {
        let app = new_app();
        let mut proc = new_processor(Arc::clone(&app.params));
        let c = &app.params.channels[0];
        c.num_slices.store(8, Ordering::Relaxed);
        c.slice_step.store(2, Ordering::Relaxed); // RND: pads must not follow it
        c.env_decay.set(1.0);
        let (data, _) = app.params.samples[0].decoded();
        let len_f = (data.len() - 1) as f32;
        for slice in [5usize, 0, 7, 3] {
            app.trigger_slice(0, slice);
            let mut buffer = vec![0.0f32; 64 * 2];
            proc.process(&mut buffer, 2, 48000.0);
            let (lower, _) = slice_bounds(data, 0.0, 1.0, 8, false, slice);
            assert!((proc.voices[0].head.lower - lower * len_f).abs() < 1.0, "pad {slice} plays slice {slice}");
            assert_eq!(c.last_slice.load(Ordering::Relaxed), slice);
        }
        app.trigger_slice(0, 12);
        assert!(!c.trig_pending.load(Ordering::Relaxed), "slice 13 of 8 doesn't exist");
        assert_eq!(app.kit_pad_label(1, 3), "4");
        assert_eq!(app.kit_pad_label(1, 9), "");
        assert_eq!(app.kit_pad_label(2, 0), "");
    }

    #[test]
    fn a_tempo_in_the_name_is_found_and_bars_are_guessed() {
        assert_eq!(tempo_in_name("KAB1_174_AmenBreak_Cut_01"), Some(174.0));
        assert_eq!(tempo_in_name("Mystery Sample 07"), None, "07 is not a tempo");
        assert_eq!(tempo_in_name("Kick 808 Long"), None);
        // 2 s is one bar at 120 BPM, and 4 s is two.
        assert_eq!(guess_loop_bars("loop_120", 2.0, 100.0, 4.0), 1);
        assert_eq!(guess_loop_bars("loop_120", 4.0, 100.0, 4.0), 2);
        // No tempo in the name: whichever length lands nearest the project tempo.
        assert_eq!(guess_loop_bars("break", 8.0, 120.0, 4.0), 4);
    }

    /// Tempo Match: a loop that is one bar at 100 BPM plays 1.2x faster at a project
    /// tempo of 120, so chopping it in 4 or in 8 both still tile the bar.
    #[test]
    fn tempo_match_fits_the_loop_to_the_project_tempo() {
        let app = new_app();
        let (data, native) = app.params.samples[0].decoded();
        let loop_s = data.len() as f32 / native;
        let c = &app.params.channels[0];
        c.tempo_match.store(true, Ordering::Relaxed);
        c.env_decay.set(1.0);
        c.env_range.store(2, Ordering::Relaxed);
        // Pick the bar count that keeps the ratio in range at 120 BPM.
        let bars = LOOP_BARS.iter().copied().min_by(|a, b| ((loop_s / (*a as f32 * 2.0)) - 1.0).abs().total_cmp(&((loop_s / (*b as f32 * 2.0)) - 1.0).abs())).unwrap();
        c.loop_bars.store(bars, Ordering::Relaxed);
        app.params.clock.set_bpm(120.0);
        let want = (loop_s / (bars as f32 * 2.0)).clamp(0.25, 4.0);
        let mut proc = new_processor(Arc::clone(&app.params));
        c.trig_pending.store(true, Ordering::Relaxed);
        let mut buffer = vec![0.0f32; 480 * 2];
        proc.process(&mut buffer, 2, 48000.0);
        let speed = proc.voices[0].head.pos / (480.0 * native / 48000.0);
        assert!((speed - want).abs() < 0.03 * want.max(1.0), "plays {want}x so {bars} bars last {bars} bars at 120: {speed}");
    }

    /// With Tempo Match, Auto Clock fires on the project's bar: the same bar of
    /// triggers whether the loop is cut in 4 or in 8.
    #[test]
    fn auto_clock_tiles_the_bar_at_the_project_tempo_for_any_slice_count() {
        let count = |slices: usize| {
            let app = new_app();
            app.params.clock.set_bpm(100.0); // a bar is 2.4 s
            let c = &app.params.channels[0];
            c.tempo_match.store(true, Ordering::Relaxed);
            c.loop_bars.store(1, Ordering::Relaxed);
            c.num_slices.store(slices, Ordering::Relaxed);
            c.auto_clock.store(true, Ordering::Relaxed);
            let mut proc = new_processor(Arc::clone(&app.params));
            let mut buffer = vec![0.0f32; 480 * 2];
            let mut triggers = 0;
            let mut prev = c.step_index.load(Ordering::Relaxed);
            for _ in 0..240 {
                proc.process(&mut buffer, 2, 48000.0);
                let now = c.step_index.load(Ordering::Relaxed);
                if now != prev {
                    triggers += 1;
                    prev = now;
                }
            }
            triggers
        };
        let (four, eight) = (count(4), count(8));
        assert!((4..=5).contains(&four) && (8..=9).contains(&eight), "one bar is 4 hits cut in 4 and 8 cut in 8: {four}, {eight}");
    }

    #[test]
    fn throws_hold_a_change_and_spring_back() {
        let mut app = new_app();
        while app.kit.layer_label() != "THROWS" {
            app.toggle_grid_mode();
        }
        let c = &app.params.channels[0];
        c.slice_step.store(0, Ordering::Relaxed);
        let decay = c.env_decay.get();
        // RANDOM is rank 0, CHOKE rank 1.
        app.tick(&Input { grid: std::array::from_fn(|i| i == kit::rank_pad(0) || i == kit::rank_pad(1)), ..Default::default() });
        assert_eq!(app.params.channels[0].slice_step.load(Ordering::Relaxed), 2, "RANDOM holds RND stepping");
        assert!(app.params.channels[0].env_decay.get() < decay, "CHOKE shortens the decay");
        app.tick(&Input::default());
        assert_eq!(app.params.channels[0].slice_step.load(Ordering::Relaxed), 0, "back to FWD");
        assert!((app.params.channels[0].env_decay.get() - decay).abs() < 1e-5, "decay back exactly");
        assert!(!app.params.channels[0].trig_pending.load(Ordering::Relaxed), "Throws pads never reach TRIG");
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(SampleDrumApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}
