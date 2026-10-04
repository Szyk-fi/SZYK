//! Blocks ported from modules that ship in Cardinal (the open-source VCV
//! Rack distribution): ten that Portamax had no equivalent for, leaving out
//! Mutable Instruments, whose real code already runs here.
//!
//! "Ported" means the technique, not the source code. Nearly every module
//! in Cardinal is GPL-3 and this simulator is MIT, so copying their C++
//! would relicense the whole project. Each block is written from the
//! published algorithm the module is built on (or the hardware it models),
//! and its doc comment says where it differs from the original.
//!
//! Same real-time rules as the rest of the library: everything is
//! allocated in `new`, `tick` never allocates, every coefficient is
//! clamped so any input is stable.

use super::{fast_tanh, onepole_coef, rising, white, Ctx, Ring, MAX_IN};
use std::f32::consts::{PI, TAU};

/// One allpass diffuser step on a ring: H = (-g + z^-N) / (1 - g z^-N).
#[inline]
fn ap_step(r: &mut Ring, x: f32, delay: f32, g: f32) -> f32 {
    let d = r.read(delay);
    let w = x + g * d;
    r.push(w);
    d - g * w
}

#[inline]
fn wrap(p: &mut f32) {
    *p -= p.floor();
}

// ------------------------------------------------------------ plateau

/// Dattorro's plate (J. Dattorro, "Effect Design Part 1", JAES 1997), the
/// tank Valley's Plateau is built on: four input diffusers into a
/// figure-eight of two halves, each a modulated allpass, a delay, a damping
/// low-pass, a second allpass and a second delay, the end of each half
/// feeding the start of the other. Delay lengths and output taps are the
/// paper's (given at 29761 Hz) scaled to the sample rate and `size`.
/// Mono out: the paper's left-channel taps. Plateau adds stereo, tuned
/// diffusion and a tank filter pair on top of this; those are not here.
pub struct Plateau {
    pre: Ring,
    bw: f32,
    inap: [Ring; 4],
    map: [Ring; 2],
    d1: [Ring; 2],
    ap2: [Ring; 2],
    d2: [Ring; 2],
    damp: [f32; 2],
    lfo: f32,
    k: f32,
}

const PLATE_SR: f32 = 29_761.0;
const PLATE_IN: [f32; 4] = [142.0, 107.0, 379.0, 277.0];
const PLATE_IN_G: [f32; 4] = [0.75, 0.75, 0.625, 0.625];
/// Per half: modulated allpass, delay, allpass, delay.
const PLATE_TANK: [[f32; 4]; 2] = [[672.0, 4453.0, 1800.0, 3720.0], [908.0, 4217.0, 2656.0, 3163.0]];
pub const PLATE_MAX_SIZE: f32 = 1.5;

impl Plateau {
    pub fn new(sr: f32) -> Self {
        let k = sr / PLATE_SR;
        let line = |n: f32| Ring::new((n * k * PLATE_MAX_SIZE) as usize + 64);
        Self {
            pre: Ring::new((0.2 * sr) as usize + 8),
            bw: 0.0,
            inap: PLATE_IN.map(|n| Ring::new((n * k) as usize + 8)),
            map: [line(PLATE_TANK[0][0] + 32.0), line(PLATE_TANK[1][0] + 32.0)],
            d1: [line(PLATE_TANK[0][1]), line(PLATE_TANK[1][1])],
            ap2: [line(PLATE_TANK[0][2]), line(PLATE_TANK[1][2])],
            d2: [line(PLATE_TANK[0][3]), line(PLATE_TANK[1][3])],
            damp: [0.0; 2],
            lfo: 0.0,
            k,
        }
    }

    pub fn reset(&mut self) {
        self.pre.clear();
        for r in self.inap.iter_mut().chain(self.map.iter_mut()).chain(self.d1.iter_mut()).chain(self.ap2.iter_mut()).chain(self.d2.iter_mut()) {
            r.clear();
        }
        self.bw = 0.0;
        self.damp = [0.0; 2];
    }

    pub fn tick(&mut self, v: &[f32; MAX_IN], c: &Ctx) -> f32 {
        let d = v[1].clamp(0.0, 1.0);
        // curved so the musical middle of the knob gets most of its travel
        let g = (1.0 - (1.0 - d) * (1.0 - d) * 0.9).min(0.995);
        let s = self.k * v[2].clamp(0.5, PLATE_MAX_SIZE);
        let damp = v[3].clamp(0.0, 0.95);
        self.pre.push(v[0].clamp(-4.0, 4.0));
        let x = self.pre.read(v[4].clamp(0.0, 0.2) * c.sr);
        // the paper's "bandwidth" low-pass on the way in
        self.bw += (x - self.bw) * 0.7;
        let mut x = self.bw;
        for j in 0..4 {
            x = ap_step(&mut self.inap[j], x, PLATE_IN[j] * self.k, PLATE_IN_G[j]);
        }
        self.lfo += c.inv_sr;
        wrap(&mut self.lfo);
        let exc = v[5].clamp(0.0, 1.0) * 16.0 * self.k;
        // each half is fed by the end of the other one
        let fb = [self.d2[1].read(PLATE_TANK[1][3] * s), self.d2[0].read(PLATE_TANK[0][3] * s)];
        for h in 0..2 {
            let m = ((self.lfo + h as f32 * 0.25) * TAU).sin() * exc;
            let y = ap_step(&mut self.map[h], x + g * fb[h], PLATE_TANK[h][0] * s + m, -0.7);
            self.d1[h].push(y);
            let y = self.d1[h].read(PLATE_TANK[h][1] * s);
            self.damp[h] += (y - self.damp[h]) * (1.0 - damp);
            let y = ap_step(&mut self.ap2[h], self.damp[h] * g, PLATE_TANK[h][2] * s, 0.5);
            self.d2[h].push(y);
        }
        let t = |r: &Ring, n: f32| r.read(n * s);
        let y = t(&self.d1[1], 266.0) + t(&self.d1[1], 2974.0) - t(&self.ap2[1], 1913.0) + t(&self.d2[1], 1996.0)
            - t(&self.d1[0], 1990.0)
            - t(&self.ap2[0], 187.0)
            - t(&self.d2[0], 1066.0);
        y * 0.6
    }
}

