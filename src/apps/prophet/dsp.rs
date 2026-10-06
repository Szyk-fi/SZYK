//! Original virtual-analogue DSP, not Profree-4 firmware or an electrical
//! model of Curtis/SSM chips. A four-stage TPT low-pass solves its feedback
//! loop algebraically. PolyBLEP removes the free-running saw/pulse edges;
//! hard sync resets A from B at a fractional sample position. Sync and
//! strong audio-rate FM can still alias at high notes (2x oversampling).
use super::{spec::*, KeyState, Shared};
use crate::audio::AudioProcessor;
use std::f32::consts::{PI, TAU};
use std::sync::{atomic::Ordering, Arc};

#[derive(Clone, Copy, Default)]
struct Env {
    value: f32,
    stage: u8,
}
#[derive(Clone, Copy)]
struct EnvSettings {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}
impl EnvSettings {
    fn new(p: &[f32; N], start: usize, sr: f32) -> Self {
        Self {
            attack: 1. / (seconds(p[start]) * sr),
            decay: (-6.907755 / (seconds(p[start + 1]) * sr)).exp(),
            sustain: p[start + 2],
            release: (-6.907755 / (seconds(p[start + 3]) * sr)).exp(),
        }
    }
}
impl Env {
    fn on(&mut self) {
        self.stage = 1;
    }
    fn off(&mut self) {
        if self.stage != 0 {
            self.stage = 4;
        }
    }
    fn tick(&mut self, s: EnvSettings) -> f32 {
        match self.stage {
            1 => {
                self.value = (self.value + s.attack).min(1.);
                if self.value >= 1. {
                    self.stage = 2;
                }
            }
            2 => {
                self.value = s.sustain + (self.value - s.sustain) * s.decay;
                if (self.value - s.sustain).abs() < 0.001 {
                    self.value = s.sustain;
                    self.stage = 3;
                }
            }
            3 => self.value = s.sustain,
            4 => {
                self.value *= s.release;
                if self.value < 0.00001 {
                    self.value = 0.;
                    self.stage = 0;
                }
            }
            _ => self.value = 0.,
        }
        self.value
    }
}

fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.
    } else if t > 1. - dt {
        let x = (t - 1.) / dt;
        x * x + x + x + 1.
    } else {
        0.
    }
}
fn wave(phase: f32, dt: f32, pw: f32, saw: bool, pulse: bool, triangle: bool) -> f32 {
    let mut x = 0.;
    let mut count = 0.;
    if saw {
        x += 2. * phase - 1. - blep(phase, dt);
        count += 1.;
    }
    if pulse {
        x += (if phase < pw { 1. } else { -1. }) + blep(phase, dt)
            - blep((phase - pw).rem_euclid(1.), dt);
        count += 1.;
    }
    if triangle {
        x += 1. - 4. * (phase - 0.5).abs();
        count += 1.;
    }
    if count > 0. {
        x / count
    } else {
        0.
    }
}
#[derive(Clone, Copy, Default)]
struct Filter {
    z: [f32; 4],
    g: f32,
}
impl Filter {
    fn tune(&mut self, hz: f32, sr: f32) {
        let g = (PI * hz.clamp(15., sr * 0.40) / sr).tan();
        self.g = g / (1. + g);
    }
    fn tick(&mut self, input: f32, resonance: f32) -> f32 {
        let g = self.g;
        let s = (1. - g) * (g * g * g * self.z[0] + g * g * self.z[1] + g * self.z[2] + self.z[3]);
        // Below the oscillation threshold, retaining a bounded, stable filter
        // across automated cutoff sweeps and all supported device rates.
        let k = 3.92 * resonance;
        let mut u = (input - k * s) / (1. + k * g.powi(4));
        for z in &mut self.z {
            let v = (u - *z) * g;
            u = v + *z;
            *z = u + v;
            if z.abs() < 1.0e-20 {
                *z = 0.;
            }
        }
        u
    }
}
#[derive(Clone, Copy)]
struct Voice {
    note: u8,
    held: bool,
    age: u64,
    velocity: f32,
    pitch: f32,
    phase_a: f32,
    phase_b: f32,
    amp: Env,
    env: Env,
    filter: Filter,
    drift_phase: f32,
    drift: f32,
    detune: f32,
    pan: f32,
}
impl Voice {
    fn new(i: usize) -> Self {
        Self {
            note: 60,
            held: false,
            age: 0,
            velocity: 0.,
            pitch: 60.,
            phase_a: (i as f32 * 0.137).fract(),
            phase_b: (i as f32 * 0.317 + 0.25).fract(),
            amp: Env::default(),
            env: Env::default(),
            filter: Filter::default(),
            drift_phase: i as f32 * 0.12,
            drift: 0.,
            detune: 0.,
            pan: 0.,
        }
    }
    fn on(&mut self, n: u8, vel: u8, age: u64, legato: bool, glide: f32) {
        let active = self.amp.stage != 0;
        self.note = n;
        self.velocity = vel as f32 / 127.;
        self.age = age;
        self.held = true;
        if !active || glide <= 0.001 {
            self.pitch = n as f32;
        }
        if !legato || !active {
            self.amp.on();
            self.env.on();
        }
    }
    fn off(&mut self) {
        self.held = false;
        self.amp.off();
        self.env.off();
    }
    fn tick(
        &mut self,
        p: &[f32; N],
        sr: f32,
        lfo: f32,
        bend: f32,
        fa: EnvSettings,
        aa: EnvSettings,
        noise: f32,
        glide_coef: f32,
        counter: usize,
    ) -> f32 {
        if self.amp.stage == 0 {
            return 0.;
        }
        let amp = self.amp.tick(aa);
        let env = self.env.tick(fa);
        self.pitch += (self.note as f32 - self.pitch) * glide_coef;
        if counter % 16 == 0 {
            self.drift_phase = (self.drift_phase + 16. * 0.13 / sr).fract();
            self.drift = (self.drift_phase * TAU).sin() * p[VINTAGE] * 0.08;
        }
        let pitch = self.pitch + bend + self.detune + self.drift;
        let wheel_pitch = if p[W_FREQ] > 0.5 { lfo * 2. } else { 0. };
        let b_pitch = (if p[B_KEY] > 0.5 { pitch } else { 60. + bend }) + wheel_pitch;
        let b_freq = 440.
            * ((b_pitch + p[B_TUNE] + p[B_FINE] / 100. - 69.) / 12.).exp2()
            * if p[B_LOW] > 0.5 { 0.005 } else { 1. };
        let db = (b_freq / sr).clamp(0.0000001, 0.35);
        let b = wave(
            self.phase_b,
            db,
            p[B_PW].clamp(0.05, 0.95),
            p[B_SAW] > 0.5,
            p[B_PULSE] > 0.5,
            p[B_TRI] > 0.5,
        );
        let poly = env * p[POLY_ENV] + b * p[POLY_B];
        let fm = if p[P_FREQ] > 0.5 { poly * 36. } else { 0. };
        let freq = 440. * ((pitch + p[A_TUNE] + fm + wheel_pitch - 69.) / 12.).exp2();
        let da = (freq / sr).clamp(0.0000001, 0.35);
        let pw = (p[A_PW]
            + if p[P_PW] > 0.5 { poly * 0.45 } else { 0. }
            + if p[W_PW] > 0.5 { lfo * 0.4 } else { 0. })
        .clamp(0.05, 0.95);
        let a = wave(
            self.phase_a,
            da,
            pw,
            p[A_SAW] > 0.5,
            p[A_PULSE] > 0.5,
            false,
        );
        let b_pw = (p[B_PW] + if p[W_PW] > 0.5 { lfo * 0.4 } else { 0. }).clamp(0.05, 0.95);
        let b_mix = wave(
            self.phase_b,
            db,
            b_pw,
            p[B_SAW] > 0.5,
            p[B_PULSE] > 0.5,
            p[B_TRI] > 0.5,
        );
        let old_b = self.phase_b;
        self.phase_b += db;
        if self.phase_b >= 1. {
            self.phase_b -= 1.;
            if p[SYNC] > 0.5 {
                self.phase_a = da * (1. - (1. - old_b) / db);
            } else {
                self.phase_a = (self.phase_a + da).fract();
            }
        } else {
            self.phase_a = (self.phase_a + da).fract();
        }
        if counter % 8 == 0 {
            let mut oct =
                env * p[ENV_AMT] * 6. + (self.pitch - 60.) / 12. * p[KEYTRACK] + self.drift * 0.5;
            if p[P_FILTER] > 0.5 {
                oct += poly * 5.;
            }
            if p[W_FILTER] > 0.5 {
                oct += lfo * 3.;
            }
            // Poly-Mod filter modulation is calculated every internal sample,
            // below; other control-rate terms can be cached at 12 kHz.
            let hz = cutoff(p[CUTOFF]) * oct.clamp(-12., 12.).exp2();
            self.filter.tune(hz, sr);
        }
        if p[P_FILTER] > 0.5 && p[POLY_B] > 0. {
            let oct = env * p[ENV_AMT] * 6.
                + (self.pitch - 60.) / 12. * p[KEYTRACK]
                + poly * 5.
                + if p[W_FILTER] > 0.5 { lfo * 3. } else { 0. };
            self.filter
                .tune(cutoff(p[CUTOFF]) * oct.clamp(-12., 12.).exp2(), sr);
        }
        let mixed = (a * p[MIX_A] + b_mix * p[MIX_B] + noise * p[NOISE]) * 0.65;
        let driven = (mixed * (1. + p[DRIVE] * 4.)).tanh();
        let velocity = 1. - p[VELOCITY] + p[VELOCITY] * self.velocity;
        self.filter.tick(driven, p[RES]) * amp * velocity
    }
}

