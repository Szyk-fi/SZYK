//! Voltage's stereo effects: chorus -> ping-pong delay -> reverb, the
//! three effects that most of synthwave's sound depends on. A dry
//! Voltage patch is a 1980s polysynth voice. Most of the genre's
//! recognisable sound comes from what follows it:
//! - the BBD chorus every Juno-class poly had built in;
//! - a dark tape-style echo, usually around a dotted eighth;
//! - a long plate or hall reverb.
//!
//! Each stage is a standard design, not a model of any one unit:
//! - **Chorus:** one short modulated delay line read by two taps whose
//!   triangle LFOs run in antiphase, one per side. This is the
//!   topology of the two-channel BBD choruses in Juno-era polysynths.
//!   Centre delay is 3.5 ms with up to ±1.85 ms of sweep, roughly the
//!   range those circuits used. There is no BBD clock noise or
//!   companding.
//! - **Delay:** a ping-pong pair of lines with a one-pole lowpass in the
//!   feedback path, so each repeat is darker than the last, like tape or
//!   analog echo.
//! - **Reverb:** an 8-line feedback delay network with a Householder
//!   mixing matrix and two input allpass diffusers. It uses the same
//!   design and line lengths as the synthesis platform's `reverb` block
//!   (src/synthesis/blocks.rs), with the outputs split across two
//!   sign patterns for a decorrelated stereo field.
//!
//! All buffers are allocated once, in `StereoFx::new`, sized for
//! 96 kHz, so the audio thread never allocates. A stage whose mix is
//! zero is skipped entirely, which saves cycles on the device.

use std::f32::consts::TAU;

const MAX_SR: f32 = 96_000.0;
/// Longest delay time the delay stage can be set to, in seconds.
pub const MAX_DELAY_S: f32 = 1.0;
pub const MIN_DELAY_S: f32 = 0.04;

/// A ring buffer read with linear interpolation.
struct Line {
    buf: Vec<f32>,
    w: usize,
}

impl Line {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(8)], w: 0 }
    }
    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.w] = x;
        self.w = (self.w + 1) % self.buf.len();
    }
    /// The sample `delay` samples ago (fractional), clamped to the buffer.
    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let len = self.buf.len();
        let d = delay.clamp(1.0, (len - 2) as f32);
        let pos = self.w as f32 + len as f32 - d;
        let i0 = pos as usize % len;
        let i1 = (i0 + 1) % len;
        let f = pos - pos.floor();
        self.buf[i0] * (1.0 - f) + self.buf[i1] * f
    }
}

/// The settings for one block, read from the app's atomics by the caller.
#[derive(Clone, Copy, Default)]
pub struct FxSettings {
    /// 0 = off, 1 = full wet/dry balance.
    pub chorus: f32,
    /// Seconds, `MIN_DELAY_S..=MAX_DELAY_S`.
    pub delay_time: f32,
    /// 0..0.9.
    pub delay_feedback: f32,
    pub delay_mix: f32,
    /// 0..1, from a small room to a long hall.
    pub reverb_size: f32,
    pub reverb_mix: f32,
}

const FDN_BASE: [f32; 8] = [1557.0, 1617.0, 1491.0, 1422.0, 1277.0, 1356.0, 1188.0, 1116.0];
/// How far `reverb_size` stretches the network's line lengths.
const FDN_MAX_STRETCH: f32 = 1.6;

pub struct StereoFx {
    chorus_line: Line,
    chorus_phase: f32,
    delay_l: Line,
    delay_r: Line,
    delay_lp: [f32; 2],
    /// The delay time in samples, slewed so turning the knob glides the
    /// pitch of the repeats the way a tape echo does, instead of clicking.
    delay_samples: f32,
    fdn: [Line; 8],
    fdn_lp: [f32; 8],
    diffusers: [Line; 2],
    fdn_lfo: f32,
}

impl Default for StereoFx {
    fn default() -> Self {
        Self::new()
    }
}

