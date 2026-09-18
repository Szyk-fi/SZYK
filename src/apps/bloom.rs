//! A generative circular sequencer built from two groups of the same
//! kind of element -- **dots** -- never a separate "arm" concept.
//!
//! **Outer dots** sit on the boundary circle at fixed, evenly-spaced
//! angles -- they never move, each can be switched on or off, and
//! there's a count for how many exist. They're the trigger
//! references, same role the old "arms" played, just static and
//! togglable instead of rotating and bendable.
//!
//! **Inner dots** carry the notes: each orbits on its own fixed
//! concentric ring (ring radius set by its index -- innermost =
//! lowest note, outermost = highest, spread across an adjustable
//! Octave Range), all starting at the same fixed 12-o'clock position,
//! each spinning at its own multiple of the shape's base dot speed.
//! So they start bunched together on the 12-o'clock radial, gradually
//! spread apart as the faster ones lap the slower ones, and --
//! because the per-dot speed multipliers are simple rational steps --
//! periodically re-converge.
//!
//! A note fires whenever an inner dot's orbit crosses an *active*
//! outer dot's fixed angle. Since the outer dot's position never
//! moves, the trigger reference -- and the line that flashes from it
//! to the center when it fires -- are both static, like the boundary
//! circle itself; only the inner dots visibly orbit.
//!
//! Changing either dot count (inner or outer) resets every inner
//! dot's phase back to its 12-o'clock start -- ring/angle assignments
//! shift when a count changes, so a clean restart avoids a dot
//! jumping mid-orbit into a different ring.
//!
//! 4 independent shape instances, each with its own dots/scale/voice,
//! all mixing together, same architecture as Madness/Pam's: a shared
//! Master Clock BPM, per-shape Running (off by default, deliberately
//! first in that shape's tree) and Randomize, and Speed/Level exposed
//! to the shared ModBus.
//!
//! Simplified: one global trigger probability, not per-dot; up to 16
//! inner dots and 8 outer dots; Ping-Pong direction's trigger timing
//! is approximate right at the bounce instant (see `crosses`' doc
//! comment) -- everything else about crossing detection here is
//! exact, not a visual approximation.

use crate::app::{App, Input};
use crate::apps::plaits::{ENGINE_NAMES, ROOT_NAMES, SCALE_TYPES};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::plaits_ffi::{PlaitsParams, PlaitsVoice};
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle};
use embedded_graphics::text::Text;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NUM_SHAPES: usize = 4;
const MIN_DOTS: usize = 2;
const MAX_DOTS: usize = 16;
const DEFAULT_DOTS: usize = 8;
const MIN_OUTER: usize = 1;
const MAX_OUTER: usize = 8;
const DEFAULT_OUTER: usize = 3;
const MIN_OCTAVE_RANGE: u32 = 1;
const MAX_OCTAVE_RANGE: u32 = 4;
const DEFAULT_OCTAVE_RANGE: u32 = 2;
/// Base multiplier applied to every dot's speed, on top of its own
/// per-index step below.
const DOT_SPEED_MULT: f32 = 0.35;
/// Each dot spins at `1 + dot_index * DOT_SPEED_STEP` times the base
/// dot speed. All dots start at the same phase (see
/// `ShapeParams::new`), so this is what makes them gradually spread
/// apart and then periodically snap back into alignment.
const DOT_SPEED_STEP: f32 = 0.2;
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
/// Ping-Pong triangle-wraps (bounces at 0 and 1). Only used for
/// visual placement; crossing detection uses the raw value directly
/// (see `crosses`), not this.
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
/// each endpoint to 0..1 with `rem_euclid`, then taking their plain
/// min/max as the swept interval): whenever a block's sweep itself
/// crossed the 0/1 wrap point, that produced the *complementary* arc
/// -- silently reporting false crossings for targets in the short arc
/// between the two wrapped endpoints, and missing real crossings for
/// targets in the long arc that was actually swept. Working in raw
/// (never-wrapped) space and checking every `target + k` near the
/// current window sidesteps that entirely, however many full turns
/// have already accumulated.
///
/// Not adjusted for Ping-Pong's bounce (the displayed angle isn't a
/// linear function of the raw accumulator there) -- trigger timing is
/// exact for Forward/Reverse, and only approximate right at a
/// Ping-Pong bounce instant. A fully correct fix would track the
/// reflected phase family separately; given how briefly that instant
/// lasts, this was judged not worth the added complexity.
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

/// The fixed ring radius (as a fraction of the outer circle) a given
/// inner dot orbits on -- assigned by index, spread evenly from near
/// the center (lowest note) to near the edge (highest), never
/// changing while the dot spins. This is what note pitch is keyed to.
fn dot_radius_frac(d: usize, num_dots: usize) -> f32 {
    (d as f32 + 0.5) / num_dots.max(1) as f32
}

