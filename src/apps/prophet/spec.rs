use crate::apps::mi_kit::Spec;

pub const N: usize = 55;
pub const A_TUNE: usize = 0;
pub const A_SAW: usize = 1;
pub const A_PULSE: usize = 2;
pub const A_PW: usize = 3;
pub const SYNC: usize = 4;
pub const B_TUNE: usize = 5;
pub const B_FINE: usize = 6;
pub const B_SAW: usize = 7;
pub const B_PULSE: usize = 8;
pub const B_TRI: usize = 9;
pub const B_PW: usize = 10;
pub const B_LOW: usize = 11;
pub const B_KEY: usize = 12;
pub const MIX_A: usize = 13;
pub const MIX_B: usize = 14;
pub const NOISE: usize = 15;
pub const CUTOFF: usize = 16;
pub const RES: usize = 17;
pub const ENV_AMT: usize = 18;
pub const KEYTRACK: usize = 19;
pub const DRIVE: usize = 20;
pub const FA: usize = 21;
pub const FD: usize = 22;
pub const FS: usize = 23;
pub const FR: usize = 24;
pub const AA: usize = 25;
pub const AD: usize = 26;
pub const AS: usize = 27;
pub const AR: usize = 28;
pub const LFO_RATE: usize = 29;
pub const LFO_SHAPE: usize = 30;
pub const WHEEL_NOISE: usize = 31;
pub const W_FREQ: usize = 32;
pub const W_PW: usize = 33;
pub const W_FILTER: usize = 34;
pub const MOD_AMOUNT: usize = 35;
pub const POLY_ENV: usize = 36;
pub const POLY_B: usize = 37;
pub const P_FREQ: usize = 38;
pub const P_PW: usize = 39;
pub const P_FILTER: usize = 40;
pub const MODE: usize = 41;
pub const VOICES: usize = 42;
pub const UNISON_DETUNE: usize = 43;
pub const GLIDE: usize = 44;
pub const VINTAGE: usize = 45;
pub const VELOCITY: usize = 46;
pub const SPREAD: usize = 47;
pub const VOLUME: usize = 48;
pub const CHORUS: usize = 49;
pub const DELAY_TIME: usize = 50;
pub const DELAY_FB: usize = 51;
pub const DELAY_MIX: usize = 52;
pub const OCTAVE: usize = 53;
pub const BEND_RANGE: usize = 54;

pub fn seconds(x: f32) -> f32 {
    0.001 * 8000.0f32.powf(x.clamp(0.0, 1.0))
}
pub fn time_norm(t: f32) -> f32 {
    (t.max(0.001) / 0.001).ln() / 8000.0f32.ln()
}
pub fn cutoff(x: f32) -> f32 {
    20.0 * 900.0f32.powf(x.clamp(0.0, 1.0))
}
pub fn cutoff_norm(hz: f32) -> f32 {
    (hz / 20.0).ln() / 900.0f32.ln()
}
pub fn rate(x: f32) -> f32 {
    0.05 * 400.0f32.powf(x.clamp(0.0, 1.0))
}
fn time_text(x: f32) -> String {
    let s = seconds(x);
    if s < 1.0 {
        format!("{:.0} ms", s * 1000.0)
    } else {
        format!("{s:.2} s")
    }
}
fn hz_text(x: f32) -> String {
    format!("{:.0} Hz", cutoff(x))
}
fn rate_text(x: f32) -> String {
    format!("{:.2} Hz", rate(x))
}
fn integer(x: f32) -> String {
    format!("{x:+.0}")
}
fn bipolar(x: f32) -> String {
    format!("{:+.0}%", x * 100.0)
}
fn delay_text(x: f32) -> String {
    format!("{:.0} ms", 30.0 + x * 720.0)
}

