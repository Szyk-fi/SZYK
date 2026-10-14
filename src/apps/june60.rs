//! June-60: a six-voice Juno-style polysynth, written from scratch in Rust.
//!
//! Why not port an open-source Juno? The two real ones (Ultramaster KR-106 and
//! the Juno 6 VSTi) are GPL, and Portamax is MIT, so their code can't be linked
//! or copied in (the same rule `docs/ADDING_AN_APP.md` applies to the GPL
//! Shruthi/Grids firmware). This app is therefore a clean-room build of the
//! architecture every Juno-6/60/106 shares, not a port of any of them, and it
//! doesn't claim to match any particular unit sample for sample.
//!
//! What is modelled, voice by voice (six voices):
//! - **DCO**: one band-limited (PolyBLEP) oscillator per voice with a ramp, a
//!   pulse whose width is manual or swept by the LFO, a square sub-oscillator
//!   an octave down, and white noise, each on its own level slider, in 16'/8'/4'.
//! - **HPF**: the four-position high-pass: bass boost, flat, and two corners.
//!   The corner frequencies and the boost amount are approximations.
//! - **VCF**: a four-pole, 24 dB/oct resonant low-pass in the topology of the
//!   IR3109 chip (four cascaded one-pole stages with global feedback), solved
//!   zero-delay (Zavalishin's TPT ladder) so resonance doesn't detune the
//!   cutoff, with a tanh at the input for overdrive. Cutoff follows the
//!   envelope (either polarity), the LFO and the keyboard. At full resonance
//!   it sings by itself, as the chip does.
//! - **ADSR**: one envelope per voice shared by filter and amplifier, with
//!   analog RC-style segments (the attack aims past full scale, so it bends
//!   like a charging capacitor), or a plain gate for the VCA.
//! - **LFO**: one triangle with delay (fade-in), to pitch, pulse width and
//!   filter. The mod wheel adds pitch modulation immediately, the way the
//!   Juno's manual LFO trigger does.
//! - **Chorus**: the two-line BBD-style stereo chorus: I and II are slow
//!   sweeps and I+II is the fast, shallow one. The rates and the delay range
//!   come from published analyses of the Juno-60 chorus as remembered, not
//!   from a measurement of a unit.
//! - Poly (voices rotate) and Unison (all six on one note, a few cents apart),
//!   portamento, bend range of two semitones. Velocity is ignored, as on
//!   the original keyboards.
//!
//! The panel is drawn after the 106: dark slate face, white silkscreen, a
//! stripe under the logo, a red LED patch number and a row of vertical sliders.
//! The twelve patches are original; none is a Roland factory sound.

use super::mi_kit::{pad_note, Keys, NoteQueue};
use crate::{
    app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12},
    util::AtomicF32,
};
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}, text::Text};
use std::f32::consts::PI;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "June-60";
const VOICES: usize = 6;

// ---------------------------------------------------------------- controls

// Ordered by importance: the first sixteen are the Controls layer's pads and
// 0-7 are the knobs' hero pairs. Everything is stored 0..1; stepped controls
// are `idx / (steps - 1)`.
const C_PRESET: usize = 0;
const C_CUTOFF: usize = 1;
const C_RES: usize = 2;
const C_ENV: usize = 3;
const C_ATTACK: usize = 4;
const C_DECAY: usize = 5;
const C_SUSTAIN: usize = 6;
const C_RELEASE: usize = 7;
const C_SAW: usize = 8;
const C_PULSE: usize = 9;
const C_SUB: usize = 10;
const C_NOISE: usize = 11;
const C_CHORUS: usize = 12;
const C_LFO_RATE: usize = 13;
const C_LFO_DCO: usize = 14;
const C_LEVEL: usize = 15;
const C_HPF: usize = 16;
const C_PWM: usize = 17;
const C_PWM_MODE: usize = 18;
const C_LFO_DELAY: usize = 19;
const C_LFO_VCF: usize = 20;
const C_KYBD: usize = 21;
const C_ENV_POL: usize = 22;
const C_VCA: usize = 23;
const C_RANGE: usize = 24;
const C_VOICES: usize = 25;
const C_PORTA: usize = 26;
const C_BEND: usize = 27;
const C_MOD: usize = 28;
const C_OCTAVE: usize = 29;
const N: usize = 30;

struct Ctl {
    name: &'static str,
    /// 0 for a continuous control, else the number of positions.
    steps: usize,
    default: f32,
}

const fn c(name: &'static str, steps: usize, default: f32) -> Ctl {
    Ctl { name, steps, default }
}

const CTLS: [Ctl; N] = [
    c("Patch", 12, 0.0),
    c("Cutoff", 0, 0.7),
    c("Resonance", 0, 0.0),
    c("Env Amount", 0, 0.0),
    c("Attack", 0, 0.02),
    c("Decay", 0, 0.45),
    c("Sustain", 0, 1.0),
    c("Release", 0, 0.25),
    c("Saw", 0, 0.8),
    c("Pulse", 0, 0.0),
    c("Sub", 0, 0.0),
    c("Noise", 0, 0.0),
    c("Chorus", 4, 1.0 / 3.0),
    c("LFO Rate", 0, 0.3),
    c("LFO Pitch", 0, 0.0),
    c("Level", 0, 0.8),
    c("HPF", 4, 1.0 / 3.0),
    c("PWM", 0, 0.5),
    c("PW Mode", 2, 0.0),
    c("LFO Delay", 0, 0.0),
    c("LFO Filter", 0, 0.0),
    c("Key Follow", 0, 0.5),
    c("Env Polarity", 2, 0.0),
    c("VCA", 2, 0.0),
    c("Range", 3, 0.5),
    c("Voices", 2, 0.0),
    c("Portamento", 0, 0.0),
    c("Bend", 0, 0.5),
    c("Mod Wheel", 0, 0.0),
    c("Octave", 7, 0.5),
];

const CHORUS_NAMES: [&str; 4] = ["OFF", "I", "II", "I+II"];
const HPF_NAMES: [&str; 4] = ["BOOST", "FLAT", "240 Hz", "720 Hz"];

/// Modulation inputs: (control they add to). Each registers as
/// "June-60: <control name>".
const CV: [usize; 8] = [C_CUTOFF, C_RES, C_LEVEL, C_LFO_RATE, C_ENV, C_MOD, C_BEND, C_PWM];
const CV_NAMES: [&str; 8] = ["Cutoff", "Resonance", "Level", "LFO Rate", "Env", "Mod Wheel", "Bend", "PWM"];

fn cv_slot(control: usize) -> Option<usize> {
    CV.iter().position(|&x| x == control)
}

fn step_norm(idx: usize, steps: usize) -> f32 {
    if steps < 2 { 0.0 } else { idx as f32 / (steps - 1) as f32 }
}

fn norm_step(v: f32, steps: usize) -> usize {
    if steps < 2 { 0 } else { (v.clamp(0.0, 1.0) * (steps - 1) as f32).round() as usize }
}