// ------------------------------------------------------------- spring

/// A spring tank modelled the way Parker and Abel do it ("Spring
/// reverberation: a physical perspective", DAFx 2009; Välimäki, Parker and
/// Abel, JAES 2010): a feedback delay for the transit time with a chain of
/// first-order allpasses inside the loop. The allpasses delay low
/// frequencies more than high ones, so every bounce comes back as the
/// falling chirp that makes a spring sound like a spring, and it smears
/// more on each trip round. Befaco's Spring Reverb instead convolves with
/// a recorded spring; this is the parametric model, not that recording.
pub struct Spring {
    ring: Ring,
    x1: [f32; SPRING_STAGES],
    y1: [f32; SPRING_STAGES],
    lp: f32,
    dc_x: f32,
    dc_y: f32,
}

const SPRING_STAGES: usize = 24;

impl Spring {
    pub fn new(sr: f32) -> Self {
        Self { ring: Ring::new((0.1 * sr) as usize + 8), x1: [0.0; SPRING_STAGES], y1: [0.0; SPRING_STAGES], lp: 0.0, dc_x: 0.0, dc_y: 0.0 }
    }

    pub fn reset(&mut self) {
        self.ring.clear();
        self.x1 = [0.0; SPRING_STAGES];
        self.y1 = [0.0; SPRING_STAGES];
        self.lp = 0.0;
        self.dc_x = 0.0;
        self.dc_y = 0.0;
    }

    pub fn tick(&mut self, v: &[f32; MAX_IN], c: &Ctx) -> f32 {
        let a = 0.3 + v[3].clamp(0.0, 1.0) * 0.55;
        let mut y = self.ring.read(v[2].clamp(0.02, 0.1) * c.sr);
        // H = (-a + z^-1) / (1 - a z^-1): group delay (1+a)/(1-a) samples
        // at DC falling to (1-a)/(1+a) at Nyquist -- lows lag the highs
        for k in 0..SPRING_STAGES {
            let o = -a * y + self.x1[k] + a * self.y1[k];
            self.x1[k] = y;
            self.y1[k] = o;
            y = o;
        }
        let fc = 1500.0 * 6f32.powf(v[4].clamp(0.0, 1.0));
        self.lp += (y - self.lp) * onepole_coef(1.0 / (TAU * fc), c.inv_sr);
        let out = self.lp - self.dc_x + 0.995 * self.dc_y;
        self.dc_x = self.lp;
        self.dc_y = out;
        let fb = v[1].clamp(0.0, 1.0) * 0.9;
        self.ring.push((v[0].clamp(-4.0, 4.0) + out * fb).clamp(-8.0, 8.0));
        out
    }
}

// ------------------------------------------------------------- phaser

/// The classic phaser (as in Surge XT's Phaser): a chain of identical
/// first-order allpasses whose corner is swept by an LFO, mixed back with
/// the dry signal. Every pair of stages puts one notch where the chain's
/// phase reaches 180 degrees; feedback round the chain sharpens them.
/// Surge adds stereo spread and per-stage spacing; this is the mono core.
pub struct Phaser {
    stages: usize,
    x1: [f32; 12],
    y1: [f32; 12],
    phase: f32,
    last: f32,
}

impl Phaser {
    pub fn new(stages: usize) -> Self {
        Self { stages: stages.clamp(2, 12), x1: [0.0; 12], y1: [0.0; 12], phase: 0.0, last: 0.0 }
    }

    pub fn reset(&mut self) {
        self.x1 = [0.0; 12];
        self.y1 = [0.0; 12];
        self.last = 0.0;
    }

    pub fn tick(&mut self, v: &[f32; MAX_IN], c: &Ctx) -> f32 {
        self.phase += v[1].clamp(0.0, 20.0) * c.inv_sr;
        wrap(&mut self.phase);
        let lfo = (TAU * self.phase).sin();
        let f = (v[3].clamp(20.0, 12_000.0) * 2f32.powf(v[2].clamp(0.0, 1.0) * 3.0 * lfo)).clamp(20.0, c.sr * 0.45);
        let t = (PI * f * c.inv_sr).tan();
        // -90 degrees per stage at f
        let a = (t - 1.0) / (t + 1.0);
        let dry = v[0].clamp(-4.0, 4.0);
        let mut y = dry + v[4].clamp(-0.95, 0.95) * self.last;
        for k in 0..self.stages {
            let o = a * y + self.x1[k] - a * self.y1[k];
            self.x1[k] = y;
            self.y1[k] = o;
            y = o;
        }
        self.last = y.clamp(-4.0, 4.0);
        let mix = v[5].clamp(0.0, 1.0);
        dry * (1.0 - mix) + y * mix
    }
}

