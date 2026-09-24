//! Magnito: a magnetic-tape-style saturation/hysteresis processor.
//! Taps another app's output (see audio_bus.rs) through a real tape-
//! saturation model. Not an explicit clone of one specific named
//! module the way Beads/Warps/Rainmaker are, but researched against
//! the closest real match found -- Zlosynth Instruments' Kaseta
//! (zlosynth.com/kaseta-user-manual.pdf), a real Eurorack module that
//! "simulates magnetic hysteresis to provide warm saturation" and
//! "offers wow and flutter control", whose own hysteresis model is
//! itself credited to Jatin Chowdhury's tape-machine physical-modeling
//! papers and ChowTape plugin (https://ccrma.stanford.edu/~jatin/420/
//! tape/TapeModel_DAFx.pdf) -- plus general tape-emulation engineering
//! practice (bias, tone, wow/flutter, hiss, HF loss/wear) found across
//! that lineage. Feature set, deepened from the original stub:
//!
//! - DRIVE + BIAS: a real (if simplified) hysteresis model -- rather
//!   than a stateless curve, the output is a *lagging follower* of an
//!   instantaneous saturation target, with the follower's tracking
//!   speed set by BIAS (narrower "loop" = faster tracking = less
//!   audible memory; wider = slower = more audible "magnetic memory")
//!   and a dead-zone that mutes quiet material at low BIAS -- matching
//!   Kaseta's manual ("higher BIAS narrows the shape; at its lowest,
//!   BIAS acts like a low-pass filter and mutes weaker sounds, and
//!   distortion gets more pronounced"). This is genuinely
//!   history/path-dependent (the actual definition of hysteresis), not
//!   a fixed nonlinearity -- but it is *not* Kaseta's own iterative
//!   Jiles-Atherton solve (see "Deliberately not implemented" below).
//! - DRIVE turned fully clockwise (>=98%) unlocks "unlimited
//!   hysteresis": a harsher, safety-uncapped saturation range -- this
//!   sim's stand-in for Kaseta's own secondary feature (hold the
//!   module's button while turning DRIVE to max disables its
//!   stability limiter for "far harsher distortions, clicks and
//!   pops"). Same "turn one knob fully clockwise for an alternate
//!   mode" pattern used by Beads' SIZE-as-delay.
//! - TONE: a single DJ-style low/high-pass knob (center = off),
//!   applied post-saturation -- Kaseta's TONE control, minus its 3-way
//!   input/feedback/both placement toggle (Magnito has no delay/
//!   feedback path to place a second filter stage into).
//! - WOW & FLUTTER: slow/fast tape-speed wobble, each modeled as a
//!   mean-reverting Ornstein-Uhlenbeck random walk blended with a
//!   slow sine rather than a pure LFO -- real tape speed variation
//!   isn't perfectly periodic. Kaseta's own manual credits exactly
//!   this technique (an OU process) for its wow effect.
//! - NOISE: a tape hiss floor, band-limited with a one-pole highpass
//!   so it doesn't dump low-frequency rumble into the mix -- a real,
//!   audible tape-transport artifact, not a decorative knob.
//! - WEAR: tape-age modeling -- a one-pole HF rolloff (oxide/binder
//!   degradation reduces high-frequency retention) plus occasional
//!   brief, randomly-triggered amplitude dropouts (oxide shedding),
//!   inspired by Strymon Magneto's "tape age"/"tape crinkle" controls
//!   and general tape-emulation "wear"/"chew" parameters.
//! - Dry/Wet and output Level, matching the original stub.
//!
//! Deliberately not implemented: Kaseta's other half is really a
//! 4-head tape *delay*/looper (independent reading heads with
//! position/volume/pan/feedback, Frippertronics-style looping, a
//! trigger sequencer, an internal VCO, per-knob CV-input mapping with
//! calibration, and a display/configuration menu) -- none of that is
//! part of what this app's own doc comment ever claimed to be (a
//! saturation/hysteresis + wow&flutter processor, not a delay/looper).
//! Building a real multi-head delay engine would essentially mean
//! cloning Beads'/Clouds' buffer machinery from scratch into an
//! unrelated app; out of scope for deepening Magnito's own stated
//! concept. Also not implemented: Kaseta's true iterative
//! Jiles-Atherton magnetization solve -- too expensive to run
//! per-sample in this sim's simple realtime block loop; the
//! direction/width-dependent lag model used here is a genuinely
//! history-dependent approximation of the same physical idea, not a
//! fake stub, but it does not literally solve the JA differential
//! equations the way Kaseta/ChowTape do.

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
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Real, live-data-driven visualization state for a bespoke Slint
/// panel -- every field here is genuine DSP state captured once per
/// audio block by `MagnitoProcessor::process` (see `Params`'
/// `loop_snapshot`/`wobble_snapshot`/etc.), not fabricated.
///
/// The centerpiece is `loop_points`: real (input, output) pairs taken
/// directly across the hysteresis follower (`hysteresis_step`) for
/// the most recent audio block. Plotted as a scatter/line with x =
/// input and y = output, this traces the actual lagging "S-curve with
/// memory" shape that is this app's whole premise -- a real
/// hysteresis loop, not a synthesized decoration. Because `mem` is a
/// lagging follower rather than a stateless curve, sweeping the same
/// input value on the way up vs. on the way down produces two
/// different output values, which is exactly what makes the plotted
/// loop visibly open up (wider at low BIAS/more "unlimited", narrower
/// at high BIAS) instead of collapsing onto a single static curve.
#[derive(Clone, Default)]
pub(crate) struct MagnitoVisual {
    /// Real (input, output) sample pairs across the most recent audio
    /// block, both in roughly -1..1 (post-wobble input to the
    /// hysteresis follower on x, its immediate output on y). Intended
    /// visual: an oscilloscope-style XY/Lissajous plot -- a straight
    /// diagonal line means no saturation; a fattened, tilted S-curve
    /// means the hysteresis follower is actively coloring the signal;
    /// visible separation between the "going up" and "going down"
    /// halves of the trace is the real hysteresis memory effect.
    /// Downsampled to a fixed point count (see `LOOP_SNAPSHOT_POINTS`)
    /// for a stable panel size regardless of block size.
    pub loop_points: Vec<(f32, f32)>,
    /// Current BIAS (0..1) -- the real hysteresis loop-width control.
    /// Intended use: label/annotate the loop plot ("narrower loop" at
    /// high bias, "wider loop" at low bias) or tint it.
    pub bias: f32,
    /// True when DRIVE >= `UNLIMITED_DRIVE_THRESHOLD` ("unlimited
    /// hysteresis" engaged, see module doc comment). Intended use:
    /// switch the loop plot to a harsher accent color/style, matching
    /// the real module's own "hold button, turn DRIVE to max" mode.
    pub unlimited: bool,
    /// A scrolling snapshot (fixed point count, see
    /// `WOBBLE_SNAPSHOT_POINTS`) of the actual combined Wow+Flutter
    /// modulation depth (in samples) that drove the tape-speed wobble
    /// read position this block, normalized to roughly -1..1 against
    /// a sane maximum depth. Intended visual: a small scrolling line
    /// plot showing the real tape wobble waveform -- slow/smooth when
    /// only Wow is up, fast/jittery when Flutter is up.
    pub wobble_snapshot: Vec<f32>,
    /// RMS magnitude of that same wobble signal this block, 0..1.
    /// Intended use: drive a simple "wobble intensity" meter/needle
    /// alongside the scrolling plot.
    pub wow_flutter_depth: f32,
    /// RMS level of the tape hiss actually added to the signal this
    /// block, 0..1 (0 when NOISE is at 0). Intended use: a small hiss
    /// level meter/LED.
    pub hiss_level: f32,
    /// Current WEAR (0..1), passed through directly. Intended use:
    /// drive a "tape condition" visual -- e.g. a worn/scratched
    /// texture overlay or discoloration that intensifies with WEAR.
    pub wear: f32,
    /// True if an amplitude dropout (real oxide-shedding simulation,
    /// see module doc comment) was in progress at the end of this
    /// block. Intended use: flash a "dropout" indicator/LED exactly
    /// when the tape is actually ducking, not on a fake timer.
    pub dropout_active: bool,
    /// Peak absolute sample value of this block's final output (post
    /// dry/wet mix and LEVEL), roughly 0..1. Intended use: a simple
    /// output level meter.
    pub output_peak: f32,
}

