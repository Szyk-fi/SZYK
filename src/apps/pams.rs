//! A clone of the core of Pamela's Pro Workout -- not the whole thing.
//! Real Pam's has 8 channels, per-channel logic combinators between
//! channels (MIX/MASK/AND/OR/XOR/NOT/ADD/SUB), loop region sleep/wake,
//! CV attenuation/offset as separate hardware-facing concerns, and true
//! CV/gate outputs. This covers the "core clock + shapes + Euclidean"
//! slice: 4 channels, each independently clocked off a shared master
//! BPM (via a clock multiply/divide), generating one of a handful of
//! shapes (Ramp Up/Down, Hump, Square, Euclidean, Sample & Hold),
//! shaped by Width/Slew/Level/Offset/Phase/Invert, optionally quantized
//! to a scale, with Swing/Human timing variation. The inter-channel
//! logic ops and loop nap/wake are real Pam's features, deliberately
//! not included here yet.
//!
//! What actually makes this a *working* modulation source rather than a
//! bench toy: every channel's Target leaf routes into the shared
//! ModBus (see modbus.rs) -- the same registry Plaits' Harmonics/
//! Timbre/Morph/Decay and Sequencer's per-track Volume publish
//! themselves into. Because every app's audio processor now runs
//! continuously in the mix bus (see audio.rs) rather than only the one
//! on screen, routing a channel to "Plaits: Timbre" modulates Plaits
//! live, whether or not Plaits' own screen happens to be showing.
//!
//! No grid usage -- channels are continuous/clocked generators, not a
//! step pattern you toggle by hand (Euclidean's pattern is *derived*
//! from Steps/Pulses/Rotation, not hand-entered). The menu + a
//! Plaits-style context panel (this channel's live output, scrolling)
//! is the whole control surface.

use crate::app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes, Throw};
use crate::app::{App, Input};
use crate::apps::plaits::{ROOT_NAMES, SCALE_TYPES};
use crate::audio::AudioProcessor;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NUM_CHANNELS: usize = 10;
const NUM_GROUPS: usize = 1 + NUM_CHANNELS;
const MIN_BPM: f32 = 40.0;
const MAX_BPM: f32 = 300.0;
const DEFAULT_BPM: f32 = 120.0;
const MAX_SWING: f32 = 0.6;
const MAX_HUMAN: f32 = 1.0;
const MAX_EUCLID_STEPS: usize = 32;
const MONITOR_LEN: usize = 150;

const SHAPE_NAMES: [&str; 6] = ["Ramp Up", "Ramp Down", "Hump", "Square", "Euclidean", "S&H"];
const SHAPE_EUCLIDEAN: u32 = 4;
const SHAPE_SAMPLE_HOLD: u32 = 5;
/// (label, multiplier relative to the master BPM's quarter-note rate).
const CLOCK_MODS: [(&str, f32); 9] =
    [("/8", 0.125), ("/4", 0.25), ("/2", 0.5), ("x1", 1.0), ("x2", 2.0), ("x3", 3.0), ("x4", 4.0), ("x8", 8.0), ("x16", 16.0)];

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * (max - min) * 0.05).clamp(min, max);
    value.set(next);
}

/// Bresenham/staircase construction -- musically equivalent to
/// Bjorklund's algorithm for standard Euclidean-rhythm parameters,
/// simpler to write allocation-free (writes into a fixed array instead
/// of returning a `Vec`, since this runs on the audio thread).
fn euclidean_pattern(steps: usize, pulses: usize, rotation: usize, out: &mut [bool; MAX_EUCLID_STEPS]) {
    for o in out.iter_mut() {
        *o = false;
    }
    let steps = steps.clamp(1, MAX_EUCLID_STEPS);
    let pulses = pulses.min(steps);
    if pulses == 0 {
        return;
    }
    let mut pattern = [false; MAX_EUCLID_STEPS];
    let mut bucket = 0.0f32;
    let step_size = pulses as f32 / steps as f32;
    for slot in pattern.iter_mut().take(steps) {
        bucket += step_size;
        if bucket >= 1.0 {
            bucket -= 1.0;
            *slot = true;
        }
    }
    let rot = rotation % steps;
    for i in 0..steps {
        out[i] = pattern[(i + rot) % steps];
    }
}

fn shape_value(shape: u32, phase: f32, width: f32) -> f32 {
    match shape {
        0 => phase * 2.0 - 1.0,   // Ramp Up
        1 => 1.0 - phase * 2.0,   // Ramp Down
        2 => {
            // Hump: rises to a peak at `width` through the cycle, falls back
            let w = width.clamp(0.05, 0.95);
            let v = if phase < w { phase / w } else { 1.0 - (phase - w) / (1.0 - w) };
            v * 2.0 - 1.0
        }
        3 => {
            if phase < width.clamp(0.01, 0.99) {
                1.0
            } else {
                -1.0
            }
        }
        _ => 0.0, // Euclidean/S&H computed separately -- see PamsProcessor::process
    }
}