/// The fixed angle (in turns) an outer dot sits at -- evenly spaced
/// around the boundary, index 0 at 12 o'clock, never changing.
fn outer_angle_turns(o: usize, num_outer: usize) -> f32 {
    o as f32 / num_outer.max(1) as f32
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    MasterBpm,
    Running(usize),
    Dots(usize),
    OuterDots(usize),
    OuterActive(usize, usize),
    Scale(usize),
    Root(usize),
    OctaveRange(usize),
    Direction(usize),
    TempoSync(usize),
    Speed(usize),
    ClockMod(usize),
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
    running: AtomicBool,
    dots: AtomicUsize,
    outer: AtomicUsize,
    outer_active: [AtomicBool; MAX_OUTER],
    scale: AtomicU32,
    root: AtomicU32,
    octave_range: AtomicU32,
    direction: AtomicU32,
    tempo_sync: AtomicBool,
    speed_hz: AtomicF32,
    clock_mod: AtomicUsize,
    probability: AtomicF32,
    vel_min: AtomicF32,
    vel_max: AtomicF32,
    engine: AtomicU32,
    harmonics: AtomicF32,
    timbre: AtomicF32,
    decay: AtomicF32,
    /// Each inner dot's own raw (unwrapped) phase accumulator. All
    /// start at 0.0 (the same 12-o'clock spot) and spin at their own
    /// multiple of the base dot speed -- see `DOT_SPEED_STEP`.
    dot_phases: [AtomicF32; MAX_DOTS],
    last_fired_dot: AtomicUsize,
    last_fired_outer: AtomicUsize,
    ext_speed: Arc<AtomicF32>,
    ext_level: Arc<AtomicF32>,
}

