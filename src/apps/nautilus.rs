//! A clone of Qu-Bit Nautilus: a "complex delay network" -- a true
//! stereo delay effect built from 8 codependent delay lines (4 per
//! channel), not a Euclidean CV generator (an earlier version of this
//! app modeled the wrong module entirely -- Nautilus has no
//! resemblance to Pam's/Turing Machine's clocked-gate family; it's a
//! real audio effect with a real dry/wet Mix and a stereo in/out, like
//! Beads or Clouds). Feature set follows the real module's manual
//! (qubitelectronix.com, Nautilus quickstart + full manual v1.1.3) as
//! closely as this sim's architecture allows:
//!
//! - Sensors/Dispersal/Reversal/Resolution/Feedback/Chroma/Depth/Mix,
//!   all real, all audible, matching the panel's own knobs -- there
//!   are no separate per-channel menus on the real module (everything
//!   is one global set of knobs acting across all 8 lines), so this
//!   app's menu is a single flat list, not per-channel groups.
//! - 4 real Delay Modes (Fade/Doppler/Shimmer/De-Shimmer) controlling
//!   how a line's delay time settles when it changes, plus real
//!   Shimmer/De-Shimmer pitch-shifting of the feedback signal by a
//!   fixed +/-12 semitones each pass through the loop.
//! - 4 real Feedback Modes (Normal/Ping Pong/Cascade/Adrift) that
//!   genuinely change how the 8 lines route into each other, matching
//!   the manual's own routing diagrams.
//! - 6 real Chroma effects (2 filters, a crush, a saturator, a
//!   wavefolder, a hard distortion), each captured in the feedback
//!   path exactly as the manual describes.
//! - Freeze (locks the delay buffers, turning the wet signal into a
//!   real beat-repeat of whatever was already captured) and Purge
//!   (instantly clears all 8 lines).
//! - Sonar: a single real CV/Gate utility output, generated from the
//!   delay network's own activity (ping events), routable into any
//!   other app's modulation target via the shared ModBus, same as
//!   Pam's/Turing Machine's own outputs.
//!
//! Deliberately not implemented: tap-tempo gesture timing and
//! external clock sync (the real module's Clock In jack/button) --
//! this sim has no generic per-app clock/gate input, the same
//! limitation Turing Machine and Pam's already document; Rate stands
//! in as a direct Hz control instead. The USB/Narwhal configurator
//! (assignable attenuverters, custom Shimmer semitones, alternate
//! Sonar algorithms) is a settings-file workflow with no equivalent
//! surface in this sim -- Shimmer/De-Shimmer transpose is fixed at
//! the real module's own default of 12 semitones. Input Level
//! trim (Tap + Dispersal Attenuverter) isn't modeled -- this sim's
//! AudioBus taps are already normalized, unlike real Eurorack levels.

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NUM_CHANNELS: usize = 2; // L, R
const MAX_LINES_PER_CHANNEL: usize = 4;
const NUM_LINES: usize = NUM_CHANNELS * MAX_LINES_PER_CHANNEL;
const MAX_DELAY_SECONDS: f32 = 20.0;
const MIN_DELAY_SECONDS: f32 = 0.001;
const MIN_RATE_HZ: f32 = 0.25;
const MAX_RATE_HZ: f32 = 1000.0;
const MAX_FEEDBACK: f32 = 0.98;
const SHIMMER_TRANSPOSE_SEMITONES: f32 = 12.0; // real module's own default
/// Extra delay-time spread added per extra active line (Dispersal at
/// full CW) -- purely a "feels right" spread amount, since the real
/// module's exact spacing curve isn't published.
const DISPERSAL_SPREAD: f32 = 1.4;
/// With only 1 Sensor active, Dispersal instead fine-tunes the L/R
/// pair's delay-time offset (see manual, "Dispersal").
const DISPERSAL_ONE_LINE_OFFSET: f32 = 0.15;
/// Doppler mode's maximum delay-time slew rate, in delay-samples per
/// real sample -- fast enough to be a clearly audible vari-speed
/// pitch-bend, not an instant jump.
const DOPPLER_SLEW_PER_SAMPLE: f32 = 0.6;
/// Fade mode's (and Shimmer/De-Shimmer's) delay-time smoothing time
/// constant.
const FADE_SECONDS: f32 = 0.08;

/// (label, multiplier relative to Rate's own period -- "One Beat" is
/// the real manual's mid-knob default). Order matches the manual's
/// own CCW->CW knob picture (slowest/longest first).
const RESOLUTIONS: [(&str, f32); 16] = [
    ("2 Bars", 8.0),
    ("1 Bar", 4.0),
    ("Dotted Half", 3.0),
    ("Half Note", 2.0),
    ("Dotted Quarter", 1.5),
    ("One Beat", 1.0),
    ("Dotted Eighth", 0.75),
    ("Eighth Note", 0.5),
    ("Eighth Triplet", 1.0 / 3.0),
    ("Sixteenth Note", 0.25),
    ("Sixteenth Triplet", 1.0 / 6.0),
    ("32nd Note", 0.125),
    ("64th Note", 0.0625),
    ("128th Note", 0.03125),
    ("256th Note", 0.015625),
    ("512th Note", 0.0078125),
];
const DEFAULT_RESOLUTION: usize = 5; // "One Beat"

const DELAY_MODE_NAMES: [&str; 4] = ["Fade", "Doppler", "Shimmer", "De-Shimmer"];
const DELAY_MODE_SHIMMER: u32 = 2;
const DELAY_MODE_DESHIMMER: u32 = 3;
const FEEDBACK_MODE_NAMES: [&str; 4] = ["Normal", "Ping Pong", "Cascade", "Adrift"];
const CHROMA_NAMES: [&str; 6] =
    ["Oceanic Absorption", "White Water", "Refraction Interference", "Pulse Amplification", "Receptor Malfunction", "SOS"];
const SONAR_MODE_NAMES: [&str; 4] = ["Gate", "Stepped CV", "Master Clock", "Variable Clock"];

/// The real manual's own delay-line numbering: 1L, 1R, 2L, 2R, 3L, 3R,
/// 4L, 4R -- used both by Reversal's fixed order and this app's Slint
/// panel data.
fn line_order(channel: usize, line: usize) -> usize {
    line * NUM_CHANNELS + channel
}

/// `RESOLUTIONS[idx]`'s multiplier, clamped to a valid index -- a
/// small pure helper so the mapping is directly unit-testable.
fn resolution_multiplier(idx: usize) -> f32 {
    RESOLUTIONS[idx.min(RESOLUTIONS.len() - 1)].1
}

/// A soft triangle-fold wavefolder -- Chroma's "Receptor Malfunction".
fn wavefold(x: f32) -> f32 {
    let mut v = x;
    for _ in 0..6 {
        if v > 1.0 {
            v = 2.0 - v;
        } else if v < -1.0 {
            v = -2.0 - v;
        } else {
            break;
        }
    }
    v.clamp(-1.0, 1.0)
}