#[derive(Clone, Copy, Default)]
struct Decimator {
    z: [f32; 2],
}
impl Decimator {
    fn tick(&mut self, x: f32) -> f32 {
        // Two cascaded TPT poles before downsampling (not a brick-wall FIR).
        let mut u = x;
        for z in &mut self.z {
            let v = (u - *z) * 0.43;
            u = v + *z;
            *z = u + v;
        }
        u
    }
}

pub struct Processor {
    shared: Arc<Shared>,
    voices: [Voice; 8],
    state: KeyState,
    prior: [u8; 128],
    note_age: [u64; 128],
    age: u64,
    mode: usize,
    count: usize,
    generation: u32,
    lfo_phase: f32,
    sample_hold: f32,
    rng: u32,
    counter: usize,
    smooth: [f32; N],
    decim: [Decimator; 2],
    chorus: Box<[[f32; 2]]>,
    delay: Box<[[f32; 2]]>,
    pos: usize,
    delay_pos: usize,
    chorus_phase: f32,
    tail: f32,
    delay_frames: f32,
}
impl Processor {
    pub fn new(shared: Arc<Shared>) -> Self {
        let smooth = std::array::from_fn(|i| shared.controls.raw(i));
        Self {
            shared,
            voices: std::array::from_fn(Voice::new),
            state: KeyState::default(),
            prior: [0; 128],
            note_age: [0; 128],
            age: 0,
            mode: usize::MAX,
            count: 0,
            generation: u32::MAX,
            lfo_phase: 0.,
            sample_hold: 0.,
            rng: 0x8a93ab71,
            counter: 0,
            smooth,
            decim: [Decimator::default(); 2],
            chorus: vec![[0.; 2]; 8192].into_boxed_slice(),
            delay: vec![[0.; 2]; 192000].into_boxed_slice(),
            pos: 0,
            delay_pos: 0,
            chorus_phase: 0.,
            tail: 0.,
            delay_frames: 0.,
        }
    }
    fn random(rng: &mut u32) -> f32 {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 17;
        *rng ^= *rng << 5;
        *rng as f32 / u32::MAX as f32 * 2. - 1.
    }
    fn read(ring: &[[f32; 2]], pos: usize, delay: f32, c: usize) -> f32 {
        let d = delay.clamp(1., ring.len() as f32 - 2.);
        let whole = d.floor() as usize;
        let f = d.fract();
        let i = (pos + ring.len() - whole) % ring.len();
        let j = (i + ring.len() - 1) % ring.len();
        ring[i][c] * (1. - f) + ring[j][c] * f
    }
    fn update_keys(&mut self, p: &[f32; N]) {
        let mode = p[MODE].round() as usize;
        let count = [4, 5, 8][(p[VOICES].round() as usize).min(2)];
        if self.mode != mode || self.count != count || self.generation != self.state.generation {
            for v in &mut self.voices {
                v.off();
            }
            self.prior = [0; 128];
            self.note_age = [0; 128];
            self.mode = mode;
            self.count = count;
            self.generation = self.state.generation;
        }
        for n in 0..128 {
            if self.state.notes[n] > 0 && self.prior[n] == 0 {
                self.age = self.age.wrapping_add(1);
                self.note_age[n] = self.age;
            }
        }
        if mode == 0 {
            for v in &mut self.voices {
                if v.held && self.state.notes[v.note as usize] == 0 {
                    v.off();
                }
            }
            for n in 0..128 {
                if self.state.notes[n] == 0 || self.prior[n] > 0 {
                    continue;
                }
                let idx = (0..count)
                    .find(|&i| self.voices[i].amp.stage == 0)
                    .or_else(|| {
                        (0..count)
                            .filter(|&i| !self.voices[i].held)
                            .min_by(|&a, &b| {
                                self.voices[a]
                                    .amp
                                    .value
                                    .total_cmp(&self.voices[b].amp.value)
                            })
                    })
                    .unwrap_or_else(|| (0..count).min_by_key(|&i| self.voices[i].age).unwrap_or(0));
                let v = &mut self.voices[idx];
                v.pan = (idx as f32 / (count - 1) as f32 * 2. - 1.) * p[SPREAD];
                v.detune = 0.;
                v.on(
                    n as u8,
                    self.state.notes[n],
                    self.note_age[n],
                    false,
                    p[GLIDE],
                );
            }
        } else {
            let last = (0..128)
                .filter(|&n| self.state.notes[n] > 0)
                .max_by_key(|&n| self.note_age[n]);
            let used = if mode == 1 { 1 } else { count };
            for i in 0..used {
                let v = &mut self.voices[i];
                match last {
                    Some(n) => {
                        let same = v.held && v.note == n as u8;
                        let legato = v.held;
                        v.pan = if mode == 1 {
                            0.
                        } else {
                            (i as f32 / (used - 1) as f32 * 2. - 1.) * p[SPREAD]
                        };
                        v.detune = if mode == 1 {
                            0.
                        } else {
                            (i as f32 / (used - 1) as f32 * 2. - 1.) * p[UNISON_DETUNE] * 0.45
                        };
                        if !same {
                            v.on(
                                n as u8,
                                self.state.notes[n],
                                self.note_age[n],
                                legato,
                                p[GLIDE],
                            );
                        }
                    }
                    None => {
                        if v.held {
                            v.off();
                        }
                    }
                }
            }
        }
        self.prior = self.state.notes;
    }
}
impl AudioProcessor for Processor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        if channels == 0 || !sample_rate.is_finite() || !(8000.0..=192000.0).contains(&sample_rate)
        {
            buffer.fill(0.);
            return;
        }
        if let Ok(s) = self.shared.keys.try_lock() {
            self.state = *s;
        }
        let target: [f32; N] = std::array::from_fn(|i| self.shared.controls.get(i));
        self.update_keys(&target);
        let sr = sample_rate * 2.;
        let fa = EnvSettings::new(&target, FA, sr);
        let aa = EnvSettings::new(&target, AA, sr);
        let glide_coef = if target[GLIDE] < 0.001 {
            1.
        } else {
            1. - (-1. / (sr * (0.004 + target[GLIDE] * 1.5))).exp()
        };
        let slew = 1. - (-1. / (sr * 0.008)).exp();
        let tail_decay = (-1. / (sample_rate * 0.7)).exp();
        let mix = (self.shared.mix_level.get() + self.shared.ext_level.get()).clamp(0., 2.);
        let mut bus = self.shared.output.try_lock().ok();
        let frames = buffer.len() / channels;
        if let Some(b) = bus.as_mut() {
            let len = frames.min(b.capacity());
            b.resize(len, 0.);
        }
        let mut peak = 0.0f32;
        for (frame_i, frame) in buffer.chunks_mut(channels).enumerate() {
            let mut stereo = [0.; 2];
            for _ in 0..2 {
                for i in 0..N {
                    self.smooth[i] = if SPECS[i].is_switch() {
                        target[i]
                    } else {
                        self.smooth[i] + (target[i] - self.smooth[i]) * slew
                    };
                }
                let mut p = self.smooth;
                p[CUTOFF] = (p[CUTOFF] + self.state.stick_x * 0.22 + self.state.aftertouch * 0.12)
                    .clamp(0., 1.);
                let noise = Self::random(&mut self.rng);
                self.lfo_phase += rate(p[LFO_RATE]) / sr;
                if self.lfo_phase >= 1. {
                    self.lfo_phase -= 1.;
                    self.sample_hold = noise;
                }
                let lfo = match p[LFO_SHAPE].round() as usize {
                    1 => 2. * self.lfo_phase - 1.,
                    2 => {
                        if self.lfo_phase < 0.5 {
                            1.
                        } else {
                            -1.
                        }
                    }
                    3 => (TAU * self.lfo_phase).sin(),
                    4 => self.sample_hold,
                    _ => 1. - 4. * (self.lfo_phase - 0.5).abs(),
                };
                let wheel = (self.state.wheel + self.state.hand + p[MOD_AMOUNT]).clamp(0., 1.);
                let modulation = (lfo * (1. - p[WHEEL_NOISE]) + noise * p[WHEEL_NOISE]) * wheel;
                let bend = (self.state.bend + self.state.stick_y).clamp(-1., 1.) * p[BEND_RANGE];
                let mut sums = [0.; 2];
                for (i, v) in self.voices.iter_mut().enumerate() {
                    v.pan = if self.mode == 1 {
                        0.
                    } else {
                        (i as f32 / (self.count.max(2) - 1) as f32 * 2. - 1.) * p[SPREAD]
                    };
                    if self.mode == 2 {
                        v.detune = (i as f32 / (self.count - 1) as f32 * 2. - 1.)
                            * p[UNISON_DETUNE]
                            * 0.45;
                    }
                    let x = v.tick(
                        &p,
                        sr,
                        modulation,
                        bend,
                        fa,
                        aa,
                        noise,
                        glide_coef,
                        self.counter,
                    );
                    let pan = (v.pan + 1.) * PI * 0.25;
                    sums[0] += x * pan.cos();
                    sums[1] += x * pan.sin();
                }
                let gain = if self.mode == 2 {
                    0.45 / (self.count as f32).sqrt()
                } else {
                    0.32
                };
                for c in 0..2 {
                    stereo[c] = self.decim[c].tick(sums[c] * gain);
                }
                self.counter = self.counter.wrapping_add(1);
            }
            let p = &self.smooth;
            self.chorus_phase = (self.chorus_phase + 0.23 / sample_rate).fract();
            self.chorus[self.pos] = stereo;
            for c in 0..2 {
                let depth = (TAU * (self.chorus_phase + c as f32 * 0.25)).sin();
                let wet = Self::read(
                    &self.chorus,
                    self.pos,
                    sample_rate * (0.012 + depth * 0.003),
                    c,
                );
                stereo[c] = stereo[c] * (1. - p[CHORUS] * 0.35) + wet * p[CHORUS] * 0.5;
            }
            self.pos = (self.pos + 1) % self.chorus.len();
            let desired = (0.03 + p[DELAY_TIME] * 0.72) * sample_rate;
            if self.delay_frames == 0. {
                self.delay_frames = desired;
            }
            self.delay_frames += (desired - self.delay_frames) * 0.0003;
            let dl = Self::read(&self.delay, self.delay_pos, self.delay_frames, 0);
            let dr = Self::read(&self.delay, self.delay_pos, self.delay_frames, 1);
            self.delay[self.delay_pos] = [
                (stereo[0] + dr * p[DELAY_FB] * 0.78).tanh(),
                (stereo[1] + dl * p[DELAY_FB] * 0.78).tanh(),
            ];
            self.delay_pos = (self.delay_pos + 1) % self.delay.len();
            stereo[0] += dl * p[DELAY_MIX] * 0.7;
            stereo[1] += dr * p[DELAY_MIX] * 0.7;
            self.tail *= tail_decay;
            self.tail = self.tail.max(stereo[0].abs()).max(stereo[1].abs());
            for c in 0..2 {
                stereo[c] = (stereo[c] * p[VOLUME] * mix).tanh();
                peak = peak.max(stereo[c].abs());
            }
            let mono = (stereo[0] + stereo[1]) * 0.5;
            for (c, x) in frame.iter_mut().enumerate() {
                *x = if channels == 1 {
                    mono
                } else if c < 2 {
                    stereo[c]
                } else {
                    mono
                };
            }
            if let Some(b) = bus.as_mut() {
                if frame_i < b.len() {
                    b[frame_i] = mono;
                }
            }
        }
        self.shared.peak.set(peak);
        self.shared.tail.store(
            self.voices.iter().any(|v| v.amp.stage != 0)
                || (self.tail > 0.00003 && (target[CHORUS] > 0. || target[DELAY_MIX] > 0.)),
            Ordering::Relaxed,
        );
        for (i, v) in self.voices.iter().enumerate() {
            self.shared.meters[i].set(v.amp.value);
        }
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;
    fn gain(freq: f32, res: f32) -> f32 {
        let mut f = Filter::default();
        f.tune(600., 48000.);
        let mut out = 0.;
        let mut input = 0.;
        for i in 0..24000 {
            let x = (TAU * freq * i as f32 / 48000.).sin() * 0.05;
            let y = f.tick(x, res);
            if i >= 12000 {
                out += y * y;
                input += x * x;
            }
        }
        (out / input).sqrt()
    }
    #[test]
    fn the_filter_has_four_poles_and_a_real_resonant_peak() {
        assert!(gain(80., 0.) > 0.9);
        assert!(
            gain(4000., 0.) < gain(2000., 0.) / 10.,
            "two octaves above cutoff must roll off near 24 dB/oct"
        );
        assert!(gain(600., 0.9) > gain(600., 0.) * 2.);
    }
    #[test]
    fn the_envelopes_reach_sustain_and_release_with_the_programmed_times() {
        let mut p = std::array::from_fn(|i| SPECS[i].default);
        p[AA] = time_norm(0.02);
        p[AD] = time_norm(0.1);
        p[AS] = 0.4;
        p[AR] = time_norm(0.08);
        let s = EnvSettings::new(&p, AA, 48000.);
        let mut e = Env::default();
        e.on();
        for _ in 0..960 {
            e.tick(s);
        }
        assert!(e.value > 0.99);
        for _ in 0..5000 {
            e.tick(s);
        }
        assert!((e.value - 0.4).abs() < 0.001);
        e.off();
        for _ in 0..4800 {
            e.tick(s);
        }
        assert!(e.value < 0.0001);
    }
}