/// Quantizes a -1..1 value to the nearest scale degree across 3
/// octaves of range, then re-normalizes back to -1..1. Not literal
/// pitch quantization (nothing here is denominated in Hz) -- it just
/// gives "steppy, in-key-feeling" modulation instead of smooth.
fn quantize(value: f32, scale_idx: usize, root: u32) -> f32 {
    let intervals = SCALE_TYPES[scale_idx % SCALE_TYPES.len()].1;
    let len = intervals.len().max(1);
    let total = len * 3;
    let idx = (((value * 0.5 + 0.5) * total as f32).round() as i32).clamp(0, total as i32 - 1) as usize;
    let octave = idx / len;
    let degree = idx % len;
    let semitone = intervals[degree] + root as i32 + (octave as i32) * 12;
    (semitone as f32 / 36.0).clamp(-1.0, 1.0)
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Bpm,
    Target(usize),
    TargetInput(usize),
    ClockMod(usize),
    Shape(usize),
    Width(usize),
    Phase(usize),
    EuclidSteps(usize),
    EuclidPulses(usize),
    EuclidRotation(usize),
    Slew(usize),
    Level(usize),
    Offset(usize),
    Invert(usize),
    QuantizerOn(usize),
    QuantizerScale(usize),
    QuantizerRoot(usize),
    Swing(usize),
    Human(usize),
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

struct ChannelParams {
    target: AtomicUsize, // 0 = none, else (modbus index + 1)
    clock_mod: AtomicUsize,
    shape: AtomicU32,
    width: AtomicF32,
    phase: AtomicF32,
    euclid_steps: AtomicU32,
    euclid_pulses: AtomicU32,
    euclid_rotation: AtomicU32,
    slew: AtomicF32,
    level: AtomicF32,
    offset: AtomicF32,
    invert: AtomicBool,
    quantizer_on: AtomicBool,
    quantizer_scale: AtomicU32,
    quantizer_root: AtomicU32,
    swing: AtomicF32,
    human: AtomicF32,
    /// Live output, for the "CV Monitoring" panel.
    monitor: AtomicF32,
    monitor_history: Mutex<VecDeque<f32>>,
}

impl ChannelParams {
    fn new() -> Self {
        Self {
            target: AtomicUsize::new(0),
            clock_mod: AtomicUsize::new(3), // "x1"
            shape: AtomicU32::new(0),
            width: AtomicF32::new(0.5),
            phase: AtomicF32::new(0.0),
            euclid_steps: AtomicU32::new(16),
            euclid_pulses: AtomicU32::new(4),
            euclid_rotation: AtomicU32::new(0),
            slew: AtomicF32::new(0.0),
            level: AtomicF32::new(1.0),
            offset: AtomicF32::new(0.0),
            invert: AtomicBool::new(false),
            quantizer_on: AtomicBool::new(false),
            quantizer_scale: AtomicU32::new(0),
            quantizer_root: AtomicU32::new(0),
            swing: AtomicF32::new(0.0),
            human: AtomicF32::new(0.0),
            monitor: AtomicF32::new(0.0),
            monitor_history: Mutex::new(VecDeque::with_capacity(MONITOR_LEN)),
        }
    }
}

struct Params {
    running: AtomicBool,
    bpm: AtomicF32,
    channels: [ChannelParams; NUM_CHANNELS],
}

impl Params {
    fn new() -> Self {
        Self { running: AtomicBool::new(true), bpm: AtomicF32::new(DEFAULT_BPM), channels: std::array::from_fn(|_| ChannelParams::new()) }
    }
}

pub struct PamsApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    modbus: Arc<ModBus>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    last_channel: usize,
    /// The shared play view (play_kit.rs). Pam's has no pad behaviour of
    /// its own, so the pads start on the kit's layers.
    kit: PlayKit,
}

/// The play view's controls, most important first. All but the master
/// clock act on the selected channel (`last_channel`, which D-pad
/// up/down picks on the play view). Rate and the waveform's level/width/
/// offset are what you ride live; the shape-choosing and Euclidean
/// controls sit on the upper pads of the Controls layer.
const C_BPM: usize = 0;
const C_LEVEL: usize = 1;
const C_WIDTH: usize = 2;
const C_OFFSET: usize = 3;
const C_SLEW: usize = 4;
const C_PHASE: usize = 5;
const C_SWING: usize = 6;
const C_HUMAN: usize = 7;
const C_CHANNEL: usize = 8;
const C_SHAPE: usize = 9;
const C_CLOCK_MOD: usize = 10;
const C_INVERT: usize = 11;
const C_QUANTIZER: usize = 12;
const C_EUCLID_STEPS: usize = 13;
const C_EUCLID_PULSES: usize = 14;
const C_EUCLID_ROTATION: usize = 15;
const NUM_CONTROLS: usize = 16;

/// A clock-mod index as a 0..1 throw/moment position.
fn clock_mod_norm(idx: usize) -> f32 {
    idx as f32 / (CLOCK_MODS.len() - 1) as f32
}

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "pams",
        // No native layer: Pam's never used the pads (its patterns are
        // derived, not hand-entered). Throws suit a clock source as much
        // as an effect: ratchets and momentary mutes are what a player
        // reaches for on the hardware's own buttons.
        // Throws first: on Controls, knob 2 follows the grabbed pad
        // instead of the hero pair, which is the wrong default.
        layers: vec![Layer::Throws, Layer::Controls, Layer::Moments],
        hero: vec![[C_BPM, C_LEVEL], [C_WIDTH, C_OFFSET], [C_SLEW, C_PHASE], [C_SWING, C_HUMAN]],
        // D-pad up/down picks the channel -- ten of them, and the play
        // view would otherwise be stuck shaping only one.
        browse: Some(C_CHANNEL),
        // Stick: width (pulse/hump shape) on X, offset (where the
        // modulation sits) on Y. Hands: slew smooths the output, and the
        // master clock rushes -- springing back to the set BPM.
        routes: Routes { stick_x: Some(C_WIDTH), stick_y: Some(C_OFFSET), hand_l: Some(C_SLEW), hand_r: Some(C_BPM) },
        throws: vec![
            // Indexes into CLOCK_MODS: 6 = x4, 8 = x16, 2 = /2.
            Throw { control: C_CLOCK_MOD, to: clock_mod_norm(6), label: "x4" },
            Throw { control: C_CLOCK_MOD, to: clock_mod_norm(8), label: "x16" },
            Throw { control: C_CLOCK_MOD, to: clock_mod_norm(2), label: "/2" },
            // Shape 5 of 0..5 = S&H: a momentary burst of random steps.
            Throw { control: C_SHAPE, to: 1.0, label: "S&H" },
            Throw { control: C_INVERT, to: 1.0, label: "INVERT" },
            Throw { control: C_QUANTIZER, to: 1.0, label: "QUANT" },
            Throw { control: C_SLEW, to: 1.0, label: "SMOOTH" },
            Throw { control: C_LEVEL, to: 0.0, label: "MUTE" },
        ],
        midi_to_pads: true,
        own_expression: false,
    }
}