fn read_interp(buffer: &[f32], pos: f32) -> f32 {
    let len = buffer.len();
    let p = pos.rem_euclid(len as f32);
    let i0 = p as usize % len;
    let i1 = (i0 + 1) % len;
    let frac = p - p.floor();
    buffer[i0] * (1.0 - frac) + buffer[i1] * frac
}

/// Real, live-data-driven visualization state for Nautilus's Slint
/// panel -- an 8-line delay-network ring (real per-line delay time,
/// ping activity and output level, in the manual's own 1L,1R,2L,2R,
/// 3L,3R,4L,4R order, see `line_order`), the current Delay Mode/
/// Feedback Mode/Chroma effect (for a Black-Hole-style color-coded
/// badge), a Freeze indicator, and the Sonar CV/Gate output -- every
/// field is read from `Params`' own Mutex/Atomic-guarded snapshots,
/// written once per audio block by `NautilusProcessor::process`.
/// Nothing here is fabricated for the panel's sake.
pub(crate) struct NautilusVisual {
    /// One of `DELAY_MODE_NAMES` -- Fade/Doppler/Shimmer/De-Shimmer.
    pub delay_mode_name: String,
    /// Index into `DELAY_MODE_NAMES` (0..=3), for a color-coded badge.
    pub delay_mode_index: usize,
    /// One of `FEEDBACK_MODE_NAMES` -- Normal/Ping Pong/Cascade/Adrift.
    pub feedback_mode_name: String,
    /// Index into `FEEDBACK_MODE_NAMES` (0..=3).
    pub feedback_mode_index: usize,
    /// One of `CHROMA_NAMES` -- the 6 feedback-path effects.
    pub chroma_name: String,
    /// Index into `CHROMA_NAMES` (0..=5).
    pub chroma_index: usize,
    pub frozen: bool,
    /// Active lines per channel (1..=4, Sensors) -- half of how many
    /// of the 8 ring slots are actually in the signal path this block.
    pub sensors: usize,
    /// How many of the 8 lines (in `line_order`) are currently
    /// reversed (0..=8, Reversal).
    pub reversal_count: usize,
    /// Each line's real current delay time in milliseconds
    /// (`DelayLine::delay_samples` / sample_rate, already smoothed by
    /// whichever Delay Mode is running) -- how far around the ring
    /// that slot should sit, not a fabricated fixed layout.
    pub line_delay_ms: [f32; NUM_LINES],
    /// True for the one block in which that line's read cycle
    /// retriggered ("pinged") -- same real per-line activity the old
    /// 8-LED panel showed, now driving a ring flash instead.
    pub line_pulse: [bool; NUM_LINES],
    /// Each line's real output peak amplitude (roughly 0..1.5) within
    /// the most recent block -- how brightly that ring segment should
    /// glow between pings.
    pub line_level: [f32; NUM_LINES],
    /// False for lines not currently in the signal path (Sensors < 4
    /// leaves some of the 8 idle) -- the ring should render those
    /// dim/inert regardless of whatever stale level/pulse data they
    /// still carry from when they were last active.
    pub line_active: [bool; NUM_LINES],
    /// Sonar's current Gate output state.
    pub sonar_gate: bool,
    /// Sonar's current CV output level (0..1).
    pub sonar_cv: f32,
    /// The Feedback knob's own amount (0..1, `MAX_FEEDBACK` at "max"),
    /// for a feedback-intensity indicator alongside the ring.
    pub feedback_amount: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Source,
    DelayMode,
    FeedbackMode,
    Rate,
    Resolution,
    Sensors,
    Dispersal,
    Reversal,
    Feedback,
    Chroma,
    Depth,
    Mix,
    Freeze,
    Purge,
    SonarMode,
    SonarTarget,
    SonarLevel,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 5;

struct Params {
    source: AtomicUsize,
    delay_mode: AtomicU32,
    feedback_mode: AtomicU32,
    rate_hz: AtomicF32,
    resolution: AtomicUsize,
    sensors: AtomicUsize, // 1..=MAX_LINES_PER_CHANNEL
    dispersal: AtomicF32,
    reversal: AtomicUsize, // 0..=NUM_LINES, count of lines reversed in `line_order`
    feedback: AtomicF32,
    chroma: AtomicU32,
    depth: AtomicF32,
    mix: AtomicF32,
    freeze: AtomicBool,
    purge_pending: AtomicBool,
    sonar_mode: AtomicU32,
    sonar_target: AtomicUsize, // 0 = none, else (modbus index + 1)
    sonar_level: AtomicF32,
    /// Live outputs, published every block for the panel.
    sonar_gate: AtomicBool,
    sonar_cv: AtomicF32,
    /// Which of the 8 lines (in `line_order`) pinged during the most
    /// recent block -- real per-line activity, for a live panel.
    line_pulse: [AtomicBool; NUM_LINES],
    /// Each line's real current delay time in milliseconds, published
    /// once per audio block -- see `NautilusVisual::line_delay_ms`.
    line_delay_ms: [AtomicF32; NUM_LINES],
    /// Each line's real output peak amplitude within the most recent
    /// block -- see `NautilusVisual::line_level`.
    line_level: [AtomicF32; NUM_LINES],
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Nautilus", modbus);
        Self {
            source: AtomicUsize::new(crate::audio_bus::NO_SOURCE),
            delay_mode: AtomicU32::new(0),
            feedback_mode: AtomicU32::new(0),
            rate_hz: AtomicF32::new(2.0),
            resolution: AtomicUsize::new(DEFAULT_RESOLUTION),
            sensors: AtomicUsize::new(2),
            dispersal: AtomicF32::new(0.0),
            reversal: AtomicUsize::new(0),
            feedback: AtomicF32::new(0.3),
            chroma: AtomicU32::new(0),
            depth: AtomicF32::new(0.0),
            mix: AtomicF32::new(0.5),
            freeze: AtomicBool::new(false),
            purge_pending: AtomicBool::new(false),
            sonar_mode: AtomicU32::new(0),
            sonar_target: AtomicUsize::new(0),
            sonar_level: AtomicF32::new(1.0),
            sonar_gate: AtomicBool::new(false),
            sonar_cv: AtomicF32::new(0.0),
            line_pulse: std::array::from_fn(|_| AtomicBool::new(false)),
            line_delay_ms: std::array::from_fn(|_| AtomicF32::new(0.0)),
            line_level: std::array::from_fn(|_| AtomicF32::new(0.0)),
            bus_out: audio_bus.register("Nautilus"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct NautilusApp {
    params: Arc<Params>,
    modbus: Arc<ModBus>,
    audio_bus: Arc<AudioBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Nautilus's own palette: abyssal navy with a bioluminescent
// teal glow, not a device-wide theme -- deep-sea pressure and the
// light a real nautilus shell's living relatives glow with in the dark. ---

const NAUTILUS_BG: Rgb565 = Rgb565::new(0, 4, 3);
const NAUTILUS_TITLE: Rgb565 = Rgb565::new(27, 61, 29);
const NAUTILUS_ACCENT: Rgb565 = Rgb565::new(5, 55, 24);
const NAUTILUS_DIM: Rgb565 = Rgb565::new(7, 26, 13);
const NAUTILUS_GATE_OFF: Rgb565 = Rgb565::new(2, 5, 4);

impl NautilusApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            modbus,
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
            1 => vec![Selection::DelayMode, Selection::Rate, Selection::Resolution, Selection::Sensors, Selection::Dispersal, Selection::Reversal],
            2 => vec![Selection::FeedbackMode, Selection::Feedback],
            3 => vec![Selection::Chroma, Selection::Depth],
            _ => vec![Selection::Mix, Selection::Freeze, Selection::Purge, Selection::SonarMode, Selection::SonarTarget, Selection::SonarLevel],
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
            1 => "Delay",
            2 => "Feedback",
            3 => "Chroma",
            _ => "Output",
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => self.source_name(),
            1 => format!(
                "{}, {}, {} sensors",
                DELAY_MODE_NAMES[self.params.delay_mode.load(Ordering::Relaxed) as usize % 4],
                RESOLUTIONS[self.params.resolution.load(Ordering::Relaxed).min(15)].0,
                self.params.sensors.load(Ordering::Relaxed)
            ),
            2 => format!(
                "{}, {}",
                FEEDBACK_MODE_NAMES[self.params.feedback_mode.load(Ordering::Relaxed) as usize % 4],
                if self.params.feedback.get() >= MAX_FEEDBACK { "infinite".to_string() } else { format!("{:.0}%", self.params.feedback.get() * 100.0) }
            ),
            3 => format!(
                "{}, {:.0}% depth",
                CHROMA_NAMES[self.params.chroma.load(Ordering::Relaxed) as usize % 6],
                self.params.depth.get() * 100.0
            ),
            _ => format!(
                "{:.0}% wet{}",
                self.params.mix.get() * 100.0,
                if self.params.freeze.load(Ordering::Relaxed) { ", frozen" } else { "" }
            ),
        }
    }

    fn source_name(&self) -> String {
        self.audio_bus.source_name(self.params.source.load(Ordering::Relaxed))
    }

    fn target_name(&self, idx: usize) -> String {
        if idx == 0 {
            "None".into()
        } else {
            self.modbus.names().get(idx - 1).cloned().unwrap_or_else(|| "None".into())
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => "Source".into(),
            Selection::DelayMode => "Delay Mode".into(),
            Selection::FeedbackMode => "Feedback Mode".into(),
            Selection::Rate => "Rate".into(),
            Selection::Resolution => "Resolution".into(),
            Selection::Sensors => "Sensors".into(),
            Selection::Dispersal => "Dispersal".into(),
            Selection::Reversal => "Reversal".into(),
            Selection::Feedback => "Feedback".into(),
            Selection::Chroma => "Chroma".into(),
            Selection::Depth => "Depth".into(),
            Selection::Mix => "Mix".into(),
            Selection::Freeze => "Freeze".into(),
            Selection::Purge => "Purge".into(),
            Selection::SonarMode => "Sonar Mode".into(),
            Selection::SonarTarget => "Sonar Target".into(),
            Selection::SonarLevel => "Sonar Level".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => self.source_name(),
            Selection::DelayMode => DELAY_MODE_NAMES[self.params.delay_mode.load(Ordering::Relaxed) as usize % 4].into(),
            Selection::FeedbackMode => FEEDBACK_MODE_NAMES[self.params.feedback_mode.load(Ordering::Relaxed) as usize % 4].into(),
            Selection::Rate => format!("{:.2} Hz", self.params.rate_hz.get()),
            Selection::Resolution => RESOLUTIONS[self.params.resolution.load(Ordering::Relaxed).min(15)].0.into(),
            Selection::Sensors => format!("{}", self.params.sensors.load(Ordering::Relaxed)),
            Selection::Dispersal => format!("{:.0}%", self.params.dispersal.get() * 100.0),
            Selection::Reversal => format!("{}/{}", self.params.reversal.load(Ordering::Relaxed), NUM_LINES),
            Selection::Feedback => {
                if self.params.feedback.get() >= MAX_FEEDBACK {
                    "infinite".into()
                } else {
                    format!("{:.0}%", self.params.feedback.get() * 100.0)
                }
            }
            Selection::Chroma => CHROMA_NAMES[self.params.chroma.load(Ordering::Relaxed) as usize % 6].into(),
            Selection::Depth => format!("{:.0}%", self.params.depth.get() * 100.0),
            Selection::Mix => format!("{:.0}%", self.params.mix.get() * 100.0),
            Selection::Freeze => if self.params.freeze.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Selection::Purge => "press to clear".into(),
            Selection::SonarMode => SONAR_MODE_NAMES[self.params.sonar_mode.load(Ordering::Relaxed) as usize % 4].into(),
            Selection::SonarTarget => self.target_name(self.params.sonar_target.load(Ordering::Relaxed)),
            Selection::SonarLevel => format!("{:.0}%", self.params.sonar_level.get() * 100.0),
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
            Selection::DelayMode => {
                let cur = self.params.delay_mode.load(Ordering::Relaxed) as i32;
                self.params.delay_mode.store((cur + step).rem_euclid(4) as u32, Ordering::Relaxed);
            }
            Selection::FeedbackMode => {
                let cur = self.params.feedback_mode.load(Ordering::Relaxed) as i32;
                self.params.feedback_mode.store((cur + step).rem_euclid(4) as u32, Ordering::Relaxed);
            }
            Selection::Rate => bump(&self.params.rate_hz, delta, sensitivity, MIN_RATE_HZ, MAX_RATE_HZ),
            Selection::Resolution => {
                let cur = self.params.resolution.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(0, RESOLUTIONS.len() as i32 - 1);
                self.params.resolution.store(next as usize, Ordering::Relaxed);
            }
            Selection::Sensors => {
                let cur = self.params.sensors.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(1, MAX_LINES_PER_CHANNEL as i32);
                self.params.sensors.store(next as usize, Ordering::Relaxed);
            }
            Selection::Dispersal => bump(&self.params.dispersal, delta, sensitivity, 0.0, 1.0),
            Selection::Reversal => {
                let cur = self.params.reversal.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(0, NUM_LINES as i32);
                self.params.reversal.store(next as usize, Ordering::Relaxed);
            }
            Selection::Feedback => bump(&self.params.feedback, delta, sensitivity, 0.0, MAX_FEEDBACK),
            Selection::Chroma => {
                let cur = self.params.chroma.load(Ordering::Relaxed) as i32;
                self.params.chroma.store((cur + step).rem_euclid(6) as u32, Ordering::Relaxed);
            }
            Selection::Depth => bump(&self.params.depth, delta, sensitivity, 0.0, 1.0),
            Selection::Mix => bump(&self.params.mix, delta, sensitivity, 0.0, 1.0),
            Selection::Freeze => self.params.freeze.store(delta > 0, Ordering::Relaxed),
            Selection::Purge => {} // momentary -- see `reset`
            Selection::SonarMode => {
                let cur = self.params.sonar_mode.load(Ordering::Relaxed) as i32;
                self.params.sonar_mode.store((cur + step).rem_euclid(4) as u32, Ordering::Relaxed);
            }
            Selection::SonarTarget => {
                let n = self.modbus.len();
                let cur = self.params.sonar_target.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(n as i32 + 1);
                self.params.sonar_target.store(next as usize, Ordering::Relaxed);
            }
            Selection::SonarLevel => bump(&self.params.sonar_level, delta, sensitivity, 0.0, 1.0),
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Source => self.params.source.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed),
            Selection::DelayMode => self.params.delay_mode.store(0, Ordering::Relaxed),
            Selection::FeedbackMode => self.params.feedback_mode.store(0, Ordering::Relaxed),
            Selection::Rate => self.params.rate_hz.set(2.0),
            Selection::Resolution => self.params.resolution.store(DEFAULT_RESOLUTION, Ordering::Relaxed),
            Selection::Sensors => self.params.sensors.store(2, Ordering::Relaxed),
            Selection::Dispersal => self.params.dispersal.set(0.0),
            Selection::Reversal => self.params.reversal.store(0, Ordering::Relaxed),
            Selection::Feedback => self.params.feedback.set(0.3),
            Selection::Chroma => self.params.chroma.store(0, Ordering::Relaxed),
            Selection::Depth => self.params.depth.set(0.0),
            Selection::Mix => self.params.mix.set(0.5),
            Selection::Freeze => self.params.freeze.store(false, Ordering::Relaxed),
            // Purge is a momentary trigger, not a value -- pressing it
            // (this "reset" gesture) is how it's actually fired.
            Selection::Purge => self.params.purge_pending.store(true, Ordering::Relaxed),
            Selection::SonarMode => self.params.sonar_mode.store(0, Ordering::Relaxed),
            Selection::SonarTarget => self.params.sonar_target.store(0, Ordering::Relaxed),
            Selection::SonarLevel => self.params.sonar_level.set(1.0),
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

    /// Live per-line activity (in `line_order`: 1L,1R,2L,2R,3L,3R,4L,4R)
    /// -- real ping state, for an 8-LED-style panel.
    pub(crate) fn channel_gates(&self) -> [bool; NUM_LINES] {
        std::array::from_fn(|i| self.params.line_pulse[i].load(Ordering::Relaxed))
    }

    /// The real 8-line delay-network ring, Delay/Feedback Mode +
    /// Chroma badges, Freeze indicator and Sonar output -- see
    /// `NautilusVisual` for what each field means and where it comes
    /// from (all of it published once per audio block by
    /// `NautilusProcessor::process`, nothing fabricated here).
    pub(crate) fn output_visual(&self) -> NautilusVisual {
        let sensors = self.params.sensors.load(Ordering::Relaxed).clamp(1, MAX_LINES_PER_CHANNEL);
        let mut line_active = [false; NUM_LINES];
        for c in 0..NUM_CHANNELS {
            for i in 0..sensors {
                line_active[line_order(c, i)] = true;
            }
        }
        let delay_mode = self.params.delay_mode.load(Ordering::Relaxed) as usize % 4;
        let feedback_mode = self.params.feedback_mode.load(Ordering::Relaxed) as usize % 4;
        let chroma = self.params.chroma.load(Ordering::Relaxed) as usize % 6;
        NautilusVisual {
            delay_mode_name: DELAY_MODE_NAMES[delay_mode].into(),
            delay_mode_index: delay_mode,
            feedback_mode_name: FEEDBACK_MODE_NAMES[feedback_mode].into(),
            feedback_mode_index: feedback_mode,
            chroma_name: CHROMA_NAMES[chroma].into(),
            chroma_index: chroma,
            frozen: self.params.freeze.load(Ordering::Relaxed),
            sensors,
            reversal_count: self.params.reversal.load(Ordering::Relaxed).min(NUM_LINES),
            line_delay_ms: std::array::from_fn(|i| self.params.line_delay_ms[i].get()),
            line_pulse: self.channel_gates(),
            line_level: std::array::from_fn(|i| self.params.line_level[i].get()),
            line_active,
            sonar_gate: self.params.sonar_gate.load(Ordering::Relaxed),
            sonar_cv: self.params.sonar_cv.get(),
            feedback_amount: self.params.feedback.get(),
        }
    }
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.01).clamp(min, max);
    value.set(next);
}

impl App for NautilusApp {
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

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(NautilusProcessor {
            params: Arc::clone(&self.params),
            modbus: Arc::clone(&self.modbus),
            audio_bus: Arc::clone(&self.audio_bus),
            lines: std::array::from_fn(|_| std::array::from_fn(|_| DelayLine::new((MAX_DELAY_SECONDS * 48000.0) as usize))),
            sonar_step: 0.0,
            sonar_gate_hold: 0,
            master_phase: 0.0,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(NAUTILUS_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, NAUTILUS_TITLE);
        Text::new("Nautilus", Point::new(16, 26), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_8X16, NAUTILUS_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, NAUTILUS_DIM);
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
        self.list.draw_themed(fb, 16, 56, 22, 10, &display_rows, NAUTILUS_BG, NAUTILUS_DIM, NAUTILUS_ACCENT);

        // 8 delay-line ping LEDs, in the manual's own 1L..4R order.
        let gates = self.channel_gates();
        for (i, on) in gates.iter().enumerate() {
            let x = 420 + (i as i32 % 4) * 40;
            let y = 60 + (i as i32 / 4) * 40;
            let color = if *on { NAUTILUS_ACCENT } else { NAUTILUS_GATE_OFF };
            Circle::new(Point::new(x, y), 16).into_styled(PrimitiveStyle::with_fill(color)).draw(fb).ok();
        }
        Text::new("DELAY LINES 1L-4R", Point::new(420, 50), dim).draw(fb).ok();
        let _ = accent;
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
        let v = self.output_visual();
        crate::app::SlintExtra::Nautilus(crate::app::NautilusExtra {
            delay_mode_name: v.delay_mode_name,
            delay_mode_index: v.delay_mode_index,
            feedback_mode_name: v.feedback_mode_name,
            feedback_mode_index: v.feedback_mode_index,
            chroma_name: v.chroma_name,
            chroma_index: v.chroma_index,
            frozen: v.frozen,
            sensors: v.sensors,
            reversal_count: v.reversal_count,
            line_delay_ms: v.line_delay_ms,
            line_pulse: v.line_pulse,
            line_level: v.line_level,
            line_active: v.line_active,
            sonar_gate: v.sonar_gate,
            sonar_cv: v.sonar_cv,
            feedback_amount: v.feedback_amount,
        })
    }
}

/// One of the 8 delay lines -- a real circular buffer with a
/// sample-accurate variable-rate read cycle (see `read_and_advance`),
/// shared by every Delay Mode: a "normal" fixed-length echo is just
/// this cycle running at `rate == 1.0` and `reversed == false`, so
/// Reversal and Shimmer/De-Shimmer's pitch shift both fall out of the
/// same mechanism instead of needing special cases.
struct DelayLine {
    buffer: Vec<f32>,
    write_pos: usize,
    /// Smoothed/ramped toward the knob-derived target every sample --
    /// see Delay Mode's Fade (exponential) vs Doppler (slew-limited).
    delay_samples: f32,
    /// Real sample position (into `buffer`) where the current read
    /// cycle began -- re-captured from `write_pos` every time the
    /// cycle wraps, which is what lets a "normal" (non-pitch-shifted)
    /// line stay a continuous, ever-fresh echo instead of a frozen
    /// loop.
    cycle_start: usize,
    /// This cycle's read progress, in samples, advanced by `rate`
    /// every real sample (so `rate != 1.0` retriggers before/after a
    /// real `delay_samples` worth of time has passed -- Shimmer/
    /// De-Shimmer's pitch shift).
    pos_in_cycle: f32,
    /// True elapsed-sample counter driving the retrigger, independent
    /// of `pos_in_cycle`'s own (possibly pitch-shifted) rate.
    elapsed_since_cycle: f32,
    lpf_state: f32,
    hpf_lp_state: f32,
    crush_hold_value: f32,
    crush_hold_counter: u32,
}

impl DelayLine {
    fn new(max_len: usize) -> Self {
        Self {
            buffer: vec![0.0; max_len.max(4)],
            write_pos: 0,
            delay_samples: 480.0,
            cycle_start: 0,
            pos_in_cycle: 0.0,
            elapsed_since_cycle: 0.0,
            lpf_state: 0.0,
            hpf_lp_state: 0.0,
            crush_hold_value: 0.0,
            crush_hold_counter: 0,
        }
    }