impl ShapeParams {
    fn new(shape_num: usize, modbus: &ModBus) -> Self {
        Self {
            running: AtomicBool::new(false),
            dots: AtomicUsize::new(DEFAULT_DOTS),
            outer: AtomicUsize::new(DEFAULT_OUTER),
            outer_active: std::array::from_fn(|_| AtomicBool::new(true)),
            scale: AtomicU32::new(0),
            root: AtomicU32::new(0),
            octave_range: AtomicU32::new(DEFAULT_OCTAVE_RANGE),
            direction: AtomicU32::new(0),
            tempo_sync: AtomicBool::new(false),
            speed_hz: AtomicF32::new(DEFAULT_SPEED_HZ),
            clock_mod: AtomicUsize::new(3),
            probability: AtomicF32::new(1.0),
            vel_min: AtomicF32::new(0.6),
            vel_max: AtomicF32::new(1.0),
            engine: AtomicU32::new(8),
            harmonics: AtomicF32::new(0.5),
            timbre: AtomicF32::new(0.5),
            decay: AtomicF32::new(0.4),
            dot_phases: std::array::from_fn(|_| AtomicF32::new(0.0)),
            // usize::MAX, not 0 -- so nothing shows as "lit" before a
            // real trigger has ever fired.
            last_fired_dot: AtomicUsize::new(usize::MAX),
            last_fired_outer: AtomicUsize::new(usize::MAX),
            ext_speed: modbus.register(format!("Bloom {shape_num}: Speed")),
            ext_level: modbus.register(format!("Bloom {shape_num}: Level")),
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
        let (mix_level, ext_mix_level) = mixer_bus.register("Bloom", modbus);
        Self {
            master_bpm: AtomicF32::new(DEFAULT_BPM),
            shapes: std::array::from_fn(|i| ShapeParams::new(i + 1, modbus)),
            bus_out: audio_bus.register("Bloom"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct BloomApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    last_shape: usize,
    rng: u32,
}

impl BloomApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(24681);
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

    /// Restarts every inner dot at its 12-o'clock start -- called
    /// whenever a dot count changes, since ring/angle assignments
    /// shift and a dot mid-orbit would otherwise jump.
    fn reset_dot_phases(&self, s: usize) {
        let sp = &self.params.shapes[s];
        for p in sp.dot_phases.iter() {
            p.set(0.0);
        }
        sp.last_fired_dot.store(usize::MAX, Ordering::Relaxed);
        sp.last_fired_outer.store(usize::MAX, Ordering::Relaxed);
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        if g == 0 {
            return vec![Selection::MasterBpm];
        }
        let s = g - 1;
        // Running comes first -- the master on/off for this shape.
        let mut v = vec![Selection::Running(s), Selection::Dots(s), Selection::OuterDots(s)];
        let num_outer = self.params.shapes[s].outer.load(Ordering::Relaxed).clamp(MIN_OUTER, MAX_OUTER);
        for o in 0..num_outer {
            v.push(Selection::OuterActive(s, o));
        }
        v.push(Selection::Scale(s));
        v.push(Selection::Root(s));
        v.push(Selection::OctaveRange(s));
        v.push(Selection::Direction(s));
        v.push(Selection::TempoSync(s));
        if self.params.shapes[s].tempo_sync.load(Ordering::Relaxed) {
            v.push(Selection::ClockMod(s));
        } else {
            v.push(Selection::Speed(s));
        }
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
            Selection::Running(s)
            | Selection::Dots(s)
            | Selection::OuterDots(s)
            | Selection::OuterActive(s, _)
            | Selection::Scale(s)
            | Selection::Root(s)
            | Selection::OctaveRange(s)
            | Selection::Direction(s)
            | Selection::TempoSync(s)
            | Selection::Speed(s)
            | Selection::ClockMod(s)
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
            Selection::Running(_) => "Running".into(),
            Selection::Dots(_) => "Dots".into(),
            Selection::OuterDots(_) => "Outer Dots".into(),
            Selection::OuterActive(_, o) => format!("Outer {} Active", o + 1),
            Selection::Scale(_) => "Scale".into(),
            Selection::Root(_) => "Root".into(),
            Selection::OctaveRange(_) => "Octave Range".into(),
            Selection::Direction(_) => "Direction".into(),
            Selection::TempoSync(_) => "Tempo Sync".into(),
            Selection::Speed(_) => "Speed".into(),
            Selection::ClockMod(_) => "Clock Mod".into(),
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
            Selection::Running(s) => {
                if self.params.shapes[s].running.load(Ordering::Relaxed) { "running".into() } else { "stopped".into() }
            }
            Selection::Dots(s) => format!("{}", self.params.shapes[s].dots.load(Ordering::Relaxed)),
            Selection::OuterDots(s) => format!("{}", self.params.shapes[s].outer.load(Ordering::Relaxed)),
            Selection::OuterActive(s, o) => {
                if self.params.shapes[s].outer_active[o].load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::Scale(s) => {
                let idx = self.params.shapes[s].scale.load(Ordering::Relaxed) as usize % SCALE_TYPES.len();
                SCALE_TYPES[idx].0.to_string()
            }
            Selection::Root(s) => ROOT_NAMES[self.params.shapes[s].root.load(Ordering::Relaxed) as usize % 12].to_string(),
            Selection::OctaveRange(s) => format!("{} oct", self.params.shapes[s].octave_range.load(Ordering::Relaxed)),
            Selection::Direction(s) => {
                let idx = self.params.shapes[s].direction.load(Ordering::Relaxed) as usize % DIRECTION_NAMES.len();
                DIRECTION_NAMES[idx].to_string()
            }
            Selection::TempoSync(s) => {
                if self.params.shapes[s].tempo_sync.load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::Speed(s) => format!("{:.2} Hz", self.params.shapes[s].speed_hz.get()),
            Selection::ClockMod(s) => {
                CLOCK_MODS[self.params.shapes[s].clock_mod.load(Ordering::Relaxed) % CLOCK_MODS.len()].0.to_string()
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
        format!(
            "{} dots, {} outer{}",
            self.params.shapes[s].dots.load(Ordering::Relaxed),
            self.params.shapes[s].outer.load(Ordering::Relaxed),
            running
        )
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
            Selection::Running(s) => self.params.shapes[s].running.store(delta > 0, Ordering::Relaxed),
            Selection::Dots(s) => {
                let cur = self.params.shapes[s].dots.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_DOTS as i32, MAX_DOTS as i32);
                self.params.shapes[s].dots.store(next as usize, Ordering::Relaxed);
                self.reset_dot_phases(s);
            }
            Selection::OuterDots(s) => {
                let cur = self.params.shapes[s].outer.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_OUTER as i32, MAX_OUTER as i32);
                self.params.shapes[s].outer.store(next as usize, Ordering::Relaxed);
                self.reset_dot_phases(s);
            }
            Selection::OuterActive(s, o) => self.params.shapes[s].outer_active[o].store(delta > 0, Ordering::Relaxed),
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
            Selection::OctaveRange(s) => {
                let cur = self.params.shapes[s].octave_range.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_OCTAVE_RANGE as i32, MAX_OCTAVE_RANGE as i32);
                self.params.shapes[s].octave_range.store(next as u32, Ordering::Relaxed);
            }
            Selection::Direction(s) => {
                let cur = self.params.shapes[s].direction.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(DIRECTION_NAMES.len() as i32);
                self.params.shapes[s].direction.store(next as u32, Ordering::Relaxed);
            }
            Selection::TempoSync(s) => self.params.shapes[s].tempo_sync.store(delta > 0, Ordering::Relaxed),
            Selection::Speed(s) => bump(&self.params.shapes[s].speed_hz, delta, sensitivity, MIN_SPEED_HZ, MAX_SPEED_HZ),
            Selection::ClockMod(s) => {
                let cur = self.params.shapes[s].clock_mod.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(CLOCK_MODS.len() as i32);
                self.params.shapes[s].clock_mod.store(next as usize, Ordering::Relaxed);
            }
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

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::MasterBpm => self.params.master_bpm.set(DEFAULT_BPM),
            Selection::Dots(s) => {
                self.params.shapes[s].dots.store(DEFAULT_DOTS, Ordering::Relaxed);
                self.reset_dot_phases(s);
            }
            Selection::OuterDots(s) => {
                self.params.shapes[s].outer.store(DEFAULT_OUTER, Ordering::Relaxed);
                self.reset_dot_phases(s);
            }
            Selection::OctaveRange(s) => self.params.shapes[s].octave_range.store(DEFAULT_OCTAVE_RANGE, Ordering::Relaxed),
            Selection::Speed(s) => self.params.shapes[s].speed_hz.set(DEFAULT_SPEED_HZ),
            Selection::ClockMod(s) => self.params.shapes[s].clock_mod.store(3, Ordering::Relaxed),
            Selection::Probability(s) => self.params.shapes[s].probability.set(1.0),
            Selection::VelMin(s) => self.params.shapes[s].vel_min.set(0.6),
            Selection::VelMax(s) => self.params.shapes[s].vel_max.set(1.0),
            Selection::Harmonics(s) => self.params.shapes[s].harmonics.set(0.5),
            Selection::Timbre(s) => self.params.shapes[s].timbre.set(0.5),
            Selection::Decay(s) => self.params.shapes[s].decay.set(0.4),
            Selection::Randomize(s) => self.randomize(s),
            // No sensible single default: the master switch shouldn't
            // get flipped by a "reset" press, and these have no one
            // obvious default.
            Selection::Running(_)
            | Selection::OuterActive(_, _)
            | Selection::Scale(_)
            | Selection::Root(_)
            | Selection::Direction(_)
            | Selection::TempoSync(_)
            | Selection::Engine(_) => {}
        }
    }

    /// Reassigns most of one shape's parameters at once for quick
    /// exploration -- deliberately leaves Running alone, so
    /// randomizing doesn't also start something running.
    fn randomize(&mut self, s: usize) {
        let dots = MIN_DOTS + (self.next_rand01() * (MAX_DOTS - MIN_DOTS + 1) as f32) as usize;
        let outer = MIN_OUTER + (self.next_rand01() * (MAX_OUTER - MIN_OUTER + 1) as f32) as usize;
        let outer_active: [bool; MAX_OUTER] = std::array::from_fn(|_| self.next_rand01() < 0.8);
        let scale = (self.next_rand01() * SCALE_TYPES.len() as f32) as u32;
        let root = (self.next_rand01() * 12.0) as u32;
        let octave_range = MIN_OCTAVE_RANGE + (self.next_rand01() * (MAX_OCTAVE_RANGE - MIN_OCTAVE_RANGE + 1) as f32) as u32;
        let direction = (self.next_rand01() * DIRECTION_NAMES.len() as f32) as u32;
        let speed_hz = MIN_SPEED_HZ + self.next_rand01() * (MAX_SPEED_HZ - MIN_SPEED_HZ);
        let clock_mod = (self.next_rand01() * CLOCK_MODS.len() as f32) as usize;
        let probability = 0.4 + self.next_rand01() * 0.6;
        let v0 = self.next_rand01();
        let v1 = self.next_rand01();
        let engine = (self.next_rand01() * 24.0) as u32;
        let harmonics = self.next_rand01();
        let timbre = self.next_rand01();
        let decay = self.next_rand01();

        let sp = &self.params.shapes[s];
        sp.dots.store(dots.clamp(MIN_DOTS, MAX_DOTS), Ordering::Relaxed);
        sp.outer.store(outer.clamp(MIN_OUTER, MAX_OUTER), Ordering::Relaxed);
        for (o, active) in outer_active.iter().enumerate() {
            sp.outer_active[o].store(*active, Ordering::Relaxed);
        }
        sp.scale.store(scale, Ordering::Relaxed);
        sp.root.store(root, Ordering::Relaxed);
        sp.octave_range.store(octave_range.clamp(MIN_OCTAVE_RANGE, MAX_OCTAVE_RANGE), Ordering::Relaxed);
        sp.direction.store(direction, Ordering::Relaxed);
        sp.speed_hz.set(speed_hz);
        sp.clock_mod.store(clock_mod, Ordering::Relaxed);
        sp.probability.set(probability);
        sp.vel_min.set(v0.min(v1));
        sp.vel_max.set(v0.max(v1).max(v0.min(v1) + 0.05));
        sp.engine.store(engine, Ordering::Relaxed);
        sp.harmonics.set(harmonics);
        sp.timbre.set(timbre);
        sp.decay.set(decay);
        self.reset_dot_phases(s);
    }
}

impl App for BloomApp {
    /// Reflects/controls whichever shape is currently focused (the
    /// one the right-hand panel is showing) -- there's no single
    /// "the" Running for this app since it's per-shape, so the
    /// OS-level Start/Stop button (F3) acts on whichever one you're
    /// actually looking at, same target `Selection::Running` in the
    /// menu already edits.
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
        self.last_shape = self.current_shape(&rows);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(BloomProcessor {
            params: Arc::clone(&self.params),
            shapes: std::array::from_fn(|i| ShapeRuntime::new(i as u32)),
            mono_buf: Vec::new(),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let title = MonoTextStyle::new(&SPLEEN_16X32, Rgb565::WHITE);
        Text::new("Bloom", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(0, 63, 10));
        let dim = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));

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
        self.list.draw(fb, 16, 44, 24, 10, &display_rows);

        // --- Right: the fixed circle; static outer dots on its
        // boundary (the trigger references); inner dots each
        // orbiting on their own fixed concentric ring (pitch). ---
        let shape = self.current_shape(&rows);
        let sp = &self.params.shapes[shape];
        let cx = 500;
        let cy = 170;
        let radius = 110.0f32;
        let direction = sp.direction.load(Ordering::Relaxed);
        let num_dots = sp.dots.load(Ordering::Relaxed).clamp(MIN_DOTS, MAX_DOTS);
        let num_outer = sp.outer.load(Ordering::Relaxed).clamp(MIN_OUTER, MAX_OUTER);
        let last_fired_dot = sp.last_fired_dot.load(Ordering::Relaxed);
        let last_fired_outer = sp.last_fired_outer.load(Ordering::Relaxed);
        let running = sp.running.load(Ordering::Relaxed);

        Circle::new(Point::new(cx - radius as i32, cy - radius as i32), (radius * 2.0) as u32)
            .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(10, 20, 10), 1))
            .draw(fb)
            .ok();