// ---------------------------------------------------------- freqshift

/// Single-sideband (Bode) frequency shifter, as in Surge XT's Frequency
/// Shifter: a Hilbert pair splits the signal into two copies 90 degrees
/// apart, and multiplying them by a quadrature oscillator and summing
/// cancels one sideband. The Hilbert pair is Olli Niemitalo's published
/// polyphase IIR design (two chains of four second-order allpasses, one
/// delayed a sample), which holds 90 degrees to within a fraction of a
/// degree from about 20 Hz to 0.98 of Nyquist.
pub struct FreqShift {
    sec: [[f32; 4]; 8],
    bdelay: f32,
    phase: f32,
}

const HILBERT_A: [f32; 4] = [0.692_387_8, 0.936_065_4, 0.988_229_5, 0.998_748_8];
const HILBERT_B: [f32; 4] = [0.402_192_1, 0.856_171_1, 0.972_290_9, 0.995_288_5];

impl FreqShift {
    pub fn new() -> Self {
        Self { sec: [[0.0; 4]; 8], bdelay: 0.0, phase: 0.0 }
    }

    pub fn reset(&mut self) {
        self.sec = [[0.0; 4]; 8];
        self.bdelay = 0.0;
    }

    #[inline]
    fn chain(sec: &mut [[f32; 4]], coefs: &[f32; 4], x: f32) -> f32 {
        // y[n] = a^2 (x[n] + y[n-2]) - x[n-2]; state = [x1, x2, y1, y2]
        let mut x = x;
        for (s, a) in sec.iter_mut().zip(coefs) {
            let y = a * a * (x + s[3]) - s[1];
            s[1] = s[0];
            s[0] = x;
            s[3] = s[2];
            s[2] = y;
            x = y;
        }
        x
    }

    pub fn tick(&mut self, v: &[f32; MAX_IN], c: &Ctx) -> f32 {
        let x = v[0].clamp(-4.0, 4.0);
        let (a, b) = self.sec.split_at_mut(4);
        let ia = Self::chain(a, &HILBERT_A, x);
        let q = Self::chain(b, &HILBERT_B, x);
        let i = self.bdelay;
        self.bdelay = ia;
        self.phase += v[1].clamp(-5000.0, 5000.0) * c.inv_sr;
        wrap(&mut self.phase);
        let (s, co) = (TAU * self.phase).sin_cos();
        // Niemitalo's design delays the A chain by one sample; the two are
        // then 90 degrees apart and this sum keeps the upper sideband
        // (the test pins both the direction and the rejection)
        let shifted = i * co + q * s;
        let mix = v[2].clamp(0.0, 1.0);
        x * (1.0 - mix) + shifted * mix
    }
}

// ------------------------------------------------------------- rotary

/// A rotary speaker cabinet (Surge XT's Rotary Speaker models the same
/// thing): an 800 Hz crossover sends the highs to a spinning horn and the
/// lows to a spinning drum. Each rotor gets Doppler (a delay that moves
/// with the mouth's distance from the listener) and tremolo (louder when
/// it faces you). Speeds are a Leslie 122's: chorale about 0.8 / 0.67 Hz,
/// tremolo about 6.7 / 5.9 Hz for horn / drum, and the light horn spins up
/// in about half a second while the heavy drum takes a few. Mono out, so
/// none of the cabinet's stereo miking; no cabinet resonance.
pub struct Rotary {
    horn: Ring,
    drum: Ring,
    lp1: f32,
    lp2: f32,
    horn_rate: f32,
    drum_rate: f32,
    hp: f32,
    dp: f32,
}

impl Rotary {
    pub fn new(sr: f32) -> Self {
        Self { horn: Ring::new((0.005 * sr) as usize + 8), drum: Ring::new((0.005 * sr) as usize + 8), lp1: 0.0, lp2: 0.0, horn_rate: 0.8, drum_rate: 0.67, hp: 0.0, dp: 0.25 }
    }

    pub fn reset(&mut self) {
        self.horn.clear();
        self.drum.clear();
        self.lp1 = 0.0;
        self.lp2 = 0.0;
    }

    pub fn rates(&self) -> (f32, f32) {
        (self.horn_rate, self.drum_rate)
    }