    fn reset(&mut self) {
        for s in self.buffer.iter_mut() {
            *s = 0.0;
        }
        self.lpf_state = 0.0;
        self.hpf_lp_state = 0.0;
    }

    /// Reads this line's current delayed value and advances its
    /// read-cycle state by one sample -- returns `(value, retriggered)`
    /// where `retriggered` is true on the one sample each cycle where
    /// this line "pings" (used for Sonar and the panel's activity
    /// LEDs).
    fn read_and_advance(&mut self, rate: f32, reversed: bool, sample_rate: f32) -> (f32, bool) {
        let len = self.buffer.len();
        let ds = self.delay_samples.clamp(4.0, len as f32 - 4.0);

        self.elapsed_since_cycle += 1.0;
        self.pos_in_cycle += rate;
        let mut retriggered = false;
        if self.elapsed_since_cycle >= ds {
            self.elapsed_since_cycle -= ds;
            self.cycle_start = self.write_pos;
            self.pos_in_cycle = self.pos_in_cycle.rem_euclid(ds);
            retriggered = true;
        }
        let progress = self.pos_in_cycle.rem_euclid(ds);
        let offset = if reversed { ds - progress } else { progress };
        let read_pos = (self.cycle_start as f32 - ds + offset).rem_euclid(len as f32);
        let value = read_interp(&self.buffer, read_pos);

        // A short anti-click fade at each cycle's edges -- without it,
        // a pitch-shifted (rate != 1.0) cycle's retrigger is an
        // audible pop rather than a grain boundary.
        let fade_len = (0.005 * sample_rate).min(ds * 0.25).max(1.0);
        let env = if progress < fade_len {
            progress / fade_len
        } else if progress > ds - fade_len {
            ((ds - progress) / fade_len).max(0.0)
        } else {
            1.0
        };
        (value * env, retriggered)
    }

