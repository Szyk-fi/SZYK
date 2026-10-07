//! Blaster's characters: a charge sound and a blast that belong together.
//!
//! A preset is a list of (parameter index, value) overrides on the initial
//! patch, so each only says what makes that blast different. They are
//! original sound designs in the spirit of classic video-game weapons, named
//! for what they do rather than copying any one game's sound.

use super::params::*;
use super::store::{from_norm, to_norm, Params};
use crate::apps::hydra::dsp::Rng;

pub struct Preset {
    pub name: &'static str,
    pub values: Vec<(usize, f32)>,
}

macro_rules! pr {
    ($name:expr; $( $p:expr => $v:expr ),* $(,)?) => {
        Preset { name: $name, values: vec![ $( ($p as usize, $v as f32) ),* ] }
    };
}

const SQUARE: f32 = 0.0;
const SAW: f32 = 1.0;
const TRI: f32 = 2.0;
const SINE: f32 = 3.0;
const PULSE: f32 = 4.0;
const NOISE: f32 = 5.0;
const METAL: f32 = 6.0;
const STEADY: f32 = 0.0;
const FLUTTER: f32 = 1.0;
const PULSING: f32 = 2.0;

pub fn factory() -> Vec<Preset> {
    use P::*;
    vec![
        // Built from measurements of a recording of a classic charge shot: a 1.4 s
        // pitch rise that accelerates (155 to 502 Hz, as u^2.2) and lands exactly
        // at full charge on a held 502 Hz tone with an unusual spectrum (the Reed
        // wave) that ripples at about 19 Hz; then, on release, no tone at all,
        // just a noise burst that swells for about 0.1 s, sweeps from bright to
        // dull and is gone in about 0.55 s.
        pr!("X Charge Shot";
            ChargeTime => 1.4, ChgWave => 7.0, ChgPitch => 155.0, Climb => 0.0, ChirpCount => 1, ChirpDepth => 1.7, ChirpCurve => 2.2, HoldTop => 1.0,
            FullMode => PULSING, FlutterRate => 19.0, RippleDepth => 0.26, ChgVol => 0.25, FullBoost => 0.0, ChgWobble => 0.1, ChgLevel => 0.08,
            ToneLevel => 0.0, BlastWave => NOISE, Body => 0.0, Length => 0.1, Attack => 0.10, Punch => 0.0, Drive => 0.0,
            Noise => 0.72, NoiseDecay => 1.7, NoiseHP => 60.0, NoiseLP => 10500.0, NoiseSweep => -3.2, NoiseSweepTime => 0.4, NoiseSweepCurve => 2.0, NoiseEnd => 0.285,
            PowerLength => 1.0, PowerNoise => 0.0, PowerPitch => 0.0, PowerVolume => 0.5),
        // A synthetic take on the same idea, in 8-bit: a square tone that climbs in quickening
        // chirps while the key is held, flutters when it is full, and fires a
        // falling square blast with a puff of noise on release.
        pr!("Buster Charge";
            ChargeTime => 1.0, ChgWave => SQUARE, ChgPitch => 440.0, Climb => 1.2, ChirpRate0 => 6.0, ChirpRate1 => 20.0, ChirpDepth => 1.2, FullMode => FLUTTER, FlutterRate => 14.0, FlutterSemis => 7,
            BlastWave => SQUARE, BlastStart => 3200.0, BlastEnd => 160.0, SweepTime => 0.28, SweepCurve => 1.8, Length => 0.45, Body => 0.4, Noise => 0.35, NoiseDecay => 0.14, Rate => 24000.0, Drive => 0.1),
        pr!("Pea Shot";
            ChargeTime => 0.6, ChgWave => SQUARE, ChgPitch => 520.0, Climb => 0.8, ChirpRate1 => 18.0, ChirpDepth => 0.8,
            BlastWave => PULSE, Width => 0.12, BlastStart => 1900.0, BlastEnd => 420.0, SweepTime => 0.1, Length => 0.13, Body => 0.0, Noise => 0.05, NoiseDecay => 0.04, PowerLength => 0.6, PowerPitch => 0.3, Bits => 8, Rate => 16000.0),
        pr!("X Buster";
            ChargeTime => 1.1, ChgWave => SAW, ChgPitch => 260.0, Climb => 2.0, ChirpRate0 => 8.0, ChirpRate1 => 26.0, ChirpDepth => 1.5, FullMode => FLUTTER, FlutterRate => 18.0, FlutterSemis => 5,
            BlastWave => SAW, BlastStart => 2800.0, BlastEnd => 120.0, SweepTime => 0.3, Length => 0.6, Body => 0.6, BodySemis => -12, Noise => 0.45, NoiseDecay => 0.2, Punch => 0.5, EchoMix => 0.15, EchoTime => 160.0),
        pr!("Plasma Beam";
            ChargeTime => 1.4, ChgWave => SINE, ChgPitch => 200.0, Climb => 2.5, ChirpDepth => 0.0, ChgWobble => 0.4, ChgNoise => 0.2, FullMode => STEADY,
            BlastWave => TRI, BlastStart => 900.0, BlastEnd => 700.0, SweepTime => 0.3, SweepCurve => 1.0, Length => 1.2, Hold => 0.5, Body => 0.5, BodySemis => 7, Noise => 0.1, PowerLength => 2.0, Punch => 0.1, EchoMix => 0.2, EchoTime => 240.0),
        pr!("Fireball";
            ChargeTime => 1.3, ChgWave => SINE, ChgPitch => 70.0, Climb => 1.5, ChirpDepth => 0.2, ChgNoise => 0.8, ChgWobble => 0.3, FullMode => PULSING, FlutterRate => 9.0,
            BlastWave => NOISE, BlastStart => 500.0, BlastEnd => 90.0, SweepTime => 0.5, Length => 0.7, Body => 0.0, Noise => 0.8, NoiseDecay => 0.5, NoiseHP => 120.0, NoiseLP => 3000.0, NoiseSweep => -1.5, Drive => 0.3, PowerNoise => 0.8),
        pr!("Wave Motion";
            ChargeTime => 1.5, ChgWave => SINE, ChgPitch => 220.0, Climb => 1.0, ClimbSteps => 4, ChirpDepth => 0.1, ChgWobble => 0.35, FullMode => STEADY,
            BlastWave => SAW, BlastStart => 1200.0, BlastEnd => 400.0, SweepTime => 0.6, Length => 1.0, Hold => 0.25, Body => 0.5, BodySemis => -12, Noise => 0.4, NoiseDecay => 0.6, NoiseLP => 5000.0, EchoMix => 0.3, EchoTime => 300.0, EchoFb => 0.5),
        pr!("Spin Rev";
            ChargeTime => 1.0, ChgWave => SAW, ChgPitch => 110.0, Climb => 3.0, ChirpRate0 => 10.0, ChirpRate1 => 30.0, ChirpDepth => 0.1, ChgWobble => 0.3, FullMode => FLUTTER, FlutterRate => 22.0, FlutterSemis => 3,
            BlastWave => SAW, BlastStart => 300.0, BlastEnd => 1900.0, SweepTime => 0.15, SweepCurve => 2.0, Length => 0.3, Body => 0.3, Noise => 0.2, NoiseHP => 1500.0, PowerPitch => 1.0, PowerLength => 1.0),
        pr!("Laser Rifle";
            ChargeTime => 0.9, ChgWave => TRI, ChgPitch => 600.0, Climb => 1.0, ChirpRate0 => 4.0, ChirpRate1 => 8.0, ChirpDepth => 0.2,
            BlastWave => SAW, BlastStart => 5000.0, BlastEnd => 400.0, SweepTime => 0.12, SweepCurve => 2.0, Length => 0.25, Body => 0.3, Noise => 0.05, Punch => 0.5, EchoMix => 0.2, EchoTime => 120.0),
        pr!("Ice Shot";
            ChargeTime => 1.2, ChgWave => METAL, ChgPitch => 880.0, Climb => 1.0, ChirpRate1 => 12.0, ChirpDepth => 0.6,
            BlastWave => METAL, BlastStart => 3500.0, BlastEnd => 900.0, SweepTime => 0.4, Length => 0.8, Body => 0.2, FmIndex => 0.3, Noise => 0.15, NoiseHP => 4000.0, NoiseDecay => 0.3, EchoMix => 0.3, EchoTime => 310.0, EchoFb => 0.5),
        pr!("Thunder Strike";
            ChargeTime => 1.6, ChgWave => TRI, ChgPitch => 90.0, Climb => 1.5, ChirpDepth => 0.4, ChgNoise => 1.0, FullMode => PULSING, FlutterRate => 20.0,
            BlastWave => NOISE, BlastStart => 400.0, BlastEnd => 60.0, SweepTime => 0.3, Length => 1.2, Body => 0.0, Noise => 1.0, NoiseDecay => 0.8, NoiseHP => 60.0, NoiseLP => 2500.0, NoiseSweep => -2.0, Punch => 1.0, Drive => 0.5,SubBlast => 0.5, SubCharge => 0.3, SubOctave => 0),
        pr!("Bomb";
            ChargeTime => 2.0, ChgWave => SQUARE, ChgPitch => 60.0, Climb => 2.0, ChirpRate0 => 2.0, ChirpRate1 => 10.0, ChirpDepth => 0.5, FullMode => PULSING, FlutterRate => 8.0,
            BlastWave => SINE, BlastStart => 160.0, BlastEnd => 25.0, SweepTime => 0.6, Length => 1.5, Body => 0.4, BodySemis => 12, Noise => 0.9, NoiseDecay => 1.1, NoiseHP => 40.0, NoiseLP => 800.0, Punch => 0.8, Drive => 0.4, PowerLength => 3.0,SubBlast => 0.8, SubCharge => 0.5, SubOctave => 0),
        pr!("Wave Cannon";
            ChargeTime => 3.0, ChgWave => TRI, ChgPitch => 150.0, Climb => 2.5, ChirpDepth => 0.2, ChgWobble => 0.5, FullMode => FLUTTER, FlutterRate => 10.0, FlutterSemis => 12,
            BlastWave => SAW, BlastStart => 1500.0, BlastEnd => 500.0, SweepTime => 0.5, Length => 1.4, Hold => 0.8, Body => 0.7, BodySemis => 7, Noise => 0.3, NoiseLP => 6000.0, EchoMix => 0.25, EchoTime => 280.0),
        pr!("Magic Orb";
            ChargeTime => 1.4, ChgWave => SINE, ChgPitch => 330.0, Climb => 2.0, ClimbSteps => 7, ChirpDepth => 0.0, FullMode => PULSING, FlutterRate => 12.0,
            BlastWave => SINE, BlastStart => 2200.0, BlastEnd => 1100.0, SweepTime => 0.5, Length => 0.9, Body => 0.5, BodySemis => 12, FmIndex => 0.4, Noise => 0.05, NoiseHP => 5000.0, EchoMix => 0.35, EchoTime => 280.0, EchoFb => 0.5),
        pr!("Sword Beam";
            ChargeTime => 0.5, ChgWave => SAW, ChgPitch => 500.0, Climb => 1.0, ChirpRate1 => 24.0, ChirpDepth => 0.8,
            BlastWave => SAW, BlastStart => 4000.0, BlastEnd => 600.0, SweepTime => 0.18, SweepCurve => 3.0, Length => 0.4, Body => 0.2, Noise => 0.3, NoiseHP => 2500.0, NoiseLP => 12000.0, Punch => 0.4),
        pr!("Rail Gun";
            ChargeTime => 2.5, ChgWave => SINE, ChgPitch => 200.0, Climb => 3.5, ChirpDepth => 0.0, FullMode => STEADY, ChgNoise => 0.15,
            BlastWave => METAL, BlastStart => 6000.0, BlastEnd => 300.0, SweepTime => 0.05, SweepCurve => 3.0, Length => 1.2, Body => 0.3, Noise => 0.8, NoiseDecay => 0.06, Punch => 1.0, EchoMix => 0.25, EchoTime => 200.0, EchoFb => 0.5),
        pr!("Pixel Pop";
            ChargeTime => 0.5, ChgWave => SQUARE, ChgPitch => 700.0, Climb => 0.6, ChirpRate1 => 22.0, ChirpDepth => 0.7, FullMode => FLUTTER,
            BlastWave => SQUARE, BlastStart => 1600.0, BlastEnd => 300.0, SweepTime => 0.09, Length => 0.1, Body => 0.0, Noise => 0.0, Punch => 0.0, PowerLength => 0.4, PowerVolume => 0.2, Bits => 5, Rate => 8000.0),
        pr!("Overdrive Buster";
            ChargeTime => 1.0, ChgWave => PULSE, ChgPitch => 300.0, Climb => 1.8, ChirpRate1 => 24.0, ChirpDepth => 1.5, Bits => 6, Rate => 11000.0, Drive => 0.6,
            BlastWave => SAW, BlastStart => 3000.0, BlastEnd => 90.0, SweepTime => 0.3, Length => 0.7, Body => 0.6, Noise => 0.5, NoiseDecay => 0.2, Punch => 0.8, PowerLength => 2.5),
        pr!("Ghost Wail";
            ChargeTime => 1.6, ChgWave => SINE, ChgPitch => 400.0, Climb => 1.2, ChirpDepth => 0.3, ChgWobble => 1.0,
            BlastWave => SINE, BlastStart => 1800.0, BlastEnd => 300.0, SweepTime => 1.0, SweepCurve => 0.6, Length => 2.0, Body => 0.4, BodySemis => 7, FmIndex => 0.5, Noise => 0.2, NoiseLP => 3000.0, EchoMix => 0.45, EchoTime => 340.0, EchoFb => 0.6),
        pr!("Tractor Beam";
            ChargeTime => 1.2, ChgWave => TRI, ChgPitch => 180.0, Climb => 1.5, ChirpRate0 => 2.0, ChirpRate1 => 5.0, ChirpDepth => 0.4,
            BlastWave => TRI, BlastStart => 200.0, BlastEnd => 1600.0, SweepTime => 0.8, SweepCurve => 1.0, Length => 1.0, Hold => 0.8, Body => 0.5, BodySemis => 7, Noise => 0.05, PowerPitch => 0.0, EchoMix => 0.2),
        pr!("Meteor";
            ChargeTime => 2.2, ChgWave => TRI, ChgPitch => 55.0, Climb => 2.0, ChirpDepth => 0.3, ChgNoise => 1.0, ChgWobble => 0.4,
            BlastWave => NOISE, BlastStart => 900.0, BlastEnd => 50.0, SweepTime => 1.5, Length => 2.5, Body => 0.0, Noise => 1.0, NoiseDecay => 1.8, NoiseHP => 80.0, NoiseLP => 4000.0, NoiseSweep => -3.0, Drive => 0.3, PowerLength => 2.0,SubBlast => 0.6, SubCharge => 0.4, SubOctave => 1),
        pr!("Gravity Well";
            ChargeTime => 2.0, ChgWave => SINE, ChgPitch => 55.0, Climb => 3.0, ChirpDepth => 0.0, ChgWobble => 0.2, FullMode => STEADY,
            BlastWave => SINE, BlastStart => 90.0, BlastEnd => 28.0, SweepTime => 1.0, Length => 2.0, Body => 0.6, BodySemis => 12, Noise => 0.4, NoiseLP => 500.0, NoiseHP => 20.0, NoiseDecay => 1.2, Punch => 0.4, PowerPitch => 0.0, PowerLength => 2.5,SubBlast => 0.7, SubCharge => 0.7, SubOctave => 0),
        pr!("Siren Charge";
            ChargeTime => 1.5, ChgWave => SAW, ChgPitch => 300.0, Climb => 1.5, ChirpRate0 => 3.0, ChirpRate1 => 6.0, ChirpDepth => 1.0, ChirpCurve => 0.6, ChgWobble => 1.0,
            BlastWave => SQUARE, BlastStart => 1200.0, BlastEnd => 1200.0, SweepTime => 0.05, Length => 0.7, Body => 0.3, Noise => 0.1, PowerPitch => 0.0),
        pr!("Dragon Breath";
            ChargeTime => 1.8, ChgWave => SAW, ChgPitch => 80.0, Climb => 1.5, ChirpDepth => 0.2, ChgNoise => 0.9, ChgWobble => 0.4,
            BlastWave => SAW, BlastStart => 300.0, BlastEnd => 120.0, SweepTime => 1.0, Length => 1.8, Hold => 0.6, Body => 0.5, Noise => 1.0, NoiseDecay => 1.4, NoiseHP => 200.0, NoiseLP => 2500.0, Drive => 0.4, PowerLength => 1.5,SubBlast => 0.5, SubCharge => 0.4, SubOctave => 0),
        pr!("Power Fist";
            ChargeTime => 0.8, ChgWave => PULSE, ChgPitch => 240.0, Climb => 1.5, ChirpRate1 => 28.0, ChirpDepth => 1.0, FullMode => FLUTTER, FlutterRate => 24.0,
            BlastWave => SQUARE, BlastStart => 400.0, BlastEnd => 50.0, SweepTime => 0.1, Length => 0.25, Body => 0.5, Noise => 0.5, NoiseDecay => 0.08, Punch => 1.0, Drive => 0.6, Bits => 8, PowerLength => 1.5,SubBlast => 0.6, SubWave => 2.0, SubOctave => 0),
        pr!("Star Shot";
            ChargeTime => 1.0, ChgWave => TRI, ChgPitch => 500.0, Climb => 2.0, ClimbSteps => 5, ChirpDepth => 0.0, FullMode => PULSING, FlutterRate => 16.0,
            BlastWave => SINE, BlastStart => 5000.0, BlastEnd => 2500.0, SweepTime => 0.4, Length => 0.6, Body => 0.4, BodySemis => 12, FmIndex => 0.5, Noise => 0.05, NoiseHP => 6000.0, EchoMix => 0.4, EchoTime => 200.0, EchoFb => 0.55),
        pr!("Ring Blast";
            ChargeTime => 1.3, ChgWave => METAL, ChgPitch => 300.0, Climb => 2.0, ChirpRate1 => 14.0, ChirpDepth => 0.8,
            BlastWave => METAL, BlastStart => 1200.0, BlastEnd => 500.0, SweepTime => 0.35, Length => 1.0, Body => 0.5, BodySemis => 5, FmIndex => 0.2, Noise => 0.2, NoiseHP => 3000.0, EchoMix => 0.3, EchoTime => 250.0),
        pr!("Plasma Pistol";
            ChargeTime => 0.9, ChgWave => PULSE, ChgPitch => 400.0, Climb => 1.3, ChirpRate0 => 5.0, ChirpRate1 => 15.0, ChirpDepth => 0.9,
            BlastWave => PULSE, Width => 0.2, BlastStart => 2600.0, BlastEnd => 260.0, SweepTime => 0.2, Length => 0.4, Body => 0.4, Noise => 0.3, NoiseDecay => 0.12, Punch => 0.5, Drive => 0.2),
    ]
}

