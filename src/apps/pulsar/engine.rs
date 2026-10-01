//! Pulsar's audio engine: 8 drum lanes (synthesized models or samples),
//! a swing/micro-timing/ratchet step sequencer with pattern slots, chain
//! and auto-fill, and a master chain (fatten, glue, DJ filter, room).
//!
//! The same `Engine` runs live on the audio thread and offline for WAV
//! export -- it only reads a plain `Settings` copy, a pattern bank and the
//! loaded samples, so both paths sound identical. `process` never
//! allocates, locks or panics.

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

pub const LANES: usize = 8;
pub const MAX_STEPS: usize = 64;
pub const SLOTS: usize = 8;
/// Bank index of the generated fill bar (16 steps) used by auto-fill.
pub const FILL: usize = SLOTS;
pub const BANK: usize = SLOTS + 1;
const MAX_PENDING: usize = 96;

pub const LANE_NAMES: [&str; LANES] = ["Kick", "Snare", "Clap", "Closed Hat", "Open Hat", "Perc 1", "Perc 2", "Crash"];
pub const LANE_SHORT: [&str; LANES] = ["KK", "SN", "CP", "CH", "OH", "P1", "P2", "CR"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Step {
    /// 0 = no hit, else MIDI-style velocity 1..127.
    pub vel: u8,
    /// Chance to play, 0..100.
    pub prob: u8,
    /// Hits per step, 1..4 (rolls).
    pub ratchet: u8,
    /// Timing offset, -50..50 % of half a step.
    pub micro: i8,
}

impl Step {
    pub const OFF: Step = Step { vel: 0, prob: 100, ratchet: 1, micro: 0 };
    pub fn hit(vel: u8) -> Step {
        Step { vel: vel.max(1), prob: 100, ratchet: 1, micro: 0 }
    }
    pub fn on(&self) -> bool {
        self.vel > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pattern {
    pub steps: [[Step; MAX_STEPS]; LANES],
    pub length: usize,
}

impl Default for Pattern {
    fn default() -> Self {
        Pattern { steps: [[Step::OFF; MAX_STEPS]; LANES], length: 16 }
    }
}

impl Pattern {
    pub fn is_empty(&self) -> bool {
        self.steps.iter().all(|l| l.iter().take(self.length).all(|s| !s.on()))
    }
    pub fn hits(&self, lane: usize) -> usize {
        self.steps[lane].iter().take(self.length).filter(|s| s.on()).count()
    }
}

// ------------------------------------------------------------ models

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Kick808,
    Kick909,
    KickHard,
    Snare,
    Snare808,
    Clap,
    Snap,
    HatClosed,
    HatOpen,
    Ride,
    Crash,
    Tom,
    Conga,
    Rim,
    Cowbell,
    Shaker,
    Clave,
    Zap,
}

pub const MODELS: [Model; 18] = [
    Model::Kick808,
    Model::Kick909,
    Model::KickHard,
    Model::Snare,
    Model::Snare808,
    Model::Clap,
    Model::Snap,
    Model::HatClosed,
    Model::HatOpen,
    Model::Ride,
    Model::Crash,
    Model::Tom,
    Model::Conga,
    Model::Rim,
    Model::Cowbell,
    Model::Shaker,
    Model::Clave,
    Model::Zap,
];

impl Model {
    pub fn name(self) -> &'static str {
        match self {
            Model::Kick808 => "808 Kick",
            Model::Kick909 => "909 Kick",
            Model::KickHard => "Hard Kick",
            Model::Snare => "Snare",
            Model::Snare808 => "808 Snare",
            Model::Clap => "Clap",
            Model::Snap => "Snap",
            Model::HatClosed => "Closed Hat",
            Model::HatOpen => "Open Hat",
            Model::Ride => "Ride",
            Model::Crash => "Crash",
            Model::Tom => "Tom",
            Model::Conga => "Conga",
            Model::Rim => "Rim",
            Model::Cowbell => "Cowbell",
            Model::Shaker => "Shaker",
            Model::Clave => "Clave",
            Model::Zap => "Zap",
        }
    }
    pub fn index(self) -> usize {
        MODELS.iter().position(|m| *m == self).unwrap_or(0)
    }
    pub fn from_index(i: usize) -> Model {
        MODELS[i % MODELS.len()]
    }
    /// General MIDI drum note, for MIDI export.
    pub fn gm_note(self) -> u8 {
        match self {
            Model::Kick808 | Model::Kick909 | Model::KickHard => 36,
            Model::Snare | Model::Snare808 => 38,
            Model::Clap | Model::Snap => 39,
            Model::HatClosed => 42,
            Model::HatOpen => 46,
            Model::Ride => 51,
            Model::Crash => 49,
            Model::Tom => 45,
            Model::Conga => 63,
            Model::Rim => 37,
            Model::Cowbell => 56,
            Model::Shaker => 70,
            Model::Clave => 75,
            Model::Zap => 81,
        }
    }
}

/// A decoded sample, mono, at its own rate.
pub struct SampleBuf {
    pub name: String,
    pub data: Vec<f32>,
    pub rate: f32,
}

// ---------------------------------------------------------- settings

#[derive(Clone, Copy, Debug)]
pub struct LaneSettings {
    pub model: Model,
    pub use_sample: bool,
    pub level: f32,
    pub pan: f32,
    /// semitones
    pub tune: f32,
    pub decay: f32,
    pub tone: f32,
    pub punch: f32,
    pub send: f32,
    pub mute: bool,
    pub solo: bool,
    /// Lane-wide probability scale 0..1.
    pub chance: f32,
}

impl Default for LaneSettings {
    fn default() -> Self {
        Self {
            model: Model::Kick909,
            use_sample: false,
            level: 0.8,
            pan: 0.0,
            tune: 0.0,
            decay: 0.5,
            tone: 0.5,
            punch: 0.5,
            send: 0.0,
            mute: false,
            solo: false,
            chance: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub lanes: [LaneSettings; LANES],
    pub bpm: f32,
    /// 0..1 = 50%..75% swing
    pub swing: f32,
    pub swing8: bool,
    /// 0..1
    pub human_time: f32,
    pub human_vel: f32,
    pub fatten: f32,
    /// semitones, applied to every lane
    pub tone_shift: f32,
    pub glue: f32,
    /// -1 lowpass .. 0 off .. +1 highpass
    pub filter: f32,
    pub room: f32,
    pub level: f32,
    pub playing: bool,
    pub slot: usize,
    pub chain: bool,
    /// 0 = off, else every N bars the last bar plays the fill
    pub fill_every: u32,
    /// one-shot pad hits this block (velocity, 0 = none)
    pub audition: [u8; LANES],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lanes: [LaneSettings::default(); LANES],
            bpm: 120.0,
            swing: 0.0,
            swing8: false,
            human_time: 0.0,
            human_vel: 0.0,
            fatten: 0.0,
            tone_shift: 0.0,
            glue: 0.0,
            filter: 0.0,
            room: 0.0,
            level: 0.8,
            playing: true,
            slot: 0,
            chain: false,
            fill_every: 0,
            audition: [0; LANES],
        }
    }
}

// ------------------------------------------------------------ helpers

#[inline]
fn white(rng: &mut u32) -> f32 {
    let mut x = *rng;
    if x == 0 {
        x = 0x6C07_8965;
    }
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *rng = x;
    (x as f32 / u32::MAX as f32) * 2.0 - 1.0
}

#[inline]
fn uni(rng: &mut u32) -> f32 {
    white(rng) * 0.5 + 0.5
}

#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// Returns (low, band, high).
    #[inline]
    fn run(&mut self, x: f32, fc: f32, q: f32, sr: f32) -> (f32, f32, f32) {
        let g = (PI * fc.clamp(20.0, sr * 0.45) / sr).tan();
        let k = 1.0 / q.max(0.3);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, x - k * v1 - v2)
    }
}