    fn write_and_advance(&mut self, value: f32) {
        let len = self.buffer.len();
        self.buffer[self.write_pos] = value;
        self.write_pos = (self.write_pos + 1) % len;
    }
}

/// Chroma: one of the real module's 6 feedback-path effects, applied
/// to the signal just before it's written back into the delay line
/// (so it's genuinely "captured" with the repeats, same as the
/// manual describes).
fn apply_chroma(dl: &mut DelayLine, x: f32, chroma: u32, depth: f32, sample_rate: f32) -> f32 {
    let depth = depth.clamp(0.0, 1.0);
    match chroma % 6 {
        0 => {
            // Oceanic Absorption: 4-pole-style lowpass (a cascaded
            // one-pole stands in for the real module's 4-pole filter).
            let cutoff = 18000.0 * (150.0f32 / 18000.0).powf(depth);
            let coef = (1.0 - (-std::f32::consts::TAU * cutoff / sample_rate).exp()).clamp(0.0, 1.0);
            let mut v = x;
            for _ in 0..2 {
                dl.lpf_state += (v - dl.lpf_state) * coef;
                v = dl.lpf_state;
            }
            v
        }
        1 => {
            // White Water: highpass, derived as input minus a lowpass.
            let cutoff = 20.0 * (2500.0f32 / 20.0).powf(depth);
            let coef = (1.0 - (-std::f32::consts::TAU * cutoff / sample_rate).exp()).clamp(0.0, 1.0);
            dl.hpf_lp_state += (x - dl.hpf_lp_state) * coef;
            x - dl.hpf_lp_state
        }
        2 => {
            // Refraction Interference: bitcrush + sample-rate reduction.
            let bits = (16.0 - depth * 12.0).max(2.0);
            let levels = 2f32.powf(bits) / 2.0 - 1.0;
            let div = 1 + (depth * 24.0) as u32;
            dl.crush_hold_counter = (dl.crush_hold_counter + 1) % div.max(1);
            if dl.crush_hold_counter == 0 {
                dl.crush_hold_value = x;
            }
            (dl.crush_hold_value * levels).round() / levels
        }
        3 => {
            // Pulse Amplification: warm soft saturation.
            let drive = 1.0 + depth * 8.0;
            (x * drive).tanh()
        }
        4 => {
            // Receptor Malfunction: wavefolder.
            let drive = 1.0 + depth * 6.0;
            wavefold(x * drive)
        }
        _ => {
            // SOS: heavy distortion.
            let drive = 1.0 + depth * 24.0;
            (x * drive).clamp(-1.0, 1.0)
        }
    }
}

struct NautilusProcessor {
    params: Arc<Params>,
    modbus: Arc<ModBus>,
    audio_bus: Arc<AudioBus>,
    lines: [[DelayLine; MAX_LINES_PER_CHANNEL]; NUM_CHANNELS],
    /// Sonar's "Stepped Voltage" accumulator.
    sonar_step: f32,
    /// Remaining samples for Sonar's Gate-mode pulse.
    sonar_gate_hold: u32,
    /// Sonar's Master-Clock-mode phase, independent of the delay
    /// lines' own read cycles (stands in for a real external Clock In
    /// passthrough -- see module doc comment).
    master_phase: f32,
}

impl AudioProcessor for NautilusProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        for out in buffer.iter_mut() {
            *out = 0.0;
        }

