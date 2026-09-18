//! A physics-based generative instrument -- not a rotating/orbiting
//! sequencer like Bloom or a phase-drifting polygon like Madness, but
//! a small 2D gravity simulation: particles drift freely inside a
//! circular arena, pulled toward up to 3 fixed gravity wells. A
//! particle that gets close enough to an *active* well "falls in,"
//! fires a note, and gets flung back outward with a fresh burst of
//! speed -- so unlike Bloom/Madness's fully deterministic rotation,
//! this keeps generating genuinely emergent, non-repeating rhythms
//! and melodies for as long as it runs, without ever needing to be
//! reseeded (though `Randomize` will happily scatter it fresh anyway).
//!
//! Every well sits at a fixed angle (like Bloom's outer dots) and
//! maps to its own scale degree, spread evenly across the configured
//! Scale/Root/Octave Range. A capture's note gets a small upward
//! pitch scatter, and its velocity (loudness), both driven by how
//! fast the particle was moving at the moment of capture -- the
//! harder the impact, the brighter and louder the hit, same physical
//! intuition as a real collision.
//!
//! The simulation itself is deliberately simple and heavily damped
//! rather than "realistic": gravity is a softened inverse-square pull
//! (never singular), particles bounce elastically off the arena's
//! circular wall, velocity bleeds off continuously (Damping), and a
//! hard speed ceiling (`MAX_SPEED`) guarantees the whole system stays
//! bounded no matter how the Gravity/Damping/Capture Radius knobs are
//! set -- the same kind of unconditional safety clamp Bloom's voice
//! pool learned it needed the hard way (see its headroom fix), just
//! applied to physics here instead of audio gain.
//!
//! One shared 8-voice polyphonic pool (same round-robin voice-stealing
//! pattern as Bloom/Madness), one Engine/Harmonics/Timbre/Decay
//! "instrument" for the whole arena, registered on AudioBus/MixerBus/
//! ModBus exactly like every other audio-producing app -- Clouds or
//! Prism can granulate this generative texture same as anything else.

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
use embedded_graphics::primitives::{Circle, PrimitiveStyle};
use embedded_graphics::text::Text;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const MIN_PARTICLES: usize = 2;
const MAX_PARTICLES: usize = 16;
const DEFAULT_PARTICLES: usize = 8;
const NUM_WELLS: usize = 3;
const NUM_VOICES: usize = 8;

/// Normalized simulation space -- the arena is a circle of this
/// radius centered on the origin; screen mapping happens only in `draw`.
const ARENA_RADIUS: f32 = 1.0;
/// How far out from center the (fixed) wells sit, as a fraction of
/// `ARENA_RADIUS` -- inside the wall, leaving room for particles to
/// swing around a well without immediately hitting the boundary.
const WELL_RADIUS_FRAC: f32 = 0.62;

const MIN_GRAVITY: f32 = 0.0;
const MAX_GRAVITY: f32 = 1.0;
const DEFAULT_GRAVITY: f32 = 0.5;
/// Knob 0..1 scales to an actual force constant this way.
const GRAVITY_FORCE_SCALE: f32 = 0.18;

const MIN_DAMPING: f32 = 0.0;
const MAX_DAMPING: f32 = 1.0;
const DEFAULT_DAMPING: f32 = 0.35;
/// Knob 0..1 maps to an actual per-second damping coefficient in this
/// range -- 0 knob is nearly frictionless (long, wild, chaotic runs),
/// 1 knob settles the system down quickly.
const MIN_DAMPING_COEF: f32 = 0.05;
const MAX_DAMPING_COEF: f32 = 2.5;

const MIN_CAPTURE_RADIUS: f32 = 0.04;
const MAX_CAPTURE_RADIUS: f32 = 0.3;
const DEFAULT_CAPTURE_RADIUS: f32 = 0.12;