// ---------------------------------------------------------------- patches

const CH_OFF: f32 = 0.0;
const CH_I: f32 = 1.0 / 3.0;
const CH_II: f32 = 2.0 / 3.0;
const CH_BOTH: f32 = 1.0;
const HPF_240: f32 = 2.0 / 3.0;
const R16: f32 = 0.0;
#[cfg(test)]
const R4: f32 = 1.0;

type PatchDef = (&'static str, &'static [(usize, f32)]);

/// Original patches. Anything not listed is the control's default.
const PATCHES: [PatchDef; 12] = [
    ("Init Saw", &[]),
    ("Brass", &[(C_SAW, 0.75), (C_PULSE, 0.45), (C_PWM, 0.25), (C_CUTOFF, 0.42), (C_RES, 0.15), (C_ENV, 0.55), (C_ATTACK, 0.22), (C_DECAY, 0.45), (C_SUSTAIN, 0.6), (C_RELEASE, 0.25), (C_CHORUS, CH_I)]),
    ("Strings", &[(C_SAW, 0.85), (C_PULSE, 0.35), (C_PWM_MODE, 1.0), (C_PWM, 0.45), (C_LFO_RATE, 0.22), (C_CUTOFF, 0.62), (C_ENV, 0.15), (C_ATTACK, 0.55), (C_DECAY, 0.5), (C_SUSTAIN, 0.85), (C_RELEASE, 0.55), (C_CHORUS, CH_II)]),
    ("Warm Pad", &[(C_SAW, 0.6), (C_PULSE, 0.5), (C_SUB, 0.25), (C_PWM_MODE, 1.0), (C_PWM, 0.35), (C_LFO_RATE, 0.15), (C_CUTOFF, 0.5), (C_RES, 0.1), (C_ENV, 0.25), (C_ATTACK, 0.65), (C_DECAY, 0.6), (C_SUSTAIN, 0.8), (C_RELEASE, 0.7), (C_CHORUS, CH_II)]),
    ("Sub Bass", &[(C_SAW, 0.0), (C_PULSE, 0.45), (C_SUB, 0.95), (C_RANGE, R16), (C_CUTOFF, 0.34), (C_RES, 0.2), (C_ENV, 0.45), (C_ATTACK, 0.0), (C_DECAY, 0.38), (C_SUSTAIN, 0.25), (C_RELEASE, 0.15), (C_CHORUS, CH_OFF)]),
    ("Pluck", &[(C_SAW, 0.3), (C_PULSE, 0.8), (C_PWM, 0.35), (C_CUTOFF, 0.3), (C_RES, 0.25), (C_ENV, 0.7), (C_ATTACK, 0.0), (C_DECAY, 0.32), (C_SUSTAIN, 0.0), (C_RELEASE, 0.3), (C_CHORUS, CH_I)]),
    ("Fat Lead", &[(C_SAW, 1.0), (C_SUB, 0.35), (C_VOICES, 1.0), (C_PORTA, 0.22), (C_CUTOFF, 0.55), (C_RES, 0.3), (C_ENV, 0.3), (C_ATTACK, 0.0), (C_DECAY, 0.5), (C_SUSTAIN, 0.8), (C_RELEASE, 0.2), (C_LFO_RATE, 0.5), (C_LFO_DELAY, 0.45), (C_LFO_DCO, 0.2), (C_CHORUS, CH_I)]),
    ("Wobble", &[(C_SAW, 0.9), (C_SUB, 0.5), (C_CUTOFF, 0.3), (C_RES, 0.55), (C_LFO_VCF, 0.75), (C_LFO_RATE, 0.45), (C_ENV, 0.2), (C_SUSTAIN, 1.0), (C_RELEASE, 0.2), (C_CHORUS, CH_OFF)]),
    ("Organ", &[(C_SAW, 0.0), (C_PULSE, 1.0), (C_PWM, 0.0), (C_SUB, 0.8), (C_CUTOFF, 0.88), (C_VCA, 1.0), (C_ATTACK, 0.0), (C_RELEASE, 0.1), (C_KYBD, 0.8), (C_CHORUS, CH_II)]),
    ("Sweep", &[(C_SAW, 0.8), (C_PULSE, 0.4), (C_CUTOFF, 0.08), (C_RES, 0.6), (C_ENV, 1.0), (C_ATTACK, 0.62), (C_DECAY, 0.62), (C_SUSTAIN, 0.3), (C_RELEASE, 0.5), (C_CHORUS, CH_II)]),
    ("Wind", &[(C_SAW, 0.0), (C_NOISE, 0.85), (C_CUTOFF, 0.4), (C_RES, 0.55), (C_LFO_VCF, 0.35), (C_LFO_RATE, 0.2), (C_ENV, 0.3), (C_ATTACK, 0.6), (C_SUSTAIN, 1.0), (C_RELEASE, 0.7), (C_CHORUS, CH_BOTH)]),
    ("E-Piano", &[(C_SAW, 0.0), (C_PULSE, 0.75), (C_PWM, 0.3), (C_SUB, 0.3), (C_HPF, HPF_240), (C_CUTOFF, 0.5), (C_ENV, 0.4), (C_ATTACK, 0.0), (C_DECAY, 0.56), (C_SUSTAIN, 0.18), (C_RELEASE, 0.36), (C_CHORUS, CH_I)]),
];

/// Controls a patch change leaves alone: they belong to the player, not the
/// sound.
fn patch_keeps(i: usize) -> bool {
    matches!(i, C_PRESET | C_LEVEL | C_BEND | C_MOD | C_OCTAVE)
}

// ---------------------------------------------------------------- shared state

struct Shared {
    vals: [AtomicF32; N],
    cv: [Arc<AtomicF32>; 8],
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    peak: AtomicF32,
    voices: AtomicUsize,
}

impl Shared {
    /// A control's value with its modulation input added.
    fn eff(&self, i: usize) -> f32 {
        let mut v = self.vals[i].get();
        if let Some(s) = cv_slot(i) {
            v += self.cv[s].get();
        }
        v.clamp(0.0, 1.0)
    }
}

/// Everything the audio thread reads once per block.
#[derive(Clone, Copy, Debug)]
struct Patch {
    saw: f32,
    pulse: f32,
    sub: f32,
    noise: f32,
    pwm: f32,
    pwm_lfo: bool,
    range_st: f32,
    octave: i32,
    hpf: usize,
    cutoff: f32,
    res: f32,
    env_amt: f32,
    env_neg: bool,
    lfo_vcf: f32,
    kybd: f32,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    vca_gate: bool,
    lfo_hz: f32,
    lfo_delay_s: f32,
    lfo_dco: f32,
    chorus: usize,
    unison: bool,
    porta_tau: f32,
    level: f32,
    bend: f32,
    wheel: f32,
}