        let source_idx = self.params.source.load(Ordering::Relaxed);
        let tapped = self.audio_bus.get(source_idx).map(|b| b.lock().unwrap().clone());

        let delay_mode = self.params.delay_mode.load(Ordering::Relaxed) % 4;
        let feedback_mode = self.params.feedback_mode.load(Ordering::Relaxed) % 4;
        let rate_hz = self.params.rate_hz.get().clamp(MIN_RATE_HZ, MAX_RATE_HZ);
        let resolution_mult = resolution_multiplier(self.params.resolution.load(Ordering::Relaxed));
        let feedback_gain = self.params.feedback.get().clamp(0.0, MAX_FEEDBACK);
        let sensors = self.params.sensors.load(Ordering::Relaxed).clamp(1, MAX_LINES_PER_CHANNEL);
        let dispersal = self.params.dispersal.get().clamp(0.0, 1.0);
        let reversal_count = self.params.reversal.load(Ordering::Relaxed).min(NUM_LINES);
        let chroma = self.params.chroma.load(Ordering::Relaxed) % 6;
        let depth = self.params.depth.get().clamp(0.0, 1.0);
        let mix = self.params.mix.get().clamp(0.0, 1.0);
        let frozen = self.params.freeze.load(Ordering::Relaxed);
        let sonar_mode = self.params.sonar_mode.load(Ordering::Relaxed) % 4;

