//! Originally built as "Bloom", in the spirit of Mario Nieto's Harmony
//! Bloom -- renamed to Madness once it became clear this mechanic
//! (independent per-position phase drift warping a polygon) isn't
//! actually what Harmony Bloom does. The real Bloom app (see bloom.rs)
//! replaces it with the correct mechanic: fixed-radius arms pivoting
//! from a center, each bendable into a spiral/wave. This file's
//! generative core is still worth keeping in its own right, just under
//! an honest name: each note position has its own independent phase
//! accumulator (`phases`), advancing at
//! `base_speed * (1 + speed_offset * distance_from_center)`. At
//! `speed_offset == 0` every position moves together (a rigid rotating
//! polygon); away from 0, positions nearer the middle index and
//! positions nearer the ends drift apart at different rates,
//! continuously bending the connecting lines into evolving
//! spirograph-like shapes -- and since 8 fixed trigger bars sit at
//! constant angles while every position sweeps past them at its own
//! slowly-diverging rate, which note fires at which bar keeps changing
//! over time too.
//!
//! 8 independent shape instances, each with its own notes/scale/bars/
//! voice, all mixing together. A shared Master Clock BPM (Global
//! group) is what "Tempo Sync" locks to, separate from each shape's
//! own free-running Speed. Each shape has its own Running (start/stop)
//! switch -- off by default, deliberately first in that shape's tree,
//! since nothing here should be audibly active until you turn it on --
//! and a Randomize action (press knob2 on that leaf) that reassigns
//! most of its parameters at once.
//!
//! Each shape plays a real Plaits voice (any of the 24 engines) and
//! exposes Speed/Level to the shared ModBus, same as Pam's.

use crate::app::{App, Input};
use crate::apps::plaits::{ENGINE_NAMES, ROOT_NAMES, SCALE_TYPES};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::plaits_ffi::{PlaitsParams, PlaitsVoice};
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const NUM_SHAPES: usize = 8;
const NUM_BARS: usize = 8;
const MIN_NOTES: usize = 2;
const MAX_NOTES: usize = 32;
const DEFAULT_NOTES: usize = 8;
const MIN_SPEED_HZ: f32 = 0.02;
const MAX_SPEED_HZ: f32 = 4.0;
const DEFAULT_SPEED_HZ: f32 = 0.25;
const MIN_BPM: f32 = 40.0;
const MAX_BPM: f32 = 300.0;
const DEFAULT_BPM: f32 = 120.0;
const DIRECTION_NAMES: [&str; 3] = ["Forward", "Reverse", "Ping-Pong"];
const CLOCK_MODS: [(&str, f32); 9] =
    [("/8", 0.125), ("/4", 0.25), ("/2", 0.5), ("x1", 1.0), ("x2", 2.0), ("x3", 3.0), ("x4", 4.0), ("x8", 8.0), ("x16", 16.0)];

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * (max - min) * 0.05).clamp(min, max);
    value.set(next);
}

fn note_for_position(pos: usize, scale_idx: usize, root: u32) -> i32 {
    let intervals = SCALE_TYPES[scale_idx % SCALE_TYPES.len()].1;
    let len = intervals.len().max(1);
    let degree = pos % len;
    let octave = pos / len;
    60 + root as i32 + intervals[degree] + octave as i32 * 12
}

/// Maps a raw, unbounded, monotonically-accumulating phase to the
/// actual 0..1 angle used for *drawing* -- Forward/Reverse just wrap,
/// Ping-Pong triangle-wraps (bounces at 0 and 1). Only used for visual
/// placement; crossing detection uses the raw value directly (see
/// `crosses`), not this -- see its doc comment for why.
fn display_phase(raw: f32, direction: u32) -> f32 {
    if direction == 2 {
        let m = raw.rem_euclid(2.0);
        if m <= 1.0 {
            m
        } else {
            2.0 - m
        }
    } else {
        raw.rem_euclid(1.0)
    }
}