// --------------------------------------------------------------- voice

#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    t: f32,
    vel: f32,
    phase: [f32; 6],
    f1: Svf,
    f2: Svf,
    choke: f32,
    pos: f64,
    model: Model,
    rng: u32,
    dc: (f32, f32),
}

impl Default for Voice {
    fn default() -> Self {
        Voice {
            active: false,
            t: 0.0,
            vel: 0.0,
            phase: [0.0; 6],
            f1: Svf::default(),
            f2: Svf::default(),
            choke: 1.0,
            pos: 0.0,
            model: Model::Kick909,
            rng: 0x1234_5679,
            dc: (0.0, 0.0),
        }
    }
}

const METAL: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

impl Voice {
    fn trigger(&mut self, model: Model, vel: f32) {
        self.active = true;
        self.t = 0.0;
        self.vel = vel;
        self.choke = 1.0;
        self.pos = 0.0;
        self.model = model;
        self.phase = [0.0, 0.13, 0.37, 0.61, 0.29, 0.83];
    }

    #[inline]
    fn render(&mut self, ls: &LaneSettings, shift: f32, sample: Option<&SampleBuf>, sr: f32) -> f32 {
        if !self.active {
            return 0.0;
        }
        let dt = 1.0 / sr;
        let t = self.t;
        self.t += dt;
        let ratio = 2f32.powf((ls.tune + shift) / 12.0);
        let (decay, tone, punch) = (ls.decay.clamp(0.0, 1.0), ls.tone.clamp(0.0, 1.0), ls.punch.clamp(0.0, 1.0));
        let env = |tau: f32| (-t / tau.max(1e-4)).exp();
        let done_after: f32;

        let out = if let (true, Some(s)) = (ls.use_sample, sample) {
            let len = s.data.len();
            let i = self.pos as usize;
            if i + 1 >= len {
                self.active = false;
                return 0.0;
            }
            let f = (self.pos - i as f64) as f32;
            let x = s.data[i] * (1.0 - f) + s.data[i + 1] * f;
            self.pos += (s.rate / sr * ratio) as f64;
            let amp = if decay >= 0.99 { 1.0 } else { env(0.03 + decay * decay * 2.5) };
            let x = x * (1.0 + punch * 2.0);
            let x = if punch > 0.05 { x.tanh() } else { x };
            let (lp, _, _) = self.f1.run(x, 200.0 * 100f32.powf(tone), 0.7, sr);
            done_after = 30.0;
            if decay < 0.99 && amp < 1e-4 {
                self.active = false;
            }
            lp * amp
        } else {
            match self.model {
                Model::Kick808 | Model::Kick909 | Model::KickHard => {
                    let (base, tau, ptau, pamt, drive) = match self.model {
                        Model::Kick808 => (48.0, 0.15 + decay * 1.4, 0.02, 2.5, 1.0),
                        Model::Kick909 => (55.0, 0.08 + decay * 0.6, 0.012, 3.5, 1.3),
                        _ => (58.0, 0.08 + decay * 0.7, 0.008, 5.0, 4.0),
                    };
                    let f = base * ratio * (1.0 + pamt * punch * (-t / ptau).exp());
                    self.phase[0] = (self.phase[0] + f * dt).fract();
                    let body = (self.phase[0] * TAU).sin() * env(tau);
                    let click = white(&mut self.rng) * env(0.0015) * tone * 0.6;
                    done_after = tau * 7.0;
                    ((body + click) * drive).tanh() / drive.tanh()
                }
                Model::Snare | Model::Snare808 => {
                    let (f1, f2, ttau, ntau) = if self.model == Model::Snare {
                        (185.0, 330.0, 0.05 + decay * 0.12, 0.08 + decay * 0.3)
                    } else {
                        (238.0, 476.0, 0.08 + decay * 0.2, 0.05 + decay * 0.2)
                    };
                    self.phase[0] = (self.phase[0] + f1 * ratio * (1.0 + 0.5 * (-t / 0.01).exp()) * dt).fract();
                    self.phase[1] = (self.phase[1] + f2 * ratio * dt).fract();
                    let tonal = ((self.phase[0] * TAU).sin() + 0.5 * (self.phase[1] * TAU).sin()) * env(ttau);
                    let n = white(&mut self.rng);
                    let (_, _, hp) = self.f1.run(n, 1200.0 + tone * 5000.0, 0.7, sr);
                    let noise = hp * env(ntau);
                    done_after = ntau.max(ttau) * 8.0;
                    tonal * (1.0 - punch * 0.6) * 0.8 + noise * (0.3 + punch * 0.9)
                }
                Model::Clap | Model::Snap => {
                    let n = white(&mut self.rng);
                    if self.model == Model::Snap {
                        let (_, bp, _) = self.f1.run(n, 2400.0 * ratio, 2.5, sr);
                        done_after = 0.3;
                        bp * (env(0.004) * 2.5 + env(0.03 + decay * 0.08) * 0.6)
                    } else {
                        let spread = 0.006 + punch * 0.012;
                        let mut e = 0.0;
                        for k in 0..3 {
                            let tk = t - k as f32 * spread;
                            if tk >= 0.0 {
                                e += (-tk / 0.006).exp();
                            }
                        }
                        let tail = if t > 2.0 * spread { (-(t - 2.0 * spread) / (0.08 + decay * 0.5)).exp() * 0.5 } else { 0.0 };
                        let (_, bp, _) = self.f1.run(n, (900.0 + tone * 1200.0) * ratio, 1.8, sr);
                        done_after = 0.1 + decay * 4.0;
                        bp * (e + tail) * 1.6
                    }
                }
                Model::HatClosed | Model::HatOpen | Model::Ride | Model::Crash => {
                    let (tau, mul, noise_mix) = match self.model {
                        Model::HatClosed => (0.015 + decay * 0.08, 1.0, 0.25),
                        Model::HatOpen => (0.12 + decay * 0.6, 1.0, 0.25),
                        Model::Ride => (0.4 + decay * 1.6, 1.45, 0.1),
                        _ => (0.5 + decay * 1.5, 1.2, 0.6),
                    };
                    let mut metal = 0.0;
                    for k in 0..6 {
                        self.phase[k] = (self.phase[k] + METAL[k] * mul * ratio * (1.0 + punch * 0.6) * dt).fract();
                        metal += if self.phase[k] < 0.5 { 1.0 } else { -1.0 };
                    }
                    let n = white(&mut self.rng);
                    let x = metal / 6.0 * (1.0 - noise_mix) + n * noise_mix;
                    let (_, bp, _) = self.f1.run(x, 7000.0 + tone * 5000.0, 1.0, sr);
                    let (_, _, hp) = self.f2.run(bp, 6000.0 + tone * 2000.0, 0.7, sr);
                    done_after = tau * 7.0;
                    hp * env(tau) * 2.2 * self.choke
                }
                Model::Tom | Model::Conga => {
                    let (base, tau, pamt) =
                        if self.model == Model::Tom { (100.0, 0.15 + decay * 0.6, 0.8) } else { (210.0, 0.06 + decay * 0.25, 0.4) };
                    let f = base * ratio * (1.0 + pamt * punch * (-t / 0.03).exp());
                    self.phase[0] = (self.phase[0] + f * dt).fract();
                    let slap = white(&mut self.rng) * env(0.003) * tone * 0.5;
                    done_after = tau * 7.0;
                    (self.phase[0] * TAU).sin() * env(tau) + slap
                }
                Model::Rim => {
                    self.phase[0] = (self.phase[0] + 1700.0 * ratio * dt).fract();
                    let tri = 1.0 - 4.0 * (self.phase[0] - 0.5).abs();
                    let (_, bp, _) = self.f1.run(white(&mut self.rng), 3000.0 * ratio, 3.0, sr);
                    done_after = 0.3;
                    tri * env(0.008 + decay * 0.03) + bp * env(0.002) * (1.0 + tone)
                }
                Model::Cowbell => {
                    let mut s = 0.0;
                    for (k, f) in [540.0, 800.0].iter().enumerate() {
                        self.phase[k] = (self.phase[k] + f * ratio * dt).fract();
                        s += if self.phase[k] < 0.5 { 0.5 } else { -0.5 };
                    }
                    let (_, bp, _) = self.f1.run(s, 1500.0 + tone * 2000.0, 1.5, sr);
                    let tau = 0.05 + decay * 0.4;
                    done_after = tau * 7.0;
                    bp * (env(0.01) * punch * 0.8 + env(tau)) * 1.4
                }
                Model::Shaker => {
                    let (_, _, hp) = self.f1.run(white(&mut self.rng), 5000.0 + tone * 4000.0, 0.7, sr);
                    let att = (t / (0.002 + (1.0 - punch) * 0.02)).min(1.0);
                    let tau = 0.02 + decay * 0.15;
                    done_after = tau * 8.0 + 0.03;
                    hp * att * env(tau) * 1.2
                }
                Model::Clave => {
                    self.phase[0] = (self.phase[0] + 2500.0 * ratio * dt).fract();
                    done_after = 0.4;
                    (self.phase[0] * TAU).sin() * env(0.015 + decay * 0.06)
                }
                Model::Zap => {
                    let f = (60.0 + 4000.0 * ratio * (-t / (0.01 + punch * 0.05)).exp()).min(sr * 0.4);
                    self.phase[0] = (self.phase[0] + f * dt).fract();
                    let tau = 0.05 + decay * 0.4;
                    done_after = tau * 7.0;
                    let s = (self.phase[0] * TAU).sin();
                    (s * (1.0 + tone * 4.0)).tanh() * env(tau)
                }
            }
        };
        if self.choke < 1.0 {
            self.choke *= 0.995;
            if self.choke < 1e-3 {
                self.active = false;
            }
        }
        if t > done_after {
            self.active = false;
        }
        // DC blocker per voice
        let y = out - self.dc.0 + 0.995 * self.dc.1;
        self.dc = (out, y);
        if y.is_finite() {
            y * self.vel
        } else {
            self.active = false;
            0.0
        }
    }
}