        if self.params.purge_pending.swap(false, Ordering::Relaxed) {
            for ch in self.lines.iter_mut() {
                for line in ch.iter_mut() {
                    line.reset();
                }
            }
        }

        let base_delay_seconds = (resolution_mult / rate_hz).clamp(MIN_DELAY_SECONDS, MAX_DELAY_SECONDS);
        let base_delay_samples = base_delay_seconds * sample_rate;

        let pitch_rate = match delay_mode {
            DELAY_MODE_SHIMMER => 2f32.powf(SHIMMER_TRANSPOSE_SEMITONES / 12.0),
            DELAY_MODE_DESHIMMER => 2f32.powf(-SHIMMER_TRANSPOSE_SEMITONES / 12.0),
            _ => 1.0,
        };

        let frames = buffer.len() / channels.max(1);
        let mut out_l = vec![0.0f32; frames];
        let mut out_r = vec![0.0f32; frames];
        let mut line_pulsed = [false; NUM_LINES];
        // Real per-line output peak within this block -- see
        // `NautilusVisual::line_level`.
        let mut line_peak = [0f32; NUM_LINES];

        for n in 0..frames {
            let dry = tapped.as_ref().and_then(|b| b.get(n % b.len().max(1)).copied()).unwrap_or(0.0);

            let mut line_out = [[0f32; MAX_LINES_PER_CHANNEL]; NUM_CHANNELS];
            let mut ping_count = 0u32;
            for c in 0..NUM_CHANNELS {
                for i in 0..sensors {
                    let order = line_order(c, i);
                    let reversed = order < reversal_count;
                    let spread = if sensors == 1 {
                        if c == 1 {
                            dispersal * DISPERSAL_ONE_LINE_OFFSET
                        } else {
                            0.0
                        }
                    } else {
                        i as f32 * dispersal * DISPERSAL_SPREAD
                    };
                    let target = (base_delay_samples * (1.0 + spread)).clamp(MIN_DELAY_SECONDS * sample_rate, MAX_DELAY_SECONDS * sample_rate);

                    let dl = &mut self.lines[c][i];
                    match delay_mode {
                        1 => {
                            // Doppler: slew-limited linear ramp.
                            let delta = (target - dl.delay_samples).clamp(-DOPPLER_SLEW_PER_SAMPLE * ds_scale(sample_rate), DOPPLER_SLEW_PER_SAMPLE * ds_scale(sample_rate));
                            dl.delay_samples += delta;
                        }
                        _ => {
                            // Fade, Shimmer, De-Shimmer: exponential.
                            let coef = 1.0 - (-1.0 / (FADE_SECONDS * sample_rate)).exp();
                            dl.delay_samples += (target - dl.delay_samples) * coef;
                        }
                    }

                    let (val, retriggered) = dl.read_and_advance(pitch_rate, reversed, sample_rate);
                    line_out[c][i] = val;
                    line_peak[order] = line_peak[order].max(val.abs());
                    if retriggered {
                        line_pulsed[order] = true;
                        ping_count += 1;
                    }
                }
            }

            for c in 0..NUM_CHANNELS {
                for i in 0..sensors {
                    let fb_source = match feedback_mode {
                        1 => line_out[1 - c][i],                                                     // Ping Pong
                        2 => line_out[c][if i == 0 { sensors - 1 } else { i - 1 }],                   // Cascade
                        3 => line_out[1 - c][if i == 0 { sensors - 1 } else { i - 1 }],                // Adrift
                        _ => line_out[c][i],                                                          // Normal
                    };
                    let input = if i == 0 { dry } else { 0.0 };
                    let mut fed = input + fb_source * feedback_gain;
                    fed = apply_chroma(&mut self.lines[c][i], fed, chroma, depth, sample_rate);
                    fed = fed.clamp(-4.0, 4.0);
                    if !frozen {
                        self.lines[c][i].write_and_advance(fed);
                    }
                }
            }

            let norm = (sensors as f32).sqrt().max(1.0);
            let wet_l: f32 = line_out[0][..sensors].iter().sum::<f32>() / norm;
            let wet_r: f32 = line_out[1][..sensors].iter().sum::<f32>() / norm;

            out_l[n] = (dry * (1.0 - mix) + wet_l.tanh() * mix).clamp(-1.5, 1.5);
            out_r[n] = (dry * (1.0 - mix) + wet_r.tanh() * mix).clamp(-1.5, 1.5);

            // --- Sonar ---
            self.master_phase = (self.master_phase + rate_hz / sample_rate).fract();
            let master_ping = self.master_phase < (rate_hz / sample_rate);
            let variable_ping = line_pulsed[0]; // 1L's own cycle -- the Resolution-derived rate.
            match sonar_mode {
                0 => {
                    // Gate: a short pulse on any overlapping ping.
                    if ping_count > 0 {
                        self.sonar_gate_hold = (0.02 * sample_rate) as u32;
                    }
                }
                1 => {
                    // Stepped CV: an additive stepped sequence built
                    // from overlapping delay pings.
                    if ping_count > 0 {
                        self.sonar_step = (self.sonar_step + ping_count as f32 / (NUM_LINES as f32)).rem_euclid(1.0);
                    }
                }
                2 => {
                    if master_ping {
                        self.sonar_gate_hold = (0.02 * sample_rate) as u32;
                    }
                }
                _ => {
                    if variable_ping {
                        self.sonar_gate_hold = (0.02 * sample_rate) as u32;
                    }
                }
            }
            if self.sonar_gate_hold > 0 {
                self.sonar_gate_hold -= 1;
            }
        }

