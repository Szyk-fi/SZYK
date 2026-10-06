//! Pitch-preserving time-stretch for Sample Drum's Tempo Match: WSOLA, a
//! waveform-similarity overlap-add.
//!
//! The sample is cut into short overlapping Hann-windowed grains. Grains are
//! spawned at a steady rate in the *output* but start at positions that move through
//! the *source* at the stretch ratio, so the sound is slower or faster while each
//! grain is still read at its original pitch (or at the voice's own pitch offset).
//! The "similarity" part keeps it from sounding like a flutter: each new grain's
//! start is nudged within a small window to the spot whose waveform best lines up
//! with where the previous grain is already heading, so the overlaps add up in phase
//! instead of cancelling.
//!
//! Honest limits: this is the standard WSOLA with no transient detection. Drums and
//! breaks, which Tempo Match is for, stay punchy at modest ratios because a grain
//! starts at the hit; big ratios on sustained, tonal material get the familiar
//! slight phasiness of any grain-based stretch.

/// Grain length in samples (~43 ms at 48 kHz): long enough to hold a couple of
/// periods of low notes, short enough not to smear a hi-hat.
const GRAIN: usize = 2048;
/// A new grain every quarter of a grain: four overlapping at a time.
const HOP: usize = GRAIN / 4;
/// How far, in source samples, a grain's start may move to find a better match.
const SEARCH: i32 = 384;
const SEARCH_STEP: i32 = 4;
/// Samples compared when scoring a candidate start.
const MATCH_LEN: usize = 384;

#[derive(Clone, Copy, Default)]
struct Grain {
    start: f32,
    age: usize,
    live: bool,
}

pub struct Stretcher {
    grains: [Grain; 4],
    next: usize,
    until_spawn: usize,
    last_start: f32,
    have_last: bool,
}

impl Default for Stretcher {
    fn default() -> Self {
        Self { grains: [Grain::default(); 4], next: 0, until_spawn: 0, last_start: 0.0, have_last: false }
    }
}

/// Reads `data` at `pos`, silent outside the playing region `[lower, upper]`.
fn read(data: &[f32], pos: f32, lower: f32, upper: f32) -> f32 {
    if pos < lower || pos > upper || data.len() < 2 {
        return 0.0;
    }
    let i = (pos as usize).min(data.len() - 2);
    let frac = (pos - i as f32).clamp(0.0, 1.0);
    data[i] + (data[i + 1] - data[i]) * frac
}

fn window(age: usize) -> f32 {
    0.5 - 0.5 * (std::f32::consts::TAU * age as f32 / GRAIN as f32).cos()
}

impl Stretcher {
    /// A fresh trigger: forget the old grains. The first grain starts half way
    /// through its window, so a hit's transient is not faded in.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// One output sample. `time_pos` is where in the source this moment belongs;
    /// `pitch_rate` is how fast each grain reads the source (1 samples per sample is
    /// the original pitch); `dir` is 1 forward, -1 backward.
    pub fn render(&mut self, data: &[f32], time_pos: f32, pitch_rate: f32, dir: f32, lower: f32, upper: f32) -> f32 {
        let step = pitch_rate * dir;
        if !self.have_last {
            // The first grain, already half played, so it starts at full level.
            let start = time_pos - step * (GRAIN / 2) as f32;
            self.grains[0] = Grain { start, age: GRAIN / 2, live: true };
            self.next = 1;
            self.last_start = start;
            self.have_last = true;
            self.until_spawn = HOP;
        } else if self.until_spawn == 0 {
            let start = self.align(data, time_pos, step, lower, upper);
            self.grains[self.next] = Grain { start, age: 0, live: true };
            self.next = (self.next + 1) % self.grains.len();
            self.last_start = start;
            self.until_spawn = HOP;
        }
        self.until_spawn -= 1;

        let (mut sum, mut weights) = (0.0, 0.0);
        for g in self.grains.iter_mut().filter(|g| g.live) {
            let w = window(g.age);
            sum += w * read(data, g.start + g.age as f32 * step, lower, upper);
            weights += w;
            g.age += 1;
            if g.age >= GRAIN {
                g.live = false;
            }
        }
        sum / weights.max(1e-3)
    }

    /// Picks the start near `time_pos` whose next stretch of waveform best matches
    /// where the previous grain is already going, so the two add up in phase.
    fn align(&self, data: &[f32], time_pos: f32, step: f32, lower: f32, upper: f32) -> f32 {
        // Where the previous grain is reading right now.
        let heading = self.last_start + HOP as f32 * step;
        let probe = |from: f32, k: usize| read(data, from + k as f32 * step, lower, upper);
        let mut best = (f32::MIN, 0.0f32);
        let mut d = -SEARCH;
        while d <= SEARCH {
            let cand = time_pos + d as f32;
            let (mut dot, mut energy) = (0.0f32, 1e-6f32);
            for k in (0..MATCH_LEN).step_by(2) {
                let (a, b) = (probe(heading, k), probe(cand, k));
                dot += a * b;
                energy += b * b;
            }
            // Normalised, and gently biased toward not moving at all.
            let score = dot / energy.sqrt() - d.abs() as f32 * 1e-5;
            if score > best.0 {
                best = (score, d as f32);
            }
            d += SEARCH_STEP;
        }
        time_pos + best.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A steady tone, stretched to 1.5x and 0.75x length, keeps its pitch.
    #[test]
    fn a_stretched_tone_keeps_its_pitch() {
        let sr = 48_000.0;
        let f = 440.0;
        let data: Vec<f32> = (0..96_000).map(|i| (i as f32 * f / sr * std::f32::consts::TAU).sin() * 0.8).collect();
        for ratio in [0.75f32, 1.0, 1.5] {
            let mut s = Stretcher::default();
            // Time moves through the source at 1/ratio... a ratio of 1.5 means the
            // output is 1.5x longer, so the source advances 1/1.5 per output sample.
            let time_rate = 1.0 / ratio;
            let mut pos = 20_000.0;
            let out: Vec<f32> = (0..48_000)
                .map(|_| {
                    let x = s.render(&data, pos, 1.0, 1.0, 0.0, 95_999.0);
                    pos += time_rate;
                    x
                })
                .collect();
            let body = &out[8_000..40_000];
            let crossings = body.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
            let hz = crossings as f32 / 2.0 / (body.len() as f32 / sr);
            assert!((hz - f).abs() < 12.0, "ratio {ratio}: still about {f} Hz, got {hz}");
            let rms = (body.iter().map(|v| v * v).sum::<f32>() / body.len() as f32).sqrt();
            assert!((0.45..0.7).contains(&rms), "ratio {ratio}: the level holds up: {rms}");
        }
    }

    /// Played in place (the first output samples), a transient is not faded in.
    #[test]
    fn a_hit_keeps_its_attack() {
        let mut data = vec![0.0f32; 20_000];
        for (i, v) in data.iter_mut().enumerate().skip(100).take(400) {
            *v = (1.0 - (i - 100) as f32 / 400.0) * if i % 2 == 0 { 0.9 } else { -0.9 };
        }
        let mut s = Stretcher::default();
        let mut pos = 0.0;
        let out: Vec<f32> = (0..2_000)
            .map(|_| {
                let x = s.render(&data, pos, 1.0, 1.0, 0.0, 19_999.0);
                pos += 0.8;
                x
            })
            .collect();
        let early_peak = out[100..300].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(early_peak > 0.5, "the transient arrives at full level: {early_peak}");
    }
}