/// Loads a preset: the initial patch, then its overrides.
pub fn apply(params: &Params, preset: &Preset) {
    params.reset_all();
    for &(i, v) in &preset.values {
        params.set_at(i, v);
    }
}

/// A fresh random blast that is meant to be usable: it charges audibly and
/// fires something with a clear sweep.
pub fn randomize(params: &Params, rng: &mut Rng) {
    params.reset_all();
    let r = |rng: &mut Rng, lo: f32, hi: f32| lo + rng.unit() * (hi - lo);
    let pick = |rng: &mut Rng, n: usize| (rng.unit() * n as f32) as usize % n;
    params.set(P::ChargeTime, 10.0f32.powf(r(rng, -0.1, 0.45)));
    params.set(P::ChgWave, pick(rng, WAVES.len()) as f32);
    params.set(P::ChgPitch, 10.0f32.powf(r(rng, 1.9, 3.0)));
    params.set(P::Climb, r(rng, 0.5, 3.2));
    params.set(P::ClimbSteps, if rng.unit() < 0.25 { (3 + pick(rng, 5)) as f32 } else { 0.0 });
    params.set(P::ChirpRate0, r(rng, 2.0, 9.0));
    params.set(P::ChirpRate1, r(rng, 8.0, 30.0));
    params.set(P::ChirpDepth, if rng.unit() < 0.2 { 0.0 } else { r(rng, 0.2, 1.6) });
    params.set(P::ChgWobble, if rng.unit() < 0.3 { r(rng, 0.2, 1.0) } else { 0.0 });
    params.set(P::ChgNoise, if rng.unit() < 0.3 { r(rng, 0.2, 0.9) } else { 0.0 });
    params.set(P::FullMode, pick(rng, 3) as f32);
    params.set(P::BlastWave, pick(rng, WAVES.len()) as f32);
    let (a, b) = (10.0f32.powf(r(rng, 2.7, 3.8)), 10.0f32.powf(r(rng, 1.8, 2.9)));
    let up = rng.unit() < 0.15;
    params.set(P::BlastStart, if up { b } else { a });
    params.set(P::BlastEnd, if up { a } else { b });
    params.set(P::SweepTime, 10.0f32.powf(r(rng, -1.4, -0.1)));
    params.set(P::SweepCurve, r(rng, 0.8, 3.0));
    params.set(P::Length, 10.0f32.powf(r(rng, -0.8, 0.3)));
    params.set(P::Hold, if rng.unit() < 0.2 { r(rng, 0.1, 0.8) } else { 0.0 });
    params.set(P::Body, r(rng, 0.0, 0.7));
    params.set(P::BodySemis, [-12.0, -7.0, 0.0, 7.0, 12.0][pick(rng, 5)]);
    params.set(P::FmIndex, if rng.unit() < 0.3 { r(rng, 0.1, 0.6) } else { 0.0 });
    params.set(P::Noise, r(rng, 0.05, 0.9));
    params.set(P::NoiseDecay, 10.0f32.powf(r(rng, -1.3, 0.0)));
    params.set(P::NoiseHP, 10.0f32.powf(r(rng, 1.8, 3.6)));
    params.set(P::Punch, r(rng, 0.0, 0.9));
    if rng.unit() < 0.4 {
        params.set(P::Bits, (5 + pick(rng, 4)) as f32);
        params.set(P::Rate, 10.0f32.powf(r(rng, 3.8, 4.4)));
    }
    params.set(P::Drive, r(rng, 0.0, 0.6));
    params.set(P::EchoMix, if rng.unit() < 0.4 { r(rng, 0.1, 0.4) } else { 0.0 });
}

/// Nudges a random handful of continuous settings, to evolve the sound.
pub fn mutate(params: &Params, rng: &mut Rng) {
    for (i, d) in DEFS.iter().enumerate() {
        let skip = matches!(d.kind, Kind::Toggle | Kind::Int { .. } | Kind::Choice(_)) || i == P::Level as usize;
        if skip || rng.unit() > 0.2 {
            continue;
        }
        let x = to_norm(d, params.at(i)) + rng.bipolar() * 0.09;
        params.set_at(i, from_norm(d, x.clamp(0.0, 1.0)));
    }
}