// --- Pam's own palette: industrial amber on graphite, not a
// device-wide theme -- the color of a utility module's own indicator
// lamps, a precise workhorse clock rather than an instrument. ---

const PAMS_BG: Rgb565 = Rgb565::new(2, 6, 3);
const PAMS_TITLE: Rgb565 = Rgb565::new(29, 58, 27);
const PAMS_ACCENT: Rgb565 = Rgb565::new(31, 43, 4);
const PAMS_DIM: Rgb565 = Rgb565::new(15, 27, 11);
const PAMS_MIDLINE: Rgb565 = Rgb565::new(5, 10, 4);

impl PamsApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>) -> Self {
        Self {
            params: Arc::new(Params::new()),
            sensitivity,
            nav_speed,
            modbus,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
            last_channel: 0,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
        }
    }

    fn group_name(&self, g: usize) -> String {
        if g == 0 {
            "Global".into()
        } else {
            format!("Channel {g}")
        }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        if g == 0 {
            return vec![Selection::Bpm];
        }
        let c = g - 1;
        // Shape (what kind of pattern this channel generates) comes
        // first, before where it's routed -- matching Sequencer's
        // Instrument-first convention for "what kind of thing is
        // this" -- with its shape-dependent extra controls
        // immediately after it, then Target/ClockMod (routing/timing).
        let mut leaves = vec![Selection::Shape(c)];
        match self.params.channels[c].shape.load(Ordering::Relaxed) {
            s if s == SHAPE_EUCLIDEAN => {
                leaves.push(Selection::EuclidSteps(c));
                leaves.push(Selection::EuclidPulses(c));
                leaves.push(Selection::EuclidRotation(c));
            }
            s if s == SHAPE_SAMPLE_HOLD => {}
            _ => {
                leaves.push(Selection::Width(c));
                leaves.push(Selection::Phase(c));
            }
        }
        leaves.push(Selection::Target(c));
        leaves.push(Selection::TargetInput(c));
        leaves.push(Selection::ClockMod(c));
        leaves.push(Selection::Slew(c));
        leaves.push(Selection::Level(c));
        leaves.push(Selection::Offset(c));
        leaves.push(Selection::Invert(c));
        leaves.push(Selection::QuantizerOn(c));
        if self.params.channels[c].quantizer_on.load(Ordering::Relaxed) {
            leaves.push(Selection::QuantizerScale(c));
            leaves.push(Selection::QuantizerRoot(c));
        }
        leaves.push(Selection::Swing(c));
        leaves.push(Selection::Human(c));
        leaves
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

    fn selection_channel(sel: Selection) -> Option<usize> {
        match sel {
            Selection::Bpm => None,
            Selection::Target(c)
            | Selection::TargetInput(c)
            | Selection::ClockMod(c)
            | Selection::Shape(c)
            | Selection::Width(c)
            | Selection::Phase(c)
            | Selection::EuclidSteps(c)
            | Selection::EuclidPulses(c)
            | Selection::EuclidRotation(c)
            | Selection::Slew(c)
            | Selection::Level(c)
            | Selection::Offset(c)
            | Selection::Invert(c)
            | Selection::QuantizerOn(c)
            | Selection::QuantizerScale(c)
            | Selection::QuantizerRoot(c)
            | Selection::Swing(c)
            | Selection::Human(c) => Some(c),
        }
    }

    fn current_channel(&self, rows: &[Row]) -> usize {
        // On the play view the menu's cursor isn't moving, so D-pad
        // up/down picks the channel directly (see `C_CHANNEL`).
        if !self.kit.menu {
            return self.last_channel;
        }
        match rows.get(self.list.selected) {
            Some(Row::Group(g)) if *g >= 1 => *g - 1,
            Some(Row::Leaf(sel)) => Self::selection_channel(*sel).unwrap_or(self.last_channel),
            _ => self.last_channel,
        }
    }

    fn target_name(&self, idx: usize) -> String {
        crate::modbus::Patch::label(&self.modbus, idx)
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Bpm => "BPM".into(),
            Selection::Target(_) => "Target App".into(),
            Selection::TargetInput(_) => "Target Input".into(),
            Selection::ClockMod(_) => "Clock Mod".into(),
            Selection::Shape(_) => "Shape".into(),
            Selection::Width(_) => "Width".into(),
            Selection::Phase(_) => "Phase".into(),
            Selection::EuclidSteps(_) => "Steps".into(),
            Selection::EuclidPulses(_) => "Pulses".into(),
            Selection::EuclidRotation(_) => "Rotation".into(),
            Selection::Slew(_) => "Slew".into(),
            Selection::Level(_) => "Level".into(),
            Selection::Offset(_) => "Offset".into(),
            Selection::Invert(_) => "Invert".into(),
            Selection::QuantizerOn(_) => "Quantizer".into(),
            Selection::QuantizerScale(_) => "Main scale".into(),
            Selection::QuantizerRoot(_) => "Root note".into(),
            Selection::Swing(_) => "Swing".into(),
            Selection::Human(_) => "Human".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Bpm => format!("{:.0}", self.params.bpm.get()),
            Selection::Target(c) => crate::modbus::Patch::app_label(&self.modbus, self.params.channels[c].target.load(Ordering::Relaxed)),
            Selection::TargetInput(c) => crate::modbus::Patch::input_label(&self.modbus, self.params.channels[c].target.load(Ordering::Relaxed)),
            Selection::ClockMod(c) => {
                CLOCK_MODS[self.params.channels[c].clock_mod.load(Ordering::Relaxed) % CLOCK_MODS.len()].0.to_string()
            }
            Selection::Shape(c) => {
                SHAPE_NAMES[self.params.channels[c].shape.load(Ordering::Relaxed) as usize % SHAPE_NAMES.len()].to_string()
            }
            Selection::Width(c) => format!("{:.2}", self.params.channels[c].width.get()),
            Selection::Phase(c) => format!("{:.2}", self.params.channels[c].phase.get()),
            Selection::EuclidSteps(c) => format!("{}", self.params.channels[c].euclid_steps.load(Ordering::Relaxed)),
            Selection::EuclidPulses(c) => format!("{}", self.params.channels[c].euclid_pulses.load(Ordering::Relaxed)),
            Selection::EuclidRotation(c) => format!("{}", self.params.channels[c].euclid_rotation.load(Ordering::Relaxed)),
            Selection::Slew(c) => format!("{:.2}", self.params.channels[c].slew.get()),
            Selection::Level(c) => format!("{:.2}", self.params.channels[c].level.get()),
            Selection::Offset(c) => format!("{:.2}", self.params.channels[c].offset.get()),
            Selection::Invert(c) => {
                if self.params.channels[c].invert.load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::QuantizerOn(c) => {
                if self.params.channels[c].quantizer_on.load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::QuantizerScale(c) => {
                let idx = self.params.channels[c].quantizer_scale.load(Ordering::Relaxed) as usize % SCALE_TYPES.len();
                SCALE_TYPES[idx].0.to_string()
            }
            Selection::QuantizerRoot(c) => {
                ROOT_NAMES[self.params.channels[c].quantizer_root.load(Ordering::Relaxed) as usize % 12].to_string()
            }
            Selection::Swing(c) => format!("{:.2}", self.params.channels[c].swing.get()),
            Selection::Human(c) => format!("{:.2}", self.params.channels[c].human.get()),
        }
    }

    fn group_summary(&self, g: usize) -> String {
        if g == 0 {
            return format!("{:.0} BPM", self.params.bpm.get());
        }
        let c = g - 1;
        let shape = self.leaf_value(Selection::Shape(c));
        let target = self.target_name(self.params.channels[c].target.load(Ordering::Relaxed));
        format!("{shape} -> {target}")
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::Bpm => {
                let next = (self.params.bpm.get() + accelerate(delta) * sensitivity * 2.0).clamp(MIN_BPM, MAX_BPM);
                self.params.bpm.set(next);
            }
            Selection::Target(c) => self.params.channels[c].target.store(crate::modbus::Patch::step_app(&self.modbus, self.params.channels[c].target.load(Ordering::Relaxed), step), Ordering::Relaxed),
            Selection::TargetInput(c) => self.params.channels[c].target.store(crate::modbus::Patch::step_input(&self.modbus, self.params.channels[c].target.load(Ordering::Relaxed), step), Ordering::Relaxed),
            Selection::ClockMod(c) => {
                let cur = self.params.channels[c].clock_mod.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(CLOCK_MODS.len() as i32);
                self.params.channels[c].clock_mod.store(next as usize, Ordering::Relaxed);
            }
            Selection::Shape(c) => {
                let cur = self.params.channels[c].shape.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(SHAPE_NAMES.len() as i32);
                self.params.channels[c].shape.store(next as u32, Ordering::Relaxed);
            }
            Selection::Width(c) => bump(&self.params.channels[c].width, delta, sensitivity, 0.0, 1.0),
            Selection::Phase(c) => bump(&self.params.channels[c].phase, delta, sensitivity, 0.0, 1.0),
            Selection::EuclidSteps(c) => {
                let cur = self.params.channels[c].euclid_steps.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(1, MAX_EUCLID_STEPS as i32);
                self.params.channels[c].euclid_steps.store(next as u32, Ordering::Relaxed);
            }
            Selection::EuclidPulses(c) => {
                let steps = self.params.channels[c].euclid_steps.load(Ordering::Relaxed) as i32;
                let cur = self.params.channels[c].euclid_pulses.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(0, steps);
                self.params.channels[c].euclid_pulses.store(next as u32, Ordering::Relaxed);
            }
            Selection::EuclidRotation(c) => {
                let steps = self.params.channels[c].euclid_steps.load(Ordering::Relaxed).max(1) as i32;
                let cur = self.params.channels[c].euclid_rotation.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(steps);
                self.params.channels[c].euclid_rotation.store(next as u32, Ordering::Relaxed);
            }
            Selection::Slew(c) => bump(&self.params.channels[c].slew, delta, sensitivity, 0.0, 1.0),
            Selection::Level(c) => bump(&self.params.channels[c].level, delta, sensitivity, 0.0, 1.0),
            Selection::Offset(c) => bump(&self.params.channels[c].offset, delta, sensitivity, -1.0, 1.0),
            Selection::Invert(c) => self.params.channels[c].invert.store(delta > 0, Ordering::Relaxed),
            Selection::QuantizerOn(c) => self.params.channels[c].quantizer_on.store(delta > 0, Ordering::Relaxed),
            Selection::QuantizerScale(c) => {
                let cur = self.params.channels[c].quantizer_scale.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(SCALE_TYPES.len() as i32);
                self.params.channels[c].quantizer_scale.store(next as u32, Ordering::Relaxed);
            }
            Selection::QuantizerRoot(c) => {
                let cur = self.params.channels[c].quantizer_root.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(12);
                self.params.channels[c].quantizer_root.store(next as u32, Ordering::Relaxed);
            }
            Selection::Swing(c) => bump(&self.params.channels[c].swing, delta, sensitivity, 0.0, MAX_SWING),
            Selection::Human(c) => bump(&self.params.channels[c].human, delta, sensitivity, 0.0, MAX_HUMAN),
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Bpm => self.params.bpm.set(DEFAULT_BPM),
            Selection::Target(c) => self.params.channels[c].target.store(0, Ordering::Relaxed),
            Selection::TargetInput(c) => self.params.channels[c].target.store(crate::modbus::Patch::first_input(&self.modbus, self.params.channels[c].target.load(Ordering::Relaxed)), Ordering::Relaxed),
            Selection::ClockMod(c) => self.params.channels[c].clock_mod.store(3, Ordering::Relaxed),
            Selection::Width(c) => self.params.channels[c].width.set(0.5),
            Selection::Phase(c) => self.params.channels[c].phase.set(0.0),
            Selection::EuclidSteps(c) => self.params.channels[c].euclid_steps.store(16, Ordering::Relaxed),
            Selection::EuclidPulses(c) => self.params.channels[c].euclid_pulses.store(4, Ordering::Relaxed),
            Selection::EuclidRotation(c) => self.params.channels[c].euclid_rotation.store(0, Ordering::Relaxed),
            Selection::Slew(c) => self.params.channels[c].slew.set(0.0),
            Selection::Level(c) => self.params.channels[c].level.set(1.0),
            Selection::Offset(c) => self.params.channels[c].offset.set(0.0),
            Selection::Invert(c) => self.params.channels[c].invert.store(false, Ordering::Relaxed),
            Selection::QuantizerOn(c) => self.params.channels[c].quantizer_on.store(false, Ordering::Relaxed),
            Selection::QuantizerScale(c) => self.params.channels[c].quantizer_scale.store(0, Ordering::Relaxed),
            Selection::QuantizerRoot(c) => self.params.channels[c].quantizer_root.store(0, Ordering::Relaxed),
            Selection::Swing(c) => self.params.channels[c].swing.set(0.0),
            Selection::Human(c) => self.params.channels[c].human.set(0.0),
            Selection::Shape(_) => {} // no sensible single default
        }
    }
}

