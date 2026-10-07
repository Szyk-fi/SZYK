//! One Blaster voice. While its key is held it *charges*: a tone that climbs
//! and chirps faster as the charge builds (and flutters once it is full).
//! When the key is released it *blasts*: a pitch sweep with a noise burst, as
//! big as the charge was. A quick tap fires a small shot instead.

use super::params::*;
use super::store::Snapshot;
use crate::apps::hydra::dsp::{pulse, saw, triangle, warp, Rng, Svf};
use std::f32::consts::TAU;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Idle,
    Charging,
    Blasting,
}

/// How far into a blast's decay counts as over (about -60 dB).
const DECAY_DB60: f32 = 6.9;

pub struct Voice {
    pub phase: Phase,
    pub note: u8,
    pub age: u64,
    vel: f32,
    /// Pitch ratio of this key (key follow, octave, the pitch input).
    key: f32,
    /// Charge and blast time scaling from the control inputs.
    ct_mul: f32,
    len_mul: f32,
    held: f32,
    charge: f32,
    chirp: f32,
    flutter: f32,
    wobble: f32,
    ph: [f32; 2],
    fm: [f32; 2],
    // fixed when the key is released, from how big the charge was
    t: f32,
    power: f32,
    p0: f32,
    p1: f32,
    sweep: f32,
    length: f32,
    hold: f32,
    noise_amt: f32,
    noise_decay: f32,
    gain: f32,
    rng: Rng,
    nz_hp: Svf,
    nz_lp: Svf,
    crush_hold: f32,
    crush_acc: f32,
}

