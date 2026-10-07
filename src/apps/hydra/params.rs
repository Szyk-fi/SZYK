//! Hydra's parameter table: every setting the synth has, in one place.
//!
//! The menu, the play view's dials, the modulation inputs, the instrument
//! settings other apps list, Moments and the factory presets are all driven
//! from this table, so adding a parameter is one line here (plus whatever
//! the DSP does with it).
//!
//! GENERATED once from a short script, then maintained by hand.

#![allow(non_camel_case_types)]

/// How a value is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    None,
    Percent,
    /// Percent, shown with its sign (-100% .. +100%).
    Signed,
    Hz,
    Seconds,
    Ms,
    Cents,
    Bpm,
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// A continuous value; `exp` spaces it exponentially (min must be > 0).
    Float { min: f32, max: f32, exp: bool, unit: Unit },
    Choice(&'static [&'static str]),
    Toggle,
    Int { min: i32, max: i32 },
}

pub const fn lin(min: f32, max: f32, unit: Unit) -> Kind {
    Kind::Float { min, max, exp: false, unit }
}
pub const fn exp(min: f32, max: f32, unit: Unit) -> Kind {
    Kind::Float { min, max, exp: true, unit }
}
pub const fn ch(names: &'static [&'static str]) -> Kind {
    Kind::Choice(names)
}
pub const fn tog() -> Kind {
    Kind::Toggle
}
pub const fn int(min: i32, max: i32) -> Kind {
    Kind::Int { min, max }
}

pub struct Def {
    pub name: &'static str,
    pub group: usize,
    pub kind: Kind,
    pub default: f32,
}

/// Menu sections, in order. A parameter's `group` indexes this.
pub const GROUPS: [&str; 20] = ["Main", "Osc A", "Osc B", "Osc C", "Mixer", "Filter 1", "Filter 2", "Amp Env", "Env 2", "Env 3", "LFO 1", "LFO 2", "Mod Matrix", "Drive", "Chorus", "Phaser", "Delay", "Reverb", "Arp", "Macros"];

/// What a mod matrix slot can read.
pub const SOURCES: [&str; 14] = ["Off", "Amp Env", "Env 2", "Env 3", "LFO 1", "LFO 2", "Velocity", "Key", "Mod Wheel", "Random", "Macro 1", "Macro 2", "Macro 3", "Macro 4"];
pub const SRC_OFF: usize = 0;
pub const SRC_AMP_ENV: usize = 1;
pub const SRC_ENV2: usize = 2;
pub const SRC_ENV3: usize = 3;
pub const SRC_LFO1: usize = 4;
pub const SRC_LFO2: usize = 5;
pub const SRC_VELOCITY: usize = 6;
pub const SRC_KEY: usize = 7;
pub const SRC_MOD_WHEEL: usize = 8;
pub const SRC_RANDOM: usize = 9;
pub const SRC_MACRO1: usize = 10;

/// What a mod matrix slot can move (per voice).
pub const TARGETS: [&str; 32] = [
    "Off", "Pitch", "Pitch A", "Pitch B", "Pitch C", "A P1", "A P2", "A P3", "B P1", "B P2", "B P3", "C P1", "C P2", "C P3", "A Level", "B Level", "C Level", "Sub level", "Ring", "Noise", "F1 Cutoff", "F1 Res", "F1 Drive", "F2 Cutoff", "F2 Res", "F2 Drive", "Amp", "Pan", "LFO1 Rate", "LFO2 Rate", "Uni detune", "Cross mod",
];
pub const T_OFF: usize = 0;
pub const T_PITCH: usize = 1;
pub const T_PITCH_A: usize = 2;
pub const T_A_P1: usize = 5;
pub const T_A_LEVEL: usize = 14;
pub const T_SUB: usize = 17;
pub const T_RING: usize = 18;
pub const T_NOISE: usize = 19;
pub const T_F1_CUT: usize = 20;
pub const T_F1_RES: usize = 21;
pub const T_F1_DRIVE: usize = 22;
pub const T_F2_CUT: usize = 23;
pub const T_F2_RES: usize = 24;
pub const T_F2_DRIVE: usize = 25;
pub const T_AMP: usize = 26;
pub const T_PAN: usize = 27;
pub const T_LFO1_RATE: usize = 28;
pub const T_LFO2_RATE: usize = 29;
pub const T_UNI_DETUNE: usize = 30;
pub const T_XMOD: usize = 31;

macro_rules! params {
    ($( $id:ident, $name:expr, $group:expr, $kind:expr, $def:expr; )*) => {
        /// A parameter's index into the table (and into a snapshot).
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(usize)]
        pub enum P { $( $id, )* }
        pub const COUNT: usize = [$( P::$id ),*].len();
        pub static DEFS: [Def; COUNT] = [ $( Def { name: $name, group: $group, kind: $kind, default: $def } ),* ];
    };
}