impl PamsApp {
    /// Real, windowed `(name, value, is_group)` rows -- mirrors this
    /// app's own `draw()` row-building, exposed for an alternate
    /// renderer (a live Slint screen) instead of drawn.
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

    /// `display_rows`, windowed to at most `visible` rows around the
    /// current selection -- see `ParamList::centered_scroll_window`. Returns
    /// `(window, selected_index_in_window, has_more_above,
    /// has_more_below)`.
    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }

    /// The currently-browsed channel's real CV monitor scope -- same
    /// data `draw()`'s own line-plot reads, exposed for an alternate
    /// renderer instead of drawn directly.
    pub(crate) fn channel_monitor(&self) -> crate::app::PamsExtra {
        let rows = self.visible_rows();
        let channel = self.current_channel(&rows);
        let history: Vec<f32> = self.params.channels[channel].monitor_history.lock().unwrap().iter().map(|v| v.clamp(-1.0, 1.0)).collect();
        let value = self.params.channels[channel].monitor.get();
        crate::app::PamsExtra { channel_index: channel, history, value }
    }
}

impl PamsApp {
    /// The menu row behind a play-view control -- label, value, edit and
    /// reset go through exactly the menu's code. `None` for the channel
    /// picker, which the menu does by cursor position instead.
    fn kit_selection(&self, i: usize) -> Option<Selection> {
        let c = self.last_channel;
        Some(match i {
            C_BPM => Selection::Bpm,
            C_LEVEL => Selection::Level(c),
            C_WIDTH => Selection::Width(c),
            C_OFFSET => Selection::Offset(c),
            C_SLEW => Selection::Slew(c),
            C_PHASE => Selection::Phase(c),
            C_SWING => Selection::Swing(c),
            C_HUMAN => Selection::Human(c),
            C_SHAPE => Selection::Shape(c),
            C_CLOCK_MOD => Selection::ClockMod(c),
            C_INVERT => Selection::Invert(c),
            C_QUANTIZER => Selection::QuantizerOn(c),
            C_EUCLID_STEPS => Selection::EuclidSteps(c),
            C_EUCLID_PULSES => Selection::EuclidPulses(c),
            C_EUCLID_ROTATION => Selection::EuclidRotation(c),
            _ => return None,
        })
    }

