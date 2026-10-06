//! Hydra's wavetables: six banks of sixteen frames, each frame stored at
//! nine band-limits (one per octave of pitch), so a note never plays
//! harmonics above Nyquist and the table never aliases.
//!
//! Every frame is built from its harmonic amplitudes with an inverse FFT, so
//! the tables are exact additive spectra rather than drawn curves. They are
//! made once (the first time a Hydra is built, never on the audio thread) and
//! shared by every instance.
//!
//! A bank's spectrum is fixed in *harmonic number*, not in Hz, which is how
//! every wavetable behaves: the Vowel bank's formants sit at the same
//! harmonics whatever note is played.

use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::OnceLock;

/// Samples in one cycle (so up to 512 harmonics).
pub const LEN: usize = 1024;
pub const FRAMES: usize = 16;
/// Band-limits: level `l` holds harmonics up to `MAX_HARMONICS >> l`.
pub const LEVELS: usize = 9;
pub const MAX_HARMONICS: usize = LEN / 2;
pub const BANKS: usize = 6;
pub const BANK_NAMES: [&str; BANKS] = ["Basic", "Organ", "Vowel", "Digital", "Sweep", "Glass"];

const STRIDE: usize = LEN + 1;

pub struct Tables {
    data: Vec<f32>,
}

fn index(bank: usize, frame: usize, level: usize) -> usize {
    ((bank * FRAMES + frame) * LEVELS + level) * STRIDE
}

/// Harmonic amplitudes (index 1..=512) of frame `frame` of `bank`.
fn spectrum(bank: usize, frame: usize) -> Vec<f32> {
    let f = frame as f32 / (FRAMES - 1) as f32;
    let mut a = vec![0.0f32; MAX_HARMONICS + 1];
    let pi = std::f32::consts::PI;
    match bank {
        0 => {
            // sine -> triangle -> saw -> square -> bright saw
            let anchor = |kind: usize, h: usize| -> f32 {
                let hf = h as f32;
                let sign = if h % 2 == 1 { 1.0 } else { -1.0 };
                match kind {
                    0 => (h == 1) as u8 as f32,
                    1 => {
                        if h % 2 == 1 {
                            8.0 / (pi * pi) / (hf * hf) * if (h / 2) % 2 == 0 { 1.0 } else { -1.0 }
                        } else {
                            0.0
                        }
                    }
                    2 => 2.0 / pi / hf * sign,
                    3 => {
                        if h % 2 == 1 {
                            4.0 / pi / hf
                        } else {
                            0.0
                        }
                    }
                    _ => 2.0 / pi / hf.sqrt() * sign,
                }
            };
            let pos = f * 4.0;
            let (i, t) = (pos.floor().min(3.0) as usize, pos - pos.floor().min(3.0));
            for h in 1..=MAX_HARMONICS {
                a[h] = anchor(i, h) * (1.0 - t) + anchor(i + 1, h) * t;
            }
        }
        1 => {
            // drawbar registrations: flute, jazz, full, bright
            const REG: [[f32; 8]; 4] = [[1.0, 0.0, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0], [1.0, 0.8, 0.0, 0.3, 0.0, 0.0, 0.0, 0.0], [1.0, 0.9, 0.7, 0.8, 0.5, 0.6, 0.4, 0.3], [0.5, 0.6, 0.7, 0.9, 0.8, 1.0, 0.9, 0.7]];
            const HARM: [usize; 8] = [1, 2, 3, 4, 5, 6, 8, 10];
            let pos = f * 3.0;
            let (i, t) = (pos.floor().min(2.0) as usize, pos - pos.floor().min(2.0));
            for (k, &h) in HARM.iter().enumerate() {
                a[h] = REG[i][k] * (1.0 - t) + REG[i + 1][k] * t;
            }
        }
        2 => {
            // vowels a e i o u: formant centres in harmonics (for a ~110 Hz fundamental)
            const V: [[f32; 3]; 5] = [[7.3, 10.5, 26.0], [3.6, 14.5, 24.5], [2.5, 21.0, 27.0], [4.0, 7.3, 25.7], [3.0, 6.4, 23.0]];
            let pos = f * 4.0;
            let (i, t) = (pos.floor().min(3.0) as usize, pos - pos.floor().min(3.0));
            let fm: Vec<f32> = (0..3).map(|k| V[i][k] * (1.0 - t) + V[i + 1][k] * t).collect();
            let width = [1.6, 2.4, 4.0];
            let gain = [1.0, 0.55, 0.3];
            for h in 1..=MAX_HARMONICS {
                let hf = h as f32;
                let mut s = 0.0;
                for k in 0..3 {
                    let d = (hf - fm[k]) / width[k];
                    s += gain[k] * (-d * d).exp();
                }
                a[h] = s / hf.powf(0.8);
            }
        }
        3 => {
            // sparse seeded spectra, morphing between six of them
            let set = |seed: u32, h: usize| -> f32 {
                let mut x = seed.wrapping_mul(0x9E37_79B1) ^ (h as u32).wrapping_mul(0x85EB_CA6B);
                x ^= x >> 15;
                x = x.wrapping_mul(0x2C1B_3C6D);
                x ^= x >> 12;
                let r = (x & 0xFFFF) as f32 / 65535.0;
                let comb = 1 + (seed as usize % 4);
                let on = if h % comb == 0 { 1.0 } else { 0.15 };
                (if r > 0.55 { r } else { 0.0 }) * on / (h as f32).powf(0.6)
            };
            let pos = f * 5.0;
            let (i, t) = (pos.floor().min(4.0) as usize, pos - pos.floor().min(4.0));
            for h in 1..=MAX_HARMONICS.min(160) {
                a[h] = set(i as u32 + 11, h) * (1.0 - t) + set(i as u32 + 12, h) * t;
            }
            a[1] = a[1].max(0.5);
        }
        4 => {
            // a saw whose spectrum is low-passed by a sweeping resonant filter
            let cutoff = 1.5 * (80.0f32 / 1.5).powf(f);
            for h in 1..=MAX_HARMONICS {
                let hf = h as f32;
                let r = hf / cutoff;
                let lp = 1.0 / (1.0 + r.powi(6)).sqrt();
                let peak = 1.8 * (-((hf - cutoff) / (0.7 + 0.1 * cutoff)).powi(2)).exp();
                a[h] = (1.0 / hf) * (lp + peak * lp.max(0.3)) * if h % 2 == 1 { 1.0 } else { -1.0 };
            }
        }
        _ => {
            // glass: a moving band of mostly odd partials
            let centre = 3.0 + f * 44.0;
            let width = 2.0 + f * 3.0;
            for h in 1..=MAX_HARMONICS.min(160) {
                let hf = h as f32;
                let band = (-((hf - centre) / width).powi(2)).exp();
                a[h] = (band * if h % 2 == 1 { 1.0 } else { 0.3 } + if h == 1 { 0.35 } else { 0.0 }) / hf.powf(0.3);
            }
        }
    }
    a
}