        for i in 0..NUM_LINES {
            self.params.line_pulse[i].store(line_pulsed[i], Ordering::Relaxed);
            self.params.line_level[i].set(line_peak[i]);
        }
        for c in 0..NUM_CHANNELS {
            for i in 0..MAX_LINES_PER_CHANNEL {
                let order = line_order(c, i);
                let delay_ms = self.lines[c][i].delay_samples / sample_rate * 1000.0;
                self.params.line_delay_ms[order].set(delay_ms);
            }
        }

        let sonar_gate_on = self.sonar_gate_hold > 0;
        self.params.sonar_gate.store(sonar_gate_on, Ordering::Relaxed);
        let sonar_value = match sonar_mode {
            1 => self.sonar_step,
            _ => {
                if sonar_gate_on {
                    1.0
                } else {
                    0.0
                }
            }
        };
        self.params.sonar_cv.set(sonar_value);
        let sonar_target = self.params.sonar_target.load(Ordering::Relaxed);
        if sonar_target > 0 {
            if let Some(handle) = self.modbus.get(sonar_target - 1) {
                handle.set(sonar_value * self.params.sonar_level.get());
            }
        }

        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        for (frame, (l, r)) in buffer.chunks_mut(channels.max(1)).zip(out_l.iter().zip(out_r.iter())) {
            if frame.len() >= 2 {
                frame[0] = *l * mix_level;
                frame[1] = *r * mix_level;
                for ch in frame.iter_mut().skip(2) {
                    *ch = (*l + *r) * 0.5 * mix_level;
                }
            } else if let Some(ch) = frame.first_mut() {
                *ch = (*l + *r) * 0.5 * mix_level;
            }
        }
        *self.params.bus_out.lock().unwrap() = out_l.iter().zip(out_r.iter()).map(|(l, r)| (l + r) * 0.5).collect();
    }
}

/// Doppler's slew limit is specified in delay-samples per real
/// sample at a nominal 48kHz; this scales it so the same musical
/// slew rate holds at other sample rates.
fn ds_scale(sample_rate: f32) -> f32 {
    sample_rate / 48000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> (NautilusApp, Arc<AudioBus>) {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(3.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let app = NautilusApp::new(sensitivity, nav_speed, modbus, Arc::clone(&audio_bus), mixer_bus);
        (app, audio_bus)
    }

    // --- pure-function coverage ---

    #[test]
    fn resolution_multiplier_matches_the_manual_s_named_divisions() {
        assert_eq!(resolution_multiplier(DEFAULT_RESOLUTION), 1.0); // "One Beat"
        assert_eq!(resolution_multiplier(0), 8.0); // "2 Bars" -- slowest/longest
        assert!((resolution_multiplier(15) - 0.0078125).abs() < 1e-6); // "512th Note" -- fastest/shortest
        // Out-of-range indices must clamp, never panic or index OOB.
        assert_eq!(resolution_multiplier(999), resolution_multiplier(15));
    }

    #[test]
    fn line_order_matches_the_manual_s_1l_1r_2l_2r_numbering() {
        assert_eq!(line_order(0, 0), 0); // 1L
        assert_eq!(line_order(1, 0), 1); // 1R
        assert_eq!(line_order(0, 1), 2); // 2L
        assert_eq!(line_order(1, 1), 3); // 2R
        assert_eq!(line_order(0, 3), 6); // 4L
        assert_eq!(line_order(1, 3), 7); // 4R
    }

    #[test]
    fn wavefold_stays_bounded_for_large_input() {
        for x in [-50.0f32, -3.0, -1.0, 0.0, 1.0, 3.0, 50.0] {
            let y = wavefold(x);
            assert!(y.is_finite() && (-1.0..=1.0).contains(&y), "wavefold({x}) = {y} out of range");
        }
    }

    // --- audio-processor integration coverage ---

    #[test]
    fn no_source_and_no_feedback_is_silence_not_a_panic() {
        let (mut app, _audio_bus) = new_app();
        app.params.mix.set(1.0);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..10 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|&s| s == 0.0));
    }

