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

use crate::app::{App, Input};
use crate::apps::plaits::{ROOT_NAMES, SCALE_TYPES};
use crate::audio::AudioProcessor;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
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
        match rows.get(self.list.selected) {
            Some(Row::Group(g)) if *g >= 1 => *g - 1,
            Some(Row::Leaf(sel)) => Self::selection_channel(*sel).unwrap_or(self.last_channel),
            _ => self.last_channel,
        }
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
            Selection::Bpm => "BPM".into(),
            Selection::Target(_) => "Target".into(),
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
            Selection::QuantizerScale(_) => "Q. Scale".into(),
            Selection::QuantizerRoot(_) => "Q. Root".into(),
            Selection::Swing(_) => "Swing".into(),
            Selection::Human(_) => "Human".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Bpm => format!("{:.0}", self.params.bpm.get()),
            Selection::Target(c) => self.target_name(self.params.channels[c].target.load(Ordering::Relaxed)),
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
            Selection::Target(c) => {
                let n = self.modbus.len();
                let cur = self.params.channels[c].target.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(n as i32 + 1);
                self.params.channels[c].target.store(next as usize, Ordering::Relaxed);
            }
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

impl App for PamsApp {
    fn running(&self) -> Option<bool> { Some(self.params.running.load(Ordering::Relaxed)) }
    fn toggle_running(&mut self) {
        let was_running = self.params.running.load(Ordering::Relaxed);
        self.params.running.store(!was_running, Ordering::Relaxed);
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
        crate::app::SlintExtra::Pams(self.channel_monitor())
    }

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
        self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, PAMS_BG, PAMS_DIM, PAMS_ACCENT);

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