impl Patch {
    fn read(p: &Shared) -> Patch {
        let e = |i| p.eff(i);
        let porta = e(C_PORTA);
        Patch {
            saw: e(C_SAW),
            pulse: e(C_PULSE),
            sub: e(C_SUB),
            noise: e(C_NOISE),
            pwm: e(C_PWM),
            pwm_lfo: norm_step(e(C_PWM_MODE), 2) == 1,
            range_st: [-12.0, 0.0, 12.0][norm_step(e(C_RANGE), 3)],
            octave: norm_step(e(C_OCTAVE), 7) as i32 - 3,
            hpf: norm_step(e(C_HPF), 4),
            cutoff: e(C_CUTOFF),
            res: e(C_RES),
            env_amt: e(C_ENV),
            env_neg: norm_step(e(C_ENV_POL), 2) == 1,
            lfo_vcf: e(C_LFO_VCF),
            kybd: e(C_KYBD),
            attack: e(C_ATTACK),
            decay: e(C_DECAY),
            sustain: e(C_SUSTAIN),
            release: e(C_RELEASE),
            vca_gate: norm_step(e(C_VCA), 2) == 1,
            // 0.2 Hz to 30 Hz.
            lfo_hz: 0.2 * 150.0f32.powf(e(C_LFO_RATE)),
            lfo_delay_s: e(C_LFO_DELAY) * 3.0,
            lfo_dco: e(C_LFO_DCO),
            chorus: norm_step(e(C_CHORUS), 4),
            unison: norm_step(e(C_VOICES), 2) == 1,
            // Squared so most of the travel is the useful short glides.
            porta_tau: porta * porta * 2.5,
            level: e(C_LEVEL),
            bend: e(C_BEND) * 2.0 - 1.0,
            wheel: e(C_MOD),
        }
    }
}

// ---------------------------------------------------------------- DSP

fn poly_blep(t: f32, dt: f32) -> f32 {
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

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Release,
}

/// Per-block constants, so the per-sample loop does no `exp`/`powf`.
struct Block {
    sr: f32,
    ca: f32,
    cd: f32,
    cr: f32,
    gate_c: f32,
    porta_c: f32,
    lfo_inc: f32,
    sub_hpf_g: f32,
    boost_g: f32,
}

/// One-pole smoothing coefficient for a time constant.
fn coef(tau: f32, sr: f32) -> f32 {
    1.0 - (-1.0 / (sr * tau.max(1e-5))).exp()
}

impl Block {
    fn new(p: &Patch, sr: f32) -> Block {
        // The Juno's segments run from 1.5 ms to about 3 s (attack) and 12 s
        // (decay, release), exponentially across the slider.
        let t_attack = 0.0015 * 2000.0f32.powf(p.attack);
        let t_decay = 0.0015 * 8000.0f32.powf(p.decay);
        let t_release = 0.0015 * 8000.0f32.powf(p.release);
        Block {
            sr,
            // The attack aims at 1.3 and stops at 1.0, like an RC charge cut
            // off at the comparator: time to 1.0 is tau * ln(1.3 / 0.3).
            ca: coef(t_attack / 1.466, sr),
            // 4.6 time constants take a decay or release to within 1%.
            cd: coef(t_decay / 4.6, sr),
            cr: coef(t_release / 4.6, sr),
            gate_c: coef(0.004, sr),
            porta_c: if p.porta_tau < 0.002 { 1.0 } else { coef(p.porta_tau, sr) },
            lfo_inc: p.lfo_hz / sr,
            sub_hpf_g: 1.0 - (-2.0 * PI * [0.0, 0.0, 240.0, 720.0][p.hpf] / sr).exp(),
            boost_g: 1.0 - (-2.0 * PI * 110.0 / sr).exp(),
        }
    }
}

#[derive(Clone)]
struct Voice {
    note: f32,
    target: f32,
    detune: f32,
    gate: bool,
    age: u64,
    stage: Stage,
    env: f32,
    gate_amp: f32,
    ph: f32,
    flip: f32,
    hp: f32,
    lad: [f32; 4],
}

impl Voice {
    fn new() -> Voice {
        Voice { note: 60.0, target: 60.0, detune: 0.0, gate: false, age: 0, stage: Stage::Idle, env: 0.0, gate_amp: 0.0, ph: 0.0, flip: 0.0, hp: 0.0, lad: [0.0; 4] }
    }

    fn idle(&self) -> bool {
        self.stage == Stage::Idle && self.gate_amp < 1e-4
    }

    fn start(&mut self, note: f32, age: u64) {
        self.target = note;
        self.gate = true;
        self.age = age;
        self.stage = Stage::Attack;
    }