/// Unconditional safety ceiling on particle speed -- checked and
/// clamped every simulation step regardless of how Gravity/Damping/
/// Capture Radius are tuned, so nothing can ever diverge. Same
/// "always clamp, don't just trust the tuning" lesson as Bloom's
/// voice-pool headroom fix (see the module doc comment).
const MAX_SPEED: f32 = 3.0;
/// Outward speed a capture kicks a particle back out at.
const CAPTURE_KICK_SPEED: f32 = 1.0;
/// How long (seconds) a just-captured particle is immune to
/// recapturing -- without this, a particle that settles right at a
/// well's capture radius would fire on every single sample.
const CAPTURE_COOLDOWN_SECONDS: f32 = 0.12;
/// Velocity retained after bouncing off the arena wall -- slightly
/// lossy, so energy the wall doesn't just perfectly conserve forever.
const BOUNCE_DAMPING: f32 = 0.85;

const MIN_OCTAVE_RANGE: u32 = 1;
const MAX_OCTAVE_RANGE: u32 = 4;
const DEFAULT_OCTAVE_RANGE: u32 = 2;

/// Same scale-degree mapping Bloom uses, duplicated locally rather
/// than shared -- it's a few lines, and the two apps' notions of
/// "position" (radius fraction there, well index here) are different
/// enough that sharing the function wouldn't simplify much.
fn note_for_position(pos: usize, scale_idx: usize, root: u32) -> i32 {
    let intervals = SCALE_TYPES[scale_idx % SCALE_TYPES.len()].1;
    let len = intervals.len().max(1);
    let degree = pos % len;
    let octave = pos / len;
    60 + root as i32 + intervals[degree] + octave as i32 * 12
}

/// A fixed well position -- well 0 at 12 o'clock, evenly spaced
/// clockwise from there, same convention Bloom's outer dots use.
fn well_position(w: usize) -> (f32, f32) {
    let angle = -std::f32::consts::FRAC_PI_2 + w as f32 / NUM_WELLS as f32 * TAU;
    (angle.cos() * ARENA_RADIUS * WELL_RADIUS_FRAC, angle.sin() * ARENA_RADIUS * WELL_RADIUS_FRAC)
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * (max - min) * 0.05).clamp(min, max);
    value.set(next);
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Running,
    ParticleCount,
    Gravity,
    Damping,
    CaptureRadius,
    WellActive(usize),
    Scale,
    Root,
    OctaveRange,
    VelMin,
    VelMax,
    Engine,
    Harmonics,
    Timbre,
    Decay,
    Randomize,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 3;