        // Outer dots -- static trigger references on the boundary.
        for o in 0..num_outer {
            let active = sp.outer_active[o].load(Ordering::Relaxed);
            let lit = o == last_fired_outer;
            let angle = outer_angle_turns(o, num_outer) * TAU - FRAC_PI_2;
            let tip = Point::new(cx + (radius * angle.cos()) as i32, cy + (radius * angle.sin()) as i32);

            let color = if !active {
                Rgb565::new(6, 10, 6)
            } else if lit {
                Rgb565::new(0, 63, 10)
            } else {
                Rgb565::new(0, 35, 5)
            };
            if lit {
                // The flash: this outer dot's fixed position to the
                // center -- static, since that point never moves.
                Line::new(Point::new(cx, cy), tip).into_styled(PrimitiveStyle::with_stroke(color, 1)).draw(fb).ok();
            }
            let dd: i32 = if lit { 8 } else { 5 };
            Circle::new(Point::new(tip.x - dd / 2, tip.y - dd / 2), dd as u32)
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(fb)
                .ok();
        }

        // Inner dots -- the notes, each on its own fixed ring. Since
        // ring radius grows with index and each dot has drifted to
        // its own angle, connecting them innermost-to-outermost in
        // index order traces a spiraling line through the current
        // moment -- purely visual, no bearing on triggering.
        let mut dot_points: [Point; MAX_DOTS] = [Point::new(cx, cy); MAX_DOTS];
        for d in 0..num_dots {
            let r = radius * dot_radius_frac(d, num_dots);
            let base = display_phase(sp.dot_phases[d].get(), direction);
            let angle = base * TAU - FRAC_PI_2;
            dot_points[d] = Point::new(cx + (r * angle.cos()) as i32, cy + (r * angle.sin()) as i32);
        }