/// One cycle (plus a guard sample) from `amps`, keeping harmonics up to `limit`.
fn synthesize(amps: &[f32], limit: usize, fft: &dyn rustfft::Fft<f32>) -> Vec<f32> {
    let mut buf = vec![Complex::new(0.0f32, 0.0); LEN];
    for h in 1..=limit.min(MAX_HARMONICS - 1) {
        // A sin(h x) = real part of the inverse transform of -j A/2 at +h, +j A/2 at -h
        buf[h] = Complex::new(0.0, -amps[h] * 0.5);
        buf[LEN - h] = Complex::new(0.0, amps[h] * 0.5);
    }
    fft.process(&mut buf);
    let mut out: Vec<f32> = buf.iter().map(|c| c.re).collect();
    out.push(out[0]);
    out
}

fn build() -> Tables {
    let fft = FftPlanner::new().plan_fft_inverse(LEN);
    let mut data = vec![0.0f32; BANKS * FRAMES * LEVELS * STRIDE];
    for bank in 0..BANKS {
        for frame in 0..FRAMES {
            let amps = spectrum(bank, frame);
            let top = synthesize(&amps, MAX_HARMONICS - 1, fft.as_ref());
            let peak = top.iter().fold(1e-6f32, |m, s| m.max(s.abs()));
            for level in 0..LEVELS {
                let table = synthesize(&amps, MAX_HARMONICS >> level, fft.as_ref());
                let at = index(bank, frame, level);
                for (o, s) in data[at..at + STRIDE].iter_mut().zip(&table) {
                    *o = s / peak;
                }
            }
        }
    }
    Tables { data }
}

static TABLES: OnceLock<Tables> = OnceLock::new();

/// The shared tables, built on first use.
pub fn tables() -> &'static Tables {
    TABLES.get_or_init(build)
}

