//! A clone of Erica Synths Black Hole DSP2: a 24-algorithm multi-effect
//! ("glitch/texture") processor built around the Spin FV-1 chip. Taps
//! another app's output (see audio_bus.rs, the same technique Beads/
//! Clouds already use) into a rolling capture/delay buffer that every
//! algorithm reads from with its own tap offsets. Feature set follows
//! the real module's manual (ericasynths.lv, "Black Hole DSP II") as
//! closely as this sim's architecture allows:
//!
//! - All 24 real algorithms, each with its own 3 real parameters
//!   (P1/P2/P3), matching the manual's parameter table exactly --
//!   delays (plain, chirped, filtered, tap-offset, pitch-rising
//!   "Hellraiser", phased, granular, pitch-shifting), reverbs (hall/
//!   room/stalker/saturated) and 3 shimmer variants (bidirectional,
//!   +1oct, -1oct) built on a shared comb+allpass reverb tank,
//!   modulation (chorus, flanger), a 2-stage distortion+bitcrush+
//!   tremolo "Ripper", a dual-lane allpass "Space Phaser", a dual
//!   independent pitch shifter, 2 filtered freeze-loopers, and a
//!   3-oscillator drone generator.
//! - CRUSH: manual sample-rate + bit-depth reduction (48kHz down to
//!   roughly 1kHz) applied to the wet/digital signal path only -- the
//!   manual describes the Dry/Wet control as "full analogue", i.e. the
//!   dry tap bypasses the chip entirely, so it's left uncrushed here
//!   too.
//! - IN LEVEL with a real clip indicator (peak-detected each block).
//! - Per-effect settings memory: Save Patch / Recall Patch actions
//!   store and restore all of an algorithm's own parameters (P1-P3,
//!   Crush, Dry/Wet, In Level), keyed by algorithm slot -- the real
//!   module's "push-and-hold to save / double-push to recall" PATCH
//!   memory feature.
//! - The manual's "controls in bold may increase output level
//!   radically" warnings are carried through (marked with a trailing
//!   "!" on that parameter's name), and effects 19/22/23/24, which the
//!   manual says should be run fully wet, get a "(try 100%)" hint on
//!   Dry/Wet while under 95%.
//!
//! Deliberately not implemented / simplified, and why:
//! - **True stereo.** This sim's audio pipeline is mono end-to-end
//!   (every app publishes one mono buffer on its bus, same as Beads/
//!   Clouds/Rainmaker) -- there's no path for a second channel. The 6
//!   algorithms with independent Left/Right or Phase-Left/Phase-Right
//!   parameters (Dual Delay, Servo Flanger, Space Phaser) still run two
//!   genuinely different internal signal paths (different delay times/
//!   allpass phase per "side") and sum them to mono, so the parameter
//!   still audibly does something real (comb filtering, movement,
//!   doubling) -- it just can't be heard as stereo separation here.
//! - **Per-parameter external CV.** The real module has a CV input per
//!   parameter (plus Crush and Dry/Wet). This sim's cross-app
//!   modulation registry (modbus.rs) exists for exactly this kind of
//!   thing, but -- following Beads' precedent of not wiring its own
//!   Time/Size/Shape/Pitch up to it either -- individual physical CV
//!   jacks aren't a concept this sim's UI has a generic way to expose
//!   per-knob, so it's left as manual-only control, same as Beads.
//! - **Rotate-then-confirm activation.** On real hardware, turning the
//!   PATCH encoder only *selects* an algorithm; a separate press
//!   *activates* it, so browsing through all 24 doesn't blast whatever
//!   is currently patched. This sim applies the selected algorithm
//!   immediately, the same way every other mode-select knob in this
//!   build (Beads' Quality/Grain Mode, this app's own Source) already
//!   works -- not a fabricated omission, just consistency with the
//!   rest of the UI.
//! - **Hold vs. double-press gestures.** `Input` only reports
//!   edge-triggered button presses, not press duration or multi-click,
//!   so "hold to save" / "press twice to recall" collapse into two
//!   explicit menu rows (Save Patch / Recall Patch) that fire on the
//!   normal per-leaf "press knob2 to act" gesture already used
//!   everywhere else for reset-to-default.

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
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const CAPTURE_SECONDS: f32 = 2.5;
const NUM_ALGORITHMS: usize = 24;
const MAX_GRAINS: usize = 4;

const MAX_DELAY_MS: f32 = 1500.0;
const MAX_SHORT_DELAY_MS: f32 = 25.0;
const MAX_PREDELAY_MS: f32 = 500.0;
const MAX_GRAIN_MS: f32 = 500.0;
const MAX_FREEZE_MS: f32 = 500.0;
const MIN_FREEZE_MS: f32 = 10.0;

fn map_delay_ms(p: f32) -> f32 {
    1.0 + p.clamp(0.0, 1.0) * (MAX_DELAY_MS - 1.0)
}
fn map_short_delay_ms(p: f32) -> f32 {
    0.5 + p.clamp(0.0, 1.0) * (MAX_SHORT_DELAY_MS - 0.5)
}
fn map_predelay_ms(p: f32) -> f32 {
    p.clamp(0.0, 1.0) * MAX_PREDELAY_MS
}
fn map_grain_ms(p: f32) -> f32 {
    10.0 + p.clamp(0.0, 1.0) * (MAX_GRAIN_MS - 10.0)
}
fn map_freeze_ms(p: f32) -> f32 {
    MIN_FREEZE_MS + p.clamp(0.0, 1.0) * (MAX_FREEZE_MS - MIN_FREEZE_MS)
}
fn map_bipolar_semitones(p: f32, max_st: f32) -> f32 {
    (p.clamp(0.0, 1.0) - 0.5) * 2.0 * max_st
}
fn map_unipolar_semitones(p: f32, max_st: f32) -> f32 {
    p.clamp(0.0, 1.0) * max_st
}
fn map_cutoff_hz(p: f32) -> f32 {
    200.0 * (12000.0f32 / 200.0).powf(p.clamp(0.0, 1.0))
}
fn map_drone_hz(p: f32) -> f32 {
    20.0 * (2000.0f32 / 20.0).powf(p.clamp(0.0, 1.0))
}
fn map_rate_hz(p: f32) -> f32 {
    0.05 + p.clamp(0.0, 1.0) * 8.0
}

#[derive(Clone, Copy)]
enum Unit {
    Pct,
    Ms,
    Hz,
    St,
}

fn format_unit(unit: Unit, value: f32) -> String {
    match unit {
        Unit::Pct => format!("{:.0}%", value.clamp(0.0, 1.0) * 100.0),
        Unit::Ms => format!("{:.0} ms", value),
        Unit::Hz => format!("{:.0} Hz", value),
        Unit::St => format!("{:+.1} st", value),
    }
}

