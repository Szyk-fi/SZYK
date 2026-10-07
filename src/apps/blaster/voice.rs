//! One Blaster voice. While its key is held it *charges*: a tone that climbs
//! and chirps faster as the charge builds (and flutters once it is full).
//! When the key is released it *blasts*: a pitch sweep with a noise burst, as
//! big as the charge was. A quick tap fires a small shot instead.

use super::params::*;
use super::store::Snapshot;
use crate::apps::hydra::dsp::{pulse, saw, triangle, warp, Rng, Svf};
use std::f32::consts::TAU;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Idle,
    Charging,
    Blasting,
}

/// Levels (dB re the fundamental) of the first sixteen harmonics of the held
/// full-charge tone of a classic charge shot, measured from a recording of it:
/// strong even harmonics and nearly missing 5th, 7th, 11th and 13th, which no
/// plain square, saw or pulse has. This is what the "Reed" wave plays.
pub(super) const REED_DB: [f32; 16] = [0.0, 2.0, -7.4, -6.6, -21.1, -7.6, -20.3, -14.8, -14.9, -6.1, -19.5, -8.4, -21.5, -28.1, -51.7, -34.5];
const REED_LEN: usize = 2048;

fn reed_table() -> &'static [f32; REED_LEN] {
    static TABLE: OnceLock<[f32; REED_LEN]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0.0f32; REED_LEN];
        for (i, o) in t.iter_mut().enumerate() {
            let ph = i as f32 / REED_LEN as f32;
            *o = REED_DB.iter().enumerate().map(|(k, db)| 10.0f32.powf(db / 20.0) * (TAU * ph * (k + 1) as f32).sin()).sum();
        }
        let peak = t.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
        t.iter_mut().for_each(|v| *v /= peak);
        t
    })
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
    /// 0..1 how far the pitch has glided to its full-charge hold.
    full_blend: f32,
    ph: [f32; 2],
    fm: [f32; 2],
    sub_ph: f32,
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
    noise_end: f32,
    gain: f32,
    rng: Rng,
    nz_hp: Svf,
    nz_lp: Svf,
    nz_lp2: Svf,
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
            full_blend: 0.0,
            ph: [0.0; 2],
            fm: [0.0; 2],
            sub_ph: 0.0,
            t: 0.0,
            power: 0.0,
            p0: 1000.0,
            p1: 100.0,
            sweep: 0.2,
            length: 0.3,
            hold: 0.0,
            noise_amt: 0.0,
            noise_decay: 0.1,
            noise_end: 0.0,
            gain: 1.0,
            rng: Rng::new(seed | 1),
            nz_hp: Svf::default(),
            nz_lp: Svf::default(),
            nz_lp2: Svf::default(),
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
        self.full_blend = 0.0;
        self.power = 0.0;
        self.ph = [0.0; 2];
        self.fm = [0.0; 2];
        self.sub_ph = 0.0;
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
        self.noise_end = s(P::NoiseEnd) * (1.0 + s(P::PowerLength) * pw);
        let size_gain = (1.0 - s(P::PowerVolume)) + s(P::PowerVolume) * pw;
        let vel_gain = (1.0 - s(P::VelSens)) + s(P::VelSens) * self.vel;
        self.gain = size_gain * vel_gain;
        self.t = 0.0;
        self.ph = [0.0; 2];
        self.fm = [0.0; 2];
        self.sub_ph = 0.0;
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
            7 => {
                let x = p * REED_LEN as f32;
                let (i, f) = (x as usize % REED_LEN, x.fract());
                let t = reed_table();
                t[i] + (t[(i + 1) % REED_LEN] - t[i]) * f
            }
            _ => {
                // an inharmonic two-operator tone: a ringing, metallic clang
                self.fm[idx] = (self.fm[idx] + inc * 1.4142).fract();
                (TAU * p + 2.8 * (TAU * self.fm[idx]).sin()).sin()
            }
        }
    }

    /// The sub oscillator: a sine, triangle or square an octave or two below
    /// the tone it follows, to bring out the bass. `f` is the tone's frequency.
    #[inline]
    fn sub(&mut self, s: &Snapshot, f: f32, dt: f32) -> f32 {
        let ratio = if s[P::SubOctave as usize] >= 0.5 { 0.25 } else { 0.5 };
        let inc = (f * ratio).max(20.0) * dt;
        self.sub_ph += inc;
        if self.sub_ph >= 1.0 {
            self.sub_ph -= self.sub_ph.floor();
        }
        match s[P::SubWave as usize] as usize {
            0 => (TAU * self.sub_ph).sin(),
            1 => triangle(self.sub_ph, inc),
            _ => pulse(self.sub_ph, inc, 0.5),
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
        let climb = g(P::Climb) * q;
        // how deep the chirps go: constant, or growing as the charge builds
        let grow = g(P::ChirpGrow);
        let depth = g(P::ChirpDepth) * (1.0 - grow + grow * self.charge);
        // Once full, "keep chirping" carries on; flutter and ripple stop the chirps.
        let chirping = !(full && mode != 0);
        let count = g(P::ChirpCount) as i32;
        if chirping {
            if count > 0 {
                // a set number of ramps counted back from full, so the last one
                // ends exactly as the charge completes
                self.chirp = if full { 1.0 } else { 1.0 - (count as f32 * (1.0 - self.charge)).fract() };
            } else {
                // free-running: a rising sweep that restarts, faster as it charges
                let rate_hz = (g(P::ChirpRate0).ln() * (1.0 - self.charge) + g(P::ChirpRate1).ln() * self.charge).exp();
                self.chirp = (self.chirp + rate_hz * dt).fract();
            }
        }
        let mut oct = climb + depth * self.chirp.powf(g(P::ChirpCurve));
        let mut amp = 1.0;
        if chirping && count == 0 {
            // a short fade-in after each restart hides the jump back down
            amp = (self.chirp * 40.0).min(1.0);
        }
        if full && mode != 0 {
            // glide to where the full-charge tone holds: the bottom of the
            // chirp, or (Hold at top) the top of it
            self.full_blend = (self.full_blend + dt / 0.08).min(1.0);
            let mut hold = climb + depth * g(P::HoldTop);
            self.flutter = (self.flutter + g(P::FlutterRate) * dt).fract();
            if mode == 1 {
                // flutter: jump between the full-charge pitch and one higher
                if self.flutter >= 0.5 {
                    hold += g(P::FlutterSemis) / 12.0;
                }
            } else {
                // ripple: a smooth shimmer in the volume
                amp *= 1.0 - g(P::RippleDepth) * (0.5 - 0.5 * (TAU * self.flutter).cos());
            }
            oct += (hold - oct) * self.full_blend;
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
        let sub = if g(P::SubCharge) > 0.0 { self.sub(s, f, dt) * g(P::SubCharge) } else { 0.0 };
        (tone + sub + bed) * vol * amp * boost * vel_gain * g(P::ChgLevel) * 0.5
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
        let mut noise_env = (-DECAY_DB60 * t / self.noise_decay.max(0.005)).exp();
        // a noise burst that is cut off (a fast fade at its end) rather than left to decay
        let noise_gate = if self.noise_end > 0.0 { ((self.noise_end - t) / 0.06).clamp(0.0, 1.0) } else { 1.0 };
        if self.noise_end > 0.0 && t >= self.noise_end {
            noise_env = 0.0;
        }
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
        let mut y = (tone + body_level * body) * env / (1.0 + body_level * 0.5) * g(P::ToneLevel);
        // the sub follows the sweep an octave or two down, whatever the tone level
        if g(P::SubBlast) > 0.0 {
            y += self.sub(s, f, dt) * env * g(P::SubBlast);
        }
        if self.noise_amt > 0.0 {
            let sweep_time = if g(P::NoiseSweepTime) > 0.0 { g(P::NoiseSweepTime) } else { self.noise_decay };
            let sweep = g(P::NoiseSweep) * (t / sweep_time.max(0.005)).min(1.0).powf(g(P::NoiseSweepCurve));
            let hp = (g(P::NoiseHP) * sweep.exp2()).clamp(20.0, 0.45 * rate);
            let lp = (g(P::NoiseLP) * sweep.exp2()).clamp(100.0, 0.45 * rate);
            let n = self.rng.bipolar();
            let n = self.nz_hp.tick(n, warp(hp, rate), 1.4).hp;
            // two poles in series: the high cut is 24 dB/octave, as steep as a real one
            let n = self.nz_lp.tick(n, warp(lp, rate), 1.4).lp;
            let n = self.nz_lp2.tick(n, warp(lp, rate), 1.4).lp;
            y += n * noise_env * noise_gate * attack * self.noise_amt * 0.8;
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