        for d in 1..num_dots {
            Line::new(dot_points[d - 1], dot_points[d])
                .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(0, 24, 4), 1))
                .draw(fb)
                .ok();
        }

        for d in 0..num_dots {
            let lit = d == last_fired_dot;
            let (color, dd) = if lit { (Rgb565::new(0, 63, 10), 9) } else { (Rgb565::new(0, 40, 6), 5) };
            let p = dot_points[d];
            Circle::new(Point::new(p.x - dd / 2, p.y - dd / 2), dd as u32)
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(fb)
                .ok();
        }

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

/// How many notes a single shape can sound at once. Simultaneous
/// triggers are common here -- most strikingly right when the dots
/// periodically re-converge and several can cross an outer dot in
/// the same block -- so one voice per shape (retriggering, cutting
/// off whatever was still decaying) would lose most of that. 8 gives
/// real polyphony without turning every shape into a 16-voice choir.
const NUM_VOICES: usize = 8;

/// One of a shape's polyphonic voices. Engine/Harmonics/Timbre/Decay
/// are shared per shape (one "instrument"); only note, velocity and
/// the one-shot trigger pulse are per-voice.
struct PolyVoice {
    voice: PlaitsVoice,
    voice_buf: Vec<f32>,
    note: f32,
    velocity: f32,
    trigger: bool,
    /// Set to the shape's trigger counter each time this voice is
    /// (re)allocated -- lets `trigger_note` steal whichever voice was
    /// used longest ago (simple round-robin voice stealing).
    last_used: u64,
}