    /// Storage behind each control, with the ranges `edit` clamps to.
    /// Pulses and rotation are bounded by the channel's own step count,
    /// as in `edit`. Clock mod is a `usize` (no `Knob` variant), so
    /// `kit_norm`/`kit_set_norm` handle it by hand.
    fn knob(&self, i: usize) -> Knob<'_> {
        let p = &self.params;
        let ch = &p.channels[self.last_channel];
        let steps = ch.euclid_steps.load(Ordering::Relaxed).clamp(1, MAX_EUCLID_STEPS as u32);
        match i {
            C_BPM => Knob::F(&p.bpm, MIN_BPM, MAX_BPM),
            C_LEVEL => Knob::F(&ch.level, 0.0, 1.0),
            C_WIDTH => Knob::F(&ch.width, 0.0, 1.0),
            C_OFFSET => Knob::F(&ch.offset, -1.0, 1.0),
            C_SLEW => Knob::F(&ch.slew, 0.0, 1.0),
            C_PHASE => Knob::F(&ch.phase, 0.0, 1.0),
            C_SWING => Knob::F(&ch.swing, 0.0, MAX_SWING),
            C_HUMAN => Knob::F(&ch.human, 0.0, MAX_HUMAN),
            C_SHAPE => Knob::U(&ch.shape, SHAPE_NAMES.len() as u32),
            C_INVERT => Knob::B(&ch.invert),
            C_QUANTIZER => Knob::B(&ch.quantizer_on),
            C_EUCLID_STEPS => Knob::UR(&ch.euclid_steps, 1, MAX_EUCLID_STEPS as u32),
            C_EUCLID_PULSES => Knob::UR(&ch.euclid_pulses, 0, steps),
            C_EUCLID_ROTATION => Knob::UR(&ch.euclid_rotation, 0, steps - 1),
            _ => Knob::None,
        }
    }
}