/// Whether the continuously-swept **raw** (unwrapped) interval from
/// `prev_raw` to `new_raw` passes through any point congruent to
/// `target` modulo 1 turn.
///
/// This replaces an earlier, buggy approach (independently wrapping
/// each endpoint to 0..1 with `rem_euclid` via `display_phase`, then
/// taking their plain min/max as the swept interval): whenever a
/// block's sweep itself crossed the 0/1 wrap point, that produced the
/// *complementary* arc -- silently reporting false crossings for
/// targets in the short arc between the two wrapped endpoints, and
/// missing real crossings for targets in the long arc that was
/// actually swept. Working in raw (never-wrapped) space and checking
/// every `target + k` near the current window sidesteps that
/// entirely, however many full turns have already accumulated.
///
/// Not adjusted for Ping-Pong's bounce (the displayed angle isn't a
/// linear function of the raw accumulator there) -- exact for
/// Forward/Reverse, only approximate right at a Ping-Pong bounce
/// instant.
fn crosses(prev_raw: f32, new_raw: f32, target: f32) -> bool {
    let lo = prev_raw.min(new_raw);
    let hi = prev_raw.max(new_raw);
    if hi - lo > 3.0 {
        return true; // pathologically large jump this block -- just fire
    }
    let start_k = (lo - target).floor() - 1.0;
    for i in 0..5 {
        let t = target + start_k + i as f32;
        if t > lo && t <= hi {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    MasterBpm,
    Notes(usize),
    Scale(usize),
    Root(usize),
    Direction(usize),
    Running(usize),
    TempoSync(usize),
    Speed(usize),
    ClockMod(usize),
    SpeedOffset(usize),
    Bar(usize, usize),
    Probability(usize),
    VelMin(usize),
    VelMax(usize),
    Engine(usize),
    Harmonics(usize),
    Timbre(usize),
    Decay(usize),
    Randomize(usize),
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 1 + NUM_SHAPES;

struct ShapeParams {
    notes: AtomicUsize,
    scale: AtomicU32,
    root: AtomicU32,
    direction: AtomicU32,
    running: AtomicBool,
    tempo_sync: AtomicBool,
    speed_hz: AtomicF32,
    clock_mod: AtomicUsize,
    /// -1..1 -- how much faster/slower positions away from the middle
    /// index drift relative to the middle. 0 = rigid rotation.
    speed_offset: AtomicF32,
    bars_active: [AtomicBool; NUM_BARS],
    probability: AtomicF32,
    vel_min: AtomicF32,
    vel_max: AtomicF32,
    engine: AtomicU32,
    harmonics: AtomicF32,
    timbre: AtomicF32,
    decay: AtomicF32,
    /// Raw (unwrapped) per-position phase accumulators -- see
    /// `display_phase`. Written by the audio thread, read by the UI to
    /// draw each note's live position.
    phases: [AtomicF32; MAX_NOTES],
    last_fired: AtomicUsize,
    ext_speed: Arc<AtomicF32>,
    ext_level: Arc<AtomicF32>,
}

impl ShapeParams {
    fn new(shape_num: usize, modbus: &ModBus) -> Self {
        Self {
            notes: AtomicUsize::new(DEFAULT_NOTES),
            scale: AtomicU32::new(0),
            root: AtomicU32::new(0),
            direction: AtomicU32::new(0),
            running: AtomicBool::new(false),
            tempo_sync: AtomicBool::new(false),
            speed_hz: AtomicF32::new(DEFAULT_SPEED_HZ),
            clock_mod: AtomicUsize::new(3),
            speed_offset: AtomicF32::new(0.0),
            bars_active: std::array::from_fn(|i| AtomicBool::new(i % 2 == 0)),
            probability: AtomicF32::new(1.0),
            vel_min: AtomicF32::new(0.6),
            vel_max: AtomicF32::new(1.0),
            engine: AtomicU32::new(8),
            harmonics: AtomicF32::new(0.5),
            timbre: AtomicF32::new(0.5),
            decay: AtomicF32::new(0.4),
            phases: std::array::from_fn(|i| AtomicF32::new(i as f32 / DEFAULT_NOTES as f32)),
            last_fired: AtomicUsize::new(0),
            ext_speed: modbus.register(format!("Madness {shape_num}: Speed")),
            ext_level: modbus.register(format!("Madness {shape_num}: Level")),
        }
    }
}

struct Params {
    master_bpm: AtomicF32,
    shapes: [ShapeParams; NUM_SHAPES],
    /// This app's rendered mono output, republished every block for
    /// another app (Clouds) to tap -- see audio_bus.rs.
    bus_out: Arc<Mutex<Vec<f32>>>,
    /// This app's channel fader in the Mixer app, plus its own
    /// modulation input -- see mixer_bus.rs.
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Madness", modbus);
        Self {
            master_bpm: AtomicF32::new(DEFAULT_BPM),
            shapes: std::array::from_fn(|i| ShapeParams::new(i + 1, modbus)),
            bus_out: audio_bus.register("Madness"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct MadnessApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    last_shape: usize,
    rng: u32,
}

// --- Madness's own palette: hot magenta clashing against acid
// chartreuse on near-black, not a device-wide theme -- a deliberately
// jarring, hypnotic clash matching this app's own name and its
// warping, never-quite-rigid polygon. ---

const MADNESS_BG: Rgb565 = Rgb565::new(1, 1, 1);
const MADNESS_TITLE: Rgb565 = Rgb565::new(31, 56, 30);
const MADNESS_ACCENT: Rgb565 = Rgb565::new(31, 11, 18);
const MADNESS_DIM: Rgb565 = Rgb565::new(15, 18, 12);
const MADNESS_CLASH: Rgb565 = Rgb565::new(26, 63, 7);
const MADNESS_LINE: Rgb565 = Rgb565::new(5, 5, 5);
const MADNESS_NOTE_OFF: Rgb565 = Rgb565::new(6, 5, 6);
const MADNESS_BAR_OFF: Rgb565 = Rgb565::new(6, 5, 6);

impl MadnessApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(12345);
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
            last_shape: 0,
            rng: seed | 1,
        }
    }

    fn next_rand01(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        if g == 0 {
            return vec![Selection::MasterBpm];
        }
        let s = g - 1;
        // Running comes first -- the master on/off for this shape,
        // deliberately the first thing you see entering its tree.
        // The Bar toggles come right after Notes -- the geometry of
        // this shape (how many notes, which trigger bars are active)
        // grouped together, before moving on to Scale/Root/Speed --
        // same position Bloom's analogous OuterDots+OuterActive list
        // sits in relative to its own Dots control.
        let mut v = vec![Selection::Running(s), Selection::Notes(s)];
        for b in 0..NUM_BARS {
            v.push(Selection::Bar(s, b));
        }
        v.push(Selection::Scale(s));
        v.push(Selection::Root(s));
        v.push(Selection::Direction(s));
        v.push(Selection::TempoSync(s));
        if self.params.shapes[s].tempo_sync.load(Ordering::Relaxed) {
            v.push(Selection::ClockMod(s));
        } else {
            v.push(Selection::Speed(s));
        }
        v.push(Selection::SpeedOffset(s));
        v.push(Selection::Probability(s));
        v.push(Selection::VelMin(s));
        v.push(Selection::VelMax(s));
        v.push(Selection::Engine(s));
        v.push(Selection::Harmonics(s));
        v.push(Selection::Timbre(s));
        v.push(Selection::Decay(s));
        v.push(Selection::Randomize(s));
        v
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

    fn selection_shape(sel: Selection) -> Option<usize> {
        match sel {
            Selection::MasterBpm => None,
            Selection::Notes(s)
            | Selection::Scale(s)
            | Selection::Root(s)
            | Selection::Direction(s)
            | Selection::Running(s)
            | Selection::TempoSync(s)
            | Selection::Speed(s)
            | Selection::ClockMod(s)
            | Selection::SpeedOffset(s)
            | Selection::Bar(s, _)
            | Selection::Probability(s)
            | Selection::VelMin(s)
            | Selection::VelMax(s)
            | Selection::Engine(s)
            | Selection::Harmonics(s)
            | Selection::Timbre(s)
            | Selection::Decay(s)
            | Selection::Randomize(s) => Some(s),
        }
    }

    fn current_shape(&self, rows: &[Row]) -> usize {
        match rows.get(self.list.selected) {
            Some(Row::Group(g)) if *g >= 1 => *g - 1,
            Some(Row::Leaf(sel)) => Self::selection_shape(*sel).unwrap_or(self.last_shape),
            _ => self.last_shape,
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::MasterBpm => "BPM".into(),
            Selection::Notes(_) => "Notes".into(),
            Selection::Scale(_) => "Scale".into(),
            Selection::Root(_) => "Root".into(),
            Selection::Direction(_) => "Direction".into(),
            Selection::Running(_) => "Running".into(),
            Selection::TempoSync(_) => "Tempo Sync".into(),
            Selection::Speed(_) => "Speed".into(),
            Selection::ClockMod(_) => "Clock Mod".into(),
            Selection::SpeedOffset(_) => "Speed Offset".into(),
            Selection::Bar(_, b) => format!("Bar {}", b + 1),
            Selection::Probability(_) => "Probability".into(),
            Selection::VelMin(_) => "Vel Min".into(),
            Selection::VelMax(_) => "Vel Max".into(),
            Selection::Engine(_) => "Engine".into(),
            Selection::Harmonics(_) => "Harmonics".into(),
            Selection::Timbre(_) => "Timbre".into(),
            Selection::Decay(_) => "Decay".into(),
            Selection::Randomize(_) => "Randomize".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::MasterBpm => format!("{:.0}", self.params.master_bpm.get()),
            Selection::Notes(s) => format!("{}", self.params.shapes[s].notes.load(Ordering::Relaxed)),
            Selection::Scale(s) => {
                let idx = self.params.shapes[s].scale.load(Ordering::Relaxed) as usize % SCALE_TYPES.len();
                SCALE_TYPES[idx].0.to_string()
            }
            Selection::Root(s) => ROOT_NAMES[self.params.shapes[s].root.load(Ordering::Relaxed) as usize % 12].to_string(),
            Selection::Direction(s) => {
                let idx = self.params.shapes[s].direction.load(Ordering::Relaxed) as usize % DIRECTION_NAMES.len();
                DIRECTION_NAMES[idx].to_string()
            }
            Selection::Running(s) => {
                if self.params.shapes[s].running.load(Ordering::Relaxed) { "running".into() } else { "stopped".into() }
            }
            Selection::TempoSync(s) => {
                if self.params.shapes[s].tempo_sync.load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::Speed(s) => format!("{:.2} Hz", self.params.shapes[s].speed_hz.get()),
            Selection::ClockMod(s) => {
                CLOCK_MODS[self.params.shapes[s].clock_mod.load(Ordering::Relaxed) % CLOCK_MODS.len()].0.to_string()
            }
            Selection::SpeedOffset(s) => format!("{:+.2}", self.params.shapes[s].speed_offset.get()),
            Selection::Bar(s, b) => {
                if self.params.shapes[s].bars_active[b].load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::Probability(s) => format!("{:.0}%", self.params.shapes[s].probability.get() * 100.0),
            Selection::VelMin(s) => format!("{:.2}", self.params.shapes[s].vel_min.get()),
            Selection::VelMax(s) => format!("{:.2}", self.params.shapes[s].vel_max.get()),
            Selection::Engine(s) => ENGINE_NAMES[self.params.shapes[s].engine.load(Ordering::Relaxed) as usize % 24].to_string(),
            Selection::Harmonics(s) => format!("{:.2}", self.params.shapes[s].harmonics.get()),
            Selection::Timbre(s) => format!("{:.2}", self.params.shapes[s].timbre.get()),
            Selection::Decay(s) => format!("{:.2}", self.params.shapes[s].decay.get()),
            Selection::Randomize(_) => "press knob2".into(),
        }
    }

    fn group_summary(&self, g: usize) -> String {
        if g == 0 {
            return format!("{:.0} BPM", self.params.master_bpm.get());
        }
        let s = g - 1;
        let running = if self.params.shapes[s].running.load(Ordering::Relaxed) { "" } else { " (stopped)" };
        format!("{} notes{}", self.params.shapes[s].notes.load(Ordering::Relaxed), running)
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::MasterBpm => {
                let next = (self.params.master_bpm.get() + accelerate(delta) * sensitivity * 2.0).clamp(MIN_BPM, MAX_BPM);
                self.params.master_bpm.set(next);
            }
            Selection::Notes(s) => {
                let cur = self.params.shapes[s].notes.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_NOTES as i32, MAX_NOTES as i32);
                self.params.shapes[s].notes.store(next as usize, Ordering::Relaxed);
            }
            Selection::Scale(s) => {
                let cur = self.params.shapes[s].scale.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(SCALE_TYPES.len() as i32);
                self.params.shapes[s].scale.store(next as u32, Ordering::Relaxed);
            }
            Selection::Root(s) => {
                let cur = self.params.shapes[s].root.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(12);
                self.params.shapes[s].root.store(next as u32, Ordering::Relaxed);
            }
            Selection::Direction(s) => {
                let cur = self.params.shapes[s].direction.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(DIRECTION_NAMES.len() as i32);
                self.params.shapes[s].direction.store(next as u32, Ordering::Relaxed);
            }
            Selection::Running(s) => self.params.shapes[s].running.store(delta > 0, Ordering::Relaxed),
            Selection::TempoSync(s) => self.params.shapes[s].tempo_sync.store(delta > 0, Ordering::Relaxed),
            Selection::Speed(s) => bump(&self.params.shapes[s].speed_hz, delta, sensitivity, MIN_SPEED_HZ, MAX_SPEED_HZ),
            Selection::ClockMod(s) => {
                let cur = self.params.shapes[s].clock_mod.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(CLOCK_MODS.len() as i32);
                self.params.shapes[s].clock_mod.store(next as usize, Ordering::Relaxed);
            }
            Selection::SpeedOffset(s) => bump(&self.params.shapes[s].speed_offset, delta, sensitivity, -1.0, 1.0),
            Selection::Bar(s, b) => self.params.shapes[s].bars_active[b].store(delta > 0, Ordering::Relaxed),
            Selection::Probability(s) => bump(&self.params.shapes[s].probability, delta, sensitivity, 0.0, 1.0),
            Selection::VelMin(s) => bump(&self.params.shapes[s].vel_min, delta, sensitivity, 0.0, 1.0),
            Selection::VelMax(s) => bump(&self.params.shapes[s].vel_max, delta, sensitivity, 0.0, 1.0),
            Selection::Engine(s) => {
                let cur = self.params.shapes[s].engine.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(24);
                self.params.shapes[s].engine.store(next as u32, Ordering::Relaxed);
            }
            Selection::Harmonics(s) => bump(&self.params.shapes[s].harmonics, delta, sensitivity, 0.0, 1.0),
            Selection::Timbre(s) => bump(&self.params.shapes[s].timbre, delta, sensitivity, 0.0, 1.0),
            Selection::Decay(s) => bump(&self.params.shapes[s].decay, delta, sensitivity, 0.0, 1.0),
            Selection::Randomize(_) => {} // action only fires on press -- see reset()
        }
    }

    /// Doubles as the "press to act" handler: a plain reset for most
    /// leaves, and the Randomize action for that one leaf.
    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::MasterBpm => self.params.master_bpm.set(DEFAULT_BPM),
            Selection::Notes(s) => self.params.shapes[s].notes.store(DEFAULT_NOTES, Ordering::Relaxed),
            Selection::Speed(s) => self.params.shapes[s].speed_hz.set(DEFAULT_SPEED_HZ),
            Selection::ClockMod(s) => self.params.shapes[s].clock_mod.store(3, Ordering::Relaxed),
            Selection::SpeedOffset(s) => self.params.shapes[s].speed_offset.set(0.0),
            Selection::Probability(s) => self.params.shapes[s].probability.set(1.0),
            Selection::VelMin(s) => self.params.shapes[s].vel_min.set(0.6),
            Selection::VelMax(s) => self.params.shapes[s].vel_max.set(1.0),
            Selection::Harmonics(s) => self.params.shapes[s].harmonics.set(0.5),
            Selection::Timbre(s) => self.params.shapes[s].timbre.set(0.5),
            Selection::Decay(s) => self.params.shapes[s].decay.set(0.4),
            Selection::Randomize(s) => self.randomize(s),
            Selection::Scale(_)
            | Selection::Root(_)
            | Selection::Direction(_)
            | Selection::Running(_)
            | Selection::TempoSync(_)
            | Selection::Bar(_, _)
            | Selection::Engine(_) => {} // no sensible single default
        }
    }

    /// "Global Randomization" -- reassigns most of one shape's
    /// parameters at once for quick exploration.
    fn randomize(&mut self, s: usize) {
        // Each random value is drawn into a local first -- `self` can't
        // be borrowed mutably (for `next_rand01`) while also borrowed
        // immutably to reach the atomic being stored into, within one
        // statement.
        let notes = MIN_NOTES + (self.next_rand01() * (MAX_NOTES - MIN_NOTES) as f32) as usize;
        let scale = (self.next_rand01() * SCALE_TYPES.len() as f32) as u32;
        let root = (self.next_rand01() * 12.0) as u32;
        let direction = (self.next_rand01() * DIRECTION_NAMES.len() as f32) as u32;
        let speed_hz = MIN_SPEED_HZ + self.next_rand01() * (MAX_SPEED_HZ - MIN_SPEED_HZ);
        let clock_mod = (self.next_rand01() * CLOCK_MODS.len() as f32) as usize;
        let speed_offset = self.next_rand01() * 2.0 - 1.0;
        let bars: [bool; NUM_BARS] = std::array::from_fn(|_| self.next_rand01() < 0.5);
        let probability = 0.4 + self.next_rand01() * 0.6;
        let v0 = self.next_rand01();
        let v1 = self.next_rand01();
        let engine = (self.next_rand01() * 24.0) as u32;
        let harmonics = self.next_rand01();
        let timbre = self.next_rand01();
        let decay = self.next_rand01();

        let sp = &self.params.shapes[s];
        sp.notes.store(notes, Ordering::Relaxed);
        sp.scale.store(scale, Ordering::Relaxed);
        sp.root.store(root, Ordering::Relaxed);
        sp.direction.store(direction, Ordering::Relaxed);
        sp.speed_hz.set(speed_hz);
        sp.clock_mod.store(clock_mod, Ordering::Relaxed);
        sp.speed_offset.set(speed_offset);
        for (b, on) in bars.iter().enumerate() {
            sp.bars_active[b].store(*on, Ordering::Relaxed);
        }
        sp.probability.set(probability);
        sp.vel_min.set(v0.min(v1));
        sp.vel_max.set(v0.max(v1).max(v0.min(v1) + 0.05));
        sp.engine.store(engine, Ordering::Relaxed);
        sp.harmonics.set(harmonics);
        sp.timbre.set(timbre);
        sp.decay.set(decay);
    }
}

impl MadnessApp {
    /// Real, windowed `(name, value, is_group)` rows -- mirrors this
    /// app's own `draw()` row-building, exposed for an alternate
    /// renderer (a live Slint screen) instead of drawn.
    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    let name = if *g == 0 { "Master Clock".to_string() } else { format!("Shape {g}") };
                    (format!("{arrow} {name}"), self.group_summary(*g), true)
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

    /// The real one-ring note polygon + 8 fixed trigger-bar markers --
    /// same data `draw()`'s own circle sketch reads, exposed as plain
    /// center-relative coordinates for an alternate renderer instead
    /// of drawn directly.
    pub(crate) fn shape_visual(&self) -> crate::app::ShapeVisual {
        let rows = self.visible_rows();
        let shape = self.current_shape(&rows);
        let sp = &self.params.shapes[shape];
        let notes = sp.notes.load(Ordering::Relaxed).clamp(MIN_NOTES, MAX_NOTES);
        let direction = sp.direction.load(Ordering::Relaxed);
        let last_fired = sp.last_fired.load(Ordering::Relaxed);
        let running = sp.running.load(Ordering::Relaxed);

        let outer = (0..NUM_BARS)
            .map(|k| {
                let angle = (k as f32 / NUM_BARS as f32 * TAU) - FRAC_PI_2;
                let active = sp.bars_active[k].load(Ordering::Relaxed);
                (angle.cos(), angle.sin(), active, false)
            })
            .collect();

        let inner = (0..notes)
            .map(|i| {
                let angle = display_phase(sp.phases[i].get(), direction) * TAU - FRAC_PI_2;
                (angle.cos(), angle.sin(), i == last_fired)
            })
            .collect();

        crate::app::ShapeVisual { shape_index: shape, running, outer, inner, closed: true }
    }
}

impl App for MadnessApp {
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
        crate::app::SlintExtra::Shape(self.shape_visual())
    }

    /// Same "whichever shape is currently focused" convention Bloom
    /// uses -- see its `running`/`toggle_running` for the rationale.
    fn running(&self) -> Option<bool> {
        let rows = self.visible_rows();
        let shape = self.current_shape(&rows);
        Some(self.params.shapes[shape].running.load(Ordering::Relaxed))
    }

    fn toggle_running(&mut self) {
        let rows = self.visible_rows();
        let shape = self.current_shape(&rows);
        let cur = self.params.shapes[shape].running.load(Ordering::Relaxed);
        self.params.shapes[shape].running.store(!cur, Ordering::Relaxed);
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
        self.last_shape = self.current_shape(&rows);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(MadnessProcessor {
            params: Arc::clone(&self.params),
            shapes: std::array::from_fn(|i| ShapeRuntime::new(i as u32)),
            mono_buf: Vec::new(),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(MADNESS_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, MADNESS_TITLE);
        Text::new("Madness", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, MADNESS_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, MADNESS_DIM);

        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    let name = if *g == 0 { "Master Clock".to_string() } else { format!("Shape {g}") };
                    (format!("{arrow} {name}"), self.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", self.leaf_name(*sel)), self.leaf_value(*sel)),
            })
            .collect();
        self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, MADNESS_BG, MADNESS_DIM, MADNESS_ACCENT);

        // --- Right: the circle -- note positions (each independently
        // drifting), the 8 fixed trigger-bar markers sitting on the
        // ring, and connecting lines that bend as positions diverge. ---
        let shape = self.current_shape(&rows);
        let sp = &self.params.shapes[shape];
        let cx = 500;
        let cy = 170;
        let radius = 110.0f32;
        let notes = sp.notes.load(Ordering::Relaxed).clamp(MIN_NOTES, MAX_NOTES);
        let direction = sp.direction.load(Ordering::Relaxed);
        let last_fired = sp.last_fired.load(Ordering::Relaxed);

        let note_point = |i: usize| -> Point {
            let angle = display_phase(sp.phases[i].get(), direction) * TAU - FRAC_PI_2;
            Point::new(cx + (radius * angle.cos()) as i32, cy + (radius * angle.sin()) as i32)
        };
        for i in 0..notes {
            let p0 = note_point(i);
            let p1 = note_point((i + 1) % notes);
            Line::new(p0, p1).into_styled(PrimitiveStyle::with_stroke(MADNESS_LINE, 1)).draw(fb).ok();
        }
        for i in 0..notes {
            let p = note_point(i);
            let lit = i == last_fired;
            // The lit flash clashes hard against the resting magenta
            // -- chartreuse against pink, a deliberately jarring pop
            // rather than a smooth same-hue brighten.
            let (color, d) = if lit { (MADNESS_CLASH, 8) } else { (MADNESS_NOTE_OFF, 4) };
            Circle::new(Point::new(p.x - d / 2, p.y - d / 2), d as u32)
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(fb)
                .ok();
        }

        // Trigger-bar markers -- small circles sitting directly on the
        // ring (not a separate outward tick).
        for k in 0..NUM_BARS {
            let a = (k as f32 / NUM_BARS as f32 * TAU) - FRAC_PI_2;
            let p = Point::new(cx + (radius * a.cos()) as i32, cy + (radius * a.sin()) as i32);
            let active = sp.bars_active[k].load(Ordering::Relaxed);
            let (color, d) = if active { (MADNESS_ACCENT, 10) } else { (MADNESS_BAR_OFF, 6) };
            Circle::new(Point::new(p.x - d / 2, p.y - d / 2), d as u32)
                .into_styled(PrimitiveStyle::with_stroke(color, 2))
                .draw(fb)
                .ok();
        }

        let running = sp.running.load(Ordering::Relaxed);
        let status = if running { "running" } else { "stopped" };
        Text::new(&format!("Shape {} -- {}", shape + 1, status), Point::new(420, 300), accent).draw(fb).ok();

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

struct ShapeRuntime {
    voice: PlaitsVoice,
    voice_buf: Vec<f32>,
    current_note: f32,
    velocity: f32,
    trigger: bool,
    rng: u32,
}

impl ShapeRuntime {
    fn new(seed: u32) -> Self {
        Self {
            voice: PlaitsVoice::new(),
            voice_buf: Vec::new(),
            current_note: 60.0,
            velocity: 1.0,
            trigger: false,
            rng: 0x9E3779B9 ^ (seed.wrapping_mul(0x85EBCA6B) | 1),
        }
    }

    fn next_rand01(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }
}

struct MadnessProcessor {
    params: Arc<Params>,
    shapes: [ShapeRuntime; NUM_SHAPES],
    mono_buf: Vec<f32>,
}

impl AudioProcessor for MadnessProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        self.mono_buf.clear();
        self.mono_buf.resize(frames, 0.0);
        let dt = frames as f32 / sample_rate;
        let master_bpm = self.params.master_bpm.get().max(1.0);

        for s in 0..NUM_SHAPES {
            let sp = &self.params.shapes[s];
            let rt = &mut self.shapes[s];
            let notes = sp.notes.load(Ordering::Relaxed).clamp(MIN_NOTES, MAX_NOTES);
            let direction = sp.direction.load(Ordering::Relaxed);
            let running = sp.running.load(Ordering::Relaxed);

            rt.trigger = false;

            if running {
                let base_speed = if sp.tempo_sync.load(Ordering::Relaxed) {
                    let ratio = CLOCK_MODS[sp.clock_mod.load(Ordering::Relaxed) % CLOCK_MODS.len()].1;
                    (master_bpm / 60.0) * ratio
                } else {
                    sp.speed_hz.get()
                };
                let speed = (base_speed + sp.ext_speed.get()).max(0.0);
                let dir_sign = if direction == 1 { -1.0 } else { 1.0 };
                let offset = sp.speed_offset.get().clamp(-1.0, 1.0);
                let probability = sp.probability.get();

                for i in 0..notes {
                    let t = if notes > 1 { i as f32 / (notes - 1) as f32 } else { 0.5 };
                    let dist_from_center = (t - 0.5).abs() * 2.0;
                    let mult = (1.0 + offset * dist_from_center).max(0.0);
                    let rate = speed * mult * dir_sign;

                    let prev_raw = sp.phases[i].get();
                    let new_raw = prev_raw + rate * dt;

                    for k in 0..NUM_BARS {
                        let bar_angle = k as f32 / NUM_BARS as f32;
                        if crosses(prev_raw, new_raw, bar_angle) && sp.bars_active[k].load(Ordering::Relaxed)
                            && rt.next_rand01() < probability {
                                let scale = sp.scale.load(Ordering::Relaxed);
                                let root = sp.root.load(Ordering::Relaxed);
                                rt.current_note = note_for_position(i, scale as usize, root) as f32;
                                let vmin = sp.vel_min.get().min(sp.vel_max.get());
                                let vmax = sp.vel_min.get().max(sp.vel_max.get());
                                rt.velocity = vmin + rt.next_rand01() * (vmax - vmin);
                                rt.trigger = true;
                                sp.last_fired.store(i, Ordering::Relaxed);
                            }
                    }

                    sp.phases[i].set(new_raw);
                }
            }

            let engine = sp.engine.load(Ordering::Relaxed) as i32;
            let params = PlaitsParams {
                engine,
                note: rt.current_note,
                harmonics: sp.harmonics.get(),
                timbre: sp.timbre.get(),
                morph: 0.5,
                decay: sp.decay.get(),
                lpg_colour: 0.5,
                trigger: rt.trigger,
            };
            rt.voice_buf.clear();
            rt.voice_buf.resize(frames, 0.0);
            rt.voice.render(&mut rt.voice_buf, sample_rate, &params);

            let level = (sp.ext_level.get() + 1.0).clamp(0.0, 2.0);
            let gain = rt.velocity * level;
            for (m, sample) in self.mono_buf.iter_mut().zip(rt.voice_buf.iter()) {
                *m += *sample * gain;
            }
        }

        {
            let mut bus_out = self.params.bus_out.lock().unwrap();
            bus_out.clear();
            bus_out.extend_from_slice(&self.mono_buf);
        }

        // The Mixer app's channel fader for this app -- applied only
        // to what reaches the device, not to `bus_out` above (see
        // plaits.rs for the same pattern).
        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        for (frame, sample) in buffer.chunks_mut(channels).zip(self.mono_buf.iter()) {
            for out in frame.iter_mut() {
                *out = *sample * mix_level;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The freshly-constructed, untouched `Params` -- the exact state
    /// a brand new app instance starts in -- must have every shape
    /// default to stopped. With N apps all loaded (and someday
    /// possibly far more), an app that defaults to *running* is a
    /// real risk on its own, not just an inconvenience.
    #[test]
    fn every_shape_defaults_to_stopped() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Params::new(&modbus, &audio_bus, &mixer_bus);
        for s in 0..NUM_SHAPES {
            assert!(!params.shapes[s].running.load(Ordering::Relaxed), "shape {s} must start stopped, not running");
        }
    }
}