impl StereoFx {
    pub fn new() -> Self {
        let fdn_scale = MAX_SR / 44_100.0 * FDN_MAX_STRETCH;
        Self {
            chorus_line: Line::new((0.012 * MAX_SR) as usize),
            chorus_phase: 0.0,
            delay_l: Line::new((MAX_DELAY_S * MAX_SR) as usize + 16),
            delay_r: Line::new((MAX_DELAY_S * MAX_SR) as usize + 16),
            delay_lp: [0.0; 2],
            delay_samples: 0.0,
            fdn: FDN_BASE.map(|b| Line::new((b * fdn_scale) as usize + 64)),
            fdn_lp: [0.0; 8],
            diffusers: [Line::new((0.013 * MAX_SR) as usize + 8), Line::new((0.021 * MAX_SR) as usize + 8)],
            fdn_lfo: 0.0,
        }
    }

    /// True when every stage is bypassed, so the caller can skip the call.
    pub fn bypassed(s: &FxSettings) -> bool {
        s.chorus <= 0.0 && s.delay_mix <= 0.0 && s.reverb_mix <= 0.0
    }

    /// Processes one mono input sample into a stereo pair.
    #[inline]
    pub fn tick(&mut self, x: f32, s: &FxSettings, sr: f32) -> (f32, f32) {
        let inv_sr = 1.0 / sr;

        // --- chorus: mono in, stereo out
        let (mut l, mut r) = (x, x);
        if s.chorus > 0.0 {
            let amt = s.chorus.clamp(0.0, 1.0);
            self.chorus_line.push(x);
            self.chorus_phase = (self.chorus_phase + (0.5 + 0.35 * amt) * inv_sr).fract();
            // Triangle LFO: BBD choruses swept their clock with a
            // triangle, which gives a steadier pitch wobble than a sine.
            let tri = 1.0 - 4.0 * (self.chorus_phase - 0.5).abs();
            let swing = 0.00185 * amt;
            let wet_l = self.chorus_line.read((0.0035 + swing * tri) * sr);
            let wet_r = self.chorus_line.read((0.0035 - swing * tri) * sr);
            let dry = 1.0 - 0.5 * amt;
            l = x * dry + wet_l * 0.5 * amt;
            r = x * dry + wet_r * 0.5 * amt;
        }

        // --- ping-pong delay: the first repeat comes from the left
        if s.delay_mix > 0.0 {
            let target = s.delay_time.clamp(MIN_DELAY_S, MAX_DELAY_S) * sr;
            if self.delay_samples <= 0.0 {
                self.delay_samples = target;
            }
            self.delay_samples += (target - self.delay_samples) * (1.0 - (-inv_sr / 0.08).exp());
            let fb = s.delay_feedback.clamp(0.0, 0.9);
            let out_l = self.delay_l.read(self.delay_samples);
            let out_r = self.delay_r.read(self.delay_samples);
            // About 3.4 kHz: each repeat loses a little top end.
            let k = 1.0 - (-TAU * 3400.0 * inv_sr).exp();
            self.delay_lp[0] += (out_l - self.delay_lp[0]) * k;
            self.delay_lp[1] += (out_r - self.delay_lp[1]) * k;
            self.delay_l.push((l + r) * 0.5 + self.delay_lp[1] * fb);
            self.delay_r.push(self.delay_lp[0] * fb);
            let mix = s.delay_mix.clamp(0.0, 1.0);
            l += out_l * mix;
            r += out_r * mix;
        }

        // --- reverb
        if s.reverb_mix > 0.0 {
            let size = s.reverb_size.clamp(0.0, 1.0);
            let fb = 0.72 + size * 0.25;
            let damp = 0.35;
            let stretch = sr / 44_100.0 * (1.0 + size * (FDN_MAX_STRETCH - 1.0));
            self.fdn_lfo = (self.fdn_lfo + 0.37 * inv_sr).fract();
            let mut d_in = (l + r) * 0.5 * 0.35;
            for (k, d) in self.diffusers.iter_mut().enumerate() {
                let t = d.buf.len() as f32 * sr / MAX_SR - 6.0;
                let dd = d.read(t);
                let g = if k == 0 { 0.7 } else { 0.6 };
                let w = d_in + g * dd;
                d.push(w);
                d_in = dd - g * w;
            }
            let mut outs = [0.0f32; 8];
            for k in 0..8 {
                // A slow, small wobble on each line's length keeps the
                // tail from ringing at fixed comb frequencies.
                let m = ((self.fdn_lfo + k as f32 * 0.125) * TAU).sin() * 8.0;
                outs[k] = self.fdn[k].read(FDN_BASE[k] * stretch + m);
            }
            let sum: f32 = outs.iter().sum::<f32>() * 0.25;
            let (mut wet_l, mut wet_r) = (0.0, 0.0);
            for k in 0..8 {
                let mixed = outs[k] - sum;
                self.fdn_lp[k] += (mixed - self.fdn_lp[k]) * (1.0 - damp);
                self.fdn[k].push(d_in + self.fdn_lp[k] * fb);
                wet_l += outs[k] * if k % 2 == 0 { 1.0 } else { -1.0 };
                wet_r += outs[k] * if k < 4 { 1.0 } else { -1.0 };
            }
            let mix = s.reverb_mix.clamp(0.0, 1.0);
            l = l * (1.0 - 0.3 * mix) + wet_l * 0.3 * mix;
            r = r * (1.0 - 0.3 * mix) + wet_r * 0.3 * mix;
        }

        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: FxSettings, impulse_at: usize, n: usize) -> Vec<(f32, f32)> {
        let mut fx = StereoFx::new();
        (0..n).map(|i| fx.tick(if i == impulse_at { 1.0 } else { 0.0 }, &s, 48_000.0)).collect()
    }