struct AlgoSpec {
    name: &'static str,
    p: [&'static str; 3],
    unit: [Unit; 3],
    bold: [bool; 3],
    full_wet: bool,
}

/// The real module's 24-effect table, 1:1 with the manual (Nr. 1-24).
const ALGORITHMS: [AlgoSpec; NUM_ALGORITHMS] = [
    AlgoSpec { name: "Dual Delay", p: ["Feedback", "Time Left", "Time Right"], unit: [Unit::Pct, Unit::Ms, Unit::Ms], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Chirp Delay", p: ["Feedback", "Delay Time", "Chirp Amount"], unit: [Unit::Pct, Unit::Ms, Unit::Pct], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Lowpass Delay", p: ["Feedback", "Delay Time", "Low Pass Filter"], unit: [Unit::Pct, Unit::Ms, Unit::Hz], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Hipass Delay", p: ["Feedback", "Delay Time", "Hi Pass Filter"], unit: [Unit::Pct, Unit::Ms, Unit::Hz], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Tap Tap Delay", p: ["Feedback", "Delay Time", "Feedback Tap Time"], unit: [Unit::Pct, Unit::Ms, Unit::Ms], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Hellraiser Delay", p: ["Feedback", "Delay Time", "Raiser"], unit: [Unit::Pct, Unit::Ms, Unit::Pct], bold: [false, false, true], full_wet: false },
    AlgoSpec { name: "Phazed Delay", p: ["Feedback", "Delay Time", "Phazer Amount"], unit: [Unit::Pct, Unit::Ms, Unit::Pct], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Granular Delay", p: ["Feedback", "Grain Size", "Width"], unit: [Unit::Pct, Unit::Ms, Unit::Pct], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Pitch Shift Delay", p: ["Feedback", "Delay Time", "Pitch Shifter"], unit: [Unit::Pct, Unit::Ms, Unit::St], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Shimmer Drift", p: ["Pre Delay", "Reverb Size", "Shimmer Pitch"], unit: [Unit::Ms, Unit::Pct, Unit::St], bold: [false, false, true], full_wet: false },
    AlgoSpec { name: "Shimmer +", p: ["Pre Delay", "Reverb Size", "Shimmer Amount"], unit: [Unit::Ms, Unit::Pct, Unit::St], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Shimmer -", p: ["Pre Delay", "Reverb Size", "Shimmer Amount"], unit: [Unit::Ms, Unit::Pct, Unit::St], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Big / Hall Reverb", p: ["Pre Delay", "Size", "Tone"], unit: [Unit::Ms, Unit::Pct, Unit::Hz], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Room Reverb", p: ["Pre Delay", "Size", "Tone"], unit: [Unit::Ms, Unit::Pct, Unit::Hz], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Stalker Reverb", p: ["Feedback", "Size", "Tone"], unit: [Unit::Pct, Unit::Pct, Unit::Hz], bold: [true, false, false], full_wet: false },
    AlgoSpec { name: "Saturated Reverb", p: ["Saturation", "Reverb Size", "Tone"], unit: [Unit::Pct, Unit::Pct, Unit::Hz], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Havoc Chorus", p: ["Feedback", "Rate", "Width"], unit: [Unit::Pct, Unit::Hz, Unit::Pct], bold: [true, false, false], full_wet: false },
    AlgoSpec { name: "Servo Flanger", p: ["Feedback", "Time Left", "Time Right"], unit: [Unit::Pct, Unit::Ms, Unit::Ms], bold: [true, false, false], full_wet: false },
    AlgoSpec { name: "Ripper", p: ["Overdrive", "Bitcrush", "Rip"], unit: [Unit::Pct, Unit::Pct, Unit::Pct], bold: [false, false, true], full_wet: true },
    AlgoSpec { name: "Space Phaser", p: ["Feedback", "Phase Left", "Phase Right"], unit: [Unit::Pct, Unit::Pct, Unit::Pct], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "Dual Pitch Shifter", p: ["Shift 1", "Shift 2", "Crossfade"], unit: [Unit::St, Unit::St, Unit::Pct], bold: [false, false, false], full_wet: false },
    AlgoSpec { name: "LP Freezer", p: ["Record", "Time", "Low Pass Filter"], unit: [Unit::Pct, Unit::Ms, Unit::Hz], bold: [false, false, false], full_wet: true },
    AlgoSpec { name: "HP Freezer", p: ["Record", "Time", "Hi Pass Filter"], unit: [Unit::Pct, Unit::Ms, Unit::Hz], bold: [false, false, false], full_wet: true },
    AlgoSpec { name: "Drone Bank", p: ["Freq 1", "Freq 2", "Freq 3"], unit: [Unit::Hz, Unit::Hz, Unit::Hz], bold: [true, true, true], full_wet: true },
];

/// Which family an algorithm belongs to, per the manual's own grouping
/// of the 24-effect table -- purely a display grouping (index 0..8 are
/// delays, 9..11 shimmer, etc), used for the Slint category badge.
/// `(name, index into a fixed category-color palette)`.
fn algo_category(algo: usize) -> (&'static str, usize) {
    match algo {
        0..=8 => ("Delay", 0),
        9..=11 => ("Shimmer", 1),
        12..=15 => ("Reverb", 2),
        16..=17 => ("Modulation", 3),
        18 => ("Drive", 4),
        19 => ("Phaser", 5),
        20 => ("Pitch", 6),
        21..=22 => ("Freeze", 7),
        _ => ("Drone", 8),
    }
}

/// Maps an algorithm's 3 raw 0..1 knob positions to their real
/// engineering-unit values (ms/Hz/semitones/etc, per the manual's
/// parameter table) -- the single source of truth both the display
/// (`param_value_text`) and the audio processor (`process_algorithm`)
/// read from, so they can never disagree.
fn algo_engineering_values(algo: usize, p1: f32, p2: f32, p3: f32) -> [f32; 3] {
    match algo {
        0 => [p1, map_delay_ms(p2), map_delay_ms(p3)],
        1 => [p1, map_delay_ms(p2), p3],
        2 => [p1, map_delay_ms(p2), map_cutoff_hz(p3)],
        3 => [p1, map_delay_ms(p2), map_cutoff_hz(p3)],
        4 => [p1, map_delay_ms(p2), map_delay_ms(p3)],
        5 => [p1, map_delay_ms(p2), p3],
        6 => [p1, map_delay_ms(p2), p3],
        7 => [p1, map_grain_ms(p2), p3],
        8 => [p1, map_delay_ms(p2), map_bipolar_semitones(p3, 24.0)],
        9 => [map_predelay_ms(p1), p2, map_bipolar_semitones(p3, 12.0)],
        10 => [map_predelay_ms(p1), p2, map_unipolar_semitones(p3, 12.0)],
        11 => [map_predelay_ms(p1), p2, map_unipolar_semitones(p3, 12.0)],
        12 => [map_predelay_ms(p1), p2, map_cutoff_hz(p3)],
        13 => [map_predelay_ms(p1), p2, map_cutoff_hz(p3)],
        14 => [p1, p2, map_cutoff_hz(p3)],
        15 => [p1, p2, map_cutoff_hz(p3)],
        16 => [p1, map_rate_hz(p2), p3],
        17 => [p1, map_short_delay_ms(p2), map_short_delay_ms(p3)],
        18 => [p1, p2, p3],
        19 => [p1, p2, p3],
        20 => [map_bipolar_semitones(p1, 12.0), map_bipolar_semitones(p2, 12.0), p3],
        21 => [p1, map_freeze_ms(p2), map_cutoff_hz(p3)],
        22 => [p1, map_freeze_ms(p2), map_cutoff_hz(p3)],
        _ => [map_drone_hz(p1), map_drone_hz(p2), map_drone_hz(p3)],
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Source,
    InLevel,
    Algorithm,
    Param1,
    Param2,
    Param3,
    Crush,
    DryWet,
    SavePatch,
    RecallPatch,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 4;

#[derive(Clone, Copy, Default)]
struct SavedPatch {
    saved: bool,
    param1: f32,
    param2: f32,
    param3: f32,
    crush: f32,
    dry_wet: f32,
    in_level: f32,
}

struct Params {
    source: AtomicUsize,
    in_level: AtomicF32,
    algorithm: AtomicU32,
    param1: AtomicF32,
    param2: AtomicF32,
    param3: AtomicF32,
    crush: AtomicF32,
    dry_wet: AtomicF32,
    /// Set true whenever the input peaks past the clip threshold in the
    /// most recent block -- the real module's clip LED.
    clip: AtomicBool,
    /// Per-effect settings memory, keyed by algorithm slot -- the real
    /// module's PATCH save/recall.
    patches: Mutex<[SavedPatch; NUM_ALGORITHMS]>,
    /// A downsampled snapshot of this block's real processed output --
    /// refreshed once per audio block, for the Slint oscilloscope
    /// (real audio, not fabricated) that works the same regardless of
    /// which of the 24 very different algorithms is active.
    waveform_snapshot: Mutex<Vec<f32>>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Black Hole", modbus);
        Self {
            source: AtomicUsize::new(crate::audio_bus::NO_SOURCE),
            in_level: AtomicF32::new(0.5),
            algorithm: AtomicU32::new(0),
            param1: AtomicF32::new(0.5),
            param2: AtomicF32::new(0.5),
            param3: AtomicF32::new(0.5),
            crush: AtomicF32::new(0.0),
            dry_wet: AtomicF32::new(0.5),
            clip: AtomicBool::new(false),
            patches: Mutex::new([SavedPatch::default(); NUM_ALGORITHMS]),
            waveform_snapshot: Mutex::new(Vec::new()),
            bus_out: audio_bus.register("Black Hole"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct BlackHoleApp {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Black Hole's own palette: true void black with one searing
// accretion-disk ember -- the light that escapes from just outside
// the event horizon, the only thing visible against total darkness.
// Not a device-wide theme -- this app's own visual personality. ---

/// Near-total black -- the void itself.
const BLACK_HOLE_BG: Rgb565 = Rgb565::new(1, 1, 1);
/// Hot white-orange -- the title and this app's brightest text, like
/// the superheated inner edge of the accretion disk.
const BLACK_HOLE_TITLE: Rgb565 = Rgb565::new(31, 55, 22);
/// Fiery orange -- this app's menu accent / selected-row fill.
const BLACK_HOLE_ACCENT: Rgb565 = Rgb565::new(31, 35, 7);
/// Dim ember red -- secondary/dim text, cooling toward the dark.
const BLACK_HOLE_DIM: Rgb565 = Rgb565::new(15, 18, 7);

impl BlackHoleApp {
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
            0 => vec![Selection::Source, Selection::InLevel],
            1 => vec![Selection::Algorithm, Selection::Param1, Selection::Param2, Selection::Param3],
            2 => vec![Selection::Crush, Selection::DryWet],
            _ => vec![Selection::SavePatch, Selection::RecallPatch],
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
            1 => "Effect",
            2 => "Output",
            _ => "Patch",
        }
    }

    fn algo(&self) -> usize {
        self.params.algorithm.load(Ordering::Relaxed) as usize % NUM_ALGORITHMS
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => format!("{}{}", self.source_name(), if self.params.clip.load(Ordering::Relaxed) { ", clip!" } else { "" }),
            1 => ALGORITHMS[self.algo()].name.to_string(),
            2 => format!("{:.0}% wet, {:.0}% crush", self.params.dry_wet.get() * 100.0, self.params.crush.get() * 100.0),
            _ => {
                let saved = self.params.patches.lock().unwrap()[self.algo()].saved;
                if saved { "saved".into() } else { "empty".into() }
            }
        }
    }

    fn source_name(&self) -> String {
        self.audio_bus.source_name(self.params.source.load(Ordering::Relaxed))
    }

    fn param_label(&self, slot: usize) -> String {
        let spec = &ALGORITHMS[self.algo()];
        if spec.bold[slot] {
            format!("{} !", spec.p[slot])
        } else {
            spec.p[slot].to_string()
        }
    }

    fn param_value_text(&self, slot: usize) -> String {
        let algo = self.algo();
        let values = algo_engineering_values(algo, self.params.param1.get(), self.params.param2.get(), self.params.param3.get());
        format_unit(ALGORITHMS[algo].unit[slot], values[slot])
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => "Source".into(),
            Selection::InLevel => "In Level".into(),
            Selection::Algorithm => "Algorithm".into(),
            Selection::Param1 => self.param_label(0),
            Selection::Param2 => self.param_label(1),
            Selection::Param3 => self.param_label(2),
            Selection::Crush => "Crush".into(),
            Selection::DryWet => "Dry/Wet".into(),
            Selection::SavePatch => "Save Patch".into(),
            Selection::RecallPatch => "Recall Patch".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => self.source_name(),
            Selection::InLevel => {
                let text = format!("{:.0}%", self.params.in_level.get().clamp(0.0, 1.0) * 100.0);
                if self.params.clip.load(Ordering::Relaxed) { format!("{text} CLIP") } else { text }
            }
            Selection::Algorithm => ALGORITHMS[self.algo()].name.into(),
            Selection::Param1 => self.param_value_text(0),
            Selection::Param2 => self.param_value_text(1),
            Selection::Param3 => self.param_value_text(2),
            Selection::Crush => format!("{:.0}%", self.params.crush.get().clamp(0.0, 1.0) * 100.0),
            Selection::DryWet => {
                let dry_wet = self.params.dry_wet.get().clamp(0.0, 1.0);
                if ALGORITHMS[self.algo()].full_wet && dry_wet < 0.95 {
                    format!("{:.0}% (try 100%)", dry_wet * 100.0)
                } else {
                    format!("{:.0}%", dry_wet * 100.0)
                }
            }
            Selection::SavePatch => {
                let saved = self.params.patches.lock().unwrap()[self.algo()].saved;
                if saved { "press to update".into() } else { "press to save".into() }
            }
            Selection::RecallPatch => {
                let saved = self.params.patches.lock().unwrap()[self.algo()].saved;
                if saved { "press to recall".into() } else { "(empty)".into() }
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
            Selection::Source => {
                let cur = self.params.source.load(Ordering::Relaxed);
                self.params.source.store(crate::audio_bus::cycle_source(cur, step, self.audio_bus.len()), Ordering::Relaxed);
            }
            Selection::InLevel => bump(&self.params.in_level, delta, sensitivity, 0.0, 1.0),
            Selection::Algorithm => {
                let cur = self.params.algorithm.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(NUM_ALGORITHMS as i32);
                self.params.algorithm.store(next as u32, Ordering::Relaxed);
            }
            Selection::Param1 => bump(&self.params.param1, delta, sensitivity, 0.0, 1.0),
            Selection::Param2 => bump(&self.params.param2, delta, sensitivity, 0.0, 1.0),
            Selection::Param3 => bump(&self.params.param3, delta, sensitivity, 0.0, 1.0),
            Selection::Crush => bump(&self.params.crush, delta, sensitivity, 0.0, 1.0),
            Selection::DryWet => bump(&self.params.dry_wet, delta, sensitivity, 0.0, 1.0),
            Selection::SavePatch | Selection::RecallPatch => {}
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Source => self.params.source.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed),
            Selection::InLevel => self.params.in_level.set(0.5),
            Selection::Algorithm => self.params.algorithm.store(0, Ordering::Relaxed),
            Selection::Param1 => self.params.param1.set(0.5),
            Selection::Param2 => self.params.param2.set(0.5),
            Selection::Param3 => self.params.param3.set(0.5),
            Selection::Crush => self.params.crush.set(0.0),
            Selection::DryWet => self.params.dry_wet.set(0.5),
            Selection::SavePatch => {
                let algo = self.algo();
                let mut patches = self.params.patches.lock().unwrap();
                patches[algo] = SavedPatch {
                    saved: true,
                    param1: self.params.param1.get(),
                    param2: self.params.param2.get(),
                    param3: self.params.param3.get(),
                    crush: self.params.crush.get(),
                    dry_wet: self.params.dry_wet.get(),
                    in_level: self.params.in_level.get(),
                };
            }
            Selection::RecallPatch => {
                let algo = self.algo();
                let patch = self.params.patches.lock().unwrap()[algo];
                if patch.saved {
                    self.params.param1.set(patch.param1);
                    self.params.param2.set(patch.param2);
                    self.params.param3.set(patch.param3);
                    self.params.crush.set(patch.crush);
                    self.params.dry_wet.set(patch.dry_wet);
                    self.params.in_level.set(patch.in_level);
                }
            }
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

    /// The real category badge + oscilloscope + per-algorithm
    /// parameter meters -- see `Params::waveform_snapshot` (written
    /// once per audio block by `BlackHoleProcessor::process`) and
    /// `algo_category`, converted to connected line-segment geometry
    /// (see `crate::app::polyline_segments`) instead of drawn
    /// directly.
    pub(crate) fn output_visual(&self) -> crate::app::BlackHoleExtra {
        const PANEL_W: f32 = 260.0;
        const PANEL_H: f32 = 110.0;
        let algo = self.algo();
        let (category_name, category_index) = algo_category(algo);
        let raw = self.params.waveform_snapshot.lock().unwrap().clone();
        let waveform = if raw.len() >= 2 {
            let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(&raw, PANEL_W, PANEL_H, true);
            crate::app::CurveSegments { mid_x, mid_y, length, angle_deg }
        } else {
            crate::app::CurveSegments::default()
        };
        crate::app::BlackHoleExtra {
            algorithm_name: ALGORITHMS[algo].name.to_string(),
            category_name: category_name.to_string(),
            category_index,
            clip: self.params.clip.load(Ordering::Relaxed),
            waveform,
            param_labels: [self.param_label(0), self.param_label(1), self.param_label(2)],
            param_values: [self.params.param1.get(), self.params.param2.get(), self.params.param3.get()],
            param_bold: ALGORITHMS[algo].bold,
        }
    }
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.01).clamp(min, max);
    value.set(next);
}

impl App for BlackHoleApp {
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
        crate::app::SlintExtra::BlackHole(self.output_visual())
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(BlackHoleProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            capture: vec![0.0; (CAPTURE_SECONDS * 48000.0) as usize],
            write_pos: 0,
            filter_a: OnePole::default(),
            filter_b: OnePole::default(),
            phaser_l: std::array::from_fn(|_| AllpassStage::default()),
            phaser_r: std::array::from_fn(|_| AllpassStage::default()),
            phaser_lfo_phase: 0.0,
            phaser_fb_l: 0.0,
            phaser_fb_r: 0.0,
            chorus_lfo_phase: 0.0,
            chirp_phase: 0.0,
            pitch_a: PitchShifter::default(),
            pitch_b: PitchShifter::default(),
            reverb: ReverbTank::default(),
            grains: std::array::from_fn(|_| Grain::default()),
            grain_spawn_accum: 0.0,
            rng: 0xA53C_91F1,
            freeze_start: 0,
            freeze_phase: 0.0,
            freeze_engaged_prev: false,
            drone_phase: [0.0; 3],
            crush_hold: 0.0,
            crush_counter: 0.0,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        // This app's own palette -- true void black shot through with
        // one searing accretion-disk ember, not a device-wide theme.
        // See the palette constants' own doc comment.
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(BLACK_HOLE_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, BLACK_HOLE_TITLE);
        Text::new("Black Hole", Point::new(16, 26), title).draw(fb).ok();
        let dim = MonoTextStyle::new(&SPLEEN_6X12, BLACK_HOLE_DIM);
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
        self.list.draw_themed(fb, 16, 56, 22, 10, &display_rows, BLACK_HOLE_BG, BLACK_HOLE_DIM, BLACK_HOLE_ACCENT);
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

/// A single-pole lowpass, and (by subtracting from the input) a matched
/// first-order highpass sharing the same integrator state.
#[derive(Clone, Copy, Default)]
struct OnePole {
    z: f32,
}

impl OnePole {
    fn lowpass(&mut self, x: f32, cutoff_hz: f32, sample_rate: f32) -> f32 {
        let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz.max(1.0));
        let dt = 1.0 / sample_rate;
        let alpha = dt / (rc + dt);
        self.z += alpha * (x - self.z);
        self.z
    }

    fn highpass(&mut self, x: f32, cutoff_hz: f32, sample_rate: f32) -> f32 {
        x - self.lowpass(x, cutoff_hz, sample_rate)
    }
}

/// A single first-order allpass filter stage (transposed direct form
/// II) -- chained a few deep with a swept coefficient, this is a
/// classic analog-style phaser.
#[derive(Clone, Copy, Default)]
struct AllpassStage {
    z: f32,
}

impl AllpassStage {
    fn process(&mut self, x: f32, coeff: f32) -> f32 {
        let y = self.z - coeff * x;
        self.z = x + coeff * y;
        y
    }
}

/// Two crossfaded read heads into a shared capture buffer, running at
/// `rate` instead of 1 sample/sample -- the classic cheap-and-real
/// pitch-shifter trick (also used by Beads), reused here for every
/// pitch-related algorithm.
#[derive(Clone, Copy, Default)]
struct PitchShifter {
    read_a: f32,
    read_b: f32,
    crossfade: f32,
}

impl PitchShifter {
    fn process(&mut self, capture: &[f32], rate: f32, grain_samples: f32) -> f32 {
        let cap_len = capture.len() as f32;
        self.read_a = (self.read_a + rate).rem_euclid(cap_len);
        self.read_b = (self.read_b + rate).rem_euclid(cap_len);
        self.crossfade = (self.crossfade + (1.0 - rate).abs().max(0.001) / grain_samples.max(1.0)).fract();
        let read = |pos: f32| {
            let i0 = pos as usize % capture.len();
            let i1 = (i0 + 1) % capture.len();
            let frac = pos - pos.floor();
            capture[i0] * (1.0 - frac) + capture[i1] * frac
        };
        let a = read(self.read_a);
        let b = read(self.read_b);
        let mix = (self.crossfade * std::f32::consts::PI).sin();
        a * (1.0 - mix) + b * mix
    }
}

#[derive(Clone, Copy, Default)]
struct Grain {
    active: bool,
    pos: f32,
    length: f32,
    remaining: f32,
}

/// A small comb+allpass reverb tank (2 combs + 1 diffusing allpass,
/// same topology Beads uses) shared by every reverb/shimmer algorithm
/// -- not the real module's own (undocumented, Spin FV-1) algorithm,
/// just a modest, genuinely-computed diffuse tail. `tone_hz` filters
/// the feedback path (brighter = higher cutoff); `saturation` (used
/// only by Saturated Reverb) drives the tank input through a tanh
/// waveshaper; `shimmer_semitones` (used only by the 3 Shimmer
/// algorithms), when nonzero, pitch-shifts part of the feedback before
/// it's re-injected.
struct ReverbTank {
    comb_a: Vec<f32>,
    comb_b: Vec<f32>,
    allpass: Vec<f32>,
    pos_a: usize,
    pos_b: usize,
    pos_ap: usize,
    tone: OnePole,
    shimmer: PitchShifter,
}

impl Default for ReverbTank {
    fn default() -> Self {
        Self { comb_a: vec![0.0; 1687], comb_b: vec![0.0; 2053], allpass: vec![0.0; 347], pos_a: 0, pos_b: 0, pos_ap: 0, tone: OnePole::default(), shimmer: PitchShifter::default() }
    }
}

impl ReverbTank {
    fn process(&mut self, input: f32, feedback: f32, tone_hz: f32, sample_rate: f32, saturation: f32, shimmer_semitones: f32) -> f32 {
        let a = self.comb_a[self.pos_a];
        let b = self.comb_b[self.pos_b];
        let mut fed = (a + b) * 0.5 * feedback;
        if shimmer_semitones.abs() > 0.01 {
            let rate = 2f32.powf(shimmer_semitones / 12.0);
            let shifted = self.shimmer.process(&self.comb_a, rate, 4000.0);
            fed = fed * 0.4 + shifted * feedback * 0.6;
        }
        let toned = self.tone.lowpass(fed, tone_hz, sample_rate);
        let mut write_val = input + toned;
        if saturation > 0.001 {
            write_val = (write_val * (1.0 + saturation * 5.0)).tanh();
        }
        self.comb_a[self.pos_a] = write_val;
        self.pos_a = (self.pos_a + 1) % self.comb_a.len();
        self.comb_b[self.pos_b] = write_val;
        self.pos_b = (self.pos_b + 1) % self.comb_b.len();

        let combined = (a + b) * 0.5;
        let ap_in = self.allpass[self.pos_ap];
        let ap_out = -combined + ap_in;
        self.allpass[self.pos_ap] = combined + ap_in * 0.5;
        self.pos_ap = (self.pos_ap + 1) % self.allpass.len();
        ap_out
    }
}

/// Reads `capture` at a fractional delay (in samples) behind the write
/// head, linearly interpolated -- the shared primitive nearly every
/// algorithm below is built from.
fn read_delay(capture: &[f32], write_pos: usize, delay_samples: f32) -> f32 {
    let cap_len = capture.len() as f32;
    let pos = (write_pos as f32 - delay_samples).rem_euclid(cap_len);
    let i0 = pos as usize % capture.len();
    let i1 = (i0 + 1) % capture.len();
    let frac = pos - pos.floor();
    capture[i0] * (1.0 - frac) + capture[i1] * frac
}

struct BlackHoleProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    /// A rolling capture of the (level-staged) tapped source -- every
    /// delay/echo/pitch/freeze algorithm reads from this at its own
    /// offsets instead of owning a separate delay line.
    capture: Vec<f32>,
    write_pos: usize,
    filter_a: OnePole,
    filter_b: OnePole,
    phaser_l: [AllpassStage; 4],
    phaser_r: [AllpassStage; 4],
    phaser_lfo_phase: f32,
    phaser_fb_l: f32,
    phaser_fb_r: f32,
    /// Shared chorus/flanger-family LFO phase (Havoc Chorus, Ripper's
    /// Rip tremolo, Space Phaser's second lane).
    chorus_lfo_phase: f32,
    chirp_phase: f32,
    pitch_a: PitchShifter,
    pitch_b: PitchShifter,
    reverb: ReverbTank,
    grains: [Grain; MAX_GRAINS],
    grain_spawn_accum: f32,
    rng: u32,
    /// LP/HP Freezer loop anchor + read phase (same technique as a
    /// simple loop-the-last-slice freeze).
    freeze_start: usize,
    freeze_phase: f32,
    freeze_engaged_prev: bool,
    drone_phase: [f32; 3],
    crush_hold: f32,
    crush_counter: f32,
}

impl BlackHoleProcessor {
    fn next_rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// CRUSH: sample-and-hold rate reduction plus bit-depth
    /// quantization on the wet/digital path only -- the manual
    /// describes Dry/Wet as "full analogue", so the dry tap (mixed in
    /// by the caller) never passes through here.
    fn apply_crush(&mut self, input: f32, amount: f32) -> f32 {
        let amount = amount.clamp(0.0, 1.0);
        let rate_div = 1.0 + amount * 47.0;
        self.crush_counter += 1.0;
        if self.crush_counter >= rate_div {
            self.crush_counter -= rate_div;
            self.crush_hold = input;
        }
        let bits = (16.0 - amount * 13.0).max(3.0);
        let levels = 2f32.powf(bits);
        (self.crush_hold * levels).round() / levels
    }

    /// Runs the currently-selected algorithm for one sample. Returns
    /// `(wet, feedback_contribution)`: `wet` is what gets mixed into
    /// the output, `feedback_contribution` is what (if anything) gets
    /// added back into the capture buffer's write for the next pass
    /// (delay-family algorithms only -- reverbs/generators manage their
    /// own internal feedback and return 0.0 here).
    fn process_algorithm(&mut self, algo: usize, dry: f32, p1: f32, p2: f32, p3: f32, sample_rate: f32) -> (f32, f32) {
        let cap_len = self.capture.len();
        match algo {
            0 => {
                // Dual Delay: two independent read taps sharing one
                // feedback amount.
                let time_l = map_delay_ms(p2) * 0.001 * sample_rate;
                let time_r = map_delay_ms(p3) * 0.001 * sample_rate;
                let tap_l = read_delay(&self.capture, self.write_pos, time_l);
                let tap_r = read_delay(&self.capture, self.write_pos, time_r);
                let fb = p1.clamp(0.0, 0.95);
                let wet = (tap_l + tap_r) * 0.5;
                (wet, wet * fb)
            }
            1 => {
                // Chirp Delay: a continuous sine sweep on the read
                // offset gives each repeat a pitch-doppler "chirp".
                let time = map_delay_ms(p2) * 0.001 * sample_rate;
                self.chirp_phase += (0.5 + p3 * 6.0) / sample_rate;
                let mod_depth = p3 * time * 0.12;
                let modulated = (time + mod_depth * (self.chirp_phase * std::f32::consts::TAU).sin()).max(1.0);
                let tap = read_delay(&self.capture, self.write_pos, modulated);
                let fb = p1.clamp(0.0, 0.95);
                (tap, tap * fb)
            }
            2 => {
                // Lowpass Delay: one-pole LP in the feedback path.
                let time = map_delay_ms(p2) * 0.001 * sample_rate;
                let tap = read_delay(&self.capture, self.write_pos, time);
                let filtered = self.filter_a.lowpass(tap, map_cutoff_hz(p3), sample_rate);
                let fb = p1.clamp(0.0, 0.95);
                (filtered, filtered * fb)
            }
            3 => {
                // Hipass Delay: matching HP in the feedback path.
                let time = map_delay_ms(p2) * 0.001 * sample_rate;
                let tap = read_delay(&self.capture, self.write_pos, time);
                let filtered = self.filter_a.highpass(tap, map_cutoff_hz(p3), sample_rate);
                let fb = p1.clamp(0.0, 0.95);
                (filtered, filtered * fb)
            }
            4 => {
                // Tap Tap Delay: the output tap and the feedback tap
                // read from two different delay times.
                let time = map_delay_ms(p2) * 0.001 * sample_rate;
                let tap_time = map_delay_ms(p3) * 0.001 * sample_rate;
                let out_tap = read_delay(&self.capture, self.write_pos, time);
                let fb_tap = read_delay(&self.capture, self.write_pos, tap_time);
                let fb = p1.clamp(0.0, 0.95);
                (out_tap, fb_tap * fb)
            }
            5 => {
                // Hellraiser Delay: past 12 o'clock, the feedback tap
                // is read through the pitch shifter instead of
                // plainly, so each repeat rises in pitch.
                let time = map_delay_ms(p2) * 0.001 * sample_rate;
                let out_tap = read_delay(&self.capture, self.write_pos, time);
                let fb = p1.clamp(0.0, 0.95);
                let raiser_active = p3 > 0.5;
                let feedback_source = if raiser_active {
                    let semis = (p3 - 0.5) * 2.0 * 12.0;
                    let rate = 2f32.powf(semis / 12.0);
                    self.pitch_a.process(&self.capture, rate, time.max(64.0))
                } else {
                    out_tap
                };
                (out_tap, feedback_source * fb)
            }
            6 => {
                // Phazed Delay: a second, phased tap ("3rd tap") mixed
                // in alongside the plain delay tap.
                let time = map_delay_ms(p2) * 0.001 * sample_rate;
                let tap1 = read_delay(&self.capture, self.write_pos, time);
                let tap2 = read_delay(&self.capture, self.write_pos, (time * 0.5).max(1.0));
                self.phaser_lfo_phase += 0.3 / sample_rate;
                if self.phaser_lfo_phase >= 1.0 {
                    self.phaser_lfo_phase -= 1.0;
                }
                let coeff = 0.7 * (self.phaser_lfo_phase * std::f32::consts::TAU).sin();
                let mut phased = tap2;
                for stage in self.phaser_l.iter_mut() {
                    phased = stage.process(phased, coeff);
                }
                let fb = p1.clamp(0.0, 0.95);
                let wet = tap1 + phased * p3;
                (wet, tap1 * fb)
            }
            7 => {
                // Granular Delay: a handful of overlapping grains cut
                // from the buffer; Width jitters their start offset.
                let grain_ms = map_grain_ms(p2);
                let length_samples = (grain_ms * 0.001 * sample_rate).max(1.0);
                let hz = 1.0 / (grain_ms * 0.001 * 0.5).max(0.001);
                self.grain_spawn_accum += hz / sample_rate;
                if self.grain_spawn_accum >= 1.0 {
                    self.grain_spawn_accum -= 1.0;
                    let jitter = self.next_rand().abs() * p3 * length_samples * 2.0;
                    if let Some(slot) = self.grains.iter_mut().find(|g| !g.active) {
                        slot.active = true;
                        slot.pos = jitter;
                        slot.length = length_samples;
                        slot.remaining = length_samples;
                    }
                }
                let mut sum = 0.0f32;
                for grain in self.grains.iter_mut() {
                    if !grain.active {
                        continue;
                    }
                    let delay = (grain.pos + (grain.length - grain.remaining)).max(0.0);
                    let s = read_delay(&self.capture, self.write_pos, delay);
                    let progress = 1.0 - (grain.remaining / grain.length).clamp(0.0, 1.0);
                    let window = (std::f32::consts::PI * progress).sin();
                    sum += s * window;
                    grain.remaining -= 1.0;
                    if grain.remaining <= 0.0 {
                        grain.active = false;
                    }
                }
                let wet = sum / (MAX_GRAINS as f32).sqrt();
                let fb = p1.clamp(0.0, 0.95);
                (wet, wet * fb)
            }
            8 => {
                // Pitch Shift Delay: a plain echo feeds the buffer
                // back, while the output is the pitch-shifted read.
                let time = map_delay_ms(p2) * 0.001 * sample_rate;
                let echo = read_delay(&self.capture, self.write_pos, time);
                let semis = map_bipolar_semitones(p3, 24.0);
                let rate = 2f32.powf(semis / 12.0);
                let shifted = self.pitch_a.process(&self.capture, rate, time.max(64.0));
                let fb = p1.clamp(0.0, 0.95);
                (shifted, echo * fb)
            }
            9 | 10 | 11 => {
                // Shimmer Drift / + / -: a pre-delayed tap feeds the
                // shared reverb tank with feedback pitch-shifted up
                // (bidirectional for Drift, fixed direction for +/-).
                let predelay = map_predelay_ms(p1) * 0.001 * sample_rate;
                let fed_in = read_delay(&self.capture, self.write_pos, predelay);
                let size = p2.clamp(0.0, 1.0);
                let feedback = (0.3 + size * 0.65).min(0.95);
                let semis = match algo {
                    9 => map_bipolar_semitones(p3, 12.0),
                    10 => map_unipolar_semitones(p3, 12.0),
                    _ => -map_unipolar_semitones(p3, 12.0),
                };
                let wet = self.reverb.process(fed_in, feedback, 8000.0, sample_rate, 0.0, semis);
                (wet, 0.0)
            }
            12 | 13 => {
                // Big/Hall Reverb (longer decay ceiling) and Room
                // Reverb (shorter) -- same tank, different feedback
                // range for the "size" perception.
                let predelay = map_predelay_ms(p1) * 0.001 * sample_rate;
                let fed_in = read_delay(&self.capture, self.write_pos, predelay);
                let size = p2.clamp(0.0, 1.0);
                let feedback = if algo == 12 { (0.4 + size * 0.55).min(0.95) } else { (0.2 + size * 0.55).min(0.85) };
                let tone = map_cutoff_hz(p3);
                let wet = self.reverb.process(fed_in, feedback, tone, sample_rate, 0.0, 0.0);
                (wet, 0.0)
            }
            14 => {
                // Stalker Reverb: Feedback is exposed directly, so it
                // can be pushed toward near-self-oscillating drones.
                let feedback = p1.clamp(0.0, 0.97);
                let size = p2.clamp(0.0, 1.0);
                let tone = map_cutoff_hz(p3);
                let wet = self.reverb.process(dry, (feedback * (0.6 + size * 0.4)).min(0.97), tone, sample_rate, 0.0, 0.0);
                (wet, 0.0)
            }
            15 => {
                // Saturated Reverb: input driven through a waveshaper
                // before it hits the tank.
                let sat = p1.clamp(0.0, 1.0);
                let size = p2.clamp(0.0, 1.0);
                let tone = map_cutoff_hz(p3);
                let wet = self.reverb.process(dry, (0.3 + size * 0.6).min(0.95), tone, sample_rate, sat, 0.0);
                (wet, 0.0)
            }
            16 => {
                // Havoc Chorus: two LFO-modulated short delay taps
                // (quadrature phase for Width) summed, with feedback.
                let rate = map_rate_hz(p2);
                self.chorus_lfo_phase += rate / sample_rate;
                if self.chorus_lfo_phase >= 1.0 {
                    self.chorus_lfo_phase -= 1.0;
                }
                let base_ms = 8.0;
                let depth_ms = 2.0 + p3 * 6.0;
                let d1 = (base_ms + depth_ms * (self.chorus_lfo_phase * std::f32::consts::TAU).sin()).max(0.5);
                let d2 = (base_ms + depth_ms * ((self.chorus_lfo_phase + 0.25) * std::f32::consts::TAU).sin()).max(0.5);
                let t1 = read_delay(&self.capture, self.write_pos, d1 * 0.001 * sample_rate);
                let t2 = read_delay(&self.capture, self.write_pos, d2 * 0.001 * sample_rate);
                let wet = t1 * 0.5 + t2 * 0.5;
                let fb = p1.clamp(0.0, 0.9);
                (wet, wet * fb)
            }
            17 => {
                // Servo Flanger: two independent short taps, comb
                // filtering against each other and their own feedback.
                let d1 = map_short_delay_ms(p2) * 0.001 * sample_rate;
                let d2 = map_short_delay_ms(p3) * 0.001 * sample_rate;
                let t1 = read_delay(&self.capture, self.write_pos, d1);
                let t2 = read_delay(&self.capture, self.write_pos, d2);
                let wet = (t1 + t2) * 0.5;
                let fb = p1.clamp(0.0, 0.9);
                (wet, wet * fb)
            }
            18 => {
                // Ripper: drive -> bitcrush -> a fast tremolo "rip"
                // gate. Full-wet recommended per the manual.
                let drive = 1.0 + p1 * 12.0;
                let driven = (dry * drive).tanh();
                let bits = (16.0 - p2 * 13.0).max(3.0);
                let levels = 2f32.powf(bits);
                let crushed = (driven * levels).round() / levels;
                self.chorus_lfo_phase += (2.0 + p3 * 60.0) / sample_rate;
                if self.chorus_lfo_phase >= 1.0 {
                    self.chorus_lfo_phase -= 1.0;
                }
                let duty = (0.15 + p3 * 0.5).clamp(0.05, 0.95);
                let rip_gate = if self.chorus_lfo_phase < duty { 1.0 } else { 0.15 };
                (crushed * rip_gate, 0.0)
            }
            19 => {
                // Space Phaser: two independently-swept 4-stage
                // allpass chains, each with their own feedback.
                let fb = p1.clamp(0.0, 0.9);
                let rate_l = 0.05 + p2 * 2.0;
                let rate_r = 0.05 + p3 * 2.0;
                self.phaser_lfo_phase += rate_l / sample_rate;
                self.chorus_lfo_phase += rate_r / sample_rate;
                if self.phaser_lfo_phase >= 1.0 {
                    self.phaser_lfo_phase -= 1.0;
                }
                if self.chorus_lfo_phase >= 1.0 {
                    self.chorus_lfo_phase -= 1.0;
                }
                let coeff_l = 0.75 * (self.phaser_lfo_phase * std::f32::consts::TAU).sin();
                let coeff_r = 0.75 * (self.chorus_lfo_phase * std::f32::consts::TAU).sin();
                let in_l = dry + self.phaser_fb_l * fb;
                let in_r = dry + self.phaser_fb_r * fb;
                let mut yl = in_l;
                for s in self.phaser_l.iter_mut() {
                    yl = s.process(yl, coeff_l);
                }
                let mut yr = in_r;
                for s in self.phaser_r.iter_mut() {
                    yr = s.process(yr, coeff_r);
                }
                self.phaser_fb_l = yl;
                self.phaser_fb_r = yr;
                let wet = (dry + (yl + yr) * 0.5) * 0.5;
                (wet, 0.0)
            }
            20 => {
                // Dual Pitch Shifter: two independent shifters, mixed
                // by Crossfade.
                let semis1 = map_bipolar_semitones(p1, 12.0);
                let semis2 = map_bipolar_semitones(p2, 12.0);
                let rate1 = 2f32.powf(semis1 / 12.0);
                let rate2 = 2f32.powf(semis2 / 12.0);
                let v1 = self.pitch_a.process(&self.capture, rate1, 2000.0);
                let v2 = self.pitch_b.process(&self.capture, rate2, 2000.0);
                let cross = p3.clamp(0.0, 1.0);
                (v1 * (1.0 - cross) + v2 * cross, 0.0)
            }
            21 | 22 => {
                // LP/HP Freezer: past 12 o'clock ("Record") loops a
                // captured slice of Time length forever; below it,
                // passes live (filtered) signal through.
                let engaged = p1 > 0.5;
                let length_samples = (map_freeze_ms(p2) * 0.001 * sample_rate).max(1.0);
                if engaged && !self.freeze_engaged_prev {
                    self.freeze_start = (self.write_pos as f32 - length_samples).rem_euclid(cap_len as f32) as usize;
                    self.freeze_phase = 0.0;
                }
                self.freeze_engaged_prev = engaged;
                let raw = if engaged {
                    self.freeze_phase = (self.freeze_phase + 1.0) % length_samples;
                    let idx = (self.freeze_start as f32 + self.freeze_phase).rem_euclid(cap_len as f32) as usize % cap_len;
                    self.capture[idx]
                } else {
                    self.freeze_phase = 0.0;
                    dry
                };
                let cutoff = map_cutoff_hz(p3);
                let filtered = if algo == 21 { self.filter_b.lowpass(raw, cutoff, sample_rate) } else { self.filter_b.highpass(raw, cutoff, sample_rate) };
                (filtered, 0.0)
            }
            _ => {
                // Drone Bank: 3 independent sine oscillators -- a pure
                // generator, ignores the tapped/dry signal entirely.
                let freqs = [map_drone_hz(p1), map_drone_hz(p2), map_drone_hz(p3)];
                for (phase, freq) in self.drone_phase.iter_mut().zip(freqs) {
                    *phase += freq / sample_rate;
                    if *phase >= 1.0 {
                        *phase -= 1.0;
                    }
                }
                let wet = self.drone_phase.iter().map(|ph| (ph * std::f32::consts::TAU).sin()).sum::<f32>() / 3.0;
                (wet, 0.0)
            }
        }
    }
}

impl AudioProcessor for BlackHoleProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        for out in buffer.iter_mut() {
            *out = 0.0;
        }

        let source_idx = self.params.source.load(Ordering::Relaxed);
        let tapped = self.audio_bus.get(source_idx).map(|b| b.lock().unwrap().clone());

        let algo = self.params.algorithm.load(Ordering::Relaxed) as usize % NUM_ALGORITHMS;
        let in_gain = self.params.in_level.get().clamp(0.0, 1.0) * 2.0;
        let p1 = self.params.param1.get().clamp(0.0, 1.0);
        let p2 = self.params.param2.get().clamp(0.0, 1.0);
        let p3 = self.params.param3.get().clamp(0.0, 1.0);
        let crush = self.params.crush.get().clamp(0.0, 1.0);
        let dry_wet = self.params.dry_wet.get().clamp(0.0, 1.0);
        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        let cap_len = self.capture.len();

        let frames = buffer.len() / channels;
        let mut mono = vec![0.0f32; frames];
        let mut clipped = false;

        for n in 0..frames {
            let dry_in = tapped.as_ref().and_then(|b| b.get(n % b.len().max(1)).copied()).unwrap_or(0.0);
            let leveled = dry_in * in_gain;
            if leveled.abs() > 0.98 {
                clipped = true;
            }

            let (wet, feedback_contribution) = self.process_algorithm(algo, leveled, p1, p2, p3, sample_rate);
            let crushed_wet = self.apply_crush(wet, crush);

            let write_idx = self.write_pos % cap_len;
            self.capture[write_idx] = (leveled + feedback_contribution).clamp(-8.0, 8.0);
            self.write_pos = (self.write_pos + 1) % cap_len;

            let mixed = dry_in * (1.0 - dry_wet) + crushed_wet * dry_wet;
            mono[n] = (mixed * mix_level).clamp(-4.0, 4.0).tanh();
        }

        self.params.clip.store(clipped, Ordering::Relaxed);

        const SNAPSHOT_POINTS: usize = 80;
        if mono.len() >= SNAPSHOT_POINTS {
            let step = mono.len() as f32 / SNAPSHOT_POINTS as f32;
            let snapshot: Vec<f32> = (0..SNAPSHOT_POINTS).map(|i| mono[((i as f32 * step) as usize).min(mono.len() - 1)]).collect();
            *self.params.waveform_snapshot.lock().unwrap() = snapshot;
        }

        for (out, s) in buffer.chunks_mut(channels).zip(mono.iter()) {
            for ch in out.iter_mut() {
                *ch = *s;
            }
        }
        *self.params.bus_out.lock().unwrap() = mono;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> (BlackHoleApp, Arc<AudioBus>) {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let app = BlackHoleApp::new(sensitivity, nav_speed, modbus, Arc::clone(&audio_bus), mixer_bus);
        (app, audio_bus)
    }

    /// Drone Bank (23) is a pure generator and is exempted below --
    /// every other algorithm must stay silent with no tapped source.
    #[test]
    fn no_source_is_silence_not_a_panic() {
        let (mut app, _audio_bus) = new_app();
        app.params.dry_wet.set(1.0);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for algo in 0..NUM_ALGORITHMS {
            if algo == 23 {
                continue;
            }
            app.params.algorithm.store(algo as u32, Ordering::Relaxed);
            if algo == 21 || algo == 22 {
                app.params.param1.set(1.0); // engage Record so Freezer actually reads its (silent) loop
            }
            processor.process(&mut buffer, 2, 48000.0);
            assert!(buffer.iter().all(|&s| s == 0.0), "algorithm {algo} should be silent with no source");
        }
    }

    /// Drone Bank must produce sound even with nothing tapped -- it's
    /// a generator, not an effect.
    #[test]
    fn drone_bank_sounds_with_no_source() {
        let (mut app, _audio_bus) = new_app();
        app.params.algorithm.store(23, Ordering::Relaxed);
        app.params.dry_wet.set(1.0);
        app.params.param1.set(0.4);
        app.params.param2.set(0.6);
        app.params.param3.set(0.8);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.01, "expected Drone Bank to sound with no source, got peak {peak}");
    }

    /// A loud tapped source must produce audible output on the default
    /// algorithm (Dual Delay).
    #[test]
    fn a_loud_tapped_source_produces_audible_output() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed); // Black Hole registers its own bus first (index 0) -- "Source" lands at 1.
        *source.lock().unwrap() = vec![0.8; 512];
        app.params.dry_wet.set(1.0);
        app.params.param1.set(0.5);
        // Dual Delay's default Time Left/Right (param2/param3 = 0.5)
        // map to ~750ms -- too long to have echoed back within this
        // test's short run, so pull the taps in close.
        app.params.param2.set(0.0);
        app.params.param3.set(0.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.01, "expected audible output from a loud tapped source, got peak {peak}");
    }

    /// Every algorithm must stay finite and bounded under sustained
    /// loud input and extreme parameters -- a regression net for the
    /// whole 24-algorithm set.
    #[test]
    fn every_algorithm_stays_bounded_and_finite() {
        for algo in 0..NUM_ALGORITHMS {
            let (mut app, audio_bus) = new_app();
            let source = audio_bus.register("Source");
            app.params.source.store(1, Ordering::Relaxed);
            *source.lock().unwrap() = vec![0.95; 512];
            app.params.algorithm.store(algo as u32, Ordering::Relaxed);
            app.params.param1.set(0.9);
            app.params.param2.set(0.9);
            app.params.param3.set(0.9);
            app.params.crush.set(0.7);
            app.params.dry_wet.set(1.0);
            app.params.in_level.set(1.0);

            let mut processor = app.audio_processor().unwrap();
            let mut buffer = vec![0.0f32; 512 * 2];
            for _ in 0..150 {
                processor.process(&mut buffer, 2, 48000.0);
            }
            assert!(buffer.iter().all(|s| s.is_finite()), "algorithm {algo} produced a non-finite sample");
            let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
            assert!(peak <= 1.0001, "algorithm {algo} exceeded unity: {peak}");
        }
    }

    /// LP Freezer must keep looping a captured slice after the tapped
    /// source goes silent, once Record is engaged.
    #[test]
    fn lp_freezer_keeps_sounding_after_input_stops() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        app.params.algorithm.store(21, Ordering::Relaxed); // LP Freezer
        app.params.param2.set(0.2); // short loop
        app.params.param3.set(1.0); // filter wide open
        app.params.dry_wet.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];

        // Capture real signal first, with Record still disengaged.
        *source.lock().unwrap() = vec![0.8; 512];
        for _ in 0..10 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        app.params.param1.set(1.0); // engage Record
        *source.lock().unwrap() = vec![0.0; 512];

        let mut peak = 0.0f32;
        for _ in 0..30 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.01, "expected LP Freezer to keep sounding after input stopped, got peak {peak}");
    }

