//! Blaster's parameter table: every setting in one place, so the menu, the
//! play view, the modulation inputs and the character presets all come from
//! it. The kinds and helpers are Hydra's (apps/hydra/params.rs).

#![allow(non_camel_case_types)]

pub use crate::apps::hydra::params::{ch, exp, int, lin, tog, Def, Kind, Unit};

pub const GROUPS: [&str; 9] = ["Main", "Charge", "Climb", "Full charge", "Blast", "Sub", "Size", "Noise", "Crush & Echo"];

pub const WAVES: [&str; 8] = ["Square", "Saw", "Triangle", "Sine", "Pulse", "Noise", "Metal", "Reed"];
pub const SUB_WAVES: [&str; 3] = ["Sine", "Triangle", "Square"];
pub const SUB_OCTAVES: [&str; 2] = ["-1 oct", "-2 oct"];
pub const FULL_MODES: [&str; 3] = ["Keep chirping", "Flutter", "Ripple"];

macro_rules! params {
    ($( $id:ident, $name:expr, $group:expr, $kind:expr, $def:expr; )*) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(usize)]
        pub enum P { $( $id, )* }
        pub const COUNT: usize = [$( P::$id ),*].len();
        pub static DEFS: [Def; COUNT] = [ $( Def { name: $name, group: $group, kind: $kind, default: $def } ),* ];
    };
}