struct Params {
    running: AtomicBool,
    particle_count: AtomicUsize,
    gravity: AtomicF32,
    ext_gravity: Arc<AtomicF32>,
    damping: AtomicF32,
    ext_damping: Arc<AtomicF32>,
    capture_radius: AtomicF32,
    well_active: [AtomicBool; NUM_WELLS],
    scale: AtomicU32,
    root: AtomicU32,
    octave_range: AtomicU32,
    vel_min: AtomicF32,
    vel_max: AtomicF32,
    engine: AtomicU32,
    harmonics: AtomicF32,
    timbre: AtomicF32,
    decay: AtomicF32,
    /// Every particle's current position, shared between the audio
    /// thread (which owns the simulation) and the UI thread (which
    /// just draws wherever they currently are) -- same "physics/phase
    /// state lives in shared atomics, everything else stays
    /// processor-local" split Bloom uses for its dot phases.
    particle_x: [AtomicF32; MAX_PARTICLES],
    particle_y: [AtomicF32; MAX_PARTICLES],
    /// Current speed, purely for `draw`'s size/brightness cue --
    /// velocity's x/y components themselves stay processor-local.
    particle_speed: [AtomicF32; MAX_PARTICLES],
    /// Which well last captured a particle -- usize::MAX (not 0)
    /// before the first capture, same sentinel convention Bloom's
    /// `last_fired_dot`/`last_fired_outer` use, so well 0 doesn't look
    /// permanently lit from frame one.
    last_fired_well: AtomicUsize,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Nebula", modbus);
        // Golden-angle spiral seed: distinct, non-degenerate starting
        // positions (spread across both angle and radius) rather than
        // every particle defaulting to (0,0) -- the exact center is
        // an unstable equilibrium of 3 symmetric wells, so an
        // all-zero start could otherwise sit frozen there forever.
        const GOLDEN_ANGLE: f32 = 2.399_963_2;
        Self {
            running: AtomicBool::new(false),
            particle_count: AtomicUsize::new(DEFAULT_PARTICLES),
            gravity: AtomicF32::new(DEFAULT_GRAVITY),
            ext_gravity: modbus.register("Nebula: Gravity".to_string()),
            damping: AtomicF32::new(DEFAULT_DAMPING),
            ext_damping: modbus.register("Nebula: Damping".to_string()),
            capture_radius: AtomicF32::new(DEFAULT_CAPTURE_RADIUS),
            well_active: std::array::from_fn(|_| AtomicBool::new(true)),
            scale: AtomicU32::new(0),
            root: AtomicU32::new(0),
            octave_range: AtomicU32::new(DEFAULT_OCTAVE_RANGE),
            vel_min: AtomicF32::new(0.5),
            vel_max: AtomicF32::new(1.0),
            engine: AtomicU32::new(18), // "Particle" -- thematically fitting default
            harmonics: AtomicF32::new(0.5),
            timbre: AtomicF32::new(0.5),
            decay: AtomicF32::new(0.4),
            particle_x: std::array::from_fn(|i| {
                let r = 0.2 + 0.5 * (i as f32 / MAX_PARTICLES as f32);
                AtomicF32::new(r * (i as f32 * GOLDEN_ANGLE).cos())
            }),
            particle_y: std::array::from_fn(|i| {
                let r = 0.2 + 0.5 * (i as f32 / MAX_PARTICLES as f32);
                AtomicF32::new(r * (i as f32 * GOLDEN_ANGLE).sin())
            }),
            particle_speed: std::array::from_fn(|_| AtomicF32::new(0.0)),
            last_fired_well: AtomicUsize::new(usize::MAX),
            bus_out: audio_bus.register("Nebula"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct NebulaApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    rng: u32,
}

impl NebulaApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(12345);
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
            rng: seed | 1,
        }
    }

    fn next_rand01(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    /// Scatters every particle to a fresh random position (within the
    /// arena) with a small random velocity -- unlike Bloom/Madness's
    /// Randomize (which reassigns fixed structural parameters), this
    /// just re-seeds a chaotic system's initial conditions, since the
    /// simulation itself never needs "resetting" to keep generating.
    fn randomize(&mut self) {
        for i in 0..MAX_PARTICLES {
            let r = self.next_rand01() * ARENA_RADIUS * 0.8;
            let angle = self.next_rand01() * TAU;
            self.params.particle_x[i].set(r * angle.cos());
            self.params.particle_y[i].set(r * angle.sin());
        }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => {
                let mut v = vec![Selection::Running, Selection::ParticleCount, Selection::Gravity, Selection::Damping, Selection::CaptureRadius];
                for w in 0..NUM_WELLS {
                    v.push(Selection::WellActive(w));
                }
                v
            }
            1 => vec![Selection::Scale, Selection::Root, Selection::OctaveRange, Selection::VelMin, Selection::VelMax],
            _ => vec![Selection::Engine, Selection::Harmonics, Selection::Timbre, Selection::Decay, Selection::Randomize],
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
            0 => "Physics",
            1 => "Notes",
            _ => "Voice",
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => {
                let n = self.params.particle_count.load(Ordering::Relaxed);
                if self.params.running.load(Ordering::Relaxed) { format!("running, {n} particles") } else { format!("stopped, {n} particles") }
            }
            1 => {
                let idx = self.params.scale.load(Ordering::Relaxed) as usize % SCALE_TYPES.len();
                format!("{} {}", ROOT_NAMES[self.params.root.load(Ordering::Relaxed) as usize % 12], SCALE_TYPES[idx].0)
            }
            _ => ENGINE_NAMES[self.params.engine.load(Ordering::Relaxed) as usize % ENGINE_NAMES.len()].to_string(),
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Running => "Running".into(),
            Selection::ParticleCount => "Particles".into(),
            Selection::Gravity => "Gravity".into(),
            Selection::Damping => "Damping".into(),
            Selection::CaptureRadius => "Capture Radius".into(),
            Selection::WellActive(w) => format!("Well {} Active", w + 1),
            Selection::Scale => "Scale".into(),
            Selection::Root => "Root".into(),
            Selection::OctaveRange => "Octave Range".into(),
            Selection::VelMin => "Vel Min".into(),
            Selection::VelMax => "Vel Max".into(),
            Selection::Engine => "Engine".into(),
            Selection::Harmonics => "Harmonics".into(),
            Selection::Timbre => "Timbre".into(),
            Selection::Decay => "Decay".into(),
            Selection::Randomize => "Randomize".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Running => {
                if self.params.running.load(Ordering::Relaxed) { "running".into() } else { "stopped".into() }
            }
            Selection::ParticleCount => format!("{}", self.params.particle_count.load(Ordering::Relaxed)),
            Selection::Gravity => format!("{:.2}", self.params.gravity.get()),
            Selection::Damping => format!("{:.2}", self.params.damping.get()),
            Selection::CaptureRadius => format!("{:.2}", self.params.capture_radius.get()),
            Selection::WellActive(w) => {
                if self.params.well_active[w].load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::Scale => {
                let idx = self.params.scale.load(Ordering::Relaxed) as usize % SCALE_TYPES.len();
                SCALE_TYPES[idx].0.to_string()
            }
            Selection::Root => ROOT_NAMES[self.params.root.load(Ordering::Relaxed) as usize % 12].to_string(),
            Selection::OctaveRange => format!("{}", self.params.octave_range.load(Ordering::Relaxed)),
            Selection::VelMin => format!("{:.2}", self.params.vel_min.get()),
            Selection::VelMax => format!("{:.2}", self.params.vel_max.get()),
            Selection::Engine => ENGINE_NAMES[self.params.engine.load(Ordering::Relaxed) as usize % ENGINE_NAMES.len()].to_string(),
            Selection::Harmonics => format!("{:.2}", self.params.harmonics.get()),
            Selection::Timbre => format!("{:.2}", self.params.timbre.get()),
            Selection::Decay => format!("{:.2}", self.params.decay.get()),
            Selection::Randomize => "press knob2".into(),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::Running => self.params.running.store(delta > 0, Ordering::Relaxed),
            Selection::ParticleCount => {
                let cur = self.params.particle_count.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_PARTICLES as i32, MAX_PARTICLES as i32);
                self.params.particle_count.store(next as usize, Ordering::Relaxed);
            }
            Selection::Gravity => bump(&self.params.gravity, delta, sensitivity, MIN_GRAVITY, MAX_GRAVITY),
            Selection::Damping => bump(&self.params.damping, delta, sensitivity, MIN_DAMPING, MAX_DAMPING),
            Selection::CaptureRadius => bump(&self.params.capture_radius, delta, sensitivity, MIN_CAPTURE_RADIUS, MAX_CAPTURE_RADIUS),
            Selection::WellActive(w) => self.params.well_active[w].store(delta > 0, Ordering::Relaxed),
            Selection::Scale => {
                let cur = self.params.scale.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(SCALE_TYPES.len() as i32);
                self.params.scale.store(next as u32, Ordering::Relaxed);
            }
            Selection::Root => {
                let cur = self.params.root.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(12);
                self.params.root.store(next as u32, Ordering::Relaxed);
            }
            Selection::OctaveRange => {
                let cur = self.params.octave_range.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_OCTAVE_RANGE as i32, MAX_OCTAVE_RANGE as i32);
                self.params.octave_range.store(next as u32, Ordering::Relaxed);
            }
            Selection::VelMin => bump(&self.params.vel_min, delta, sensitivity, 0.0, 1.0),
            Selection::VelMax => bump(&self.params.vel_max, delta, sensitivity, 0.0, 1.0),
            Selection::Engine => {
                let cur = self.params.engine.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(ENGINE_NAMES.len() as i32);
                self.params.engine.store(next as u32, Ordering::Relaxed);
            }
            Selection::Harmonics => bump(&self.params.harmonics, delta, sensitivity, 0.0, 1.0),
            Selection::Timbre => bump(&self.params.timbre, delta, sensitivity, 0.0, 1.0),
            Selection::Decay => bump(&self.params.decay, delta, sensitivity, 0.0, 1.0),
            Selection::Randomize => {} // press-only, see `reset`
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Running => self.params.running.store(false, Ordering::Relaxed),
            Selection::ParticleCount => self.params.particle_count.store(DEFAULT_PARTICLES, Ordering::Relaxed),
            Selection::Gravity => self.params.gravity.set(DEFAULT_GRAVITY),
            Selection::Damping => self.params.damping.set(DEFAULT_DAMPING),
            Selection::CaptureRadius => self.params.capture_radius.set(DEFAULT_CAPTURE_RADIUS),
            Selection::WellActive(w) => self.params.well_active[w].store(true, Ordering::Relaxed),
            Selection::OctaveRange => self.params.octave_range.store(DEFAULT_OCTAVE_RANGE, Ordering::Relaxed),
            Selection::VelMin => self.params.vel_min.set(0.5),
            Selection::VelMax => self.params.vel_max.set(1.0),
            Selection::Harmonics => self.params.harmonics.set(0.5),
            Selection::Timbre => self.params.timbre.set(0.5),
            Selection::Decay => self.params.decay.set(0.4),
            Selection::Randomize => self.randomize(),
            Selection::Scale | Selection::Root | Selection::Engine => {} // no single sensible default
        }
    }
}

