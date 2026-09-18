//! A multi-effect audio processor in the spirit of Hologram's
//! Microcosm pedal -- not a clone of it (no access to, or interest in,
//! their firmware internals; this is built from the manual's
//! documented *behavior* plus standard, publicly-known DSP
//! techniques, same as Madness/Bloom/Pam's were built "in the spirit
//! of" their references). Microcosm is a processor, not a voice: like
//! Clouds, it has no microphone input in this sim, so its signal comes
//! from the shared AudioBus (see audio_bus.rs) via an input mixer,
//! same pattern Clouds already established.
//!
//! Microcosm organizes 11 effects into 4 categories (Micro Loop /
//! Granules / Glitch / Multidelay), each with 4 preset variations
//! (A-D), all sharing 6 macro knobs whose *meaning* shifts per effect
//! (Activity, Repeats, Shape, Filter, Mix, Space). All 4 categories --
//! 11 effects x 4 presets = 44 total, matching the real pedal's
//! count -- are implemented below, plus everything that was still an
//! explicit gap versus the real hardware:
//!
//! - **Space**: a stereo reverb send (4 parallel combs + 2 series
//!   allpasses per side, the classic Freeverb topology) -- see
//!   `ReverbChannel`.
//! - **Pitch Mod**: a fixed-depth chorus/vibrato stage, always in the
//!   chain when toggled on -- a real always-present stage on the
//!   hardware, not one of its 6 macro knobs, so it's a toggle here
//!   too rather than a 7th knob nothing on the real pedal has.
//! - **Phrase Looper** and **Hold Sampler**: footswitch-style utility
//!   modes (`UtilityMode`) that take over the wet signal path entirely
//!   instead of running one of the 44 presets, selected from the
//!   Utility group and driven by a single Trigger action.
//! - **User presets**: 8 session-only save/recall slots for the main
//!   knobs (`UserPresetData`) -- session-only because *nothing* in
//!   this whole sim persists to disk, not because this app is special.
//! - **MIDI CC mapping**: Time/Repeats/Shape/Filter/Mix/Space are each
//!   direct-mapped to a fixed CC number (see `PRISM_*_CC` in main.rs).
//!   Activity and Preset are deliberately left out -- see
//!   `PrismCcTargets`'s doc comment.
//! - **Stereo**: honestly partial, not "true" independent-channel
//!   processing -- every AudioBus tap in this whole sim, Prism's own
//!   input included, is mono, so the 44-preset effect chain and Pitch
//!   Mod stay mono too (identical on both channels). Only Space's
//!   reverb tail is genuinely decorrelated L/R (two comb+allpass
//!   chains with offset lengths), which is where whatever stereo
//!   width reaches the output actually comes from. A full rewrite to
//!   independent per-channel delay lines and tap state throughout
//!   would roughly double this file's state for a difference that's
//!   inaudible on anything but the reverb tail anyway, so it wasn't
//!   done.
//!
//! **Multidelay** (Pattern/Warp):
//!
//! - **Pattern**: a multi-tap delay line, 4 different rhythmic tap-time
//!   arrangements, same simple per-tap gain falloff for all of them.
//! - **Warp**: the *same* tap timing as Pattern A, but each tap is
//!   processed differently before being summed -- an envelope-shrinking
//!   lowpass (A), a resonant bandpass (B), an escalating per-tap pitch
//!   shift (C), or a dry/shimmer crossfade with a fixed +1-octave
//!   grain (D).
//!
//! **Micro Loop** (Mosaic/Seq/Glide):
//!
//! - **Mosaic**: the shared buffer becomes a rolling "last `Time`
//!   seconds of input" loop window instead of a delay history; each
//!   active layer (Activity) plays that same window back at its own
//!   fixed speed (the same grain-based pitch shifter Warp's C/D use,
//!   just with the grain window set to the loop length itself) --
//!   octave-up layers (A), octave-down (B), all double-speed (C), or
//!   a four-way half/normal/double/quad stack (D).
//! - **Seq**: Mosaic's same loop layers, each additionally run through
//!   one more process -- a resonant bandpass (A), a global speed flip
//!   between two rates on a slow timer instead of each layer having
//!   its own fixed rate (B), a slowly sweeping bandpass instead of a
//!   fixed one (C), or a bit-crusher (D, amount from Shape).
//! - **Glide**: a single loop layer (two for D) whose playback speed
//!   continuously glides back and forth between two fixed rates
//!   instead of sitting still -- Activity sets the glide's speed
//!   (repurposed from "layer count," since there's only ever 1-2
//!   layers here) and Shape blends the glide's curve between a linear
//!   ramp and an eased (sine-shaped) one, both matching the real
//!   pedal's documented repurposing of those two knobs specifically
//!   for this effect.
//!
//! **Granules** (Haze/Tunnel/Strum):
//!
//! - **Haze**: one-shot retriggering grains, each slot picking a fresh
//!   random start point and rate (pitch) every cycle instead of
//!   continuously looping a fixed window -- a Hann-windowed burst per
//!   retrigger, so grains fade in and out rather than looping seamlessly.
//! - **Tunnel**: a long, heavily-processed drone loop -- Activity is
//!   repurposed as "modifier depth" (layer count is fixed at 1-2) --
//!   with variants adding a slow filter sweep, a resonant bandpass, or
//!   a slow pitch drift depending on variant.
//! - **Strum**: onset-detected (fast/slow envelope comparator) input
//!   triggers a simultaneous cascade of pitch-shifted layers, each
//!   anchored to a different point in the onset history -- variants
//!   vary the per-layer pitch spread and filtering.
//!
//! **Glitch** (Blocks/Interrupt/Arp):
//!
//! - **Blocks**: staggered per-layer retriggering loop slices (a hard,
//!   percussive cousin of Haze's grains) -- each layer re-rolls its own
//!   start point and rate on its own cycle, with variants adding a
//!   +/-1 octave pitch jump, a softer crossfade window, a resonant
//!   bandpass, or a bit-crusher.
//! - **Interrupt**: a probability-gated periodic glitch burst --
//!   Repeats is repurposed as "how often it fires" rather than
//!   feedback amount -- that replaces the dry signal outright while
//!   bursting (so Mix=100% truly mutes dry only during a burst, never
//!   otherwise); variants add a filter sweep + secondary delay tap, or
//!   a bit-crusher.
//! - **Arp**: reuses Strum's onset detector, but instead of a
//!   simultaneous cascade, steps through the onset history one at a
//!   time on its own clock -- a musical-arpeggio-like sequencing of
//!   past onsets rather than a stacked chord of them; variants add a
//!   per-step rate pattern, a per-step pseudo-random filter sweep, or
//!   a bit-crusher.
//!
//! Every effect shares one delay/loop buffer and one feedback rule:
//! feedback (Multidelay only -- every other category never feeds back,
//! so it's trivially bounded by the input itself) comes from only the
//! *longest* active tap's raw, unprocessed content, never from
//! anything a variant's own filtering/pitch-shifting touched. That's
//! what keeps every preset -- however it processes what gets read --
//! exactly as stable as the simplest one.

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle};
use embedded_graphics::text::Text;
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Fixed capacity for input slots -- storage only; how many are
/// actually shown/used follows `AudioBus::len()` live, same pattern
/// Clouds/Bloom/Pam's use. Was 8, which silently dropped any app
/// registered after the 8th (alphabetically, already losing Tape and
/// Voltage) from this app's input list -- bumped with real headroom
/// so the next few apps added to `apps/` don't quietly hit the same
/// ceiling.
const MAX_INPUTS: usize = 32;
const MAX_TAPS: usize = 8;
const MIN_TAPS: usize = 1;
const DEFAULT_TAPS: usize = 4;
// `pub(crate)` (not private) on these three -- main.rs's MIDI CC
// handler needs the same ranges `edit()`/`bump()` clamp to, to scale
// an incoming 0..127 CC value correctly (see `PrismCcTargets`).
pub(crate) const MIN_TIME: f32 = 0.02;
pub(crate) const MAX_TIME: f32 = 1.0;
const DEFAULT_TIME: f32 = 0.3;
pub(crate) const MAX_REPEATS: f32 = 0.92; // headroom below 1.0 -- the single feedback tap must stay a contraction
const DEFAULT_REPEATS: f32 = 0.35;
const DEFAULT_SPACE: f32 = 0.0; // off by default -- a dry pedal until you reach for it

/// Comb/allpass delay lengths in samples at a 44100 Hz reference (scaled
/// by the actual sample rate at init) -- classic Freeverb-style values,
/// staggered to avoid harmonically-related resonances. The right
/// channel reuses the same lengths plus a fixed offset rather than a
/// second hand-tuned table -- enough to decorrelate L/R without
/// doubling the tuning work, since true stereo *input* doesn't exist
/// anywhere in this sim (every AudioBus tap, Prism's own input
/// included, is mono -- see the module doc comment) and this reverb
/// tail is the only place stereo width actually comes from.
const REVERB_COMB_COUNT: usize = 4;
const REVERB_ALLPASS_COUNT: usize = 2;
const REVERB_COMB_LENGTHS: [usize; REVERB_COMB_COUNT] = [1557, 1617, 1491, 1422];
const REVERB_ALLPASS_LENGTHS: [usize; REVERB_ALLPASS_COUNT] = [556, 441];
const REVERB_STEREO_OFFSET: usize = 23;
const REVERB_COMB_FEEDBACK: f32 = 0.82; // < 1.0 -- always a contraction, so the tail can't run away
const REVERB_ALLPASS_FEEDBACK: f32 = 0.5;
const REVERB_REFERENCE_RATE: f32 = 44100.0;

/// Pitch Mod is a separate always-in-the-signal-chain stage on the real
/// pedal, not one of its 6 macro knobs -- modeled here as a fixed-depth
/// chorus/vibrato (LFO-modulated short delay) toggled on/off rather
/// than exposing a 7th knob nothing on the hardware has.
const PITCH_MOD_BUFFER_SECONDS: f32 = 0.05;
const PITCH_MOD_CENTER_SECONDS: f32 = 0.015;
const PITCH_MOD_DEPTH_SECONDS: f32 = 0.005;
const PITCH_MOD_RATE_HZ: f32 = 0.4;
const PITCH_MOD_MIX: f32 = 0.35;

/// Utility modes: footswitch-style modules on the real pedal that take
/// over the wet signal path entirely instead of running one of the 44
/// main presets -- see `Effect` for those. `UtilityMode::Off` is the
/// default, in which the normal preset processing below runs
/// unchanged.
#[derive(Clone, Copy, PartialEq)]
enum UtilityMode {
    Off,
    Looper,
    Sampler,
}

impl UtilityMode {
    fn from_u32(v: u32) -> Self {
        match v % 3 {
            1 => UtilityMode::Looper,
            2 => UtilityMode::Sampler,
            _ => UtilityMode::Off,
        }
    }
    fn name(self) -> &'static str {
        match self {
            UtilityMode::Off => "Off",
            UtilityMode::Looper => "Looper",
            UtilityMode::Sampler => "Sampler",
        }
    }
}

const LOOPER_MAX_SECONDS: f32 = 8.0;
const SAMPLER_WINDOW_SECONDS: f32 = 0.2;
const NUM_USER_PRESETS: usize = 8;

/// Which effect a preset index falls into, and its variant (0=A..3=D)
/// within that effect -- see the module doc comment for what each
/// one does.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Effect {
    Pattern,
    Warp,
    Mosaic,
    Seq,
    Glide,
    Haze,
    Tunnel,
    Strum,
    Blocks,
    Interrupt,
    Arp,
}

const VARIANTS_PER_EFFECT: usize = 4;
const EFFECT_ORDER: [Effect; 11] = [
    Effect::Pattern,
    Effect::Warp,
    Effect::Mosaic,
    Effect::Seq,
    Effect::Glide,
    Effect::Haze,
    Effect::Tunnel,
    Effect::Strum,
    Effect::Blocks,
    Effect::Interrupt,
    Effect::Arp,
];

fn effect_for_preset(preset: usize) -> (Effect, usize) {
    let idx = (preset / VARIANTS_PER_EFFECT).min(EFFECT_ORDER.len() - 1);
    (EFFECT_ORDER[idx], preset % VARIANTS_PER_EFFECT)
}

fn category_name(effect: Effect) -> &'static str {
    match effect {
        Effect::Pattern | Effect::Warp => "Multidelay",
        Effect::Mosaic | Effect::Seq | Effect::Glide => "Micro Loop",
        Effect::Haze | Effect::Tunnel | Effect::Strum => "Granules",
        Effect::Blocks | Effect::Interrupt | Effect::Arp => "Glitch",
    }
}

/// 11 effects x 4 variants = 44 -- the same total preset count as the
/// real device (see module doc comment), even though this is an
/// independent implementation, not a port.
const PRESET_NAMES: [&str; EFFECT_ORDER.len() * VARIANTS_PER_EFFECT] = [
    "Pattern A", "Pattern B", "Pattern C", "Pattern D", // Multidelay
    "Warp A", "Warp B", "Warp C", "Warp D", // Multidelay
    "Mosaic A", "Mosaic B", "Mosaic C", "Mosaic D", // Micro Loop
    "Seq A", "Seq B", "Seq C", "Seq D", // Micro Loop
    "Glide A", "Glide B", "Glide C", "Glide D", // Micro Loop
    "Haze A", "Haze B", "Haze C", "Haze D", // Granules
    "Tunnel A", "Tunnel B", "Tunnel C", "Tunnel D", // Granules
    "Strum A", "Strum B", "Strum C", "Strum D", // Granules
    "Blocks A", "Blocks B", "Blocks C", "Blocks D", // Glitch
    "Interrupt A", "Interrupt B", "Interrupt C", "Interrupt D", // Glitch
    "Arp A", "Arp B", "Arp C", "Arp D", // Glitch
];