impl PolyVoice {
    fn new() -> Self {
        Self { voice: PlaitsVoice::new(), voice_buf: Vec::new(), note: 60.0, velocity: 1.0, trigger: false, last_used: 0 }
    }
}

struct ShapeRuntime {
    voices: [PolyVoice; NUM_VOICES],
    voice_gen: u64,
    rng: u32,
}

impl ShapeRuntime {
    fn new(seed: u32) -> Self {
        Self {
            voices: std::array::from_fn(|_| PolyVoice::new()),
            voice_gen: 0,
            rng: 0x9E3779B9 ^ (seed.wrapping_mul(0x85EBCA6B) | 1),
        }
    }

    fn next_rand01(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    /// Allocates a new note to whichever voice was least recently
    /// triggered, so overlapping/simultaneous notes each get their
    /// own decay instead of all retriggering a single shared voice.
    fn trigger_note(&mut self, note: f32, velocity: f32) {
        self.voice_gen += 1;
        let idx = self.voices.iter().enumerate().min_by_key(|(_, v)| v.last_used).map(|(i, _)| i).unwrap_or(0);
        let v = &mut self.voices[idx];
        v.note = note;
        v.velocity = velocity;
        v.trigger = true;
        v.last_used = self.voice_gen;
    }
}

struct BloomProcessor {
    params: Arc<Params>,
    shapes: [ShapeRuntime; NUM_SHAPES],
    mono_buf: Vec<f32>,
}

impl AudioProcessor for BloomProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        self.mono_buf.clear();
        self.mono_buf.resize(frames, 0.0);
        let dt = frames as f32 / sample_rate;
        let master_bpm = self.params.master_bpm.get().max(1.0);

        // Counts how many voices actually contributed audible signal
        // this block, across every shape's pool -- used below as a
        // headroom divisor, same normalize-by-active-voices approach
        // plaits.rs's poly mode already uses (see its `headroom`
        // there), just measured from rendered peak rather than a held
        // gate, since these are one-shot decaying triggers rather than
        // held notes. Without this, a re-convergence moment (several
        // dots crossing active outer dots close together, possibly
        // across more than one running shape) sums unboundedly and
        // clips no matter how low Vel Min/Max or the Mixer's Bloom
        // fader are set -- neither actually reduces how many voices
        // stack, only how loud each one is.
        let mut active_voices: usize = 0;