    fn release(&mut self) {
        self.gate = false;
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    /// The four-stage ladder, solved without a unit delay in the feedback loop.
    fn ladder(&mut self, x: f32, g: f32, k: f32) -> f32 {
        let gg = g / (1.0 + g);
        let a = 1.0 - gg;
        let g2 = gg * gg;
        let g3 = g2 * gg;
        let g4 = g3 * gg;
        let sg = g3 * self.lad[0] * a + g2 * self.lad[1] * a + gg * self.lad[2] * a + self.lad[3] * a;
        let u = ((x - k * sg) / (1.0 + k * g4)).tanh();
        let mut input = u;
        for s in self.lad.iter_mut() {
            let v = (input - *s) * gg;
            let y = v + *s;
            *s = y + v;
            input = y;
        }
        input
    }

    fn render(&mut self, p: &Patch, b: &Block, lfo: f32, lfo_gain: f32, noise: f32) -> f32 {
        // Envelope.
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.env += (1.3 - self.env) * b.ca;
                if self.env >= 1.0 {
                    self.env = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => self.env += (p.sustain - self.env) * b.cd,
            Stage::Release => {
                self.env -= self.env * b.cr;
                if self.env < 1e-4 {
                    self.env = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.gate_amp += ((self.gate as u8 as f32) - self.gate_amp) * if self.gate { b.gate_c } else { b.gate_c * 0.5 };
        if self.idle() {
            return 0.0;
        }

        // Pitch.
        self.note += (self.target - self.note) * b.porta_c;
        let vib = lfo * (p.lfo_dco * lfo_gain + p.wheel);
        let st = self.note + p.range_st + p.octave as f32 * 12.0 + p.bend * 2.0 + vib + self.detune;
        let freq = 440.0 * ((st - 69.0) / 12.0).exp2();
        let dt = (freq / b.sr).clamp(1e-5, 0.45);

        // Oscillators.
        let t = self.ph;
        let saw = 2.0 * t - 1.0 - poly_blep(t, dt);
        let w = if p.pwm_lfo { 0.5 + 0.45 * p.pwm * lfo } else { 0.5 - 0.45 * p.pwm }.clamp(0.05, 0.95);
        let mut pulse = if t < w { 1.0 } else { -1.0 };
        pulse += poly_blep(t, dt) - poly_blep((t + 1.0 - w).fract(), dt);
        let ts = (t + self.flip) * 0.5;
        let dts = dt * 0.5;
        let mut sub = if ts < 0.5 { 1.0 } else { -1.0 };
        sub += poly_blep(ts, dts) - poly_blep((ts + 0.5).fract(), dts);
        self.ph += dt;
        if self.ph >= 1.0 {
            self.ph -= 1.0;
            self.flip = 1.0 - self.flip;
        }
        let mut x = (saw * p.saw + pulse * p.pulse * 0.8 + sub * p.sub + noise * p.noise) * 0.5;

        // High-pass filter: boost, flat, 240 Hz, 720 Hz.
        match p.hpf {
            0 => {
                self.hp += (x - self.hp) * b.boost_g;
                x += self.hp * 0.7;
            }
            1 => {}
            _ => {
                self.hp += (x - self.hp) * b.sub_hpf_g;
                x -= self.hp;
            }
        }

        // Low-pass filter.
        let env = if p.env_neg { -self.env } else { self.env };
        let note_hz_oct = (self.note + p.octave as f32 * 12.0 - 60.0) / 12.0;
        let octaves = env * p.env_amt * 6.0 + lfo * lfo_gain * p.lfo_vcf * 2.5 + p.kybd * note_hz_oct;
        let fc = (20.0 * 1000.0f32.powf(p.cutoff) * octaves.exp2()).clamp(20.0, (0.45 * b.sr).min(18_000.0));
        let g = (PI * fc / b.sr).tan();
        // The chip sustains oscillation a little above k = 4; the tiny
        // noise term gives a silent filter something to start from.
        let k = p.res * 4.3;
        let y = self.ladder(x + noise * 1e-6, g, k);

        let amp = if p.vca_gate { self.gate_amp } else { self.env };
        y * amp
    }
}

/// The Juno chorus: two taps of a modulated delay, opposite in phase, over
/// a band-limited signal (a BBD's clock-rate filtering, here a one-pole).
struct Chorus {
    buf: Vec<f32>,
    pos: usize,
    phase: f32,
    wet: f32,
    lp: [f32; 2],
}

impl Chorus {
    fn new() -> Chorus {
        Chorus { buf: vec![0.0; 8192], pos: 0, phase: 0.0, wet: 0.0, lp: [0.0; 2] }
    }

    /// (rate Hz, centre ms, depth ms). I and II sweep 1.66-5.35 ms; I+II is
    /// the fast shallow mode.
    fn mode(m: usize) -> (f32, f32, f32) {
        match m {
            1 => (0.513, 3.505, 1.845),
            2 => (0.863, 3.505, 1.845),
            _ => (9.75, 3.505, 0.55),
        }
    }

    fn process(&mut self, x: f32, mode: usize, sr: f32) -> (f32, f32) {
        let mask = self.buf.len() - 1;
        self.buf[self.pos] = x;
        let on = (mode > 0) as u8 as f32;
        self.wet += (on - self.wet) * coef(0.02, sr);
        let (rate, centre, depth) = Chorus::mode(mode.max(1));
        self.phase = (self.phase + rate / sr).fract();
        let tri = 1.0 - 4.0 * (self.phase - 0.5).abs();
        let mut out = [0.0f32; 2];
        for (side, o) in out.iter_mut().enumerate() {
            let m = if side == 0 { tri } else { -tri };
            let d = ((centre + depth * m) * 0.001 * sr).clamp(1.0, (mask - 2) as f32);
            let r = self.pos as f32 - d;
            let i0 = r.floor();
            let frac = r - i0;
            let a = self.buf[(i0 as isize).rem_euclid(mask as isize + 1) as usize];
            let bb = self.buf[(i0 as isize + 1).rem_euclid(mask as isize + 1) as usize];
            let tap = a + (bb - a) * frac;
            self.lp[side] += (tap - self.lp[side]) * 0.55;
            *o = self.lp[side];
        }
        self.pos = (self.pos + 1) & mask;
        let dry = x * (1.0 - 0.3 * self.wet);
        (dry + out[0] * 0.7 * self.wet, dry + out[1] * 0.7 * self.wet)
    }
}

const UNISON_CENTS: [f32; VOICES] = [-6.0, -3.6, -1.2, 1.2, 3.6, 6.0];

struct Engine {
    voices: Vec<Voice>,
    sr: f32,
    lfo_ph: f32,
    lfo_t: f32,
    held: [u8; 16],
    n_held: usize,
    rr: usize,
    age: u64,
    rng: u32,
    chorus: Chorus,
}

impl Engine {
    fn new() -> Engine {
        Engine { voices: vec![Voice::new(); VOICES], sr: 48_000.0, lfo_ph: 0.0, lfo_t: 0.0, held: [0; 16], n_held: 0, rr: 0, age: 0, rng: 0x1234_5678, chorus: Chorus::new() }
    }

    fn note_on(&mut self, note: u8, p: &Patch) {
        if self.n_held == 0 {
            self.lfo_t = 0.0;
        }
        self.held[..self.n_held].iter().position(|&n| n == note).map(|i| {
            self.held.copy_within(i + 1..self.n_held, i);
            self.n_held -= 1;
        });
        if self.n_held < self.held.len() {
            self.held[self.n_held] = note;
            self.n_held += 1;
        }
        self.age += 1;
        let age = self.age;
        let n = note as f32;
        if p.unison {
            for (i, v) in self.voices.iter_mut().enumerate() {
                v.detune = UNISON_CENTS[i] / 100.0;
                v.start(n, age);
            }
            return;
        }
        let pick = (0..VOICES)
            .map(|k| (self.rr + k) % VOICES)
            .find(|&i| self.voices[i].gate && self.voices[i].target == n)
            .or_else(|| (0..VOICES).map(|k| (self.rr + k) % VOICES).find(|&i| self.voices[i].idle()))
            .or_else(|| (0..VOICES).filter(|&i| !self.voices[i].gate).min_by_key(|&i| self.voices[i].age))
            .unwrap_or_else(|| (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap_or(0));
        self.rr = (pick + 1) % VOICES;
        let v = &mut self.voices[pick];
        v.detune = 0.0;
        v.start(n, age);
    }

    fn note_off(&mut self, note: u8, p: &Patch) {
        if let Some(i) = self.held[..self.n_held].iter().position(|&n| n == note) {
            self.held.copy_within(i + 1..self.n_held, i);
            self.n_held -= 1;
        }
        if p.unison {
            if self.n_held > 0 {
                let back = self.held[self.n_held - 1] as f32;
                for v in self.voices.iter_mut() {
                    v.target = back;
                }
            } else {
                self.voices.iter_mut().for_each(Voice::release);
            }
            return;
        }
        for v in self.voices.iter_mut() {
            if v.gate && v.target == note as f32 {
                v.release();
            }
        }
    }

    fn active(&self) -> usize {
        self.voices.iter().filter(|v| !v.idle()).count()
    }

    fn render(&mut self, p: &Patch, b: &Block) -> (f32, f32) {
        self.lfo_ph = (self.lfo_ph + b.lfo_inc).fract();
        let lfo = 1.0 - 4.0 * (self.lfo_ph - 0.5).abs();
        self.lfo_t += 1.0 / b.sr;
        let lfo_gain = if p.lfo_delay_s < 0.005 { 1.0 } else { (self.lfo_t / p.lfo_delay_s).min(1.0) };
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        let noise = self.rng as i32 as f32 / i32::MAX as f32;
        let mut sum = 0.0;
        for v in self.voices.iter_mut() {
            sum += v.render(p, b, lfo, lfo_gain, noise);
        }
        self.chorus.process(sum, p.chorus, b.sr)
    }
}

/// Voices sum well past 1.0 on a full chord; bend smoothly instead of
/// clipping, so a thick pad saturates rather than crackles.
fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.6;
    let a = x.abs();
    if a <= KNEE { x } else { x.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh()) }
}

struct Processor {
    p: Arc<Shared>,
    notes: Arc<NoteQueue>,
    engine: Engine,
    master_prev: f32,
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.0);
        if channels == 0 || !rate.is_finite() || rate < 1000.0 {
            return;
        }
        self.engine.sr = rate;
        let patch = Patch::read(&self.p);
        while let Some(e) = self.notes.pop() {
            if e.velocity > 0 {
                self.engine.note_on(e.note, &patch);
            } else {
                self.engine.note_off(e.note, &patch);
            }
        }
        let block = Block::new(&patch, rate);
        let master = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0) * patch.level * patch.level * 2.0;
        let frames = out.len() / channels;
        let from = self.master_prev;
        let step = (master - from) / frames.max(1) as f32;
        self.master_prev = master;
        let mut bus = self.p.output.try_lock().ok();
        if let Some(b) = bus.as_mut() {
            b.clear();
        }
        let mut peak = 0.0f32;
        for (n, frame) in out.chunks_mut(channels).enumerate() {
            let (l, r) = self.engine.render(&patch, &block);
            let gain = from + step * n as f32;
            let fix = |s: f32| {
                let s = soft_limit(s * gain);
                if s.is_finite() { s } else { 0.0 }
            };
            let (l, r) = (fix(l), fix(r));
            peak = peak.max(l.abs()).max(r.abs());
            if channels == 1 {
                frame[0] = (l + r) * 0.5;
            } else {
                frame[0] = l;
                frame[1] = r;
            }
            if let Some(b) = bus.as_mut() {
                b.push((l + r) * 0.5);
            }
        }
        self.p.peak.set(peak);
        self.p.voices.store(self.engine.active(), Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------- the app

// The 106's face: slate panel, cream silkscreen, one stripe, a red LED.
const BG: Rgb565 = Rgb565::new(4, 8, 5);
const PLATE: Rgb565 = Rgb565::new(2, 5, 3);
const INK: Rgb565 = Rgb565::new(28, 56, 26);
const DIM: Rgb565 = Rgb565::new(14, 29, 14);
const FAINT: Rgb565 = Rgb565::new(7, 15, 8);
const CAP: Rgb565 = Rgb565::new(25, 50, 24);
const LED: Rgb565 = Rgb565::new(31, 12, 4);
const LED_DIM: Rgb565 = Rgb565::new(9, 3, 2);
const STRIPE: [Rgb565; 5] = [Rgb565::new(30, 12, 4), Rgb565::new(31, 34, 2), Rgb565::new(31, 52, 3), Rgb565::new(6, 46, 8), Rgb565::new(4, 26, 28)];
const ACCENT: Rgb565 = Rgb565::new(31, 34, 2);

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "june60",
        layers: vec![Layer::Native(0, "KEYS"), Layer::Controls, Layer::Moments],
        hero: vec![[C_CUTOFF, C_RES], [C_ENV, C_LFO_VCF], [C_ATTACK, C_RELEASE], [C_DECAY, C_SUSTAIN], [C_SAW, C_PULSE], [C_SUB, C_NOISE], [C_CHORUS, C_LFO_RATE], [C_LEVEL, C_PORTA]],
        browse: Some(C_PRESET),
        routes: Routes { stick_x: Some(C_BEND), stick_y: Some(C_MOD), hand_l: Some(C_CUTOFF), hand_r: Some(C_RES) },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: false,
    }
}

pub struct June60App {
    p: Arc<Shared>,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
    keys: Keys,
    patch: usize,
}

impl June60App {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let cv: [Arc<AtomicF32>; 8] = std::array::from_fn(|i| mods.register(format!("{APP_NAME}: {}", CV_NAMES[i])));
        let mut app = Self {
            p: Arc::new(Shared { vals: std::array::from_fn(|i| AtomicF32::new(CTLS[i].default)), cv, mix_level, ext_mix_level, output, peak: AtomicF32::new(0.0), voices: AtomicUsize::new(0) }),
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            keys: Keys::default(),
            patch: 0,
        };
        app.load_patch(0);
        app
    }

