//! Original software DSP for the Rev2 patch architecture. It is not Sequential
//! firmware or a calibrated Curtis circuit model. All storage is preallocated.
use super::{Shared, State};
use crate::audio::AudioProcessor;
use std::{
    f32::consts::{PI, TAU},
    sync::{atomic::Ordering, Arc},
};
fn seconds(x: f32) -> f32 {
    0.001 * 20000.0f32.powf((x / 127.).clamp(0., 1.))
}
fn random(r: &mut u32) -> f32 {
    *r ^= *r << 13;
    *r ^= *r >> 17;
    *r ^= *r << 5;
    *r as f32 / u32::MAX as f32 * 2. - 1.
}
fn frequency(n: f32) -> f32 {
    440. * ((n - 69.) / 12.).exp2()
}
fn blep(t: f32, d: f32) -> f32 {
    if t < d {
        let x = t / d;
        2. * x - x * x - 1.
    } else if t > 1. - d {
        let x = (t - 1.) / d;
        x * x + 2. * x + 1.
    } else {
        0.
    }
}
fn wave(t: f32, d: f32, shape: u8, modulation: f32) -> f32 {
    let pw = (modulation / 100.).clamp(0.02, 0.98);
    let phase = if shape == 4 {
        t
    } else {
        let s = (modulation / 100.).clamp(0.02, 0.98);
        if t < s {
            t * 0.5 / s
        } else {
            0.5 + (t - s) * 0.5 / (1. - s)
        }
    };
    let saw = 2. * phase - 1. - blep(t, d);
    let tri = 1. - 4. * (phase - 0.5).abs();
    match shape {
        1 => saw,
        2 => (saw + tri) * 0.5,
        3 => tri,
        4 => (if t < pw { 1. } else { -1. }) + blep(t, d) - blep((t - pw).rem_euclid(1.), d),
        _ => 0.,
    }
}
#[derive(Clone, Copy, Default)]
struct Envelope {
    v: f32,
    stage: u8,
    delay: f32,
    held: bool,
}
#[derive(Clone, Copy, Default)]
struct EnvCoeffs {
    delay: f32,
    a: f32,
    d: f32,
    s: f32,
    r: f32,
}
impl EnvCoeffs {
    fn new(p: &[f32; 256], e: usize, sr: f32) -> Self {
        Self {
            delay: if p[38 + e] > 0. {
                seconds(p[38 + e]) * sr
            } else {
                0.
            },
            a: 1. / (seconds(p[41 + e]) * sr),
            d: (-6.9078 / (seconds(p[44 + e]) * sr)).exp(),
            s: p[47 + e] / 127.,
            r: (-6.9078 / (seconds(p[50 + e]) * sr)).exp(),
        }
    }
}
impl Envelope {
    fn on(&mut self, c: EnvCoeffs) {
        self.held = true;
        self.delay = c.delay;
        self.stage = 1;
    }
    fn off(&mut self) {
        self.held = false;
        if self.stage != 0 {
            self.stage = 4;
        }
    }
    fn tick(&mut self, c: EnvCoeffs, repeat: bool) -> f32 {
        if self.delay > 0. && self.stage == 1 {
            self.delay -= 1.;
            return self.v;
        }
        match self.stage {
            1 => {
                self.v = (self.v + c.a).min(1.);
                if self.v >= 1. {
                    self.stage = 2;
                }
            }
            2 => {
                self.v = c.s + (self.v - c.s) * c.d;
                if (self.v - c.s).abs() < 0.001 {
                    self.stage = if repeat { 4 } else { 3 };
                }
            }
            3 => self.v = c.s,
            4 => {
                self.v *= c.r;
                if self.v < 0.00001 {
                    self.v = 0.;
                    if repeat && self.held {
                        self.on(c);
                    } else {
                        self.stage = 0;
                    }
                }
            }
            _ => self.v = 0.,
        }
        self.v
    }
}
#[derive(Clone, Copy, Default)]
struct Filter {
    z: [f32; 4],
    g: f32,
}
impl Filter {
    fn tune(&mut self, hz: f32, sr: f32) {
        let t = (PI * hz.clamp(5., sr * 0.38) / sr).tan();
        self.g = t / (1. + t);
    }
    fn tick(&mut self, x: f32, res: f32, four: bool) -> f32 {
        let g = self.g;
        let poles = if four { 4 } else { 2 };
        let k = if four { 4.15 } else { 1.85 } * (res / 127.).clamp(0., 1.);
        let s = if four {
            (1. - g) * (g * g * g * self.z[0] + g * g * self.z[1] + g * self.z[2] + self.z[3])
        } else {
            (1. - g) * (g * self.z[0] + self.z[1])
        };
        let mut u = (x - k * s) / (1. + k * g.powi(poles as i32));
        for z in &mut self.z[..poles] {
            let v = (u - *z) * g;
            u = v + *z;
            *z = (u + v).clamp(-8., 8.);
            if four {
                *z = (*z * 0.2).tanh() * 5.;
            }
            if z.abs() < 1e-20 {
                *z = 0.;
            }
        }
        u.clamp(-8., 8.)
    }
}
/// All 52 numeric modulation destinations. Multiple paths accumulate; a
/// preceding control tick feeds amount-to-amount paths without recursion.
fn route(m: &mut [f32; 256], dest: usize, amount: f32) {
    let offsets: &[usize] = match dest {
        1 => &[0],
        2 => &[1],
        3 => &[0, 1],
        4 => &[14],
        5 => &[16],
        6 => &[15],
        7 => &[6],
        8 => &[7],
        9 => &[6, 7],
        10 => &[22],
        11 => &[23],
        12 => &[25],
        13 => &[27],
        14 => &[29],
        15 => &[53],
        16 => &[54],
        17 => &[55],
        18 => &[56],
        19 => &[53, 54, 55, 56],
        20 => &[61],
        21 => &[62],
        22 => &[63],
        23 => &[64],
        24 => &[61, 62, 63, 64],
        25 => &[32],
        26 => &[33],
        27 => &[34],
        28 => &[32, 33, 34],
        29 => &[41],
        30 => &[42],
        31 => &[43],
        32 => &[41, 42, 43],
        33 => &[44],
        34 => &[45],
        35 => &[46],
        36 => &[44, 45, 46],
        37 => &[50],
        38 => &[51],
        39 => &[52],
        40 => &[50, 51, 52],
        41 => &[85],
        42 => &[86],
        43 => &[87],
        44 => &[88],
        45 => &[89],
        46 => &[90],
        47 => &[91],
        48 => &[92],
        49 => &[21],
        50 => &[117],
        51 => &[118],
        52 => &[119],
        _ => &[],
    };
    let scale = if dest <= 3 { 24. } else { 127. };
    for &i in offsets {
        m[i] += amount * scale;
    }
}
#[derive(Clone, Copy)]
struct Voice {
    note: u8,
    vel: f32,
    held: bool,
    age: u64,
    phase: [f32; 2],
    sub: f32,
    pitch: [f32; 2],
    detune: f32,
    pan: f32,
    env: [Envelope; 3],
    ec: [EnvCoeffs; 3],
    filter: Filter,
    lfo_phase: [f32; 4],
    lfo: [f32; 4],
    lfo_hold: [f32; 4],
    mod_previous: [f32; 256],
    freq: [f32; 2],
    shape_mod: [f32; 2],
    params: [f32; 256],
    last: f32,
    rng: u32,
    tick: u32,
}
impl Voice {
    fn new(i: usize) -> Self {
        Self {
            note: 60,
            vel: 0.,
            held: false,
            age: 0,
            phase: [i as f32 * 0.137 % 1., i as f32 * 0.317 % 1.],
            sub: 0.,
            pitch: [60.; 2],
            detune: 0.,
            pan: 0.,
            env: [Envelope::default(); 3],
            ec: [EnvCoeffs::default(); 3],
            filter: Filter::default(),
            lfo_phase: [0.; 4],
            lfo: [0.; 4],
            lfo_hold: [0.; 4],
            mod_previous: [0.; 256],
            freq: [0.; 2],
            shape_mod: [50.; 2],
            params: [0.; 256],
            last: 0.,
            rng: 0x8913281u32.wrapping_add(i as u32 * 7919),
            tick: 0,
        }
    }
    fn on(&mut self, n: u8, vel: u8, age: u64, p: &[f32; 256], sr: f32, legato: bool) {
        let active = self.held;
        self.note = n;
        self.vel = vel as f32 / 127.;
        self.age = age;
        self.held = true;
        for e in 0..3 {
            self.ec[e] = EnvCoeffs::new(p, e, sr);
            if !legato || !active {
                self.env[e].on(self.ec[e]);
            }
        }
        for o in 0..2 {
            if p[19] < 0.5 || (p[18] as usize % 2 == 1 && !active) || p[8 + o] <= 0. {
                self.pitch[o] = n as f32;
            }
            if p[12 + o] > 0.5 {
                self.phase[o] = 0.;
            }
        }
        for l in 0..4 {
            if p[73 + l] > 0.5 {
                self.lfo_phase[l] = 0.;
            }
        }
        self.tick = 0;
    }
    fn off(&mut self) {
        self.held = false;
        for e in &mut self.env {
            e.off();
        }
    }
    fn control(&mut self, p: &[f32; 256], state: &State, seq: [f32; 4], sr: f32, bpm: f32) {
        let mut sources = [0.; 23];
        sources[1..5].copy_from_slice(&seq);
        sources[5..9].copy_from_slice(&self.lfo);
        for e in 0..3 {
            sources[9 + e] = self.env[e].v;
        }
        sources[12] = state.bend;
        sources[13] = state.wheel;
        sources[14] = state.pressure;
        sources[15] = state.breath;
        sources[16] = state.foot;
        sources[17] = state.expression;
        sources[18] = self.vel;
        sources[19] = self.note as f32 / 127.;
        sources[20] = random(&mut self.rng);
        sources[21] = 1.;
        sources[22] = self.last;
        let mut m = [0.; 256];
        for slot in 0..8 {
            let src = (p[77 + slot] as usize).min(22);
            route(
                &mut m,
                p[93 + slot] as usize,
                sources[src] * (p[85 + slot] + self.mod_previous[85 + slot] - 127.) / 127.,
            );
        }
        for (slot, src) in [13, 14, 15, 18, 16].iter().enumerate() {
            route(
                &mut m,
                p[102 + slot * 2] as usize,
                sources[*src] * (p[101 + slot * 2] - 127.) / 127.,
            );
        }
        for l in 0..4 {
            route(
                &mut m,
                p[65 + l] as usize,
                self.lfo[l] * (p[61 + l] + self.mod_previous[61 + l]) / 127.,
            );
        }
        route(
            &mut m,
            p[30] as usize,
            self.env[2].v * (p[34] - 127.) / 127. * (1. - p[37] / 127. * (1. - self.vel)),
        );
        for t in 0..4 {
            let dest = p[111 + t] as usize;
            if dest <= 3 && dest > 0 {
                route(&mut m, dest, seq[t] * 125. / 48.);
            } else {
                route(&mut m, dest, seq[t]);
            }
        }
        self.mod_previous = m;
        self.params = std::array::from_fn(|i| p[i] + m[i]);
        for l in 0..4 {
            let rate = (p[53 + l] + m[53 + l]).clamp(0., 150.);
            let hz = if p[69 + l] > 0.5 || rate > 127. {
                let periods = [
                    32.,
                    16.,
                    8.,
                    6.,
                    4.,
                    3.,
                    2.,
                    1.5,
                    1.,
                    2. / 3.,
                    0.5,
                    1. / 3.,
                    0.25,
                    1. / 6.,
                    0.125,
                    0.0625,
                ];
                let idx = if rate > 127. {
                    (rate - 128.) as usize
                } else {
                    (rate / 8.) as usize
                };
                bpm / 60. / (periods[idx.min(15)] * step_beats(p[131] as usize, 0))
            } else {
                0.01 * 30000.0f32.powf(rate / 127.)
            };
            let next = self.lfo_phase[l] + 16. * hz / sr;
            if next >= 1. {
                self.lfo_hold[l] = random(&mut self.rng);
            }
            self.lfo_phase[l] = next.fract();
            let t = self.lfo_phase[l];
            self.lfo[l] = match p[57 + l] as usize {
                0 => 1. - 4. * (t - 0.5).abs(),
                1 => 2. * t - 1.,
                2 => 1. - 2. * t,
                3 => {
                    if t < 0.5 {
                        1.
                    } else {
                        -1.
                    }
                }
                _ => self.lfo_hold[l],
            };
        }
        for e in 0..3 {
            self.ec[e] = EnvCoeffs::new(&self.params, e, sr);
        }
        let drift = random(&mut self.rng) * self.params[21].clamp(0., 127.) / 127. * 0.12;
        for o in 0..2 {
            let target = state.tuning[self.note as usize];
            let glide = seconds(p[8 + o]) * 0.2;
            if p[19] < 0.5 || p[8 + o] <= 0. {
                self.pitch[o] = target;
            } else if p[18] < 2. {
                let move_by = 16. * 12. / (glide * sr);
                self.pitch[o] += (target - self.pitch[o]).clamp(-move_by, move_by);
            } else {
                self.pitch[o] += (target - self.pitch[o]) * (1. - (-16. / (glide * sr)).exp());
            }
            let pitch = if p[10 + o] > 0.5 {
                self.pitch[o] - 24.
            } else {
                0.
            };
            self.freq[o] = frequency(
                pitch
                    + self.params[o]
                    + (p[2 + o] - 50.) / 100.
                    + state.bend * p[20]
                    + self.detune
                    + drift
                    + state.master_coarse
                    + state.master_fine / 100.,
            )
            .clamp(0.01, sr * 0.35);
            self.shape_mod[o] = self.params[6 + o].clamp(0., 99.);
        }
        let env = self.env[0].v * (self.params[32] - 127.) * (1. - p[35] / 127. * (1. - self.vel));
        let cut = self.params[22] + env + p[24] / 127. * (self.note as f32 - 60.);
        self.filter.tune(frequency(cut) * 0.5, sr);
    }
    fn sample(&mut self, p: &[f32; 256], state: &State, seq: [f32; 4], sr: f32, bpm: f32) -> f32 {
        if self.env[1].stage == 0 && p[27] <= 0. && !self.held {
            return 0.;
        }
        if self.tick % 16 == 0 {
            self.control(p, state, seq, sr, bpm);
        }
        self.tick = self.tick.wrapping_add(1);
        for e in 0..3 {
            self.env[e].tick(self.ec[e], e == 2 && p[31] > 0.5);
        }
        let d = [self.freq[0] / sr, self.freq[1] / sr];
        let a = wave(self.phase[0], d[0], p[4] as u8, self.shape_mod[0]);
        let b = wave(self.phase[1], d[1], p[5] as u8, self.shape_mod[1]);
        self.phase[0] = (self.phase[0] + d[0]).fract();
        let next = self.phase[1] + d[1];
        self.phase[1] = next.fract();
        if next >= 1. && p[17] > 0.5 {
            self.phase[0] = self.phase[1] / d[1] * d[0];
        }
        self.sub = (self.sub + d[0] * 0.5).fract();
        let sub = 1. - 4. * (self.sub - 0.5).abs();
        let mix = (self.params[14] / 127.).clamp(0., 1.);
        let noise = random(&mut self.rng);
        let signal = a * (1. - mix)
            + b * mix
            + sub * self.params[15] / 127.
            + noise * self.params[16] / 127.;
        if self.params[25] > 0. {
            let cut = self.params[22]
                + self.env[0].v * (self.params[32] - 127.)
                + p[24] / 127. * (self.note as f32 - 60.)
                + b * self.params[25] * 0.5;
            self.filter.tune(frequency(cut) * 0.5, sr);
        }
        let filtered = self
            .filter
            .tick(signal * 0.45 + 1e-6, self.params[23], p[26] > 0.5);
        let amp = (self.env[1].v * self.params[33] / 127. * (1. - p[36] / 127. * (1. - self.vel))
            + self.params[27] / 127.)
            .clamp(0., 2.);
        self.last = (filtered * amp).clamp(-4., 4.);
        self.last
    }
}
pub fn step_beats(div: usize, step: usize) -> f32 {
    let base = [
        2.,
        1.,
        0.5,
        0.5,
        0.5,
        1. / 3.,
        0.25,
        0.25,
        0.25,
        1. / 6.,
        0.125,
        1. / 12.,
        0.0625,
    ][div.min(12)];
    let swing = match div {
        3 | 7 => 1. / 6.,
        4 | 8 => 1. / 3.,
        _ => 0.,
    };
    base * (1. + if step % 2 == 0 { swing } else { -swing })
}
struct Effects {
    ring: Box<[[f32; 2]]>,
    pos: usize,
    phase: f32,
    ap: [[f32; 6]; 2],
    last: [f32; 2],
    tone: [f32; 2],
    hp: [Filter; 2],
    kind: u8,
}
impl Effects {
    fn new() -> Self {
        Self {
            ring: vec![[0.; 2]; 192002].into_boxed_slice(),
            pos: 0,
            phase: 0.,
            ap: [[0.; 6]; 2],
            last: [0.; 2],
            tone: [0.; 2],
            hp: [Filter::default(); 2],
            kind: 0,
        }
    }
    fn read(&self, delay: f32, c: usize) -> f32 {
        let d = delay.clamp(1., self.ring.len() as f32 - 2.);
        let w = d as usize;
        let f = d.fract();
        let i = (self.pos + self.ring.len() - w) % self.ring.len();
        let j = (i + self.ring.len() - 1) % self.ring.len();
        self.ring[i][c] * (1. - f) + self.ring[j][c] * f
    }
    fn process(
        &mut self,
        x: [f32; 2],
        p: &[f32; 256],
        sr: f32,
        bpm: f32,
        note: u8,
        modulation: [f32; 3],
    ) -> [f32; 2] {
        let kind = if p[116] > 0.5 { p[115] as u8 } else { 0 };
        let mix = ((p[117] + modulation[0]) / 127.).clamp(0., 1.);
        let a = ((p[118] + modulation[1]) / 255.).clamp(0., 1.);
        let b = ((p[119] + modulation[2]) / 127.).clamp(0., 1.);
        if kind != self.kind {
            self.ap = [[0.; 6]; 2];
            self.last = [0.; 2];
            self.kind = kind;
        }
        self.phase = (self.phase + (0.03 + 8. * a * a) / sr).fract();
        let lfo = (self.phase * TAU).sin();
        let mut wet = x;
        let mut write = [0.; 2];
        match kind {
            1..=3 => {
                let mut time = 0.001 + 0.999 * a;
                if p[120] > 0.5 {
                    let beats = [
                        4.,
                        3.,
                        2.,
                        1.5,
                        1.,
                        2. / 3.,
                        0.75,
                        0.5,
                        1. / 3.,
                        0.375,
                        0.25,
                    ];
                    time = beats[((p[118] + modulation[1]).clamp(0., 255.) as usize).min(10)] * 60.
                        / bpm;
                    while time > 1. {
                        time *= 0.5;
                    }
                }
                for c in 0..2 {
                    wet[c] = self.read(time * sr, c);
                    if kind == 3 {
                        self.tone[c] += 0.08 * (wet[c] - self.tone[c]);
                        wet[c] = self.tone[c].tanh();
                    }
                    write[c] = x[if kind == 1 { 0 } else { c }]
                        + wet[if kind == 2 { 1 - c } else { c }] * b * 0.98;
                }
            }
            4 | 8 | 9 => {
                for c in 0..2 {
                    let l = if c == 0 {
                        lfo
                    } else {
                        (self.phase * TAU + PI * 0.5).sin()
                    };
                    let delay = if kind == 4 {
                        sr * (0.018 + 0.01 * b * l)
                    } else {
                        sr * (0.003 + 0.0028 * b * l)
                    };
                    let delayed = self.read(delay, c);
                    let dry = if kind == 4 {
                        x[c]
                    } else {
                        self.read(sr * 0.003, c)
                    };
                    wet[c] = if kind == 4 {
                        (dry + delayed) * 0.5
                    } else {
                        dry - delayed
                    };
                    write[c] = x[c] + if kind == 8 { delayed * 0.75 } else { 0. };
                }
            }
            5..=7 => {
                let g = (0.15 + 0.75 * (lfo * 0.5 + 0.5) * b).clamp(0.05, 0.92);
                let feedback = if kind == 5 {
                    0.72
                } else if kind == 6 {
                    0.22
                } else {
                    0.45
                };
                for c in 0..2 {
                    let mut u = x[c] + self.last[c] * feedback;
                    for z in &mut self.ap[c] {
                        let y = *z - g * u;
                        *z = u + g * y;
                        u = y;
                    }
                    self.last[c] = u.clamp(-4., 4.);
                    wet[c] = (x[c] + u) * 0.5;
                }
            }
            10 => {
                let taps = [0.0297, 0.0371, 0.0411, 0.0437];
                for c in 0..2 {
                    let mut u = 0.;
                    for (i, t) in taps.iter().enumerate() {
                        u +=
                            self.read(sr * t * (0.4 + 2.5 * a) + (c * 17 + i * 7) as f32, c) * 0.25;
                    }
                    self.tone[c] += (0.01 + 0.8 * b) * (u - self.tone[c]);
                    wet[c] = self.tone[c];
                    write[c] = x[c] + self.tone[1 - c] * (0.35 + 0.61 * a);
                }
            }
            11 => {
                let hz = if p[119] > 0.5 {
                    frequency(note as f32) * 0.125 * 64.0f32.powf(a)
                } else {
                    0.1 * 20000.0f32.powf(a)
                };
                self.phase = (self.phase - (0.03 + 8. * a * a) / sr + hz / sr).rem_euclid(1.);
                for c in 0..2 {
                    wet[c] = x[c] * (self.phase * TAU).sin();
                }
            }
            12 => {
                for c in 0..2 {
                    let u = (x[c] * (1. + a * 60.)).tanh();
                    self.tone[c] += (0.01 + 0.99 * b) * (u - self.tone[c]);
                    wet[c] = self.tone[c];
                }
            }
            13 => {
                for c in 0..2 {
                    self.hp[c].tune(20. * 900.0f32.powf(a), sr);
                    wet[c] = x[c] - self.hp[c].tick(x[c], b * 127., false);
                }
            }
            _ => {}
        }
        self.ring[self.pos] = write.map(|v| if v.is_finite() { v.clamp(-4., 4.) } else { 0. });
        self.pos = (self.pos + 1) % self.ring.len();
        if kind == 0 {
            x
        } else {
            std::array::from_fn(|c| x[c] * (1. - mix) + wet[c] * mix)
        }
    }
}
struct Layer {
    voices: [Voice; 16],
    prior: [u8; 128],
    age: [u64; 128],
    serial: u64,
    step: usize,
    elapsed: f32,
    first: bool,
    generated: [u8; 128],
    ties: [bool; 128],
    poly_notes: [Option<u8>; 6],
    poly_vel: [u8; 6],
    gate: [usize; 4],
    seq: [f32; 4],
    seq_target: [f32; 4],
    gate_off: bool,
    duration: f32,
    count: usize,
    fx: Effects,
    order: [u8; 128],
    order_len: usize,
    arp_index: usize,
    last_run: bool,
}
impl Layer {
    fn new() -> Self {
        Self {
            voices: std::array::from_fn(Voice::new),
            prior: [0; 128],
            age: [0; 128],
            serial: 0,
            step: 0,
            elapsed: 0.,
            first: true,
            generated: [0; 128],
            ties: [false; 128],
            poly_notes: [None; 6],
            poly_vel: [0; 6],
            gate: [0; 4],
            seq: [0.; 4],
            seq_target: [0.; 4],
            gate_off: false,
            duration: 0.,
            count: 16,
            fx: Effects::new(),
            order: [0; 128],
            order_len: 0,
            arp_index: 0,
            last_run: false,
        }
    }
    fn keys(&mut self, notes: &[u8; 128], p: &[f32; 256], sr: f32) {
        for n in 0..128 {
            if notes[n] > 0 && self.prior[n] == 0 {
                self.serial = self.serial.wrapping_add(1);
                self.age[n] = self.serial;
            }
        }
        if p[123] > 0.5 {
            let key = match p[122] as usize % 3 {
                0 => (0..128).find(|&n| notes[n] > 0),
                1 => (0..128).rev().find(|&n| notes[n] > 0),
                _ => (0..128)
                    .filter(|&n| notes[n] > 0)
                    .max_by_key(|&n| self.age[n]),
            };
            let used = if p[124] >= 16. {
                self.count
            } else {
                (p[124] as usize + 1).min(self.count)
            };
            for i in 0..self.count {
                let v = &mut self.voices[i];
                if let Some(n) = key.filter(|_| i < used) {
                    let retrigger = p[122] >= 3. && self.prior[n] == 0;
                    if !v.held || v.note != n as u8 || retrigger {
                        v.detune = if used > 1 {
                            (i as f32 / (used - 1) as f32 * 2. - 1.) * p[208] / 16. * 0.5
                        } else {
                            0.
                        };
                        v.on(n as u8, notes[n], self.age[n], p, sr, p[122] < 3.);
                    }
                } else if v.held {
                    v.off();
                }
            }
        } else {
            for v in &mut self.voices {
                if v.held && notes[v.note as usize] == 0 {
                    v.off();
                }
            }
            for n in 0..128 {
                if notes[n] == 0 || self.prior[n] > 0 {
                    continue;
                }
                let i = (0..self.count)
                    .find(|&i| self.voices[i].env[1].stage == 0)
                    .or_else(|| {
                        (0..self.count)
                            .filter(|&i| !self.voices[i].held)
                            .min_by(|&a, &b| {
                                self.voices[a].env[1].v.total_cmp(&self.voices[b].env[1].v)
                            })
                    })
                    .unwrap_or_else(|| {
                        (0..self.count)
                            .min_by_key(|&i| self.voices[i].age)
                            .unwrap_or(0)
                    });
                let v = &mut self.voices[i];
                v.detune = 0.;
                v.on(n as u8, notes[n], self.age[n], p, sr, false);
            }
        }
        for (i, v) in self.voices.iter_mut().enumerate() {
            v.pan = if p[209] > 0.5 {
                (i as f32 / 15. * 2. - 1.) * p[29] / 127.
            } else {
                (if v.age % 2 == 0 { -1. } else { 1. }) * p[29] / 127.
            };
        }
        self.prior = *notes;
    }
    fn schedule(
        &mut self,
        held: &[u8; 128],
        p: &[f32; 256],
        state: &State,
        sr: f32,
        bpm: f32,
    ) -> [u8; 128] {
        let arp = p[136] > 0.5;
        let poly = p[139] > 0.5 && state.playing && !arp;
        let gated = p[139] < 0.5 && !arp;
        let down = held.iter().any(|v| *v > 0);
        let fresh =
            (0..128).any(|n| held[n] > 0 && !self.order[..self.order_len].contains(&(n as u8)));
        if fresh {
            if p[135] > 0.5 && arp {
                self.order_len = 0;
            }
            for n in 0..128 {
                if held[n] > 0
                    && !self.order[..self.order_len].contains(&(n as u8))
                    && self.order_len < 128
                {
                    self.order[self.order_len] = n as u8;
                    self.order_len += 1;
                }
            }
            if gated && (p[138] == 0. || p[138] == 2.) {
                self.gate = [0; 4];
                self.step = 0;
                self.elapsed = 0.;
                self.first = true;
            }
        }
        if !state.hold {
            let mut len = 0;
            for i in 0..self.order_len {
                let n = self.order[i];
                if held[n as usize] > 0 {
                    self.order[len] = n;
                    len += 1;
                }
            }
            self.order_len = len;
        }
        if poly && !self.last_run {
            self.step = 0;
            self.elapsed = 0.;
            self.first = true;
        }
        self.last_run = poly;
        let run = poly || ((arp || gated) && down);
        let duration = if self.duration > 0. {
            self.duration
        } else {
            step_beats(p[131] as usize, self.step) * 60. / bpm
        };
        let key_step = gated && p[138] == 4.;
        let trigger = run
            && (self.first
                || if key_step {
                    fresh
                } else {
                    self.elapsed >= duration
                });
        if trigger {
            self.first = false;
            self.elapsed = 0.;
            self.gate_off = false;
            self.duration = step_beats(p[131] as usize, self.step) * 60. / bpm;
            if arp {
                self.generated = [0; 128];
                let mut list = [0u8; 384];
                let mut velocities = [0u8; 384];
                let mut len = 0;
                let octaves = (p[133] as usize + 1).min(3);
                for oct in 0..octaves {
                    for j in 0..self.order_len {
                        let n = if p[132] == 4. {
                            self.order[j] as usize
                        } else {
                            let mut sorted = self.order;
                            sorted[..self.order_len].sort_unstable();
                            sorted[j] as usize
                        };
                        let velocity = held[n].max(1);
                        let n = n + oct * 12;
                        if n < 128 {
                            list[len] = n as u8;
                            velocities[len] = velocity;
                            len += 1;
                        }
                    }
                }
                if len > 0 {
                    let repeat = (p[134] as usize + 1).min(4);
                    let idx = self.arp_index / repeat;
                    let k = match p[132] as usize {
                        1 => len - 1 - idx % len,
                        2 => {
                            let period = (len * 2).saturating_sub(2).max(1);
                            let at = idx % period;
                            if at < len {
                                at
                            } else {
                                period - at
                            }
                        }
                        3 => {
                            (self
                                .serial
                                .wrapping_mul(1664525)
                                .wrapping_add(self.arp_index as u64 * 1013904223)
                                as usize)
                                % len
                        }
                        _ => idx % len,
                    };
                    let n = list[k];
                    self.generated[n as usize] = velocities[k];
                    self.arp_index = self.arp_index.wrapping_add(1);
                }
            } else if poly {
                self.generated = [0; 128];
                self.ties = [false; 128];
                let b = state.layer * 1024;
                let mut length = 64;
                for s in 0..64 {
                    if state.patch.data[b + 256 + s] == 0 && state.patch.data[b + 320 + s] == 0 {
                        length = s.max(1);
                        break;
                    }
                }
                self.step %= length;
                let mut continuing = [false; 128];
                for track in 0..6 {
                    let n = state.patch.data[b + 256 + track * 128 + self.step];
                    let vel = state.patch.data[b + 320 + track * 128 + self.step];
                    if n == 128 && vel >= 128 {
                        if let Some(note) = self.poly_notes[track] {
                            self.generated[note as usize] = self.poly_vel[track];
                            continuing[note as usize] = true;
                        }
                    } else if n < 128 && vel >= 128 {
                        let transpose = held
                            .iter()
                            .position(|v| *v > 0)
                            .map_or(0, |n| n as i32 - 60);
                        let note = (n as i32 + transpose).clamp(0, 127) as u8;
                        self.poly_notes[track] = Some(note);
                        self.poly_vel[track] = vel - 127;
                        self.generated[note as usize] = vel - 127;
                    } else {
                        self.poly_notes[track] = None;
                        self.poly_vel[track] = 0;
                    }
                    let next = (self.step + 1) % length;
                    if state.patch.data[b + 256 + track * 128 + next] == 128
                        && state.patch.data[b + 320 + track * 128 + next] >= 128
                    {
                        if let Some(note) = self.poly_notes[track] {
                            self.ties[note as usize] = true;
                        }
                    }
                }
                for n in 0..128 {
                    if self.generated[n] > 0 && !continuing[n] {
                        self.prior[n] = 0;
                    }
                }
                for v in &mut self.voices {
                    if v.held && !continuing[v.note as usize] {
                        v.off();
                    }
                }
            } else if gated {
                for t in 0..4 {
                    let offset = 140 + t * 16;
                    let step = self.gate[t];
                    let mut value = p[offset + step];
                    if value == 126. {
                        self.gate[t] = 0;
                        value = p[offset];
                    }
                    self.seq_target[t] = if value >= 126. { 0. } else { value / 125. };
                    self.gate[t] = (self.gate[t] + 1) % 16;
                }
                if p[138] < 2. && p[140 + self.gate[0].wrapping_add(15) % 16] != 127. {
                    for v in &mut self.voices {
                        if v.held {
                            for e in 0..3 {
                                v.env[e].on(v.ec[e]);
                            }
                        }
                    }
                }
            }
            self.step = self.step.wrapping_add(1);
        }
        if gated {
            for t in 0..4 {
                if t % 2 == 0 && p[112 + t] == 53. {
                    let seconds = 0.001 * 10000.0f32.powf(self.seq_target[t + 1]);
                    self.seq[t] +=
                        (self.seq_target[t] - self.seq[t]) * (1. - (-1. / (seconds * sr)).exp());
                } else {
                    self.seq[t] = self.seq_target[t];
                }
            }
            if p[138] < 2. && !self.gate_off && self.elapsed >= duration * 0.5 {
                for v in &mut self.voices {
                    if v.held {
                        for e in &mut v.env {
                            e.off();
                        }
                    }
                }
                self.gate_off = true;
            }
            let prior_step = self.gate[0].wrapping_add(15) % 16;
            if p[138] < 2. && p[140 + prior_step] == 127. {
                for v in &mut self.voices {
                    for e in &mut v.env {
                        e.off();
                    }
                }
            }
        }
        if run && !key_step {
            self.elapsed += 1. / sr;
        }
        if !run {
            self.generated = [0; 128];
            if !down {
                self.first = true;
                self.arp_index = 0;
            }
        }
        if arp || poly {
            if self.elapsed >= duration * 0.5 {
                let mut out = self.generated;
                for n in 0..128 {
                    if !self.ties[n] {
                        out[n] = 0;
                    }
                }
                out
            } else {
                self.generated
            }
        } else {
            *held
        }
    }
    fn render(&mut self, p: &[f32; 256], state: &State, sr: f32, bpm: f32) -> [f32; 2] {
        let mut out = [0.; 2];
        let mut fxmod = [0.; 3];
        let mut active = 0;
        for v in &mut self.voices[..self.count] {
            let u = v.sample(p, state, self.seq, sr, bpm);
            let pan = (v.pan
                + v.mod_previous[29] / 127.
                    * if p[209] < 0.5 {
                        if v.age % 2 == 0 {
                            -1.
                        } else {
                            1.
                        }
                    } else {
                        1.
                    })
            .clamp(-1., 1.);
            out[0] += u * ((1. - pan) * 0.5).sqrt();
            out[1] += u * ((1. + pan) * 0.5).sqrt();
            if v.env[1].stage > 0 {
                active += 1;
                for i in 0..3 {
                    fxmod[i] += v.mod_previous[117 + i];
                }
            }
        }
        if active > 0 {
            for m in &mut fxmod {
                *m /= active as f32;
            }
        }
        let gain = 0.3 * (p[28] / 127.).clamp(0., 1.);
        out = out.map(|v| v * gain);
        let low = (0..128).find(|&n| self.prior[n] > 0).unwrap_or(60) as u8;
        self.fx.process(out, p, sr, bpm, low, fxmod)
    }
}
pub struct Processor {
    shared: Arc<Shared>,
    state: State,
    layers: [Layer; 2],
    generation: u32,
    params: [[f32; 256]; 2],
    clock: Arc<crate::clock::Clock>,
    tail: f32,
    level: f32,
}
impl Processor {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            state: State::default(),
            layers: [Layer::new(), Layer::new()],
            generation: u32::MAX,
            params: [[0.; 256]; 2],
            clock: crate::clock::Clock::shared(),
            tail: 0.,
            level: 0.,
        }
    }
}
impl AudioProcessor for Processor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sr: f32) {
        if channels == 0 || !sr.is_finite() || !(8000.0..=192000.0).contains(&sr) {
            buffer.fill(0.);
            return;
        }
        if let Ok(s) = self.shared.state.try_lock() {
            self.state = *s;
        }
        if self.generation != self.state.generation {
            for layer in &mut self.layers {
                for v in &mut layer.voices {
                    v.off();
                }
                layer.prior = [0; 128];
                layer.first = true;
            }
            self.generation = self.state.generation;
        }
        for layer in 0..2 {
            let b = layer * 1024;
            for i in 0..256 {
                self.params[layer][i] = self.state.patch.data[b + i] as f32;
            }
            for (j, &i) in [22, 23, 6, 7, 14, 15, 16, 32].iter().enumerate() {
                self.params[layer][i] = (self.params[layer][i]
                    + self.shared.mods[layer * 8 + j].get() * 127.)
                    .clamp(0., 255.);
            }
        }
        let snap = self.clock.snap();
        let mut layer_states = [self.state; 2];
        for (l, s) in layer_states.iter_mut().enumerate() {
            s.layer = l;
            if self.state.follow_clock && !snap.running {
                s.playing = false;
            }
        }
        let mode = if self.state.multi {
            2
        } else {
            self.state.patch.data[231].min(2)
        };
        let voices = if self.state.voices == 8 { 8 } else { 16 };
        let count = if mode > 0 { voices / 2 } else { voices };
        for l in &mut self.layers {
            l.count = count;
        }
        let target = (self.shared.level.get() + self.shared.external.get()).clamp(0., 2.);
        let slew = 1. - (-1. / (sr * 0.01)).exp();
        let decay = (-1. / (sr * 0.8)).exp();
        let mut bus = self.shared.output.try_lock().ok();
        if let Some(b) = bus.as_mut() {
            let len = (buffer.len() / channels).min(b.capacity());
            b.resize(len, 0.);
        }
        let mut peak = 0.0f32;
        for (frame_i, frame) in buffer.chunks_mut(channels).enumerate() {
            let mut sum = [0.; 2];
            for layer in 0..if mode == 0 { 1 } else { 2 } {
                let mut held = self.state.notes;
                for n in 0..128 {
                    held[n] = held[n]
                        .max(self.state.midi_notes[if self.state.multi { layer } else { 0 }][n]);
                }
                if mode == 1 {
                    let split = self.state.patch.data[232];
                    for n in 0..128 {
                        if (layer == 0 && n >= split as usize) || (layer == 1 && n < split as usize)
                        {
                            held[n] = 0;
                        }
                    }
                }
                let p = &self.params[layer];
                let bpm = if self.state.follow_clock {
                    snap.bpm
                } else {
                    p[130].clamp(30., 250.)
                };
                let state = &layer_states[layer];
                let notes = self.layers[layer].schedule(&held, p, state, sr, bpm);
                if notes != self.layers[layer].prior {
                    self.layers[layer].keys(&notes, p, sr);
                }
                let x = self.layers[layer].render(p, state, sr, bpm);
                for c in 0..2 {
                    sum[c] += x[c];
                }
            }
            self.level += (target - self.level) * slew;
            sum = sum.map(|v| {
                if v.is_finite() {
                    // Restore useful instrument output after the conservative
                    // voice/filter and effect input gains. Keeping makeup here
                    // leaves effect drive and imported patch balances intact;
                    // the soft ceiling handles dense chords and stacked layers.
                    (v * 4. * self.level).tanh()
                } else {
                    0.
                }
            });
            if self.state.mono {
                sum = [(sum[0] + sum[1]) * 0.5; 2];
            }
            peak = peak.max(sum[0].abs()).max(sum[1].abs());
            self.tail = (self.tail * decay).max(sum[0].abs()).max(sum[1].abs());
            for (c, v) in frame.iter_mut().enumerate() {
                *v = if channels == 1 {
                    (sum[0] + sum[1]) * 0.5
                } else {
                    sum[c % 2]
                };
            }
            if let Some(b) = bus.as_mut() {
                if frame_i < b.len() {
                    b[frame_i] = (sum[0] + sum[1]) * 0.5;
                }
            }
        }
        self.shared.peak.set(peak);
        self.shared
            .tail
            .store(self.tail > 0.00002 || self.state.playing, Ordering::Relaxed);
    }
}
#[cfg(test)]
mod behavior {
    use super::*;
    fn output(patch: super::super::patch::Patch, notes: &[usize], level: f32) -> Vec<f32> {
        use super::super::AtomicF32;
        use std::sync::{atomic::AtomicBool, Mutex};
        let mut state = State::default();
        state.patch = patch;
        for &note in notes {
            state.notes[note] = 100;
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(state),
            mods: (0..16).map(|_| Arc::new(AtomicF32::new(0.))).collect(),
            output: Arc::new(Mutex::new(Vec::with_capacity(256))),
            level: Arc::new(AtomicF32::new(level)),
            external: Arc::new(AtomicF32::new(0.)),
            peak: AtomicF32::new(0.),
            tail: AtomicBool::new(false),
        });
        let mut processor = Processor::new(shared);
        let mut audio = vec![0.; 48_000];
        processor.process(&mut audio, 2, 48_000.);
        audio[10_000..].to_vec()
    }
    fn rms(audio: &[f32]) -> f32 {
        (audio.iter().map(|v| v * v).sum::<f32>() / audio.len() as f32).sqrt()
    }
    #[test]
    fn init_note_has_useful_output_and_preserves_volume_controls() {
        use super::super::patch::Patch;
        let patch = Patch::default();
        let loud = output(patch, &[60], 1.);
        assert!(rms(&loud) > 0.10 && rms(&loud) < 0.20);
        let mut quiet = patch;
        quiet.data[28] /= 2;
        let quiet = output(quiet, &[60], 1.);
        assert!(rms(&quiet) < rms(&loud) * 0.60);
        assert!(rms(&output(patch, &[60], 0.5)) < rms(&loud) * 0.60);
        assert_eq!(rms(&output(patch, &[60], 0.)), 0.);
    }
    #[test]
    fn dense_and_stacked_chords_remain_finite_without_hard_clipping() {
        use super::super::patch::Patch;
        let notes: Vec<_> = (48..64).collect();
        for mode in [0, 2] {
            let mut patch = Patch::default();
            patch.data[231] = mode;
            let audio = output(patch, &notes, 1.);
            assert!(audio.iter().all(|v| v.is_finite() && v.abs() < 1.));
            assert!(rms(&audio) > 0.25);
            // Ordinary dense chords should not live on the soft ceiling.
            let hot = audio.iter().filter(|v| v.abs() > 0.95).count();
            assert!(hot as f32 / (audio.len() as f32) < 0.02);
        }
    }
    #[test]
    fn poly_ties_keep_only_their_track_and_do_not_release_between_steps() {
        let mut state = State::default();
        state.playing = true;
        state.patch.data[131] = 6;
        for (i, v) in [
            (256, 60),
            (257, 128),
            (258, 0),
            (320, 227),
            (321, 227),
            (322, 0),
            (384, 64),
            (385, 67),
            (448, 227),
            (449, 227),
        ] {
            state.patch.data[i] = v;
        }
        let p = std::array::from_fn(|i| state.patch.data[i] as f32);
        let mut l = Layer::new();
        let held = [0; 128];
        let notes = l.schedule(&held, &p, &state, 48000., 120.);
        l.keys(&notes, &p, 48000.);
        let age = l
            .voices
            .iter()
            .find(|v| v.note == 60 && v.held)
            .unwrap()
            .age;
        for _ in 0..6100 {
            let notes = l.schedule(&held, &p, &state, 48000., 120.);
            if notes != l.prior {
                l.keys(&notes, &p, 48000.);
            }
        }
        assert_eq!(
            l.voices
                .iter()
                .find(|v| v.note == 60 && v.held)
                .unwrap()
                .age,
            age
        );
        assert!(l.prior[67] > 0);
        assert_eq!(l.prior[64], 0);
    }
    #[test]
    fn all_numeric_mod_destinations_reach_a_real_parameter() {
        for dest in 1..=52 {
            let mut m = [0.; 256];
            route(&mut m, dest, 0.5);
            assert!(m.iter().any(|v| *v != 0.), "{dest}");
        }
    }
    #[test]
    fn gated_slew_smooths_track_one_and_half_step_closes_the_gate() {
        let mut state = State::default();
        state.patch.data[139] = 0;
        state.patch.data[131] = 6;
        state.patch.data[111] = 10;
        state.patch.data[112] = 53;
        state.patch.data[140] = 125;
        state.patch.data[156] = 125;
        let p = std::array::from_fn(|i| state.patch.data[i] as f32);
        let mut l = Layer::new();
        let mut held = [0; 128];
        held[60] = 100;
        let n = l.schedule(&held, &p, &state, 48000., 120.);
        l.keys(&n, &p, 48000.);
        assert!(l.seq[0] > 0. && l.seq[0] < 0.01);
        for _ in 0..4000 {
            l.schedule(&held, &p, &state, 48000., 120.);
        }
        assert!(l.gate_off);
        assert_eq!(l.voices[0].env[1].stage, 4);
        assert!(l.seq[0] < 1.);
    }
    #[test]
    fn sixteen_voice_limit_and_unison_note_priority_work() {
        let mut p = [0.; 256];
        p[123] = 1.;
        p[124] = 15.;
        p[122] = 0.;
        let mut l = Layer::new();
        let mut held = [0; 128];
        held[60] = 100;
        held[72] = 100;
        l.keys(&held, &p, 48000.);
        assert!(l.voices.iter().all(|v| v.held && v.note == 60));
        p[122] = 1.;
        l.keys(&held, &p, 48000.);
        assert!(l.voices.iter().all(|v| v.held && v.note == 72));
    }
}