/// Each Pattern preset's per-tap time, as a multiple of the Time knob
/// -- original arrangements (not reverse-engineered from anything),
/// chosen to give each preset a distinct rhythmic character: A is a
/// classic evenly-spaced echo, B groups taps in syncopated pairs, C
/// has a triplet feel, D accelerates outward. Warp reuses A's timing
/// (see `tap_ratios_for`) since its 4 variants differ in per-tap
/// processing, not rhythm.
const PATTERN_RATIOS: [[f32; MAX_TAPS]; VARIANTS_PER_EFFECT] = [
    [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
    [1.0, 1.5, 2.5, 3.0, 4.0, 4.5, 5.5, 6.0],
    [0.667, 1.333, 2.0, 2.667, 3.333, 4.0, 4.667, 5.333],
    [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 11.0, 14.0],
];
/// Each Mosaic/Seq preset's fixed set of per-layer playback-speed
/// multiples -- active layers (Activity) cycle through this list, so
/// requesting more layers than the list's length thickens it with
/// unison copies (at a phase offset from `TapState`) rather than
/// running out of speeds.
const MOSAIC_RATES: [[f32; 4]; VARIANTS_PER_EFFECT] = [
    [1.0, 2.0, 1.0, 2.0],  // A: octave-up harmonies
    [1.0, 0.5, 1.0, 0.5],  // B: octave-down
    [2.0, 2.0, 2.0, 2.0],  // C: all double speed
    [0.5, 1.0, 2.0, 4.0],  // D: half/normal/double/quad stack
];
/// Seq always alternates between these two rates (see `Effect::Seq`
/// in the module doc comment) -- which rate is "active" depends on
/// the variant (B flips both between them on a shared slow timer;
/// A/C/D just split layers between the two, same as Mosaic's sets).
const SEQ_RATES: [f32; 2] = [1.0, 0.5];
/// Buffer capacity in seconds -- covers MAX_TIME (1.0s) * the largest
/// ratio in any preset (Pattern D's 14.0x), plus margin.
const DELAY_BUFFER_SECONDS: f32 = 15.0;
/// Grain window for Warp C/D's pitch shifter -- a typical size for a
/// delay-line pitch shifter; shorter warbles more, longer smears more.
const WARP_GRAIN_WINDOW_SECONDS: f32 = 0.04;
/// Fixed resonance for Warp B's bandpass, and Seq A/C's -- not
/// user-adjustable in this first pass (Filter instead sets the
/// bandpass's center frequency; see `process()`).
const BANDPASS_Q: f32 = 3.0;
/// How long one full slow-LFO cycle takes, as a multiple of the Time
/// knob -- used by Seq B's speed flip, Seq C's filter sweep, and
/// Glide's rate glide (each interprets the same 0..1 phase its own
/// way; see `process()`).
const SLOW_LFO_TIME_MULT: f32 = 4.0;
const MIN_BITCRUSH_LEVELS: f32 = 2.0;
const MAX_BITCRUSH_LEVELS: f32 = 32.0;
/// Strum's onset detector: a fast envelope compared against a slower
/// one -- a rise of `ONSET_THRESHOLD_RATIO` (plus a small floor, so
/// near-silence doesn't false-trigger) marks an onset.
const ONSET_ENV_FAST_SECONDS: f32 = 0.005;
const ONSET_ENV_SLOW_SECONDS: f32 = 0.15;
const ONSET_THRESHOLD_RATIO: f32 = 1.3;
const ONSET_THRESHOLD_FLOOR: f32 = 0.01;
/// Tunnel D's envelope follower on the raw input, driving its
/// "envelope-triggered" loop-length breathing.
const INPUT_ENVELOPE_SECONDS: f32 = 0.05;
/// Arp variant B's per-step playback speeds, cycling if there are
/// more steps (Activity) than entries here.
const ARP_STEP_RATES: [f32; 4] = [1.0, 1.5, 2.0, 0.75];

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * (max - min) * 0.05).clamp(min, max);
    value.set(next);
}

/// Which tap-time arrangement a preset uses -- Warp always reuses
/// Pattern A's (see module doc comment).
fn tap_ratios_for(preset: usize) -> [f32; MAX_TAPS] {
    let (effect, variant) = effect_for_preset(preset);
    match effect {
        Effect::Pattern => PATTERN_RATIOS[variant],
        _ => PATTERN_RATIOS[0],
    }
}

/// A tap's gain falloff for Warp/Mosaic/Seq -- fixed and
/// shape-independent (those hand Shape to their own per-variant
/// effect instead), unlike Pattern's shape-driven contour. Also
/// normalized by `1/sqrt(num_taps)` so stacking more loop layers
/// (Mosaic/Seq) doesn't get proportionally louder.
fn flat_tap_gain(t: usize, num_taps: usize) -> f32 {
    let taper = (1.0 - t as f32 / num_taps.max(1) as f32 * 0.5).max(0.3);
    taper / (num_taps as f32).sqrt().max(1.0)
}

/// Linear-interpolated read from a circular buffer at a fractional
/// sample position (wrapping, and tolerant of a negative `pos`).
fn read_interp(buf: &[f32], pos: f32) -> f32 {
    let len = buf.len() as f32;
    let p = pos.rem_euclid(len);
    // `rem_euclid` guarantees a result in [0, len) with exact
    // arithmetic, but at f32 precision (buf.len() here can be in the
    // hundreds of thousands) it can round to exactly `len` -- clamp
    // defensively rather than indexing out of bounds on that edge.
    let i0 = (p as usize).min(buf.len() - 1);
    let i1 = (i0 + 1) % buf.len();
    let frac = p - i0 as f32;
    buf[i0] + (buf[i1] - buf[i0]) * frac
}

/// 0 at phase 0 and 1, peak 1 at phase 0.5 -- the crossfade envelope
/// `pitch_shift_tap`'s two read heads use so neither ever pops when
/// its phase wraps.
fn triangular_window(phase: f32) -> f32 {
    1.0 - (phase * 2.0 - 1.0).abs()
}

/// 0 at phase 0 and 1, smooth peak of 1 at phase 0.5 -- Haze's
/// one-shot retriggering grain envelope (a raised-cosine/Hann
/// window), softer-edged than `triangular_window`.
fn hann(phase: f32) -> f32 {
    0.5 - 0.5 * (phase * TAU).cos()
}

/// A minimal delay-line pitch shifter (the classic "dual read-head"
/// technique): two read heads a half-grain apart, each drifting away
/// from `base_delay` at `rate` relative to real time, crossfaded with
/// a triangular window so neither ever pops when it wraps back to the
/// start of its grain. `phase` is this tap's persistent grain-cycle
/// position (0..1), advanced by exactly one sample's worth each call.
/// `rate` may vary from call to call (Glide relies on this) -- it's
/// just this instant's playback-speed multiple, not baked in.
fn pitch_shift_tap(buf: &[f32], write_pos: usize, base_delay: f32, rate: f32, phase: &mut f32, window: f32) -> f32 {
    *phase = (*phase + (rate - 1.0) / window).rem_euclid(1.0);
    let phase_b = (*phase + 0.5).rem_euclid(1.0);
    let sample_at = |ph: f32| {
        let offset = base_delay + ph * window - window * 0.5;
        read_interp(buf, write_pos as f32 - offset)
    };
    sample_at(*phase) * triangular_window(*phase) + sample_at(phase_b) * triangular_window(phase_b)
}

/// Seamlessly loops through a *frozen* buffer -- Sampler's Held state.
/// Two read heads a half-buffer apart, crossfaded the same way
/// `pitch_shift_tap`'s two heads are, so the loop seam never clicks.
/// Unlike `pitch_shift_tap`, there's no moving write head to track
/// drift against (the buffer isn't being written to while frozen), so
/// `phase` just directly is the loop position, advanced by exactly
/// one sample's worth of real time each call.
fn read_frozen_loop(buf: &[f32], phase: &mut f32) -> f32 {
    let len = buf.len() as f32;
    *phase = (*phase + 1.0 / len).rem_euclid(1.0);
    let phase_b = (*phase + 0.5).rem_euclid(1.0);
    read_interp(buf, *phase * len) * triangular_window(*phase) + read_interp(buf, phase_b * len) * triangular_window(phase_b)
}

/// Amplitude-quantizes `x` to `levels` steps across -1..1 -- Seq D's
/// bit-crusher. `levels` isn't literal bit depth, just a step count,
/// but it gives the same "staircase" character.
fn bitcrush(x: f32, levels: f32) -> f32 {
    let levels = levels.max(1.0);
    (x * levels).round() / levels
}

/// Chamberlin state-variable filter, one step -- returns (new_lp,
/// new_bp, bandpass_output). Shared by Warp B and Seq A/C so there's
/// one tested implementation of the resonant bandpass, not three.
fn svf_bandpass_step(input: f32, lp: f32, bp: f32, f_coef: f32, q: f32) -> (f32, f32, f32) {
    let new_lp = lp + f_coef * bp;
    let high = input - new_lp - (1.0 / q) * bp;
    let new_bp = bp + f_coef * high;
    (new_lp, new_bp, new_bp)
}

/// A ping-pong triangle wave from a monotonically-advancing 0..1
/// `phase` -- 0 at phase 0, 1 at phase 0.5, back to 0 at phase 1.
/// `shape` (0..1) blends it toward a sine-eased version of the same
/// ramp -- Glide's documented use of Shape as "the shape of the Glide
/// pattern".
fn glide_triangle(phase: f32, shape: f32) -> f32 {
    let linear = 1.0 - (phase * 2.0 - 1.0).abs();
    let eased = (linear * PI * 0.5).sin();
    linear * (1.0 - shape) + eased * shape
}

/// A single feedback comb filter -- one of Space's parallel resonators.
/// `feedback` is fixed below 1.0 (see `REVERB_COMB_FEEDBACK`), so this
/// is always a contraction regardless of how long it's left running.
#[derive(Clone)]
struct Comb {
    buf: Vec<f32>,
    pos: usize,
    feedback: f32,
}

impl Comb {
    fn new(len: usize, feedback: f32) -> Self {
        Self { buf: vec![0.0; len.max(1)], pos: 0, feedback }
    }

    fn process(&mut self, input: f32) -> f32 {
        let out = self.buf[self.pos];
        self.buf[self.pos] = input + out * self.feedback;
        self.pos = (self.pos + 1) % self.buf.len();
        out
    }
}

/// A single allpass diffuser -- Space's second stage, after the combs,
/// smearing their comb-y resonance into a denser tail.
#[derive(Clone)]
struct Allpass {
    buf: Vec<f32>,
    pos: usize,
    feedback: f32,
}

impl Allpass {
    fn new(len: usize, feedback: f32) -> Self {
        Self { buf: vec![0.0; len.max(1)], pos: 0, feedback }
    }

    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buf[self.pos];
        let out = -input * self.feedback + buffered;
        self.buf[self.pos] = input + buffered * self.feedback;
        self.pos = (self.pos + 1) % self.buf.len();
        out
    }
}

/// One stereo side of Space's reverb tail: 4 parallel combs summed,
/// then run in series through 2 allpasses -- the standard Freeverb
/// topology. `new` takes a `stereo_offset` (0 for left, a small
/// constant for right) so the two channels decorrelate without a
/// second hand-tuned length table.
#[derive(Clone)]
struct ReverbChannel {
    combs: [Comb; REVERB_COMB_COUNT],
    allpasses: [Allpass; REVERB_ALLPASS_COUNT],
}

impl ReverbChannel {
    fn new(sample_rate: f32, stereo_offset: usize) -> Self {
        let scale = sample_rate / REVERB_REFERENCE_RATE;
        Self {
            combs: std::array::from_fn(|i| {
                Comb::new(((REVERB_COMB_LENGTHS[i] + stereo_offset) as f32 * scale) as usize, REVERB_COMB_FEEDBACK)
            }),
            allpasses: std::array::from_fn(|i| {
                Allpass::new(((REVERB_ALLPASS_LENGTHS[i] + stereo_offset) as f32 * scale) as usize, REVERB_ALLPASS_FEEDBACK)
            }),
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let mut sum = 0.0;
        for comb in self.combs.iter_mut() {
            sum += comb.process(input);
        }
        sum *= 1.0 / REVERB_COMB_COUNT as f32;
        for allpass in self.allpasses.iter_mut() {
            sum = allpass.process(sum);
        }
        sum
    }
}

/// One saved snapshot of Prism's main knobs -- see `Selection::UserPreset`.
/// Session-only: like every other piece of state in this sim, nothing
/// is written to disk, so slots reset when the program restarts.
#[derive(Clone, Copy, Default)]
struct UserPresetData {
    preset: u32,
    time: f32,
    taps: usize,
    repeats: f32,
    shape: f32,
    filter: f32,
    mix: f32,
    space: f32,
    pitch_mod_on: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    InputLevel(usize),
    Preset,
    Time,
    Activity,
    Repeats,
    Shape,
    Filter,
    Mix,
    Space,
    PitchMod,
    UtilityMode,
    UtilityTrigger,
    UserPreset(usize),
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 4;

/// Handles for main.rs to drive directly from fixed MIDI CC numbers
/// (see `handle_midi_message` in main.rs) -- threaded in from outside
/// exactly like plaits.rs's `cutoff`, so the atomic a hardware CC
/// writes to is the *same* one `Params` reads, not a separate "MIDI
/// value" that would need reconciling with the knob's own value.
/// Activity/Preset are deliberately left out -- they're discrete
/// (tap count / a preset index), not the kind of continuous 0..127
/// sweep a CC knob suits.
pub struct PrismCcTargets {
    pub time: Arc<AtomicF32>,
    pub repeats: Arc<AtomicF32>,
    pub shape: Arc<AtomicF32>,
    pub filter: Arc<AtomicF32>,
    pub mix: Arc<AtomicF32>,
    pub space: Arc<AtomicF32>,
}

impl PrismCcTargets {
    pub fn new() -> Self {
        Self {
            time: Arc::new(AtomicF32::new(DEFAULT_TIME)),
            repeats: Arc::new(AtomicF32::new(DEFAULT_REPEATS)),
            shape: Arc::new(AtomicF32::new(0.4)),
            filter: Arc::new(AtomicF32::new(1.0)),
            mix: Arc::new(AtomicF32::new(0.5)),
            space: Arc::new(AtomicF32::new(DEFAULT_SPACE)),
        }
    }
}

impl Default for PrismCcTargets {
    fn default() -> Self {
        Self::new()
    }
}

struct Params {
    input_levels: [AtomicF32; MAX_INPUTS],
    ext_input_level: [Arc<AtomicF32>; MAX_INPUTS],
    preset: AtomicU32,
    time: Arc<AtomicF32>,
    taps: AtomicUsize,
    repeats: Arc<AtomicF32>,
    shape: Arc<AtomicF32>,
    filter: Arc<AtomicF32>,
    mix: Arc<AtomicF32>,
    ext_mix: Arc<AtomicF32>,
    space: Arc<AtomicF32>,
    ext_space: Arc<AtomicF32>,
    pitch_mod_on: AtomicBool,
    /// Which footswitch-style utility (if any) owns the wet signal
    /// path right now -- see `UtilityMode`.
    utility_mode: AtomicU32,
    /// Looper's record/play state, 0=Idle 1=Recording 2=Playing --
    /// plain `u32` rather than an enum so it can live in an atomic and
    /// be flipped from both the UI thread (the trigger action) and the
    /// audio thread (auto-stopping when the buffer fills).
    looper_state: AtomicU32,
    /// Sampler's state, 0=Idle (continuously updating its rolling
    /// window) 1=Held (frozen, looping).
    sampler_state: AtomicU32,
    /// Session-only saved knob snapshots -- see `Selection::UserPreset`
    /// and `UserPresetData`. UI-thread-only; the audio thread never
    /// touches this.
    user_presets: [Mutex<Option<UserPresetData>>; NUM_USER_PRESETS],
    /// This app's rendered mono output, republished every block for
    /// another app (Clouds) to tap -- see audio_bus.rs.
    bus_out: Arc<Mutex<Vec<f32>>>,
    /// This app's channel fader in the Mixer app, plus its own
    /// modulation input -- see mixer_bus.rs.
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    /// Test-only convenience -- real construction always goes through
    /// `new_with_cc` (see `PrismApp::new`), so main.rs's MIDI CC
    /// handler shares the exact atomics `Params` reads rather than a
    /// separate set of defaults.
    #[cfg(test)]
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        Self::new_with_cc(modbus, audio_bus, mixer_bus, &PrismCcTargets::new())
    }