/// DRIVE at or above this (fraction of full CW rotation) unlocks
/// "unlimited hysteresis" -- see module doc comment.
const UNLIMITED_DRIVE_THRESHOLD: f32 = 0.98;
const WOBBLE_BUF_LEN: usize = 4096; // ~85ms at 48kHz -- plenty for wow/flutter depth

/// Slow, mean-reverting Ornstein-Uhlenbeck parameters for Wow -- see
/// module doc comment. THETA is the reversion rate (roughly 1/theta
/// seconds to settle), SIGMA the noise strength.
const WOW_OU_THETA: f32 = 1.3;
const WOW_OU_SIGMA: f32 = 5.0;
/// Faster OU parameters for Flutter -- same technique, quicker
/// reversion for a more jittery, less periodic wobble.
const FLUTTER_OU_THETA: f32 = 10.0;
const FLUTTER_OU_SIGMA: f32 = 14.0;
/// Both OU processes are statistically bounded (their stationary
/// variance is sigma^2/(2*theta)) but not hard-bounded -- clamp
/// defensively so an unlucky run of noise samples can never push the
/// wobble read position out of a sane range.
const OU_CLAMP: f32 = 8.0;

/// Fixed point counts the per-block visualization snapshots are
/// downsampled to, so the Slint panel gets a stable-sized buffer
/// regardless of the audio block size.
const LOOP_SNAPSHOT_POINTS: usize = 160;
const WOBBLE_SNAPSHOT_POINTS: usize = 160;
/// Wobble depth (samples) treated as "100%" when normalizing the
/// wobble snapshot/RMS to roughly -1..1 / 0..1 -- chosen against the
/// real max combined depth (`wow` and `flutter` each contribute up to
/// ~28 and ~4 samples of sine swing plus OU noise, see `process`).
const WOBBLE_NORMALIZE_SAMPLES: f32 = 40.0;
/// Hiss amplitude treated as "100%" when normalizing `hiss_rms` to
/// 0..1 -- matches the real max hiss amplitude at NOISE = 1.0 (see
/// `process`'s `hiss` term).
const HISS_NORMALIZE_AMPLITUDE: f32 = 0.08;

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Source,
    Drive,
    Bias,
    Tone,
    Wow,
    Flutter,
    Noise,
    Wear,
    DryWet,
    Level,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 4;