    pub fn tick(&mut self, v: &[f32; MAX_IN], c: &Ctx) -> f32 {
        let x = v[0].clamp(-4.0, 4.0);
        let speed = v[1].clamp(0.0, 1.0);
        let depth = v[2].clamp(0.0, 1.0);
        self.horn_rate += (0.8 + speed * 5.9 - self.horn_rate) * onepole_coef(0.5, c.inv_sr);
        self.drum_rate += (0.67 + speed * 5.23 - self.drum_rate) * onepole_coef(2.5, c.inv_sr);
        self.hp += self.horn_rate * c.inv_sr;
        wrap(&mut self.hp);
        self.dp += self.drum_rate * c.inv_sr;
        wrap(&mut self.dp);
        // two one-poles: a gentle crossover, as the 122's passive one is
        let g = onepole_coef(1.0 / (TAU * 800.0), c.inv_sr);
        self.lp1 += (x - self.lp1) * g;
        self.lp2 += (self.lp1 - self.lp2) * g;
        let low = self.lp2;
        let high = x - low;
        // horn mouth at about 15 cm: +-0.44 ms of path; the drum's baffle
        // moves less of the sound, so less Doppler, more of it is tremolo
        let sh = (TAU * self.hp).sin();
        let sd = (TAU * self.dp).sin();
        self.horn.push(high);
        self.drum.push(low);
        let h = self.horn.read((0.0006 + 0.00044 * depth * sh) * c.sr);
        let d = self.drum.read((0.0006 + 0.00015 * depth * sd) * c.sr);
        // nearest (shortest delay, sin = -1) is when it faces the listener
        let ah = 1.0 - 0.6 * depth * (1.0 + sh) * 0.5;
        let ad = 1.0 - 0.3 * depth * (1.0 + sd) * 0.5;
        let wet = h * ah + d * ad;
        let mix = v[3].clamp(0.0, 1.0);
        x * (1.0 - mix) + wet * mix
    }
}

// --------------------------------------------------------------- tape

/// A tape machine in the spirit of ChowDSP's ChowTape, but simpler: Chow
/// solves the Jiles-Atherton magnetic hysteresis equations, which this
/// does not. What is modelled: record pre-emphasis (highs boosted before
/// the saturating tape, cut again on playback, so highs saturate first,
/// which is why hot tape sounds dense and dark), a tanh tape curve, wow
/// (slow, drifting speed error) and flutter (fast capstan wobble) as a
/// modulated playback delay, and playback-head gap loss as a low-pass that
/// closes as the head wears. The 4 ms delay the wobble needs is real
/// latency.
pub struct Tape {
    ring: Ring,
    pre: f32,
    de: f32,
    head: [f32; 2],
    wow: f32,
    flut: f32,
    drift: f32,
}

impl Tape {
    pub fn new(sr: f32) -> Self {
        Self { ring: Ring::new((0.012 * sr) as usize + 8), pre: 0.0, de: 0.0, head: [0.0; 2], wow: 0.0, flut: 0.0, drift: 0.0 }
    }

    pub fn reset(&mut self) {
        self.ring.clear();
        self.pre = 0.0;
        self.de = 0.0;
        self.head = [0.0; 2];
        self.drift = 0.0;
    }

    pub fn tick(&mut self, v: &[f32; MAX_IN], c: &Ctx, rng: &mut u32) -> f32 {
        let x = v[0].clamp(-4.0, 4.0);
        let drive = v[1].clamp(0.5, 10.0);
        let g = onepole_coef(1.0 / (TAU * 3000.0), c.inv_sr);
        self.pre += (x - self.pre) * g;
        let emph = x + (x - self.pre); // about +6 dB above 3 kHz
        let sat = fast_tanh(emph * drive) / fast_tanh(drive);
        self.de += (sat - self.de) * g;
        let rec = 0.5 * (sat + self.de); // the inverse curve
        // wow ~0.6 Hz with a slow random drift; flutter ~7 Hz plus a
        // faster partial, as capstan and pinch-roller faults produce
        self.wow += 0.6 * c.inv_sr;
        wrap(&mut self.wow);
        self.flut += 7.1 * c.inv_sr;
        wrap(&mut self.flut);
        self.drift += (white(rng) - self.drift) * onepole_coef(0.8, c.inv_sr);
        let wow = v[2].clamp(0.0, 1.0) * 0.0015 * (0.7 * (TAU * self.wow).sin() + 6.0 * self.drift);
        let flutter = v[3].clamp(0.0, 1.0) * 0.00012 * ((TAU * self.flut).sin() + 0.4 * (TAU * 2.3 * self.flut).sin());
        self.ring.push(rec);
        let y = self.ring.read((0.004 + wow + flutter).clamp(0.0005, 0.011) * c.sr);
        let fc = 18_000.0 * (1.0 - v[4].clamp(0.0, 1.0) * 0.85);
        let h = onepole_coef(1.0 / (TAU * fc.min(c.sr * 0.45)), c.inv_sr);
        self.head[0] += (y - self.head[0]) * h;
        self.head[1] += (self.head[0] - self.head[1]) * h;
        self.head[1]
    }
}

// --------------------------------------------------------------- comp

/// Feed-forward compressor with a soft knee, built like Bogaudio's Pressor
/// (detector -> gain computer in dB -> attack/release on the gain
/// reduction), following Giannoulis, Massberg and Reiss, "Digital Dynamic
/// Range Compressor Design", JAES 2012: the gain is computed from the
/// instantaneous level and the gain reduction is what gets smoothed. The
/// detector can listen to the signal itself or to a sidechain.
pub struct Comp {
    gr: f32,
}

impl Comp {
    pub fn new() -> Self {
        Self { gr: 0.0 }
    }

    pub fn reset(&mut self) {
        self.gr = 0.0;
    }