impl App for NebulaApp {
    fn running(&self) -> Option<bool> {
        Some(self.params.running.load(Ordering::Relaxed))
    }

    fn toggle_running(&mut self) {
        let cur = self.params.running.load(Ordering::Relaxed);
        self.params.running.store(!cur, Ordering::Relaxed);
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
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(NebulaProcessor {
            params: Arc::clone(&self.params),
            voices: std::array::from_fn(|_| PolyVoice::new()),
            voice_gen: 0,
            vx: [0.0; MAX_PARTICLES],
            vy: [0.0; MAX_PARTICLES],
            cooldown: [0.0; MAX_PARTICLES],
            mono_buf: Vec::new(),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let title = MonoTextStyle::new(&SPLEEN_16X32, Rgb565::WHITE);
        Text::new("Nebula", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(0, 63, 10));
        let dim = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));

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
        self.list.draw(fb, 16, 44, 24, 10, &display_rows);

        // --- Right: the arena -- boundary circle, wells, particles. ---
        let center = Point::new(500, 175);
        let px_radius = 105i32;
        Circle::with_center(center, (px_radius * 2) as u32)
            .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(12, 24, 12), 1))
            .draw(fb)
            .ok();

        let last_fired = self.params.last_fired_well.load(Ordering::Relaxed);
        for w in 0..NUM_WELLS {
            let (wx, wy) = well_position(w);
            let point = Point::new(center.x + (wx * px_radius as f32) as i32, center.y + (wy * px_radius as f32) as i32);
            let active = self.params.well_active[w].load(Ordering::Relaxed);
            let lit = w == last_fired;
            let color = if !active {
                Rgb565::new(8, 8, 8)
            } else if lit {
                Rgb565::new(0, 63, 30)
            } else {
                Rgb565::new(0, 30, 50)
            };
            Circle::with_center(point, if lit { 14 } else { 10 })
                .into_styled(PrimitiveStyle::with_stroke(color, if lit { 3 } else { 2 }))
                .draw(fb)
                .ok();
        }

        for i in 0..self.params.particle_count.load(Ordering::Relaxed).clamp(MIN_PARTICLES, MAX_PARTICLES) {
            let x = self.params.particle_x[i].get();
            let y = self.params.particle_y[i].get();
            let speed = self.params.particle_speed[i].get();
            let point = Point::new(center.x + (x * px_radius as f32) as i32, center.y + (y * px_radius as f32) as i32);
            let brightness = (20.0 + (speed / MAX_SPEED).clamp(0.0, 1.0) * 43.0) as u8;
            let color = Rgb565::new(brightness / 3, brightness, brightness / 2);
            Circle::with_center(point, 5)
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(fb)
                .ok();
        }

        Text::new(&format!("{} particles, {} wells active", self.params.particle_count.load(Ordering::Relaxed), (0..NUM_WELLS).filter(|&w| self.params.well_active[w].load(Ordering::Relaxed)).count()), Point::new(360, 300), accent)
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

