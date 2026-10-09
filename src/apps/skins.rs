//! Skins: a drum synthesizer with per-step sound locks.
//!
//! Eight synthesized voices -- kick, snare, clap, closed and open hats,
//! tom, rim and metal (cowbell to cymbal) -- each with six sound controls
//! plus level and pan. Nothing is a sample: the kick is a sine whose pitch
//! sweeps down, the snare two tuned sines and high-passed noise, the clap
//! band-passed noise struck in bursts, the hats and metal the TR-808's
//! six detuned square waves through filters, so every control changes
//! the sound the way it would on an analog drum machine.
//!
//! Each voice has its own 16-step lane, and its own length (1-16): set
//! the hats to 12 against a 16-step kick and the pattern shifts every bar
//! (polymeter). Every step can be accented, ratcheted (2-4 hits within
//! the step), given a chance, and can lock any of the voice's six
//! controls to its own value for that step only -- a higher tom, a longer
//! hat, a dirtier kick on the last beat.
//!
//! Views (F2): STEP (pads = the selected voice's steps; hold one and use
//! the D-pad to accent, ratchet, chance or lock it), PLAY (pads 1-8 play
//! the voices, 9-16 mute them; SELECT records what you play), SOUND (the
//! selected voice's controls, the tempo, swing and the kits).
//!
//! Skins is also an instrument: notes from MIDI or other apps (C2 = kick,
//! up in semitones) play the voices, so Session's drum tracks can use it.

use crate::app::{App, Input, SlintExtra};
use crate::apps::kids_kit::{self as kit, Extra, Noise, Size2, Sound, Svf};
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::note_bus::{NoteBus, NoteInboxRef, NoteView};
use embedded_graphics::pixelcolor::Rgb565;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Skins";
pub const VOICES: usize = 8;
const PARAMS: usize = 6;
const STEPS: usize = 16;
const VOICE_NAMES: [&str; VOICES] = ["Kick", "Snare", "Clap", "Closed hat", "Open hat", "Tom", "Rim", "Metal"];
const PARAM_NAMES: [[&str; PARAMS]; VOICES] = [
    ["Tune", "Decay", "Sweep", "Sweep time", "Click", "Drive"],
    ["Tune", "Decay", "Noise", "Snap", "Body", "Drive"],
    ["Tone", "Decay", "Spread", "Bursts", "Tail", "Drive"],
    ["Pitch", "Decay", "Color", "Ring", "Noise", "Drive"],
    ["Pitch", "Decay", "Color", "Ring", "Noise", "Drive"],
    ["Tune", "Decay", "Sweep", "Sweep time", "Noise", "Drive"],
    ["Tune", "Decay", "Tone", "Click", "Noise", "Drive"],
    ["Tune", "Decay", "Ratio", "Ring", "Noise", "Drive"],
];
const COLORS: [(u8, u8, u8); VOICES] = [(240, 90, 80), (250, 170, 60), (240, 220, 80), (130, 220, 110), (80, 210, 200), (90, 150, 240), (160, 120, 240), (230, 120, 200)];
/// The TR-808's six hat oscillators, Hz.
const METAL_HZ: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub struct VoiceSet {
    pub p: [f32; PARAMS],
    pub level: f32,
    pub pan: f32,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Default)]