    fn load_patch(&mut self, idx: usize) {
        self.patch = idx.min(PATCHES.len() - 1);
        for i in 0..N {
            if !patch_keeps(i) {
                self.p.vals[i].set(CTLS[i].default);
            }
        }
        for &(i, v) in PATCHES[self.patch].1 {
            self.p.vals[i].set(v);
        }
    }

    fn step_patch(&mut self, d: i32) {
        if d != 0 {
            self.load_patch((self.patch as i32 + d.signum()).rem_euclid(PATCHES.len() as i32) as usize);
        }
    }

    fn val(&self, i: usize) -> f32 {
        self.p.vals[i].get()
    }

    fn step_of(&self, i: usize) -> usize {
        norm_step(self.val(i), CTLS[i].steps)
    }

    fn text(&self, i: usize) -> (String, String) {
        let v = self.p.eff(i);
        let pct = || format!("{:.0}%", v * 100.0);
        let value = match i {
            C_PRESET => format!("{:02} {}", self.patch + 1, PATCHES[self.patch].0),
            C_CHORUS => CHORUS_NAMES[self.step_of(i)].to_string(),
            C_HPF => HPF_NAMES[self.step_of(i)].to_string(),
            C_PWM_MODE => ["MAN", "LFO"][self.step_of(i)].into(),
            C_ENV_POL => ["NORMAL", "INVERTED"][self.step_of(i)].into(),
            C_VCA => ["ENV", "GATE"][self.step_of(i)].into(),
            C_RANGE => ["16'", "8'", "4'"][self.step_of(i)].into(),
            C_VOICES => ["POLY", "UNISON"][self.step_of(i)].into(),
            C_BEND => format!("{:+.1} st", (v * 2.0 - 1.0) * 2.0),
            C_OCTAVE => format!("{:+}", self.step_of(i) as i32 - 3),
            C_LFO_RATE => format!("{:.1} Hz", 0.2 * 150.0f32.powf(v)),
            C_LFO_DELAY => format!("{:.2} s", v * 3.0),
            _ => pct(),
        };
        (CTLS[i].name.to_string(), value)
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        (0..N).map(|i| {
            let (n, v) = self.text(i);
            (n, v, false)
        }).collect()
    }