struct Params {
    source: AtomicUsize,
    /// Pre-saturation gain / saturation intensity. At or above
    /// `UNLIMITED_DRIVE_THRESHOLD`, engages "unlimited hysteresis".
    drive: AtomicF32,
    /// Hysteresis loop width/character -- see module doc comment.
    bias: AtomicF32,
    /// Post-saturation DJ-style tone filter: 0 = full low-pass,
    /// 0.5 = off, 1 = full high-pass.
    tone: AtomicF32,
    /// Slow (~0.5-1Hz-ish, stochastic) pitch wobble.
    wow: AtomicF32,
    /// Fast (~a few Hz, stochastic) pitch wobble.
    flutter: AtomicF32,
    /// Tape hiss floor.
    noise: AtomicF32,
    /// Tape age: HF rolloff + occasional amplitude dropouts.
    wear: AtomicF32,
    dry_wet: AtomicF32,
    level: AtomicF32,
    /// Real (input, output) pairs across the hysteresis follower,
    /// downsampled to `LOOP_SNAPSHOT_POINTS` and refreshed once per
    /// audio block -- the actual hysteresis-loop shape for the Slint
    /// XY-scope visual (not fabricated). See `MagnitoVisual`.
    loop_snapshot: Mutex<Vec<(f32, f32)>>,
    /// Real combined Wow+Flutter wobble-depth-in-samples signal,
    /// downsampled to `WOBBLE_SNAPSHOT_POINTS` and normalized, plus
    /// its RMS -- refreshed once per audio block. See `MagnitoVisual`.
    wobble_snapshot: Mutex<Vec<f32>>,
    wow_flutter_rms: AtomicF32,
    /// RMS of the tape hiss actually added this block, 0..1.
    hiss_rms: AtomicF32,
    /// True if an amplitude dropout was in progress at block end.
    dropout_active: AtomicBool,
    /// Peak of this block's final (post dry/wet, post level) output.
    output_peak: AtomicF32,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Magnito", modbus);
        Self {
            source: AtomicUsize::new(crate::audio_bus::NO_SOURCE),
            drive: AtomicF32::new(0.4),
            bias: AtomicF32::new(0.3),
            tone: AtomicF32::new(0.5),
            wow: AtomicF32::new(0.15),
            flutter: AtomicF32::new(0.1),
            noise: AtomicF32::new(0.0),
            wear: AtomicF32::new(0.0),
            dry_wet: AtomicF32::new(1.0),
            level: AtomicF32::new(0.8),
            loop_snapshot: Mutex::new(Vec::new()),
            wobble_snapshot: Mutex::new(Vec::new()),
            wow_flutter_rms: AtomicF32::new(0.0),
            hiss_rms: AtomicF32::new(0.0),
            dropout_active: AtomicBool::new(false),
            output_peak: AtomicF32::new(0.0),
            bus_out: audio_bus.register("Magnito"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct MagnitoApp {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Magnito's own palette: flat solid colors, not a
// device-wide theme -- Vintage magnetic tape -- warm oxide-rust on dark tape-shell brown, the color of a well-loved reel box. ---

const MAGNITO_BG: Rgb565 = Rgb565::new(4, 6, 2);
const MAGNITO_TITLE: Rgb565 = Rgb565::new(31, 60, 28);
const MAGNITO_ACCENT: Rgb565 = Rgb565::new(22, 20, 6);
const MAGNITO_DIM: Rgb565 = Rgb565::new(17, 26, 10);

impl MagnitoApp {
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

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => vec![Selection::Source],
            1 => vec![Selection::Drive, Selection::Bias, Selection::Tone],
            2 => vec![Selection::Wow, Selection::Flutter, Selection::Noise, Selection::Wear],
            _ => vec![Selection::DryWet, Selection::Level],
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

    fn group_name(&self, g: usize) -> &'static str {
        match g {
            0 => "Input",
            1 => "Saturation",
            2 => "Tape Character",
            _ => "Output",
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => self.source_name(),
            1 => {
                let drive = self.params.drive.get();
                let drive_text = if drive >= UNLIMITED_DRIVE_THRESHOLD { "MAX*".to_string() } else { format!("{:.0}%", drive * 100.0) };
                format!("drive {}, bias {:.0}%", drive_text, self.params.bias.get() * 100.0)
            }
            2 => {
                format!(
                    "wow {:.0}%, flutter {:.0}%, wear {:.0}%",
                    self.params.wow.get() * 100.0,
                    self.params.flutter.get() * 100.0,
                    self.params.wear.get() * 100.0
                )
            }
            _ => format!("{:.0}% wet, lvl {:.0}%", self.params.dry_wet.get() * 100.0, self.params.level.get() * 100.0),
        }
    }

    fn source_name(&self) -> String {
        self.audio_bus.source_name(self.params.source.load(Ordering::Relaxed))
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => "Source".into(),
            Selection::Drive => "Drive".into(),
            Selection::Bias => "Bias".into(),
            Selection::Tone => "Tone".into(),
            Selection::Wow => "Wow".into(),
            Selection::Flutter => "Flutter".into(),
            Selection::Noise => "Noise".into(),
            Selection::Wear => "Wear".into(),
            Selection::DryWet => "Dry/Wet".into(),
            Selection::Level => "Level".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => self.source_name(),
            Selection::Drive => {
                let drive = self.params.drive.get();
                if drive >= UNLIMITED_DRIVE_THRESHOLD {
                    "MAX (unlimited)".into()
                } else {
                    format!("{:.0}%", drive * 100.0)
                }
            }
            Selection::Bias => format!("{:.0}%", self.params.bias.get() * 100.0),
            Selection::Tone => {
                let tone = self.params.tone.get();
                if tone < 0.49 {
                    format!("LP {:.0}%", (0.5 - tone) * 200.0)
                } else if tone > 0.51 {
                    format!("HP {:.0}%", (tone - 0.5) * 200.0)
                } else {
                    "off".into()
                }
            }
            Selection::Wow => format!("{:.0}%", self.params.wow.get() * 100.0),
            Selection::Flutter => format!("{:.0}%", self.params.flutter.get() * 100.0),
            Selection::Noise => format!("{:.0}%", self.params.noise.get() * 100.0),
            Selection::Wear => format!("{:.0}%", self.params.wear.get() * 100.0),
            Selection::DryWet => format!("{:.0}%", self.params.dry_wet.get() * 100.0),
            Selection::Level => format!("{:.0}%", self.params.level.get() * 100.0),
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
            Selection::Drive => bump(&self.params.drive, delta, sensitivity, 0.0, 1.0),
            Selection::Bias => bump(&self.params.bias, delta, sensitivity, 0.0, 1.0),
            Selection::Tone => bump(&self.params.tone, delta, sensitivity, 0.0, 1.0),
            Selection::Wow => bump(&self.params.wow, delta, sensitivity, 0.0, 1.0),
            Selection::Flutter => bump(&self.params.flutter, delta, sensitivity, 0.0, 1.0),
            Selection::Noise => bump(&self.params.noise, delta, sensitivity, 0.0, 1.0),
            Selection::Wear => bump(&self.params.wear, delta, sensitivity, 0.0, 1.0),
            Selection::DryWet => bump(&self.params.dry_wet, delta, sensitivity, 0.0, 1.0),
            Selection::Level => bump(&self.params.level, delta, sensitivity, 0.0, 1.0),
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Source => self.params.source.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed),
            Selection::Drive => self.params.drive.set(0.4),
            Selection::Bias => self.params.bias.set(0.3),
            Selection::Tone => self.params.tone.set(0.5),
            Selection::Wow => self.params.wow.set(0.15),
            Selection::Flutter => self.params.flutter.set(0.1),
            Selection::Noise => self.params.noise.set(0.0),
            Selection::Wear => self.params.wear.set(0.0),
            Selection::DryWet => self.params.dry_wet.set(1.0),
            Selection::Level => self.params.level.set(0.8),
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

    /// The real hysteresis-loop XY plot plus the tape-character
    /// telemetry (wow/flutter wobble trace, hiss/output level, wear
    /// dropout state) -- see `Params::loop_snapshot`/`wobble_snapshot`
    /// and friends, all written once per audio block by
    /// `MagnitoProcessor::process`. See `MagnitoVisual` for the
    /// intended visual treatment of each field.
    pub(crate) fn output_visual(&self) -> MagnitoVisual {
        MagnitoVisual {
            loop_points: self.params.loop_snapshot.lock().unwrap().clone(),
            bias: self.params.bias.get(),
            unlimited: self.params.drive.get() >= UNLIMITED_DRIVE_THRESHOLD,
            wobble_snapshot: self.params.wobble_snapshot.lock().unwrap().clone(),
            wow_flutter_depth: self.params.wow_flutter_rms.get(),
            hiss_level: self.params.hiss_rms.get(),
            wear: self.params.wear.get(),
            dropout_active: self.params.dropout_active.load(Ordering::Relaxed),
            output_peak: self.params.output_peak.get(),
        }
    }
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.01).clamp(min, max);
    value.set(next);
}

/// Converts a series of real (x, y) points (roughly -1.5..1.5 on both
/// axes, with some headroom for a hot signal) into connected line-
/// segment geometry ready for Slint, the same technique
/// `crate::app::polyline_segments` uses for a 1D waveform -- just
/// driven by two real signals (input, output) instead of one sample
/// stream against a synthetic time axis.
fn xy_segments(points: &[(f32, f32)], width_px: f32, height_px: f32) -> crate::app::CurveSegments {
    if points.len() < 2 {
        return crate::app::CurveSegments::default();
    }
    let to_px = |(x, y): (f32, f32)| {
        let px = (x.clamp(-1.5, 1.5) + 1.5) / 3.0 * width_px;
        let py = height_px - (y.clamp(-1.5, 1.5) + 1.5) / 3.0 * height_px;
        (px, py)
    };
    let mut mid_x = Vec::with_capacity(points.len() - 1);
    let mut mid_y = Vec::with_capacity(points.len() - 1);
    let mut length = Vec::with_capacity(points.len() - 1);
    let mut angle_deg = Vec::with_capacity(points.len() - 1);
    for w in points.windows(2) {
        let (x0, y0) = to_px(w[0]);
        let (x1, y1) = to_px(w[1]);
        let dx = x1 - x0;
        let dy = y1 - y0;
        mid_x.push((x0 + x1) / 2.0);
        mid_y.push((y0 + y1) / 2.0);
        length.push((dx * dx + dy * dy).sqrt().max(0.5));
        angle_deg.push(dy.atan2(dx).to_degrees());
    }
    crate::app::CurveSegments { mid_x, mid_y, length, angle_deg }
}

impl App for MagnitoApp {
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

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        const PANEL_PX: f32 = 130.0;
        const WOBBLE_W: f32 = 260.0;
        const WOBBLE_H: f32 = 60.0;
        let v = self.output_visual();
        crate::app::SlintExtra::Magnito(crate::app::MagnitoExtra {
            loop_trace: xy_segments(&v.loop_points, PANEL_PX, PANEL_PX),
            bias: v.bias,
            unlimited: v.unlimited,
            wobble_trace: {
                if v.wobble_snapshot.len() >= 2 {
                    let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(&v.wobble_snapshot, WOBBLE_W, WOBBLE_H, true);
                    crate::app::CurveSegments { mid_x, mid_y, length, angle_deg }
                } else {
                    crate::app::CurveSegments::default()
                }
            },
            wow_flutter_depth: v.wow_flutter_depth,
            hiss_level: v.hiss_level,
            wear: v.wear,
            dropout_active: v.dropout_active,
            output_peak: v.output_peak,
        })
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(MagnitoProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            mem: 0.0,
            wobble_buf: vec![0.0; WOBBLE_BUF_LEN],
            wobble_write: 0,
            wow_phase: 0.0,
            flutter_phase: 0.0,
            wow_ou: 0.0,
            flutter_ou: 0.0,
            hiss_lp: 0.0,
            tone_lp: 0.0,
            wear_lp: 0.0,
            dropout_remaining: 0,
            dropout_total: 1,
            dropout_depth: 0.0,
            rng: 0xC0FF_EE42,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(MAGNITO_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, MAGNITO_TITLE);
        Text::new("Magnito", Point::new(16, 26), title).draw(fb).ok();
        let dim = MonoTextStyle::new(&SPLEEN_6X12, MAGNITO_DIM);
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
        self.list.draw_themed(fb, 16, 56, 22, 10, &display_rows, MAGNITO_BG, MAGNITO_DIM, MAGNITO_ACCENT);
        let _ = dim;
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
}

struct MagnitoProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    /// The hysteresis follower's "magnetic memory" state -- see
    /// `hysteresis_step`.
    mem: f32,
    /// A short rolling buffer the wow/flutter pitch wobble reads back
    /// from at a modulated rate -- the same "vary the delay-line read
    /// rate" trick a real tape transport's speed wobble is.
    wobble_buf: Vec<f32>,
    wobble_write: usize,
    wow_phase: f32,
    flutter_phase: f32,
    /// Ornstein-Uhlenbeck process states for Wow/Flutter -- see module
    /// doc comment and the `WOW_OU_*`/`FLUTTER_OU_*` constants.
    wow_ou: f32,
    flutter_ou: f32,
    /// One-pole lowpass state used (as signal-minus-its-own-lowpass)
    /// to band-limit the tape hiss away from DC/rumble.
    hiss_lp: f32,
    /// One-pole state shared by the TONE control's low/high-pass.
    tone_lp: f32,
    /// One-pole lowpass state for WEAR's HF-rolloff.
    wear_lp: f32,
    /// Remaining samples in an in-progress amplitude dropout (WEAR).
    dropout_remaining: u32,
    dropout_total: u32,
    dropout_depth: f32,
    rng: u32,
}

impl MagnitoProcessor {
    fn next_rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Asymmetric, direction/width-dependent hysteresis follower -- see
/// module doc comment. The output is a lagging follower (`mem`) of an
/// instantaneous saturation target, so it genuinely depends on recent
/// history (not just the instantaneous input), which is exactly what
/// magnetic hysteresis physically is.
fn hysteresis_step(x: f32, drive: f32, bias: f32, mem: &mut f32, extreme: bool) -> f32 {
    let bias = bias.clamp(0.0, 1.0);
    let gain_ceiling = if extreme { 26.0 } else { 8.0 };

    // Low BIAS mutes quiet material (a crude stand-in for the real
    // module's "low BIAS acts like a low-pass filter and mutes weaker
    // sounds") and makes whatever gets through more distorted.
    let dead_zone = (1.0 - bias) * 0.07;
    let extra_gain = 1.0 + (1.0 - bias) * 0.6;
    let squashed = if x.abs() <= dead_zone { 0.0 } else { x.signum() * (x.abs() - dead_zone) };
    let gain = 1.0 + drive.clamp(0.0, 1.0) * gain_ceiling * extra_gain;
    let target = (squashed * gain).tanh();

    // Loop width: higher BIAS narrows the hysteresis loop (faster,
    // less-lagging tracking, per the real module's manual); lower
    // BIAS widens it (slower tracking, more audible "magnetic
    // memory"). `extreme` (unlimited hysteresis) widens the possible
    // range further for harsher, less-damped coloration.
    let width = (0.12 + (1.0 - bias) * (if extreme { 0.75 } else { 0.45 })).clamp(0.02, 0.95);
    let coeff = (1.0 - width).clamp(0.02, 1.0);
    *mem += (target - *mem) * coeff;
    mem.clamp(-1.0, 1.0)
}

impl AudioProcessor for MagnitoProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        for out in buffer.iter_mut() {
            *out = 0.0;
        }

        let source_idx = self.params.source.load(Ordering::Relaxed);
        let tapped = self.audio_bus.get(source_idx).map(|b| b.lock().unwrap().clone());
        let drive = self.params.drive.get().clamp(0.0, 1.0);
        let extreme = drive >= UNLIMITED_DRIVE_THRESHOLD;
        let bias = self.params.bias.get().clamp(0.0, 1.0);
        let tone = self.params.tone.get().clamp(0.0, 1.0);
        let wow = self.params.wow.get().clamp(0.0, 1.0);
        let flutter = self.params.flutter.get().clamp(0.0, 1.0);
        let noise = self.params.noise.get().clamp(0.0, 1.0);
        let wear = self.params.wear.get().clamp(0.0, 1.0);
        let dry_wet = self.params.dry_wet.get().clamp(0.0, 1.0);
        let level = self.params.level.get().clamp(0.0, 1.0);
        let wobble_len = self.wobble_buf.len();
        let dt = 1.0 / sample_rate;

        // TONE: one-pole cutoff (Hz), derived once per block -- below
        // 0.5 is a low-pass sweeping down as the knob turns further
        // CCW, above 0.5 a high-pass (implemented as signal minus its
        // own lowpass, same convention used elsewhere in this sim)
        // sweeping up as it turns further CW.
        let tone_is_hp = tone > 0.5;
        let tone_cutoff = if tone_is_hp {
            let amt = (tone - 0.5) * 2.0;
            40.0 + amt * amt * 3500.0
        } else {
            let amt = (0.5 - tone) * 2.0;
            9000.0 * (1.0 - amt) + 200.0 * amt
        };
        let tone_alpha = dt / (1.0 / (TAU * tone_cutoff) + dt);

        // WEAR's HF rolloff: cutoff sweeps from a transparent ~20kHz
        // (wear = 0, effectively bypassed) down to a dull ~1.2kHz
        // (wear = 1, badly worn oxide).
        let wear_cutoff = 20_000.0 * (1.0 - wear) + 1200.0 * wear;
        let wear_alpha = dt / (1.0 / (TAU * wear_cutoff) + dt);

        let frames = buffer.len() / channels;
        let mut mono = vec![0.0f32; frames];
        // Real per-sample telemetry for the Slint visualization,
        // collected once per sample below and downsampled/reduced
        // once per block after the loop -- see `MagnitoVisual`.
        let mut loop_pairs: Vec<(f32, f32)> = Vec::with_capacity(frames);
        let mut wobble_raw: Vec<f32> = Vec::with_capacity(frames);
        let mut hiss_sq_sum = 0.0f32;

        for n in 0..frames {
            let dry = tapped.as_ref().and_then(|b| b.get(n % b.len().max(1)).copied()).unwrap_or(0.0);

            self.wobble_buf[self.wobble_write] = dry;
            self.wobble_write = (self.wobble_write + 1) % wobble_len;

            // Wow/Flutter: each a slow sine blended with a
            // mean-reverting Ornstein-Uhlenbeck random walk -- real
            // tape speed variation isn't perfectly periodic. See
            // module doc comment.
            self.wow_phase = (self.wow_phase + 0.7 / sample_rate).fract();
            self.flutter_phase = (self.flutter_phase + 7.0 / sample_rate).fract();
            self.wow_ou += (-WOW_OU_THETA * self.wow_ou) * dt + WOW_OU_SIGMA * self.next_rand() * dt.sqrt();
            self.wow_ou = self.wow_ou.clamp(-OU_CLAMP, OU_CLAMP);
            self.flutter_ou += (-FLUTTER_OU_THETA * self.flutter_ou) * dt + FLUTTER_OU_SIGMA * self.next_rand() * dt.sqrt();
            self.flutter_ou = self.flutter_ou.clamp(-OU_CLAMP, OU_CLAMP);

            let wobble_depth_samples = wow * (28.0 * (self.wow_phase * TAU).sin() + 3.0 * self.wow_ou)
                + flutter * (4.0 * (self.flutter_phase * TAU).sin() + 1.2 * self.flutter_ou);
            let read_pos = (self.wobble_write as f32 - 8.0 - wobble_depth_samples).rem_euclid(wobble_len as f32);
            let i0 = read_pos as usize % wobble_len;
            let i1 = (i0 + 1) % wobble_len;
            let frac = read_pos - i0 as f32;
            let wobbled = self.wobble_buf[i0] * (1.0 - frac) + self.wobble_buf[i1] * frac;

            let wet_sat = hysteresis_step(wobbled, drive, bias, &mut self.mem, extreme);
            loop_pairs.push((wobbled.clamp(-1.0, 1.0), wet_sat));
            wobble_raw.push((wobble_depth_samples / WOBBLE_NORMALIZE_SAMPLES).clamp(-1.0, 1.0));

            // NOISE: tape hiss, band-limited away from DC/rumble by
            // subtracting its own lowpassed copy (same "signal minus
            // its own lowpass = highpass" convention used elsewhere
            // in this sim, e.g. the sequencer's hi-hat voice).
            let raw_noise = self.next_rand();
            self.hiss_lp += (raw_noise - self.hiss_lp) * 0.35;
            let hiss = (raw_noise - self.hiss_lp) * noise * 0.08;
            hiss_sq_sum += hiss * hiss;
            let wet_hissy = wet_sat + hiss;

            // TONE.
            self.tone_lp += (wet_hissy - self.tone_lp) * tone_alpha;
            let wet_toned = if tone_is_hp { wet_hissy - self.tone_lp } else { self.tone_lp };

            // WEAR: HF rolloff plus occasional brief amplitude
            // dropouts, modeled as a smooth (raised-cosine) gain duck
            // rather than a hard step, so it never clicks.
            self.wear_lp += (wet_toned - self.wear_lp) * wear_alpha;
            let wear_gain = if wear > 0.0 {
                if self.dropout_remaining == 0 {
                    // A handful of dropouts per second at max WEAR;
                    // none at wear == 0.
                    if self.next_rand().abs() < wear * 0.00006 {
                        self.dropout_total = (sample_rate * (0.01 + self.next_rand().abs() * 0.03)) as u32 + 1;
                        self.dropout_remaining = self.dropout_total;
                        self.dropout_depth = 0.3 + self.next_rand().abs() * 0.5;
                    }
                }
                if self.dropout_remaining > 0 {
                    let progress = 1.0 - (self.dropout_remaining as f32 / self.dropout_total as f32);
                    self.dropout_remaining -= 1;
                    1.0 - self.dropout_depth * (std::f32::consts::PI * progress).sin()
                } else {
                    1.0
                }
            } else {
                1.0
            };
            let wet = self.wear_lp * wear_gain;

            mono[n] = ((dry * (1.0 - dry_wet) + wet * dry_wet) * level).clamp(-1.0, 1.0);
        }

        for (out, s) in buffer.chunks_mut(channels).zip(mono.iter()) {
            for ch in out.iter_mut() {
                *ch = *s;
            }
        }
        *self.params.bus_out.lock().unwrap() = mono.clone();

        // Once per block (not per-sample): downsample/reduce the real
        // telemetry collected above for the Slint visualization -- see
        // `MagnitoVisual`.
        if !loop_pairs.is_empty() {
            let points = LOOP_SNAPSHOT_POINTS.min(loop_pairs.len());
            let snapshot: Vec<(f32, f32)> = (0..points).map(|i| loop_pairs[i * loop_pairs.len() / points]).collect();
            *self.params.loop_snapshot.lock().unwrap() = snapshot;
        }
        if !wobble_raw.is_empty() {
            let points = WOBBLE_SNAPSHOT_POINTS.min(wobble_raw.len());
            let snapshot: Vec<f32> = (0..points).map(|i| wobble_raw[i * wobble_raw.len() / points]).collect();
            *self.params.wobble_snapshot.lock().unwrap() = snapshot;
            let rms = (wobble_raw.iter().map(|w| w * w).sum::<f32>() / wobble_raw.len() as f32).sqrt();
            self.params.wow_flutter_rms.set(rms.clamp(0.0, 1.0));
        }
        if frames > 0 {
            let hiss_rms = (hiss_sq_sum / frames as f32).sqrt() / HISS_NORMALIZE_AMPLITUDE;
            self.params.hiss_rms.set(hiss_rms.clamp(0.0, 1.0));
        }
        self.params.dropout_active.store(self.dropout_remaining > 0, Ordering::Relaxed);
        let peak = mono.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        self.params.output_peak.set(peak.clamp(0.0, 1.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> (MagnitoApp, Arc<AudioBus>) {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let app = MagnitoApp::new(sensitivity, nav_speed, modbus, Arc::clone(&audio_bus), mixer_bus);
        (app, audio_bus)
    }

    /// No tapped source and every default-off coloration (noise, wear)
    /// must stay exactly silent, not just "quiet".
    #[test]
    fn no_source_is_silence_not_a_panic() {
        let (mut app, _audio_bus) = new_app();
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..10 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|&s| s == 0.0));
    }

    /// Loud input at full drive must stay bounded (soft-clipped, not
    /// exploding) and audible -- this also exercises "unlimited
    /// hysteresis" mode, since drive at 1.0 is above the threshold.
    #[test]
    fn loud_input_at_full_drive_stays_bounded_and_audible() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed); // Magnito registers its own bus first (index 0) -- "Source" lands at 1.
        *source.lock().unwrap() = vec![0.99; 512];
        app.params.drive.set(1.0);
        app.params.bias.set(0.9);
        app.params.dry_wet.set(1.0);
        app.params.level.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..50 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|s| s.is_finite()));
        let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.1, "expected audible saturated output, got peak {peak}");
        assert!(peak <= 1.0001, "expected soft-clipped output, got peak {peak}");
    }

    /// A quiet signal with BIAS at its lowest must be pulled toward
    /// silence by the dead-zone (the real module's "low BIAS acts
    /// like a low-pass filter and mutes weaker sounds").
    #[test]
    fn low_bias_mutes_quiet_material() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.02; 512]; // very quiet
        app.params.drive.set(0.3);
        app.params.bias.set(0.0);
        app.params.dry_wet.set(1.0);
        app.params.level.set(1.0);
        app.params.wow.set(0.0);
        app.params.flutter.set(0.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak < 0.01, "expected near-silence from a quiet signal at zero bias, got peak {peak}");
    }

    /// Max wow/flutter (now Ornstein-Uhlenbeck-driven) must never blow
    /// up or produce non-finite samples -- the pitch-wobble read must
    /// stay in bounds.
    #[test]
    fn max_wow_and_flutter_stays_bounded_and_finite() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed); // Magnito registers its own bus first (index 0) -- "Source" lands at 1.
        *source.lock().unwrap() = vec![0.7; 512];
        app.params.wow.set(1.0);
        app.params.flutter.set(1.0);
        app.params.dry_wet.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..300 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|s| s.is_finite()));
    }

    /// TONE fully counter-clockwise (low-pass) must audibly attenuate
    /// a high-frequency-heavy (alternating +/-1) source relative to
    /// TONE at center (off).
    #[test]
    fn tone_lowpass_attenuates_high_frequency_content() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        let mut hf = vec![0.0f32; 512];
        for (i, s) in hf.iter_mut().enumerate() {
            *s = if i % 2 == 0 { 0.9 } else { -0.9 };
        }
        *source.lock().unwrap() = hf;
        app.params.drive.set(0.0); // isolate the tone filter from saturation coloration
        app.params.bias.set(1.0); // no dead-zone muting
        app.params.dry_wet.set(1.0);
        app.params.wow.set(0.0);
        app.params.flutter.set(0.0);
        app.params.level.set(1.0);

        app.params.tone.set(0.0); // full low-pass
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let lp_energy: f32 = buffer.iter().map(|s| s * s).sum();

        app.params.tone.set(0.5); // off
        let mut processor2 = app.audio_processor().unwrap();
        let mut buffer2 = vec![0.0f32; 512 * 2];
        for _ in 0..20 {
            processor2.process(&mut buffer2, 2, 48000.0);
        }
        let flat_energy: f32 = buffer2.iter().map(|s| s * s).sum();

        assert!(lp_energy < flat_energy * 0.5, "expected low-pass TONE to attenuate a HF-heavy source, got lp={lp_energy} flat={flat_energy}");
    }

    /// NOISE at zero must never add hiss; at max it must add audible,
    /// bounded, finite hiss even with no tapped source.
    #[test]
    fn noise_adds_audible_bounded_hiss() {
        let (mut app, _audio_bus) = new_app();
        app.params.noise.set(0.0);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        assert!(buffer.iter().all(|&s| s == 0.0), "expected exact silence with noise at zero");

        let (mut app2, _audio_bus2) = new_app();
        app2.params.noise.set(1.0);
        app2.params.dry_wet.set(1.0);
        app2.params.level.set(1.0);
        let mut processor2 = app2.audio_processor().unwrap();
        let mut buffer2 = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..40 {
            processor2.process(&mut buffer2, 2, 48000.0);
            peak = peak.max(buffer2.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(buffer2.iter().all(|s| s.is_finite()));
        assert!(peak > 0.0001, "expected audible hiss at max noise, got peak {peak}");
        assert!(peak <= 1.0001, "expected bounded hiss, got peak {peak}");
    }

    /// WEAR at max must stay bounded/finite over a long run (exercises
    /// both the HF rolloff and the dropout envelope) and must darken
    /// (reduce high-frequency energy of) a HF-heavy source.
    #[test]
    fn wear_stays_bounded_and_darkens_the_signal() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        let mut hf = vec![0.0f32; 512];
        for (i, s) in hf.iter_mut().enumerate() {
            *s = if i % 2 == 0 { 0.9 } else { -0.9 };
        }
        *source.lock().unwrap() = hf;
        app.params.drive.set(0.0);
        app.params.bias.set(1.0);
        app.params.dry_wet.set(1.0);
        app.params.wow.set(0.0);
        app.params.flutter.set(0.0);
        app.params.tone.set(0.5);
        app.params.level.set(1.0);

        app.params.wear.set(0.0);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..10 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let flat_energy: f32 = buffer.iter().map(|s| s * s).sum();

        app.params.wear.set(1.0);
        let mut processor2 = app.audio_processor().unwrap();
        let mut buffer2 = vec![0.0f32; 512 * 2];
        for _ in 0..200 {
            processor2.process(&mut buffer2, 2, 48000.0);
        }
        assert!(buffer2.iter().all(|s| s.is_finite()), "expected finite output under sustained max WEAR");
        assert!(buffer2.iter().all(|s| s.abs() <= 1.0001), "expected bounded output under sustained max WEAR");

        let mut processor3 = app.audio_processor().unwrap();
        let mut buffer3 = vec![0.0f32; 512 * 2];
        for _ in 0..10 {
            processor3.process(&mut buffer3, 2, 48000.0);
        }
        let worn_energy: f32 = buffer3.iter().map(|s| s * s).sum();
        assert!(worn_energy < flat_energy, "expected max WEAR to reduce HF energy relative to no wear, got worn={worn_energy} flat={flat_energy}");
    }

    #[test]
    fn republishes_its_output_on_the_audio_bus() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed); // Magnito registers its own bus first (index 0) -- "Source" lands at 1.
        *source.lock().unwrap() = vec![0.5; 256];

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 256 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        let published = app.params.bus_out.lock().unwrap();
        assert_eq!(published.len(), 256);
    }

    /// The hysteresis-loop snapshot exposed via `output_visual()` must
    /// show a real, driven loop -- non-empty, finite, bounded, and
    /// actually varying (not a flat/silent line) when a loud, changing
    /// signal is run through a saturating amount of DRIVE.
    #[test]
    fn output_visual_loop_snapshot_shows_a_real_driven_loop() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed); // Magnito registers its own bus first (index 0) -- "Source" lands at 1.
        let mut sig = vec![0.0f32; 512];
        for (i, s) in sig.iter_mut().enumerate() {
            *s = (i as f32 * 0.3).sin() * 0.9;
        }
        *source.lock().unwrap() = sig;
        app.params.drive.set(0.8);
        app.params.bias.set(0.2); // wide loop -- lots of audible "magnetic memory"
        app.params.wow.set(0.0);
        app.params.flutter.set(0.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
        }

        let visual = app.output_visual();
        assert!(!visual.loop_points.is_empty(), "expected a non-empty hysteresis loop snapshot");
        assert!(
            visual.loop_points.iter().all(|&(x, y)| x.is_finite() && y.is_finite() && x.abs() <= 1.0001 && y.abs() <= 1.0001),
            "expected finite, bounded loop points"
        );
        let y_min = visual.loop_points.iter().fold(f32::INFINITY, |m, &(_, y)| m.min(y));
        let y_max = visual.loop_points.iter().fold(f32::NEG_INFINITY, |m, &(_, y)| m.max(y));
        assert!(y_max - y_min > 0.1, "expected the driven loop's output to actually vary, got range {}", y_max - y_min);
        assert!(!visual.unlimited, "0.8 drive must not be in unlimited-hysteresis mode");
    }

    /// With no tapped source and no coloration, `output_visual()` must
    /// report a silent/flat state, not fabricated activity.
    #[test]
    fn output_visual_is_quiescent_with_no_source_and_no_coloration() {
        let (mut app, _audio_bus) = new_app();
        app.params.wow.set(0.0);
        app.params.flutter.set(0.0);
        app.params.noise.set(0.0);
        app.params.wear.set(0.0);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..10 {
            processor.process(&mut buffer, 2, 48000.0);
        }

        let visual = app.output_visual();
        assert!(visual.loop_points.iter().all(|&(x, y)| x == 0.0 && y == 0.0), "expected a silent loop with no tapped source");
        assert_eq!(visual.hiss_level, 0.0, "expected zero hiss level with NOISE at zero");
        assert!(!visual.dropout_active, "expected no dropout with WEAR at zero");
        assert_eq!(visual.output_peak, 0.0, "expected zero output peak with nothing feeding the processor");
    }

    /// WEAR at max, run long enough to guarantee a dropout occurs,
    /// must eventually report `dropout_active` true via
    /// `output_visual()` -- real dropout state, not a decorative flag.
    #[test]
    fn output_visual_reports_real_dropout_state() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.6; 512];
        app.params.wear.set(1.0);
        app.params.dry_wet.set(1.0);
        app.params.level.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut saw_dropout = false;
        for _ in 0..400 {
            processor.process(&mut buffer, 2, 48000.0);
            if app.output_visual().dropout_active {
                saw_dropout = true;
                break;
            }
        }
        assert!(saw_dropout, "expected at least one real dropout to be reported over a long run at max WEAR");
    }
}