    fn new_with_cc(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus, cc: &PrismCcTargets) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Prism", modbus);
        Self {
            input_levels: std::array::from_fn(|_| AtomicF32::new(0.0)),
            ext_input_level: std::array::from_fn(|i| modbus.register(format!("Prism: Input {} Level", i + 1))),
            preset: AtomicU32::new(0),
            time: Arc::clone(&cc.time),
            taps: AtomicUsize::new(DEFAULT_TAPS),
            repeats: Arc::clone(&cc.repeats),
            shape: Arc::clone(&cc.shape),
            filter: Arc::clone(&cc.filter), // fully open by default -- matches Microcosm's "100% clockwise = bypassed"
            mix: Arc::clone(&cc.mix),
            ext_mix: modbus.register("Prism: Mix".to_string()),
            space: Arc::clone(&cc.space),
            ext_space: modbus.register("Prism: Space".to_string()),
            pitch_mod_on: AtomicBool::new(false),
            utility_mode: AtomicU32::new(0),
            looper_state: AtomicU32::new(0),
            sampler_state: AtomicU32::new(0),
            user_presets: std::array::from_fn(|_| Mutex::new(None)),
            bus_out: audio_bus.register("Prism"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct PrismApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    audio_bus: Arc<AudioBus>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

impl PrismApp {
    /// A Prism with its own private, unshared CC targets -- convenient
    /// for tests and any caller that doesn't need MIDI CC to reach it.
    /// registry.rs uses `new_with_cc` instead, so main.rs's MIDI
    /// handler shares the *same* atomics this app's `Params` reads.
    #[allow(dead_code)]
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self::new_with_cc(sensitivity, nav_speed, modbus, audio_bus, mixer_bus, &PrismCcTargets::new())
    }

    pub fn new_with_cc(
        sensitivity: Arc<AtomicF32>,
        nav_speed: Arc<AtomicF32>,
        modbus: Arc<ModBus>,
        audio_bus: Arc<AudioBus>,
        mixer_bus: Arc<MixerBus>,
        cc: &PrismCcTargets,
    ) -> Self {
        Self {
            params: Arc::new(Params::new_with_cc(&modbus, &audio_bus, &mixer_bus, cc)),
            sensitivity,
            nav_speed,
            audio_bus,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
        }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => (0..self.audio_bus.len().min(MAX_INPUTS)).map(Selection::InputLevel).collect(),
            1 => vec![
                Selection::Preset,
                Selection::Time,
                Selection::Activity,
                Selection::Repeats,
                Selection::Shape,
                Selection::Filter,
                Selection::Mix,
                Selection::Space,
                Selection::PitchMod,
            ],
            2 => vec![Selection::UtilityMode, Selection::UtilityTrigger],
            _ => (0..NUM_USER_PRESETS).map(Selection::UserPreset).collect(),
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

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::InputLevel(i) => self.audio_bus.names().get(i).cloned().unwrap_or_else(|| format!("Input {}", i + 1)),
            Selection::Preset => "Preset".into(),
            Selection::Time => "Time".into(),
            Selection::Activity => "Activity".into(),
            Selection::Repeats => "Repeats".into(),
            Selection::Shape => "Shape".into(),
            Selection::Filter => "Filter".into(),
            Selection::Mix => "Mix".into(),
            Selection::Space => "Space".into(),
            Selection::PitchMod => "Pitch Mod".into(),
            Selection::UtilityMode => "Mode".into(),
            Selection::UtilityTrigger => "Trigger".into(),
            Selection::UserPreset(i) => format!("Slot {}", i + 1),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::InputLevel(i) => format!("{:.2}", self.params.input_levels[i].get()),
            Selection::Preset => {
                let idx = self.params.preset.load(Ordering::Relaxed) as usize % PRESET_NAMES.len();
                PRESET_NAMES[idx].to_string()
            }
            Selection::Time => format!("{:.0} ms", self.params.time.get() * 1000.0),
            Selection::Activity => format!("{} taps", self.params.taps.load(Ordering::Relaxed)),
            Selection::Repeats => format!("{:.0}%", self.params.repeats.get() * 100.0),
            Selection::Shape => format!("{:.2}", self.params.shape.get()),
            Selection::Filter => format!("{:.2}", self.params.filter.get()),
            Selection::Mix => format!("{:.2}", self.params.mix.get()),
            Selection::Space => format!("{:.2}", self.params.space.get()),
            Selection::PitchMod => {
                if self.params.pitch_mod_on.load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::UtilityMode => UtilityMode::from_u32(self.params.utility_mode.load(Ordering::Relaxed)).name().to_string(),
            Selection::UtilityTrigger => self.utility_trigger_label(),
            Selection::UserPreset(i) => {
                if self.params.user_presets[i].lock().unwrap().is_some() { "saved".into() } else { "empty".into() }
            }
        }
    }

    /// What pressing/turning the Utility group's Trigger row will do,
    /// given the current mode and looper/sampler state -- purely
    /// descriptive, `edit`/`reset` do the actual state advance.
    fn utility_trigger_label(&self) -> String {
        match UtilityMode::from_u32(self.params.utility_mode.load(Ordering::Relaxed)) {
            UtilityMode::Off => "-- (pick a Mode first)".into(),
            UtilityMode::Looper => match self.params.looper_state.load(Ordering::Relaxed) {
                0 => "press: start recording".into(),
                1 => "recording -- press: play loop".into(),
                _ => "playing -- press: clear".into(),
            },
            UtilityMode::Sampler => match self.params.sampler_state.load(Ordering::Relaxed) {
                0 => "press: freeze".into(),
                _ => "held -- press: release".into(),
            },
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => {
                let active = (0..self.audio_bus.len().min(MAX_INPUTS)).filter(|&i| self.params.input_levels[i].get() > 0.0).count();
                format!("{active} active")
            }
            // Just the preset name -- tap count already has its own
            // Activity leaf right below this once expanded, and
            // repeating it here was part of what pushed this row's
            // width into the tap-time panel at `axis_x0` once
            // `paramlist::TEXT_SCALE` is applied (see
            // `list_text_never_reaches_the_right_panel`).
            1 => {
                let idx = self.params.preset.load(Ordering::Relaxed) as usize % PRESET_NAMES.len();
                PRESET_NAMES[idx].to_string()
            }
            2 => UtilityMode::from_u32(self.params.utility_mode.load(Ordering::Relaxed)).name().to_string(),
            _ => {
                let saved = self.params.user_presets.iter().filter(|s| s.lock().unwrap().is_some()).count();
                format!("{saved}/{NUM_USER_PRESETS} saved")
            }
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::InputLevel(i) => bump(&self.params.input_levels[i], delta, sensitivity, 0.0, 1.0),
            Selection::Preset => {
                let cur = self.params.preset.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(PRESET_NAMES.len() as i32);
                self.params.preset.store(next as u32, Ordering::Relaxed);
            }
            Selection::Time => bump(&self.params.time, delta, sensitivity, MIN_TIME, MAX_TIME),
            Selection::Activity => {
                let cur = self.params.taps.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_TAPS as i32, MAX_TAPS as i32);
                self.params.taps.store(next as usize, Ordering::Relaxed);
            }
            Selection::Repeats => bump(&self.params.repeats, delta, sensitivity, 0.0, MAX_REPEATS),
            Selection::Shape => bump(&self.params.shape, delta, sensitivity, 0.0, 1.0),
            Selection::Filter => bump(&self.params.filter, delta, sensitivity, 0.0, 1.0),
            Selection::Mix => bump(&self.params.mix, delta, sensitivity, 0.0, 1.0),
            Selection::Space => bump(&self.params.space, delta, sensitivity, 0.0, 1.0),
            Selection::PitchMod => self.params.pitch_mod_on.store(step > 0, Ordering::Relaxed),
            Selection::UtilityMode => {
                let cur = self.params.utility_mode.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(3);
                self.params.utility_mode.store(next as u32, Ordering::Relaxed);
                // Switching modes drops whatever the previous one was
                // mid-way through -- a half-recorded loop or a frozen
                // sample would otherwise linger, silently active,
                // under a mode selector that no longer says so.
                self.params.looper_state.store(0, Ordering::Relaxed);
                self.params.sampler_state.store(0, Ordering::Relaxed);
            }
            Selection::UtilityTrigger => {} // press (see `reset`), not turn, advances it -- there's no continuous value here to sweep
            Selection::UserPreset(i) => {
                if step > 0 {
                    self.save_user_preset(i);
                } else {
                    self.load_user_preset(i);
                }
            }
        }
    }

    /// Saves the current knob state into user preset slot `i`.
    fn save_user_preset(&self, i: usize) {
        let data = UserPresetData {
            preset: self.params.preset.load(Ordering::Relaxed),
            time: self.params.time.get(),
            taps: self.params.taps.load(Ordering::Relaxed),
            repeats: self.params.repeats.get(),
            shape: self.params.shape.get(),
            filter: self.params.filter.get(),
            mix: self.params.mix.get(),
            space: self.params.space.get(),
            pitch_mod_on: self.params.pitch_mod_on.load(Ordering::Relaxed),
        };
        *self.params.user_presets[i].lock().unwrap() = Some(data);
    }

    /// Recalls user preset slot `i`'s knob state, if it has one saved.
    fn load_user_preset(&self, i: usize) {
        let Some(data) = *self.params.user_presets[i].lock().unwrap() else { return };
        self.params.preset.store(data.preset, Ordering::Relaxed);
        self.params.time.set(data.time);
        self.params.taps.store(data.taps, Ordering::Relaxed);
        self.params.repeats.set(data.repeats);
        self.params.shape.set(data.shape);
        self.params.filter.set(data.filter);
        self.params.mix.set(data.mix);
        self.params.space.set(data.space);
        self.params.pitch_mod_on.store(data.pitch_mod_on, Ordering::Relaxed);
    }

    /// The Utility group's Trigger row advances whichever state
    /// machine the current Mode owns -- see `UtilityMode`,
    /// `leaf_value`'s `utility_trigger_label`, and `process()`'s
    /// looper/sampler handling for what each state actually does to
    /// the audio.
    fn advance_utility_trigger(&self) {
        match UtilityMode::from_u32(self.params.utility_mode.load(Ordering::Relaxed)) {
            UtilityMode::Off => {}
            UtilityMode::Looper => {
                let next = (self.params.looper_state.load(Ordering::Relaxed) + 1) % 3;
                self.params.looper_state.store(next, Ordering::Relaxed);
            }
            UtilityMode::Sampler => {
                let next = (self.params.sampler_state.load(Ordering::Relaxed) + 1) % 2;
                self.params.sampler_state.store(next, Ordering::Relaxed);
            }
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::InputLevel(i) => self.params.input_levels[i].set(0.0),
            Selection::Time => self.params.time.set(DEFAULT_TIME),
            Selection::Activity => self.params.taps.store(DEFAULT_TAPS, Ordering::Relaxed),
            Selection::Repeats => self.params.repeats.set(DEFAULT_REPEATS),
            Selection::Shape => self.params.shape.set(0.4),
            Selection::Filter => self.params.filter.set(1.0),
            Selection::Mix => self.params.mix.set(0.5),
            Selection::Space => self.params.space.set(DEFAULT_SPACE),
            Selection::PitchMod => self.params.pitch_mod_on.store(false, Ordering::Relaxed),
            Selection::UtilityTrigger => self.advance_utility_trigger(),
            Selection::UserPreset(i) => *self.params.user_presets[i].lock().unwrap() = None,
            Selection::Preset | Selection::UtilityMode => {} // no single sensible default among equal options
        }
    }
}

impl App for PrismApp {
    fn tick(&mut self, input: &Input) {
        let rows = self.visible_rows();
        self.list.navigate(input.knob1, rows.len(), self.nav_speed.get() as i32);
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

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(PrismProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            input_mono: Vec::new(),
            output_mono: Vec::new(),
            reverb_out_l: Vec::new(),
            reverb_out_r: Vec::new(),
            delay_buf: Vec::new(),
            write_pos: 0,
            lp_state: 0.0,
            lfo_phase: 0.0,
            tap_states: std::array::from_fn(TapState::new),
            onset_env_fast: 0.0,
            onset_env_slow: 0.0,
            onset_ages: [0.0; MAX_TAPS],
            input_envelope: 0.0,
            glitch_phase: 0.0,
            glitch_step: 0,
            glitch_bursting: false,
            reverb_l: None,
            reverb_r: None,
            pitch_mod_buf: Vec::new(),
            pitch_mod_write_pos: 0,
            pitch_mod_lfo_phase: 0.0,
            looper_buf: Vec::new(),
            looper_write_pos: 0,
            looper_len: 0,
            looper_read_pos: 0,
            sampler_buf: Vec::new(),
            sampler_write_pos: 0,
            sampler_grain_phase: 0.0,
            prev_looper_state: 0,
            prev_sampler_state: 0,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let title = MonoTextStyle::new(&SPLEEN_16X32, Rgb565::WHITE);
        Text::new("Prism", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(0, 63, 10));
        let dim = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));

        let preset = self.params.preset.load(Ordering::Relaxed) as usize % PRESET_NAMES.len();
        let (effect, _variant) = effect_for_preset(preset);

        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    let name = match *g {
                        0 => "Mixer",
                        1 => category_name(effect),
                        2 => "Utility",
                        _ => "User Presets",
                    };
                    (format!("{arrow} {name}"), self.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", self.leaf_name(*sel)), self.leaf_value(*sel)),
            })
            .collect();
        self.list.draw(fb, 16, 44, 24, 10, &display_rows);