pub struct StepData {
    pub on: bool,
    pub accent: bool,
    /// Hits within the step, 1-4.
    pub ratchet: u8,
    /// Percent; 0 is read as 100.
    pub chance: u8,
    pub locks: [Option<f32>; PARAMS],
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Lane {
    pub steps: [StepData; STEPS],
    pub length: usize,
    pub mute: bool,
}

impl Default for Lane {
    fn default() -> Self {
        Lane { steps: [StepData::default(); STEPS], length: 16, mute: false }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Kit {
    pub name: String,
    pub voices: [VoiceSet; VOICES],
    pub lanes: Vec<Lane>,
}

fn vs(p: [f32; PARAMS], level: f32, pan: f32) -> VoiceSet {
    VoiceSet { p, level, pan }
}

fn lane(pattern: &str, accents: &str) -> Lane {
    let mut l = Lane::default();
    for (i, c) in pattern.chars().enumerate().take(STEPS) {
        l.steps[i].on = c != '.';
        l.steps[i].ratchet = if c == 'r' { 2 } else { 1 };
    }
    for (i, c) in accents.chars().enumerate().take(STEPS) {
        l.steps[i].accent = c == '>';
    }
    l
}

/// Four kits to start from.
pub fn factory() -> Vec<Kit> {
    let groove = || vec![lane("x.........x.....", ">..............."), lane("....x.......x...", ""), lane("............x...", ""), lane("x.x.x.x.x.x.x.xr", ""), lane("..............x.", ""), lane("..........x..x..", ""), lane("...x......x.....", ""), lane("", "")];
    vec![
        Kit {
            name: "Boom".into(),
            voices: [vs([0.25, 0.6, 0.55, 0.35, 0.3, 0.2], 0.9, 0.0), vs([0.4, 0.45, 0.6, 0.5, 0.35, 0.1], 0.75, 0.05), vs([0.45, 0.5, 0.5, 0.6, 0.5, 0.0], 0.7, -0.1), vs([0.5, 0.2, 0.6, 0.4, 0.1, 0.0], 0.5, 0.3), vs([0.5, 0.6, 0.6, 0.4, 0.1, 0.0], 0.45, 0.3), vs([0.35, 0.5, 0.4, 0.3, 0.1, 0.0], 0.7, -0.3), vs([0.5, 0.3, 0.5, 0.5, 0.2, 0.0], 0.5, 0.2), vs([0.45, 0.45, 0.45, 0.5, 0.0, 0.0], 0.45, -0.2)],
            lanes: groove(),
        },
        Kit {
            name: "Tight".into(),
            voices: [vs([0.4, 0.3, 0.4, 0.2, 0.6, 0.35], 0.9, 0.0), vs([0.55, 0.3, 0.7, 0.7, 0.2, 0.3], 0.75, 0.0), vs([0.55, 0.35, 0.35, 0.5, 0.3, 0.1], 0.7, 0.0), vs([0.65, 0.12, 0.75, 0.6, 0.2, 0.1], 0.5, 0.2), vs([0.65, 0.45, 0.75, 0.6, 0.2, 0.1], 0.45, 0.2), vs([0.5, 0.35, 0.3, 0.2, 0.2, 0.2], 0.7, -0.2), vs([0.6, 0.15, 0.6, 0.7, 0.2, 0.2], 0.5, 0.1), vs([0.6, 0.3, 0.6, 0.3, 0.1, 0.2], 0.45, -0.1)],
            lanes: vec![lane("x...x...x...x...", ">...>...>...>..."), lane("....x.......x...", ""), lane("....x.......x..x", ""), lane("..x...x...x...x.", ""), lane("", ""), lane("", ""), lane("x..x..x...x..x..", ""), lane("", "")],
        },
        Kit {
            name: "Dust".into(),
            voices: [vs([0.3, 0.5, 0.3, 0.4, 0.15, 0.7], 0.85, 0.0), vs([0.35, 0.55, 0.45, 0.35, 0.5, 0.6], 0.75, -0.1), vs([0.35, 0.6, 0.6, 0.7, 0.7, 0.4], 0.65, 0.1), vs([0.4, 0.25, 0.4, 0.2, 0.5, 0.5], 0.45, 0.35), vs([0.4, 0.7, 0.4, 0.2, 0.5, 0.5], 0.4, 0.35), vs([0.25, 0.6, 0.5, 0.5, 0.3, 0.4], 0.7, -0.3), vs([0.4, 0.4, 0.4, 0.3, 0.6, 0.5], 0.5, 0.2), vs([0.35, 0.6, 0.3, 0.6, 0.3, 0.4], 0.4, -0.2)],
            lanes: groove(),
        },
        Kit {
            name: "Metal".into(),
            voices: [vs([0.2, 0.7, 0.7, 0.5, 0.2, 0.5], 0.9, 0.0), vs([0.6, 0.4, 0.8, 0.85, 0.2, 0.3], 0.7, 0.0), vs([0.7, 0.3, 0.3, 0.3, 0.2, 0.3], 0.6, 0.0), vs([0.8, 0.15, 0.9, 0.8, 0.0, 0.3], 0.5, 0.25), vs([0.8, 0.8, 0.9, 0.8, 0.0, 0.3], 0.45, -0.25), vs([0.7, 0.4, 0.6, 0.15, 0.0, 0.3], 0.6, 0.0), vs([0.8, 0.2, 0.8, 0.8, 0.0, 0.3], 0.5, 0.0), vs([0.75, 0.7, 0.8, 0.8, 0.1, 0.4], 0.5, 0.0)],
            lanes: vec![lane("x..x..x...x..x..", ">.............."), lane("....x.......x...", ""), lane("", ""), lane("xxxxxxxxxxxxxxxx", ">.>.>.>.>.>.>.>."), lane("", ""), lane("..............rr", ""), lane("", ""), lane("x.......x.......", "")],
        },
    ]
}

/// One sounding drum.
#[derive(Clone)]
struct Hit {
    kind: usize,
    p: [f32; PARAMS],
    vel: f32,
    pan: f32,
    t: f32,
    ph: [f32; 6],
    f1: Svf,
    f2: Svf,
    noise: Noise,
    active: bool,
}

#[inline]
fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + (b - a) * x
}

/// Exponential map 0..1 -> lo..hi.
#[inline]
fn expmap(lo: f32, hi: f32, x: f32) -> f32 {
    lo * (hi / lo).powf(x.clamp(0.0, 1.0))
}

impl Hit {
    fn idle(seed: u32) -> Hit {
        Hit { kind: 0, p: [0.5; PARAMS], vel: 0.0, pan: 0.0, t: 0.0, ph: [0.0; 6], f1: Svf::default(), f2: Svf::default(), noise: Noise::new(seed), active: false }
    }

    fn decay(&self) -> f32 {
        let d = self.p[1];
        match self.kind {
            0 => expmap(0.08, 1.5, d),
            1 => expmap(0.05, 0.6, d),
            2 => expmap(0.05, 0.8, d),
            3 => expmap(0.01, 0.15, d),
            4 => expmap(0.1, 1.5, d),
            5 => expmap(0.08, 1.2, d),
            6 => expmap(0.005, 0.12, d),
            _ => expmap(0.05, 2.5, d),
        }
    }

    /// Long enough to be silent after.
    fn life(&self) -> f32 {
        self.decay() * 9.5 + if self.kind == 2 { expmap(0.05, 0.6, self.p[4]) * 6.0 } else { 0.0 } + 0.02
    }

    fn metal(&mut self, scale: f32, sr: f32) -> f32 {
        let mut s = 0.0;
        for k in 0..6 {
            s += if self.ph[k] < 0.5 { 1.0 } else { -1.0 };
            self.ph[k] = (self.ph[k] + METAL_HZ[k] * scale / sr).fract();
        }
        s / 6.0
    }