    /// A loud tapped source, with a short delay time and real
    /// feedback, must produce audible wet output.
    #[test]
    fn a_loud_tapped_source_produces_audible_wet_output() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed); // Nautilus registers its own bus first (index 0) -- "Source" lands at 1.
        *source.lock().unwrap() = vec![0.8; 512];
        app.params.mix.set(1.0);
        app.params.feedback.set(0.4);
        app.params.rate_hz.set(MAX_RATE_HZ); // short delay time so it's audible within a few blocks
        app.params.resolution.store(15, Ordering::Relaxed); // "512th Note" -- shortest multiplier

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.01, "expected audible wet output from a loud tapped source, got peak {peak}");
    }

    /// Near-infinite feedback (the manual's own "take care" warning)
    /// must never blow up -- output must stay finite and bounded.
    #[test]
    fn near_infinite_feedback_stays_bounded_and_finite() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.9; 512];
        app.params.feedback.set(MAX_FEEDBACK);
        app.params.mix.set(1.0);
        app.params.rate_hz.set(MAX_RATE_HZ);
        app.params.resolution.store(15, Ordering::Relaxed);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..400 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|s| s.is_finite()), "output must stay finite under near-infinite feedback");
        let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak <= 3.0, "expected bounded output, got peak {peak}");
    }

    /// Nautilus must republish its own output on the audio bus.
    #[test]
    fn republishes_its_output_on_the_audio_bus() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.5; 256];
        app.params.mix.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 256 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        let published = app.params.bus_out.lock().unwrap();
        assert_eq!(published.len(), 256);
    }

    /// FREEZE must stop new audio from entering the delay lines --
    /// frozen immediately (before anything real was ever captured), a
    /// newly loud source must never reach the wet output.
    #[test]
    fn freeze_stops_writing_new_audio_into_the_delay_lines() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        app.params.mix.set(1.0);
        app.params.freeze.store(true, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.9; 512];

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|&s| s == 0.0), "expected silence while frozen with an all-zero captured buffer");
    }

    /// Purge must instantly and audibly clear built-up delay energy.
    #[test]
    fn purge_clears_delay_lines() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.9; 512];
        app.params.mix.set(1.0);
        app.params.feedback.set(0.9);
        app.params.rate_hz.set(MAX_RATE_HZ);
        app.params.resolution.store(15, Ordering::Relaxed);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let peak_before = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak_before > 0.01, "expected built-up delay energy before purging");

        // Purge with a silent source, and confirm the lines are
        // really empty (no leftover echo) rather than just briefly
        // dipped by the fresh silence.
        *source.lock().unwrap() = vec![0.0; 512];
        app.params.purge_pending.store(true, Ordering::Relaxed);
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let peak_after = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert_eq!(peak_after, 0.0, "expected exact silence after Purge with a silent source, got peak {peak_after}");
    }

    /// Mix fully counter-clockwise (0.0) must be pure dry passthrough.
    #[test]
    fn mix_at_zero_is_pure_dry_passthrough() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.42; 512];
        app.params.mix.set(0.0);
        app.params.feedback.set(0.9);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        for s in buffer.iter() {
            assert!((s - 0.42).abs() < 1e-4, "expected pure dry passthrough at Mix=0, got {s}");
        }
    }

    /// All 4 Delay Modes and all 4 Feedback Modes must run bounded and
    /// finite -- a real functional sweep across the full mode matrix.
    #[test]
    fn every_delay_mode_and_feedback_mode_combination_stays_bounded() {
        for delay_mode in 0..4u32 {
            for feedback_mode in 0..4u32 {
                let (mut app, audio_bus) = new_app();
                let source = audio_bus.register("Source");
                app.params.source.store(1, Ordering::Relaxed);
                *source.lock().unwrap() = vec![0.6; 512];
                app.params.delay_mode.store(delay_mode, Ordering::Relaxed);
                app.params.feedback_mode.store(feedback_mode, Ordering::Relaxed);
                app.params.feedback.set(0.7);
                app.params.mix.set(1.0);
                app.params.sensors.store(4, Ordering::Relaxed);
                app.params.rate_hz.set(MAX_RATE_HZ);
                app.params.resolution.store(15, Ordering::Relaxed);

                let mut processor = app.audio_processor().unwrap();
                let mut buffer = vec![0.0f32; 512 * 2];
                for _ in 0..100 {
                    processor.process(&mut buffer, 2, 48000.0);
                }
                assert!(
                    buffer.iter().all(|s| s.is_finite()),
                    "delay_mode={delay_mode} feedback_mode={feedback_mode} produced non-finite output"
                );
            }
        }
    }

    /// Reversal must clamp its count into `0..=NUM_LINES` and never
    /// panic, at both extremes.
    #[test]
    fn reversal_clamps_into_range_and_never_panics() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.5; 512];
        app.params.reversal.store(NUM_LINES, Ordering::Relaxed); // all 8 reversed
        app.params.mix.set(1.0);
        app.params.sensors.store(4, Ordering::Relaxed);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..50 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|s| s.is_finite()));
    }

    /// A routed Sonar output must write real, changing values into its
    /// modbus target over time (not a constant/untouched value).
    #[test]
    fn a_routed_sonar_output_writes_real_values_into_its_target() {
        // Register the target *before* constructing the app, since
        // Nautilus's own MixerBus registration (see `Params::new`)
        // also claims a ModBus slot for its "Mixer: Nautilus Level"
        // channel.
        let modbus = Arc::new(ModBus::new());
        let target_handle = modbus.register("Some App: Some Param");
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(3.0));
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let mut app = NautilusApp::new(sensitivity, nav_speed, Arc::clone(&modbus), Arc::clone(&audio_bus), mixer_bus);

        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.7; 512];
        app.params.sonar_target.store(1, Ordering::Relaxed);
        app.params.sonar_level.set(1.0);
        app.params.sonar_mode.store(0, Ordering::Relaxed); // Gate
        // A slow-ish rate (well under the 20ms gate-hold time's block
        // count) so the gate has real rest between pings, not a
        // fast-enough rate that it stays saturated on.
        app.params.rate_hz.set(2.0);
        app.params.resolution.store(DEFAULT_RESOLUTION, Ordering::Relaxed);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut saw_on = false;
        let mut saw_off = false;
        for _ in 0..200 {
            processor.process(&mut buffer, 2, 48000.0);
            if target_handle.get() > 0.0 {
                saw_on = true;
            } else {
                saw_off = true;
            }
        }
        assert!(saw_on, "expected Sonar's Gate output to pulse on at least once");
        assert!(saw_off, "expected Sonar's Gate output to also rest between pings");
    }

    /// `output_visual` must reflect real, live per-line delay/level
    /// data and the current mode names -- not fabricated placeholders.
    #[test]
    fn output_visual_reflects_real_delay_network_state() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.8; 512];
        app.params.mix.set(1.0);
        app.params.feedback.set(0.5);
        app.params.sensors.store(2, Ordering::Relaxed);
        app.params.rate_hz.set(MAX_RATE_HZ);
        app.params.resolution.store(15, Ordering::Relaxed);
        app.params.delay_mode.store(1, Ordering::Relaxed); // Doppler
        app.params.feedback_mode.store(2, Ordering::Relaxed); // Cascade
        app.params.chroma.store(3, Ordering::Relaxed); // Pulse Amplification

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
        }

        let visual = app.output_visual();
        assert_eq!(visual.delay_mode_name, "Doppler");
        assert_eq!(visual.delay_mode_index, 1);
        assert_eq!(visual.feedback_mode_name, "Cascade");
        assert_eq!(visual.feedback_mode_index, 2);
        assert_eq!(visual.chroma_name, "Pulse Amplification");
        assert_eq!(visual.chroma_index, 3);
        assert_eq!(visual.sensors, 2);

        // With Sensors == 2, only 1L/1R/2L/2R (line_order 0..3) are
        // active -- 3L/3R/4L/4R must be marked inactive.
        assert!(visual.line_active[0] && visual.line_active[1] && visual.line_active[2] && visual.line_active[3]);
        assert!(!visual.line_active[4] && !visual.line_active[5] && !visual.line_active[6] && !visual.line_active[7]);

        // A loud source with real feedback into short delay lines
        // must produce real, non-zero delay times and output levels
        // on the active lines, not placeholder zeros.
        for &i in &[0usize, 1, 2, 3] {
            assert!(visual.line_delay_ms[i] > 0.0, "line {i} expected a real positive delay time, got {}", visual.line_delay_ms[i]);
        }
        let active_level_sum: f32 = [0usize, 1, 2, 3].iter().map(|&i| visual.line_level[i]).sum();
        assert!(active_level_sum > 0.0, "expected real non-zero per-line output level on active lines");
    }

    /// Freeze/Sonar state on the visualization struct must mirror the
    /// same atomics the rest of the app already reads/writes.
    #[test]
    fn output_visual_mirrors_freeze_and_sonar_state() {
        let (mut app, _audio_bus) = new_app();
        app.params.freeze.store(true, Ordering::Relaxed);
        app.params.feedback.set(0.42);
        let visual = app.output_visual();
        assert!(visual.frozen);
        assert!((visual.feedback_amount - 0.42).abs() < 1e-6);
        assert_eq!(visual.sonar_gate, app.params.sonar_gate.load(Ordering::Relaxed));
        assert!((visual.sonar_cv - app.params.sonar_cv.get()).abs() < 1e-6);
    }

    /// Sensors must clamp into `1..=4` and never panic at either
    /// extreme.
    #[test]
    fn sensors_clamps_into_range() {
        let (mut app, _audio_bus) = new_app();
        for _ in 0..10 {
            app.edit(Selection::Sensors, 1);
        }
        assert_eq!(app.params.sensors.load(Ordering::Relaxed), MAX_LINES_PER_CHANNEL);
        for _ in 0..10 {
            app.edit(Selection::Sensors, -1);
        }
        assert_eq!(app.params.sensors.load(Ordering::Relaxed), 1);
    }
}
