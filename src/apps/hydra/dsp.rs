//! Hydra's DSP building blocks: band-limited oscillator shapes, filters,
//! envelopes, LFOs and small utilities. Everything here is allocation-free
//! per sample (delay lines are sized once, when a voice is built, off the
//! audio thread).

use std::f32::consts::{PI, TAU};

/// Frequency of MIDI note `n` (fractional notes allowed), A4 = 440 Hz.
pub fn midi_hz(n: f32) -> f32 {
    440.0 * ((n - 69.0) / 12.0).exp2()
}

/// A random-number source for noise, drift and sample-and-hold
/// (xorshift32: fast, no allocation, repeatable from its seed).
#[derive(Clone)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed | 1)
    }
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// -1..1.
    pub fn bipolar(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 8_388_608.0 - 1.0
    }
    /// 0..1.
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16_777_216.0
    }
}

/// PolyBLEP residual for a unit step at phase 0: subtract it from a naive
/// sawtooth (or add/subtract it at each edge of a pulse) to remove the
/// aliasing of the discontinuity. `t` is the phase in 0..1, `dt` the phase
/// increment per sample.
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// PolyBLAMP residual for a kink (a slope change) at phase 0, scaled to a
/// unit slope change per unit phase; multiply by `slope_change * dt`.
pub fn poly_blamp(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt - 1.0;
        -(x * x * x) / 3.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt + 1.0;
        (x * x * x) / 3.0
    } else {
        0.0
    }
}

/// Band-limited sawtooth, -1..1 rising, at phase `p` (0..1).
pub fn saw(p: f32, dt: f32) -> f32 {
    2.0 * p - 1.0 - poly_blep(p, dt)
}

/// Band-limited pulse of width `w` (0..1), +1 for the first `w` of the cycle.
pub fn pulse(p: f32, dt: f32, w: f32) -> f32 {
    let w = w.clamp(0.02, 0.98);
    let mut y = if p < w { 1.0 } else { -1.0 };
    y += poly_blep(p, dt);
    y -= poly_blep((p - w).rem_euclid(1.0), dt);
    y
}

/// Triangle, -1..1, with PolyBLAMP at its two slope changes (so its
/// aliasing is far below a naive triangle's).
pub fn triangle(p: f32, dt: f32) -> f32 {
    let mut y = 4.0 * (p - 0.5).abs() - 1.0;
    // Slope goes from +4 to -4 at phase 0 (change -8) and from -4 to +4 at
    // phase 0.5 (change +8).
    y -= 8.0 * dt * poly_blamp(p, dt);
    y += 8.0 * dt * poly_blamp((p + 0.5).rem_euclid(1.0), dt);
    y
}

/// A one-pole low-pass (also used for parameter smoothing).
#[derive(Clone, Copy, Default)]
pub struct OnePole {
    pub y: f32,
}

impl OnePole {
    pub fn new(y: f32) -> Self {
        Self { y }
    }
    /// `a` is the coefficient (0..1): the fraction of the way to `x` per call.
    #[inline]
    pub fn tick(&mut self, x: f32, a: f32) -> f32 {
        self.y += (x - self.y) * a;
        self.y
    }
}

/// Coefficient for a one-pole that moves most of the way in about `seconds`.
pub fn smooth_coef(seconds: f32, rate: f32) -> f32 {
    1.0 - (-1.0 / (seconds.max(1e-5) * rate)).exp()
}

/// Removes DC (the offset a waveshaper can add).
#[derive(Clone, Copy, Default)]
pub struct DcBlock {
    x1: f32,
    y1: f32,
}