// --------------------------------------------------------------- reverb

struct Comb {
    buf: Vec<f32>,
    i: usize,
    lp: f32,
}

struct Room {
    combs: Vec<Comb>,
    aps: Vec<(Vec<f32>, usize)>,
}

impl Room {
    fn new(sr: f32) -> Self {
        let s = sr / 44_100.0;
        let combs = [1116.0, 1188.0, 1277.0, 1356.0, 1422.0, 1491.0]
            .iter()
            .map(|n: &f32| Comb { buf: vec![0.0; (n * s) as usize + 1], i: 0, lp: 0.0 })
            .collect();
        let aps = [556.0, 441.0].iter().map(|n: &f32| (vec![0.0; (n * s) as usize + 1], 0usize)).collect();
        Room { combs, aps }
    }

    #[inline]
    fn run(&mut self, x: f32, size: f32) -> f32 {
        let fb = 0.7 + size.clamp(0.0, 1.0) * 0.27;
        let mut out = 0.0;
        for c in self.combs.iter_mut() {
            let len = c.buf.len();
            let y = c.buf[c.i % len];
            c.lp += (y - c.lp) * 0.6;
            c.buf[c.i % len] = x + c.lp * fb;
            c.i = (c.i + 1) % len;
            out += y;
        }
        let mut y = out / 6.0;
        for (buf, i) in self.aps.iter_mut() {
            let len = buf.len();
            let d = buf[*i % len];
            let w = y + 0.5 * d;
            buf[*i % len] = w;
            *i = (*i + 1) % len;
            y = d - 0.5 * w;
        }
        y
    }
}