    #[inline]
    fn render(&mut self, sr: f32) -> f32 {
        let p = self.p;
        let t = self.t;
        let dec = self.decay();
        let e = (-t / dec).exp();
        let n = self.noise.next();
        let out = match self.kind {
            0 => {
                let base = expmap(35.0, 90.0, p[0]);
                let f = base * (1.0 + p[2] * 5.0 * (-t / expmap(0.005, 0.12, p[3])).exp());
                self.ph[0] = (self.ph[0] + f / sr).fract();
                let body = (self.ph[0] * TAU).sin() * e;
                let click = if t < 0.003 { self.f1.tick(n, 3000.0, 0.7, sr).hp * p[4] * 1.5 } else { 0.0 };
                body + click
            }
            1 => {
                let f = expmap(120.0, 350.0, p[0]);
                self.ph[0] = (self.ph[0] + f / sr).fract();
                self.ph[1] = (self.ph[1] + f * 1.47 / sr).fract();
                let body_e = (-t / expmap(0.02, 0.3, p[4])).exp();
                let body = ((self.ph[0] * TAU).sin() + 0.6 * (self.ph[1] * TAU).sin()) * 0.5 * body_e;
                let snap = self.f1.tick(n, expmap(800.0, 7000.0, p[3]), 0.7, sr).hp * e;
                body * (1.0 - p[2] * 0.7) + snap * p[2] * 1.3
            }
            2 => {
                let b = self.f1.tick(n, expmap(600.0, 3000.0, p[0]), 1.8, sr).bp;
                let gap = expmap(0.004, 0.025, p[2]);
                let bursts = 2 + (p[3] * 3.0).round() as i32;
                let mut env = 0.0;
                for k in 0..bursts {
                    let dt = t - k as f32 * gap;
                    if dt >= 0.0 {
                        env += (-dt / 0.006).exp();
                    }
                }
                let tail_start = bursts as f32 * gap;
                if t >= tail_start {
                    env += p[4] * 1.2 * (-(t - tail_start) / dec).exp();
                }
                b * env * 1.6
            }
            3 | 4 => {
                let scale = expmap(0.6, 1.8, p[0]);
                let m = self.metal(scale, sr);
                let src = m * (1.0 - p[4]) + n * p[4];
                // The state-variable band-pass peaks at Q; divide it back out
                // so Ring changes the sound, not the level.
                let q = 0.6 + p[3] * 3.0;
                let bp = self.f1.tick(src, expmap(5000.0, 12000.0, p[2]), q, sr).bp / q.sqrt();
                let hp = self.f2.tick(bp, expmap(4000.0, 9000.0, p[2]), 0.7, sr).hp;
                hp * 3.0 * e
            }
            5 => {
                let base = expmap(70.0, 300.0, p[0]);
                let f = base * (1.0 + p[2] * 1.5 * (-t / expmap(0.01, 0.2, p[3])).exp());
                self.ph[0] = (self.ph[0] + f / sr).fract();
                (self.ph[0] * TAU).sin() * e + self.f1.tick(n, base * 3.0, 1.0, sr).bp * p[4] * e * 0.5
            }
            6 => {
                let f = expmap(800.0, 2600.0, p[0]);
                self.ph[0] = (self.ph[0] + f / sr).fract();
                self.ph[1] = (self.ph[1] + f * lerp(1.2, 2.2, p[2]) / sr).fract();
                let click = if t < 0.0015 { n * p[3] } else { 0.0 };
                ((self.ph[0] * TAU).sin() + 0.5 * (self.ph[1] * TAU).sin()) * e * 0.7 + click + n * p[4] * e * 0.3
            }
            _ => {
                // Two of the square pairs detuned by Ratio: cowbell low, cymbal high.
                let f = expmap(300.0, 900.0, p[0]);
                let r = lerp(1.2, 2.6, p[2]);
                let mut sq = 0.0;
                for (k, ratio) in [1.0, r, r * 1.37, 2.13].iter().enumerate() {
                    sq += if self.ph[k] < 0.5 { 1.0 } else { -1.0 };
                    self.ph[k] = (self.ph[k] + f * ratio / sr).fract();
                }
                let src = sq * 0.25 * (1.0 - p[4]) + n * p[4];
                let q = 0.8 + p[3] * 6.0;
                self.f1.tick(src, f * 3.0, q, sr).bp / q.sqrt() * e * 2.0
            }
        };
        let drive = p[5];
        let y = if drive > 0.01 { (out * (1.0 + drive * 8.0)).tanh() / (1.0 + drive * 2.0).sqrt() } else { out };
        self.t += 1.0 / sr;
        let life = self.life();
        if self.t > life {
            self.active = false;
        }
        // The last 3 ms fade out, so a cut-off (or a retrigger) never clicks.
        y * self.vel * ((life - self.t) / 0.003).clamp(0.0, 1.0)
    }
}

pub struct Shared {
    pub kit: Mutex<Kit>,
    pub playing: AtomicBool,
    pub tempo: crate::util::AtomicF32,
    pub swing: crate::util::AtomicF32,
    /// Each lane's step last played.
    pub pos: [AtomicI32; VOICES],
    /// Hits per voice, for the screen and tests.
    pub hits: [AtomicU32; VOICES],
    /// Live hits from the pads: voice + 1, or 0.
    pub live: Mutex<Vec<(usize, f32)>>,
    /// Steps played since start.
    pub step: AtomicU32,
    pub step_fraction: crate::util::AtomicF32,
    /// The device transport, and whether the pattern follows it (on the
    /// device, by default: press play in Session and Skins plays along).
    pub clock: Arc<crate::clock::Clock>,
    pub follow: AtomicBool,
}

impl Shared {
    pub fn following(&self) -> bool {
        self.follow.load(Ordering::Relaxed)
    }
    pub fn is_playing(&self) -> bool {
        if self.following() {
            self.clock.running()
        } else {
            self.playing.load(Ordering::Relaxed)
        }
    }
    pub fn set_playing(&self, on: bool) {
        if self.following() {
            if on {
                // joining a transport that's already going doesn't restart it
                if !self.clock.running() {
                    self.clock.start();
                }
            } else {
                self.clock.stop();
            }
        } else {
            self.playing.store(on, Ordering::Relaxed);
        }
    }
    pub fn bpm(&self) -> f32 {
        if self.following() {
            self.clock.bpm()
        } else {
            self.tempo.get()
        }
    }
    pub fn set_bpm(&self, bpm: f32) {
        let bpm = bpm.clamp(40.0, 240.0);
        if self.following() {
            self.clock.set_bpm(bpm)
        } else {
            self.tempo.set(bpm)
        }
    }
}

/// The drum machine on the audio thread: sequencer, voices, note input.
struct Engine {
    s: Arc<Shared>,
    hits: Vec<Hit>,
    next_hit: usize,
    until: f64,
    step: u64,
    was_playing: bool,
    /// This block's transport snapshot, the frame within the block, and
    /// the step follower, when following the device clock.
    snap: crate::clock::Snap,
    fi: usize,
    follower: crate::clock::StepFollower,
    /// Ratchets still to come: (samples until, voice, velocity, params).
    pending: Vec<(u32, usize, f32, [f32; PARAMS])>,
    inbox: Option<Arc<NoteInboxRef>>,
    view: NoteView,
    strikes: [u8; 128],
    rng: kit::Rng,
    sr: f32,
}

impl Engine {
    fn trigger(&mut self, v: usize, vel: f32, p: [f32; PARAMS], pan: f32, level: f32) {
        // The open hat stops when the closed one plays, as on a real kit.
        if v == 3 {
            for h in self.hits.iter_mut().filter(|h| h.active && h.kind == 4) {
                h.active = false;
            }
        }
        // Retriggering a voice cuts its last hit short, after a 2 ms fade.
        for h in self.hits.iter_mut().filter(|h| h.active && h.kind == v) {
            h.t = h.t.max(h.life() - 0.002);
        }
        let i = self.next_hit;
        self.next_hit = (self.next_hit + 1) % self.hits.len();
        let h = &mut self.hits[i];
        *h = Hit { kind: v, p, vel: vel * level, pan, t: 0.0, ph: [0.0, 0.13, 0.37, 0.61, 0.79, 0.91], f1: Svf::default(), f2: Svf::default(), noise: h.noise, active: true };
        self.s.hits[v].fetch_add(1, Ordering::Relaxed);
    }