/// One of the shared pool's polyphonic voices -- same shape as
/// Bloom/Madness's `PolyVoice`.
struct PolyVoice {
    voice: PlaitsVoice,
    voice_buf: Vec<f32>,
    note: f32,
    velocity: f32,
    trigger: bool,
    last_used: u64,
}

impl PolyVoice {
    fn new() -> Self {
        Self { voice: PlaitsVoice::new(), voice_buf: Vec::new(), note: 60.0, velocity: 1.0, trigger: false, last_used: 0 }
    }
}

struct NebulaProcessor {
    params: Arc<Params>,
    voices: [PolyVoice; NUM_VOICES],
    voice_gen: u64,
    /// Processor-local velocity -- position lives in `Params` (see its
    /// doc comment), velocity doesn't need to be UI-visible.
    vx: [f32; MAX_PARTICLES],
    vy: [f32; MAX_PARTICLES],
    /// Seconds remaining before a just-captured particle can be
    /// recaptured -- without this, a particle sitting right at a
    /// well's capture radius would fire every single sample.
    cooldown: [f32; MAX_PARTICLES],
    mono_buf: Vec<f32>,
}

impl NebulaProcessor {
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

impl AudioProcessor for NebulaProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        self.mono_buf.clear();
        self.mono_buf.resize(frames, 0.0);
        let dt = 1.0 / sample_rate;

