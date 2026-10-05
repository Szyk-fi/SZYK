//! Analog drum-machine voices for the Sequencer: the Roland TR-808 and TR-909
//! sounds, built from how those circuits make them rather than from samples.
//!
//! What "real" means here, said plainly. The 808's voices are a handful of
//! well-documented circuit ideas and this models those ideas directly:
//! - kick, toms: a bridged-T resonator rung by a trigger pulse -- a decaying sine
//!   that starts sharp and settles, so a sine with a fast downward pitch sweep
//!   and an exponential amplitude decay, softly saturated;
//! - snare: two such resonators (about 180 and 330 Hz) plus high-passed noise;
//! - hats and cowbell: six (hats) or two (cowbell) square-wave oscillators at the
//!   808's own frequencies, summed, band-passed and high-passed, with a fast
//!   exponential VCA -- the hat set is 205.3, 304.4, 369.6, 522.7, 540 and
//!   800 Hz, and the cowbell is 540 and 800 Hz;
//! - clap: band-passed noise with a sawtooth-style envelope that restarts a few
//!   times (the "multiple hands" bursts) before one longer tail.
//! The 909's analog voices (kick, snare, toms, clap, rim) follow the same
//! construction with its different tunings, sweeps and drive. **Its hi-hats and
//! cymbals, though, were 6-bit samples in ROM on the real machine**, and no sample
//! is shipped here, so the 909 hats are the six-square metal through a brighter,
//! tighter filter and envelope -- in the right spirit, not the same data.
//! Frequencies, decay times and sweeps are set by ear against what is known of the
//! hardware, not measured from a schematic: it is a model of the circuits'
//! behaviour, not a SPICE simulation.

use std::f32::consts::TAU;

/// Voice names, by `Kind`. The Sequencer's own four basic voices come first.
pub const KIND_NAMES: [&str; 14] = [
    "808 Kick", "909 Kick", "808 Snare", "909 Snare", "808 Clap", "909 Clap", "808 Hat C", "808 Hat O", "909 Hat C", "909 Hat O", "808 Tom", "909 Tom", "808 Rim", "808 Cowbell",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Kick808,
    Kick909,
    Snare808,
    Snare909,
    Clap808,
    Clap909,
    HatC808,
    HatO808,
    HatC909,
    HatO909,
    Tom808,
    Tom909,
    Rim808,
    Cowbell808,
}

const ALL: [Kind; 14] = [
    Kind::Kick808, Kind::Kick909, Kind::Snare808, Kind::Snare909, Kind::Clap808, Kind::Clap909, Kind::HatC808, Kind::HatO808, Kind::HatC909, Kind::HatO909, Kind::Tom808, Kind::Tom909,
    Kind::Rim808, Kind::Cowbell808,
];

impl Kind {
    pub fn from_index(i: usize) -> Kind {
        ALL[i % ALL.len()]
    }

    /// An open hat is cut off by a closed hat.
    pub fn is_open_hat(self) -> bool {
        matches!(self, Kind::HatO808 | Kind::HatO909)
    }

    pub fn is_closed_hat(self) -> bool {
        matches!(self, Kind::HatC808 | Kind::HatC909)
    }
}

/// The 808 hi-hat's six square oscillators.
const METAL_808: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

