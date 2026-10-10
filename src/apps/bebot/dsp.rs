//! Independent band-limited oscillators, TPT SVF and stereo modulation effects.
use super::{Control, Patch};
use crate::audio::AudioProcessor;
use crate::util::AtomicF32;
use std::f32::consts::{PI, TAU};
use std::sync::{Arc, Mutex};
const SCALES: [&[i32]; 5] = [
    &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    &[0, 2, 4, 5, 7, 9, 11],
    &[0, 2, 3, 5, 7, 8, 10],
    &[0, 2, 4, 7, 9],
    &[0, 3, 5, 6, 7, 10],
];
pub fn snap(note: f32, p: Patch) -> f32 {
    let root = p.v[3] as i32;
    let scale = SCALES[p.v[1] as usize];
    let center = note.round() as i32;
    (-12..=12)
        .map(|d| center + d)
        .filter(|n| scale.contains(&(n - root).rem_euclid(12)))
        .min_by(|a, b| {
            (*a as f32 - note)
                .abs()
                .total_cmp(&(*b as f32 - note).abs())
        })
        .unwrap_or(center) as f32
}
pub fn degree_note(p: Patch, d: i32) -> f32 {
    let scale = SCALES[p.v[1] as usize];
    let n = scale.len() as i32;
    let base = p.v[4] as i32;
    let root = base - base.rem_euclid(12) + p.v[3] as i32;
    (root + 12 * d.div_euclid(n) + scale[d.rem_euclid(n) as usize]) as f32
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
#[derive(Clone, Copy)]
struct Voice {
    phase: f32,
    amp: f32,
    note: f32,
    correction: f32,
    y: f32,
    ic1: f32,
    ic2: f32,
}
impl Default for Voice {
    fn default() -> Self {
        Self {
            phase: 0.,
            amp: 0.,
            note: 60.,
            correction: 0.,
            y: 0.7,
            ic1: 0.,
            ic2: 0.,
        }
    }
}
struct Coefficients {
    note: f32,
    y: f32,
    tune: f32,
    amp: [f32; 8],
    correction: f32,
    snapped: f32,
}
impl Voice {
    fn sample(&mut self, c: &Control, i: usize, sr: f32, cf: &Coefficients) -> f32 {
        let p = c.patch.v;
        let held = c.gates[i] > 0.;
        let target = c.notes[i] + c.bend;
        if self.amp < 0.00001 && held {
            self.note = target;
            self.correction = 0.;
        }
        self.note += (target - self.note) * cf.note;
        self.correction += (if i == 0 { cf.correction } else { 0. } - self.correction) * cf.tune;
        let note = if i == 0 && p[2] == 1. {
            cf.snapped
        } else {
            self.note + self.correction
        };
        let dt = (440. * 2f32.powf((note - 69.) / 12.) / sr).clamp(0.00001, 0.45);
        self.y += (c.y[i] - self.y) * cf.y;
        self.amp += ((if held { c.gates[i] } else { 0. }) - self.amp) * cf.amp[i];
        let mode = p[0] as usize;
        let width = if mode == 3 {
            0.05 + self.y * 0.45
        } else {
            0.05 + p[7] * 0.45
        };
        let mut x = match mode {
            2 => (self.phase * TAU).sin(),
            1 | 3 => {
                let q = (self.phase - width).rem_euclid(1.);
                (if self.phase < width { 1. } else { -1. }) + blep(self.phase, dt) - blep(q, dt)
            }
            _ => 2. * self.phase - 1. - blep(self.phase, dt),
        };
        if mode == 1 || mode == 3 {
            // A unipolar duty change otherwise sends DC into the resonant filter.
            x -= 2. * width - 1.;
        }
        self.phase = (self.phase + dt).fract();
        if mode != 2 {
            let cutoff = if mode == 3 {
                p[8]
            } else {
                100. * 120f32.powf(self.y)
            };
            let g = (PI * cutoff.clamp(30., sr * 0.42) / sr).tan();
            let k = 2. - p[6] * 1.85;
            let a1 = 1. / (1. + g * (g + k));
            let a2 = g * a1;
            let a3 = g * a2;
            let v3 = x - self.ic2;
            let v1 = a1 * self.ic1 + a2 * v3;
            let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
            self.ic1 = 2. * v1 - self.ic1;
            self.ic2 = 2. * v2 - self.ic2;
            x = v2;
        } else {
            x *= self.y;
        }
        if p[12] < 0.5 {
            x = drive(x, p[11]);
        }
        x * self.amp
    }
}
fn drive(x: f32, amount: f32) -> f32 {
    if amount <= 0. {
        x
    } else {
        let g = 1. + amount * 12.;
        (x * g).tanh() / g.sqrt()
    }
}
pub struct Processor {
    shared: Arc<Mutex<Control>>,
    state: Control,
    voices: [Voice; 8],
    level: Arc<AtomicF32>,
    ext: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    peak: Arc<AtomicF32>,
    chorus: Vec<f32>,
    echo: [Vec<f32>; 2],
    pos: usize,
    lfo: f32,
    gain: f32,
}
impl Processor {
    pub fn new(
        shared: Arc<Mutex<Control>>,
        level: Arc<AtomicF32>,
        ext: Arc<AtomicF32>,
        output: Arc<Mutex<Vec<f32>>>,
        peak: Arc<AtomicF32>,
    ) -> Self {
        // Allocate for a 2 s delay up to 192 kHz, never in process().
        {
            let mut b = output.lock().unwrap();
            b.resize(16384, 0.);
        }
        Self {
            shared,
            state: Control::default(),
            voices: [Voice::default(); 8],
            level,
            ext,
            output,
            peak,
            chorus: vec![0.; 16384],
            echo: [vec![0.; 384004], vec![0.; 384004]],
            pos: 0,
            lfo: 0.,
            gain: 0.,
        }
    }
    fn read(line: &[f32], pos: usize, delay: f32) -> f32 {
        let t = (pos % line.len()) as f32 - delay;
        let a = t.floor();
        let f = t - a;
        let i = (a as i64).rem_euclid(line.len() as i64) as usize;
        line[i] * (1. - f) + line[(i + 1) % line.len()] * f
    }
}
impl AudioProcessor for Processor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sr: f32) {
        if channels == 0 || !sr.is_finite() || sr < 8000. {
            buffer.fill(0.);
            return;
        }
        if let Ok(c) = self.shared.try_lock() {
            self.state = *c;
        }
        let p = self.state.patch.v;
        let coeff = |time: f32| 1. - (-1. / (sr * time)).exp();
        let target = self.state.notes[0] + self.state.bend;
        let snapped = snap(target, self.state.patch);
        let cf = Coefficients {
            note: coeff(0.003),
            y: coeff(0.008),
            tune: coeff(match p[2] as usize {
                2 => 0.22,
                3 => 0.045,
                _ => 0.001,
            }),
            amp: std::array::from_fn(|i| {
                coeff(if self.state.gates[i] > 0. {
                    p[9]
                } else {
                    p[10]
                })
            }),
            correction: if p[2] > 0. { snapped - target } else { 0. },
            snapped,
        };
        let target_gain = (self.level.get() + self.ext.get()).clamp(0., 2.);
        let gain_coef = coeff(0.01);
        let mut published = self.output.try_lock().ok();
        if let Some(b) = published.as_mut() {
            let len = (buffer.len() / channels).min(b.capacity());
            b.resize(len, 0.);
            b.fill(0.);
        }
        let mut peak = 0f32;
        for (frame_i, frame) in buffer.chunks_mut(channels).enumerate() {
            self.gain += (target_gain - self.gain) * gain_coef;
            let mut dry = 0.;
            for i in 0..8 {
                dry += self.voices[i].sample(&self.state, i, sr, &cf);
            }
            dry *= 0.24;
            if p[12] >= 0.5 {
                dry = drive(dry, p[11]);
            }
            let ci = self.pos % self.chorus.len();
            self.chorus[ci] = dry;
            let mut stereo = [0.; 2];
            for (ch, out) in stereo.iter_mut().enumerate() {
                let phase = self.lfo + ch as f32 * 0.25;
                let delay = sr * (0.014 + 0.003 * (phase * TAU).sin());
                let wet = Self::read(&self.chorus, self.pos, delay);
                let chor = dry * (1. - p[13] * 0.4) + wet * p[13] * 0.6;
                let et = (p[15] * sr).min(self.echo[ch].len() as f32 - 3.);
                let echo = Self::read(&self.echo[ch], self.pos, et);
                let idx = self.pos % self.echo[ch].len();
                self.echo[ch][idx] = (chor + echo * self.state.patch.feedback).tanh();
                *out = ((chor + echo * p[14]) * self.gain).tanh();
            }
            peak = peak.max(stereo[0].abs()).max(stereo[1].abs());
            for (ch, out) in frame.iter_mut().enumerate() {
                *out = if channels == 1 {
                    (stereo[0] + stereo[1]) * 0.5
                } else {
                    stereo[ch % 2]
                };
            }
            if let Some(b) = published.as_mut() {
                if frame_i < b.len() {
                    b[frame_i] = (stereo[0] + stereo[1]) * 0.5;
                }
            }
            self.pos = self.pos.wrapping_add(1);
            self.lfo = (self.lfo + 0.27 / sr).fract();
        }
        self.peak.set(peak);
    }
}