        // --- Right: for Multidelay, a tap-time map (unchanged); for
        // Micro Loop, the same axis repurposed to show each layer's
        // playback-speed multiple instead of its delay time. ---
        let num_taps = self.params.taps.load(Ordering::Relaxed).clamp(MIN_TAPS, MAX_TAPS);
        let time = self.params.time.get();
        let shape = self.params.shape.get();

        let axis_x0 = 380;
        let axis_x1 = 600;
        let axis_y = 200;
        Line::new(Point::new(axis_x0, axis_y), Point::new(axis_x1, axis_y))
            .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(12, 24, 12), 1))
            .draw(fb)
            .ok();

        let (values, caption): (Vec<f32>, String) = match effect {
            Effect::Pattern | Effect::Warp => {
                let ratios = tap_ratios_for(preset);
                let vals: Vec<f32> = ratios[..num_taps].to_vec();
                let longest = vals.iter().cloned().fold(0.0f32, f32::max);
                (vals, format!("{} taps -- longest {:.0} ms", num_taps, time * longest * 1000.0))
            }
            Effect::Mosaic | Effect::Seq => {
                let rate_set = if effect == Effect::Mosaic { MOSAIC_RATES[preset % VARIANTS_PER_EFFECT] } else { [SEQ_RATES[0], SEQ_RATES[1], SEQ_RATES[0], SEQ_RATES[1]] };
                let vals: Vec<f32> = (0..num_taps).map(|t| rate_set[t % rate_set.len()]).collect();
                (vals, format!("{} layers -- loop {:.0} ms", num_taps, time * 1000.0))
            }
            Effect::Glide => {
                let (rate_lo, rate_hi, _) = glide_rates_for(preset % VARIANTS_PER_EFFECT);
                (vec![rate_lo, rate_hi], format!("gliding {:.2}x <-> {:.2}x", rate_lo, rate_hi))
            }
            Effect::Haze => {
                // Grain start/rate are randomized per retrigger on
                // the audio thread, not something the UI side has a
                // live view of -- just show how many grain slots are
                // active, evenly spaced, as a density indicator.
                let vals: Vec<f32> = (0..num_taps).map(|t| t as f32 + 1.0).collect();
                (vals, format!("{} grain slots -- {:.0} ms each", num_taps, time * 1000.0))
            }
            Effect::Tunnel => (vec![1.0], format!("drone loop -- {:.0} ms", time * 1000.0)),
            Effect::Strum => {
                let vals: Vec<f32> = (0..num_taps).map(|t| t as f32 + 1.0).collect();
                (vals, format!("{} onset layers", num_taps))
            }
            Effect::Blocks => {
                let vals: Vec<f32> = (0..num_taps).map(|t| t as f32 + 1.0).collect();
                (vals, format!("{} staggered blocks -- {:.0} ms loop", num_taps, time * 1000.0))
            }
            Effect::Interrupt => (vec![1.0], format!("burst rate {:.0}% -- {:.0} ms loop", self.params.repeats.get() * 100.0, time * 1000.0)),
            Effect::Arp => {
                let vals: Vec<f32> = (0..num_taps).map(|t| t as f32 + 1.0).collect();
                (vals, format!("{} step arp -- {:.0} ms/step", num_taps, time * 1000.0))
            }
        };
        let max_val = values.iter().cloned().fold(0.0f32, f32::max).max(0.001);

        for (t, &v) in values.iter().enumerate() {
            let frac = v / max_val;
            let x = axis_x0 + (frac * (axis_x1 - axis_x0) as f32) as i32;
            let gain = match effect {
                Effect::Pattern => (1.0 - (t as f32 / num_taps.max(1) as f32) * shape).max(0.05),
                _ => flat_tap_gain(t, values.len().max(1)),
            };
            let h = (gain * 60.0) as i32;
            let bright = (gain * 63.0) as u8;
            Line::new(Point::new(x, axis_y), Point::new(x, axis_y - h))
                .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(0, bright, bright / 6), 2))
                .draw(fb)
                .ok();
        }

        Text::new(&caption, Point::new(380, 230), accent).draw(fb).ok();

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(Selection::UserPreset(_))) => "knob2: turn one way to save, the other to load   press knob2: clear".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

/// Glide's two fixed rate extremes per variant, and which layer(s)
/// glide -- (rate_lo, rate_hi, both_directions). `both_directions`
/// (D only) means a second layer glides the opposite way at the same
/// time, rather than both layers doing the same thing in unison.
fn glide_rates_for(variant: usize) -> (f32, f32, bool) {
    match variant {
        0 => (0.5, 1.0, false), // A: half <-> normal
        1 => (2.0, 0.5, false), // B: double <-> half
        2 => (1.0, 2.0, false), // C: normal <-> double
        _ => (1.0, 2.0, true),  // D: one layer up, one down, at once
    }
}

/// Per-tap persistent DSP state. Not every field is used by every
/// effect -- see `process()` for which effect touches what.
#[derive(Clone, Copy)]
struct TapState {
    /// One-pole lowpass state for Warp A's envelope-shrinking filter.
    env_lp: f32,
    /// Chamberlin state-variable filter's two states, for Warp B and
    /// Seq A/C's resonant bandpass (same center frequency/Q for every
    /// tap within one block).
    svf_bp: f32,
    svf_lp: f32,
    /// Grain-cycle phase (0..1) for Warp C/D's pitch shifter, and for
    /// Mosaic/Seq/Glide's loop-window reader (same function either
    /// way -- see `pitch_shift_tap`).
    grain_phase: f32,
    /// Haze's one-shot retriggering grain: `grain_age` (0..1, wraps to
    /// 0 and re-randomizes `grain_start`/`grain_rate` on each
    /// retrigger) is a *different* mechanic from `grain_phase` above
    /// -- a single Hann-windowed burst that restarts with fresh
    /// random parameters each cycle, not a continuous dual-head
    /// crossfade. `grain_start`/`grain_rate` persist between
    /// retriggers so a slot keeps sounding the same until its next one.
    grain_age: f32,
    grain_start: f32,
    grain_rate: f32,
    /// Independent per-slot RNG for retrigger randomization -- seeded
    /// non-zero per index in `TapState::new` (never left at the
    /// `Default`-derived 0, which would make xorshift stick at 0
    /// forever).
    rng: u32,
}

impl TapState {
    /// `i` staggers `grain_age`'s starting phase across slots (so
    /// Haze's slots don't all retrigger in lockstep) and seeds `rng`
    /// distinctly per slot.
    fn new(i: usize) -> Self {
        Self {
            env_lp: 0.0,
            svf_bp: 0.0,
            svf_lp: 0.0,
            grain_phase: 0.0,
            grain_age: i as f32 / MAX_TAPS as f32,
            grain_start: 0.0,
            grain_rate: 1.0,
            rng: 0x9E3779B9 ^ ((i as u32 + 1).wrapping_mul(0x85EBCA6B)) | 1,
        }
    }

    fn next_rand01(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }
}

struct PrismProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    input_mono: Vec<f32>,
    output_mono: Vec<f32>,
    /// Per-block scratch for Space's stereo reverb tail -- carries
    /// each sample's L/R reverb output from the main per-sample loop
    /// (where the combs/allpasses actually advance) to the final
    /// device-buffer write, which needs both `output_mono` and these
    /// to build a genuinely stereo frame. Not persistent DSP state
    /// itself -- that lives in `reverb_l`/`reverb_r` below.
    reverb_out_l: Vec<f32>,
    reverb_out_r: Vec<f32>,
    /// The single shared delay/loop buffer every tap or layer reads
    /// from. Sized lazily on the first `process()` call, once
    /// `sample_rate` is known.
    delay_buf: Vec<f32>,
    write_pos: usize,
    /// One-pole lowpass state for Pattern's shared wet-signal filter.
    lp_state: f32,
    /// Shared slow LFO phase (0..1, wrapping) -- Seq B's speed flip,
    /// Seq C's filter sweep, and Glide's rate glide each read this
    /// the same way (a continuously-advancing phase) but interpret it
    /// differently; never more than one of those runs at once, so one
    /// field covers all three.
    lfo_phase: f32,
    tap_states: [TapState; MAX_TAPS],
    /// Strum's onset detector: a fast envelope compared against a
    /// slower-following reference -- a rising edge on the fast one
    /// relative to the slow one marks an onset. See `process()`.
    onset_env_fast: f32,
    onset_env_slow: f32,
    /// How many samples ago each of the last `MAX_TAPS` onsets
    /// happened, most recent first -- shifted along (not reset) on
    /// every new onset, aged by 1.0 every sample regardless. Strum's
    /// layers each anchor to one of these instead of a fixed delay
    /// time, which is what makes it track *onsets* rather than a
    /// fixed rhythm.
    onset_ages: [f32; MAX_TAPS],
    /// Tunnel D's envelope follower on the raw input -- drives its
    /// "envelope-triggered" loop-length breathing.
    input_envelope: f32,
    /// Glitch's shared cycle clock: `glitch_phase` (0..1) is progress
    /// through the current cycle (one cycle = `loop_window` samples);
    /// `glitch_step` increments (wrapping) every time `glitch_phase`
    /// wraps, giving Blocks/Interrupt/Arp a discrete "which cycle is
    /// this" counter to key per-cycle randomization/sequencing off
    /// of. Never used by anything outside Glitch.
    glitch_phase: f32,
    glitch_step: usize,
    /// Interrupt's per-cycle decision of whether this cycle is
    /// "bursting" -- gates the dry signal per the manual's documented
    /// Mix=100% behavior (see `Effect::Interrupt` in `process()`).
    glitch_bursting: bool,
    /// Space's stereo reverb tail -- `None` until the first `process()`
    /// call, once `sample_rate` is known (see `ReverbChannel::new`).
    reverb_l: Option<ReverbChannel>,
    reverb_r: Option<ReverbChannel>,
    /// Pitch Mod's chorus/vibrato delay line, and the LFO phase
    /// modulating its read position -- independent of `delay_buf`
    /// since this stage runs on every preset, not per-effect state.
    pitch_mod_buf: Vec<f32>,
    pitch_mod_write_pos: usize,
    pitch_mod_lfo_phase: f32,
    /// Looper's recorded phrase, and how many samples of it are
    /// actually valid (<= its capacity) -- see `UtilityMode::Looper`.
    looper_buf: Vec<f32>,
    looper_write_pos: usize,
    looper_len: usize,
    looper_read_pos: usize,
    /// Sampler's rolling window (continuously overwritten while Idle)
    /// and, once frozen, the read phase looping through it -- see
    /// `UtilityMode::Sampler`. Reuses `pitch_shift_tap`'s crossfaded
    /// dual-read-head for a click-free loop seam, same as Haze/Tunnel.
    sampler_buf: Vec<f32>,
    sampler_write_pos: usize,
    sampler_grain_phase: f32,
    /// What the audio thread last saw `Params`'s looper/sampler state
    /// atomic as -- the UI thread's button press is what actually
    /// advances that atomic (`advance_utility_trigger`); comparing
    /// against these each block is just how the audio thread detects
    /// *entering* a new state (Recording/Playing/Held) to reset its
    /// own local position counters exactly once, not every block.
    prev_looper_state: u32,
    prev_sampler_state: u32,
}

impl PrismProcessor {
    /// Pitch Mod's chorus/vibrato stage: a short delay line whose read
    /// offset is swept by a slow sine LFO, blended with the dry signal
    /// -- a fixed depth/rate/mix (see the `PITCH_MOD_*` constants),
    /// since this is a real pedal's always-in-the-chain stage, not one
    /// of its 6 macro knobs.
    fn apply_pitch_mod(&mut self, input: f32, sample_rate: f32) -> f32 {
        let len = self.pitch_mod_buf.len();
        self.pitch_mod_buf[self.pitch_mod_write_pos] = input;
        self.pitch_mod_write_pos = (self.pitch_mod_write_pos + 1) % len;

        self.pitch_mod_lfo_phase = (self.pitch_mod_lfo_phase + PITCH_MOD_RATE_HZ / sample_rate).rem_euclid(1.0);
        let lfo = (self.pitch_mod_lfo_phase * TAU).sin();
        let offset_seconds = PITCH_MOD_CENTER_SECONDS + lfo * PITCH_MOD_DEPTH_SECONDS;
        let offset_samples = (offset_seconds * sample_rate).max(1.0);
        let modulated = read_interp(&self.pitch_mod_buf, self.pitch_mod_write_pos as f32 - offset_samples);

        input * (1.0 - PITCH_MOD_MIX) + modulated * PITCH_MOD_MIX
    }

    /// Space's stereo reverb send -- feeds `input` (already scaled by
    /// the Space knob at the call site) into both of the L/R parallel
    /// comb+allpass chains, returning their independent tails. See
    /// `ReverbChannel` and the module doc comment for why L/R only
    /// decorrelate here, not throughout the whole signal chain.
    fn process_reverb(&mut self, input: f32) -> (f32, f32) {
        let l = self.reverb_l.as_mut().expect("lazily created in process()").process(input);
        let r = self.reverb_r.as_mut().expect("lazily created in process()").process(input);
        (l, r)
    }

    /// Computes the wet signal for whichever utility mode currently
    /// owns the wet path (see `UtilityMode`), called once per sample
    /// in place of the 44-preset `Effect` match. Never called with
    /// `UtilityMode::Off` (the caller keeps that case on the normal
    /// preset path instead), but handled here too for exhaustiveness.
    fn process_utility(&mut self, mode: UtilityMode, input_sample: f32) -> f32 {
        match mode {
            UtilityMode::Off => input_sample,
            UtilityMode::Looper => {
                let state = self.params.looper_state.load(Ordering::Relaxed);
                if state != self.prev_looper_state {
                    match state {
                        1 => self.looper_write_pos = 0,
                        2 => self.looper_read_pos = 0,
                        _ => {}
                    }
                    self.prev_looper_state = state;
                }
                match state {
                    // Recording: capture the dry input (what a real
                    // phrase looper records off its own input, not
                    // its wet tail) -- auto-stopping (and switching to
                    // Playing) once the phrase buffer fills, rather
                    // than silently wrapping and overwriting its own
                    // start.
                    1 => {
                        let len = self.looper_buf.len();
                        self.looper_buf[self.looper_write_pos] = input_sample;
                        self.looper_write_pos += 1;
                        if self.looper_write_pos >= len {
                            self.looper_len = len;
                            self.looper_read_pos = 0;
                            self.params.looper_state.store(2, Ordering::Relaxed);
                            self.prev_looper_state = 2;
                        } else {
                            self.looper_len = self.looper_write_pos;
                        }
                        input_sample
                    }
                    2 => {
                        if self.looper_len == 0 {
                            return input_sample;
                        }
                        let out = self.looper_buf[self.looper_read_pos];
                        self.looper_read_pos = (self.looper_read_pos + 1) % self.looper_len;
                        out
                    }
                    _ => input_sample, // Idle -- armed, passing dry through until triggered
                }
            }
            UtilityMode::Sampler => {
                let state = self.params.sampler_state.load(Ordering::Relaxed);
                if state != self.prev_sampler_state {
                    if state == 1 {
                        self.sampler_grain_phase = 0.0;
                    }
                    self.prev_sampler_state = state;
                }
                if state == 1 {
                    read_frozen_loop(&self.sampler_buf, &mut self.sampler_grain_phase)
                } else {
                    // Idle -- keep the rolling window fresh so
                    // whenever it's frozen, it's freezing something
                    // recent rather than stale silence.
                    let len = self.sampler_buf.len();
                    self.sampler_buf[self.sampler_write_pos] = input_sample;
                    self.sampler_write_pos = (self.sampler_write_pos + 1) % len;
                    input_sample
                }
            }
        }
    }
}