    fn step_samples(&self) -> f64 {
        let base = 60.0 / self.s.bpm().max(20.0) as f64 / 4.0 * self.sr as f64;
        let sw = self.s.swing.get().clamp(0.0, 0.4) as f64;
        if self.step % 2 == 0 {
            base * (1.0 + sw)
        } else {
            base * (1.0 - sw)
        }
    }

    fn do_step(&mut self) {
        let Ok(k) = self.s.kit.try_lock() else { return };
        let kit = k.clone();
        drop(k);
        let len = self.step_samples();
        for v in 0..VOICES {
            let lane = &kit.lanes[v];
            let at = (self.step % lane.length.clamp(1, STEPS) as u64) as usize;
            self.s.pos[v].store(at as i32, Ordering::Relaxed);
            let st = lane.steps[at];
            if !st.on || lane.mute {
                continue;
            }
            let chance = if st.chance == 0 { 100 } else { st.chance };
            if chance < 100 && self.rng.below(100) >= chance as u32 {
                continue;
            }
            let mut p = kit.voices[v].p;
            for (k, l) in st.locks.iter().enumerate() {
                if let Some(x) = l {
                    p[k] = *x;
                }
            }
            let vel = if st.accent { 1.0 } else { 0.72 };
            let r = st.ratchet.clamp(1, 4) as u32;
            self.trigger(v, vel, p, kit.voices[v].pan, kit.voices[v].level);
            for k in 1..r {
                self.pending.push(((len * k as f64 / r as f64) as u32, v, vel * 0.85, p));
            }
        }
        self.s.step.store(self.step as u32, Ordering::Relaxed);
    }
}

impl Extra for Engine {
    fn block(&mut self, frames: usize, sr: f32) {
        self.sr = sr;
        self.snap = self.s.clock.snap();
        self.fi = 0;
        // Live pads and notes from other apps.
        let live: Vec<(usize, f32)> = self.s.live.try_lock().map(|mut l| std::mem::take(&mut *l)).unwrap_or_default();
        let kit = self.s.kit.try_lock().ok().map(|k| k.voices);
        if let Some(voices) = kit {
            for (v, vel) in live {
                self.trigger(v, vel, voices[v].p, voices[v].pan, voices[v].level);
            }
            if let Some(inbox) = self.inbox.as_ref() {
                inbox.poll(&mut self.view);
                for n in 36..36 + VOICES {
                    if self.view.strikes[n] != self.strikes[n] {
                        self.strikes[n] = self.view.strikes[n];
                        let v = n - 36;
                        let vel = self.view.keys[n].max(1) as f32 / 127.0;
                        self.trigger(v, vel.max(0.3), voices[v].p, voices[v].pan, voices[v].level);
                    }
                }
            }
        }
        let _ = frames;
    }