    pub fn tick(&mut self, v: &[f32; MAX_IN], c: &Ctx) -> f32 {
        const KNEE: f32 = 6.0;
        let x = v[0].clamp(-8.0, 8.0);
        let key = v[2].clamp(0.0, 1.0);
        let det = (x.abs() * (1.0 - key) + v[1].clamp(-8.0, 8.0).abs() * key).max(1e-6);
        let over = 20.0 * det.log10() - v[3].clamp(-60.0, 0.0);
        let slope = 1.0 - 1.0 / v[4].clamp(1.0, 20.0);
        let target = if over <= -KNEE / 2.0 {
            0.0
        } else if over >= KNEE / 2.0 {
            over * slope
        } else {
            slope * (over + KNEE / 2.0).powi(2) / (2.0 * KNEE)
        };
        let t = if target > self.gr { v[5] } else { v[6] };
        self.gr += (target - self.gr) * onepole_coef(t.clamp(0.0001, 5.0), c.inv_sr);
        x * 10f32.powf((v[7].clamp(-24.0, 24.0) - self.gr) / 20.0)
    }
}

// --------------------------------------------------------------- kick

/// An analogue-style kick as Befaco's Kickall makes it: a sine whose pitch
/// starts high and falls exponentially to the body pitch, under an
/// exponential amplitude envelope, with a drive control that bends the
/// sine towards a square. The phase restarts on every hit, so each one
/// starts on the same click. Kickall's own envelope shapes and CV ranges
/// are not copied, only the structure.
pub struct Kick {
    phase: f32,
    amp: f32,
    penv: f32,
    prev: f32,
}