impl DcBlock {
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + 0.995 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// Zero-delay-feedback state-variable filter (Zavalishin / Cytomic form):
/// stable and well-behaved when its cutoff is modulated at audio rate.
#[derive(Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

pub struct SvfOut {
    pub lp: f32,
    pub bp: f32,
    pub hp: f32,
}

/// The tangent warp for cutoff `fc` at `rate`.
#[inline]
pub fn warp(fc: f32, rate: f32) -> f32 {
    (PI * (fc / rate).clamp(1e-5, 0.49)).tan()
}

impl Svf {
    /// `g` from `warp`, `k` the damping (2 = no resonance, small = resonant).
    #[inline]
    pub fn tick(&mut self, x: f32, g: f32, k: f32) -> SvfOut {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut { lp: v2, bp: v1, hp: x - k * v1 - v2 }
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// A four-pole transistor-ladder low-pass in the zero-delay-feedback form
/// (four trapezoidal one-poles whose feedback is solved exactly in the
/// linear case), with `tanh` saturating the signal entering the stages.
/// That solves the loop for the linear ladder and saturates its result, which
/// is an approximation of the full nonlinear ladder; it self-oscillates
/// near full resonance and stays stable.
#[derive(Clone, Copy, Default)]
pub struct Ladder {
    z: [f32; 4],
}

pub struct LadderOut {
    pub input: f32,
    pub y: [f32; 4],
}

impl Ladder {
    #[inline]
    pub fn tick(&mut self, x: f32, g: f32, res: f32, drive: f32) -> LadderOut {
        let big_g = g / (1.0 + g);
        let leak = 1.0 / (1.0 + g); // 1 - G
        // A little past the critical gain of 4 at full resonance, so it
        // really self-oscillates; the tanh on the input is what bounds it.
        let k = 4.2 * res.clamp(0.0, 1.0);
        let s = big_g * big_g * big_g * leak * self.z[0] + big_g * big_g * leak * self.z[1] + big_g * leak * self.z[2] + leak * self.z[3];
        let u = (x - k * s) / (1.0 + k * big_g * big_g * big_g * big_g);
        let input = (u * (1.0 + 6.0 * drive.clamp(0.0, 1.0))).tanh();
        let mut y = [0.0; 4];
        let mut inp = input;
        for n in 0..4 {
            let v = (inp - self.z[n]) * big_g;
            y[n] = v + self.z[n];
            self.z[n] = y[n] + v;
            inp = y[n];
        }
        LadderOut { input, y }
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// A feedback comb filter, tuned by its cutoff (`rate / cutoff` samples
/// long), with the delay line sized when it is built.
#[derive(Clone)]
pub struct Comb {
    line: Vec<f32>,
    pos: usize,
}

impl Comb {
    pub fn new(max_samples: usize) -> Self {
        Self { line: vec![0.0; max_samples.max(8)], pos: 0 }
    }
    #[inline]
    pub fn tick(&mut self, x: f32, delay: f32, feedback: f32) -> f32 {
        let n = self.line.len();
        let d = delay.clamp(2.0, n as f32 - 2.0);
        let rp = self.pos as f32 + n as f32 - d;
        let i0 = rp as usize % n;
        let f = rp.fract();
        let a = self.line[i0];
        let b = self.line[(i0 + 1) % n];
        let delayed = a + (b - a) * f;
        let y = x + feedback * delayed;
        self.line[self.pos] = y.clamp(-8.0, 8.0);
        self.pos = (self.pos + 1) % n;
        y * (1.0 - 0.5 * feedback.abs())
    }
    pub fn reset(&mut self) {
        self.line.iter_mut().for_each(|s| *s = 0.0);
    }
}

/// A fixed-length stereo-agnostic fractional delay line.
#[derive(Clone)]
pub struct Delay {
    line: Vec<f32>,
    pos: usize,
}

impl Delay {
    pub fn new(samples: usize) -> Self {
        Self { line: vec![0.0; samples.max(4)], pos: 0 }
    }
    pub fn len(&self) -> usize {
        self.line.len()
    }
    #[inline]
    pub fn write(&mut self, x: f32) {
        self.line[self.pos] = x;
        self.pos = (self.pos + 1) % self.line.len();
    }
    /// The sample written `d` samples ago (fractional, linear interpolation).
    #[inline]
    pub fn read(&self, d: f32) -> f32 {
        let n = self.line.len();
        let d = d.clamp(1.0, n as f32 - 2.0);
        let rp = self.pos as f32 + n as f32 - d;
        let i0 = rp as usize % n;
        let f = rp.fract();
        let a = self.line[i0];
        let b = self.line[(i0 + 1) % n];
        a + (b - a) * f
    }
    pub fn clear(&mut self) {
        self.line.iter_mut().for_each(|s| *s = 0.0);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Release,
}

/// An exponential ADSR: the attack overshoots toward 1.25 so it bends the
/// way an analog envelope does, and the decay and release fall by about
/// 40 dB in the time they are set to.
#[derive(Clone, Copy)]
pub struct Env {
    stage: Stage,
    pub level: f32,
}

impl Default for Env {
    fn default() -> Self {
        Self { stage: Stage::Idle, level: 0.0 }
    }
}

const ATTACK_TARGET: f32 = 1.25;
/// time-constant divisor so a decay reaches ~1% of the way in the set time
const FALL: f32 = 4.6;

impl Env {
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }
    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }
    pub fn kill(&mut self) {
        self.stage = Stage::Idle;
        self.level = 0.0;
    }
    pub fn idle(&self) -> bool {
        self.stage == Stage::Idle
    }
    pub fn releasing(&self) -> bool {
        self.stage == Stage::Release
    }
    /// Advances one sample. Times in seconds.
    #[inline]
    pub fn tick(&mut self, a: f32, d: f32, s: f32, r: f32, rate: f32) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                let tau = a.max(1e-4) / 1.609;
                self.level += (ATTACK_TARGET - self.level) * (1.0 - (-1.0 / (tau * rate)).exp());
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                let tau = d.max(1e-3) / FALL;
                self.level += (s - self.level) * (1.0 - (-1.0 / (tau * rate)).exp());
            }
            Stage::Release => {
                let tau = r.max(1e-3) / FALL;
                self.level -= self.level * (1.0 - (-1.0 / (tau * rate)).exp());
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

/// An LFO with six shapes. Output is -1..1.
#[derive(Clone)]
pub struct Lfo {
    pub phase: f32,
    held: f32,
    target: f32,
    start: f32,
    rng: Rng,
}

impl Lfo {
    pub fn new(seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        let held = rng.bipolar();
        let target = rng.bipolar();
        Self { phase: 0.0, held, target, start: held, rng }
    }
    pub fn retrigger(&mut self, phase: f32) {
        self.phase = phase.rem_euclid(1.0);
    }
    /// Advances by `dt` seconds at `rate` Hz and returns the value.
    #[inline]
    pub fn tick(&mut self, shape: usize, rate: f32, dt: f32) -> f32 {
        let inc = rate * dt;
        self.phase += inc;
        let wrapped = self.phase >= 1.0;
        if wrapped {
            self.phase -= self.phase.floor();
            self.held = self.target;
            self.start = self.held;
            self.target = self.rng.bipolar();
        }
        let p = self.phase;
        match shape {
            0 => (p * TAU).sin(),
            1 => 4.0 * (p - 0.5).abs() - 1.0,
            2 => 2.0 * p - 1.0,
            3 => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            4 => self.held,
            _ => {
                // smooth random: a cosine glide from the last value to the next
                let x = 0.5 - 0.5 * (p * PI).cos();
                self.start + (self.target - self.start) * x
            }
        }
    }
}

/// A smooth saturator: `tanh` scaled so small signals pass at unity.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    x.tanh()
}

/// Folds a signal back on itself (a sine wavefolder), identity at `amount` 0.
#[inline]
pub fn fold(x: f32, amount: f32) -> f32 {
    if amount <= 0.0 {
        return x;
    }
    let k = 1.0 + 7.0 * amount;
    let folded = (x * k * std::f32::consts::FRAC_PI_2).sin();
    x + (folded - x) * (amount * 4.0).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustfft::{num_complex::Complex, FftPlanner};

    const FS: f32 = 48_000.0;

    /// Energy outside the harmonics of `f0`, relative to the total, in dB.
    fn inharmonic_db(signal: &[f32], f0: f32) -> f32 {
        let n = signal.len();
        let mut buf: Vec<Complex<f32>> = signal.iter().enumerate().map(|(i, s)| Complex::new(s * (0.5 - 0.5 * (TAU * i as f32 / n as f32).cos()), 0.0)).collect();
        FftPlanner::new().plan_fft_forward(n).process(&mut buf);
        let bin_hz = FS / n as f32;
        let (mut total, mut off) = (0.0f32, 0.0f32);
        for (k, c) in buf.iter().enumerate().take(n / 2).skip(1) {
            let e = c.norm_sqr();
            total += e;
            let h = (k as f32 * bin_hz / f0).round();
            if (k as f32 * bin_hz - h * f0).abs() > 3.5 * bin_hz {
                off += e;
            }
        }
        10.0 * (off / total).log10()
    }

    fn render(f0: f32, n: usize, f: impl Fn(f32, f32) -> f32) -> Vec<f32> {
        let dt = f0 / FS;
        let mut p = 0.0f32;
        (0..n)
            .map(|_| {
                let y = f(p, dt);
                p += dt;
                if p >= 1.0 {
                    p -= 1.0;
                }
                y
            })
            .collect()
    }

    #[test]
    fn band_limited_shapes_alias_far_less_than_naive_ones() {
        // 4.8 kHz is hard on a naive saw: harmonics reach well past Nyquist.
        let f0 = 4800.0 * 1.0012;
        let n = 16384;
        let naive = render(f0, n, |p, _| 2.0 * p - 1.0);
        let blep = render(f0, n, saw);
        let (nv, bl) = (inharmonic_db(&naive, f0), inharmonic_db(&blep, f0));
        assert!(bl < nv - 12.0, "PolyBLEP saw ({bl:.1} dB) must alias well below the naive saw ({nv:.1} dB)");
        let naive_pulse = render(f0, n, |p, _| if p < 0.3 { 1.0 } else { -1.0 });
        let blep_pulse = render(f0, n, |p, dt| pulse(p, dt, 0.3));
        assert!(inharmonic_db(&blep_pulse, f0) < inharmonic_db(&naive_pulse, f0) - 6.0, "PolyBLEP pulse: {:.1} vs naive {:.1} dB", inharmonic_db(&blep_pulse, f0), inharmonic_db(&naive_pulse, f0));
        let naive_tri = render(f0, n, |p, _| 4.0 * (p - 0.5).abs() - 1.0);
        let blamp_tri = render(f0, n, triangle);
        assert!(inharmonic_db(&blamp_tri, f0) < inharmonic_db(&naive_tri, f0) - 6.0, "PolyBLAMP triangle");
    }

    #[test]
    fn shapes_stay_in_range_and_have_no_dc() {
        for f0 in [55.0, 440.0, 3000.0] {
            for (name, y) in [("saw", render(f0, 48000, saw)), ("pulse", render(f0, 48000, |p, dt| pulse(p, dt, 0.5))), ("tri", render(f0, 48000, triangle))] {
                let peak = y.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                let dc = y.iter().sum::<f32>() / y.len() as f32;
                assert!(peak < 1.25 && peak > 0.7, "{name} at {f0}: peak {peak}");
                assert!(dc.abs() < 0.05, "{name} at {f0}: dc {dc}");
            }
        }
    }

    fn run_svf(mode: usize, f: f32, fc: f32) -> f32 {
        let mut s = Svf::default();
        let g = warp(fc, FS);
        let mut peak = 0.0f32;
        for i in 0..9600 {
            let x = (TAU * f * i as f32 / FS).sin();
            let o = s.tick(x, g, 1.4);
            let y = [o.lp, o.bp, o.hp][mode];
            if i > 4800 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn the_svf_lowpasses_highpasses_and_bandpasses() {
        assert!(run_svf(0, 200.0, 2000.0) > 0.9, "LP passes lows");
        assert!(run_svf(0, 12000.0, 1000.0) < 0.02, "LP rejects highs");
        assert!(run_svf(2, 12000.0, 1000.0) > 0.9, "HP passes highs");
        assert!(run_svf(2, 100.0, 4000.0) < 0.02, "HP rejects lows");
        let bp = run_svf(1, 1000.0, 1000.0);
        assert!(bp > run_svf(1, 100.0, 1000.0) * 3.0 && bp > run_svf(1, 10000.0, 1000.0) * 3.0, "BP peaks at its centre");
    }

    #[test]
    fn the_ladder_is_a_24db_lowpass_that_resonates_but_stays_bounded() {
        let measure = |f: f32, res: f32| {
            let mut l = Ladder::default();
            let g = warp(1000.0, FS);
            let mut peak = 0.0f32;
            for i in 0..14400 {
                let x = 0.5 * (TAU * f * i as f32 / FS).sin();
                let y = l.tick(x, g, res, 0.0).y[3];
                assert!(y.is_finite() && y.abs() < 8.0, "bounded: {y}");
                if i > 9600 {
                    peak = peak.max(y.abs());
                }
            }
            peak
        };
        let (pass, stop) = (measure(200.0, 0.0), measure(4000.0, 0.0));
        assert!(stop < pass * 0.02, "two octaves above cutoff is down more than 34 dB: {pass} vs {stop}");
        assert!(measure(1000.0, 0.9) > measure(1000.0, 0.0) * 2.0, "resonance boosts the cutoff region");
        // near full resonance it rings on its own from a tiny kick
        let mut l = Ladder::default();
        let g = warp(1000.0, FS);
        let mut late = 0.0f32;
        for i in 0..48000 {
            let x = if i == 0 { 0.1 } else { 0.0 };
            let y = l.tick(x, g, 0.995, 0.0).y[3];
            assert!(y.is_finite() && y.abs() < 8.0);
            if i > 40000 {
                late = late.max(y.abs());
            }
        }
        assert!(late > 0.001, "it self-oscillates: {late}");
    }

    #[test]
    fn the_envelope_attacks_decays_holds_and_releases() {
        let mut e = Env::default();
        e.gate_on();
        let mut t_attack = 0;
        for i in 0..48000 {
            let v = e.tick(0.1, 0.2, 0.5, 0.3, FS);
            if v >= 1.0 && t_attack == 0 {
                t_attack = i;
            }
        }
        assert!((t_attack as f32 / FS - 0.1).abs() < 0.01, "a 100 ms attack reaches 1.0 in about 100 ms: {t_attack}");
        assert!((e.level - 0.5).abs() < 0.01, "it settles on the sustain level: {}", e.level);
        e.gate_off();
        let mut quiet = 0;
        for i in 0..96000 {
            e.tick(0.1, 0.2, 0.5, 0.3, FS);
            if e.idle() {
                quiet = i;
                break;
            }
        }
        assert!(quiet > 0 && quiet as f32 / FS < 1.0, "the release ends: {quiet}");
        assert_eq!(e.level, 0.0);
    }

    #[test]
    fn lfo_shapes_cover_their_range_at_their_rate() {
        for shape in 0..6 {
            let mut l = Lfo::new(7);
            let (mut lo, mut hi) = (1.0f32, -1.0f32);
            for _ in 0..48000 {
                let v = l.tick(shape, 5.0, 1.0 / FS);
                assert!((-1.0001..=1.0001).contains(&v), "shape {shape}: {v}");
                lo = lo.min(v);
                hi = hi.max(v);
            }
            assert!(hi - lo > 1.0, "shape {shape} moves: {lo}..{hi}");
        }
        let mut l = Lfo::new(1);
        let mut wraps = 0;
        let mut last = l.tick(2, 4.0, 1.0 / FS);
        for _ in 0..48000 {
            let v = l.tick(2, 4.0, 1.0 / FS);
            if v < last {
                wraps += 1;
            }
            last = v;
        }
        assert!((3..=4).contains(&wraps), "4 Hz is four cycles a second (the first may start mid-cycle): {wraps}");
    }

    #[test]
    fn the_wavefolder_is_transparent_at_zero_and_stays_bounded() {
        assert_eq!(fold(0.3, 0.0), 0.3);
        for a in [0.2, 0.6, 1.0] {
            for i in -10..=10 {
                assert!(fold(i as f32 / 10.0, a).abs() <= 1.2);
            }
        }
    }

    #[test]
    fn the_comb_rings_at_the_period_it_is_tuned_to() {
        let mut c = Comb::new(4096);
        let delay = FS / 480.0;
        let mut out = Vec::new();
        for i in 0..4800 {
            out.push(c.tick(if i == 0 { 1.0 } else { 0.0 }, delay, 0.9));
        }
        let first_echo = out.iter().enumerate().skip(10).max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i).unwrap();
        assert!((first_echo as f32 - delay).abs() <= 1.5, "echo at {first_echo}, expected {delay}");
    }
}