        if self.params.running.load(Ordering::Relaxed) {
            let n = self.params.particle_count.load(Ordering::Relaxed).clamp(MIN_PARTICLES, MAX_PARTICLES);
            let gravity = (self.params.gravity.get() + self.params.ext_gravity.get()).clamp(MIN_GRAVITY, MAX_GRAVITY) * GRAVITY_FORCE_SCALE;
            let damping_knob = (self.params.damping.get() + self.params.ext_damping.get()).clamp(MIN_DAMPING, MAX_DAMPING);
            let damping_coef = MIN_DAMPING_COEF + damping_knob * (MAX_DAMPING_COEF - MIN_DAMPING_COEF);
            let capture_radius = self.params.capture_radius.get().clamp(MIN_CAPTURE_RADIUS, MAX_CAPTURE_RADIUS);
            let well_pos: [(f32, f32); NUM_WELLS] = std::array::from_fn(well_position);
            let well_active: [bool; NUM_WELLS] = std::array::from_fn(|w| self.params.well_active[w].load(Ordering::Relaxed));

            let scale_idx = self.params.scale.load(Ordering::Relaxed) as usize;
            let root = self.params.root.load(Ordering::Relaxed);
            let octave_range = self.params.octave_range.load(Ordering::Relaxed).max(MIN_OCTAVE_RANGE);
            let scale_len = SCALE_TYPES[scale_idx % SCALE_TYPES.len()].1.len().max(1);
            let total_positions = (scale_len * octave_range as usize).max(1);
            let vmin = self.params.vel_min.get().min(self.params.vel_max.get());
            let vmax = self.params.vel_min.get().max(self.params.vel_max.get());

            for _ in 0..frames {
                for i in 0..n {
                    if self.cooldown[i] > 0.0 {
                        self.cooldown[i] -= dt;
                    }
                    let mut px = self.params.particle_x[i].get();
                    let mut py = self.params.particle_y[i].get();
                    let mut vx = self.vx[i];
                    let mut vy = self.vy[i];

                    let mut ax = 0.0f32;
                    let mut ay = 0.0f32;
                    let mut captured_well: Option<usize> = None;
                    for w in 0..NUM_WELLS {
                        if !well_active[w] {
                            continue;
                        }
                        let (wx, wy) = well_pos[w];
                        let dx = wx - px;
                        let dy = wy - py;
                        let dist2 = dx * dx + dy * dy;
                        let dist = dist2.sqrt().max(1e-4);
                        if dist < capture_radius && self.cooldown[i] <= 0.0 {
                            captured_well = Some(w);
                            break;
                        }
                        let denom = dist2.max(capture_radius * capture_radius * 0.5);
                        let force = gravity / denom;
                        ax += force * dx / dist;
                        ay += force * dy / dist;
                    }

                    if let Some(w) = captured_well {
                        let (wx, wy) = well_pos[w];
                        let dx = px - wx;
                        let dy = py - wy;
                        let dist = (dx * dx + dy * dy).sqrt().max(1e-4);
                        let speed_before = (vx * vx + vy * vy).sqrt();
                        let norm_speed = (speed_before / MAX_SPEED).clamp(0.0, 1.0);
                        vx = dx / dist * CAPTURE_KICK_SPEED;
                        vy = dy / dist * CAPTURE_KICK_SPEED;
                        self.cooldown[i] = CAPTURE_COOLDOWN_SECONDS;

                        let degree_pos = ((w as f32 / NUM_WELLS as f32) * total_positions as f32) as usize % total_positions;
                        let note = note_for_position(degree_pos, scale_idx, root) as f32 + (norm_speed * 4.0).round();
                        let velocity = vmin + norm_speed * (vmax - vmin);
                        self.trigger_note(note, velocity);
                        self.params.last_fired_well.store(w, Ordering::Relaxed);
                    } else {
                        vx += ax * dt;
                        vy += ay * dt;
                    }

                    let damp_mult = (1.0 - damping_coef * dt).max(0.0);
                    vx *= damp_mult;
                    vy *= damp_mult;

                    // Unconditional safety ceiling -- see module doc comment.
                    let speed = (vx * vx + vy * vy).sqrt();
                    if speed > MAX_SPEED {
                        let scale = MAX_SPEED / speed;
                        vx *= scale;
                        vy *= scale;
                    }

                    px += vx * dt;
                    py += vy * dt;

                    let r = (px * px + py * py).sqrt();
                    if r > ARENA_RADIUS {
                        let nx = px / r;
                        let ny = py / r;
                        let vdotn = vx * nx + vy * ny;
                        vx -= 2.0 * vdotn * nx;
                        vy -= 2.0 * vdotn * ny;
                        vx *= BOUNCE_DAMPING;
                        vy *= BOUNCE_DAMPING;
                        px = nx * ARENA_RADIUS;
                        py = ny * ARENA_RADIUS;
                    }

                    self.vx[i] = vx;
                    self.vy[i] = vy;
                    self.params.particle_x[i].set(px);
                    self.params.particle_y[i].set(py);
                    self.params.particle_speed[i].set((vx * vx + vy * vy).sqrt());
                }
            }
        }