params! {
    Level, "Level", 0, lin(0.0, 1.0, Unit::Percent), 0.7;
    KeyFollow, "Key follow", 0, lin(0.0, 1.0, Unit::Percent), 1.0;
    Octave, "Octave", 0, int(-3, 3), 0.0;
    Tap, "Tap shot below", 0, lin(0.0, 0.4, Unit::Seconds), 0.12;
    AutoFire, "Fire at full", 0, tog(), 0.0;
    VelSens, "Velocity", 0, lin(0.0, 1.0, Unit::Percent), 0.5;
    Scale, "Pad scale", 0, int(0, 15), 1.0;
    Root, "Pad root", 0, int(0, 11), 0.0;

    ChargeTime, "Charge time", 1, exp(0.1, 6.0, Unit::Seconds), 1.2;
    ChgWave, "Wave", 1, ch(&WAVES), 0.0;
    ChgPitch, "Pitch", 1, exp(40.0, 4000.0, Unit::Hz), 330.0;
    ChgVol, "Start volume", 1, lin(0.0, 1.0, Unit::Percent), 0.35;
    ChgNoise, "Noise bed", 1, lin(0.0, 1.0, Unit::Percent), 0.0;
    ChgWobble, "Wobble", 1, lin(0.0, 1.0, Unit::Percent), 0.0;

    Climb, "Climb", 2, lin(0.0, 4.0, Unit::None), 1.5;
    ClimbSteps, "Climb steps", 2, int(0, 12), 0.0;
    ChirpRate0, "Chirp rate start", 2, exp(0.5, 40.0, Unit::Hz), 5.0;
    ChirpRate1, "Chirp rate full", 2, exp(0.5, 40.0, Unit::Hz), 16.0;
    ChirpDepth, "Chirp depth", 2, lin(0.0, 3.0, Unit::None), 1.0;
    ChirpCurve, "Chirp curve", 2, exp(0.3, 3.0, Unit::None), 1.0;

    FullMode, "At full charge", 3, ch(&FULL_MODES), 1.0;
    FlutterRate, "Flutter rate", 3, exp(2.0, 40.0, Unit::Hz), 14.0;
    FlutterSemis, "Flutter interval", 3, int(1, 12), 7.0;
    FullBoost, "Full volume", 3, lin(0.0, 1.0, Unit::Percent), 1.0;

    BlastWave, "Wave", 4, ch(&WAVES), 0.0;
    BlastStart, "Start pitch", 4, exp(60.0, 8000.0, Unit::Hz), 2400.0;
    BlastEnd, "End pitch", 4, exp(20.0, 4000.0, Unit::Hz), 180.0;
    SweepTime, "Sweep time", 4, exp(0.02, 2.0, Unit::Seconds), 0.22;
    SweepCurve, "Sweep curve", 4, exp(0.3, 4.0, Unit::None), 1.6;
    Length, "Length", 4, exp(0.03, 4.0, Unit::Seconds), 0.35;
    Hold, "Hold", 4, lin(0.0, 2.0, Unit::Seconds), 0.0;
    Attack, "Attack", 4, exp(0.0005, 0.2, Unit::Seconds), 0.001;
    Body, "Body", 4, lin(0.0, 1.0, Unit::Percent), 0.4;
    BodySemis, "Body interval", 4, int(-24, 24), -12.0;
    FmIndex, "FM", 4, lin(0.0, 1.0, Unit::Percent), 0.0;
    Width, "Pulse width", 4, lin(0.05, 0.5, Unit::Percent), 0.25;
    Punch, "Punch", 4, lin(0.0, 1.0, Unit::Percent), 0.3;

    PowerCurve, "Power curve", 6, exp(0.3, 3.0, Unit::None), 1.0;
    PowerPitch, "Size: pitch", 6, lin(-2.0, 3.0, Unit::None), 0.5;
    PowerLength, "Size: length", 6, lin(0.0, 6.0, Unit::None), 2.0;
    PowerNoise, "Size: noise", 6, lin(0.0, 1.0, Unit::Percent), 0.5;
    PowerVolume, "Size: volume", 6, lin(0.0, 1.0, Unit::Percent), 0.5;

    Noise, "Noise", 7, lin(0.0, 1.0, Unit::Percent), 0.3;
    NoiseDecay, "Noise decay", 7, exp(0.01, 3.0, Unit::Seconds), 0.12;
    NoiseHP, "Noise low cut", 7, exp(20.0, 12000.0, Unit::Hz), 800.0;
    NoiseLP, "Noise high cut", 7, exp(200.0, 16000.0, Unit::Hz), 9000.0;
    NoiseSweep, "Noise sweep", 7, lin(-4.0, 4.0, Unit::None), -1.0;

    Bits, "Bits", 8, int(3, 16), 16.0;
    Rate, "Sample rate", 8, exp(1000.0, 48000.0, Unit::Hz), 48000.0;
    Drive, "Drive", 8, lin(0.0, 1.0, Unit::Percent), 0.2;
    EchoMix, "Echo", 8, lin(0.0, 1.0, Unit::Percent), 0.0;
    EchoTime, "Echo time", 8, exp(40.0, 800.0, Unit::Ms), 220.0;
    EchoFb, "Echo feedback", 8, lin(0.0, 0.9, Unit::Percent), 0.4;
    ChirpCount, "Chirps per charge", 2, int(0, 16), 0.0;
    ChirpGrow, "Chirps deepen", 2, lin(0.0, 1.0, Unit::Percent), 0.0;
    HoldTop, "Hold at top", 3, lin(0.0, 1.0, Unit::Percent), 0.0;
    RippleDepth, "Ripple depth", 3, lin(0.0, 1.0, Unit::Percent), 0.6;
    ChgLevel, "Charge level", 1, lin(0.0, 1.0, Unit::Percent), 1.0;
    ToneLevel, "Tone level", 4, lin(0.0, 1.0, Unit::Percent), 1.0;
    NoiseSweepTime, "Noise sweep time", 7, lin(0.0, 3.0, Unit::Seconds), 0.0;
    NoiseEnd, "Noise ends", 7, lin(0.0, 3.0, Unit::Seconds), 0.0;
    NoiseSweepCurve, "Noise sweep curve", 7, exp(0.3, 4.0, Unit::None), 1.0;
    SubCharge, "Sub (charge)", 5, lin(0.0, 1.0, Unit::Percent), 0.0;
    SubBlast, "Sub (blast)", 5, lin(0.0, 1.0, Unit::Percent), 0.0;
    SubWave, "Sub wave", 5, ch(&SUB_WAVES), 0.0;
    SubOctave, "Sub octave", 5, ch(&SUB_OCTAVES), 0.0;
}

