//! Hydra's effects chain, stereo: drive, chorus, phaser, ping-pong delay and
//! a feedback-delay-network reverb. Every effect passes the signal through
//! untouched when its mix is zero, and every delay line is allocated when
//! the chain is built (never on the audio thread).

use super::dsp::*;
use super::params::*;
use super::store::Snapshot;
use std::f32::consts::TAU;

/// Longest delay time, seconds, and the highest sample rate lines are sized for.
const DELAY_SECONDS: f32 = 1.6;
const MAX_RATE: f32 = 192_000.0;
/// The reverb's eight delay lengths, milliseconds (mutually prime-ish so the
/// echoes don't line up).
const REVERB_MS: [f32; 8] = [29.7, 37.1, 41.1, 43.7, 53.3, 59.9, 67.3, 73.9];
const REVERB_LINE: usize = (0.0739 * 1.7 * MAX_RATE) as usize + 64;
const FX_BLOCK: usize = 16;

struct Chorus {
    line: [Delay; 2],
    phase: f32,
}

struct Phaser {
    z: [[f32; 6]; 2],
    last: [f32; 2],
    phase: f32,
    coef: [f32; 2],
}

struct DelayFx {
    line: [Delay; 2],
    lp: [f32; 2],
    time: OnePole,
}

struct Reverb {
    line: Vec<Delay>,
    lp: [f32; 8],
    pre: [Delay; 2],
    phase: [f32; 8],
    size: OnePole,
}

pub struct Fx {
    dc: [DcBlock; 2],
    crush_hold: [f32; 2],
    crush_count: f32,
    chorus: Chorus,
    phaser: Phaser,
    delay: DelayFx,
    reverb: Reverb,
}

impl Fx {
    pub fn new() -> Self {
        let delay_len = (DELAY_SECONDS * MAX_RATE) as usize + 16;
        Self {
            dc: [DcBlock::default(); 2],
            crush_hold: [0.0; 2],
            crush_count: 0.0,
            chorus: Chorus { line: [Delay::new(16_384), Delay::new(16_384)], phase: 0.0 },
            phaser: Phaser { z: [[0.0; 6]; 2], last: [0.0; 2], phase: 0.0, coef: [0.0; 2] },
            delay: DelayFx { line: [Delay::new(delay_len), Delay::new(delay_len)], lp: [0.0; 2], time: OnePole::new(0.35 * 48_000.0) },
            reverb: Reverb { line: (0..8).map(|_| Delay::new(REVERB_LINE)).collect(), lp: [0.0; 8], pre: [Delay::new(40_000), Delay::new(40_000)], phase: [0.0, 0.7, 1.9, 2.6, 3.7, 4.4, 5.3, 6.1], size: OnePole::new(0.6) },
        }
    }

    pub fn reset(&mut self) {
        let fresh = Fx::new();
        *self = fresh;
    }