        let engine = self.params.engine.load(Ordering::Relaxed) as i32;
        let harmonics = self.params.harmonics.get();
        let timbre = self.params.timbre.get();
        let decay = self.params.decay.get();

        // Same active-voice headroom normalization Bloom's voice pool
        // needed (see the module doc comment) -- render every voice,
        // count how many actually produced audible signal this block,
        // and divide the sum by that count so a burst of simultaneous
        // captures can't sum to an amplitude that scales with voice
        // count.
        let mut active_voices: usize = 0;
        for v in self.voices.iter_mut() {
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
            v.trigger = false;

            let peak = v.voice_buf.iter().fold(0.0f32, |a, s| a.max(s.abs()));
            if peak > 1e-4 {
                active_voices += 1;
            }

            let gain = v.velocity;
            for (m, sample) in self.mono_buf.iter_mut().zip(v.voice_buf.iter()) {
                *m += *sample * gain;
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

    fn new_app() -> NebulaApp {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        NebulaApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus)
    }

    fn new_processor(params: Arc<Params>) -> NebulaProcessor {
        NebulaProcessor {
            params,
            voices: std::array::from_fn(|_| PolyVoice::new()),
            voice_gen: 0,
            vx: [0.0; MAX_PARTICLES],
            vy: [0.0; MAX_PARTICLES],
            cooldown: [0.0; MAX_PARTICLES],
            mono_buf: Vec::new(),
        }
    }

    /// However Gravity/Damping/Capture Radius are tuned, and however
    /// long it runs, every particle must stay within the arena --
    /// this is what `MAX_SPEED` plus the wall bounce are for.
    #[test]
    fn particles_stay_within_arena_under_sustained_running() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.running.store(true, Ordering::Relaxed);
        params.gravity.set(MAX_GRAVITY); // worst case
        params.damping.set(MIN_DAMPING); // worst case -- least energy loss
        params.capture_radius.set(MIN_CAPTURE_RADIUS); // rare captures -- more time building speed

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..500 {
            proc.process(&mut buffer, 2, 48000.0);
        }

        for i in 0..MAX_PARTICLES {
            let x = params.particle_x[i].get();
            let y = params.particle_y[i].get();
            assert!(x.is_finite() && y.is_finite(), "particle {i} diverged to non-finite: ({x}, {y})");
            let r = (x * x + y * y).sqrt();
            assert!(r <= ARENA_RADIUS * 1.05, "particle {i} escaped the arena: r={r}");
        }
    }