params! {
    Level, "Level", 0, lin(0.0, 1.0, Unit::Percent), 0.7;
    Voices, "Voices", 0, int(1, 16), 8.0;
    Mode, "Mode", 0, ch(&["Poly", "Mono", "Legato"]), 0.0;
    Glide, "Glide", 0, exp(0.001, 2.0, Unit::Seconds), 0.001;
    Bend, "Bend range", 0, int(0, 24), 2.0;
    Tune, "Tune", 0, lin(-100.0, 100.0, Unit::Cents), 0.0;
    Octave, "Octave", 0, int(-3, 3), 0.0;
    VelSens, "Velocity", 0, lin(0.0, 1.0, Unit::Percent), 0.6;
    Unison, "Unison", 0, int(1, 7), 1.0;
    UniDetune, "Uni detune", 0, lin(0.0, 1.0, Unit::Percent), 0.25;
    UniSpread, "Uni spread", 0, lin(0.0, 1.0, Unit::Percent), 0.5;
    Drift, "Drift", 0, lin(0.0, 1.0, Unit::Percent), 0.1;
    XMod, "Cross mod A>B", 0, lin(0.0, 1.0, Unit::Percent), 0.0;
    KeySync, "Key sync", 0, tog(), 0.0;
    Scale, "Pad scale", 0, int(0, 15), 0.0;
    Root, "Pad root", 0, int(0, 11), 0.0;
    ModWheel, "Mod wheel", 0, lin(0.0, 1.0, Unit::Percent), 0.0;
    A_Type, "Type", 1, ch(&["Analog", "Wavetable", "FM", "Pluck", "Noise"]), 0.0;
    A_Level, "Level", 1, lin(0.0, 1.0, Unit::Percent), 0.8;
    A_Coarse, "Coarse", 1, int(-24, 24), 0.0;
    A_Fine, "Fine", 1, lin(-100.0, 100.0, Unit::Cents), 0.0;
    A_P1, "P1", 1, lin(0.0, 1.0, Unit::Percent), 0.667;
    A_P2, "P2", 1, lin(0.0, 1.0, Unit::Percent), 0.5;
    A_P3, "P3", 1, lin(0.0, 1.0, Unit::Percent), 0.0;
    A_Pan, "Pan", 1, lin(-1.0, 1.0, Unit::Signed), 0.0;
    B_Type, "Type", 2, ch(&["Analog", "Wavetable", "FM", "Pluck", "Noise"]), 0.0;
    B_Level, "Level", 2, lin(0.0, 1.0, Unit::Percent), 0.0;
    B_Coarse, "Coarse", 2, int(-24, 24), 0.0;
    B_Fine, "Fine", 2, lin(-100.0, 100.0, Unit::Cents), 0.0;
    B_P1, "P1", 2, lin(0.0, 1.0, Unit::Percent), 0.667;
    B_P2, "P2", 2, lin(0.0, 1.0, Unit::Percent), 0.5;
    B_P3, "P3", 2, lin(0.0, 1.0, Unit::Percent), 0.0;
    B_Pan, "Pan", 2, lin(-1.0, 1.0, Unit::Signed), 0.0;
    B_Sync, "Sync to A", 2, tog(), 0.0;
    C_Type, "Type", 3, ch(&["Analog", "Wavetable", "FM", "Pluck", "Noise"]), 0.0;
    C_Level, "Level", 3, lin(0.0, 1.0, Unit::Percent), 0.0;
    C_Coarse, "Coarse", 3, int(-24, 24), 0.0;
    C_Fine, "Fine", 3, lin(-100.0, 100.0, Unit::Cents), 0.0;
    C_P1, "P1", 3, lin(0.0, 1.0, Unit::Percent), 0.667;
    C_P2, "P2", 3, lin(0.0, 1.0, Unit::Percent), 0.5;
    C_P3, "P3", 3, lin(0.0, 1.0, Unit::Percent), 0.0;
    C_Pan, "Pan", 3, lin(-1.0, 1.0, Unit::Signed), 0.0;
    SubLevel, "Sub level", 4, lin(0.0, 1.0, Unit::Percent), 0.0;
    SubShape, "Sub shape", 4, ch(&["Sine", "Square"]), 0.0;
    SubOct, "Sub octave", 4, ch(&["-1 oct", "-2 oct"]), 0.0;
    Ring, "Ring A x B", 4, lin(0.0, 1.0, Unit::Percent), 0.0;
    Noise, "Noise", 4, lin(0.0, 1.0, Unit::Percent), 0.0;
    NoiseColor, "Noise color", 4, lin(0.0, 1.0, Unit::Percent), 0.5;
    F1_Type, "Type", 5, ch(&["LP12", "LP24", "HP12", "HP24", "BP", "Notch", "Peak", "Comb"]), 1.0;
    F1_Cut, "Cutoff", 5, exp(16.0, 22000.0, Unit::Hz), 8000.0;
    F1_Res, "Resonance", 5, lin(0.0, 1.0, Unit::Percent), 0.1;
    F1_Drive, "Drive", 5, lin(0.0, 1.0, Unit::Percent), 0.0;
    F1_Key, "Key track", 5, lin(-1.0, 1.0, Unit::Signed), 0.5;
    F1_Env, "Env 2 amount", 5, lin(-1.0, 1.0, Unit::Signed), 0.0;
    Route, "Routing", 5, ch(&["Serial", "Parallel"]), 0.0;
    FMix, "Parallel mix", 5, lin(0.0, 1.0, Unit::Percent), 0.5;
    F2_Type, "Type", 6, ch(&["Ladder LP24", "Ladder LP12", "Ladder BP", "Ladder HP"]), 0.0;
    F2_Cut, "Cutoff", 6, exp(16.0, 22000.0, Unit::Hz), 22000.0;
    F2_Res, "Resonance", 6, lin(0.0, 1.0, Unit::Percent), 0.0;
    F2_Drive, "Drive", 6, lin(0.0, 1.0, Unit::Percent), 0.0;
    F2_Key, "Key track", 6, lin(-1.0, 1.0, Unit::Signed), 0.0;
    F2_Env, "Env 2 amount", 6, lin(-1.0, 1.0, Unit::Signed), 0.0;
    AmpA, "Attack", 7, exp(0.0005, 10.0, Unit::Seconds), 0.003;
    AmpD, "Decay", 7, exp(0.001, 10.0, Unit::Seconds), 0.4;
    AmpS, "Sustain", 7, lin(0.0, 1.0, Unit::Percent), 0.8;
    AmpR, "Release", 7, exp(0.005, 15.0, Unit::Seconds), 0.25;
    E2A, "Attack", 8, exp(0.0005, 10.0, Unit::Seconds), 0.003;
    E2D, "Decay", 8, exp(0.001, 10.0, Unit::Seconds), 0.5;
    E2S, "Sustain", 8, lin(0.0, 1.0, Unit::Percent), 0.0;
    E2R, "Release", 8, exp(0.005, 15.0, Unit::Seconds), 0.3;
    E3A, "Attack", 9, exp(0.0005, 10.0, Unit::Seconds), 0.003;
    E3D, "Decay", 9, exp(0.001, 10.0, Unit::Seconds), 0.5;
    E3S, "Sustain", 9, lin(0.0, 1.0, Unit::Percent), 0.0;
    E3R, "Release", 9, exp(0.005, 15.0, Unit::Seconds), 0.3;
    L1_Shape, "Shape", 10, ch(&["Sine", "Triangle", "Saw", "Square", "Sample & hold", "Smooth random"]), 0.0;
    L1_Rate, "Rate", 10, exp(0.02, 40.0, Unit::Hz), 4.0;
    L1_Mode, "Mode", 10, ch(&["Free", "Key"]), 0.0;
    L1_Fade, "Fade in", 10, lin(0.0, 5.0, Unit::Seconds), 0.0;
    L1_Phase, "Start phase", 10, lin(0.0, 1.0, Unit::Percent), 0.0;
    L2_Shape, "Shape", 11, ch(&["Sine", "Triangle", "Saw", "Square", "Sample & hold", "Smooth random"]), 1.0;
    L2_Rate, "Rate", 11, exp(0.02, 40.0, Unit::Hz), 0.5;
    L2_Mode, "Mode", 11, ch(&["Free", "Key"]), 0.0;
    L2_Fade, "Fade in", 11, lin(0.0, 5.0, Unit::Seconds), 0.0;
    L2_Phase, "Start phase", 11, lin(0.0, 1.0, Unit::Percent), 0.0;
    M1_Src, "1 source", 12, ch(&SOURCES), 0.0;
    M1_Dst, "1 target", 12, ch(&TARGETS), 0.0;
    M1_Amt, "1 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    M2_Src, "2 source", 12, ch(&SOURCES), 0.0;
    M2_Dst, "2 target", 12, ch(&TARGETS), 0.0;
    M2_Amt, "2 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    M3_Src, "3 source", 12, ch(&SOURCES), 0.0;
    M3_Dst, "3 target", 12, ch(&TARGETS), 0.0;
    M3_Amt, "3 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    M4_Src, "4 source", 12, ch(&SOURCES), 0.0;
    M4_Dst, "4 target", 12, ch(&TARGETS), 0.0;
    M4_Amt, "4 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    M5_Src, "5 source", 12, ch(&SOURCES), 0.0;
    M5_Dst, "5 target", 12, ch(&TARGETS), 0.0;
    M5_Amt, "5 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    M6_Src, "6 source", 12, ch(&SOURCES), 0.0;
    M6_Dst, "6 target", 12, ch(&TARGETS), 0.0;
    M6_Amt, "6 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    M7_Src, "7 source", 12, ch(&SOURCES), 0.0;
    M7_Dst, "7 target", 12, ch(&TARGETS), 0.0;
    M7_Amt, "7 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    M8_Src, "8 source", 12, ch(&SOURCES), 0.0;
    M8_Dst, "8 target", 12, ch(&TARGETS), 0.0;
    M8_Amt, "8 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    M9_Src, "9 source", 12, ch(&SOURCES), 0.0;
    M9_Dst, "9 target", 12, ch(&TARGETS), 0.0;
    M9_Amt, "9 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    M10_Src, "10 source", 12, ch(&SOURCES), 0.0;
    M10_Dst, "10 target", 12, ch(&TARGETS), 0.0;
    M10_Amt, "10 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    M11_Src, "11 source", 12, ch(&SOURCES), 0.0;
    M11_Dst, "11 target", 12, ch(&TARGETS), 0.0;
    M11_Amt, "11 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    M12_Src, "12 source", 12, ch(&SOURCES), 0.0;
    M12_Dst, "12 target", 12, ch(&TARGETS), 0.0;
    M12_Amt, "12 amount", 12, lin(-1.0, 1.0, Unit::Signed), 0.0;
    Drv_Type, "Type", 13, ch(&["Off", "Soft", "Hard", "Fold", "Crush"]), 0.0;
    Drv_Amt, "Amount", 13, lin(0.0, 1.0, Unit::Percent), 0.3;
    Drv_Mix, "Mix", 13, lin(0.0, 1.0, Unit::Percent), 1.0;
    Cho_Rate, "Rate", 14, exp(0.05, 5.0, Unit::Hz), 0.4;
    Cho_Depth, "Depth", 14, lin(0.0, 1.0, Unit::Percent), 0.5;
    Cho_Mix, "Mix", 14, lin(0.0, 1.0, Unit::Percent), 0.0;
    Ph_Rate, "Rate", 15, exp(0.02, 8.0, Unit::Hz), 0.3;
    Ph_Depth, "Depth", 15, lin(0.0, 1.0, Unit::Percent), 0.6;
    Ph_Fb, "Feedback", 15, lin(0.0, 1.0, Unit::Percent), 0.4;
    Ph_Mix, "Mix", 15, lin(0.0, 1.0, Unit::Percent), 0.0;
    Dly_Time, "Time", 16, exp(10.0, 1500.0, Unit::Ms), 350.0;
    Dly_Fb, "Feedback", 16, lin(0.0, 0.95, Unit::Percent), 0.35;
    Dly_Tone, "Tone", 16, lin(0.0, 1.0, Unit::Percent), 0.6;
    Dly_Mix, "Mix", 16, lin(0.0, 1.0, Unit::Percent), 0.0;
    Dly_Ping, "Ping-pong", 16, tog(), 1.0;
    Rev_Size, "Size", 17, lin(0.0, 1.0, Unit::Percent), 0.6;
    Rev_Decay, "Decay", 17, exp(0.1, 20.0, Unit::Seconds), 3.0;
    Rev_Damp, "Damping", 17, lin(0.0, 1.0, Unit::Percent), 0.4;
    Rev_Mix, "Mix", 17, lin(0.0, 1.0, Unit::Percent), 0.0;
    Rev_Pre, "Pre-delay", 17, lin(0.0, 200.0, Unit::Ms), 10.0;
    Arp_On, "Arpeggiator", 18, tog(), 0.0;
    Arp_Mode, "Pattern", 18, ch(&["Up", "Down", "Up/Down", "Random", "As played"]), 0.0;
    Arp_Rate, "Rate", 18, ch(&["1/4", "1/8", "1/8T", "1/16", "1/16T", "1/32"]), 3.0;
    Arp_Oct, "Octaves", 18, int(1, 4), 1.0;
    Arp_Gate, "Gate", 18, lin(0.05, 1.0, Unit::Percent), 0.5;
    Arp_Bpm, "Tempo", 18, lin(40.0, 240.0, Unit::Bpm), 120.0;
    Arp_Swing, "Swing", 18, lin(0.0, 0.5, Unit::Percent), 0.0;
    Mac1, "Macro 1", 19, lin(0.0, 1.0, Unit::Percent), 0.0;
    Mac2, "Macro 2", 19, lin(0.0, 1.0, Unit::Percent), 0.0;
    Mac3, "Macro 3", 19, lin(0.0, 1.0, Unit::Percent), 0.0;
    Mac4, "Macro 4", 19, lin(0.0, 1.0, Unit::Percent), 0.0;
}