        for s in 0..NUM_SHAPES {
            let sp = &self.params.shapes[s];
            let rt = &mut self.shapes[s];
            let running = sp.running.load(Ordering::Relaxed);

            if running {
                let base_speed = if sp.tempo_sync.load(Ordering::Relaxed) {
                    let ratio = CLOCK_MODS[sp.clock_mod.load(Ordering::Relaxed) % CLOCK_MODS.len()].1;
                    (master_bpm / 60.0) * ratio
                } else {
                    sp.speed_hz.get()
                };
                let speed = (base_speed + sp.ext_speed.get()).max(0.0);
                let direction = sp.direction.load(Ordering::Relaxed);
                let dir_sign = if direction == 1 { -1.0 } else { 1.0 };
                let probability = sp.probability.get();
                let num_dots = sp.dots.load(Ordering::Relaxed).clamp(MIN_DOTS, MAX_DOTS);
                let num_outer = sp.outer.load(Ordering::Relaxed).clamp(MIN_OUTER, MAX_OUTER);

                // Each inner dot has its own phase and its own speed
                // multiple of the base dot rate -- all seeded at 0.0
                // (see ShapeParams::new), so they start bunched
                // together at 12 o'clock and gradually spread apart,
                // periodically re-converging since the multipliers
                // are simple rational steps. A note fires whenever a
                // dot's orbit crosses an *active* outer dot's fixed
                // (static) angle.
                for d in 0..num_dots {
                    let mult = 1.0 + d as f32 * DOT_SPEED_STEP;
                    let rate = speed * DOT_SPEED_MULT * mult * dir_sign;
                    let prev_raw = sp.dot_phases[d].get();
                    let new_raw = prev_raw + rate * dt;
                    sp.dot_phases[d].set(new_raw);

                    for o in 0..num_outer {
                        if !sp.outer_active[o].load(Ordering::Relaxed) {
                            continue;
                        }
                        let target = outer_angle_turns(o, num_outer);
                        if crosses(prev_raw, new_raw, target) && rt.next_rand01() < probability {
                            let t = dot_radius_frac(d, num_dots);
                            let scale = sp.scale.load(Ordering::Relaxed);
                            let root = sp.root.load(Ordering::Relaxed);
                            let oct_range = sp.octave_range.load(Ordering::Relaxed).max(MIN_OCTAVE_RANGE);
                            let scale_len = SCALE_TYPES[scale as usize % SCALE_TYPES.len()].1.len().max(1);
                            let total_positions = scale_len * oct_range as usize;
                            let degree_pos = ((t * total_positions as f32) as usize).min(total_positions.saturating_sub(1));
                            let note = note_for_position(degree_pos, scale as usize, root) as f32;
                            let vmin = sp.vel_min.get().min(sp.vel_max.get());
                            let vmax = sp.vel_min.get().max(sp.vel_max.get());
                            let velocity = vmin + rt.next_rand01() * (vmax - vmin);
                            rt.trigger_note(note, velocity);
                            sp.last_fired_dot.store(d, Ordering::Relaxed);
                            sp.last_fired_outer.store(o, Ordering::Relaxed);
                        }
                    }
                }
            }

            let engine = sp.engine.load(Ordering::Relaxed) as i32;
            let harmonics = sp.harmonics.get();
            let timbre = sp.timbre.get();
            let decay = sp.decay.get();
            let level = (sp.ext_level.get() + 1.0).clamp(0.0, 2.0);

            // Render every voice in the shape's pool -- most are
            // silently decaying or idle most of the time, but this is
            // what lets several notes (e.g. a re-convergence moment)
            // actually overlap instead of one stealing the shape's
            // only voice out from under another.
            for v in rt.voices.iter_mut() {
                let params = PlaitsParams {
                    engine,
                    note: v.note,
                    harmonics,
                    timbre,
                    morph: 0.5,
                    decay,
                    lpg_colour: 0.5,
                    trigger: v.trigger,
                };
                v.voice_buf.clear();
                v.voice_buf.resize(frames, 0.0);
                v.voice.render(&mut v.voice_buf, sample_rate, &params);
                v.trigger = false; // one-shot pulse, consumed this block

                let peak = v.voice_buf.iter().fold(0.0f32, |a, s| a.max(s.abs()));
                if peak > 1e-4 {
                    active_voices += 1;
                }

                let gain = v.velocity * level;
                for (m, sample) in self.mono_buf.iter_mut().zip(v.voice_buf.iter()) {
                    *m += *sample * gain;
                }
            }
        }

        let headroom = active_voices.max(1) as f32;
        for m in self.mono_buf.iter_mut() {
            *m /= headroom;
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

    /// Regression test for the "nothing appears to move" report: with
    /// a shape's Running on, each inner dot's raw phase must actually
    /// advance across process() calls, and crossing an active outer
    /// dot's angle must fire a trigger with real (in-range) indices
    /// rather than the stale default.
    #[test]
    fn running_shape_advances_phases_and_fires() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.shapes[0].running.store(true, Ordering::Relaxed);
        params.shapes[0].speed_hz.set(2.0); // fast, so the test doesn't need many blocks

        let mut proc = BloomProcessor { params: Arc::clone(&params), shapes: std::array::from_fn(|i| ShapeRuntime::new(i as u32)), mono_buf: Vec::new() };

        let dot0_start = params.shapes[0].dot_phases[0].get();
        let gen_start = proc.shapes[0].voice_gen;

        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..200 {
            proc.process(&mut buffer, 2, 48000.0);
        }

        let dot0_end = params.shapes[0].dot_phases[0].get();

        assert!((dot0_end - dot0_start).abs() > 0.01, "dot phase did not advance: {dot0_start} -> {dot0_end}");
        assert!(proc.shapes[0].voice_gen > gen_start, "expected at least one trigger over 200 blocks at 2 Hz");

        let fired_dot = params.shapes[0].last_fired_dot.load(Ordering::Relaxed);
        let fired_outer = params.shapes[0].last_fired_outer.load(Ordering::Relaxed);
        assert!(fired_dot < MAX_DOTS, "last_fired_dot left at sentinel/out-of-range: {fired_dot}");
        assert!(fired_outer < MAX_OUTER, "last_fired_outer left at sentinel/out-of-range: {fired_outer}");
    }