/// A two-pole filter (RBJ cookbook), used as a band-pass or high-pass.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn band_pass(fc: f32, q: f32, sr: f32) -> Self {
        let w = TAU * fc.min(sr * 0.45) / sr;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self { b0: alpha / a0, b1: 0.0, b2: -alpha / a0, a1: -2.0 * w.cos() / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0 }
    }

    fn high_pass(fc: f32, q: f32, sr: f32) -> Self {
        let w = TAU * fc.min(sr * 0.45) / sr;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        let c = w.cos();
        Self { b0: (1.0 + c) / 2.0 / a0, b1: -(1.0 + c) / a0, b2: (1.0 + c) / 2.0 / a0, a1: -2.0 * c / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0 }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// One drum voice. `trigger` restarts it; `render` makes one sample.
pub struct Voice {
    t: f32,
    live: bool,
    choked: f32,
    phases: [f32; 6],
    rng: u32,
    band: Biquad,
    high: Biquad,
    high2: Biquad,
    /// What `band`/`high` were last configured for.
    set_for: Option<(Kind, u32)>,
}

impl Voice {
    pub fn new(seed: u32) -> Self {
        Self {
            t: 0.0,
            live: false,
            choked: 1.0,
            phases: [0.0; 6],
            rng: 0x9E3779B9 ^ (seed.wrapping_mul(0x85EBCA6B) | 1),
            band: Biquad::default(),
            high: Biquad::default(),
            high2: Biquad::default(),
            set_for: None,
        }
    }

    fn noise(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    pub fn trigger(&mut self) {
        self.t = 0.0;
        self.live = true;
        self.choked = 1.0;
        self.phases = [0.0; 6];
        self.band.z1 = 0.0;
        self.band.z2 = 0.0;
        self.high.z1 = 0.0;
        self.high.z2 = 0.0;
        self.high2.z1 = 0.0;
        self.high2.z2 = 0.0;
    }

    /// A closed hat cutting off an open one: a few ms of fade, not a click.
    pub fn choke(&mut self) {
        if self.live {
            self.choked = 0.999;
        }
    }

    fn configure(&mut self, kind: Kind, sr: f32) {
        if self.set_for == Some((kind, sr as u32)) {
            return;
        }
        self.set_for = Some((kind, sr as u32));
        match kind {
            Kind::Snare808 => self.high = Biquad::high_pass(1800.0, 0.7, sr),
            Kind::Snare909 => {
                self.high = Biquad::high_pass(2200.0, 0.7, sr);
                self.band = Biquad::band_pass(6500.0, 0.6, sr);
            }
            Kind::Clap808 => self.band = Biquad::band_pass(1000.0, 2.2, sr),
            Kind::Clap909 => self.band = Biquad::band_pass(1250.0, 1.8, sr),
            Kind::HatC808 | Kind::HatO808 => {
                self.band = Biquad::band_pass(3400.0, 1.4, sr);
                self.high = Biquad::high_pass(7000.0, 0.7, sr);
            }
            Kind::HatC909 | Kind::HatO909 => {
                self.band = Biquad::band_pass(5200.0, 1.2, sr);
                self.high = Biquad::high_pass(9000.0, 0.8, sr);
            }
            Kind::Rim808 => self.high = Biquad::high_pass(1200.0, 0.7, sr),
            Kind::Cowbell808 => self.band = Biquad::band_pass(2640.0, 1.0, sr),
            _ => {}
        }
    }

    /// `decay` is the track's 0..1 Decay knob, mapped per voice to a range
    /// that suits it; `semitones` retunes every oscillator.
    pub fn render(&mut self, kind: Kind, decay: f32, semitones: i32, sr: f32) -> f32 {
        if !self.live {
            return 0.0;
        }
        self.configure(kind, sr);
        let pm = 2f32.powf(semitones as f32 / 12.0);
        let t = self.t;
        self.t += 1.0 / sr;
        let dk = decay.clamp(0.0, 1.0);
        let env = |tau: f32| (-t / tau).exp();
        let mut out = match kind {
            // A bridged-T kick: sine, sharp pitch fall, long exponential decay, mild saturation.
            Kind::Kick808 => {
                let f = 50.0 * pm * (1.0 + 2.2 * env(0.010));
                self.phases[0] = (self.phases[0] + f / sr).fract();
                let click = self.noise() * 0.25 * env(0.0015);
                ((self.phases[0] * TAU).sin() * env(0.10 + dk * 0.9) * 1.1).tanh() + click
            }
            // The 909 kick: a faster, deeper sweep with a bright attack and more drive.
            Kind::Kick909 => {
                let f = 47.0 * pm * (1.0 + 4.5 * env(0.007));
                self.phases[0] = (self.phases[0] + f / sr).fract();
                let click = (self.noise() * 0.5 + 0.5 * (t * 3400.0 * TAU).sin()) * 0.3 * env(0.0015);
                ((self.phases[0] * TAU).sin() * env(0.05 + dk * 0.45) * 2.4).tanh() * 0.85 + click
            }
            Kind::Snare808 => {
                let (f1, f2) = (180.0 * pm, 330.0 * pm);
                self.phases[0] = (self.phases[0] + f1 * (1.0 + 0.15 * env(0.008)) / sr).fract();
                self.phases[1] = (self.phases[1] + f2 * (1.0 + 0.10 * env(0.008)) / sr).fract();
                let tone = (self.phases[0] * TAU).sin() * env(0.075) * 0.55 + (self.phases[1] * TAU).sin() * env(0.05) * 0.35;
                let n = self.noise();
                let snap = self.high.process(n) * env(0.06 + dk * 0.25);
                tone + snap * 0.75
            }
            // The 909 snare: triangle-ish shells, brighter noise, hit harder.
            Kind::Snare909 => {
                let (f1, f2) = (185.0 * pm, 330.0 * pm);
                self.phases[0] = (self.phases[0] + f1 * (1.0 + 0.25 * env(0.006)) / sr).fract();
                self.phases[1] = (self.phases[1] + f2 * (1.0 + 0.12 * env(0.006)) / sr).fract();
                let tri = |p: f32| 4.0 * (p - 0.5).abs() - 1.0;
                let tone = tri(self.phases[0]) * env(0.05) * 0.6 + tri(self.phases[1]) * env(0.035) * 0.3;
                let n = self.noise();
                let hi = self.high.process(n);
                let snap = self.band.process(hi) * 2.0 * env(0.09 + dk * 0.2) + hi * 0.3 * env(0.03);
                (tone + snap * 0.85).tanh() * 0.95
            }
            Kind::Clap808 | Kind::Clap909 => {
                // Three (808) or four (909) quick bursts, each a fast saw-down envelope,
                // then one longer tail.
                let (bursts, gap, tail) = if kind == Kind::Clap808 { (3, 0.0105, 0.07 + dk * 0.3) } else { (4, 0.0085, 0.06 + dk * 0.25) };
                let burst_end = gap * bursts as f32;
                let level = if t < burst_end { (-(t % gap) / 0.0028).exp() } else { (-(t - burst_end) / tail).exp() * 0.8 };
                let n = self.noise();
                self.band.process(n) * level * 2.4
            }
            Kind::HatC808 | Kind::HatO808 | Kind::HatC909 | Kind::HatO909 => {
                let mut sum = 0.0;
                for (i, f) in METAL_808.iter().enumerate() {
                    self.phases[i] = (self.phases[i] + f * pm / sr).fract();
                    sum += if self.phases[i] < 0.5 { 1.0 } else { -1.0 };
                }
                let metal = self.high.process(self.band.process(sum / 6.0));
                let tau = match kind {
                    Kind::HatC808 => 0.018 + dk * 0.05,
                    Kind::HatO808 => 0.12 + dk * 0.55,
                    Kind::HatC909 => 0.012 + dk * 0.04,
                    _ => 0.09 + dk * 0.4,
                };
                let gain = if matches!(kind, Kind::HatC909 | Kind::HatO909) { 6.0 } else { 5.0 };
                metal * env(tau) * gain
            }
            // Toms: a bridged-T pair of sine decays, with a pitch sweep; the 909's is deeper.
            Kind::Tom808 => {
                let f = 100.0 * pm * (1.0 + 0.7 * env(0.03));
                self.phases[0] = (self.phases[0] + f / sr).fract();
                (self.phases[0] * TAU).sin() * env(0.12 + dk * 0.45) * 0.9
            }
            Kind::Tom909 => {
                let f = 110.0 * pm * (1.0 + 1.1 * env(0.025));
                self.phases[0] = (self.phases[0] + f / sr).fract();
                let click = self.noise() * 0.2 * env(0.002);
                ((self.phases[0] * TAU).sin() * env(0.10 + dk * 0.4) * 1.6).tanh() * 0.85 + click
            }
            // The rim shot: two short, high resonances and a click.
            Kind::Rim808 => {
                self.phases[0] = (self.phases[0] + 1667.0 * pm / sr).fract();
                self.phases[1] = (self.phases[1] + 455.0 * pm / sr).fract();
                let ring = (self.phases[0] * TAU).sin() * 0.5 + (self.phases[1] * TAU).sin() * 0.5;
                let n = self.noise();
                (ring * env(0.012 + dk * 0.02) + self.high.process(n) * 0.5 * env(0.002)) * 0.9
            }
            // The cowbell: two squares a fifth-ish apart, band-passed, with a quick
            // knock on a longer tail.
            Kind::Cowbell808 => {
                self.phases[0] = (self.phases[0] + 540.0 * pm / sr).fract();
                self.phases[1] = (self.phases[1] + 800.0 * pm / sr).fract();
                let sq = |p: f32| if p < 0.5 { 1.0 } else { -1.0 };
                let tones = self.band.process((sq(self.phases[0]) + sq(self.phases[1])) * 0.5);
                tones * (0.65 * env(0.010) + 0.35 * env(0.18 + dk * 0.35)) * 1.6
            }
        };
        if self.choked < 1.0 {
            self.choked *= 0.99;
            out *= self.choked;
        }
        // Done once everything has died away (the longest voices ring about 2 s).
        if self.t > 2.5 || (self.choked < 1.0 && self.choked < 0.001) {
            self.live = false;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn hit(kind: Kind, decay: f32, semis: i32, seconds: f32) -> Vec<f32> {
        let mut v = Voice::new(1);
        v.trigger();
        (0..(seconds * SR) as usize).map(|_| v.render(kind, decay, semis, SR)).collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// Zero crossings per second: a rough pitch / brightness measure.
    fn crossings_per_s(x: &[f32]) -> f32 {
        let n = x.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        n as f32 / (x.len() as f32 / SR)
    }

    #[test]
    fn every_voice_sounds_stays_bounded_and_dies_away() {
        for (i, kind) in ALL.iter().enumerate() {
            for decay in [0.0, 0.5, 1.0] {
                let x = hit(*kind, decay, 0, 3.0);
                assert!(x.iter().all(|v| v.is_finite() && v.abs() < 2.5), "{}: finite and bounded", KIND_NAMES[i]);
                assert!(rms(&x[..4800]) > 0.02, "{} (decay {decay}) is audible: {}", KIND_NAMES[i], rms(&x[..4800]));
                assert!(rms(&x[x.len() - 4800..]) < 0.01, "{} (decay {decay}) dies away", KIND_NAMES[i]);
            }
        }
    }

    #[test]
    fn kicks_are_low_hats_are_bright() {
        // After the pitch sweep has settled, a kick rings near its fundamental.
        let kick = hit(Kind::Kick808, 0.5, 0, 0.4);
        let tail = &kick[(0.05 * SR) as usize..(0.30 * SR) as usize];
        let f = crossings_per_s(tail) / 2.0;
        assert!((40.0..75.0).contains(&f), "808 kick settles near 50 Hz: {f}");
        let hat = hit(Kind::HatC808, 0.5, 0, 0.1);
        assert!(crossings_per_s(&hat) / 2.0 > 4000.0, "a hat is mostly high frequencies: {}", crossings_per_s(&hat) / 2.0);
        // Retuning moves the pitch by the interval.
        let up = hit(Kind::Kick808, 0.5, 12, 0.4);
        let fu = crossings_per_s(&up[(0.05 * SR) as usize..(0.30 * SR) as usize]) / 2.0;
        assert!((fu / f - 2.0).abs() < 0.2, "an octave up doubles it: {f} -> {fu}");
    }

    #[test]
    fn the_decay_knob_lengthens_the_voices_that_have_one() {
        for kind in [Kind::Kick808, Kind::Snare808, Kind::HatO808, Kind::Tom808, Kind::Clap808] {
            let short = hit(kind, 0.0, 0, 1.5);
            let long = hit(kind, 1.0, 0, 1.5);
            let at = (0.25 * SR) as usize;
            assert!(rms(&long[at..at + 4800]) > rms(&short[at..at + 4800]) * 1.5 + 1e-6, "{} rings longer with more decay", KIND_NAMES[ALL.iter().position(|k| *k == kind).unwrap()]);
        }
    }

    #[test]
    fn an_open_hat_rings_longer_than_a_closed_one_and_a_closed_hat_cuts_it() {
        let open = hit(Kind::HatO808, 0.5, 0, 1.0);
        let closed = hit(Kind::HatC808, 0.5, 0, 1.0);
        assert!(rms(&open[(0.2 * SR) as usize..(0.3 * SR) as usize]) > rms(&closed[(0.2 * SR) as usize..(0.3 * SR) as usize]) * 4.0);

        let mut v = Voice::new(2);
        v.trigger();
        for _ in 0..4800 {
            v.render(Kind::HatO808, 1.0, 0, SR);
        }
        v.choke();
        let after: Vec<f32> = (0..4800).map(|_| v.render(Kind::HatO808, 1.0, 0, SR)).collect();
        assert!(rms(&after[2400..]) < 0.01, "choked within a few milliseconds: {}", rms(&after[2400..]));
    }

    /// The clap is bursts, then a tail: more than one separate peak early on.
    #[test]
    fn a_clap_is_several_bursts_before_its_tail() {
        let x = hit(Kind::Clap808, 0.2, 0, 0.1);
        let env: Vec<f32> = x.chunks(48).map(|c| c.iter().fold(0.0f32, |m, v| m.max(v.abs()))).collect();
        let mut peaks = 0;
        for i in 2..env.len().min(40) - 2 {
            if env[i] > env[i - 2] * 1.3 && env[i] > env[i + 2] * 1.3 && env[i] > 0.1 {
                peaks += 1;
            }
        }
        assert!(peaks >= 2, "bursts show up as separate peaks: {peaks}");
    }
}