    #[test]
    fn bypass_is_transparent() {
        let s = FxSettings::default();
        assert!(StereoFx::bypassed(&s));
        let mut fx = StereoFx::new();
        assert_eq!(fx.tick(0.5, &s, 48_000.0), (0.5, 0.5));
    }

    #[test]
    fn the_delay_repeats_left_then_right_at_the_set_time() {
        let s = FxSettings { delay_time: 0.25, delay_feedback: 0.5, delay_mix: 1.0, ..Default::default() };
        let out = run(s, 0, 48_000);
        let at = 12_000; // 0.25 s at 48 kHz
        let left: f32 = out[at - 4..at + 4].iter().map(|o| o.0.abs()).sum();
        let right_first: f32 = out[at - 4..at + 4].iter().map(|o| o.1.abs()).sum();
        let right_second: f32 = out[2 * at - 4..2 * at + 4].iter().map(|o| o.1.abs()).sum();
        assert!(left > 0.1, "first repeat on the left, got {left}");
        assert!(right_first < 1e-3, "nothing on the right yet, got {right_first}");
        assert!(right_second > 0.01, "second repeat on the right, got {right_second}");
    }

    #[test]
    fn the_reverb_tail_is_stereo_and_decays() {
        let s = FxSettings { reverb_size: 0.6, reverb_mix: 1.0, ..Default::default() };
        let out = run(s, 0, 48_000 * 4);
        let early: f32 = out[4_800..24_000].iter().map(|o| o.0 * o.0).sum();
        let late: f32 = out[48_000 * 3..].iter().map(|o| o.0 * o.0).sum();
        let diff: f32 = out[4_800..24_000].iter().map(|o| (o.0 - o.1).abs()).sum();
        assert!(early > 1e-4, "a tail exists, got {early}");
        assert!(late < early * 0.05, "and decays: early {early}, late {late}");
        assert!(diff > 1e-3, "left and right differ (a stereo field)");
        assert!(out.iter().all(|o| o.0.is_finite() && o.1.is_finite()));
    }

    #[test]
    fn the_chorus_widens_a_mono_tone() {
        let s = FxSettings { chorus: 1.0, ..Default::default() };
        let mut fx = StereoFx::new();
        let mut diff = 0.0;
        for i in 0..48_000 {
            let x = (i as f32 * 220.0 / 48_000.0 * TAU).sin() * 0.5;
            let (l, r) = fx.tick(x, &s, 48_000.0);
            diff += (l - r).abs();
            assert!(l.abs() < 1.0 && r.abs() < 1.0);
        }
        assert!(diff > 100.0, "the two sides differ, got {diff}");
    }

    #[test]
    fn full_feedback_and_size_stay_bounded() {
        let s = FxSettings { chorus: 1.0, delay_time: 0.04, delay_feedback: 0.9, delay_mix: 1.0, reverb_size: 1.0, reverb_mix: 1.0 };
        let mut fx = StereoFx::new();
        let mut peak = 0.0f32;
        for i in 0..48_000 * 5 {
            let x = if i < 48_000 { ((i * 7919) % 200) as f32 / 100.0 - 1.0 } else { 0.0 };
            let (l, r) = fx.tick(x, &s, 48_000.0);
            peak = peak.max(l.abs()).max(r.abs());
        }
        assert!(peak.is_finite() && peak < 8.0, "peak {peak}");
    }
}