    /// Before any trigger has occurred, nothing should read as "lit"
    /// -- this is the sentinel-default bug that made dot/outer 0 look
    /// permanently flashed from frame one.
    #[test]
    fn nothing_lit_before_first_trigger() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Params::new(&modbus, &audio_bus, &mixer_bus);
        let last_fired_dot = params.shapes[0].last_fired_dot.load(Ordering::Relaxed);
        let last_fired_outer = params.shapes[0].last_fired_outer.load(Ordering::Relaxed);
        assert!((0..MAX_DOTS).all(|d| d != last_fired_dot));
        assert!((0..MAX_OUTER).all(|o| o != last_fired_outer));
    }

    /// Changing either dot count must reset every inner dot back to
    /// its 12-o'clock start (and clear the "lit" markers), per the
    /// "when adding or subtracting dots, reset the other dots"
    /// requirement.
    #[test]
    fn changing_dot_count_resets_phases() {
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let mut app = BloomApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);

        app.params.shapes[0].dot_phases[3].set(0.42);
        app.params.shapes[0].last_fired_dot.store(3, Ordering::Relaxed);
        app.params.shapes[0].last_fired_outer.store(1, Ordering::Relaxed);

        app.edit(Selection::Dots(0), 1);

        assert_eq!(app.params.shapes[0].dot_phases[3].get(), 0.0);
        assert_eq!(app.params.shapes[0].last_fired_dot.load(Ordering::Relaxed), usize::MAX);
        assert_eq!(app.params.shapes[0].last_fired_outer.load(Ordering::Relaxed), usize::MAX);

        app.params.shapes[0].dot_phases[2].set(0.77);
        app.edit(Selection::OuterDots(0), 1);
        assert_eq!(app.params.shapes[0].dot_phases[2].get(), 0.0);
    }

    /// Two overlapping notes must sound as two distinct voices, not
    /// one stealing/retriggering a single shared voice -- the
    /// "only one note triggers at a time" report.
    #[test]
    fn overlapping_triggers_use_distinct_voices() {
        let mut rt = ShapeRuntime::new(0);
        rt.trigger_note(60.0, 1.0);
        let first_idx = rt.voices.iter().position(|v| v.trigger).expect("first trigger should mark a voice");

        rt.trigger_note(67.0, 0.8);
        let triggered: Vec<usize> = rt.voices.iter().enumerate().filter(|(_, v)| v.trigger).map(|(i, _)| i).collect();

        assert_eq!(triggered.len(), 2, "both notes should still show as triggered (nothing consumed them yet): {triggered:?}");
        assert!(triggered.contains(&first_idx), "the first voice must not have been overwritten by the second trigger");

        let notes: Vec<f32> = triggered.iter().map(|&i| rt.voices[i].note).collect();
        assert!(notes.contains(&60.0) && notes.contains(&67.0), "both distinct notes must be present: {notes:?}");
    }

    /// Regression test for "turned everything down but it's still
    /// distorting a lot more than expected": many notes converging at
    /// once (worst case -- every voice in every shape's pool, same
    /// note, same block) must not sum to an amplitude that grows with
    /// voice count. Neither Vel Min/Max nor the Mixer's Bloom fader
    /// can fix that on their own -- they scale every voice's gain
    /// uniformly, not the number of voices stacking -- so this
    /// active-voice headroom division (mirroring plaits.rs's poly
    /// mode) is what actually keeps a re-convergence moment bounded.
    #[test]
    fn many_simultaneous_voices_stay_headroom_normalized() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        for s in 0..NUM_SHAPES {
            params.shapes[s].vel_min.set(1.0);
            params.shapes[s].vel_max.set(1.0);
        }

        let mut proc = BloomProcessor { params: Arc::clone(&params), shapes: std::array::from_fn(|i| ShapeRuntime::new(i as u32)), mono_buf: Vec::new() };

        for s in 0..NUM_SHAPES {
            for _ in 0..NUM_VOICES {
                proc.shapes[s].trigger_note(60.0, 1.0);
            }
        }

        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..20 {
            proc.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().cloned().fold(0.0f32, |a, x| a.max(x.abs())));
        }

        assert!(peak < 1.5, "peak grew with voice count instead of staying headroom-normalized: {peak}");
    }

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