// ----------------------------------------------------------- the engine

#[derive(Clone, Copy, Default)]
struct Pending {
    lane: u8,
    vel: f32,
    delay: i64,
}

/// What happened during a block (for the UI).
#[derive(Clone, Copy, Default)]
pub struct Report {
    pub step: usize,
    pub bar: u32,
    pub slot: usize,
    pub in_fill: bool,
    pub flashes: [f32; LANES],
    pub peak: f32,
}

pub struct Engine {
    voices: [Voice; LANES],
    pending: [Pending; MAX_PENDING],
    n_pending: usize,
    step: usize,
    pos: f64,
    started: bool,
    bar: u32,
    slot: usize,
    last_desired: usize,
    in_fill: bool,
    rng: u32,
    room: Room,
    fat_lp: [f32; 2],
    glue_env: f32,
    filt: [Svf; 2],
    flashes: [f32; LANES],
    pub kick_tap: Vec<f32>,
}

impl Engine {
    /// Allocates (reverb, kick tap) -- build off the audio thread.
    pub fn new(sr: f32) -> Self {
        Engine {
            voices: [Voice::default(); LANES],
            pending: [Pending::default(); MAX_PENDING],
            n_pending: 0,
            step: 0,
            pos: 0.0,
            started: false,
            bar: 0,
            slot: 0,
            last_desired: 0,
            in_fill: false,
            rng: 0x9E37_79B9,
            room: Room::new(sr.max(8000.0)),
            fat_lp: [0.0; 2],
            glue_env: 0.0,
            filt: [Svf::default(); 2],
            flashes: [0.0; LANES],
            kick_tap: Vec::with_capacity(8192),
        }
    }