impl Kick {
    pub fn new() -> Self {
        Self { phase: 0.0, amp: 0.0, penv: 0.0, prev: 0.0 }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn tick(&mut self, v: &[f32; MAX_IN], c: &Ctx) -> f32 {
        if rising(&mut self.prev, v[0]) {
            self.phase = 0.0;
            self.amp = 1.0;
            self.penv = 1.0;
        }
        let f = (v[1].clamp(20.0, 400.0) * 2f32.powf(v[2].clamp(0.0, 1.0) * 4.0 * self.penv)).min(c.sr * 0.45);
        self.penv -= self.penv * onepole_coef(v[3].clamp(0.002, 0.5), c.inv_sr);
        // time constant decay/4.6: about -40 dB at `decay` seconds
        self.amp -= self.amp * onepole_coef(v[4].clamp(0.02, 4.0) / 4.6, c.inv_sr);
        let s = (TAU * self.phase).sin();
        self.phase += f * c.inv_sr;
        wrap(&mut self.phase);
        let k = 1.0 + v[5].clamp(0.0, 1.0) * 7.0;
        fast_tanh(s * k) / fast_tanh(k) * self.amp
    }
}

// --------------------------------------------------------------- walk

/// A bounded random walk, the idea behind Bogaudio's Walk: Brownian motion
/// (independent random steps scaled by the square root of the time step,
/// so `rate` is a real diffusion speed whatever the sample rate) that
/// reflects off +-range instead of sticking to it, with a jump trigger and
/// an output lag.
pub struct Walk {
    x: f32,
    y: f32,
    prev: f32,
}

impl Walk {
    pub fn new() -> Self {
        Self { x: 0.0, y: 0.0, prev: 0.0 }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn tick(&mut self, v: &[f32; MAX_IN], c: &Ctx, rng: &mut u32) -> f32 {
        let range = v[1].clamp(0.0, 10.0);
        if rising(&mut self.prev, v[2]) {
            self.x = white(rng) * range;
        }
        // white() is uniform in -1..1 (variance 1/3), so sqrt(3) makes
        // each step unit variance
        self.x += white(rng) * v[0].clamp(0.0, 20.0) * (3.0 * c.inv_sr).sqrt();
        if self.x > range {
            self.x = 2.0 * range - self.x;
        }
        if self.x < -range {
            self.x = -2.0 * range - self.x;
        }
        self.x = self.x.clamp(-range, range);
        self.y += (self.x - self.y) * onepole_coef(v[3].clamp(0.0001, 5.0), c.inv_sr);
        self.y
    }
}

// ----------------------------------------------------------------- eq

/// Three-band EQ (the layout of Bogaudio's EQ): RBJ cookbook low shelf,
/// peaking bell and high shelf biquads in series. Coefficients are only
/// recomputed when a setting moves.
pub struct Eq3 {
    bq: [Biquad; 3],
    key: [f32; 7],
}

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
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// kind 0 low shelf, 1 bell, 2 high shelf (shelves at slope S = 1).
    fn set(&mut self, kind: u8, f: f32, db: f32, q: f32, sr: f32) {
        let a = 10f32.powf(db / 40.0);
        let w = TAU * f / sr;
        let (sn, cs) = w.sin_cos();
        let (b0, b1, b2, a0, a1, a2) = match kind {
            1 => {
                let al = sn / (2.0 * q);
                (1.0 + al * a, -2.0 * cs, 1.0 - al * a, 1.0 + al / a, -2.0 * cs, 1.0 - al / a)
            }
            _ => {
                let al2 = 2.0 * a.sqrt() * sn / 2.0 * std::f32::consts::SQRT_2;
                if kind == 0 {
                    (
                        a * ((a + 1.0) - (a - 1.0) * cs + al2),
                        2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                        a * ((a + 1.0) - (a - 1.0) * cs - al2),
                        (a + 1.0) + (a - 1.0) * cs + al2,
                        -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                        (a + 1.0) + (a - 1.0) * cs - al2,
                    )
                } else {
                    (
                        a * ((a + 1.0) + (a - 1.0) * cs + al2),
                        -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                        a * ((a + 1.0) + (a - 1.0) * cs - al2),
                        (a + 1.0) - (a - 1.0) * cs + al2,
                        2.0 * ((a - 1.0) - (a + 1.0) * cs),
                        (a + 1.0) - (a - 1.0) * cs - al2,
                    )
                }
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }
}

impl Eq3 {
    pub fn new() -> Self {
        Self { bq: [Biquad::default(); 3], key: [f32::NAN; 7] }
    }

    pub fn reset(&mut self) {
        for b in self.bq.iter_mut() {
            b.z1 = 0.0;
            b.z2 = 0.0;
        }
    }

    pub fn tick(&mut self, v: &[f32; MAX_IN], c: &Ctx) -> f32 {
        let top = c.sr * 0.45;
        let key = [
            v[1].clamp(-24.0, 24.0),
            v[2].clamp(-24.0, 24.0),
            v[3].clamp(-24.0, 24.0),
            v[4].clamp(20.0, top),
            v[5].clamp(20.0, top),
            v[6].clamp(20.0, top),
            v[7].clamp(0.1, 10.0),
        ];
        // (NaN in the stored key forces the first computation)
        if key.iter().zip(self.key.iter()).any(|(a, b)| !((a - b).abs() <= 1e-4 * a.abs().max(1e-3))) {
            self.bq[0].set(0, key[3], key[0], 0.707, c.sr);
            self.bq[1].set(1, key[4], key[1], key[6], c.sr);
            self.bq[2].set(2, key[5], key[2], 0.707, c.sr);
            self.key = key;
        }
        let mut y = v[0].clamp(-8.0, 8.0);
        for b in self.bq.iter_mut() {
            y = b.run(y);
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Block, Opts};
    use super::*;

    const SR: f32 = 48_000.0;

    fn ctx() -> Ctx {
        Ctx { sr: SR, inv_sr: 1.0 / SR, beat: 0.0 }
    }

    fn block(kind: &str, opts: &'static [(&'static str, &'static str)]) -> Block {
        let o = Opts { get: Box::leak(Box::new(move |k: &str| opts.iter().find(|p| p.0 == k).map(|p| p.1.to_string()))), values: vec![] };
        Block::new(kind, &o, SR).unwrap()
    }

    fn ins(v: &[f32]) -> [f32; MAX_IN] {
        let mut a = [0.0; MAX_IN];
        a[..v.len()].copy_from_slice(v);
        a
    }

    /// Run `n` samples; `f(n)` gives the inputs at sample n.
    fn run(b: &mut Block, n: usize, f: impl Fn(usize) -> [f32; MAX_IN]) -> Vec<f32> {
        let mut rng = 11;
        (0..n).map(|k| b.tick(&f(k), &ctx(), &mut rng)).collect()
    }

    fn sine(hz: f32, k: usize) -> f32 {
        (TAU * hz * k as f32 / SR).sin()
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|v| v * v).sum()
    }

    /// Amplitude at one frequency (single DFT bin; a unit sine reads 1).
    fn amp(x: &[f32], hz: f32) -> f32 {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, s) in x.iter().enumerate() {
            let ph = TAU * hz * n as f32 / SR;
            re += s * ph.cos();
            im += s * ph.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f32
    }

    /// Steady-state gain of a block at one frequency (input on input 0).
    fn gain(kind: &str, opts: &'static [(&'static str, &'static str)], rest: &[f32], hz: f32) -> f32 {
        let mut b = block(kind, opts);
        let rest = rest.to_vec();
        let out = run(&mut b, 24_000, |k| {
            let mut a = ins(&rest);
            a[0] = sine(hz, k) * 0.25;
            a
        });
        amp(&out[12_000..], hz) / 0.25
    }

    fn impulse(k: usize) -> f32 {
        if k == 0 {
            1.0
        } else {
            0.0
        }
    }

    #[test]
    fn plateau_waits_for_its_predelay_and_decay_sets_the_tail() {
        let tail = |decay: f32| {
            let mut b = block("plateau", &[]);
            let out = run(&mut b, 96_000, |k| ins(&[impulse(k), decay, 1.0, 0.2, 0.05, 0.0]));
            (out[..2000].iter().all(|v| v.abs() < 1e-9), energy(&out[48_000..]), out.iter().all(|v| v.is_finite()))
        };
        let (quiet_first, long, ok) = tail(0.9);
        assert!(quiet_first, "nothing before the 50 ms pre-delay");
        assert!(ok);
        let (_, short, _) = tail(0.1);
        assert!(long > 1e-4, "a long plate still rings after 1 s ({long})");
        assert!(short < long * 1e-3, "decay shortens it ({short} vs {long})");
    }

    #[test]
    fn spring_echoes_at_its_transit_time_and_its_lows_arrive_late() {
        // one trip round the tank: decay 0
        let mut b = block("spring", &[]);
        let out = run(&mut b, 9600, |k| ins(&[impulse(k), 0.0, 0.05, 1.0, 1.0]));
        let first = out.iter().position(|v| v.abs() > 1e-4).unwrap();
        assert!((2350..2500).contains(&first), "echo after 50 ms, got {first}");
        // energy centroid in time of the lows vs the highs
        let mut lp = 0.0;
        let g = onepole_coef(1.0 / (TAU * 300.0), 1.0 / SR);
        let (mut lo, mut hi) = ((0.0, 0.0), (0.0, 0.0));
        for (n, x) in out.iter().enumerate() {
            lp += (x - lp) * g;
            let h = x - lp;
            lo = (lo.0 + n as f32 * lp * lp, lo.1 + lp * lp);
            hi = (hi.0 + n as f32 * h * h, hi.1 + h * h);
        }
        let (tl, th) = (lo.0 / lo.1, hi.0 / hi.1);
        assert!(tl > th + 48.0, "lows lag the highs by over 1 ms (lows {tl}, highs {th})");
        // with decay up it keeps bouncing
        let mut b = block("spring", &[]);
        let long = run(&mut b, 48_000, |k| ins(&[impulse(k), 0.8, 0.05, 0.6, 0.5]));
        assert!(energy(&long[24_000..]) > 1e-6);
    }

    #[test]
    fn a_static_phaser_cuts_notches_and_fb_deepens_nothing_unstable() {
        let g: Vec<f32> = (0..80).map(|j| gain("phaser", &[("stages", "4")], &[0.0, 0.0, 0.0, 1000.0, 0.0, 0.5], 100.0 * 1.06f32.powi(j))).collect();
        let (lo, hi) = g.iter().fold((f32::MAX, 0.0f32), |(a, b), x| (a.min(*x), b.max(*x)));
        assert!(hi > 0.9, "passes the rest ({hi})");
        assert!(lo < 0.1, "4 stages put a deep notch in ({lo})");
        let mut b = block("phaser", &[("stages", "12")]);
        let out = run(&mut b, 48_000, |k| ins(&[sine(330.0, k), 2.0, 1.0, 800.0, 0.95, 0.5]));
        assert!(out.iter().all(|v| v.is_finite() && v.abs() < 20.0));
    }

    #[test]
    fn the_frequency_shifter_moves_a_tone_up_or_down_by_hz() {
        for (shift, want, not) in [(100.0, 1100.0, 900.0), (-100.0, 900.0, 1100.0), (250.0, 450.0, -50.0)] {
            let mut b = block("freqshift", &[]);
            let src = if shift == 250.0 { 200.0 } else { 1000.0 };
            let out = run(&mut b, 24_000, |k| ins(&[sine(src, k) * 0.5, shift, 1.0]));
            let w = amp(&out[4800..], want);
            assert!(w > 0.4, "{src} + {shift} lands on {want} ({w})");
            if not > 0.0 {
                let n = amp(&out[4800..], not);
                assert!(n < w * 0.03, "the other sideband is cancelled ({n} vs {w})");
            }
            assert!(amp(&out[4800..], src) < w * 0.03, "the original is gone");
        }
    }

    #[test]
    fn the_rotary_spins_up_slowly_and_its_tremolo_follows_the_rotor() {
        let mut r = Rotary::new(SR);
        let mut rng = 1;
        let _ = &mut rng;
        let x = ins(&[0.0, 1.0, 0.7, 1.0]);
        for _ in 0..(SR as usize / 4) {
            r.tick(&x, &ctx());
        }
        let (h, d) = r.rates();
        assert!(h > 2.0 && h < 6.6, "the horn is on its way after 0.25 s ({h})");
        assert!(d < 2.0, "the drum lags behind it ({d})");
        // after spin-up: the level of a 3 kHz tone (horn only) swings ~6.7 times a second
        let envelope_rate = |speed: f32| {
            let mut b = block("rotary", &[]);
            let out = run(&mut b, 6 * 48_000, |k| ins(&[sine(3000.0, k), speed, 1.0, 1.0]));
            let w: Vec<f32> = out[4 * 48_000..].chunks(480).map(|c| (energy(c) / 480.0).sqrt()).collect();
            let mean = w.iter().sum::<f32>() / w.len() as f32;
            let crossings = w.windows(2).filter(|p| (p[0] - mean) * (p[1] - mean) < 0.0).count();
            crossings as f32 / 2.0 / 2.0 // two crossings per cycle, over 2 s
        };
        let fast = envelope_rate(1.0);
        let slow = envelope_rate(0.0);
        assert!((fast - 6.7).abs() < 1.0, "tremolo horn ~6.7 Hz ({fast})");
        assert!(slow < 1.5, "chorale horn ~0.8 Hz ({slow})");
    }

    #[test]
    fn tape_wow_bends_the_pitch_and_drive_saturates() {
        // spread of zero-crossing periods of a 1 kHz tone
        let wobble = |wow: f32| {
            let mut b = block("tape", &[]);
            let out = run(&mut b, 96_000, |k| ins(&[sine(1000.0, k) * 0.3, 0.5, wow, 0.0, 0.0]));
            let mut last = 0.0;
            let mut periods = vec![];
            for n in 24_000..96_000 {
                if out[n - 1] <= 0.0 && out[n] > 0.0 {
                    let t = n as f32 - out[n] / (out[n] - out[n - 1]);
                    if last > 0.0 {
                        periods.push(t - last);
                    }
                    last = t;
                }
            }
            let (lo, hi) = periods.iter().fold((f32::MAX, 0.0f32), |(a, b), p| (a.min(*p), b.max(*p)));
            hi - lo
        };
        let steady = wobble(0.0);
        let wowed = wobble(1.0);
        assert!(steady < 0.05, "no wow, steady pitch ({steady} samples)");
        assert!(wowed > 0.1, "wow bends it ({wowed} samples)");
        let third = |drive: f32| {
            let mut b = block("tape", &[]);
            let out = run(&mut b, 24_000, |k| ins(&[sine(200.0, k) * 0.5, drive, 0.0, 0.0, 0.0]));
            amp(&out[4800..], 600.0) / amp(&out[4800..], 200.0)
        };
        assert!(third(8.0) > third(0.5) * 10.0, "hot tape adds odd harmonics");
    }

    #[test]
    fn the_compressor_holds_its_ratio_and_ducks_from_the_sidechain() {
        // a 0 dBFS-peak sine is ~-3 dB by peak detector? no: the detector is
        // instantaneous, so judge on a square (constant |x|)
        let level = |x_amp: f32, side: f32, key: f32| {
            let mut b = block("comp", &[]);
            let out = run(&mut b, 24_000, |k| ins(&[if (k / 50) % 2 == 0 { x_amp } else { -x_amp }, side, key, -20.0, 4.0, 0.001, 0.1, 0.0]));
            out[20_000..].iter().fold(0.0f32, |m, v| m.max(v.abs()))
        };
        // 0 dB in, 20 dB over a -20 threshold at 4:1 -> 5 dB over -> -15 dB out
        let loud = level(1.0, 0.0, 0.0);
        assert!((20.0 * loud.log10() + 15.0).abs() < 0.3, "0 dB in comes out at -15 dB ({} dB)", 20.0 * loud.log10());
        let quiet = level(0.05, 0.0, 0.0);
        assert!((quiet - 0.05).abs() < 0.001, "below threshold untouched ({quiet})");
        let ducked = level(0.05, 1.0, 1.0);
        assert!(ducked < 0.05 * 0.2, "a loud sidechain ducks a quiet signal ({ducked})");
    }

    #[test]
    fn the_kick_sweeps_down_to_its_body_pitch_and_decays() {
        let hit = |decay: f32| {
            let mut b = block("kick", &[]);
            run(&mut b, 48_000, |k| ins(&[if k < 10 { 1.0 } else { 0.0 }, 100.0, 1.0, 0.02, decay, 0.0]))
        };
        let x = hit(1.0);
        // first 5 ms: still far above the body pitch
        let early = x[..240].windows(2).filter(|p| p[0] <= 0.0 && p[1] > 0.0).count();
        assert!(early >= 3, "starts high ({early} cycles in 5 ms)");
        // 0.2..0.4 s: on 100 Hz
        let cycles = x[9600..19_200].windows(2).filter(|p| p[0] <= 0.0 && p[1] > 0.0).count();
        assert!((19..=21).contains(&cycles), "settles at 100 Hz ({cycles} cycles in 0.2 s)");
        let short = hit(0.1);
        assert!(energy(&short[9600..]) < energy(&x[9600..]) * 1e-3, "decay sets the length");
        assert!(x.iter().all(|v| v.abs() <= 1.0001));
    }

    #[test]
    fn walk_wanders_inside_its_range_and_rate_sets_the_speed() {
        let path = |rate: f32| {
            let mut b = block("walk", &[]);
            run(&mut b, 480_000, |_| ins(&[rate, 0.5, 0.0, 0.001]))
        };
        let x = path(2.0);
        assert!(x.iter().all(|v| v.abs() <= 0.5 + 1e-4), "stays inside +-0.5");
        let (lo, hi) = x.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!(hi - lo > 0.6, "covers the range over 10 s ({lo}..{hi})");
        // mean step over 0.1 s scales with rate
        let travel = |x: &[f32]| x.chunks(4800).map(|c| (c[c.len() - 1] - c[0]).abs()).sum::<f32>();
        assert!(travel(&path(0.05)) < travel(&x) * 0.2);
        let still = path(0.0);
        assert!(still.iter().all(|v| *v == 0.0));
        let mut b = block("walk", &[]);
        let j = run(&mut b, 4800, |k| ins(&[0.0, 1.0, if k > 100 { 1.0 } else { 0.0 }, 0.001]));
        assert!(j[4799].abs() > 1e-3, "jump moves it ({})", j[4799]);
    }

    #[test]
    fn the_eq_bands_boost_and_cut_where_they_say() {
        let db = |g: f32| 20.0 * g.log10();
        let low = gain("eq", &[], &[0.0, 12.0, 0.0, 0.0, 200.0, 1000.0, 4000.0, 0.7], 20.0);
        assert!((db(low) - 12.0).abs() < 0.5, "+12 dB low shelf at 20 Hz: {} dB", db(low));
        let mid = gain("eq", &[], &[0.0, 0.0, 6.0, 0.0, 200.0, 1000.0, 4000.0, 0.7], 1000.0);
        assert!((db(mid) - 6.0).abs() < 0.3, "+6 dB bell at its centre: {} dB", db(mid));
        let high = gain("eq", &[], &[0.0, 0.0, 0.0, -12.0, 200.0, 1000.0, 4000.0, 0.7], 16_000.0);
        assert!((db(high) + 12.0).abs() < 0.6, "-12 dB high shelf at 16 kHz: {} dB", db(high));
        let flat = gain("eq", &[], &[0.0, 0.0, 0.0, 0.0, 200.0, 1000.0, 4000.0, 0.7], 1000.0);
        assert!((db(flat)).abs() < 0.05, "flat when all gains are 0");
    }
}