    /// Runs the chain over `l` / `r` in place.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], snap: &Snapshot, rate: f32) {
        self.drive(l, r, snap, rate);
        self.chorus(l, r, snap, rate);
        self.phaser(l, r, snap, rate);
        self.delay(l, r, snap, rate);
        self.reverb(l, r, snap, rate);
    }

    fn drive(&mut self, l: &mut [f32], r: &mut [f32], snap: &Snapshot, _rate: f32) {
        let kind = snap[P::Drv_Type as usize] as usize;
        let mix = snap[P::Drv_Mix as usize];
        if kind == 0 || mix <= 0.0 {
            return;
        }
        let a = snap[P::Drv_Amt as usize];
        let gain = 1.0 + 24.0 * a * a;
        let comp = 1.0 / gain.sqrt();
        let bits = 16.0 - 13.0 * a;
        let levels = (2.0f32).powf(bits - 1.0);
        let hold = 1.0 + 24.0 * a * a;
        for i in 0..l.len() {
            for ch in 0..2 {
                let x = if ch == 0 { l[i] } else { r[i] };
                let y = match kind {
                    1 => (x * gain).tanh() * comp,
                    2 => (x * gain).clamp(-1.0, 1.0) * comp,
                    3 => fold(x * (1.0 + 3.0 * a), a.max(0.15)) * (1.0 - 0.3 * a),
                    _ => {
                        if ch == 0 {
                            self.crush_count += 1.0;
                        }
                        if self.crush_count >= hold {
                            self.crush_hold[ch] = (x * levels).round() / levels;
                            if ch == 1 {
                                self.crush_count -= hold;
                            }
                        }
                        self.crush_hold[ch]
                    }
                };
                let y = self.dc[ch].tick(y);
                let out = x + (y - x) * mix;
                if ch == 0 {
                    l[i] = out;
                } else {
                    r[i] = out;
                }
            }
        }
    }

    fn chorus(&mut self, l: &mut [f32], r: &mut [f32], snap: &Snapshot, rate: f32) {
        let mix = snap[P::Cho_Mix as usize];
        if mix <= 0.0 {
            return;
        }
        let c = &mut self.chorus;
        let inc = snap[P::Cho_Rate as usize] / rate;
        let depth = snap[P::Cho_Depth as usize] * 0.008 * rate;
        let base = 0.012 * rate;
        for i in 0..l.len() {
            c.phase += inc;
            c.phase -= c.phase.floor();
            c.line[0].write(l[i]);
            c.line[1].write(r[i]);
            let (mut wl, mut wr) = (0.0, 0.0);
            for tap in 0..3 {
                let off = tap as f32 / 3.0;
                let ml = (TAU * (c.phase + off)).sin();
                let mr = (TAU * (c.phase + off + 0.5)).sin();
                wl += c.line[0].read(base + depth * ml);
                wr += c.line[1].read(base + depth * mr);
            }
            let w = 0.7 / 3.0f32.sqrt();
            l[i] = l[i] * (1.0 - 0.4 * mix) + wl * w * mix;
            r[i] = r[i] * (1.0 - 0.4 * mix) + wr * w * mix;
        }
    }

    fn phaser(&mut self, l: &mut [f32], r: &mut [f32], snap: &Snapshot, rate: f32) {
        let mix = snap[P::Ph_Mix as usize];
        if mix <= 0.0 {
            return;
        }
        let p = &mut self.phaser;
        let inc = snap[P::Ph_Rate as usize] / rate;
        let depth = snap[P::Ph_Depth as usize];
        let fb = snap[P::Ph_Fb as usize] * 0.85;
        for i in 0..l.len() {
            if i % FX_BLOCK == 0 {
                p.phase += inc * FX_BLOCK as f32;
                p.phase -= p.phase.floor();
                for ch in 0..2 {
                    let lfo = (TAU * (p.phase + 0.25 * ch as f32)).sin();
                    let fc = 200.0 * (4000.0f32 / 200.0).powf(0.5 + 0.5 * lfo * depth);
                    let t = (std::f32::consts::PI * (fc / rate).min(0.45)).tan();
                    p.coef[ch] = (t - 1.0) / (t + 1.0);
                }
            }
            for ch in 0..2 {
                let x = if ch == 0 { l[i] } else { r[i] };
                let mut y = x + fb * p.last[ch];
                for z in p.z[ch].iter_mut() {
                    let out = p.coef[ch] * y + *z;
                    *z = y - p.coef[ch] * out;
                    y = out;
                }
                p.last[ch] = y.clamp(-4.0, 4.0);
                let wet = 0.5 * (x + y);
                let out = x * (1.0 - mix) + wet * mix * 1.3;
                if ch == 0 {
                    l[i] = out;
                } else {
                    r[i] = out;
                }
            }
        }
    }

    fn delay(&mut self, l: &mut [f32], r: &mut [f32], snap: &Snapshot, rate: f32) {
        let mix = snap[P::Dly_Mix as usize];
        if mix <= 0.0 {
            // keep the lines from holding stale audio that would reappear
            return;
        }
        let d = &mut self.delay;
        let target = (snap[P::Dly_Time as usize] * 0.001 * rate).min(DELAY_SECONDS * rate);
        let fb = snap[P::Dly_Fb as usize].min(0.95);
        let tone = snap[P::Dly_Tone as usize];
        let lp_a = 0.04 + 0.96 * tone * tone;
        let ping = snap[P::Dly_Ping as usize] >= 0.5;
        let t_coef = smooth_coef(0.08, rate);
        for i in 0..l.len() {
            let t = d.time.tick(target, t_coef);
            let dl = d.line[0].read(t);
            let dr = d.line[1].read(t);
            d.lp[0] += (dl - d.lp[0]) * lp_a;
            d.lp[1] += (dr - d.lp[1]) * lp_a;
            let (fl, fr) = (d.lp[0], d.lp[1]);
            if ping {
                let mono = 0.5 * (l[i] + r[i]);
                d.line[0].write(mono + fb * fr);
                d.line[1].write(fb * fl);
            } else {
                d.line[0].write(l[i] + fb * fl);
                d.line[1].write(r[i] + fb * fr);
            }
            l[i] += mix * dl;
            r[i] += mix * dr;
        }
    }

    fn reverb(&mut self, l: &mut [f32], r: &mut [f32], snap: &Snapshot, rate: f32) {
        let mix = snap[P::Rev_Mix as usize];
        if mix <= 0.0 {
            return;
        }
        let rv = &mut self.reverb;
        let size_target = 0.55 + snap[P::Rev_Size as usize] * 0.95;
        let size_coef = smooth_coef(0.25, rate);
        let rt60 = snap[P::Rev_Decay as usize].max(0.1);
        let damp = snap[P::Rev_Damp as usize];
        let damp_a = 1.0 - 0.88 * damp;
        let pre = snap[P::Rev_Pre as usize] * 0.001 * rate;
        let scale = rate / 1000.0;
        let mod_depth = 0.0007 * rate * 0.05 + 1.0;
        let norm = 1.0 / 8.0f32.sqrt();
        for i in 0..l.len() {
            let size = rv.size.tick(size_target, size_coef);
            rv.pre[0].write(l[i]);
            rv.pre[1].write(r[i]);
            let (in_l, in_r) = (rv.pre[0].read(pre.max(1.0)), rv.pre[1].read(pre.max(1.0)));
            let mut y = [0.0f32; 8];
            for n in 0..8 {
                let len = REVERB_MS[n] * scale * size;
                rv.phase[n] += (0.11 + 0.037 * n as f32) / rate;
                rv.phase[n] -= rv.phase[n].floor();
                let m = (TAU * rv.phase[n]).sin() * mod_depth;
                let tap = rv.line[n].read(len + m);
                rv.lp[n] += (tap - rv.lp[n]) * damp_a;
                // gain so that this line's signal falls 60 dB in `rt60` seconds
                let g = 10.0f32.powf(-3.0 * (len / rate) / rt60);
                y[n] = rv.lp[n] * g;
            }
            let wet_l = (y[0] + y[2] + y[4] + y[6]) * 0.35;
            let wet_r = (y[1] + y[3] + y[5] + y[7]) * 0.35;
            // Hadamard mixing (energy preserving)
            let mut h = y;
            let mut step = 1;
            while step < 8 {
                let mut k = 0;
                while k < 8 {
                    for j in k..k + step {
                        let (a, b) = (h[j], h[j + step]);
                        h[j] = a + b;
                        h[j + step] = a - b;
                    }
                    k += step * 2;
                }
                step *= 2;
            }
            for n in 0..8 {
                let inj = if n % 2 == 0 { in_l } else { in_r } * 0.35;
                rv.line[n].write((h[n] * norm + inj).clamp(-8.0, 8.0));
            }
            l[i] = l[i] * (1.0 - 0.4 * mix) + wet_l * mix * 1.4;
            r[i] = r[i] * (1.0 - 0.4 * mix) + wet_r * mix * 1.4;
        }
    }
}