    pub fn seed(&mut self, s: u32) {
        self.rng = s | 1;
    }

    fn trigger(&mut self, lane: usize, vel: f32, s: &Settings) {
        let model = s.lanes[lane].model;
        if model == Model::HatClosed && !s.lanes[lane].use_sample {
            for v in self.voices.iter_mut() {
                if v.active && v.model == Model::HatOpen {
                    v.choke = 0.99;
                }
            }
        }
        self.voices[lane].trigger(model, vel.clamp(0.0, 1.0));
        self.flashes[lane] = vel;
    }

    fn schedule(&mut self, lane: usize, vel: f32, delay: i64) {
        if self.n_pending < MAX_PENDING {
            self.pending[self.n_pending] = Pending { lane: lane as u8, vel, delay: delay.max(0) };
            self.n_pending += 1;
        }
    }

    fn step_offset(&self, step: usize, s: &Settings, step_len: f64) -> f64 {
        let swing = s.swing.clamp(0.0, 1.0) as f64 * 0.5;
        if s.swing8 {
            if step % 4 == 2 {
                swing * 2.0 * step_len
            } else {
                0.0
            }
        } else if step % 2 == 1 {
            swing * step_len
        } else {
            0.0
        }
    }

    /// Queue every hit of `step` (from `pat`) `base` samples from now.
    fn queue_step(&mut self, pat: &Pattern, step: usize, base: f64, s: &Settings, step_len: f64, sr: f32) {
        let any_solo = s.lanes.iter().any(|l| l.solo);
        let swing = self.step_offset(step, s, step_len);
        for lane in 0..LANES {
            let ls = &s.lanes[lane];
            if ls.mute || (any_solo && !ls.solo) {
                continue;
            }
            let st = pat.steps[lane][step % MAX_STEPS];
            if !st.on() {
                continue;
            }
            let p = st.prob as f32 / 100.0 * ls.chance.clamp(0.0, 1.0);
            if p < 1.0 && uni(&mut self.rng) > p {
                continue;
            }
            let micro = st.micro.clamp(-50, 50) as f64 / 100.0 * step_len * 0.5;
            let human = white(&mut self.rng) as f64 * s.human_time.clamp(0.0, 1.0) as f64 * 0.012 * sr as f64;
            let mut vel = st.vel as f32 / 127.0;
            vel *= 1.0 + white(&mut self.rng) * s.human_vel.clamp(0.0, 1.0) * 0.35;
            let r = st.ratchet.clamp(1, 4) as usize;
            let when = base + swing + micro + human;
            for k in 0..r {
                let v = vel * (1.0 - k as f32 * 0.12);
                self.schedule(lane, v, (when + k as f64 * step_len / r as f64) as i64);
            }
        }
    }