impl PlayHost for PamsApp {
    fn kit_control_count(&self) -> usize {
        NUM_CONTROLS
    }
    fn kit_label(&self, i: usize) -> String {
        match i {
            C_BPM => "Clock".into(),
            C_CHANNEL => "Channel".into(),
            C_EUCLID_STEPS => "Eu Steps".into(),
            C_EUCLID_PULSES => "Eu Pulses".into(),
            C_EUCLID_ROTATION => "Eu Rotate".into(),
            _ => self.kit_selection(i).map(|s| self.leaf_name(s)).unwrap_or_default(),
        }
    }
    fn kit_value(&self, i: usize) -> String {
        if i == C_CHANNEL {
            let c = self.last_channel;
            return format!("{} {}", c + 1, self.leaf_value(Selection::Shape(c)));
        }
        self.kit_selection(i).map(|s| self.leaf_value(s)).unwrap_or_default()
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        match i {
            // Deliberately no position: recalling a moment shouldn't
            // change which channel the knobs are on.
            C_CHANNEL => None,
            C_CLOCK_MOD => Some(clock_mod_norm(self.params.channels[self.last_channel].clock_mod.load(Ordering::Relaxed) % CLOCK_MODS.len())),
            _ => self.knob(i).norm(),
        }
    }
    fn kit_stepped(&self, i: usize) -> bool {
        matches!(i, C_CHANNEL | C_CLOCK_MOD) || self.knob(i).stepped()
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        if i == C_CHANNEL {
            if delta != 0 {
                self.last_channel = (self.last_channel as i32 + delta.signum()).rem_euclid(NUM_CHANNELS as i32) as usize;
            }
            return;
        }
        if let Some(sel) = self.kit_selection(i) {
            self.edit(sel, delta);
        }
    }
    fn kit_reset(&mut self, i: usize) {
        if let Some(sel) = self.kit_selection(i) {
            self.reset(sel);
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i {
            C_CHANNEL => {}
            C_CLOCK_MOD => {
                let idx = (v * (CLOCK_MODS.len() - 1) as f32).round() as usize;
                self.params.channels[self.last_channel].clock_mod.store(idx.min(CLOCK_MODS.len() - 1), Ordering::Relaxed);
            }
            _ => self.knob(i).set(v),
        }
    }
    fn kit_line(&self) -> String {
        let c = self.last_channel;
        let ch = &self.params.channels[c];
        if !self.params.running.load(Ordering::Relaxed) {
            return format!("Ch{} stopped", c + 1);
        }
        format!("Ch{} {:+.2} {}", c + 1, ch.monitor.get(), CLOCK_MODS[ch.clock_mod.load(Ordering::Relaxed) % CLOCK_MODS.len()].0)
    }
}

impl App for PamsApp {
    fn play_surface(&self) -> bool { true }
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
    fn running(&self) -> Option<bool> { Some(self.params.running.load(Ordering::Relaxed)) }
    fn toggle_running(&mut self) {
        let was_running = self.params.running.load(Ordering::Relaxed);
        self.params.running.store(!was_running, Ordering::Relaxed);
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_scale_info(&self) -> Option<crate::app::music_scales::ScaleInfo> {
        match self.visible_rows().get(self.list.selected) { Some(Row::Leaf(Selection::QuantizerScale(c) | Selection::QuantizerRoot(c))) => {let p=&self.params.channels[*c];Some(crate::app::music_scales::ScaleInfo::new(p.quantizer_scale.load(Ordering::Relaxed) as usize,p.quantizer_root.load(Ordering::Relaxed) as i32))}, _ => None }
    }
    fn slint_selected(&self) -> usize {
        self.selected_row()
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        self.windowed_rows(visible)
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        crate::app::SlintExtra::Pams(self.channel_monitor())
    }

    fn tick(&mut self, input: &Input) {
        // The play view takes the knobs and D-pad first; in the menu they
        // pass straight through. The pads only ever reach the kit.
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

        self.last_channel = self.current_channel(&rows);
        // No grid usage -- channels are clocked/derived generators, not
        // a hand-toggled step pattern.
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(PamsProcessor {
            params: Arc::clone(&self.params),
            modbus: Arc::clone(&self.modbus),
            runtimes: std::array::from_fn(|i| ChannelRuntime::new(i as u32)),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(PAMS_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, PAMS_TITLE);
        Text::new("Pam's Workout", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, PAMS_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, PAMS_DIM);

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
            self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, PAMS_BG, PAMS_DIM, PAMS_ACCENT);
        } else if let Some(col) = self.play_column() {
            // Stops short of the monitor panel at x=400.
            let pal = kit::draw::Palette { bg: PAMS_BG, ink: PAMS_TITLE, accent: PAMS_ACCENT, dim: PAMS_DIM, faint: PAMS_MIDLINE };
            kit::draw::column(fb, &col, 16, 40, 370, 285, pal);
        }

        // --- Right: the current channel's live output, scrolling --
        // "CV Monitoring", same line-plot technique as Plaits' mod panel.
        let channel = self.current_channel(&rows);
        let panel_x = 400;
        let panel_y = 44;
        let panel_w = 220;
        let panel_h = 250;
        Text::new(&format!("Channel {} monitor", channel + 1), Point::new(panel_x, panel_y - 4), accent)
            .draw(fb)
            .ok();

        let mid_y = panel_y + panel_h / 2;
        Line::new(Point::new(panel_x, mid_y), Point::new(panel_x + panel_w, mid_y))
            .into_styled(PrimitiveStyle::with_stroke(PAMS_MIDLINE, 1))
            .draw(fb)
            .ok();

        let history = self.params.channels[channel].monitor_history.lock().unwrap();
        if history.len() >= 2 {
            let step = panel_w as f32 / (history.len() - 1) as f32;
            let style = PrimitiveStyle::with_stroke(PAMS_ACCENT, 1);
            for i in 0..history.len() - 1 {
                let v0 = history[i].clamp(-1.0, 1.0);
                let v1 = history[i + 1].clamp(-1.0, 1.0);
                let p0 = Point::new(panel_x + (i as f32 * step) as i32, mid_y - (v0 * panel_h as f32 / 2.0) as i32);
                let p1 =
                    Point::new(panel_x + ((i + 1) as f32 * step) as i32, mid_y - (v1 * panel_h as f32 / 2.0) as i32);
                Line::new(p0, p1).into_styled(style).draw(fb).ok();
            }
        }
        drop(history);
        Text::new(
            &format!("value: {:.2}", self.params.channels[channel].monitor.get()),
            Point::new(panel_x, panel_y + panel_h + 12),
            dim,
        )
        .draw(fb)
        .ok();

        let hint = match rows.get(self.list.selected) {
            _ if !self.kit.menu => "knobs: dials   D-pad: channel   F2: pads   F3: clock run/stop   R1: menu".to_string(),
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

struct ChannelRuntime {
    phase: f32,
    euclid_step: usize,
    sh_value: f32,
    slew_state: f32,
    cycle_count: u32,
    rng: u32,
}

impl ChannelRuntime {
    fn new(seed: u32) -> Self {
        Self {
            phase: 0.0,
            euclid_step: 0,
            sh_value: 0.0,
            slew_state: 0.0,
            cycle_count: 0,
            rng: 0x9E3779B9 ^ (seed.wrapping_mul(0x85EBCA6B) | 1),
        }
    }

    fn next_rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

struct PamsProcessor {
    params: Arc<Params>,
    modbus: Arc<ModBus>,
    runtimes: [ChannelRuntime; NUM_CHANNELS],
}

impl AudioProcessor for PamsProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        // Pam's makes no sound of its own -- it only writes into other
        // apps' modulation inputs (see modbus.rs). Silence here is
        // correct, not a bug: the mix bus just adds zero.
        for out in buffer.iter_mut() {
            *out = 0.0;
        }

        if !self.params.running.load(Ordering::Relaxed) {
            for channel in &self.params.channels {
                channel.monitor.set(0.0);
                let target = channel.target.load(Ordering::Relaxed);
                if target > 0 { if let Some(value) = self.modbus.get(target - 1) { value.set(0.0); } }
            }
            return;
        }

        let frames = (buffer.len() / channels) as f32;
        let dt = frames / sample_rate;
        let bpm = self.params.bpm.get().max(1.0);
        let base_hz = bpm / 60.0; // one quarter note per beat

        for c in 0..NUM_CHANNELS {
            let ch = &self.params.channels[c];
            let rt = &mut self.runtimes[c];
            let shape = ch.shape.load(Ordering::Relaxed);
            let clock_mod = CLOCK_MODS[ch.clock_mod.load(Ordering::Relaxed) % CLOCK_MODS.len()].1;
            let swing = ch.swing.get().clamp(0.0, MAX_SWING);
            let human = ch.human.get().clamp(0.0, MAX_HUMAN);

            // Swing/Human both perturb the *rate*, not the value --
            // swing alternates every cycle so the pair's total duration
            // (and therefore the average rate) stays correct; human
            // adds a small one-off random nudge each cycle for a
            // "not perfectly locked" feel.
            let swing_mult = if rt.cycle_count % 2 == 1 { 1.0 + swing } else { 1.0 - swing };
            let rate_hz = (base_hz * clock_mod * swing_mult).max(0.001);

            let prev_phase = rt.phase;
            rt.phase = (rt.phase + rate_hz * dt).fract();
            let wrapped = rt.phase < prev_phase;
            if wrapped {
                rt.cycle_count = rt.cycle_count.wrapping_add(1);
                if human > 0.0 {
                    rt.phase = (rt.phase + rt.next_rand() * human * 0.05).rem_euclid(1.0);
                }
                if shape == SHAPE_EUCLIDEAN {
                    let steps = ch.euclid_steps.load(Ordering::Relaxed).max(1) as usize;
                    rt.euclid_step = (rt.euclid_step + 1) % steps;
                } else if shape == SHAPE_SAMPLE_HOLD {
                    rt.sh_value = rt.next_rand();
                }
            }

            let phase_shifted = (rt.phase + ch.phase.get()).rem_euclid(1.0);
            let width = ch.width.get();
            let raw = match shape {
                s if s == SHAPE_EUCLIDEAN => {
                    let steps = ch.euclid_steps.load(Ordering::Relaxed).max(1) as usize;
                    let pulses = ch.euclid_pulses.load(Ordering::Relaxed) as usize;
                    let rotation = ch.euclid_rotation.load(Ordering::Relaxed) as usize;
                    let mut pattern = [false; MAX_EUCLID_STEPS];
                    euclidean_pattern(steps, pulses, rotation, &mut pattern);
                    if pattern[rt.euclid_step % steps] { 1.0 } else { -1.0 }
                }
                s if s == SHAPE_SAMPLE_HOLD => rt.sh_value,
                _ => shape_value(shape, phase_shifted, width),
            };

            let raw = if ch.invert.load(Ordering::Relaxed) { -raw } else { raw };

            let slew = ch.slew.get();
            let slew_coef = 1.0 - (-dt / (0.001 + slew * 0.3)).exp();
            rt.slew_state += (raw - rt.slew_state) * slew_coef;

            let mut value = (rt.slew_state * ch.level.get() + ch.offset.get()).clamp(-1.0, 1.0);
            if ch.quantizer_on.load(Ordering::Relaxed) {
                let scale_idx = ch.quantizer_scale.load(Ordering::Relaxed) as usize;
                let root = ch.quantizer_root.load(Ordering::Relaxed);
                value = quantize(value, scale_idx, root);
            }

            ch.monitor.set(value);
            {
                let mut hist = ch.monitor_history.lock().unwrap();
                hist.push_back(value);
                if hist.len() > MONITOR_LEN {
                    hist.pop_front();
                }
            }

            let target = ch.target.load(Ordering::Relaxed);
            if target > 0 {
                if let Some(handle) = self.modbus.get(target - 1) {
                    handle.set(value);
                }
            }
        }
    }
}

#[cfg(test)]
mod transport_contract_tests {
    use super::*;
    #[test]
    fn clock_stop_silences_routes_and_play_resumes() {
        let bus = Arc::new(ModBus::new());
        let target = bus.register("test: clock");
        let mut app = PamsApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), bus);
        app.params.channels[0].target.store(1, Ordering::Relaxed);
        let mut processor = app.audio_processor().unwrap();
        let mut audio = [0.0; 1024];
        processor.process(&mut audio, 2, 48000.0);
        assert_eq!(app.transport_action(), Some("STOP"));
        app.toggle_running(); target.set(1.0);
        processor.process(&mut audio, 2, 48000.0);
        assert_eq!(target.get(), 0.0);
        assert_eq!(app.transport_action(), Some("PLAY"));
        app.toggle_running();
        processor.process(&mut audio, 2, 48000.0);
        assert_eq!(app.running(), Some(true));
        assert_ne!(target.get(), 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PamsApp {
        PamsApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(1.0)), Arc::new(ModBus::new()))
    }

    fn pad(rank: usize) -> Input {
        Input { grid: std::array::from_fn(|k| k == crate::app::play_kit::rank_pad(rank)), ..Default::default() }
    }

    #[test]
    fn opens_on_the_play_view_and_knob_1_turns_the_clock() {
        let mut a = app();
        assert!(a.play_column().is_some(), "play view first");
        let bpm = a.params.bpm.get();
        a.tick(&Input { knob1: 3, ..Default::default() });
        assert!(a.params.bpm.get() > bpm, "knob 1 is the master clock");
        a.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(a.play_column().is_none(), "R1 opens the full menu");
    }

    #[test]
    fn d_pad_picks_the_channel_the_knobs_shape() {
        let mut a = app();
        a.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(a.last_channel, 1, "D-pad up = next channel");
        a.tick(&Input { knob2: -3, ..Default::default() });
        assert!(a.params.channels[1].level.get() < 1.0, "knob 2 is the selected channel's level");
        assert_eq!(a.params.channels[0].level.get(), 1.0, "other channels untouched");
    }

    #[test]
    fn throws_ratchet_the_clock_and_spring_back() {
        let mut a = app();
        assert_eq!(a.grid_mode_label(), Some("THROWS"));
        assert_eq!(a.params.channels[0].clock_mod.load(Ordering::Relaxed), 3, "x1");
        a.tick(&pad(0));
        assert_eq!(CLOCK_MODS[a.params.channels[0].clock_mod.load(Ordering::Relaxed)].0, "x4", "held throw ratchets");
        a.tick(&Input::default());
        assert_eq!(a.params.channels[0].clock_mod.load(Ordering::Relaxed), 3, "and is put back exactly");
        a.tick(&pad(7));
        assert_eq!(a.params.channels[0].level.get(), 0.0, "MUTE holds the level at zero");
        a.tick(&Input::default());
        assert!((a.params.channels[0].level.get() - 1.0).abs() < 1e-5, "and springs back");
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(PamsApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get()))
}