    fn edit(&mut self, i: usize, d: i32) {
        if d == 0 {
            return;
        }
        if i == C_PRESET {
            return self.step_patch(d);
        }
        let steps = CTLS[i].steps;
        if steps > 0 {
            let next = (self.step_of(i) as i32 + d.signum()).clamp(0, steps as i32 - 1) as usize;
            self.p.vals[i].set(step_norm(next, steps));
        } else {
            let step = d as f32 * 0.01 * self.sensitivity.get().max(0.01) * 10.0;
            self.p.vals[i].set((self.val(i) + step).clamp(0.0, 1.0));
        }
    }
}

impl PlayHost for June60App {
    fn kit_control_count(&self) -> usize {
        N
    }
    fn kit_label(&self, i: usize) -> String {
        self.text(i.min(N - 1)).0
    }
    fn kit_value(&self, i: usize) -> String {
        self.text(i.min(N - 1)).1
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        Some(if i == C_PRESET { step_norm(self.patch, PATCHES.len()) } else { self.val(i) })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        CTLS[i.min(N - 1)].steps > 0
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit(i.min(N - 1), delta);
    }
    fn kit_reset(&mut self, i: usize) {
        let i = i.min(N - 1);
        if i == C_PRESET {
            self.load_patch(self.patch);
        } else {
            self.p.vals[i].set(CTLS[i].default);
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let i = i.min(N - 1);
        let v = v.clamp(0.0, 1.0);
        if i == C_PRESET {
            let target = norm_step(v, PATCHES.len());
            if target != self.patch {
                self.load_patch(target);
            }
        } else if CTLS[i].steps > 0 {
            self.p.vals[i].set(step_norm(norm_step(v, CTLS[i].steps), CTLS[i].steps));
        } else {
            self.p.vals[i].set(v);
        }
    }
    fn kit_snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "patch": self.patch, "v": (0..N).map(|i| self.val(i)).collect::<Vec<_>>() })
    }
    fn kit_recall(&mut self, v: &serde_json::Value) {
        if let Some(p) = v["patch"].as_u64() {
            self.patch = (p as usize).min(PATCHES.len() - 1);
        }
        if let Some(a) = v["v"].as_array() {
            for (i, x) in a.iter().enumerate().take(N) {
                if let Some(x) = x.as_f64() {
                    self.p.vals[i].set((x as f32).clamp(0.0, 1.0));
                }
            }
        }
    }
    fn kit_line(&self) -> String {
        format!("{:02} {}", self.patch + 1, PATCHES[self.patch].0)
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        crate::util::note_name(pad_note(pad, 0) as i32)
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, held: bool) -> crate::led_output::PadColor {
        use crate::led_output::PadColor;
        if held {
            PadColor::Green
        } else if pad_note(pad, 0) % 12 == 0 {
            PadColor::Blue
        } else {
            PadColor::Off
        }
    }
}

/// The panel's sliders, left to right: (section, silkscreen label, control).
const SLIDERS: [(&str, &str, usize); 19] = [
    ("LFO", "RATE", C_LFO_RATE),
    ("LFO", "DLY", C_LFO_DELAY),
    ("DCO", "PCH", C_LFO_DCO),
    ("DCO", "PWM", C_PWM),
    ("DCO", "SAW", C_SAW),
    ("DCO", "PUL", C_PULSE),
    ("DCO", "SUB", C_SUB),
    ("DCO", "NOI", C_NOISE),
    ("HPF", "HPF", C_HPF),
    ("VCF", "FREQ", C_CUTOFF),
    ("VCF", "RES", C_RES),
    ("VCF", "ENV", C_ENV),
    ("VCF", "LFO", C_LFO_VCF),
    ("VCF", "KYB", C_KYBD),
    ("VCA", "LVL", C_LEVEL),
    ("ENV", "A", C_ATTACK),
    ("ENV", "D", C_DECAY),
    ("ENV", "S", C_SUSTAIN),
    ("ENV", "R", C_RELEASE),
];

const PITCH: i32 = 28;
const GROUP_GAP: i32 = 14;
const SLOT_TOP: i32 = 106;
const SLOT_H: i32 = 140;

fn rect(f: &mut FrameBuffer, x: i32, y: i32, w: u32, h: u32, c: Rgb565) {
    Rectangle::new(Point::new(x, y), Size::new(w, h)).into_styled(PrimitiveStyle::with_fill(c)).draw(f).ok();
}

fn outline(f: &mut FrameBuffer, x: i32, y: i32, w: u32, h: u32, c: Rgb565) {
    Rectangle::new(Point::new(x, y), Size::new(w, h)).into_styled(PrimitiveStyle::with_stroke(c, 1)).draw(f).ok();
}

fn label(f: &mut FrameBuffer, s: &str, x: i32, y: i32, c: Rgb565) {
    Text::new(s, Point::new(x, y), MonoTextStyle::new(&SPLEEN_6X12, c)).draw(f).ok();
}

impl June60App {
    /// Which control the panel should mark: the menu's row, else the control
    /// the kit has grabbed.
    fn marked(&self) -> Option<usize> {
        self.kit.menu.then_some(self.list.selected.min(N - 1))
    }