    /// Render one block. `bank` = 8 slots + the fill bar.
    pub fn process(
        &mut self,
        s: &Settings,
        bank: &[Pattern; BANK],
        samples: &[Option<Arc<SampleBuf>>; LANES],
        out_l: &mut [f32],
        out_r: &mut [f32],
        sr: f32,
    ) -> Report {
        let frames = out_l.len().min(out_r.len());
        let sr = if sr > 1000.0 { sr } else { 48_000.0 };
        let step_len = sr as f64 * 60.0 / s.bpm.clamp(20.0, 400.0) as f64 / 4.0;
        self.kick_tap.clear();
        self.kick_tap.resize(frames, 0.0);
        if s.slot != self.last_desired {
            self.last_desired = s.slot.min(SLOTS - 1);
            if !s.playing {
                self.slot = self.last_desired;
            }
        }

        // pads
        for lane in 0..LANES {
            if s.audition[lane] > 0 {
                self.trigger(lane, s.audition[lane] as f32 / 127.0, s);
            }
        }

        if !s.playing {
            self.started = false;
            self.n_pending = 0;
        }

        let any_solo = s.lanes.iter().any(|l| l.solo);
        let lane_gain: [(f32, f32); LANES] = std::array::from_fn(|i| {
            let ls = &s.lanes[i];
            let g = ls.level.clamp(0.0, 1.5);
            let g = g * g;
            let p = (ls.pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * PI;
            (g * p.cos() * 1.414, g * p.sin() * 1.414)
        });
        let sends: [f32; LANES] = std::array::from_fn(|i| s.lanes[i].send.clamp(0.0, 1.0));
        let fat = s.fatten.clamp(0.0, 1.0);
        let fat_drive = 1.0 + fat * 5.0;
        let glue = s.glue.clamp(0.0, 1.0);
        let glue_att = 1.0 - (-1.0 / (0.003 * sr)).exp();
        let glue_rel = 1.0 - (-1.0 / (0.12 * sr)).exp();
        let filt = s.filter.clamp(-1.0, 1.0);
        let gain = s.level.clamp(0.0, 1.5);
        let mut peak = 0.0f32;

        for n in 0..frames {
            if s.playing {
                if !self.started {
                    self.started = true;
                    self.pos = 0.0;
                    self.step = 0;
                    self.bar = 0;
                    self.in_fill = false;
                    let pat = bank[self.slot];
                    self.queue_step(&pat, 0, 0.0, s, step_len, sr);
                    let nxt = 1 % pat.length.max(1);
                    let p2 = self.pattern_for(bank, nxt, s);
                    self.queue_step(&p2, nxt, step_len, s, step_len, sr);
                }
                self.pos += 1.0;
                if self.pos >= step_len {
                    self.pos -= step_len;
                    let len = bank[self.slot].length.clamp(1, MAX_STEPS);
                    self.step += 1;
                    if self.step % 16 == 0 {
                        self.bar += 1;
                    }
                    if self.step >= len {
                        self.step = 0;
                        // pattern boundary: queued slot change or chain
                        if self.last_desired != self.slot {
                            self.slot = self.last_desired;
                        } else if s.chain {
                            for k in 1..=SLOTS {
                                let c = (self.slot + k) % SLOTS;
                                if !bank[c].is_empty() {
                                    self.slot = c;
                                    break;
                                }
                            }
                        }
                    }
                    // queue the step after this one (one-step lookahead
                    // lets micro-timing pull hits early)
                    let len = bank[self.slot].length.clamp(1, MAX_STEPS);
                    let nxt = (self.step + 1) % len;
                    let pat = self.pattern_for(bank, nxt, s);
                    self.queue_step(&pat, nxt, step_len - self.pos, s, step_len, sr);
                }
            }

            // fire due hits
            let mut i = 0;
            while i < self.n_pending {
                if self.pending[i].delay <= 0 {
                    let p = self.pending[i];
                    self.trigger(p.lane as usize, p.vel, s);
                    self.n_pending -= 1;
                    self.pending[i] = self.pending[self.n_pending];
                } else {
                    self.pending[i].delay -= 1;
                    i += 1;
                }
            }

            // voices
            let (mut l, mut r, mut send) = (0.0f32, 0.0f32, 0.0f32);
            for lane in 0..LANES {
                let ls = &s.lanes[lane];
                if !self.voices[lane].active {
                    continue;
                }
                let smp = samples[lane].as_deref();
                let x = self.voices[lane].render(ls, s.tone_shift, smp, sr);
                if ls.mute || (any_solo && !ls.solo) {
                    continue;
                }
                l += x * lane_gain[lane].0;
                r += x * lane_gain[lane].1;
                send += x * sends[lane];
                if lane == 0 {
                    self.kick_tap[n] = x;
                }
            }
            if s.room > 0.0 || send.abs() > 0.0 {
                let wet = self.room.run(send * 0.5, s.room);
                l += wet;
                r += wet;
            }

            // fatten: drive + low-end weight
            if fat > 0.0 {
                for (k, x) in [&mut l, &mut r].into_iter().enumerate() {
                    self.fat_lp[k] += (*x - self.fat_lp[k]) * 0.02;
                    let driven = (*x * fat_drive).tanh() / fat_drive.tanh();
                    *x = *x + (driven - *x) * fat + self.fat_lp[k] * fat * 0.6;
                }
            }
            // glue compressor
            if glue > 0.0 {
                let lvl = l.abs().max(r.abs());
                let c = if lvl > self.glue_env { glue_att } else { glue_rel };
                self.glue_env += (lvl - self.glue_env) * c;
                let thr = 0.5 - glue * 0.35;
                let g = if self.glue_env > thr { (thr / self.glue_env).powf(glue * 0.75) } else { 1.0 };
                let makeup = 1.0 + glue * 0.8;
                l *= g * makeup;
                r *= g * makeup;
            }
            // DJ filter
            if filt.abs() > 0.02 {
                for (k, x) in [&mut l, &mut r].into_iter().enumerate() {
                    let (lp, _, hp) = if filt < 0.0 {
                        self.filt[k].run(*x, 20_000.0 * (1.0 + filt).powf(3.0).max(0.004), 0.9, sr)
                    } else {
                        self.filt[k].run(*x, 20.0 + 6000.0 * filt.powf(2.0), 0.9, sr)
                    };
                    *x = if filt < 0.0 { lp } else { hp };
                }
            }
            l = (l * gain).tanh();
            r = (r * gain).tanh();
            if !l.is_finite() {
                l = 0.0;
            }
            if !r.is_finite() {
                r = 0.0;
            }
            peak = peak.max(l.abs()).max(r.abs());
            out_l[n] = l;
            out_r[n] = r;
        }

        let rep = Report { step: self.step, bar: self.bar, slot: self.slot, in_fill: self.in_fill, flashes: self.flashes, peak };
        for f in self.flashes.iter_mut() {
            *f *= 0.6;
        }
        rep
    }

    /// The pattern a given step comes from: the slot, or the fill bar on the
    /// last bar of each N-bar auto-fill cycle.
    fn pattern_for(&mut self, bank: &[Pattern; BANK], step: usize, s: &Settings) -> Pattern {
        let pat = bank[self.slot];
        let len = pat.length.clamp(1, MAX_STEPS);
        if s.fill_every > 0 && !bank[FILL].is_empty() {
            let bars_per_pat = (len / 16).max(1) as u32;
            let cycle = s.fill_every.max(1);
            // bar number of `step` within the running song
            let bar_in_pat = (step / 16) as u32;
            let pat_count = self.bar / bars_per_pat;
            let global_bar = pat_count * bars_per_pat + bar_in_pat;
            if global_bar % cycle == cycle - 1 {
                self.in_fill = true;
                let mut out = pat;
                let base = (step / 16) * 16;
                for lane in 0..LANES {
                    for k in 0..16 {
                        if base + k < MAX_STEPS {
                            out.steps[lane][base + k] = bank[FILL].steps[lane][k];
                        }
                    }
                }
                return out;
            }
        }
        self.in_fill = false;
        pat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(e: &mut Engine, s: &Settings, bank: &[Pattern; BANK], secs: f32) -> (Vec<f32>, Vec<Report>) {
        let none: [Option<Arc<SampleBuf>>; LANES] = Default::default();
        let (mut l, mut r) = (vec![0.0; 480], vec![0.0; 480]);
        let mut out = Vec::new();
        let mut reps = Vec::new();
        for _ in 0..(secs * 100.0) as usize {
            reps.push(e.process(s, bank, &none, &mut l, &mut r, 48_000.0));
            out.extend_from_slice(&l);
        }
        (out, reps)
    }

    fn four_floor() -> [Pattern; BANK] {
        let mut b = [Pattern::default(); BANK];
        for i in [0, 4, 8, 12] {
            b[0].steps[0][i] = Step::hit(110);
        }
        b
    }

    #[test]
    fn every_model_sounds_and_ends() {
        let none: [Option<Arc<SampleBuf>>; LANES] = Default::default();
        for m in MODELS {
            let mut e = Engine::new(48_000.0);
            let mut s = Settings { playing: false, ..Settings::default() };
            s.lanes[0].model = m;
            s.audition[0] = 120;
            let bank = [Pattern::default(); BANK];
            let (mut l, mut r) = (vec![0.0; 480], vec![0.0; 480]);
            let mut peak = 0.0f32;
            e.process(&s, &bank, &none, &mut l, &mut r, 48_000.0);
            peak = peak.max(l.iter().fold(0.0, |a, x| a.max(x.abs())));
            s.audition[0] = 0;
            for _ in 0..1200 {
                e.process(&s, &bank, &none, &mut l, &mut r, 48_000.0);
                assert!(l.iter().all(|x| x.is_finite()));
            }
            assert!(peak > 0.02, "{} silent", m.name());
            assert!(!e.voices[0].active, "{} never ends", m.name());
        }
    }

    #[test]
    fn four_on_the_floor_hits_four_times_per_bar_at_120() {
        let mut e = Engine::new(48_000.0);
        let s = Settings::default();
        let (_, reps) = run(&mut e, &s, &four_floor(), 2.0); // one bar = 2 s at 120
        let hits = reps.windows(2).filter(|w| w[1].flashes[0] > w[0].flashes[0] + 0.3).count();
        assert!((4..=5).contains(&hits), "hits {hits}");
    }

    #[test]
    fn swing_delays_odd_sixteenths() {
        let e = Engine::new(48_000.0);
        let s = Settings { swing: 1.0, ..Settings::default() };
        assert_eq!(e.step_offset(0, &s, 100.0), 0.0);
        assert_eq!(e.step_offset(1, &s, 100.0), 50.0);
        let s8 = Settings { swing: 1.0, swing8: true, ..Settings::default() };
        assert_eq!(e.step_offset(2, &s8, 100.0), 100.0);
    }

    #[test]
    fn closed_hat_chokes_open_hat() {
        let mut e = Engine::new(48_000.0);
        let mut s = Settings { playing: false, ..Settings::default() };
        s.lanes[3].model = Model::HatClosed;
        s.lanes[4].model = Model::HatOpen;
        s.lanes[4].decay = 1.0;
        let bank = [Pattern::default(); BANK];
        let none: [Option<Arc<SampleBuf>>; LANES] = Default::default();
        let (mut l, mut r) = (vec![0.0; 480], vec![0.0; 480]);
        s.audition[4] = 120;
        e.process(&s, &bank, &none, &mut l, &mut r, 48_000.0);
        s.audition = [0; LANES];
        s.audition[3] = 100;
        e.process(&s, &bank, &none, &mut l, &mut r, 48_000.0);
        s.audition = [0; LANES];
        for _ in 0..10 {
            e.process(&s, &bank, &none, &mut l, &mut r, 48_000.0);
        }
        assert!(!e.voices[4].active, "open hat should be choked");
    }

    #[test]
    fn mute_solo_and_queued_slot_change() {
        let mut e = Engine::new(48_000.0);
        let mut bank = four_floor();
        bank[1].steps[1][4] = Step::hit(100);
        let mut s = Settings::default();
        s.lanes[0].mute = true;
        let (out, _) = run(&mut e, &s, &bank, 1.0);
        assert!(out.iter().all(|x| x.abs() < 1e-4), "muted kick is silent");
        s.lanes[0].mute = false;
        s.slot = 1;
        let (_, reps) = run(&mut e, &s, &bank, 3.0);
        assert_eq!(reps.last().unwrap().slot, 1, "switches at the bar line");
    }

    #[test]
    fn mix_fx_stay_bounded() {
        let mut e = Engine::new(48_000.0);
        let mut s = Settings { fatten: 1.0, glue: 1.0, filter: -0.7, room: 1.0, level: 1.5, ..Settings::default() };
        for l in s.lanes.iter_mut() {
            l.send = 1.0;
            l.level = 1.5;
        }
        let mut bank = four_floor();
        for lane in 0..LANES {
            for i in 0..16 {
                bank[0].steps[lane][i] = Step { vel: 127, prob: 100, ratchet: 4, micro: 0 };
            }
        }
        let (out, _) = run(&mut e, &s, &bank, 3.0);
        assert!(out.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
    }
}