impl Voice {
    pub fn new(seed: u32) -> Self {
        Self {
            phase: Phase::Idle,
            note: 0,
            age: 0,
            vel: 1.0,
            key: 1.0,
            ct_mul: 1.0,
            len_mul: 1.0,
            held: 0.0,
            charge: 0.0,
            chirp: 0.0,
            flutter: 0.0,
            wobble: 0.0,
            ph: [0.0; 2],
            fm: [0.0; 2],
            t: 0.0,
            power: 0.0,
            p0: 1000.0,
            p1: 100.0,
            sweep: 0.2,
            length: 0.3,
            hold: 0.0,
            noise_amt: 0.0,
            noise_decay: 0.1,
            gain: 1.0,
            rng: Rng::new(seed | 1),
            nz_hp: Svf::default(),
            nz_lp: Svf::default(),
            crush_hold: 0.0,
            crush_acc: 0.0,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// 0..1 how charged this voice is (0 unless it is charging).
    pub fn charge(&self) -> f32 {
        if self.phase == Phase::Charging { self.charge } else { 0.0 }
    }

    /// How big the blast being played was (0 for a tap).
    pub fn power(&self) -> f32 {
        self.power
    }

    /// Starts charging. `pitch_cv` is in octaves, `ct`/`len` multiply the
    /// charge time and blast length.
    pub fn note_on(&mut self, note: u8, velocity: u8, snap: &Snapshot, pitch_cv: f32, ct: f32, len: f32, age: u64) {
        self.phase = Phase::Charging;
        self.note = note;
        self.age = age;
        self.vel = velocity as f32 / 127.0;
        let semis = (note as f32 - 60.0) * snap[P::KeyFollow as usize] + 12.0 * snap[P::Octave as usize];
        self.key = (semis / 12.0 + pitch_cv).exp2();
        self.ct_mul = ct;
        self.len_mul = len;
        self.held = 0.0;
        self.charge = 0.0;
        self.chirp = 0.0;
        self.flutter = 0.0;
        self.power = 0.0;
        self.ph = [0.0; 2];
        self.fm = [0.0; 2];
    }

    /// Lets go of the key: the charge becomes a blast.
    pub fn note_off(&mut self, snap: &Snapshot) {
        if self.phase == Phase::Charging {
            self.fire(snap);
        }
    }

    pub fn kill(&mut self) {
        self.phase = Phase::Idle;
    }

    fn fire(&mut self, snap: &Snapshot) {
        let s = |p: P| snap[p as usize];
        let tapped = self.held < s(P::Tap);
        self.power = if tapped { 0.0 } else { self.charge.powf(s(P::PowerCurve)) };
        let pw = self.power;
        self.p1 = (s(P::BlastEnd) * self.key).max(20.0);
        self.p0 = (s(P::BlastStart) * self.key * (s(P::PowerPitch) * pw).exp2()).max(20.0);
        self.sweep = s(P::SweepTime) * (1.0 + s(P::PowerLength) * pw * 0.5);
        self.length = s(P::Length) * self.len_mul * (1.0 + s(P::PowerLength) * pw);
        self.hold = s(P::Hold) * (1.0 + pw);
        self.noise_amt = (s(P::Noise) * (1.0 + s(P::PowerNoise) * pw * 2.0)).min(1.5);
        self.noise_decay = s(P::NoiseDecay) * (1.0 + s(P::PowerLength) * pw * 0.5);
        let size_gain = (1.0 - s(P::PowerVolume)) + s(P::PowerVolume) * pw;
        let vel_gain = (1.0 - s(P::VelSens)) + s(P::VelSens) * self.vel;
        self.gain = size_gain * vel_gain;
        self.t = 0.0;
        self.ph = [0.0; 2];
        self.fm = [0.0; 2];
        self.phase = Phase::Blasting;
    }

    /// One cycle of waveform `kind` at phase `p` (0..1), `inc` per sample.
    #[inline]
    fn wave(&mut self, idx: usize, kind: usize, inc: f32, width: f32, mod_in: f32) -> f32 {
        self.ph[idx] += inc;
        if self.ph[idx] >= 1.0 {
            self.ph[idx] -= self.ph[idx].floor();
        }
        let p = (self.ph[idx] + mod_in).rem_euclid(1.0);
        match kind {
            0 => pulse(p, inc, 0.5),
            1 => saw(p, inc),
            2 => triangle(p, inc),
            3 => (TAU * p).sin(),
            4 => pulse(p, inc, width),
            5 => self.rng.bipolar(),
            _ => {
                // an inharmonic two-operator tone: a ringing, metallic clang
                self.fm[idx] = (self.fm[idx] + inc * 1.4142).fract();
                (TAU * p + 2.8 * (TAU * self.fm[idx]).sin()).sin()
            }
        }
    }

    /// Renders (adds to) `out`. `rate` is the sample rate.
    pub fn render(&mut self, out: &mut [f32], snap: &Snapshot, rate: f32) {
        let dt = 1.0 / rate;
        for o in out.iter_mut() {
            let raw = match self.phase {
                Phase::Idle => return,
                Phase::Charging => {
                    let y = self.charge_sample(snap, dt, rate);
                    if snap[P::AutoFire as usize] >= 0.5 && self.charge >= 1.0 {
                        self.fire(snap);
                    }
                    y
                }
                Phase::Blasting => self.blast_sample(snap, dt, rate),
            };
            *o += self.finish(raw, snap, rate);
        }
    }

    fn charge_sample(&mut self, s: &Snapshot, dt: f32, rate: f32) -> f32 {
        let g = |p: P| s[p as usize];
        let ct = (g(P::ChargeTime) * self.ct_mul).max(0.05);
        self.held += dt;
        self.charge = (self.held / ct).min(1.0);
        let full = self.charge >= 1.0;
        let steps = g(P::ClimbSteps) as i32;
        let q = if steps > 0 { ((self.charge * steps as f32).floor() / steps as f32).min(1.0) } else { self.charge };
        let mode = g(P::FullMode) as usize;
        let mut oct = g(P::Climb) * q;
        let mut amp = 1.0;
        if full && mode == 1 {
            // flutter: jump between the full-charge pitch and one higher
            self.flutter = (self.flutter + g(P::FlutterRate) * dt).fract();
            if self.flutter >= 0.5 {
                oct += g(P::FlutterSemis) / 12.0;
            }
        } else if full && mode == 2 {
            self.flutter = (self.flutter + g(P::FlutterRate) * dt).fract();
            if self.flutter >= 0.5 {
                amp = 0.25;
            }
        } else {
            // the chirps: a rising sweep that restarts, faster as it charges
            let rate_hz = (g(P::ChirpRate0).ln() * (1.0 - self.charge) + g(P::ChirpRate1).ln() * self.charge).exp();
            self.chirp = (self.chirp + rate_hz * dt).fract();
            oct += g(P::ChirpDepth) * self.chirp.powf(g(P::ChirpCurve));
            // a short fade-in after each restart hides the jump back down
            amp = (self.chirp * 40.0).min(1.0);
        }
        self.wobble = (self.wobble + 6.0 * dt).fract();
        oct += g(P::ChgWobble) * 0.12 * (TAU * self.wobble).sin();
        let f = (g(P::ChgPitch) * self.key * oct.exp2()).clamp(20.0, 0.45 * rate);
        let tone = self.wave(0, g(P::ChgWave) as usize, f * dt, 0.25, 0.0);
        let mut bed = 0.0;
        if g(P::ChgNoise) > 0.0 {
            let n = self.rng.bipolar();
            bed = self.nz_hp.tick(n, warp(1500.0, rate), 1.5).hp * g(P::ChgNoise) * self.charge * 0.6;
        }
        let vol = g(P::ChgVol) + (1.0 - g(P::ChgVol)) * self.charge;
        let boost = if full { 1.0 + 0.4 * g(P::FullBoost) } else { 1.0 };
        let vel_gain = (1.0 - g(P::VelSens)) + g(P::VelSens) * self.vel;
        (tone + bed) * vol * amp * boost * vel_gain * 0.5
    }

    fn blast_sample(&mut self, s: &Snapshot, dt: f32, rate: f32) -> f32 {
        let g = |p: P| s[p as usize];
        let t = self.t;
        self.t += dt;
        let x = (1.0 - (t / self.sweep).min(1.0)).powf(g(P::SweepCurve));
        let f = (self.p1 * (self.p0 / self.p1).powf(x)).clamp(20.0, 0.45 * rate);
        let attack = (t / g(P::Attack)).min(1.0);
        let tail = if t < self.hold { 1.0 } else { (-DECAY_DB60 * (t - self.hold) / self.length.max(0.01)).exp() };
        let env = attack * tail;
        let noise_env = (-DECAY_DB60 * t / self.noise_decay.max(0.005)).exp();
        if env < 1e-4 && noise_env < 1e-4 {
            self.phase = Phase::Idle;
            return 0.0;
        }
        let kind = g(P::BlastWave) as usize;
        let inc = f * dt;
        let body_level = g(P::Body);
        // the body: a second oscillator at an interval, which can also bend the first
        let body = if body_level > 0.0 || g(P::FmIndex) > 0.0 {
            let inc2 = (f * (g(P::BodySemis) / 12.0).exp2()).clamp(20.0, 0.45 * rate) * dt;
            self.wave(1, if kind == 5 { 3 } else { kind }, inc2, g(P::Width), 0.0)
        } else {
            0.0
        };
        let tone = self.wave(0, kind, inc, g(P::Width), g(P::FmIndex) * 0.35 * body);
        let mut y = (tone + body_level * body) * env / (1.0 + body_level * 0.5);
        if self.noise_amt > 0.0 {
            let sweep = g(P::NoiseSweep) * (t / self.noise_decay.max(0.005)).min(1.0);
            let hp = (g(P::NoiseHP) * sweep.exp2()).clamp(20.0, 0.45 * rate);
            let lp = (g(P::NoiseLP) * sweep.exp2()).clamp(100.0, 0.45 * rate);
            let n = self.rng.bipolar();
            let n = self.nz_hp.tick(n, warp(hp, rate), 1.4).hp;
            let n = self.nz_lp.tick(n, warp(lp, rate), 1.4).lp;
            y += n * noise_env * self.noise_amt * 0.8;
        }
        // the click of the shot leaving
        y += g(P::Punch) * (-t * 900.0).exp() * self.rng.bipolar() * 1.5;
        y * self.gain * 0.6
    }

    /// Bit depth, sample rate and drive: the 8-bit grit, applied per voice.
    fn finish(&mut self, y: f32, s: &Snapshot, rate: f32) -> f32 {
        let g = |p: P| s[p as usize];
        let mut y = y;
        let target = g(P::Rate);
        if target < rate * 0.98 {
            self.crush_acc += target / rate;
            if self.crush_acc >= 1.0 {
                self.crush_acc -= 1.0;
                self.crush_hold = y;
            }
            y = self.crush_hold;
        }
        let bits = g(P::Bits);
        if bits < 15.5 {
            let levels = (2.0f32).powf(bits - 1.0);
            y = (y * levels).round() / levels;
        }
        let drive = g(P::Drive);
        if drive > 0.0 {
            let k = 1.0 + drive * 6.0;
            y = (y * k).tanh() / k.sqrt();
        }
        y
    }
}