impl Default for Fx {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::store::Params;
    use super::*;

    const FS: f32 = 48_000.0;

    fn run(fx: &mut Fx, snap: &Snapshot, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (input.to_vec(), input.to_vec());
        for (cl, cr) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
            fx.process(cl, cr, snap, FS);
        }
        (l, r)
    }

    fn impulse(n: usize) -> Vec<f32> {
        let mut v = vec![0.0; n];
        v[0] = 1.0;
        v
    }

    fn tone(n: usize) -> Vec<f32> {
        (0..n).map(|i| 0.4 * (TAU * 440.0 * i as f32 / FS).sin()).collect()
    }

    #[test]
    fn with_every_mix_at_zero_the_chain_is_transparent() {
        let p = Params::new();
        p.set(P::Drv_Type, 0.0);
        let x = tone(4096);
        let (l, r) = run(&mut Fx::new(), &p.snapshot(), &x);
        assert!(l.iter().zip(&x).all(|(a, b)| (a - b).abs() < 1e-6));
        assert!(r.iter().zip(&x).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn every_effect_stays_finite_and_bounded_at_its_extremes() {
        for kind in 1..5 {
            let p = Params::new();
            p.set(P::Drv_Type, kind as f32);
            p.set(P::Drv_Amt, 1.0);
            p.set(P::Cho_Mix, 1.0);
            p.set(P::Cho_Depth, 1.0);
            p.set(P::Ph_Mix, 1.0);
            p.set(P::Ph_Fb, 1.0);
            p.set(P::Dly_Mix, 1.0);
            p.set(P::Dly_Fb, 1.0);
            p.set(P::Rev_Mix, 1.0);
            p.set(P::Rev_Decay, 20.0);
            p.set(P::Rev_Size, 1.0);
            let x = tone(96_000);
            let (l, r) = run(&mut Fx::new(), &p.snapshot(), &x);
            assert!(l.iter().chain(&r).all(|s| s.is_finite() && s.abs() < 40.0), "drive type {kind}");
        }
    }

    #[test]
    fn the_delay_echoes_at_its_time_and_the_feedback_decays() {
        let p = Params::new();
        p.set(P::Dly_Mix, 1.0);
        p.set(P::Dly_Time, 100.0);
        p.set(P::Dly_Fb, 0.5);
        p.set(P::Dly_Tone, 1.0);
        p.set(P::Dly_Ping, 0.0);
        let mut fx = Fx::new();
        // let the time smoother settle on 100 ms
        let warm = vec![0.0; 48_000];
        run(&mut fx, &p.snapshot(), &warm);
        let (l, _) = run(&mut fx, &p.snapshot(), &impulse(24_000));
        let first = l.iter().enumerate().skip(100).max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map(|(i, _)| i).unwrap();
        assert!((first as f32 - 4800.0).abs() < 8.0, "first echo at {first}, wanted 4800");
        let second = l[6000..].iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map(|(i, v)| (i + 6000, *v)).unwrap();
        assert!((second.0 as f32 - 9600.0).abs() < 8.0, "second echo at {}", second.0);
        assert!(second.1.abs() < l[first].abs(), "the repeat is quieter");
    }

    #[test]
    fn the_reverb_tails_off_in_about_its_decay_time_and_never_grows() {
        let p = Params::new();
        p.set(P::Rev_Mix, 1.0);
        p.set(P::Rev_Decay, 2.0);
        p.set(P::Rev_Damp, 0.0);
        p.set(P::Rev_Size, 0.5);
        let mut fx = Fx::new();
        let (l, r) = run(&mut fx, &p.snapshot(), &impulse(48_000 * 6));
        let energy = |a: usize, b: usize| l[a..b].iter().chain(&r[a..b]).map(|s| s * s).sum::<f32>();
        let early = energy(2000, 12_000);
        let at_decay = energy(96_000, 105_000);
        let late = energy(240_000, 249_000);
        assert!(early > 1e-4, "it rings: {early}");
        assert!(at_decay < early * 1e-2, "after the decay time it has fallen away: {at_decay} vs {early}");
        assert!(late < at_decay.max(1e-12), "and keeps falling: {late} vs {at_decay}");
        assert!(l.iter().chain(&r).all(|s| s.is_finite() && s.abs() < 4.0));
    }

    #[test]
    fn the_reverb_is_stereo() {
        let p = Params::new();
        p.set(P::Rev_Mix, 1.0);
        let (l, r) = run(&mut Fx::new(), &p.snapshot(), &impulse(24_000));
        let diff: f32 = l.iter().zip(&r).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 0.01, "the two channels differ: {diff}");
    }

    #[test]
    fn the_chorus_and_phaser_change_the_sound() {
        let x = tone(24_000);
        let p = Params::new();
        p.set(P::Cho_Mix, 1.0);
        let (c, _) = run(&mut Fx::new(), &p.snapshot(), &x);
        assert!(c.iter().zip(&x).any(|(a, b)| (a - b).abs() > 0.05), "chorus");
        let p = Params::new();
        p.set(P::Ph_Mix, 1.0);
        let (ph, _) = run(&mut Fx::new(), &p.snapshot(), &x);
        assert!(ph.iter().zip(&x).any(|(a, b)| (a - b).abs() > 0.05), "phaser");
    }

    #[test]
    fn drive_adds_harmonics_and_crush_quantises() {
        let x = tone(8192);
        let p = Params::new();
        p.set(P::Drv_Type, 2.0);
        p.set(P::Drv_Amt, 1.0);
        let (hard, _) = run(&mut Fx::new(), &p.snapshot(), &x);
        let settled = &hard[2048..];
        let top = settled.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let flat = settled.iter().filter(|s| s.abs() > 0.95 * top).count();
        assert!(flat > 500, "hard clipping flattens the peaks: {flat}");
        p.set(P::Drv_Type, 4.0);
        let (crushed, _) = run(&mut Fx::new(), &p.snapshot(), &x);
        let mut distinct = crushed.clone();
        distinct.sort_by(|a, b| a.total_cmp(b));
        distinct.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        assert!(distinct.len() < 400, "bit crushing leaves few distinct levels: {}", distinct.len());
    }
}