    fn frame(&mut self, sr: f32) -> (f32, f32) {
        let follow = self.s.following();
        let playing = if follow { self.snap.running } else { self.s.playing.load(Ordering::Relaxed) };
        if follow {
            // Sample-locked to the device transport (see clock.rs).
            let beat = self.snap.beat_at(self.fi, sr);
            self.fi += 1;
            if let Some(step) = self.follower.poll(&self.snap, beat, 4, self.s.swing.get()) {
                self.step = step;
                self.do_step();
                self.step += 1;
            }
            if playing {
                self.s.step_fraction.set((beat * 4.0).fract() as f32);
            }
        } else if playing && !self.was_playing {
            self.step = 0;
            self.until = 0.0;
        }
        self.was_playing = playing;
        if playing && !follow {
            if self.until <= 0.0 {
                self.do_step();
                self.until += self.step_samples();
                self.step += 1;
            }
            self.until -= 1.0;
            self.s.step_fraction.set(1.0 - (self.until / self.step_samples()).clamp(0.0, 1.0) as f32);
        }
        let mut k = 0;
        while k < self.pending.len() {
            if self.pending[k].0 == 0 {
                let (_, v, vel, p) = self.pending.remove(k);
                let (pan, level) = self.s.kit.try_lock().map(|kt| (kt.voices[v].pan, kt.voices[v].level)).unwrap_or((0.0, 0.8));
                self.trigger(v, vel, p, pan, level);
            } else {
                self.pending[k].0 -= 1;
                k += 1;
            }
        }
        let (mut l, mut r) = (0.0, 0.0);
        for h in self.hits.iter_mut().filter(|h| h.active) {
            let y = h.render(sr) * 0.8;
            let (gl, gr) = kit::pan_gains(h.pan);
            l += y * gl;
            r += y * gr;
        }
        (l, r)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum View {
    Step,
    Play,
    Sound,
}
const VIEWS: [View; 3] = [View::Step, View::Play, View::Sound];

pub struct Skins {
    sound: Sound,
    pub s: Arc<Shared>,
    inbox: Option<Arc<NoteInboxRef>>,
    kits: Vec<Kit>,
    kit_index: usize,
    pub view: usize,
    pub voice: usize,
    field: usize,
    row: usize,
    held: Option<usize>,
    edited: bool,
    prev: [bool; 16],
    pub recording: bool,
    flash: [f32; VOICES],
    seen: [u32; VOICES],
}

/// Fields when holding a step: accent, ratchet, chance, then the six locks.
const STEP_FIELDS: usize = 3 + PARAMS;

impl Skins {
    pub fn new(sound: Sound, notes: Option<Arc<NoteBus>>) -> Skins {
        let kits = factory();
        let s = Arc::new(Shared {
            kit: Mutex::new(kits[0].clone()),
            playing: AtomicBool::new(false),
            clock: Arc::clone(sound.clock()),
            follow: AtomicBool::new(false),
            tempo: crate::util::AtomicF32::new(120.0),
            swing: crate::util::AtomicF32::new(0.0),
            pos: std::array::from_fn(|_| AtomicI32::new(-1)),
            hits: std::array::from_fn(|_| AtomicU32::new(0)),
            live: Mutex::new(Vec::new()),
            step: AtomicU32::new(0),
            step_fraction: crate::util::AtomicF32::new(0.0),
        });
        sound.set_reverb(0.08);
        let inbox = notes.and_then(|b| b.register_instrument("skins", NAME));
        Skins { sound, s, inbox, kits, kit_index: 0, view: 0, voice: 0, field: 0, row: 0, held: None, edited: false, prev: [false; 16], recording: false, flash: [0.0; VOICES], seen: [0; VOICES] }
    }

    fn play_voice(&mut self, v: usize, vel: f32) {
        self.s.live.lock().unwrap().push((v, vel));
        if self.recording && self.s.is_playing() {
            let step = self.s.step.load(Ordering::Relaxed) as usize;
            let frac = self.s.step_fraction.get();
            let mut k = self.s.kit.lock().unwrap();
            let len = k.lanes[v].length.clamp(1, STEPS);
            let at = (step + if frac > 0.5 { 1 } else { 0 }) % len;
            k.lanes[v].steps[at].on = true;
            if k.lanes[v].steps[at].ratchet == 0 {
                k.lanes[v].steps[at].ratchet = 1;
            }
        }
    }

    fn field_name(&self, f: usize) -> String {
        match f {
            0 => "Accent".into(),
            1 => "Ratchet".into(),
            2 => "Chance".into(),
            k => format!("Lock {}", PARAM_NAMES[self.voice][k - 3]),
        }
    }

    fn field_value(&self, st: &StepData, f: usize) -> String {
        match f {
            0 => if st.accent { "on".into() } else { "off".into() },
            1 => format!("x{}", st.ratchet.max(1)),
            2 => format!("{}%", if st.chance == 0 { 100 } else { st.chance }),
            k => st.locks[k - 3].map_or("-".into(), |v| format!("{:.0}", v * 100.0)),
        }
    }

    fn edit_field(&mut self, step: usize, d: i32) {
        let f = self.field;
        let v = self.voice;
        let mut k = self.s.kit.lock().unwrap();
        let base = k.voices[v].p;
        let st = &mut k.lanes[v].steps[step];
        st.on = true;
        st.ratchet = st.ratchet.max(1);
        match f {
            0 => st.accent = !st.accent,
            1 => st.ratchet = (st.ratchet as i32 + d).clamp(1, 4) as u8,
            2 => {
                let c = if st.chance == 0 { 100 } else { st.chance as i32 };
                st.chance = (c + d * 10).clamp(0, 100).max(5) as u8;
            }
            p => {
                let cur = st.locks[p - 3].unwrap_or(base[p - 3]);
                st.locks[p - 3] = Some((cur + d as f32 * 0.04).clamp(0.0, 1.0));
            }
        }
    }

    /// The SOUND rows: six controls, level, pan, then tempo, swing, kit.
    fn sound_rows(&self) -> usize {
        PARAMS + 6
    }

    fn sound_row(&self, i: usize) -> (String, String) {
        let k = self.s.kit.lock().unwrap();
        let vs = &k.voices[self.voice];
        match i {
            0..=5 => (PARAM_NAMES[self.voice][i].into(), format!("{:.0}", vs.p[i] * 100.0)),
            6 => ("Level".into(), format!("{:.0}", vs.level * 100.0)),
            7 => ("Pan".into(), format!("{:+.0}", vs.pan * 100.0)),
            8 => ("Tempo".into(), format!("{:.0} bpm", self.s.bpm())),
            9 => ("Swing".into(), format!("{:.0}%", self.s.swing.get() * 100.0)),
            10 => ("Clock".into(), if self.s.following() { "device".into() } else { "own".into() }),
            _ => ("Kit".into(), format!("{} (< > loads)", self.kits[self.kit_index].name)),
        }
    }

    fn sound_edit(&mut self, d: i32) {
        let i = self.row;
        if i == 10 {
            self.s.set_playing(false);
            self.s.follow.store(!self.s.following(), Ordering::Relaxed);
            return;
        }
        if i == 11 {
            self.kit_index = (self.kit_index as i32 + d).rem_euclid(self.kits.len() as i32) as usize;
            *self.s.kit.lock().unwrap() = self.kits[self.kit_index].clone();
            return;
        }
        if i == 8 {
            self.s.set_bpm(self.s.bpm().round() + d as f32);
            return;
        }
        if i == 9 {
            self.s.swing.set(((self.s.swing.get() * 50.0).round() + d as f32) / 50.0);
            self.s.swing.set(self.s.swing.get().clamp(0.0, 0.4));
            return;
        }
        let mut k = self.s.kit.lock().unwrap();
        let vs = &mut k.voices[self.voice];
        match i {
            0..=5 => vs.p[i] = (vs.p[i] + d as f32 * 0.02).clamp(0.0, 1.0),
            6 => vs.level = (vs.level + d as f32 * 0.02).clamp(0.0, 1.0),
            _ => vs.pan = (vs.pan + d as f32 * 0.05).clamp(-1.0, 1.0),
        }
        drop(k);
        self.s.live.lock().unwrap().push((self.voice, 0.8));
    }
}

impl App for Skins {
    fn instrument_settings(&self) -> Vec<crate::app::Setting> {
        (0..self.sound_rows()).map(|i| self.sound_row(i)).map(|(label, value)| crate::app::Setting { label, value }).collect()
    }
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        if index < self.sound_rows() {
            let keep = std::mem::replace(&mut self.row, index);
            self.sound_edit(delta);
            self.row = keep;
        }
    }
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn needs_background_audio(&self) -> bool {
        self.s.is_playing()
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![("View".into(), format!("{:?}", VIEWS[self.view]), false), ("Voice".into(), VOICE_NAMES[self.voice].into(), false), ("Kit".into(), self.kits[self.kit_index].name.clone(), false)]
    }
    fn running(&self) -> Option<bool> {
        Some(self.s.is_playing())
    }
    fn toggle_running(&mut self) {
        let p = !self.s.is_playing();
        self.s.set_playing(p);
        if !p {
            self.recording = false;
        }
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(match VIEWS[self.view] {
            View::Step => "STEP",
            View::Play => "PLAY",
            View::Sound => "SOUND",
        })
    }
    fn toggle_grid_mode(&mut self) {
        self.view = (self.view + 1) % VIEWS.len();
        self.held = None;
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let k = self.s.kit.lock().unwrap();
        match VIEWS[self.view] {
            View::Step => {
                let lane = &k.lanes[self.voice];
                let pos = self.s.pos[self.voice].load(Ordering::Relaxed);
                std::array::from_fn(|i| {
                    if pos == i as i32 && self.s.is_playing() {
                        PadColor::Red
                    } else if i >= lane.length {
                        PadColor::Off
                    } else if lane.steps[i].on {
                        if lane.steps[i].accent { PadColor::Yellow } else { PadColor::Green }
                    } else {
                        PadColor::Blue
                    }
                })
            }
            View::Play => std::array::from_fn(|i| if i < VOICES { PadColor::Green } else if k.lanes[i - VOICES].mute { PadColor::Red } else { PadColor::Blue }),
            View::Sound => std::array::from_fn(|i| if i == self.voice { PadColor::Red } else if i < VOICES { PadColor::Blue } else { PadColor::Off }),
        }
    }

    fn tick(&mut self, input: &Input) {
        let pressed: Vec<usize> = (0..16).filter(|&p| input.grid[p] && !self.prev[p]).collect();
        let released: Vec<usize> = (0..16).filter(|&p| !input.grid[p] && self.prev[p]).collect();
        match VIEWS[self.view] {
            View::Step => {
                for p in pressed {
                    if self.held.is_none() {
                        self.held = Some(p);
                        self.edited = false;
                    }
                }
                if let Some(h) = self.held {
                    if input.navigation_steps != 0 {
                        self.field = (self.field as i32 + input.navigation_steps).clamp(0, STEP_FIELDS as i32 - 1) as usize;
                        self.edited = true;
                    }
                    if input.knob2 != 0 {
                        self.edit_field(h, input.knob2.signum());
                        self.edited = true;
                    }
                    if released.contains(&h) {
                        if !self.edited {
                            let mut k = self.s.kit.lock().unwrap();
                            let len = k.lanes[self.voice].length;
                            if h < len {
                                let st = &mut k.lanes[self.voice].steps[h];
                                if st.on {
                                    *st = StepData::default();
                                } else {
                                    st.on = true;
                                    st.ratchet = 1;
                                }
                            }
                        }
                        self.held = None;
                    }
                } else {
                    if input.navigation_steps != 0 {
                        self.voice = (self.voice as i32 + input.navigation_steps).rem_euclid(VOICES as i32) as usize;
                    }
                    if input.knob2 != 0 {
                        let mut k = self.s.kit.lock().unwrap();
                        let l = &mut k.lanes[self.voice].length;
                        *l = (*l as i32 + input.knob2.signum()).clamp(1, STEPS as i32) as usize;
                    }
                    if input.knob1_press {
                        self.toggle_running();
                    }
                }
            }
            View::Play => {
                for p in pressed {
                    if p < VOICES {
                        self.voice = p;
                        self.play_voice(p, 0.9);
                    } else {
                        let mut k = self.s.kit.lock().unwrap();
                        let m = &mut k.lanes[p - VOICES].mute;
                        *m = !*m;
                    }
                }
                if input.knob1_press {
                    self.recording = !self.recording;
                    if self.recording && !self.s.is_playing() {
                        self.toggle_running();
                    }
                }
                if input.knob2_press {
                    self.s.kit.lock().unwrap().lanes[self.voice] = Lane::default();
                }
            }
            View::Sound => {
                for p in pressed {
                    if p < VOICES {
                        self.voice = p;
                        self.s.live.lock().unwrap().push((p, 0.85));
                    }
                }
                if input.navigation_steps != 0 {
                    self.row = (self.row as i32 + input.navigation_steps).clamp(0, self.sound_rows() as i32 - 1) as usize;
                }
                if input.knob2 != 0 {
                    self.sound_edit(input.knob2.signum());
                }
                if input.knob1_press {
                    self.toggle_running();
                }
            }
        }
        self.prev = input.grid;
        for v in 0..VOICES {
            let h = self.s.hits[v].load(Ordering::Relaxed);
            if h != self.seen[v] {
                self.seen[v] = h;
                self.flash[v] = 1.0;
            }
            self.flash[v] = (self.flash[v] - 0.08).max(0.0);
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = Rgb565::new(4, 3, 5);
        let panel = Rgb565::new(9, 7, 11);
        let dim = Rgb565::new(18, 26, 24);
        let ink = kit::rgb(250, 160, 120);
        kit::clear(fb, bg);
        kit::rect(fb, 0, 0, 640, 30, panel);
        kit::text(fb, NAME, 12, 7, Size2::Medium, kit::WHITE, -1);
        for (i, v) in ["STEP", "PLAY", "SOUND"].iter().enumerate() {
            let x = 80 + i as i32 * 64;
            let sel = i == self.view;
            kit::round_rect(fb, x, 6, 60, 18, 6, if sel { ink } else { Rgb565::new(12, 10, 14) });
            kit::text(fb, v, x + 30, 9, Size2::Small, if sel { kit::BLACK } else { dim }, 0);
        }
        let k = self.s.kit.lock().unwrap().clone();
        let playing = self.s.is_playing();
        kit::text(fb, &format!("{}  {:.0} bpm{}{}", k.name, self.s.bpm(), if playing { "  playing" } else { "" }, if self.recording { "  REC" } else { "" }), 630, 9, Size2::Small, if self.recording { kit::rgb(250, 90, 90) } else { dim }, 1);
        // All eight lanes.
        let (gx, gy, cw, ch) = (96, 38, 21, 20);
        for v in 0..VOICES {
            let y = gy + v as i32 * (ch + 4);
            let (r, g, b) = COLORS[v];
            let col = kit::rgb(r, g, b);
            let sel = v == self.voice;
            kit::round_rect(fb, 8, y, 84, ch, 5, if sel { kit::blend(col, bg, 0.3) } else { kit::blend(panel, col, self.flash[v] * 0.6) });
            kit::text(fb, VOICE_NAMES[v], 14, y + 4, Size2::Small, if sel { kit::BLACK } else if k.lanes[v].mute { dim } else { kit::WHITE }, -1);
            let pos = self.s.pos[v].load(Ordering::Relaxed);
            for st in 0..STEPS {
                let x = gx + st as i32 * cw + (st as i32 / 4) * 3;
                let d = &k.lanes[v].steps[st];
                let inside = st < k.lanes[v].length;
                let mut c = if !inside { bg } else if d.on { if d.accent { kit::blend(col, kit::WHITE, 0.3) } else { col } } else { panel };
                if pos == st as i32 && playing {
                    c = kit::blend(c, kit::WHITE, 0.5);
                }
                kit::round_rect(fb, x, y, cw - 3, ch, 3, c);
                if d.on && d.ratchet > 1 {
                    kit::text(fb, &d.ratchet.to_string(), x + 9, y + 4, Size2::Small, kit::BLACK, 0);
                } else if d.on && d.locks.iter().any(|l| l.is_some()) {
                    kit::circle(fb, x + 9, y + 10, 3, kit::BLACK);
                }
            }
        }
        // The side panel: the held step's fields, or the voice's sound.
        let py = 236;
        kit::round_rect(fb, 8, py, 624, 98, 8, panel);
        match VIEWS[self.view] {
            View::Step => {
                if let Some(h) = self.held {
                    let st = k.lanes[self.voice].steps[h];
                    kit::text(fb, &format!("{} step {}", VOICE_NAMES[self.voice], h + 1), 16, py + 6, Size2::Small, kit::WHITE, -1);
                    for f in 0..STEP_FIELDS {
                        let x = 16 + (f as i32 % 5) * 122;
                        let y = py + 24 + (f as i32 / 5) * 34;
                        let sel = f == self.field;
                        kit::round_rect(fb, x, y, 116, 30, 6, if sel { kit::blend(panel, kit::WHITE, 0.18) } else { Rgb565::new(12, 10, 15) });
                        kit::text(fb, &self.field_name(f), x + 6, y + 3, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
                        kit::text(fb, &self.field_value(&st, f), x + 6, y + 16, Size2::Small, ink, -1);
                    }
                } else {
                    kit::paragraph(fb, &format!("{}: tap pads to set steps; hold a step and use the D-pad to accent, ratchet, set its chance or lock a control. Up/down: voice. Left/right: this lane's length ({} steps). SELECT: play/stop.", VOICE_NAMES[self.voice], k.lanes[self.voice].length), 16, py + 8, 600, Size2::Small, dim);
                }
            }
            View::Play => {
                kit::paragraph(fb, "Pads 1-8 play the voices; pads 9-16 mute or unmute them. SELECT records what you play (to the nearest step); hold SELECT clears the selected voice's lane.", 16, py + 8, 600, Size2::Small, dim);
            }
            View::Sound => {
                for i in 0..self.sound_rows() {
                    let x = 16 + (i as i32 % 6) * 101;
                    let y = py + 8 + (i as i32 / 6) * 44;
                    let (l, v) = self.sound_row(i);
                    let sel = i == self.row;
                    kit::round_rect(fb, x, y, 96, 40, 6, if sel { kit::blend(panel, kit::WHITE, 0.18) } else { Rgb565::new(12, 10, 15) });
                    kit::text(fb, &l, x + 6, y + 4, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
                    let v: String = v.chars().take(15).collect();
                    kit::text(fb, &v, x + 6, y + 22, Size2::Small, ink, -1);
                }
            }
        }
        kit::footer(fb, "F2: view   pads 1-8 on SOUND pick a voice   F3/SELECT: play", panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn crate::audio::AudioProcessor>> {
        let engine = Engine { s: Arc::clone(&self.s), hits: (0..24).map(|i| Hit::idle(0x1234 + i * 7919)).collect(), next_hit: 0, until: 0.0, step: 0, was_playing: false, snap: self.s.clock.snap(), fi: 0, follower: Default::default(), pending: Vec::with_capacity(32), inbox: self.inbox.take(), view: NoteView::default(), strikes: [0; 128], rng: kit::Rng::seeded_from_time(), sr: 48_000.0 };
        Some(self.sound.processor(None, Some(Box::new(engine))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    let skins = Skins::new(Sound::new(NAME, &modbus, &mixer, &bus), ctx.try_get());
    skins.s.follow.store(true, Ordering::Relaxed);
    Box::new(skins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render};

    /// A hit rendered until it says it's over, plus 0.1 s.
    fn hit(kind: usize, p: [f32; PARAMS]) -> Vec<f32> {
        let mut h = Hit::idle(5);
        h.kind = kind;
        h.p = p;
        h.vel = 1.0;
        h.active = true;
        let n = ((h.life() + 0.1) * 48_000.0) as usize;
        (0..n).map(|_| if h.active { h.render(48_000.0) } else { 0.0 }).collect()
    }

    #[test]
    fn every_voice_sounds_and_ends() {
        for kit in factory() {
            for v in 0..VOICES {
                let x = hit(v, kit.voices[v].p);
                let e = energy(&x[..4800]);
                assert!(e > 1e-5, "{} {}: {e}", kit.name, VOICE_NAMES[v]);
                let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                assert!(peak.is_finite() && peak < 3.0, "{} {}: peak {peak}", kit.name, VOICE_NAMES[v]);
                let tail = &x[x.len() - 2400..];
                assert!(tail.iter().all(|s| *s == 0.0), "{} {} still going", kit.name, VOICE_NAMES[v]);
                // The 20 ms before it stops (it may run a little past life + 0.1 s).
                let before = &x[x.len() - 6000..x.len() - 5040];
                assert!(before.iter().all(|s| s.abs() < 2e-3), "{} {} stopped while audible", kit.name, VOICE_NAMES[v]);
            }
        }
    }

    #[test]
    fn the_kicks_tune_sets_its_pitch() {
        // Count zero crossings late in the hit, once the sweep has gone.
        let pitch = |tune: f32| {
            let x = hit(0, [tune, 0.9, 0.0, 0.2, 0.0, 0.0]);
            let w = &x[9_600..57_600];
            w.windows(2).filter(|p| p[0] < 0.0 && p[1] >= 0.0).count() as f32
        };
        assert!((pitch(0.0) - 35.0).abs() < 4.0, "{}", pitch(0.0));
        assert!((pitch(1.0) - 90.0).abs() < 6.0, "{}", pitch(1.0));
    }

    #[test]
    fn locks_ratchets_and_lengths_play_as_set() {
        let mut s = Skins::new(Sound::detached(), None);
        {
            let mut k = s.s.kit.lock().unwrap();
            k.lanes = (0..VOICES).map(|_| Lane::default()).collect();
            k.lanes[0].steps[0] = StepData { on: true, ratchet: 3, ..Default::default() };
            k.lanes[6].length = 3;
            k.lanes[6].steps[0] = StepData { on: true, ratchet: 1, ..Default::default() };
        }
        s.s.tempo.set(120.0);
        let mut p = s.audio_processor().unwrap();
        s.toggle_running();
        // Just under one bar at 120 bpm (2 s = 187.5 blocks).
        render(&mut p, 187);
        assert_eq!(s.s.hits[0].load(Ordering::Relaxed), 3, "a 3-ratchet on step 1, once a bar");
        // The rim's 3-step lane plays on steps 1, 4, 7, 10, 13, 16: six times.
        assert_eq!(s.s.hits[6].load(Ordering::Relaxed), 6);
    }

    #[test]
    fn a_lock_changes_only_its_step() {
        let mut s = Skins::new(Sound::detached(), None);
        s.view = 0;
        s.voice = 5;
        let pad = |i: usize| Input { grid: std::array::from_fn(|g| g == i), ..Default::default() };
        // Hold step 3, go to the Tune lock, raise it.
        s.tick(&pad(2));
        s.tick(&Input { navigation_steps: 3, ..pad(2) });
        for _ in 0..5 {
            s.tick(&Input { knob2: 1, ..pad(2) });
        }
        s.tick(&Input::default());
        let k = s.s.kit.lock().unwrap();
        let base = k.voices[5].p[0];
        assert!((k.lanes[5].steps[2].locks[0].unwrap() - (base + 0.2)).abs() < 1e-5);
        assert!(k.lanes[5].steps[2].on);
        assert_eq!(k.lanes[5].steps[3].locks[0], None);
    }

    #[test]
    fn notes_from_other_apps_play_the_voices() {
        let bus = Arc::new(NoteBus::new());
        let mut s = Skins::new(Sound::detached(), Some(Arc::clone(&bus)));
        let mut p = s.audio_processor().unwrap();
        let idx = bus.instrument_index(NAME).unwrap();
        let inbox = bus.inbox_ref(idx);
        inbox.note_on(37, 100); // snare
        render(&mut p, 2);
        inbox.note_off(37);
        inbox.note_on(37, 100); // struck again before anyone looked
        render(&mut p, 2);
        assert_eq!(s.s.hits[1].load(Ordering::Relaxed), 2, "both strikes heard");
        assert!(energy(&render(&mut p, 5)) > 1e-6);
        let mut fb = FrameBuffer::new();
        for v in 0..3 {
            s.view = v;
            s.draw(&mut fb);
        }
    }

    #[test]
    fn the_closed_hat_chokes_the_open_one() {
        let mut s = Skins::new(Sound::detached(), None);
        s.sound.set_reverb(0.0);
        let mut p = s.audio_processor().unwrap();
        s.s.live.lock().unwrap().push((4, 1.0));
        render(&mut p, 5);
        s.s.live.lock().unwrap().push((3, 1.0));
        render(&mut p, 30);
        let tail = energy(&render(&mut p, 10));
        assert!(tail < 1e-7, "the open hat was cut: {tail}");
    }

    #[test]
    fn following_the_device_clock_it_plays_when_anyone_presses_play() {
        let clock = Arc::new(crate::clock::Clock::new());
        let mut s = Skins::new(Sound::detached_with_clock(Arc::clone(&clock)), None);
        s.s.follow.store(true, Ordering::Relaxed);
        let mut p = s.audio_processor().unwrap();
        clock.set_bpm(120.0);
        clock.start(); // someone else's play button
        assert_eq!(s.running(), Some(true));
        let mut buf = vec![0.0f32; 480 * 2];
        // two bars at 120 bpm = 4 s = 400 blocks of 480
        for _ in 0..400 {
            p.process(&mut buf, 2, 48_000.0);
            clock.end_block(480, 48_000.0);
        }
        let kick_steps = s.s.kit.lock().unwrap().lanes[0].steps.iter().take(16).filter(|st| st.on).count() as u32;
        let kicks = s.s.hits[0].load(Ordering::Relaxed);
        assert!(kicks >= kick_steps * 2 && kicks <= kick_steps * 2 + 1, "{kicks} kicks for {kick_steps} a bar");
        // its own play button stops the device
        s.toggle_running();
        p.process(&mut buf, 2, 48_000.0);
        clock.end_block(480, 48_000.0);
        assert!(!clock.snap().running);
    }
}