impl AudioProcessor for PrismProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        self.input_mono.clear();
        self.input_mono.resize(frames, 0.0);
        self.output_mono.clear();
        self.output_mono.resize(frames, 0.0);
        self.reverb_out_l.clear();
        self.reverb_out_l.resize(frames, 0.0);
        self.reverb_out_r.clear();
        self.reverb_out_r.resize(frames, 0.0);

        if self.delay_buf.is_empty() {
            self.delay_buf = vec![0.0; (DELAY_BUFFER_SECONDS * sample_rate) as usize];
        }
        if self.reverb_l.is_none() {
            self.reverb_l = Some(ReverbChannel::new(sample_rate, 0));
            self.reverb_r = Some(ReverbChannel::new(sample_rate, REVERB_STEREO_OFFSET));
        }
        if self.pitch_mod_buf.is_empty() {
            self.pitch_mod_buf = vec![0.0; (PITCH_MOD_BUFFER_SECONDS * sample_rate) as usize];
        }
        if self.looper_buf.is_empty() {
            self.looper_buf = vec![0.0; (LOOPER_MAX_SECONDS * sample_rate) as usize];
        }
        if self.sampler_buf.is_empty() {
            self.sampler_buf = vec![0.0; (SAMPLER_WINDOW_SECONDS * sample_rate) as usize];
        }
        let buf_len = self.delay_buf.len();

        // Input mixer: sum every routed source at its own level --
        // each level additionally modulatable via modbus.rs, same
        // additive pattern as everywhere else. See Clouds for the
        // original version of this.
        let num_inputs = self.audio_bus.len().min(MAX_INPUTS);
        for i in 0..num_inputs {
            let level = (self.params.input_levels[i].get() + self.params.ext_input_level[i].get()).clamp(0.0, 2.0);
            if level <= 0.0 {
                continue;
            }
            if let Some(src) = self.audio_bus.get(i) {
                let src = src.lock().unwrap();
                for (m, s) in self.input_mono.iter_mut().zip(src.iter()) {
                    *m += *s * level;
                }
            }
        }

        let preset = self.params.preset.load(Ordering::Relaxed) as usize % PRESET_NAMES.len();
        let (effect, variant) = effect_for_preset(preset);
        let num_taps = self.params.taps.load(Ordering::Relaxed).clamp(MIN_TAPS, MAX_TAPS);
        let time = self.params.time.get().clamp(MIN_TIME, MAX_TIME);
        let repeats = self.params.repeats.get().clamp(0.0, MAX_REPEATS);
        let shape = self.params.shape.get().clamp(0.0, 1.0);
        let filter = self.params.filter.get().clamp(0.0, 1.0);
        let mix = (self.params.mix.get() + self.params.ext_mix.get()).clamp(0.0, 1.0);
        let space_amt = (self.params.space.get() + self.params.ext_space.get()).clamp(0.0, 1.0);
        let pitch_mod_on = self.params.pitch_mod_on.load(Ordering::Relaxed);
        let utility_mode = UtilityMode::from_u32(self.params.utility_mode.load(Ordering::Relaxed));
        let lp_coef = 0.02 + filter * 0.98;
        // Warp B/Seq A/C repurpose Filter as the bandpass's center
        // frequency instead of a shared post-sum lowpass.
        let svf_f_coef = (2.0 * (PI * (200.0 + filter * 3800.0) / sample_rate).sin()).clamp(0.0, 1.0);
        let grain_window = (WARP_GRAIN_WINDOW_SECONDS * sample_rate).max(4.0);
        let loop_window = (time * sample_rate).max(4.0);
        let dt = 1.0 / sample_rate;

        // Only Multidelay uses this at all; harmless to compute either
        // way since it's cheap and Micro Loop just won't reference it.
        let ratios = tap_ratios_for(preset);
        let mut tap_delay = [0usize; MAX_TAPS];
        let mut tap_gain = [0.0f32; MAX_TAPS];
        for t in 0..num_taps {
            let seconds = (time * ratios[t]).max(0.0);
            tap_delay[t] = ((seconds * sample_rate) as usize).min(buf_len.saturating_sub(1));
            tap_gain[t] = match effect {
                Effect::Pattern => (1.0 - (t as f32 / num_taps as f32) * shape).max(0.05),
                _ => flat_tap_gain(t, num_taps),
            };
        }
        let feedback_tap = tap_delay[num_taps.saturating_sub(1)];

        // Seq B's speed flip: a hard 0/1 gate instead of the
        // continuous glide Glide itself uses.
        let lfo_rate = 1.0 / (time * SLOW_LFO_TIME_MULT).max(0.01);
        // Granules-only coefficients -- cheap to compute even when
        // Granules isn't the active effect.
        let onset_fast_coef = 1.0 / (ONSET_ENV_FAST_SECONDS * sample_rate).max(1.0);
        let onset_slow_coef = 1.0 / (ONSET_ENV_SLOW_SECONDS * sample_rate).max(1.0);
        let input_env_coef = 1.0 / (INPUT_ENVELOPE_SECONDS * sample_rate).max(1.0);
        let tunnel_depth = (num_taps as f32 - MIN_TAPS as f32) / (MAX_TAPS - MIN_TAPS) as f32;

        for n in 0..frames {
            let input_sample = self.input_mono[n];
            self.lfo_phase = (self.lfo_phase + lfo_rate * dt).rem_euclid(1.0);

            let mut wet = 0.0f32;
            if utility_mode != UtilityMode::Off {
                wet = self.process_utility(utility_mode, input_sample);
            } else {
            match effect {
                Effect::Pattern => {
                    for t in 0..num_taps {
                        let read_pos = (self.write_pos + buf_len - tap_delay[t]) % buf_len;
                        wet += self.delay_buf[read_pos] * tap_gain[t];
                    }
                }
                Effect::Warp => {
                    for t in 0..num_taps {
                        let read_pos = (self.write_pos + buf_len - tap_delay[t]) % buf_len;
                        let raw = self.delay_buf[read_pos];
                        let processed = match variant {
                            // A: envelope-shrinking lowpass -- Shape
                            // sets how much further each later tap closes.
                            0 => {
                                let coef = (1.0 - (t as f32 / num_taps as f32) * shape).max(0.02);
                                let st = &mut self.tap_states[t];
                                st.env_lp += (raw - st.env_lp) * coef;
                                st.env_lp
                            }
                            // B: resonant bandpass, same center
                            // frequency/Q (from Filter) for every tap.
                            1 => {
                                let st = &mut self.tap_states[t];
                                let (lp, bp, out) = svf_bandpass_step(raw, st.svf_lp, st.svf_bp, svf_f_coef, BANDPASS_Q);
                                st.svf_lp = lp;
                                st.svf_bp = bp;
                                out
                            }
                            // C: pitch shift escalating per tap --
                            // Shape sets semitones of shift *per tap index*.
                            2 => {
                                let semitones = (t + 1) as f32 * shape * 12.0;
                                let rate = 2f32.powf(semitones / 12.0);
                                pitch_shift_tap(&self.delay_buf, self.write_pos, tap_delay[t] as f32, rate, &mut self.tap_states[t].grain_phase, grain_window)
                            }
                            // D: dry tap crossfaded with a fixed
                            // +1-octave "double speed" grain -- Shape
                            // sets the crossfade blend.
                            _ => {
                                let shifted = pitch_shift_tap(
                                    &self.delay_buf,
                                    self.write_pos,
                                    tap_delay[t] as f32,
                                    2.0,
                                    &mut self.tap_states[t].grain_phase,
                                    grain_window,
                                );
                                raw * (1.0 - shape) + shifted * shape
                            }
                        };
                        wet += processed * tap_gain[t];
                    }
                }
                Effect::Mosaic => {
                    let rates = MOSAIC_RATES[variant];
                    for t in 0..num_taps {
                        let rate = rates[t % rates.len()];
                        let layer = pitch_shift_tap(&self.delay_buf, self.write_pos, loop_window * 0.5, rate, &mut self.tap_states[t].grain_phase, loop_window);
                        wet += layer * tap_gain[t];
                    }
                }
                Effect::Seq => {
                    // Seq B flips every active layer's rate together
                    // on the shared slow timer; A/C/D keep the static
                    // per-layer split Mosaic uses, differing only in
                    // the extra processing each applies below.
                    let flip_rate = if self.lfo_phase < 0.5 { SEQ_RATES[0] } else { SEQ_RATES[1] };
                    for t in 0..num_taps {
                        let rate = if variant == 1 { flip_rate } else { SEQ_RATES[t % SEQ_RATES.len()] };
                        let layer = pitch_shift_tap(&self.delay_buf, self.write_pos, loop_window * 0.5, rate, &mut self.tap_states[t].grain_phase, loop_window);
                        let processed = match variant {
                            // A: fixed resonant bandpass per layer.
                            0 => {
                                let st = &mut self.tap_states[t];
                                let (lp, bp, out) = svf_bandpass_step(layer, st.svf_lp, st.svf_bp, svf_f_coef, BANDPASS_Q);
                                st.svf_lp = lp;
                                st.svf_bp = bp;
                                out
                            }
                            // B: the rate flip above is the whole effect.
                            1 => layer,
                            // C: bandpass with a slowly sweeping
                            // center frequency instead of a fixed one.
                            2 => {
                                let sweep_hz = 200.0 + (self.lfo_phase * TAU).sin() * 0.5 * (0.5 + filter) * 1800.0 + 400.0;
                                let sweep_f_coef = (2.0 * (PI * sweep_hz.max(20.0) / sample_rate).sin()).clamp(0.0, 1.0);
                                let st = &mut self.tap_states[t];
                                let (lp, bp, out) = svf_bandpass_step(layer, st.svf_lp, st.svf_bp, sweep_f_coef, BANDPASS_Q);
                                st.svf_lp = lp;
                                st.svf_bp = bp;
                                out
                            }
                            // D: bit-crush, amount from Shape.
                            _ => bitcrush(layer, MAX_BITCRUSH_LEVELS - shape * (MAX_BITCRUSH_LEVELS - MIN_BITCRUSH_LEVELS)),
                        };
                        wet += processed * tap_gain[t];
                    }
                }
                Effect::Glide => {
                    let (rate_lo, rate_hi, both_directions) = glide_rates_for(variant);
                    let pos = glide_triangle(self.lfo_phase, shape);
                    let rate_a = rate_lo + (rate_hi - rate_lo) * pos;
                    let layer_a = pitch_shift_tap(&self.delay_buf, self.write_pos, loop_window * 0.5, rate_a, &mut self.tap_states[0].grain_phase, loop_window);
                    wet += layer_a * flat_tap_gain(0, if both_directions { 2 } else { 1 });
                    if both_directions {
                        // The second layer glides the opposite
                        // direction, using the inverse position so
                        // the two always move apart/together in sync
                        // rather than independently drifting.
                        let rate_b = rate_hi + (rate_lo - rate_hi) * pos;
                        let layer_b = pitch_shift_tap(&self.delay_buf, self.write_pos, loop_window * 0.5, rate_b, &mut self.tap_states[1].grain_phase, loop_window);
                        wet += layer_b * flat_tap_gain(1, 2);
                    }
                }
                Effect::Haze => {
                    // Activity doubles as both density (more slots)
                    // and spread (wider slots implicitly cover more
                    // history) per the manual's "Activity: controls
                    // grain density and spread" -- no separate spread
                    // knob needed.
                    let max_offset = loop_window * (1.0 + num_taps as f32 * 0.5);
                    for t in 0..num_taps {
                        let st = &mut self.tap_states[t];
                        st.grain_age += 1.0 / loop_window;
                        if st.grain_age >= 1.0 {
                            st.grain_age -= 1.0;
                            let r1 = st.next_rand01();
                            let r2 = st.next_rand01();
                            st.grain_rate = match variant {
                                0 => 0.5 + r1 * 0.4,                                  // A: stretched (slower)
                                1 => 0.9 + r1 * 0.2,                                  // B: near-normal; position varies instead
                                2 => if r1 < 1.0 - shape { 1.0 } else { 2.0 },        // C: normal/double mix
                                _ => if r1 < 1.0 - shape { 1.0 } else { 0.5 },        // D: normal/half mix
                            };
                            // Only B randomizes *where* each grain
                            // starts -- A/C/D all pull from roughly
                            // the same recent spot, matching "short,
                            // diffused... stretching" (A) rather than
                            // scattering across all of history.
                            st.grain_start = if variant == 1 { r2 * max_offset } else { max_offset * 0.5 };
                        }
                        let window = hann(st.grain_age);
                        let read_offset = st.grain_start + st.grain_age * loop_window * st.grain_rate;
                        let grain = read_interp(&self.delay_buf, self.write_pos as f32 - read_offset) * window;
                        wet += grain * flat_tap_gain(t, num_taps);
                    }
                }
                Effect::Tunnel => {
                    // Fixed at 2 possible layers regardless of
                    // Activity -- a drone, not a stack. Activity
                    // instead controls "the depth of each modifier"
                    // (per the manual, specific to this effect), same
                    // repurposing precedent as Glide's Activity/Shape.
                    let window = match variant {
                        0 => loop_window * (1.0 + (self.lfo_phase * TAU).sin() * 0.4 * tunnel_depth), // A: breathing length
                        3 => loop_window * (1.0 + self.input_envelope * 2.0 * tunnel_depth),          // D: envelope-triggered length
                        _ => loop_window,
                    }
                    .max(4.0);
                    let rate = if variant == 1 { 0.5 } else { 1.0 }; // B: sub-octave drone
                    let raw = pitch_shift_tap(&self.delay_buf, self.write_pos, window * 0.5, rate, &mut self.tap_states[0].grain_phase, window);

                    let processed = match variant {
                        // B: sub-octave + a slowly sweeping filter.
                        1 => {
                            let sweep_hz = 200.0 + (self.lfo_phase * TAU).sin() * 0.5 * tunnel_depth * 1800.0 + 400.0;
                            let sweep_f_coef = (2.0 * (PI * sweep_hz.max(20.0) / sample_rate).sin()).clamp(0.0, 1.0);
                            let st = &mut self.tap_states[0];
                            let (lp, bp, out) = svf_bandpass_step(raw, st.svf_lp, st.svf_bp, sweep_f_coef, BANDPASS_Q);
                            st.svf_lp = lp;
                            st.svf_bp = bp;
                            out
                        }
                        // C: fixed resonant bandpass (Filter sets frequency).
                        2 => {
                            let st = &mut self.tap_states[0];
                            let (lp, bp, out) = svf_bandpass_step(raw, st.svf_lp, st.svf_bp, svf_f_coef, BANDPASS_Q);
                            st.svf_lp = lp;
                            st.svf_bp = bp;
                            out
                        }
                        // A/D: just the (variably-lengthed) loop itself.
                        _ => raw,
                    };
                    wet += processed;

                    // Kept warm regardless of variant, so switching to
                    // D mid-performance doesn't start from a cold
                    // (zero) envelope.
                    self.input_envelope += (input_sample.abs() - self.input_envelope) * input_env_coef;
                }
                Effect::Strum => {
                    // Onset detector: a fast envelope vs. a
                    // slower-following reference -- a big-enough jump
                    // in the fast one (relative to the slow one) marks
                    // an onset.
                    let rectified = input_sample.abs();
                    self.onset_env_fast += (rectified - self.onset_env_fast) * onset_fast_coef;
                    let onset = self.onset_env_fast > self.onset_env_slow * ONSET_THRESHOLD_RATIO + ONSET_THRESHOLD_FLOOR;
                    self.onset_env_slow += (self.onset_env_fast - self.onset_env_slow) * onset_slow_coef;
                    if onset {
                        for k in (1..MAX_TAPS).rev() {
                            self.onset_ages[k] = self.onset_ages[k - 1];
                        }
                        self.onset_ages[0] = 0.0;
                    }
                    for age in self.onset_ages.iter_mut() {
                        *age += 1.0;
                    }
                    // Never read closer than half a grain window to
                    // "now" -- `pitch_shift_tap`'s window straddles
                    // its `base_delay`, so anything smaller would
                    // wrap into not-yet-written (future) samples.
                    let anchor = |k: usize| self.onset_ages[k].max(loop_window * 0.5);

                    match variant {
                        // A: repeats the most recent onset continuously.
                        0 => {
                            let layer = pitch_shift_tap(&self.delay_buf, self.write_pos, anchor(0), 1.0, &mut self.tap_states[0].grain_phase, loop_window);
                            wet += layer;
                        }
                        // B: many copies of the same onset at slightly
                        // different rates -- phasing; Shape sets how
                        // far the rates spread apart.
                        1 => {
                            let base = anchor(0);
                            for t in 0..num_taps {
                                let spread = (t as f32 / num_taps.max(1) as f32 - 0.5) * shape * 0.4;
                                let rate = 1.0 + spread;
                                let layer = pitch_shift_tap(&self.delay_buf, self.write_pos, base, rate, &mut self.tap_states[t].grain_phase, loop_window);
                                wet += layer * flat_tap_gain(t, num_taps);
                            }
                        }
                        // C/D: a cascade -- each layer anchored to a
                        // *different* past onset. D adds one extra
                        // +1-octave layer riding on the most recent one.
                        _ => {
                            for t in 0..num_taps {
                                let layer = pitch_shift_tap(&self.delay_buf, self.write_pos, anchor(t), 1.0, &mut self.tap_states[t].grain_phase, loop_window);
                                wet += layer * flat_tap_gain(t, num_taps);
                            }
                            if variant == 3 {
                                let last_slot = MAX_TAPS - 1;
                                let shimmer =
                                    pitch_shift_tap(&self.delay_buf, self.write_pos, anchor(0), 2.0, &mut self.tap_states[last_slot].grain_phase, loop_window);
                                wet += shimmer * shape;
                            }
                        }
                    }
                }
                Effect::Blocks => {
                    // A shared cycle clock -- unlike Haze (each slot
                    // free-runs on its own), Blocks' slots retrigger
                    // in a staggered *run* through the layers rather
                    // than a smooth continuous cloud, matching
                    // "rearranges... into sequenced runs".
                    self.glitch_phase = (self.glitch_phase + 1.0 / loop_window).rem_euclid(1.0);
                    for t in 0..num_taps {
                        let local_phase = (self.glitch_phase + t as f32 / num_taps as f32).rem_euclid(1.0);
                        let st = &mut self.tap_states[t];
                        let old_progress = st.grain_age;
                        st.grain_age = local_phase;
                        let retriggered = local_phase < old_progress;
                        if retriggered {
                            let r1 = st.next_rand01();
                            let r2 = st.next_rand01();
                            st.grain_start = r1 * loop_window * 3.0; // jump somewhere in recent history
                            st.grain_rate = if variant == 1 || variant == 3 {
                                2f32.powf((r2 * 24.0 - 12.0) / 12.0) // B/D: pitch-shifted jumps, +/-1 octave
                            } else {
                                1.0
                            };
                        }
                        // C: wider fade -> softer, less angular cuts.
                        let fade_frac: f32 = if variant == 2 { 0.4 } else { 0.05 };
                        let progress = st.grain_age;
                        let fade = (progress / fade_frac).min((1.0 - progress) / fade_frac).clamp(0.0, 1.0);
                        let read_offset = st.grain_start + progress * loop_window * st.grain_rate;
                        let mut slice = read_interp(&self.delay_buf, self.write_pos as f32 - read_offset) * fade;
                        if variant == 2 {
                            let (lp, bp, out) = svf_bandpass_step(slice, st.svf_lp, st.svf_bp, svf_f_coef, BANDPASS_Q);
                            st.svf_lp = lp;
                            st.svf_bp = bp;
                            slice = out;
                        }
                        if variant == 3 {
                            slice = bitcrush(slice, MAX_BITCRUSH_LEVELS - shape * (MAX_BITCRUSH_LEVELS - MIN_BITCRUSH_LEVELS));
                        }
                        wet += slice * flat_tap_gain(t, num_taps);
                    }
                }
                Effect::Interrupt => {
                    // Repeats controls how *often* a glitch burst
                    // fires (a per-cycle probability), not feedback
                    // amount -- there's no delay-feedback role for it
                    // to play here, same repurposing precedent as
                    // Interrupt's dry-gating below.
                    self.glitch_phase += 1.0 / loop_window;
                    if self.glitch_phase >= 1.0 {
                        self.glitch_phase -= 1.0;
                        self.glitch_bursting = self.tap_states[0].next_rand01() < repeats;
                        if self.glitch_bursting {
                            let r1 = self.tap_states[0].next_rand01();
                            let r2 = self.tap_states[0].next_rand01();
                            self.tap_states[0].grain_start = r1 * loop_window * 2.0;
                            self.tap_states[0].grain_rate =
                                if variant == 1 { 2f32.powf((r2 * 24.0 - 12.0) / 12.0) } else { 1.0 };
                        }
                    }
                    // `wet` here is the *unblended* signal (either
                    // pure dry when not bursting, or the pure glitch
                    // burst content) -- the shared dry/wet blend below
                    // the match handles Mix generically, which is what
                    // makes "Mix=100% truly mutes the dry signal
                    // during a burst, but leaves it alone otherwise"
                    // fall out for free rather than needing special
                    // -cased blending here.
                    if self.glitch_bursting {
                        let st = &mut self.tap_states[0];
                        let progress = self.glitch_phase;
                        let read_offset = st.grain_start + progress * loop_window * st.grain_rate;
                        let mut glitch = read_interp(&self.delay_buf, self.write_pos as f32 - read_offset);
                        match variant {
                            // C: filter sweep blended with a second,
                            // slightly-later tap for a touch of delay.
                            2 => {
                                let sweep_hz = 200.0 + (self.lfo_phase * TAU).sin() * 0.5 * 1800.0 + 400.0;
                                let sweep_f_coef = (2.0 * (PI * sweep_hz.max(20.0) / sample_rate).sin()).clamp(0.0, 1.0);
                                let (lp, bp, out) = svf_bandpass_step(glitch, st.svf_lp, st.svf_bp, sweep_f_coef, BANDPASS_Q);
                                st.svf_lp = lp;
                                st.svf_bp = bp;
                                let delayed = read_interp(&self.delay_buf, self.write_pos as f32 - st.grain_start - loop_window * 0.5);
                                glitch = out * 0.7 + delayed * 0.3;
                            }
                            3 => glitch = bitcrush(glitch, MAX_BITCRUSH_LEVELS - shape * (MAX_BITCRUSH_LEVELS - MIN_BITCRUSH_LEVELS)),
                            _ => {}
                        }
                        wet = glitch;
                    } else {
                        wet = input_sample;
                    }
                }
                Effect::Arp => {
                    // Reuses Strum's onset detector -- Arp *sequences
                    // through* those same captured onsets one at a
                    // time (a step sequencer), rather than layering
                    // them all at once the way Strum's cascade does.
                    let rectified = input_sample.abs();
                    self.onset_env_fast += (rectified - self.onset_env_fast) * onset_fast_coef;
                    let onset = self.onset_env_fast > self.onset_env_slow * ONSET_THRESHOLD_RATIO + ONSET_THRESHOLD_FLOOR;
                    self.onset_env_slow += (self.onset_env_fast - self.onset_env_slow) * onset_slow_coef;
                    if onset {
                        for k in (1..MAX_TAPS).rev() {
                            self.onset_ages[k] = self.onset_ages[k - 1];
                        }
                        self.onset_ages[0] = 0.0;
                    }
                    for age in self.onset_ages.iter_mut() {
                        *age += 1.0;
                    }

                    // Step clock: one step per `loop_window` samples,
                    // cycling through `num_taps` steps -- "Activity:
                    // determines the number of steps in the arpeggio."
                    self.glitch_phase += 1.0 / loop_window;
                    if self.glitch_phase >= 1.0 {
                        self.glitch_phase -= 1.0;
                        self.glitch_step = (self.glitch_step + 1) % num_taps.max(1);
                    }
                    let step = self.glitch_step % num_taps.max(1);
                    let anchor = self.onset_ages[step].max(loop_window * 0.5);
                    let rate = if variant == 1 { ARP_STEP_RATES[step % ARP_STEP_RATES.len()] } else { 1.0 };
                    let mut sample = pitch_shift_tap(&self.delay_buf, self.write_pos, anchor, rate, &mut self.tap_states[step].grain_phase, loop_window);
                    if variant == 2 {
                        // C: each step gets its own random-but-fixed
                        // filter cutoff, derived from the step index
                        // itself so it doesn't need extra state.
                        let hash = (step as u32).wrapping_mul(2_654_435_761);
                        let r = ((hash >> 8) & 0xFF_FFFF) as f32 / 16_777_216.0;
                        let cutoff = 200.0 + r * 3800.0;
                        let f_coef = (2.0 * (PI * cutoff / sample_rate).sin()).clamp(0.0, 1.0);
                        let st = &mut self.tap_states[step];
                        let (lp, bp, out) = svf_bandpass_step(sample, st.svf_lp, st.svf_bp, f_coef, BANDPASS_Q);
                        st.svf_lp = lp;
                        st.svf_bp = bp;
                        sample = out;
                    }
                    if variant == 3 {
                        sample = bitcrush(sample, MAX_BITCRUSH_LEVELS - shape * (MAX_BITCRUSH_LEVELS - MIN_BITCRUSH_LEVELS));
                    }
                    wet += sample;
                }
            }
            }

            // Multidelay runs its wet signal through a shared post-sum
            // lowpass (Filter); Warp/Seq's variants that need Filter
            // already used it per-tap above, and Mosaic/Glide simply
            // don't touch it, so it stays inert for them in this pass.
            let filtered_wet = if matches!(effect, Effect::Pattern) {
                self.lp_state += (wet - self.lp_state) * lp_coef;
                self.lp_state
            } else {
                wet
            };

            self.output_mono[n] = input_sample * (1.0 - mix) + filtered_wet * mix;

            if pitch_mod_on {
                self.output_mono[n] = self.apply_pitch_mod(self.output_mono[n], sample_rate);
            }
            // Always run, even at Space=0 -- feeding it zero input
            // rather than skipping the call lets any existing tail
            // decay away naturally through the combs' own feedback
            // instead of freezing mid-decay and popping back in later.
            let (rl, rr) = self.process_reverb(self.output_mono[n] * space_amt);
            self.reverb_out_l[n] = rl;
            self.reverb_out_r[n] = rr;

            // Feedback (Multidelay only) comes from only the longest
            // active tap's raw, unprocessed buffer content -- one
            // bounded scalar (`repeats`) governs the whole loop's
            // gain, so it can't run away regardless of tap count,
            // per-tap gain shaping, or per-tap filtering/pitch-
            // shifting, none of which this feedback read is ever
            // exposed to (see module doc comment). Micro Loop writes
            // the plain input with no feedback at all -- it's a
            // rolling record of "what was just played", not a
            // self-oscillating network, so it's bounded by the input
            // alone regardless of preset.
            let write_sample = if matches!(effect, Effect::Pattern | Effect::Warp) {
                let feedback_read = self.delay_buf[(self.write_pos + buf_len - feedback_tap) % buf_len];
                input_sample + feedback_read * repeats
            } else {
                input_sample
            };
            self.delay_buf[self.write_pos] = write_sample;
            self.write_pos = (self.write_pos + 1) % buf_len;
        }

        {
            let mut bus_out = self.params.bus_out.lock().unwrap();
            bus_out.clear();
            bus_out.extend_from_slice(&self.output_mono);
        }

        // The Mixer app's channel fader for this app -- applied only
        // to what reaches the device, not to `bus_out` above (see
        // plaits.rs for the same pattern).
        //
        // This is the one place L and R actually differ: `output_mono`
        // (the 44-preset effect, plus Pitch Mod) is identical on both
        // sides -- every AudioBus tap feeding it, Prism's own input
        // included, is mono system-wide (see the module doc comment)
        // -- but Space's reverb tail runs two decorrelated chains, so
        // adding each channel's own tail back in produces genuine
        // stereo width rather than the same value duplicated to every
        // channel, which is all this used to do.
        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        for (i, frame) in buffer.chunks_mut(channels).enumerate() {
            let dry_wet = self.output_mono[i];
            let l = (dry_wet + self.reverb_out_l[i]) * mix_level;
            let r = (dry_wet + self.reverb_out_r[i]) * mix_level;
            if frame.len() >= 2 {
                frame[0] = l;
                frame[1] = r;
                for extra in frame[2..].iter_mut() {
                    *extra = l;
                }
            } else if let Some(m) = frame.first_mut() {
                *m = (l + r) * 0.5;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// More than the old `MAX_INPUTS` (8) real apps registered into
    /// `AudioBus` -- every one of them must still show up in this
    /// app's own input list, not just the first 8 (the bug that made
    /// Tape and Voltage, alphabetically last of the 11 real apps that
    /// register, invisible as inputs here).
    #[test]
    fn every_registered_source_appears_as_an_input_past_the_old_cap() {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        for i in 0..12 {
            audio_bus.register(format!("source-{i}"));
        }
        let app = PrismApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);

        // 12 test sources + this app's own registration of itself = 13.
        let leaves = app.group_leaves(0);
        assert_eq!(leaves.len(), 13, "expected all 13 registered sources (12 test ones + itself) to be selectable inputs, got {}", leaves.len());
        assert!(matches!(leaves[12], Selection::InputLevel(12)), "the last source (past the old cap of 8) must still be reachable");
    }

    fn new_processor(params: Arc<Params>, audio_bus: Arc<AudioBus>) -> PrismProcessor {
        PrismProcessor {
            params,
            audio_bus,
            input_mono: Vec::new(),
            output_mono: Vec::new(),
            reverb_out_l: Vec::new(),
            reverb_out_r: Vec::new(),
            delay_buf: Vec::new(),
            write_pos: 0,
            lp_state: 0.0,
            lfo_phase: 0.0,
            tap_states: std::array::from_fn(TapState::new),
            onset_env_fast: 0.0,
            onset_env_slow: 0.0,
            onset_ages: [0.0; MAX_TAPS],
            input_envelope: 0.0,
            glitch_phase: 0.0,
            glitch_step: 0,
            glitch_bursting: false,
            reverb_l: None,
            reverb_r: None,
            pitch_mod_buf: Vec::new(),
            pitch_mod_write_pos: 0,
            pitch_mod_lfo_phase: 0.0,
            looper_buf: Vec::new(),
            looper_write_pos: 0,
            looper_len: 0,
            looper_read_pos: 0,
            sampler_buf: Vec::new(),
            sampler_write_pos: 0,
            sampler_grain_phase: 0.0,
            prev_looper_state: 0,
            prev_sampler_state: 0,
        }
    }

    fn preset_index(effect: Effect, variant: usize) -> u32 {
        let effect_idx = EFFECT_ORDER.iter().position(|&e| e == effect).unwrap();
        (effect_idx * VARIANTS_PER_EFFECT + variant) as u32
    }

    /// The Mixer app's channel fader must only affect what reaches the
    /// device output -- not what this app publishes to audio_bus.rs
    /// for another app (Clouds) to tap. Muting Prism's channel in the
    /// Mixer shouldn't also starve whatever's granulating it.
    #[test]
    fn mixer_fader_does_not_affect_audio_bus_publish() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0);
        params.mix.set(1.0);
        params.repeats.set(0.0);
        params.taps.store(1, Ordering::Relaxed);
        params.time.set(0.005); // short enough that the tap is already live within one block
        params.mix_level.set(0.0); // fully muted in the Mixer app

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let sample_rate = 48000.0;
        let frames = 512;
        let mut buffer = vec![0.0f32; frames * 2];
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 0.5);
        }
        // A few blocks of sustained input -- the first block's tap
        // still reads mostly-empty history right at the very start,
        // so give it a moment to fill before checking either output.
        for _ in 0..5 {
            proc.process(&mut buffer, 2, sample_rate);
        }

        let device_peak = buffer.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert_eq!(device_peak, 0.0, "muted-in-Mixer output must be silent at the device");

        let published_peak = {
            let bus_out = params.bus_out.lock().unwrap();
            bus_out.iter().fold(0.0f32, |m, v| m.max(v.abs()))
        };
        assert!(published_peak > 0.0, "audio_bus publish must stay at full (pre-fader) level regardless of the Mixer's channel fader");
    }

    /// A single-block impulse fed at 100% mix should come back out
    /// delayed, not immediately -- the whole point of a delay line.
    #[test]
    fn delayed_tap_echoes_the_input() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0); // route the test source into the mixer
        params.mix.set(1.0);
        params.repeats.set(0.0); // isolate a single pass, no feedback buildup
        params.taps.store(1, Ordering::Relaxed);
        params.time.set(0.05); // 50ms

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let sample_rate = 48000.0;
        let frames = 512;
        let mut buffer = vec![0.0f32; frames * 2];

        // First block: a single-sample impulse at the very start --
        // the processor's delay buffer starts empty/zero, so this is
        // also absolute sample 0, keeping the timing math below exact.
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 0.0);
            s[0] = 1.0;
        }
        proc.process(&mut buffer, 2, sample_rate);
        let immediate: f32 = buffer[0..2].iter().map(|v| v.abs()).sum();
        assert!(immediate < 0.01, "impulse should not appear immediately at 100% mix: {immediate}");

        // Keep feeding silence and watch for the echo ~50ms (2400
        // samples) after that absolute-sample-0 impulse.
        {
            let mut s = src.lock().unwrap();
            s.iter_mut().for_each(|v| *v = 0.0);
        }
        let delay_samples = (0.05 * sample_rate) as usize;
        let mut found = false;
        let mut total_samples = frames; // the first block above already ran
        for _ in 0..20 {
            buffer.fill(0.0);
            proc.process(&mut buffer, 2, sample_rate);
            for frame in 0..frames {
                if (total_samples + frame).abs_diff(delay_samples) <= 2 && buffer[frame * 2].abs() > 0.05 {
                    found = true;
                }
            }
            total_samples += frames;
        }
        assert!(found, "expected the delayed echo near sample {delay_samples}");
    }

    /// Repeats must stay bounded regardless of tap count -- the
    /// feedback-explosion bug this design specifically avoids.
    #[test]
    fn feedback_stays_bounded_with_many_taps() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0); // route the test source into the mixer
        params.mix.set(1.0);
        params.repeats.set(MAX_REPEATS);
        params.taps.store(MAX_TAPS, Ordering::Relaxed);
        params.time.set(0.05);
        params.shape.set(0.0); // every tap at full gain -- the worst case

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let sample_rate = 48000.0;
        let frames = 512;
        let mut buffer = vec![0.0f32; frames * 2];

        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 0.0);
            s[0] = 1.0;
        }
        proc.process(&mut buffer, 2, sample_rate);
        {
            let mut s = src.lock().unwrap();
            s.iter_mut().for_each(|v| *v = 0.0);
        }

        let mut max_abs = 0.0f32;
        for _ in 0..400 {
            buffer.fill(0.0);
            proc.process(&mut buffer, 2, sample_rate);
            for v in buffer.iter() {
                max_abs = max_abs.max(v.abs());
                assert!(v.is_finite(), "output must never be NaN/inf");
            }
        }
        assert!(max_abs < 10.0, "feedback should stay bounded, not blow up: peak was {max_abs}");
    }

    /// Every preset across all 5 effects must produce finite, bounded
    /// output under sustained, worst-case-ish input -- the one
    /// invariant that must hold no matter which effect/variant is
    /// selected.
    #[test]
    fn every_preset_stays_bounded_with_sustained_input() {
        for preset in 0..PRESET_NAMES.len() {
            let modbus = ModBus::new();
            let audio_bus = Arc::new(AudioBus::new());
            let mixer_bus = MixerBus::new();
            let src = audio_bus.register("test-source");
            let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
            params.input_levels[0].set(1.0);
            params.mix.set(1.0);
            params.repeats.set(MAX_REPEATS);
            params.taps.store(MAX_TAPS, Ordering::Relaxed);
            params.time.set(0.05);
            params.shape.set(1.0);
            params.filter.set(1.0);
            params.preset.store(preset as u32, Ordering::Relaxed);

            let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
            let sample_rate = 48000.0;
            let frames = 512;
            let mut buffer = vec![0.0f32; frames * 2];

            // Sustained noise-ish input (not just one impulse) --
            // Micro Loop's grain-based readers need continuous
            // content to reveal any instability a single impulse
            // might not exercise.
            {
                let mut s = src.lock().unwrap();
                s.resize(frames, 0.0);
                let mut rng: u32 = 0x1234_5678;
                for v in s.iter_mut() {
                    rng ^= rng << 13;
                    rng ^= rng >> 17;
                    rng ^= rng << 5;
                    *v = (rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
                }
            }

            let mut max_abs = 0.0f32;
            for _ in 0..200 {
                buffer.fill(0.0);
                proc.process(&mut buffer, 2, sample_rate);
                for v in buffer.iter() {
                    max_abs = max_abs.max(v.abs());
                    assert!(v.is_finite(), "preset {preset} ({}): output must never be NaN/inf", PRESET_NAMES[preset]);
                }
            }
            // Not a tight bound -- the point is catching genuine
            // runaway/explosion (which shows up as NaN or values in
            // the hundreds+), not fine-tuning steady-state gain. A
            // single feedback tap at `repeats` near its ceiling has a
            // legitimate steady-state gain of `1/(1-repeats)` (~12.5
            // at MAX_REPEATS) under sustained full-scale input; this
            // just needs enough margin above that to not be a false
            // positive on an otherwise-stable preset.
            assert!(max_abs < 25.0, "preset {preset} ({}): output should stay bounded, peak was {max_abs}", PRESET_NAMES[preset]);
        }
    }

    /// Warp must actually process each tap differently from Pattern's
    /// plain gain falloff -- otherwise it would just be Pattern A
    /// under a different name. Same check extended to confirm every
    /// Micro Loop preset also differs from Pattern A and from a
    /// silent/dry pass-through.
    #[test]
    fn every_non_pattern_preset_differs_from_pattern_a() {
        let render = |preset: u32| -> Vec<f32> {
            let modbus = ModBus::new();
            let audio_bus = Arc::new(AudioBus::new());
            let mixer_bus = MixerBus::new();
            let src = audio_bus.register("test-source");
            let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
            params.input_levels[0].set(1.0);
            params.mix.set(1.0);
            params.repeats.set(0.5);
            // All 8 taps, not fewer -- Pattern's 4 rhythmic
            // arrangements only actually diverge from each other
            // past the 4th tap (they're deliberately similar early
            // on), so a smaller tap count would make e.g. Pattern D
            // indistinguishable from Pattern A here.
            params.taps.store(MAX_TAPS, Ordering::Relaxed);
            params.time.set(0.02);
            params.shape.set(0.8);
            params.preset.store(preset, Ordering::Relaxed);

            let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
            let sample_rate = 48000.0;
            let frames = 512;
            let mut buffer = vec![0.0f32; frames * 2];
            {
                let mut s = src.lock().unwrap();
                s.resize(frames, 0.0);
                s[0] = 1.0;
            }
            proc.process(&mut buffer, 2, sample_rate);
            {
                let mut s = src.lock().unwrap();
                s.iter_mut().for_each(|v| *v = 0.0);
            }
            let mut out = Vec::new();
            for _ in 0..10 {
                buffer.fill(0.0);
                proc.process(&mut buffer, 2, sample_rate);
                out.extend(buffer.iter().step_by(2)); // left channel only
            }
            out
        };

        let pattern_a = render(preset_index(Effect::Pattern, 0));
        for (preset, name) in PRESET_NAMES.iter().enumerate().skip(1) {
            let other = render(preset as u32);
            let diff: f32 = pattern_a.iter().zip(other.iter()).map(|(a, b)| (a - b).abs()).sum();
            assert!(diff > 0.01, "{name} should sound different from Pattern A, total diff was {diff}");
        }
    }

    /// Micro Loop and Granules presets must never touch the feedback
    /// path -- a steady input should never build up into runaway
    /// gain the way a mis-wired feedback loop would.
    #[test]
    fn non_delay_effects_never_feed_back() {
        for effect in [Effect::Mosaic, Effect::Seq, Effect::Glide, Effect::Haze, Effect::Tunnel, Effect::Strum, Effect::Blocks, Effect::Interrupt, Effect::Arp] {
            for variant in 0..VARIANTS_PER_EFFECT {
                let modbus = ModBus::new();
                let audio_bus = Arc::new(AudioBus::new());
                let mixer_bus = MixerBus::new();
                let src = audio_bus.register("test-source");
                let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
                params.input_levels[0].set(1.0);
                params.mix.set(1.0);
                params.taps.store(MAX_TAPS, Ordering::Relaxed);
                params.time.set(0.05);
                params.preset.store(preset_index(effect, variant), Ordering::Relaxed);

                let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
                let sample_rate = 48000.0;
                let frames = 512;
                let mut buffer = vec![0.0f32; frames * 2];
                {
                    let mut s = src.lock().unwrap();
                    s.resize(frames, 1.0); // sustained full-scale DC-ish input
                }

                // Warm up first: some variants (Haze's widest offsets
                // especially) read several hundred ms back into a
                // buffer that starts at all-zero, so the *true*
                // steady-state output only appears once that whole
                // lookback window has been overwritten with the
                // sustained input -- not measuring this separately
                // would misread that ordinary fill-up transient as
                // "growing over time".
                for _ in 0..100 {
                    proc.process(&mut buffer, 2, sample_rate);
                }

                // The real invariant isn't "stays under some fixed
                // amplitude" -- summing several coherent (here,
                // literally identical, since the input is constant)
                // unity-ish-gain layers legitimately produces an
                // output several times louder than the input, with
                // no feedback involved at all. What actually
                // distinguishes "no feedback" from "feeding back" is
                // that the peak must not keep *growing* block over
                // block: a real feedback loop at gain >= 1 diverges
                // over time, while a feedforward layer stack (however
                // many layers) settles immediately and stays flat.
                let mut early_max = 0.0f32;
                let mut late_max = 0.0f32;
                for i in 0..200 {
                    proc.process(&mut buffer, 2, sample_rate);
                    let block_max = buffer.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                    assert!(block_max.is_finite(), "{effect:?} variant {variant}: output must never be NaN/inf");
                    if i < 10 {
                        early_max = early_max.max(block_max);
                    } else if i >= 190 {
                        late_max = late_max.max(block_max);
                    }
                }
                assert!(
                    late_max <= early_max * 1.1 + 0.01,
                    "{effect:?} variant {variant}: output grew over time (early {early_max}, late {late_max}) -- looks like unintended feedback"
                );
            }
        }
    }

    /// Haze's grain slots must actually retrigger (pick a fresh
    /// random rate/start) over time, not just sit on their initial
    /// state -- that retriggering is the whole mechanic.
    #[test]
    fn haze_grains_retrigger() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0);
        params.mix.set(1.0);
        params.taps.store(4, Ordering::Relaxed);
        params.time.set(0.02); // short grains so several retriggers happen quickly
        params.preset.store(preset_index(Effect::Haze, 2), Ordering::Relaxed); // C: normal/double mix

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let sample_rate = 48000.0;
        let frames = 512;
        let mut buffer = vec![0.0f32; frames * 2];
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 0.3);
        }

        let initial_rate = proc.tap_states[0].grain_rate;
        let mut saw_retrigger = false;
        for _ in 0..50 {
            proc.process(&mut buffer, 2, sample_rate);
            if proc.tap_states[0].grain_rate != initial_rate {
                saw_retrigger = true;
                break;
            }
        }
        assert!(saw_retrigger, "expected slot 0's grain to retrigger with a new rate within 50 blocks");
    }

    /// Strum's onset detector must actually fire on a transient, and
    /// the resulting cascade (C) must spread across several distinct
    /// onset ages, not just the same one repeated.
    #[test]
    fn strum_detects_onset_and_cascades() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0);
        params.mix.set(1.0);
        params.taps.store(4, Ordering::Relaxed);
        params.time.set(0.02);
        params.preset.store(preset_index(Effect::Strum, 2), Ordering::Relaxed); // C: cascade

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let sample_rate = 48000.0;
        let frames = 512;
        let mut buffer = vec![0.0f32; frames * 2];

        // A few sharp transients separated by silence -- clear onsets.
        for _ in 0..4 {
            {
                let mut s = src.lock().unwrap();
                s.resize(frames, 0.0);
                s[0] = 1.0;
            }
            proc.process(&mut buffer, 2, sample_rate);
            {
                let mut s = src.lock().unwrap();
                s.iter_mut().for_each(|v| *v = 0.0);
            }
            for _ in 0..5 {
                proc.process(&mut buffer, 2, sample_rate);
            }
        }

        let distinct_ages = proc.onset_ages.iter().filter(|&&a| a > 1.0).count();
        assert!(distinct_ages >= 2, "expected at least 2 distinct onset ages to be tracked, got {:?}", proc.onset_ages);
    }

    /// Space's reverb tail must actually decorrelate L/R -- otherwise
    /// it's just duplicating a mono signal to both channels again,
    /// the exact thing "true stereo" was meant to fix.
    #[test]
    fn space_reverb_creates_genuine_stereo_width() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0);
        params.mix.set(1.0);
        params.space.set(1.0);

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let sample_rate = 48000.0;
        let frames = 512;
        let mut buffer = vec![0.0f32; frames * 2];
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 0.0);
            s[0] = 1.0;
        }
        proc.process(&mut buffer, 2, sample_rate);
        {
            let mut s = src.lock().unwrap();
            s.iter_mut().for_each(|v| *v = 0.0);
        }
        let mut max_lr_diff = 0.0f32;
        for _ in 0..40 {
            proc.process(&mut buffer, 2, sample_rate);
            for i in 0..frames {
                max_lr_diff = max_lr_diff.max((proc.reverb_out_l[i] - proc.reverb_out_r[i]).abs());
            }
        }
        assert!(max_lr_diff > 1e-5, "L and R reverb tails should diverge over time, stayed identical (diff={max_lr_diff})");
    }

    /// Space must stay bounded under sustained worst-case input --
    /// comb feedback below 1.0 is a contraction, so it should settle
    /// to a stable (if high-gain) plateau rather than keep growing
    /// forever. A fixed absolute ceiling isn't the right check here:
    /// a comb filter's own steady-state DC gain is `1/(1-feedback)`
    /// (~5.56x at `REVERB_COMB_FEEDBACK`), stacked on top of
    /// Pattern's own feedback gain at `MAX_REPEATS` -- legitimately a
    /// large number, same "high but bounded" shape as
    /// `feedback_stays_bounded_with_many_taps` elsewhere in this file.
    /// What actually matters is that it plateaus instead of diverging,
    /// so this compares two later windows instead of guessing a
    /// magic ceiling.
    #[test]
    fn space_reverb_stays_bounded_with_sustained_input() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0);
        params.mix.set(1.0);
        params.space.set(1.0);
        params.repeats.set(MAX_REPEATS); // worst case
        // Pattern's own feedback tap settles in roughly
        // log(0.01)/log(repeats) passes, each one `time`-seconds long
        // -- at MAX_REPEATS that's ~55 passes, so a short `time`
        // keeps the *effect's* settling time well inside this test's
        // budget (the comb reverb's own settling, independent of
        // `time`, is the shorter of the two at these lengths).
        params.time.set(MIN_TIME);

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let sample_rate = 48000.0;
        let frames = 512;
        let mut buffer = vec![0.0f32; frames * 2];
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 1.0);
        }

        let mut peak_of = |proc: &mut PrismProcessor, blocks: usize| {
            let mut peak = 0.0f32;
            for _ in 0..blocks {
                proc.process(&mut buffer, 2, sample_rate);
                for v in buffer.iter() {
                    peak = peak.max(v.abs());
                }
            }
            peak
        };

        peak_of(&mut proc, 250); // past the initial transient
        let mid_peak = peak_of(&mut proc, 100);
        let late_peak = peak_of(&mut proc, 100);

        assert!(late_peak.is_finite(), "output diverged to non-finite: {late_peak}");
        assert!(
            late_peak <= mid_peak * 1.1,
            "should have settled to a stable plateau, not kept growing: mid={mid_peak} late={late_peak}"
        );
    }

    /// Pitch Mod must actually change the signal when engaged --
    /// otherwise the toggle is a no-op.
    #[test]
    fn pitch_mod_audibly_changes_output() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0);
        // Some dry blended in (not 100% wet) so a real signal reaches
        // Pitch Mod from block 1 -- at Mix=100% with the default
        // Pattern preset's 300ms delay time, the wet path is still
        // reading pure silence for the whole length of this short
        // test, which would make "on vs off" trivially identical for
        // the wrong reason (nothing to modulate yet, not a broken
        // toggle).
        params.mix.set(0.3);

        let run = |pitch_mod_on: bool| {
            params.pitch_mod_on.store(pitch_mod_on, Ordering::Relaxed);
            let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
            let sample_rate = 48000.0;
            let frames = 512;
            let mut buffer = vec![0.0f32; frames * 2];
            {
                let mut s = src.lock().unwrap();
                s.resize(frames, 0.0);
                for (i, v) in s.iter_mut().enumerate() {
                    *v = (i as f32 * 0.1).sin();
                }
            }
            let mut out = Vec::new();
            for _ in 0..10 {
                proc.process(&mut buffer, 2, sample_rate);
                out.extend_from_slice(&buffer);
            }
            out
        };

        let off = run(false);
        let on = run(true);
        let diff: f32 = off.iter().zip(on.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 0.01, "Pitch Mod on vs off should sound different, total abs diff was {diff}");
    }

    /// Looper: recording must actually capture the input, and
    /// playback must reproduce it -- the whole point of a phrase
    /// looper.
    #[test]
    fn looper_records_and_plays_back() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0);
        params.mix.set(1.0);
        params.utility_mode.store(1, Ordering::Relaxed); // Looper
        params.looper_state.store(1, Ordering::Relaxed); // Recording

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let sample_rate = 48000.0;
        let frames = 8;
        let mut buffer = vec![0.0f32; frames * 2];
        let pattern = [0.5f32, -0.5, 0.25, -0.25, 0.1, -0.1, 0.75, -0.75];
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 0.0);
            s.copy_from_slice(&pattern);
        }
        proc.process(&mut buffer, 2, sample_rate);
        assert_eq!(proc.looper_len, frames, "expected all {frames} samples to have been recorded, got {}", proc.looper_len);

        params.looper_state.store(2, Ordering::Relaxed); // stop recording, play back
        {
            let mut s = src.lock().unwrap();
            s.iter_mut().for_each(|v| *v = 0.0); // silence the input -- playback shouldn't need it
        }
        proc.process(&mut buffer, 2, sample_rate);
        let played: Vec<f32> = buffer.chunks(2).map(|f| f[0]).collect();
        for (i, (&expected, &got)) in pattern.iter().zip(played.iter()).enumerate() {
            assert!((expected - got).abs() < 1e-4, "sample {i}: expected {expected}, got {got}");
        }
    }

    /// Sampler: once frozen (Held), it must keep looping the captured
    /// window on its own, indefinitely, even with silence at the
    /// input -- that's the entire point of a freeze/hold effect.
    #[test]
    fn sampler_freezes_and_loops_without_new_input() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0);
        params.mix.set(1.0);
        params.utility_mode.store(2, Ordering::Relaxed); // Sampler

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let sample_rate = 48000.0;
        let frames = 512;
        let mut buffer = vec![0.0f32; frames * 2];
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 0.0);
            for (i, v) in s.iter_mut().enumerate() {
                *v = (i as f32 * 0.2).sin();
            }
        }
        // Idle: fills the rolling window.
        for _ in 0..5 {
            proc.process(&mut buffer, 2, sample_rate);
        }

        params.sampler_state.store(1, Ordering::Relaxed); // freeze
        {
            let mut s = src.lock().unwrap();
            s.iter_mut().for_each(|v| *v = 0.0);
        }
        let mut max_abs = 0.0f32;
        for _ in 0..20 {
            proc.process(&mut buffer, 2, sample_rate);
            for v in buffer.iter() {
                max_abs = max_abs.max(v.abs());
            }
        }
        assert!(max_abs > 0.05, "frozen Sampler should keep outputting its captured content with no new input, got max {max_abs}");
    }

    /// A user preset must round-trip every knob it saves -- and must
    /// not clobber a slot until explicitly saved into.
    #[test]
    fn user_preset_saves_and_loads_knob_state() {
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let app = PrismApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);

        app.params.time.set(0.5);
        app.params.repeats.set(0.6);
        app.params.shape.set(0.7);
        app.params.filter.set(0.3);
        app.params.mix.set(0.8);
        app.params.space.set(0.9);
        app.params.pitch_mod_on.store(true, Ordering::Relaxed);
        app.params.taps.store(6, Ordering::Relaxed);
        app.params.preset.store(preset_index(Effect::Haze, 1), Ordering::Relaxed);

        assert!(app.params.user_presets[0].lock().unwrap().is_none(), "slot 0 should start empty");
        app.save_user_preset(0);
        assert!(app.params.user_presets[0].lock().unwrap().is_some(), "save should fill the slot");

        // Change everything, then load the slot back.
        app.params.time.set(0.01);
        app.params.repeats.set(0.0);
        app.params.shape.set(0.0);
        app.params.filter.set(0.0);
        app.params.mix.set(0.0);
        app.params.space.set(0.0);
        app.params.pitch_mod_on.store(false, Ordering::Relaxed);
        app.params.taps.store(1, Ordering::Relaxed);
        app.params.preset.store(0, Ordering::Relaxed);

        app.load_user_preset(0);
        assert_eq!(app.params.time.get(), 0.5);
        assert_eq!(app.params.repeats.get(), 0.6);
        assert_eq!(app.params.shape.get(), 0.7);
        assert_eq!(app.params.filter.get(), 0.3);
        assert_eq!(app.params.mix.get(), 0.8);
        assert_eq!(app.params.space.get(), 0.9);
        assert!(app.params.pitch_mod_on.load(Ordering::Relaxed));
        assert_eq!(app.params.taps.load(Ordering::Relaxed), 6);
        assert_eq!(app.params.preset.load(Ordering::Relaxed), preset_index(Effect::Haze, 1));
    }

    /// A CC-mapped knob (see `PrismCcTargets`) must be the *same*
    /// atomic `Params` reads -- writing to the target externally
    /// (standing in for `handle_midi_message` in main.rs) must be
    /// visible through the app immediately, with no separate "MIDI
    /// value" to reconcile.
    #[test]
    fn cc_targets_share_the_same_atomics_as_params() {
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let cc = PrismCcTargets::new();

        let app = PrismApp::new_with_cc(sensitivity, nav_speed, modbus, audio_bus, mixer_bus, &cc);

        cc.space.set(0.66);
        assert_eq!(app.params.space.get(), 0.66, "writing the external CC handle should be visible through Params immediately");

        app.params.mix.set(0.33);
        assert_eq!(cc.mix.get(), 0.33, "and the reverse -- editing the knob in-app should be visible on the external handle too");
    }

    /// The list column's text (at `paramlist::TEXT_SCALE`) must never
    /// reach `axis_x0` (380), where the tap-time/Micro-Loop axis
    /// starts -- with every group expanded (every leaf, including the
    /// longest preset/effect/input names this app has, all on screen
    /// at once) rather than guessing which single row is worst.
    #[test]
    fn list_text_never_reaches_the_right_panel() {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let mut app = PrismApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);
        app.expanded = [true; NUM_GROUPS];
        app.params.taps.store(MAX_TAPS, Ordering::Relaxed);
        for i in 0..NUM_USER_PRESETS {
            app.save_user_preset(i);
        }

        let preset = app.params.preset.load(Ordering::Relaxed) as usize % PRESET_NAMES.len();
        let (effect, _variant) = effect_for_preset(preset);
        let rows = app.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if app.expanded[*g] { "v" } else { ">" };
                    let name = match *g {
                        0 => "Mixer",
                        1 => category_name(effect),
                        2 => "Utility",
                        _ => "User Presets",
                    };
                    (format!("{arrow} {name}"), app.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", app.leaf_name(*sel)), app.leaf_value(*sel)),
            })
            .collect();

        let mut list = ParamList::new();
        let mut fb = FrameBuffer::new();
        list.draw(&mut fb, 16, 44, 24, rows.len(), &display_rows);
        let mut max_x = 0i32;
        for (i, &p) in fb.buffer().iter().enumerate() {
            if p != 0 {
                max_x = max_x.max((i % crate::display::WIDTH) as i32);
            }
        }
        const AXIS_X0: i32 = 380;
        assert!(max_x < AXIS_X0, "list text reached x={max_x}, at or past axis_x0 ({AXIS_X0})");
    }
}