    /// The whole point of a gravity well is that particles eventually
    /// fall into one and fire a note -- this must actually happen
    /// within a generous but bounded amount of simulated time at
    /// default settings.
    #[test]
    fn particles_eventually_get_captured_and_fire() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.running.store(true, Ordering::Relaxed);

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..2000 {
            proc.process(&mut buffer, 2, 48000.0);
        }

        assert_ne!(params.last_fired_well.load(Ordering::Relaxed), usize::MAX, "expected at least one capture within 2000 blocks at default settings");
        assert!(proc.voice_gen > 0, "expected at least one voice trigger");
    }

    /// Deactivating every well must stop captures entirely -- proves
    /// `WellActive` actually gates the simulation, not just the
    /// visual.
    #[test]
    fn inactive_wells_never_capture() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.running.store(true, Ordering::Relaxed);
        for w in 0..NUM_WELLS {
            params.well_active[w].store(false, Ordering::Relaxed);
        }

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..2000 {
            proc.process(&mut buffer, 2, 48000.0);
        }

        assert_eq!(params.last_fired_well.load(Ordering::Relaxed), usize::MAX, "no well should ever capture while all are inactive");
        assert_eq!(proc.voice_gen, 0, "no voice should ever trigger while all wells are inactive");
    }

    /// Randomize must actually move particles, not be a no-op.
    #[test]
    fn randomize_moves_particles() {
        let mut app = new_app();
        let before: Vec<(f32, f32)> = (0..MAX_PARTICLES).map(|i| (app.params.particle_x[i].get(), app.params.particle_y[i].get())).collect();
        app.reset(Selection::Randomize);
        let after: Vec<(f32, f32)> = (0..MAX_PARTICLES).map(|i| (app.params.particle_x[i].get(), app.params.particle_y[i].get())).collect();
        assert_ne!(before, after, "Randomize should move at least one particle");
    }

    /// A burst of many simultaneous captures must not sum to an
    /// amplitude that scales with voice count -- the same headroom
    /// regression Bloom hit (see the module doc comment).
    #[test]
    fn many_simultaneous_captures_stay_headroom_normalized() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.vel_min.set(1.0);
        params.vel_max.set(1.0);

        let mut proc = new_processor(Arc::clone(&params));
        for _ in 0..NUM_VOICES {
            proc.trigger_note(60.0, 1.0);
        }

        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..20 {
            proc.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().cloned().fold(0.0f32, |a, x| a.max(x.abs())));
        }
        assert!(peak < 1.5, "peak grew with voice count instead of staying headroom-normalized: {peak}");
    }

    /// The Mixer app's channel fader must only affect what reaches
    /// the device output, not what this app publishes to audio_bus.rs
    /// for another app (Clouds, Prism) to tap -- same contract every
    /// other audio-producing app in this build guarantees.
    #[test]
    fn mixer_fader_does_not_affect_audio_bus_publish() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.mix_level.set(0.0); // fully down in the Mixer app
        params.vel_min.set(1.0);
        params.vel_max.set(1.0);

        let mut proc = new_processor(Arc::clone(&params));
        proc.trigger_note(60.0, 1.0);

        let mut buffer = vec![0.0f32; 512 * 2];
        proc.process(&mut buffer, 2, 48000.0);

        let device_peak = buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert_eq!(device_peak, 0.0, "device output should be silent with mix_level at 0");

        let bus_out = params.bus_out.lock().unwrap();
        let bus_peak = bus_out.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(bus_peak > 0.0, "audio_bus publish should be unaffected by the Mixer channel fader");
    }

    /// The freshly-constructed, untouched `Params` -- the exact state
    /// a brand new app instance starts in -- must default to
    /// stopped. With N apps all loaded (and someday possibly far
    /// more), an app that defaults to *running* is a real risk on
    /// its own, not just an inconvenience.
    #[test]
    fn defaults_to_stopped() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Params::new(&modbus, &audio_bus, &mixer_bus);
        assert!(!params.running.load(Ordering::Relaxed), "Nebula must start stopped, not running");
    }
}