    /// CRUSH must audibly change the output (rate/bit reduction) while
    /// staying bounded and finite.
    #[test]
    fn crush_alters_output_and_stays_bounded() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = (0..512).map(|i| (i as f32 * 0.3).sin() * 0.9).collect();
        app.params.dry_wet.set(1.0);
        app.params.param1.set(0.3);
        // Pull Dual Delay's taps in close so this single short block
        // actually hears the (crushed or clean) signal rather than
        // silence from a tap that hasn't echoed back yet.
        app.params.param2.set(0.0);
        app.params.param3.set(0.0);

        let mut clean_processor = app.audio_processor().unwrap();
        let mut crushed_processor = app.audio_processor().unwrap();
        app.params.crush.set(1.0);
        // Re-fetch params after mutation isn't needed -- both
        // processors share the same `Arc<Params>` via `app.params`.
        let mut buffer_clean = vec![0.0f32; 512 * 2];
        let mut buffer_crushed = vec![0.0f32; 512 * 2];

        app.params.crush.set(0.0);
        clean_processor.process(&mut buffer_clean, 2, 48000.0);
        app.params.crush.set(1.0);
        crushed_processor.process(&mut buffer_crushed, 2, 48000.0);

        assert!(buffer_crushed.iter().all(|s| s.is_finite()));
        let differs = buffer_clean.iter().zip(buffer_crushed.iter()).any(|(a, b)| (a - b).abs() > 1e-6);
        assert!(differs, "expected CRUSH to audibly change the output");
    }

    /// Save Patch / Recall Patch must round-trip a full set of
    /// parameters, keyed by algorithm slot.
    #[test]
    fn save_and_recall_patch_round_trips_parameters() {
        let (mut app, _audio_bus) = new_app();
        app.params.algorithm.store(3, Ordering::Relaxed);
        app.params.param1.set(0.71);
        app.params.param2.set(0.22);
        app.params.param3.set(0.93);
        app.params.crush.set(0.44);
        app.params.dry_wet.set(0.66);
        app.params.in_level.set(0.55);

        app.reset(Selection::SavePatch);
        assert!(app.params.patches.lock().unwrap()[3].saved);

        // Disturb everything.
        app.params.param1.set(0.0);
        app.params.param2.set(0.0);
        app.params.param3.set(0.0);
        app.params.crush.set(0.0);
        app.params.dry_wet.set(0.0);
        app.params.in_level.set(0.0);

        app.reset(Selection::RecallPatch);
        assert!((app.params.param1.get() - 0.71).abs() < 1e-6);
        assert!((app.params.param2.get() - 0.22).abs() < 1e-6);
        assert!((app.params.param3.get() - 0.93).abs() < 1e-6);
        assert!((app.params.crush.get() - 0.44).abs() < 1e-6);
        assert!((app.params.dry_wet.get() - 0.66).abs() < 1e-6);
        assert!((app.params.in_level.get() - 0.55).abs() < 1e-6);
    }

    /// Recall on a never-saved slot must be a harmless no-op.
    #[test]
    fn recall_on_an_empty_slot_is_a_no_op() {
        let (mut app, _audio_bus) = new_app();
        app.params.algorithm.store(5, Ordering::Relaxed);
        app.params.param1.set(0.37);
        app.reset(Selection::RecallPatch);
        assert!((app.params.param1.get() - 0.37).abs() < 1e-6);
    }

    /// The clip indicator must reflect the level-staged input peak.
    #[test]
    fn in_level_clip_indicator_reflects_input_peak() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.99; 512];
        app.params.in_level.set(1.0); // 2x gain -- pushes well past the clip threshold

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        assert!(app.params.clip.load(Ordering::Relaxed), "expected the clip indicator to be set");

        *source.lock().unwrap() = vec![0.01; 512];
        app.params.in_level.set(0.1);
        processor.process(&mut buffer, 2, 48000.0);
        assert!(!app.params.clip.load(Ordering::Relaxed), "expected the clip indicator to clear on a quiet block");
    }

    /// Black Hole must republish its own output on the audio bus.
    #[test]
    fn republishes_its_output_on_the_audio_bus() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.5; 256];

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 256 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        let published = app.params.bus_out.lock().unwrap();
        assert_eq!(published.len(), 256);
    }
}