const OFF_ON: &[&str] = &["Off", "On"];
pub static SPECS: [Spec; N] = [
    Spec::range("A Tune", -24.0, 24.0, 0.0, integer),
    Spec::switch("A Saw", OFF_ON, 1),
    Spec::switch("A Pulse", OFF_ON, 0),
    Spec::knob("A Pulse Width", 0.5),
    Spec::switch("A Sync to B", OFF_ON, 0),
    Spec::range("B Tune", -24.0, 24.0, 0.0, integer),
    Spec::range("B Fine", -50.0, 50.0, 5.0, integer),
    Spec::switch("B Saw", OFF_ON, 1),
    Spec::switch("B Pulse", OFF_ON, 0),
    Spec::switch("B Triangle", OFF_ON, 0),
    Spec::knob("B Pulse Width", 0.5),
    Spec::switch("B Low Frequency", OFF_ON, 0),
    Spec::switch("B Keyboard", OFF_ON, 1),
    Spec::knob("Oscillator A", 0.65),
    Spec::knob("Oscillator B", 0.60),
    Spec::knob("Noise", 0.0),
    Spec::range("Cutoff", 0.0, 1.0, 0.72, hz_text),
    Spec::knob("Resonance", 0.12),
    Spec::range("Filter Env Amount", -1.0, 1.0, 0.35, bipolar),
    Spec::knob("Filter Key Track", 0.65),
    Spec::knob("Drive", 0.10),
    Spec::range("Filter Attack", 0.0, 1.0, 0.27, time_text),
    Spec::range("Filter Decay", 0.0, 1.0, 0.64, time_text),
    Spec::knob("Filter Sustain", 0.4),
    Spec::range("Filter Release", 0.0, 1.0, 0.59, time_text),
    Spec::range("Amp Attack", 0.0, 1.0, 0.27, time_text),
    Spec::range("Amp Decay", 0.0, 1.0, 0.68, time_text),
    Spec::knob("Amp Sustain", 0.8),
    Spec::range("Amp Release", 0.0, 1.0, 0.59, time_text),
    Spec::range("LFO Rate", 0.0, 1.0, 0.67, rate_text),
    Spec::switch(
        "LFO Shape",
        &["Triangle", "Saw", "Square", "Sine", "Random"],
        0,
    ),
    Spec::knob("Wheel Noise Mix", 0.0),
    Spec::switch("Wheel Frequency", OFF_ON, 1),
    Spec::switch("Wheel Pulse Width", OFF_ON, 0),
    Spec::switch("Wheel Filter", OFF_ON, 0),
    Spec::knob("Mod Amount", 0.0),
    Spec::range("Poly-Mod Env", -1.0, 1.0, 0.0, bipolar),
    Spec::knob("Poly-Mod B", 0.0),
    Spec::switch("Poly-Mod A Freq", OFF_ON, 0),
    Spec::switch("Poly-Mod A PW", OFF_ON, 0),
    Spec::switch("Poly-Mod Filter", OFF_ON, 0),
    Spec::switch("Voice Mode", &["Poly", "Mono", "Unison"], 0),
    Spec::switch("Voice Count", &["4", "5", "8"], 1),
    Spec::knob("Unison Detune", 0.18),
    Spec::knob("Glide", 0.0),
    Spec::knob("Vintage", 0.18),
    Spec::knob("Velocity", 0.35),
    Spec::knob("Stereo Spread", 0.4),
    Spec::knob("Volume", 0.70),
    Spec::knob("Chorus", 0.0),
    Spec::range("Delay Time", 0.0, 1.0, 0.40, delay_text),
    Spec::knob("Delay Feedback", 0.25),
    Spec::knob("Delay Mix", 0.0),
    Spec::range("Pad Octave", -3.0, 3.0, 0.0, integer),
    Spec::range("Bend Range", 1.0, 12.0, 2.0, integer),
];

pub const GROUPS: &[(&str, &[usize])] = &[
    ("OSC A", &[A_TUNE, A_PW, A_SAW, A_PULSE, SYNC]),
    (
        "OSC B",
        &[B_TUNE, B_FINE, B_PW, B_SAW, B_PULSE, B_TRI, B_LOW, B_KEY],
    ),
    ("MIXER", &[MIX_A, MIX_B, NOISE, DRIVE]),
    ("FILTER", &[CUTOFF, RES, ENV_AMT, KEYTRACK]),
    ("ENVELOPES", &[FA, FD, FS, FR, AA, AD, AS, AR]),
    ("POLY MOD", &[POLY_ENV, POLY_B, P_FREQ, P_PW, P_FILTER]),
    (
        "WHEEL MOD",
        &[
            LFO_RATE,
            LFO_SHAPE,
            MOD_AMOUNT,
            WHEEL_NOISE,
            W_FREQ,
            W_PW,
            W_FILTER,
        ],
    ),
    (
        "PERFORM / FX",
        &[
            MODE,
            VOICES,
            UNISON_DETUNE,
            GLIDE,
            VINTAGE,
            VELOCITY,
            SPREAD,
            VOLUME,
            CHORUS,
            DELAY_TIME,
            DELAY_FB,
            DELAY_MIX,
            OCTAVE,
            BEND_RANGE,
        ],
    ),
];