    fn draw_panel(&self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        // Header: logo, stripe, LED patch number.
        Text::new("JUNE-60", Point::new(16, 34), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(f).ok();
        label(f, "POLYPHONIC SYNTHESIZER", 18, 48, DIM);
        for (i, c) in STRIPE.iter().enumerate() {
            rect(f, 250, 10 + i as i32 * 5, 270, 3, *c);
        }
        rect(f, 540, 8, 86, 44, PLATE);
        outline(f, 540, 8, 86, 44, FAINT);
        let num = format!("{:02}", self.patch + 1);
        Text::new("88", Point::new(552, 42), MonoTextStyle::new(&SPLEEN_16X32, LED_DIM)).draw(f).ok();
        Text::new(&num, Point::new(552, 42), MonoTextStyle::new(&SPLEEN_16X32, LED)).draw(f).ok();
        label(f, "PATCH", 592, 20, DIM);
        rect(f, 14, 58, 612, 1, FAINT);

        // Sliders, grouped by section with a silkscreen bar over each group.
        let col = if self.kit.menu { None } else { Some(self.kit.column(self)) };
        let badge = |ctl: usize| -> u8 {
            col.as_ref().map_or(0, |c| c.dials.iter().find(|d| d.label == CTLS[ctl].name).map_or(0, |d| d.knob))
        };
        let marked = self.marked();
        let mut x = 22;
        let mut group_start = (0, SLIDERS[0].0);
        let mut groups = Vec::new();
        let mut centres = Vec::new();
        for (i, (sec, _, _)) in SLIDERS.iter().enumerate() {
            if i > 0 && *sec != SLIDERS[i - 1].0 {
                x += GROUP_GAP;
                groups.push((group_start.1, group_start.0, i));
                group_start = (i, sec);
            }
            centres.push(x + PITCH / 2);
            x += PITCH;
        }
        groups.push((group_start.1, group_start.0, SLIDERS.len()));
        for (name, from, to) in groups {
            let (a, b) = (centres[from] - 12, centres[to - 1] + 12);
            let colour = STRIPE[["LFO", "DCO", "HPF", "VCF", "VCA", "ENV"].iter().position(|s| *s == name).unwrap_or(0) % 5];
            rect(f, a, 70, (b - a) as u32, 2, colour);
            label(f, name, a, 86, INK);
        }
        for (i, (_, name, ctl)) in SLIDERS.iter().enumerate() {
            let cx = centres[i];
            let (slot_x, bottom) = (cx - 1, SLOT_TOP + SLOT_H);
            for t in 0..=10 {
                let y = SLOT_TOP + t * SLOT_H / 10;
                rect(f, cx - 7, y, if t % 5 == 0 { 4 } else { 2 }, 1, FAINT);
            }
            rect(f, slot_x, SLOT_TOP, 3, SLOT_H as u32, PLATE);
            outline(f, slot_x - 1, SLOT_TOP - 1, 5, SLOT_H as u32 + 2, FAINT);
            // Continuous sliders show the value the audio sees, modulation included.
            let norm = if CTLS[*ctl].steps > 0 { self.val(*ctl) } else { self.p.eff(*ctl) };
            let y = bottom - (norm * SLOT_H as f32) as i32 - 8;
            rect(f, cx - 7, y, 15, 16, CAP);
            rect(f, cx - 7, y + 15, 15, 1, FAINT);
            rect(f, cx - 6, y + 7, 13, 2, BG);
            let hot = marked == Some(*ctl);
            if hot {
                outline(f, cx - 9, y - 2, 19, 20, ACCENT);
            }
            let k = badge(*ctl);
            if k > 0 {
                label(f, &k.to_string(), cx - 2, SLOT_TOP - 8, ACCENT);
            }
            label(f, name, cx - (name.len() as i32 * 3), bottom + 20, if hot || k > 0 { ACCENT } else { INK });
        }

        // Switch row: lit tabs, like the 106's buttons.
        let tabs: [(&str, &[&str], usize); 6] = [
            ("RANGE", &["16'", "8'", "4'"], C_RANGE),
            ("PWM", &["MAN", "LFO"], C_PWM_MODE),
            ("ENV", &["+", "-"], C_ENV_POL),
            ("VCA", &["ENV", "GATE"], C_VCA),
            ("VOICES", &["POLY", "UNI"], C_VOICES),
            ("CHORUS", &["OFF", "I", "II", "I+II"], C_CHORUS),
        ];
        let mut tx = 16;
        for (title, opts, ctl) in tabs {
            let on = self.step_of(ctl);
            label(f, title, tx, 288, DIM);
            let mut ox = tx;
            for (n, o) in opts.iter().enumerate() {
                let w = (o.len() as u32 * 6 + 8).max(22);
                let lit = n == on;
                rect(f, ox, 294, w, 16, if lit { PLATE } else { BG });
                outline(f, ox, 294, w, 16, if marked == Some(ctl) { ACCENT } else { FAINT });
                rect(f, ox + 3, 296, w - 6, 2, if lit { LED } else { LED_DIM });
                label(f, o, ox + 4, 308, if lit { INK } else { DIM });
                ox += w as i32 + 2;
            }
            tx = ox + 10;
        }

        // Footer: the performance controls with no slider, then the line the kit wants shown.
        let (porta, bend, wheel, oct) = (self.text(C_PORTA).1, self.text(C_BEND).1, self.text(C_MOD).1, self.text(C_OCTAVE).1);
        label(f, &format!("PORTA {porta}   BEND {bend}   MOD {wheel}   OCT {oct}"), 16, 334, DIM);
        let line = match (self.marked(), &col) {
            (Some(i), _) => format!("{}: {}", self.text(i).0, self.text(i).1),
            (None, Some(c)) if !c.status.is_empty() => c.status.clone(),
            _ => format!("{}   {} voices", PATCHES[self.patch].0, self.p.voices.load(Ordering::Relaxed)),
        };
        label(f, &line.chars().take(48).collect::<String>(), 16, 350, INK);
        let level = self.p.peak.get().clamp(0.0, 1.0).sqrt();
        outline(f, 480, 340, 146, 8, FAINT);
        rect(f, 481, 341, (level * 144.0) as u32, 6, LED);
    }
}

impl App for June60App {
    fn instrument_settings(&self) -> Vec<crate::app::Setting> {
        kit::settings_of(self)
    }
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        kit::adjust_in(self, index, delta)
    }
    fn supports_pad_lock(&self) -> bool {
        true
    }
    fn play_surface(&self) -> bool {
        true
    }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (!self.kit.menu).then(|| self.kit.column(self))
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }
    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
    }
    fn grid_led_overlay(&self) -> [crate::led_output::PadColor; 16] {
        self.kit.led_overlay(self)
    }
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let mut keyed = step.input.clone();
        keyed.midi_keys = input.midi_keys;
        self.keys.update(&keyed, step.native.is_some(), |p| pad_note(p, 0));
        if step.menu {
            let input = &step.input;
            self.list.navigate_input(input, N, self.nav.get() as i32);
            let sel = self.list.selected.min(N - 1);
            self.kit_edit(sel, input.knob2);
            if input.knob2_press {
                self.kit_reset(sel);
            }
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        self.draw_panel(f);
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if self.taken {
            return None;
        }
        self.taken = true;
        Some(Box::new(Processor { p: Arc::clone(&self.p), notes: Arc::clone(&self.keys.queue), engine: Engine::new(), master_prev: 0.0 }))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(June60App::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> June60App {
        June60App::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
    }

    /// Stereo blocks of 512 frames: (left, right).
    fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (Vec::new(), Vec::new());
        for _ in 0..blocks {
            let mut buf = vec![0.0f32; 512 * 2];
            p.process(&mut buf, 2, 48_000.0);
            assert!(buf.iter().all(|v| v.is_finite() && v.abs() <= 1.0), "output stays finite and inside full scale");
            l.extend(buf.iter().step_by(2));
            r.extend(buf.iter().skip(1).step_by(2));
        }
        (l, r)
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// Energy of the first difference relative to the signal: higher means brighter.
    fn brightness(x: &[f32]) -> f32 {
        let d: f32 = x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
        d / x.iter().map(|v| v * v).sum::<f32>().max(1e-12)
    }

    fn hz(x: &[f32]) -> f32 {
        x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f32 / (x.len() as f32 / 48_000.0)
    }

    fn keys(a: &mut June60App, notes: &[u8], vel: u8) {
        let mut input = Input::default();
        for &n in notes {
            input.midi_keys.0[n as usize] = vel;
        }
        a.tick(&input);
    }

    #[test]
    fn a_key_plays_at_its_pitch_and_releases_to_silence() {
        let mut a = app();
        a.kit_set_norm(C_CHORUS, 0.0);
        let mut p = a.audio_processor().unwrap();
        assert!(rms(&render(&mut p, 4).0) < 1e-6, "silent before a note");
        keys(&mut a, &[69], 100);
        render(&mut p, 6);
        let out = render(&mut p, 40).0;
        assert!(rms(&out) > 0.02, "a held key sounds ({})", rms(&out));
        let f = hz(&out);
        assert!((f - 440.0).abs() < 8.0, "A4 at 8' plays 440 Hz, got {f}");
        keys(&mut a, &[69], 0);
        render(&mut p, 150);
        assert!(rms(&render(&mut p, 10).0) < 0.003, "the release dies away");
    }

    #[test]
    fn the_range_switch_moves_the_pitch_by_octaves() {
        let mut a = app();
        a.kit_set_norm(C_CHORUS, 0.0);
        a.kit_set_norm(C_RANGE, R4);
        let mut p = a.audio_processor().unwrap();
        keys(&mut a, &[57], 100);
        render(&mut p, 6);
        let f = hz(&render(&mut p, 40).0);
        assert!((f - 440.0).abs() < 8.0, "A3 at 4' sounds an octave up, got {f}");
    }

    #[test]
    fn the_cutoff_darkens_the_sound_and_resonance_sings() {
        let bright = |cutoff: f32, res: f32| {
            let mut a = app();
            a.kit_set_norm(C_CHORUS, 0.0);
            a.kit_set_norm(C_CUTOFF, cutoff);
            a.kit_set_norm(C_RES, res);
            let mut p = a.audio_processor().unwrap();
            keys(&mut a, &[48], 100);
            render(&mut p, 8);
            render(&mut p, 20).0
        };
        let open = bright(1.0, 0.0);
        let shut = bright(0.3, 0.0);
        assert!(brightness(&shut) < brightness(&open) * 0.1, "a lower cutoff removes the highs ({} vs {})", brightness(&shut), brightness(&open));
        // 24 dB/oct: a note 4 octaves above the cutoff is gone, not dulled.
        assert!(rms(&shut) < rms(&open) * 0.6);

        // With nothing but the filter and full resonance, it still rings.
        let mut a = app();
        a.kit_set_norm(C_CHORUS, 0.0);
        a.kit_set_norm(C_SAW, 0.0);
        a.kit_set_norm(C_CUTOFF, 0.55);
        a.kit_set_norm(C_RES, 1.0);
        a.kit_set_norm(C_VCA, 1.0);
        let mut p = a.audio_processor().unwrap();
        keys(&mut a, &[48], 100);
        render(&mut p, 100);
        let ring = render(&mut p, 20).0;
        assert!(rms(&ring) > 0.01, "full resonance self-oscillates with no oscillator ({})", rms(&ring));
        let expect = 20.0 * 1000.0f32.powf(0.55) * 2.0f32.powf(0.5 * -1.0);
        assert!(hz(&ring) > expect * 0.5 && hz(&ring) < expect * 2.0, "and near the cutoff: {} vs {expect}", hz(&ring));
    }

    #[test]
    fn six_voices_in_poly_and_all_six_in_unison() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        keys(&mut a, &[48, 52, 55, 59, 62, 65, 69, 72], 100);
        render(&mut p, 4);
        assert_eq!(a.p.voices.load(Ordering::Relaxed), 6, "eight keys can only hold six voices");

        let mut u = app();
        u.kit_set_norm(C_VOICES, 1.0);
        let mut p = u.audio_processor().unwrap();
        keys(&mut u, &[60], 100);
        render(&mut p, 4);
        assert_eq!(u.p.voices.load(Ordering::Relaxed), 6, "unison stacks every voice on one note");
    }

    #[test]
    fn the_chorus_widens_the_image_and_off_is_mono() {
        let stereo = |chorus: f32| {
            let mut a = app();
            a.kit_set_norm(C_CHORUS, chorus);
            let mut p = a.audio_processor().unwrap();
            keys(&mut a, &[48, 55], 100);
            render(&mut p, 10);
            render(&mut p, 40)
        };
        let (l, r) = stereo(0.0);
        assert!(l.iter().zip(&r).all(|(a, b)| (a - b).abs() < 1e-6), "chorus off gives identical channels");
        let (l, r) = stereo(2.0 / 3.0);
        let diff: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        assert!(rms(&diff) > 0.01, "chorus II decorrelates the two sides ({})", rms(&diff));
    }

    #[test]
    fn every_patch_loads_and_plays_a_chord_inside_full_scale() {
        for i in 0..PATCHES.len() {
            let mut a = app();
            a.load_patch(i);
            assert_eq!(a.kit_value(C_PRESET), format!("{:02} {}", i + 1, PATCHES[i].0));
            let mut p = a.audio_processor().unwrap();
            keys(&mut a, &[36, 48, 55, 60, 64, 67], 100);
            let out = render(&mut p, 30).0;
            assert!(out.iter().all(|v| v.is_finite()), "{} stays finite", PATCHES[i].0);
            assert!(rms(&out) > 1e-4, "{} makes sound ({})", PATCHES[i].0, rms(&out));
        }
    }

    #[test]
    fn a_patch_change_keeps_the_players_controls() {
        let mut a = app();
        a.kit_set_norm(C_LEVEL, 0.3);
        a.kit_edit(C_PRESET, 1);
        assert_eq!(a.patch, 1);
        assert!((a.val(C_LEVEL) - 0.3).abs() < 1e-6, "level survives a patch change");
        assert!(a.val(C_CUTOFF) < 0.5, "but the sound changes: Brass is dark");
    }

    #[test]
    fn it_opens_on_the_play_view_and_knob_one_turns_cutoff() {
        let mut a = app();
        assert!(!a.kit.menu);
        let before = a.val(C_CUTOFF);
        a.tick(&Input { knob1: 2, ..Default::default() });
        assert!(a.val(C_CUTOFF) > before, "knob 1 turns the first hero control");
        let mut fb = FrameBuffer::new();
        a.draw(&mut fb);
    }

    /// Renders the panel to a PNG in the folder named by PORTAMAX_JUNE_SHOT.
    #[test]
    #[ignore = "writes a screenshot to the folder in PORTAMAX_JUNE_SHOT"]
    fn screenshot() {
        let Ok(dir) = std::env::var("PORTAMAX_JUNE_SHOT") else { return };
        let mut a = app();
        a.load_patch(2);
        let mut p = a.audio_processor().unwrap();
        keys(&mut a, &[48, 55, 60], 100);
        render(&mut p, 20);
        a.tick(&Input { knob1_press: true, ..Default::default() });
        let mut fb = FrameBuffer::new();
        a.draw(&mut fb);
        let file = std::fs::File::create(std::path::Path::new(&dir).join("june60.png")).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), crate::display::WIDTH as u32, crate::display::HEIGHT as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let data: Vec<u8> = fb.buffer().iter().flat_map(|px| [(px >> 16) as u8, (px >> 8) as u8, *px as u8]).collect();
        enc.write_header().unwrap().write_image_data(&data).unwrap();
    }
}