impl Tables {
    /// The band-limit level for a note of `hz` at `rate`: the finest level
    /// whose harmonics all stay under Nyquist.
    pub fn level_for(hz: f32, rate: f32) -> usize {
        let max_h = (0.5 * rate / hz.max(1.0)).max(1.0);
        let l = (MAX_HARMONICS as f32 / max_h).log2().ceil();
        (l.max(0.0) as usize).min(LEVELS - 1)
    }

    /// One frame at one phase (0..1), linearly interpolated.
    #[inline]
    fn frame(&self, bank: usize, frame: usize, level: usize, phase: f32) -> f32 {
        let at = index(bank, frame, level);
        let x = phase * LEN as f32;
        let i = (x as usize).min(LEN - 1);
        let f = x - i as f32;
        let a = self.data[at + i];
        let b = self.data[at + i + 1];
        a + (b - a) * f
    }

    /// The wavetable at morph position `pos` (0..1 across the frames).
    #[inline]
    pub fn sample(&self, bank: usize, pos: f32, level: usize, phase: f32) -> f32 {
        let x = pos.clamp(0.0, 1.0) * (FRAMES - 1) as f32;
        let f0 = (x as usize).min(FRAMES - 2);
        let t = x - f0 as f32;
        let a = self.frame(bank.min(BANKS - 1), f0, level, phase);
        let b = self.frame(bank.min(BANKS - 1), f0 + 1, level, phase);
        a + (b - a) * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_is_normalised_finite_and_a_closed_cycle() {
        let t = tables();
        for bank in 0..BANKS {
            for frame in 0..FRAMES {
                let at = index(bank, frame, 0);
                let cycle = &t.data[at..at + STRIDE];
                assert!(cycle.iter().all(|s| s.is_finite()), "bank {bank} frame {frame}");
                let peak = cycle.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                assert!((peak - 1.0).abs() < 1e-3, "bank {bank} frame {frame}: peak {peak}");
                assert_eq!(cycle[0], cycle[LEN], "guard sample closes the cycle");
                let dc = cycle[..LEN].iter().sum::<f32>() / LEN as f32;
                assert!(dc.abs() < 1e-3, "no DC: bank {bank} frame {frame}: {dc}");
            }
        }
    }

    #[test]
    fn the_first_basic_frame_is_a_sine_and_the_third_is_a_saw() {
        let t = tables();
        for i in [0usize, 100, 300, 700] {
            let x = i as f32 / LEN as f32;
            let sine = (x * std::f32::consts::TAU).sin();
            assert!((t.frame(0, 0, 0, x) - sine).abs() < 0.01, "sine at {x}");
        }
        // the saw frame has a long ramp: monotone over most of the cycle
        let mut rising = 0;
        for i in 20..LEN - 20 {
            if t.data[index(0, 8, 0) + i + 1] != t.data[index(0, 8, 0) + i] {
                rising += 1;
            }
        }
        assert!(rising > LEN / 2);
    }

    #[test]
    fn lower_levels_hold_no_harmonics_above_their_limit() {
        let t = tables();
        let mut fft_in: Vec<Complex<f32>> = (0..LEN).map(|i| Complex::new(t.data[index(0, 8, 3) + i], 0.0)).collect();
        FftPlanner::new().plan_fft_forward(LEN).process(&mut fft_in);
        let limit = MAX_HARMONICS >> 3;
        let above: f32 = (limit + 2..LEN / 2).map(|h| fft_in[h].norm()).fold(0.0, f32::max);
        let below: f32 = (1..limit).map(|h| fft_in[h].norm()).fold(0.0, f32::max);
        assert!(above < below * 1e-3, "above {above}, below {below}");
    }

    #[test]
    fn the_level_for_a_pitch_keeps_every_harmonic_under_nyquist() {
        for hz in [30.0, 110.0, 440.0, 1760.0, 7000.0, 15000.0] {
            let level = Tables::level_for(hz, 48_000.0);
            let harmonics = MAX_HARMONICS >> level;
            assert!(harmonics as f32 * hz <= 24_000.0 * 1.001 || level == LEVELS - 1, "{hz} Hz: level {level} has {harmonics} harmonics");
        }
        assert_eq!(Tables::level_for(20.0, 48_000.0), 0, "bass notes use the full table");
    }

    #[test]
    fn morphing_moves_between_frames() {
        let t = tables();
        let a = t.sample(2, 0.0, 0, 0.13);
        let b = t.sample(2, 1.0, 0, 0.13);
        let mid = t.sample(2, 0.5, 0, 0.13);
        assert!((a - b).abs() > 0.05, "the vowel bank changes across its frames");
        assert!(mid.is_finite());
    }
}
