//! Oracle's block library: the stateful DSP building blocks a patch wires
//! together. Every block takes up to `MAX_IN` evaluated inputs per sample
//! and returns one value.
//!
//! `SPECS` is the single source of truth for what exists: the validator
//! checks patches against it and the AI's system prompt is generated from
//! it, so adding a block here teaches the Oracle about it automatically.
//!
//! Real-time rules: buffers are allocated in `Block::new` (UI/worker
//! thread) and never resized; every read is modulo a non-zero length;
//! every block is stable for any input (coefficients clamped).

use super::expr::white;
use std::f32::consts::{PI, TAU};

pub const MAX_IN: usize = 8;

/// Per-sample context shared by every block.
pub struct Ctx {
    pub sr: f32,
    pub inv_sr: f32,
    /// Song position in beats (advances only while the transport runs).
    pub beat: f64,
}

pub struct InputSpec {
    pub name: &'static str,
    /// Default expression when the patch omits this input, in voice scope.
    pub voice_default: &'static str,
    /// ...and in global scope.
    pub global_default: &'static str,
    pub doc: &'static str,
}

pub struct OptSpec {
    pub name: &'static str,
    /// Allowed string values; empty = numeric (or a number list for `values`).
    pub choices: &'static [&'static str],
    pub default: &'static str,
    pub doc: &'static str,
}

pub struct BlockSpec {
    pub name: &'static str,
    pub summary: &'static str,
    /// Output range description for the AI.
    pub output: &'static str,
    pub inputs: &'static [InputSpec],
    pub opts: &'static [OptSpec],
}

const fn i(name: &'static str, d: &'static str, doc: &'static str) -> InputSpec {
    InputSpec { name, voice_default: d, global_default: d, doc }
}
const fn ig(name: &'static str, voice: &'static str, global: &'static str, doc: &'static str) -> InputSpec {
    InputSpec { name, voice_default: voice, global_default: global, doc }
}

pub const SPECS: &[BlockSpec] = &[
    BlockSpec {
        name: "osc",
        summary: "band-limited oscillator",
        output: "-1..1",
        inputs: &[
            ig("freq", "pitch", "110", "frequency in Hz"),
            i("pm", "0", "phase modulation in cycles (FM: feed another osc * index)"),
            i("pw", "0.5", "pulse width 0..1 (pulse wave only)"),
            i("sync", "0", "hard-sync: phase resets when this rises above 0.5"),
        ],
        opts: &[OptSpec { name: "wave", choices: &["sine", "tri", "saw", "square", "pulse"], default: "sine", doc: "" }],
    },
    BlockSpec {
        name: "supersaw",
        summary: "7 detuned band-limited saws",
        output: "about -1..1",
        inputs: &[
            ig("freq", "pitch", "110", "centre frequency Hz"),
            i("detune", "0.3", "spread 0..1"),
            i("mix", "0.7", "side saws level 0..1"),
        ],
        opts: &[],
    },
    BlockSpec {
        name: "noise",
        summary: "noise source",
        output: "-1..1",
        inputs: &[],
        opts: &[OptSpec { name: "color", choices: &["white", "pink", "brown"], default: "white", doc: "" }],
    },
    BlockSpec {
        name: "filter",
        summary: "state-variable filter (clean, stable at any setting)",
        output: "follows input",
        inputs: &[
            i("in", "0", "signal"),
            i("cutoff", "1000", "cutoff Hz (20..20000)"),
            i("res", "0.2", "resonance 0..1 (0.95+ rings)"),
        ],
        opts: &[OptSpec {
            name: "mode",
            choices: &["lp", "hp", "bp", "notch", "peak"],
            default: "lp",
            doc: "",
        }],
    },
    BlockSpec {
        name: "ladder",
        summary: "4-pole saturating ladder lowpass (warm, acid)",
        output: "follows input, saturates",
        inputs: &[
            i("in", "0", "signal"),
            i("cutoff", "800", "cutoff Hz"),
            i("res", "0.3", "resonance 0..1.1 (self-oscillates near 1)"),
            i("drive", "1", "input drive 1..10"),
        ],
        opts: &[],
    },
    BlockSpec {
        name: "adsr",
        summary: "ADSR envelope",
        output: "0..1",
        inputs: &[
            ig("gate", "gate", "padgate", "on while > 0.5"),
            i("a", "0.01", "attack s"),
            i("d", "0.2", "decay s"),
            i("s", "0.7", "sustain level 0..1"),
            i("r", "0.3", "release s"),
        ],
        opts: &[],
    },
    BlockSpec {
        name: "ad",
        summary: "one-shot attack/decay envelope (drums, plucks, pings)",
        output: "0..1",
        inputs: &[
            ig("trig", "trig", "padtrig", "fires when this rises above 0.5"),
            i("a", "0.002", "attack s"),
            i("d", "0.3", "decay s"),
        ],
        opts: &[OptSpec { name: "curve", choices: &["exp", "lin"], default: "exp", doc: "decay shape" }],
    },
    BlockSpec {
        name: "lfo",
        summary: "low-frequency oscillator",
        output: "-1..1",
        inputs: &[i("rate", "1", "Hz"), i("phase", "0", "phase offset in cycles")],
        opts: &[
            OptSpec {
                name: "shape",
                choices: &["sine", "tri", "saw", "square", "random", "smooth"],
                default: "sine",
                doc: "random = stepped S&H, smooth = interpolated random",
            },
            OptSpec { name: "retrig", choices: &["false", "true"], default: "false", doc: "voice scope: restart on each note" },
        ],
    },
    BlockSpec {
        name: "delay",
        summary: "feedback delay line with damping (output is the wet signal only)",
        output: "follows input",
        inputs: &[
            i("in", "0", "signal"),
            i("time", "0.3", "delay time s (up to max)"),
            i("fb", "0.4", "feedback 0..0.98"),
            i("damp", "0.3", "high-cut in the loop 0..1"),
        ],
        opts: &[OptSpec { name: "max", choices: &[], default: "1", doc: "max delay s, 0.01..4 (memory)" }],
    },
    BlockSpec {
        name: "comb",
        summary: "tuned comb / Karplus-Strong resonator: excite with noise bursts for plucked strings",
        output: "follows input, rings",
        inputs: &[
            i("in", "0", "exciter signal"),
            ig("freq", "pitch", "220", "resonant pitch Hz"),
            i("fb", "0.98", "feedback -0.999..0.999 (length of ring)"),
            i("damp", "0.3", "brightness loss 0..1"),
        ],
        opts: &[],
    },
    BlockSpec {
        name: "allpass",
        summary: "allpass diffuser (smears transients, phasers)",
        output: "follows input",
        inputs: &[i("in", "0", "signal"), i("time", "0.01", "delay s up to 0.1"), i("g", "0.6", "coefficient -0.95..0.95")],
        opts: &[],
    },
    BlockSpec {
        name: "reverb",
        summary: "lush 8-line feedback-delay-network reverb (output is wet only)",
        output: "follows input",
        inputs: &[
            i("in", "0", "signal"),
            i("size", "0.7", "room size / decay 0..1 (1 = near infinite)"),
            i("damp", "0.4", "high damping 0..1"),
            i("mod", "0.3", "internal chorus 0..1"),
        ],
        opts: &[],
    },
    BlockSpec {
        name: "shift",
        summary: "granular pitch shifter (shimmer, octaves, detune)",
        output: "follows input",
        inputs: &[i("in", "0", "signal"), i("semis", "12", "shift in semitones -24..24"), i("window", "0.06", "grain s 0.01..0.2")],
        opts: &[],
    },
    BlockSpec {
        name: "sh",
        summary: "sample & hold",
        output: "held input",
        inputs: &[i("in", "noise()", "value to sample"), i("trig", "0", "samples when this rises above 0.5")],
        opts: &[],
    },
    BlockSpec {
        name: "slew",
        summary: "slew limiter / portamento / lag",
        output: "follows input",
        inputs: &[i("in", "0", "signal"), i("rise", "0.05", "rise time s"), i("fall", "0.05", "fall time s")],
        opts: &[],
    },
    BlockSpec {
        name: "chaos",
        summary: "chaotic map (smoothly interpolated)",
        output: "-1..1",
        inputs: &[i("rate", "4", "iterations per second"), i("r", "0.5", "chaos amount 0..1")],
        opts: &[OptSpec { name: "map", choices: &["logistic", "henon", "lorenz"], default: "logistic", doc: "" }],
    },
    BlockSpec {
        name: "crush",
        summary: "bit and sample-rate reducer",
        output: "follows input",
        inputs: &[i("in", "0", "signal"), i("bits", "8", "bit depth 1..16"), i("rate", "8000", "sample rate Hz")],
        opts: &[],
    },
    BlockSpec {
        name: "follower",
        summary: "envelope follower (ducking, auto-wah, dynamics)",
        output: "0..~1",
        inputs: &[ig("in", "0", "in", "signal to follow"), i("attack", "0.01", "s"), i("release", "0.15", "s")],
        opts: &[],
    },
    BlockSpec {
        name: "clock",
        summary: "tempo-synced gate clock (follows Tempo and Start/Stop)",
        output: "0 or 1 (gate)",
        inputs: &[
            i("div", "0.25", "beats per tick (1 = quarter, 0.25 = 16th)"),
            i("width", "0.5", "gate length 0..1 of a tick"),
            i("swing", "0", "delay of every other tick 0..0.5"),
            i("rate", "0", "if > 0: free-running Hz instead of tempo"),
        ],
        opts: &[],
    },
    BlockSpec {
        name: "seq",
        summary: "step sequencer: advances one step per rising clock edge",
        output: "current step value",
        inputs: &[i("clock", "0", "advance on rising edge"), i("reset", "0", "back to step 1 on rising edge")],
        opts: &[
            OptSpec { name: "values", choices: &[], default: "[0]", doc: "list of 1..32 numbers, e.g. [0, 3, 7, 12]" },
            OptSpec { name: "mode", choices: &["fwd", "rev", "pingpong", "random"], default: "fwd", doc: "" },
        ],
    },
    BlockSpec {
        name: "euclid",
        summary: "Euclidean rhythm gate: passes the clock only on hit steps",
        output: "0 or 1 (gate)",
        inputs: &[
            i("clock", "0", "clock gate"),
            i("steps", "16", "pattern length 1..32"),
            i("pulses", "5", "hits 0..steps"),
            i("rotate", "0", "rotation in steps"),
        ],
        opts: &[],
    },
    // ---------------------------------------------------- platform v2
    BlockSpec {
        name: "wavetable",
        summary: "morphing wavetable oscillator (8 frames, band-limited)",
        output: "-1..1",
        inputs: &[
            ig("freq", "pitch", "110", "frequency in Hz"),
            i("pos", "0", "position through the table 0..1 (morphs between frames)"),
            i("pm", "0", "phase modulation in cycles"),
            i("warp", "0", "phase bend 0..1 (brighter, phase-distortion style)"),
        ],
        opts: &[OptSpec { name: "table", choices: &["analog", "vocal", "glass", "metal", "digital"], default: "analog", doc: "analog: sine>tri>saw>square>pulses; vocal: vowels a-e-i-o-u; glass: sparse bell partials; metal: harsh odd/prime partials; digital: stepped random spectra" }],
    },
    BlockSpec {
        name: "fold",
        summary: "wavefolder (adds rich, bright harmonics as amount rises)",
        output: "-1..1",
        inputs: &[i("in", "0", "signal"), i("amount", "2", "fold amount 1..10"), i("bias", "0", "offset -1..1 (asymmetry, even harmonics)")],
        opts: &[],
    },
    BlockSpec {
        name: "drive",
        summary: "saturation / distortion with a tone control",
        output: "-1..1",
        inputs: &[i("in", "0", "signal"), i("gain", "4", "input gain 1..40"), i("tone", "0.7", "brightness after the distortion 0..1")],
        opts: &[OptSpec { name: "mode", choices: &["soft", "hard", "tube", "rectify"], default: "soft", doc: "" }],
    },
    BlockSpec {
        name: "chorus",
        summary: "three-voice chorus (output includes the dry signal by mix)",
        output: "follows input",
        inputs: &[i("in", "0", "signal"), i("rate", "0.4", "LFO Hz"), i("depth", "0.5", "0..1"), i("mix", "0.5", "wet 0..1")],
        opts: &[],
    },
    BlockSpec {
        name: "modal",
        summary: "modal resonator: 8 tuned modes rung by an exciter (strings, bars, bells, plates)",
        output: "follows exciter, rings",
        inputs: &[
            i("in", "0", "exciter: noise bursts, clicks, any audio"),
            ig("freq", "pitch", "220", "fundamental Hz"),
            i("structure", "0.3", "0 harmonic (string) .. 0.5 bar .. 1 plate/bell"),
            i("damp", "0.4", "0 rings long .. 1 dead"),
            i("bright", "0.6", "upper modes level 0..1"),
        ],
        opts: &[],
    },
    BlockSpec {
        name: "grain",
        summary: "live granular: grains from the last 2 s of its input (put it in global scope)",
        output: "follows input",
        inputs: &[
            i("in", "0", "signal to record and granulate"),
            i("pos", "0.2", "how far back grains start 0..1 of 2 s"),
            i("size", "0.08", "grain length s 0.01..0.5"),
            i("density", "12", "grains per second 0..100"),
            i("pitch", "0", "grain pitch in semitones -24..24"),
            i("spread", "0.2", "randomness of position and pitch 0..1"),
            i("freeze", "0", "stop recording while > 0.5 (hold the texture)"),
        ],
        opts: &[],
    },
    BlockSpec {
        name: "sampler",
        summary: "sample player (WAV from the SD card's samples folder)",
        output: "-1..1",
        inputs: &[
            ig("freq", "pitch", "261.63", "playback pitch Hz (the root note plays at its own pitch)"),
            ig("trig", "trig", "padtrig", "restart on a rising edge"),
            i("start", "0", "start point 0..1"),
            i("end", "1", "end point 0..1"),
            i("rev", "0", "reverse while > 0.5"),
            i("loop", "0", "loop between start and end while > 0.5"),
        ],
        opts: &[
            OptSpec { name: "file", choices: &[], default: "", doc: "WAV path, relative to samples/" },
            OptSpec { name: "root", choices: &[], default: "60", doc: "MIDI note the sample was recorded at" },
        ],
    },
    BlockSpec {
        name: "plaits",
        summary: "a Mutable Instruments Plaits voice (the real DSP): 24 engines with their own envelope",
        output: "-1..1",
        inputs: &[
            ig("freq", "pitch", "110", "pitch Hz"),
            i("harm", "0.5", "harmonics 0..1"),
            i("timbre", "0.5", "timbre 0..1"),
            i("morph", "0.5", "morph 0..1"),
            i("decay", "0.5", "internal envelope decay 0..1"),
            ig("gate", "gate", "padgate", "strikes on a rising edge, holds while > 0.5"),
        ],
        opts: &[OptSpec {
            name: "engine",
            choices: &PLAITS_ENGINES,
            default: "va",
            doc: "",
        }],
    },
    BlockSpec {
        name: "pitch",
        summary: "pitch tracker for monophonic input (hum, sing, play into it)",
        output: "Hz (holds the last pitch found)",
        inputs: &[ig("in", "0", "in", "signal"), i("min", "60", "lowest Hz"), i("max", "1000", "highest Hz")],
        opts: &[],
    },
    BlockSpec {
        name: "onset",
        summary: "onset detector: a 5 ms trigger on each new hit or note",
        output: "0 or 1 (trigger)",
        inputs: &[ig("in", "0", "in", "signal"), i("sens", "0.5", "sensitivity 0..1")],
        opts: &[],
    },
];

/// Relative CPU cost of one sample of a block, in units of one `osc`
/// (measured on the desktop sim with `cargo test -- --ignored
/// block_costs --nocapture`; ratios carry over to the device better than
/// absolute times). Unknown kinds count as 1.
pub fn cost(name: &str) -> f32 {
    // Measured October 2026 (desktop sim, see `block_costs`), rounded up;
    // content-dependent blocks (grain density, modal retuning while
    // modulated) are costed at a busy setting.
    match name {
        "noise" | "adsr" | "ad" | "sh" | "chaos" | "crush" | "seq" | "sampler" => 0.6,
        "osc" | "lfo" | "clock" | "euclid" | "allpass" | "pitch" | "fold" => 1.0,
        "filter" | "comb" => 1.3,
        // was 12x before the rational tanh (fast_tanh) replaced libm's
        "ladder" => 1.5,
        "slew" | "follower" | "delay" | "onset" => 1.8,
        "wavetable" | "drive" => 2.3,
        "supersaw" => 3.0,
        "modal" => 3.0,
        "shift" | "chorus" => 4.0,
        "grain" => 5.0,
        "plaits" => 7.0,
        "reverb" => 14.0,
        _ => 1.0,
    }
}

pub fn spec(name: &str) -> Option<&'static BlockSpec> {
    SPECS.iter().find(|s| s.name == name)
}

// ------------------------------------------------------------ helpers

#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
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

/// tanh to within about 0.5% (a rational approximation, exact at 0 and
/// saturating at +-3): several times cheaper than `f32::tanh`.
#[inline]
pub fn fast_tanh(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

#[inline]
fn rising(prev: &mut f32, now: f32) -> bool {
    let r = now > 0.5 && *prev <= 0.5;
    *prev = now;
    r
}

#[inline]
fn onepole_coef(seconds: f32, inv_sr: f32) -> f32 {
    1.0 - (-inv_sr / seconds.max(1e-4)).exp()
}

/// A ring buffer read with linear interpolation.
pub struct Ring {
    buf: Vec<f32>,
    w: usize,
}

impl Ring {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(4)], w: 0 }
    }
    #[inline]
    fn push(&mut self, x: f32) {
        let len = self.buf.len();
        self.buf[self.w % len] = x;
        self.w = (self.w + 1) % len;
    }
    /// Sample `delay` samples ago (fractional), clamped to the buffer.
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
    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|x| *x = 0.0);
    }
}


// ------------------------------------------------------- wavetables

/// Plaits engine names for the `plaits` block, in Plaits' own order.
pub const PLAITS_ENGINES: [&str; 24] = [
    "va_vcf", "phase_dist", "fm6_a", "fm6_b", "fm6_c", "wave_terrain", "string_machine", "chiptune", "va", "waveshaper", "fm", "grain",
    "additive", "wavetable", "chord", "speech", "swarm", "noise", "particle", "string", "modal", "bass_drum", "snare_drum", "hi_hat",
];

pub const WT_TABLES: [&str; 5] = ["analog", "vocal", "glass", "metal", "digital"];
const WT_FRAMES: usize = 8;
const WT_LEN: usize = 1024;
/// Harmonic limits of the three band-limited versions of every frame.
const WT_MIPS: [usize; 3] = [64, 16, 4];

/// Amplitude of harmonic `h` (1-based) in frame `k` of table `t`.
fn wt_amp(t: usize, k: usize, h: usize) -> f32 {
    let hf = h as f32;
    let x = k as f32 / (WT_FRAMES - 1) as f32;
    let odd = h % 2 == 1;
    match t {
        // analog: sine, triangle, saw, square, pulse 25%, pulse 10%,
        // saw + octave, saw with a resonant bump
        0 => match k {
            0 => if h == 1 { 1.0 } else { 0.0 },
            1 => if odd { (if (h / 2) % 2 == 0 { 1.0 } else { -1.0 }) / (hf * hf) } else { 0.0 },
            2 => 1.0 / hf,
            3 => if odd { 1.0 / hf } else { 0.0 },
            4 => (std::f32::consts::PI * hf * 0.25).sin() / hf,
            5 => (std::f32::consts::PI * hf * 0.1).sin() / hf,
            6 => (1.0 + if h % 2 == 0 { 0.6 } else { 0.0 }) / hf,
            _ => (1.0 + 2.5 * (-(hf - 7.0) * (hf - 7.0) / 6.0).exp()) / hf,
        },
        // vocal: two formant bumps walking a, e, i, o, u
        1 => {
            const F: [(f32, f32); 8] = [(800.0, 1150.0), (600.0, 1600.0), (400.0, 2100.0), (350.0, 2400.0), (300.0, 2700.0), (400.0, 1400.0), (450.0, 800.0), (325.0, 700.0)];
            let (f1, f2) = F[k.min(7)];
            let f0 = 130.0;
            let bump = |f: f32, w: f32| (-((hf - f / f0) / w).powi(2)).exp();
            (bump(f1, 1.4) + 0.6 * bump(f2, 2.2) + 0.15) / hf.sqrt()
        }
        // glass: sparse Fibonacci partials, brighter along the table
        2 => {
            const FIB: [usize; 9] = [1, 2, 3, 5, 8, 13, 21, 34, 55];
            if FIB.contains(&h) { (-(hf) / (3.0 + x * 40.0)).exp() / hf.powf(0.5) } else { 0.0 }
        }
        // metal: primes loud, the rest quiet, highs rising
        3 => {
            let prime = h == 2 || (h > 2 && (2..h).take_while(|d| d * d <= h).all(|d| h % d != 0));
            (if prime || h == 1 { 1.0 } else { 0.12 }) / hf.powf(1.1 - 0.6 * x) * if h % 4 == 3 { -1.0 } else { 1.0 }
        }
        // digital: deterministic random spectra, wider along the table
        _ => {
            let mut s = (k as u32 + 1).wrapping_mul(2_654_435_761) ^ (h as u32).wrapping_mul(40_503);
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let r = (s % 1000) as f32 / 1000.0;
            if h <= 6 + k * 8 { r * r * r / hf.sqrt() } else { 0.0 }
        }
    }
}

/// All tables, built once (on the UI thread, the first time a patch
/// uses one): [table][mip][frame][sample].
fn wavetables() -> &'static [f32] {
    static WT: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    WT.get_or_init(|| {
        let mut out = vec![0.0f32; WT_TABLES.len() * WT_MIPS.len() * WT_FRAMES * WT_LEN];
        for t in 0..WT_TABLES.len() {
            for (m, &hmax) in WT_MIPS.iter().enumerate() {
                for k in 0..WT_FRAMES {
                    let base = ((t * WT_MIPS.len() + m) * WT_FRAMES + k) * WT_LEN;
                    let amps: Vec<f32> = (1..=hmax).map(|h| wt_amp(t, k, h)).collect();
                    let frame = &mut out[base..base + WT_LEN];
                    for (n, s) in frame.iter_mut().enumerate() {
                        let ph = n as f32 / WT_LEN as f32 * TAU;
                        *s = amps.iter().enumerate().map(|(i, a)| a * ((i + 1) as f32 * ph).sin()).sum();
                    }
                    let peak = frame.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
                    frame.iter_mut().for_each(|s| *s /= peak);
                }
            }
        }
        out
    })
}

#[inline]
fn wt_read(frame: &[f32], p: f32) -> f32 {
    let x = p * WT_LEN as f32;
    let i = x as usize % WT_LEN;
    let f = x - x.floor();
    frame[i] * (1.0 - f) + frame[(i + 1) % WT_LEN] * f
}

// ------------------------------------------------------------- modal

pub const MODES: usize = 8;
const RATIOS_HARMONIC: [f32; MODES] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
/// Free-free bar modes (ratios of the Euler-Bernoulli beam).
const RATIOS_BAR: [f32; MODES] = [1.0, 2.756, 5.404, 8.933, 13.344, 18.638, 24.814, 31.873];
/// A plate/bell-like cluster.
const RATIOS_PLATE: [f32; MODES] = [1.0, 1.594, 2.136, 2.296, 2.653, 2.918, 3.156, 3.501];

pub struct Modal {
    y1: [f32; MODES],
    y2: [f32; MODES],
    a1: [f32; MODES],
    a2: [f32; MODES],
    b: [f32; MODES],
    /// Inputs the coefficients were computed for.
    last: [f32; 4],
}

impl Modal {
    fn new() -> Self {
        Self { y1: [0.0; MODES], y2: [0.0; MODES], a1: [0.0; MODES], a2: [0.0; MODES], b: [0.0; MODES], last: [-1.0; 4] }
    }
    fn retune(&mut self, freq: f32, structure: f32, damp: f32, bright: f32, sr: f32) {
        let s = structure.clamp(0.0, 1.0);
        let base_t60 = 0.05 + (1.0 - damp.clamp(0.0, 1.0)).powi(2) * 6.0;
        for k in 0..MODES {
            let ratio = if s < 0.5 {
                let u = s * 2.0;
                RATIOS_HARMONIC[k] * (1.0 - u) + RATIOS_BAR[k] * u
            } else {
                let u = (s - 0.5) * 2.0;
                RATIOS_BAR[k] * (1.0 - u) + RATIOS_PLATE[k] * u
            };
            let f = freq.abs() * ratio;
            if f >= sr * 0.45 || f < 10.0 {
                self.b[k] = 0.0;
                self.a1[k] = 0.0;
                self.a2[k] = 0.0;
                continue;
            }
            let w = TAU * f / sr;
            let t60 = base_t60 / (1.0 + k as f32 * 0.35);
            let r = 10f32.powf(-3.0 / (t60 * sr)).min(0.99999);
            let level = (1.0 - (1.0 - bright.clamp(0.0, 1.0)) * k as f32 / MODES as f32).max(0.0).powi(2);
            self.a1[k] = 2.0 * r * w.cos();
            self.a2[k] = r * r;
            // roughly level-matched: louder input = louder ring, without
            // long decays exploding
            self.b[k] = (1.0 - r * r).sqrt() * w.sin().max(0.05) * level;
        }
    }
}

// ----------------------------------------------------------- granular

const GRAINS: usize = 16;

#[derive(Clone, Copy, Default)]
struct Grain {
    on: bool,
    /// Read position, absolute index into the ring (fractional).
    pos: f32,
    rate: f32,
    age: f32,
    len: f32,
}

pub struct Granular {
    ring: Vec<f32>,
    w: usize,
    grains: [Grain; GRAINS],
    acc: f32,
    rng: u32,
}

// ------------------------------------------------------------ samples

/// Where sample files live (`sampler` file option is relative to this).
pub fn samples_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/samples"))
}

/// A loaded sample: mono frames and the rate it was recorded at.
pub struct SampleData {
    pub frames: Vec<f32>,
    pub rate: f32,
}

/// Loads (once, then shared by every voice and patch) a WAV file.
/// Call off the audio thread.
pub fn load_sample(file: &str) -> Result<std::sync::Arc<SampleData>, String> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<SampleData>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(s) = cache.lock().unwrap().get(file) {
        return Ok(Arc::clone(s));
    }
    let path = {
        let p = std::path::Path::new(file);
        if p.is_absolute() { p.to_path_buf() } else { samples_dir().join(p) }
    };
    let mut r = hound::WavReader::open(&path).map_err(|e| format!("sample \"{file}\": {e}"))?;
    let spec = r.spec();
    let ch = spec.channels.max(1) as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1i64 << (spec.bits_per_sample.clamp(1, 32) - 1)) as f32;
            r.samples::<i32>().filter_map(Result::ok).map(|s| s as f32 * scale).collect()
        }
    };
    let frames: Vec<f32> = raw.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
    if frames.is_empty() {
        return Err(format!("sample \"{file}\" is empty"));
    }
    let data = Arc::new(SampleData { frames, rate: spec.sample_rate as f32 });
    cache.lock().unwrap().insert(file.to_string(), Arc::clone(&data));
    Ok(data)
}

// -------------------------------------------------------------- blocks

#[derive(Clone, Copy, PartialEq)]
pub enum Wave {
    Sine,
    Tri,
    Saw,
    Square,
    Pulse,
}

pub enum Block {
    Osc { wave: Wave, phase: f32, prev_sync: f32, tri_state: f32 },
    SuperSaw { phases: [f32; 7] },
    Noise { color: u8, b: [f32; 7] },
    Filter { mode: u8, ic1: f32, ic2: f32 },
    Ladder { s: [f32; 4], fc: f32, g: f32 },
    Adsr { stage: u8, level: f32, prev: f32 },
    Ad { exp: bool, stage: u8, level: f32, prev: f32 },
    Lfo { shape: u8, retrig: bool, phase: f32, held: f32, next: f32 },
    Delay { ring: Ring, lp: f32 },
    Comb { ring: Ring, lp: f32 },
    Allpass { ring: Ring },
    Reverb(Box<Fdn>),
    Shift { ring: Ring, phase: f32 },
    Sh { held: f32, prev: f32 },
    Slew { y: f32 },
    Chaos { map: u8, x: f32, y: f32, z: f32, a: f32, b: f32, t: f32 },
    Crush { held: f32, acc: f32 },
    Follower { env: f32 },
    Clock { phase: f64 },
    Seq { values: Vec<f32>, mode: u8, idx: usize, dir: i32, prev_clk: f32, prev_rst: f32, rng: u32 },
    Euclid { step: usize, prev_clk: f32, hit: bool },
    Wavetable { table: usize, phase: f32 },
    Fold,
    Drive { mode: u8, lp: f32 },
    Chorus { ring: Ring, phase: f32 },
    Modal(Box<Modal>),
    Grain(Box<Granular>),
    Sampler { data: Option<std::sync::Arc<SampleData>>, root: f32, pos: f64, playing: bool, prev_trig: f32 },
    Plaits { voice: Box<crate::plaits_ffi::PlaitsVoice>, engine: i32, prev_gate: f32 },
    Pitch { lp: f32, prev: f32, last_cross: f32, t: f32, period: f32, hz: f32, env: f32 },
    Onset { fast: f32, slow: f32, hold: f32, refractory: f32 },
}

pub struct Fdn {
    lines: [Ring; 8],
    base: [f32; 8],
    lp: [f32; 8],
    diff: [Ring; 2],
    lfo: f32,
}

/// Option values handed to `Block::new`, already validated.
pub struct Opts<'a> {
    pub get: &'a dyn Fn(&str) -> Option<String>,
    pub values: Vec<f32>,
}

impl Block {
    /// Build a block. Allocates -- call on the UI/worker thread only.
    pub fn new(kind: &str, opts: &Opts, sr: f32) -> Option<Block> {
        let choice = |name: &str, list: &[&str]| -> u8 {
            let v = (opts.get)(name).unwrap_or_default();
            list.iter().position(|c| *c == v).unwrap_or(0) as u8
        };
        Some(match kind {
            "osc" => {
                let wave = match (opts.get)("wave").as_deref() {
                    Some("tri") => Wave::Tri,
                    Some("saw") => Wave::Saw,
                    Some("square") => Wave::Square,
                    Some("pulse") => Wave::Pulse,
                    _ => Wave::Sine,
                };
                Block::Osc { wave, phase: 0.0, prev_sync: 0.0, tri_state: 0.0 }
            }
            "supersaw" => Block::SuperSaw { phases: [0.0, 0.31, 0.57, 0.13, 0.79, 0.42, 0.91] },
            "noise" => Block::Noise { color: choice("color", &["white", "pink", "brown"]), b: [0.0; 7] },
            "filter" => Block::Filter { mode: choice("mode", &["lp", "hp", "bp", "notch", "peak"]), ic1: 0.0, ic2: 0.0 },
            "ladder" => Block::Ladder { s: [0.0; 4], fc: -1.0, g: 0.0 },
            "adsr" => Block::Adsr { stage: 0, level: 0.0, prev: 0.0 },
            "ad" => Block::Ad { exp: choice("curve", &["exp", "lin"]) == 0, stage: 0, level: 0.0, prev: 0.0 },
            "lfo" => Block::Lfo {
                shape: choice("shape", &["sine", "tri", "saw", "square", "random", "smooth"]),
                retrig: (opts.get)("retrig").as_deref() == Some("true"),
                phase: 0.0,
                held: 0.0,
                next: 0.0,
            },
            "delay" => {
                let max: f32 = (opts.get)("max").and_then(|s| s.parse().ok()).unwrap_or(1.0);
                let max = max.clamp(0.01, 4.0);
                Block::Delay { ring: Ring::new((max * sr) as usize + 8), lp: 0.0 }
            }
            "comb" => Block::Comb { ring: Ring::new((sr / 20.0) as usize + 8), lp: 0.0 },
            "allpass" => Block::Allpass { ring: Ring::new((0.1 * sr) as usize + 8) },
            "reverb" => {
                let scale = sr / 44_100.0;
                let base = [1557.0, 1617.0, 1491.0, 1422.0, 1277.0, 1356.0, 1188.0, 1116.0].map(|b: f32| b * scale);
                let lines = base.map(|b| Ring::new((b * 2.2) as usize + 64));
                let diff = [Ring::new((0.013 * sr) as usize + 8), Ring::new((0.021 * sr) as usize + 8)];
                Block::Reverb(Box::new(Fdn { lines, base, lp: [0.0; 8], diff, lfo: 0.0 }))
            }
            "shift" => Block::Shift { ring: Ring::new((0.45 * sr) as usize + 8), phase: 0.0 },
            "sh" => Block::Sh { held: 0.0, prev: 0.0 },
            "slew" => Block::Slew { y: 0.0 },
            "chaos" => Block::Chaos {
                map: choice("map", &["logistic", "henon", "lorenz"]),
                x: 0.4,
                y: 0.1,
                z: 0.1,
                a: 0.0,
                b: 0.0,
                t: 0.0,
            },
            "crush" => Block::Crush { held: 0.0, acc: 1.0 },
            "follower" => Block::Follower { env: 0.0 },
            "clock" => Block::Clock { phase: 0.0 },
            "seq" => Block::Seq {
                values: if opts.values.is_empty() { vec![0.0] } else { opts.values.clone() },
                mode: choice("mode", &["fwd", "rev", "pingpong", "random"]),
                idx: 0,
                dir: 1,
                prev_clk: 0.0,
                prev_rst: 0.0,
                rng: 0x1234_5678,
            },
            "euclid" => Block::Euclid { step: 0, prev_clk: 0.0, hit: false },
            "wavetable" => {
                wavetables(); // build the tables here, not on the audio thread
                Block::Wavetable { table: choice("table", &WT_TABLES) as usize, phase: 0.0 }
            }
            "fold" => Block::Fold,
            "drive" => Block::Drive { mode: choice("mode", &["soft", "hard", "tube", "rectify"]), lp: 0.0 },
            "chorus" => Block::Chorus { ring: Ring::new((0.05 * sr) as usize + 8), phase: 0.0 },
            "modal" => Block::Modal(Box::new(Modal::new())),
            "grain" => Block::Grain(Box::new(Granular { ring: vec![0.0; (2.0 * sr) as usize + 8], w: 0, grains: [Grain::default(); GRAINS], acc: 0.0, rng: 0x51ed_270b })),
            "sampler" => {
                let file = (opts.get)("file").unwrap_or_default();
                let data = if file.is_empty() { None } else { load_sample(&file).ok() };
                let root: f32 = (opts.get)("root").and_then(|s| s.parse().ok()).unwrap_or(60.0);
                Block::Sampler { data, root: root.clamp(0.0, 127.0), pos: 0.0, playing: false, prev_trig: 0.0 }
            }
            "plaits" => Block::Plaits { voice: Box::new(crate::plaits_ffi::PlaitsVoice::new()), engine: choice("engine", &PLAITS_ENGINES) as i32, prev_gate: 0.0 },
            "pitch" => Block::Pitch { lp: 0.0, prev: 0.0, last_cross: 0.0, t: 0.0, period: 0.0, hz: 0.0, env: 0.0 },
            "onset" => Block::Onset { fast: 0.0, slow: 0.0, hold: 0.0, refractory: 0.0 },
            _ => return None,
        })
    }

    /// A voice note-on: restart what should restart.
    pub fn note_on(&mut self) {
        if let Block::Lfo { retrig: true, phase, .. } = self {
            *phase = 0.0;
        }
    }

    /// Clear state (used when a block produced a non-finite value).
    pub fn reset(&mut self) {
        match self {
            Block::Osc { phase, tri_state, .. } => {
                *phase = 0.0;
                *tri_state = 0.0;
            }
            Block::Noise { b, .. } => *b = [0.0; 7],
            Block::Filter { ic1, ic2, .. } => {
                *ic1 = 0.0;
                *ic2 = 0.0;
            }
            Block::Ladder { s, .. } => *s = [0.0; 4],
            Block::Delay { ring, lp } | Block::Comb { ring, lp } => {
                ring.clear();
                *lp = 0.0;
            }
            Block::Allpass { ring } | Block::Shift { ring, .. } => ring.clear(),
            Block::Reverb(f) => {
                f.lines.iter_mut().for_each(Ring::clear);
                f.diff.iter_mut().for_each(Ring::clear);
                f.lp = [0.0; 8];
            }
            Block::Slew { y } => *y = 0.0,
            Block::Chaos { x, y, z, .. } => {
                *x = 0.4;
                *y = 0.1;
                *z = 0.1;
            }
            Block::Follower { env } => *env = 0.0,
            Block::Wavetable { phase, .. } => *phase = 0.0,
            Block::Drive { lp, .. } => *lp = 0.0,
            Block::Chorus { ring, .. } => ring.clear(),
            Block::Modal(m) => {
                m.y1 = [0.0; MODES];
                m.y2 = [0.0; MODES];
            }
            Block::Grain(g) => {
                g.ring.iter_mut().for_each(|x| *x = 0.0);
                g.grains = [Grain::default(); GRAINS];
            }
            Block::Sampler { playing, .. } => *playing = false,
            Block::Pitch { lp, hz, period, .. } => {
                *lp = 0.0;
                *hz = 0.0;
                *period = 0.0;
            }
            Block::Onset { fast, slow, .. } => {
                *fast = 0.0;
                *slow = 0.0;
            }
            _ => {}
        }
    }

    #[inline]
    pub fn tick(&mut self, raw: &[f32; MAX_IN], c: &Ctx, rng: &mut u32) -> f32 {
        // Sanitize once so no block ever sees NaN/inf or absurd magnitudes.
        let mut v = [0.0f32; MAX_IN];
        for (d, s) in v.iter_mut().zip(raw.iter()) {
            *d = if s.is_finite() { s.clamp(-1e6, 1e6) } else { 0.0 };
        }
        let v = &v;
        match self {
            Block::Osc { wave, phase, prev_sync, tri_state } => {
                let freq = v[0].clamp(-20_000.0, 20_000.0);
                let dt = (freq * c.inv_sr).abs().min(0.5);
                if rising(prev_sync, v[3]) {
                    *phase = 0.0;
                }
                let p = {
                    let x = *phase + v[1];
                    x - x.floor()
                };
                let out = match wave {
                    Wave::Sine => (p * TAU).sin(),
                    Wave::Saw => 2.0 * p - 1.0 - poly_blep(p, dt),
                    Wave::Square | Wave::Pulse | Wave::Tri => {
                        let pw = if *wave == Wave::Pulse { v[2].clamp(0.02, 0.98) } else { 0.5 };
                        let mut sq = if p < pw { 1.0 } else { -1.0 };
                        sq += poly_blep(p, dt);
                        let q = {
                            let x = p - pw + 1.0;
                            x - x.floor()
                        };
                        sq -= poly_blep(q, dt);
                        if *wave == Wave::Tri {
                            // leaky-integrated square = band-limited triangle
                            *tri_state = *tri_state * 0.999 + sq * 4.0 * dt;
                            tri_state.clamp(-1.0, 1.0)
                        } else {
                            sq
                        }
                    }
                };
                *phase += freq * c.inv_sr;
                *phase -= phase.floor();
                out
            }
            Block::SuperSaw { phases } => {
                const RATIO: [f32; 7] = [-0.11, -0.063, -0.019, 0.0, 0.02, 0.0645, 0.108];
                let freq = v[0].clamp(0.0, 20_000.0);
                let detune = v[1].clamp(0.0, 1.0);
                let side = v[2].clamp(0.0, 1.0);
                let mut sum = 0.0;
                for (k, ph) in phases.iter_mut().enumerate() {
                    let f = freq * (1.0 + RATIO[k] * detune);
                    let dt = (f * c.inv_sr).min(0.5);
                    let s = 2.0 * *ph - 1.0 - poly_blep(*ph, dt);
                    sum += if k == 3 { s } else { s * side };
                    *ph += dt;
                    *ph -= ph.floor();
                }
                sum / (1.0 + 6.0 * side).sqrt()
            }
            Block::Noise { color, b } => {
                let w = white(rng);
                match color {
                    1 => {
                        b[0] = 0.99886 * b[0] + w * 0.0555179;
                        b[1] = 0.99332 * b[1] + w * 0.0750759;
                        b[2] = 0.96900 * b[2] + w * 0.153852;
                        b[3] = 0.86650 * b[3] + w * 0.3104856;
                        b[4] = 0.55000 * b[4] + w * 0.5329522;
                        b[5] = -0.7616 * b[5] - w * 0.0168980;
                        let p = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
                        b[6] = w * 0.115926;
                        p * 0.11
                    }
                    2 => {
                        b[0] = (b[0] + w * 0.02).clamp(-1.0, 1.0) * 0.998;
                        b[0] * 3.0
                    }
                    _ => w,
                }
            }
            Block::Filter { mode, ic1, ic2 } => {
                let fc = v[1].clamp(20.0, c.sr * 0.45);
                let g = (PI * fc * c.inv_sr).tan();
                let k = 2.0 - 2.0 * v[2].clamp(0.0, 0.995);
                let a1 = 1.0 / (1.0 + g * (g + k));
                let a2 = g * a1;
                let a3 = g * a2;
                let x = v[0];
                let v3 = x - *ic2;
                let v1 = a1 * *ic1 + a2 * v3;
                let v2 = *ic2 + a2 * *ic1 + a3 * v3;
                *ic1 = 2.0 * v1 - *ic1;
                *ic2 = 2.0 * v2 - *ic2;
                match mode {
                    1 => x - k * v1 - v2,
                    2 => v1,
                    3 => x - k * v1,
                    4 => x - k * v1 - 2.0 * v2,
                    _ => v2,
                }
            }
            Block::Ladder { s, fc, g } => {
                let f = v[1].clamp(20.0, c.sr * 0.42);
                if f != *fc {
                    *fc = f;
                    *g = 1.0 - (-TAU * f * c.inv_sr).exp();
                }
                let g = *g;
                let res = v[2].clamp(0.0, 1.1) * 4.0;
                let drive = v[3].clamp(0.1, 10.0);
                let x = fast_tanh(v[0] * drive - res * s[3]);
                // each stage's saturation once per sample (it was computed
                // twice): same filter, about half the work
                let t0 = fast_tanh(s[0]);
                let t1 = fast_tanh(s[1]);
                let t2 = fast_tanh(s[2]);
                let t3 = fast_tanh(s[3]);
                s[0] += g * (x - t0);
                s[1] += g * (t0 - t1);
                s[2] += g * (t1 - t2);
                s[3] += g * (t2 - t3);
                s[3] * (1.0 + res * 0.35) / drive.sqrt()
            }
            Block::Adsr { stage, level, prev } => {
                let gate = v[0] > 0.5;
                if gate && *prev <= 0.5 {
                    *stage = 1;
                }
                *prev = v[0];
                if !gate && *stage != 0 {
                    *stage = 4;
                }
                let sus = v[3].clamp(0.0, 1.0);
                match *stage {
                    1 => {
                        *level += c.inv_sr / v[1].max(1e-4);
                        if *level >= 0.999 {
                            *level = 1.0;
                            *stage = 2;
                        }
                    }
                    2 => {
                        *level += (sus - *level) * onepole_coef(v[2] * 0.3, c.inv_sr);
                        if (*level - sus).abs() < 1e-3 {
                            *stage = 3;
                        }
                    }
                    3 => *level = sus,
                    4 => {
                        *level -= *level * onepole_coef(v[4] * 0.3, c.inv_sr);
                        if *level < 1e-5 {
                            *level = 0.0;
                            *stage = 0;
                        }
                    }
                    _ => *level = 0.0,
                }
                *level
            }
            Block::Ad { exp, stage, level, prev } => {
                if rising(prev, v[0]) {
                    *stage = 1;
                }
                match *stage {
                    1 => {
                        *level += c.inv_sr / v[1].max(1e-4);
                        if *level >= 1.0 {
                            *level = 1.0;
                            *stage = 2;
                        }
                    }
                    2 => {
                        if *exp {
                            *level -= *level * onepole_coef(v[2] * 0.25, c.inv_sr);
                        } else {
                            *level -= c.inv_sr / v[2].max(1e-4);
                        }
                        if *level <= 1e-5 {
                            *level = 0.0;
                            *stage = 0;
                        }
                    }
                    _ => {}
                }
                *level
            }
            Block::Lfo { shape, phase, held, next, .. } => {
                let p = {
                    let x = *phase + v[1];
                    x - x.floor()
                };
                let out = match shape {
                    0 => (p * TAU).sin(),
                    1 => 1.0 - 4.0 * (p - 0.5).abs(),
                    2 => 2.0 * p - 1.0,
                    3 => {
                        if p < 0.5 {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    4 => *held,
                    _ => *held + (*next - *held) * (p * p * (3.0 - 2.0 * p)),
                };
                let prev_phase = *phase;
                *phase += v[0].clamp(0.0, 2000.0) * c.inv_sr;
                if *phase >= 1.0 || *phase < prev_phase {
                    *phase -= phase.floor();
                    *held = if *shape == 5 { *next } else { white(rng) };
                    *next = white(rng);
                }
                out
            }
            Block::Delay { ring, lp } => {
                let wet = ring.read(v[1].max(0.0) * c.sr);
                let damp = v[3].clamp(0.0, 0.99);
                *lp += (wet - *lp) * (1.0 - damp);
                ring.push(v[0] + *lp * v[2].clamp(-0.98, 0.98));
                wet
            }
            Block::Comb { ring, lp } => {
                let freq = v[1].clamp(20.0, c.sr * 0.45);
                let delayed = ring.read(c.sr / freq);
                let damp = v[3].clamp(0.0, 0.99);
                *lp += (delayed - *lp) * (1.0 - damp);
                let y = v[0] + *lp * v[2].clamp(-0.999, 0.999);
                ring.push(y.clamp(-8.0, 8.0));
                y
            }
            Block::Allpass { ring } => {
                let g = v[2].clamp(-0.95, 0.95);
                let d = ring.read(v[1].clamp(0.0, 0.099) * c.sr);
                let w = v[0] + g * d;
                ring.push(w);
                d - g * w
            }
            Block::Reverb(f) => {
                let size = v[1].clamp(0.0, 1.0);
                let fb = 0.55 + size * 0.44;
                let damp = v[2].clamp(0.0, 0.95);
                let depth = v[3].clamp(0.0, 1.0) * 12.0;
                f.lfo += 0.37 * c.inv_sr;
                f.lfo -= f.lfo.floor();
                // input diffusion
                let mut x = v[0] * 0.35;
                for (k, d) in f.diff.iter_mut().enumerate() {
                    let t = d.buf.len() as f32 - 6.0;
                    let dd = d.read(t);
                    let g = if k == 0 { 0.7 } else { 0.6 };
                    let w = x + g * dd;
                    d.push(w);
                    x = dd - g * w;
                }
                let stretch = 0.6 + size * 1.5;
                let mut outs = [0.0f32; 8];
                for k in 0..8 {
                    let m = ((f.lfo + k as f32 * 0.125) * TAU).sin() * depth;
                    outs[k] = f.lines[k].read(f.base[k] * stretch + m);
                }
                // Householder mix
                let sum: f32 = outs.iter().sum::<f32>() * 0.25;
                let mut wet = 0.0;
                for k in 0..8 {
                    let mixed = outs[k] - sum;
                    f.lp[k] += (mixed - f.lp[k]) * (1.0 - damp);
                    f.lines[k].push(x + f.lp[k] * fb);
                    wet += outs[k] * if k % 2 == 0 { 1.0 } else { -1.0 };
                }
                wet * 0.3
            }
            Block::Shift { ring, phase } => {
                ring.push(v[0]);
                let ratio = 2f32.powf(v[1].clamp(-24.0, 24.0) / 12.0);
                let win = (v[2].clamp(0.01, 0.2) * c.sr).max(8.0);
                *phase += (1.0 - ratio) / win;
                *phase -= phase.floor();
                let p2 = {
                    let x = *phase + 0.5;
                    x - x.floor()
                };
                let a = ring.read(*phase * win + 2.0);
                let b = ring.read(p2 * win + 2.0);
                let wa = (PI * *phase).sin();
                let wb = (PI * p2).sin();
                a * wa * wa + b * wb * wb
            }
            Block::Sh { held, prev } => {
                if rising(prev, v[1]) {
                    *held = v[0];
                }
                *held
            }
            Block::Slew { y } => {
                let t = if v[0] > *y { v[1] } else { v[2] };
                *y += (v[0] - *y) * onepole_coef(t, c.inv_sr);
                *y
            }
            Block::Chaos { map, x, y, z, a, b, t } => {
                let r = v[1].clamp(0.0, 1.0);
                if *map == 2 {
                    // Lorenz, integrated continuously
                    let dt = (v[0].clamp(0.0, 200.0) * c.inv_sr * 0.5).min(0.01);
                    let rho = 20.0 + r * 18.0;
                    let dx = 10.0 * (*y - *x);
                    let dy = *x * (rho - *z) - *y;
                    let dz = *x * *y - 8.0 / 3.0 * *z;
                    *x += dx * dt;
                    *y += dy * dt;
                    *z += dz * dt;
                    if !(x.is_finite() && y.is_finite() && z.is_finite()) || x.abs() > 100.0 {
                        *x = 0.1;
                        *y = 0.0;
                        *z = 0.0;
                    }
                    return (*x / 20.0).clamp(-1.0, 1.0);
                }
                *t += v[0].clamp(0.0, 5000.0) * c.inv_sr;
                if *t >= 1.0 {
                    *t -= t.floor();
                    *a = *b;
                    if *map == 0 {
                        let rr = 3.5 + r * 0.499;
                        *x = (rr * *x * (1.0 - *x)).clamp(1e-4, 1.0 - 1e-4);
                        *b = *x * 2.0 - 1.0;
                    } else {
                        let aa = 1.0 + r * 0.4;
                        let nx = 1.0 - aa * *x * *x + *y;
                        *y = 0.3 * *x;
                        *x = nx;
                        if !x.is_finite() || x.abs() > 2.0 {
                            *x = 0.1;
                            *y = 0.0;
                        }
                        *b = (*x / 1.3).clamp(-1.0, 1.0);
                    }
                }
                *a + (*b - *a) * *t
            }
            Block::Crush { held, acc } => {
                *acc += v[2].clamp(20.0, c.sr) * c.inv_sr;
                if *acc >= 1.0 {
                    *acc -= acc.floor();
                    let levels = 2f32.powf(v[1].clamp(1.0, 16.0) - 1.0);
                    *held = (v[0] * levels).round() / levels;
                }
                *held
            }
            Block::Follower { env } => {
                let x = v[0].abs();
                let t = if x > *env { v[1] } else { v[2] };
                *env += (x - *env) * onepole_coef(t, c.inv_sr);
                *env
            }
            Block::Clock { phase } => {
                let width = v[1].clamp(0.01, 0.99) as f64;
                let (pos, swing) = if v[3] > 0.0 {
                    *phase += (v[3].min(1000.0) * c.inv_sr) as f64;
                    if *phase > 1e6 {
                        *phase -= 1e6;
                    }
                    (*phase, 0.0)
                } else {
                    (c.beat / (v[0].clamp(1.0 / 64.0, 64.0) as f64), v[2].clamp(0.0, 0.5) as f64)
                };
                let n = pos.floor();
                let mut frac = pos - n;
                if (n as i64) % 2 == 1 {
                    frac -= swing;
                }
                if frac >= 0.0 && frac < width {
                    1.0
                } else {
                    0.0
                }
            }
            Block::Seq { values, mode, idx, dir, prev_clk, prev_rst, rng: srng } => {
                let n = values.len().max(1);
                if rising(prev_rst, v[1]) {
                    *idx = 0;
                    *dir = 1;
                }
                if rising(prev_clk, v[0]) {
                    match mode {
                        1 => *idx = (*idx + n - 1) % n,
                        2 => {
                            if n > 1 {
                                let next = *idx as i32 + *dir;
                                if next < 0 || next >= n as i32 {
                                    *dir = -*dir;
                                }
                                *idx = (*idx as i32 + *dir).clamp(0, n as i32 - 1) as usize;
                            }
                        }
                        3 => *idx = ((white(srng) * 0.5 + 0.5) * n as f32) as usize % n,
                        _ => *idx = (*idx + 1) % n,
                    }
                }
                values.get(*idx % n).copied().unwrap_or(0.0)
            }
            Block::Euclid { step, prev_clk, hit } => {
                let steps = (v[1].round() as i64).clamp(1, 32) as usize;
                let pulses = (v[2].round() as i64).clamp(0, steps as i64) as usize;
                let rot = v[3].round() as i64;
                if rising(prev_clk, v[0]) {
                    *step = (*step + 1) % steps;
                    let s = ((*step as i64 + rot).rem_euclid(steps as i64)) as usize;
                    // Bjorklund-equivalent: hit when the running bucket overflows
                    *hit = pulses > 0 && (s * pulses) % steps < pulses;
                }
                if v[0] > 0.5 && *hit {
                    1.0
                } else {
                    0.0
                }
            }
            Block::Wavetable { table, phase } => {
                let freq = v[0].clamp(-20_000.0, 20_000.0);
                let dt = (freq * c.inv_sr).abs().min(0.5);
                *phase += freq * c.inv_sr;
                *phase -= phase.floor();
                let mut p = *phase + v[2];
                p -= p.floor();
                let warp = v[3].clamp(0.0, 1.0);
                // phase bend, kept monotonic (slope 1 + 0.94 w cos >= 0.06)
                p += warp * 0.15 * (TAU * p).sin();
                p -= p.floor();
                let hmax = if dt > 0.0 { 0.45 / dt } else { f32::MAX };
                let mip = WT_MIPS.iter().position(|&h| (h as f32) <= hmax).unwrap_or(WT_MIPS.len() - 1);
                let pos = v[1].clamp(0.0, 1.0) * (WT_FRAMES - 1) as f32;
                let f0 = (pos as usize).min(WT_FRAMES - 1);
                let f1 = (f0 + 1).min(WT_FRAMES - 1);
                let fr = pos - f0 as f32;
                let wt = wavetables();
                let frame = |k: usize| {
                    let base = ((*table * WT_MIPS.len() + mip) * WT_FRAMES + k) * WT_LEN;
                    &wt[base..base + WT_LEN]
                };
                wt_read(frame(f0), p) * (1.0 - fr) + wt_read(frame(f1), p) * fr
            }
            Block::Fold => {
                let amount = v[1].clamp(1.0, 10.0);
                let bias = v[2].clamp(-1.0, 1.0);
                (std::f32::consts::FRAC_PI_2 * (v[0].clamp(-4.0, 4.0) * amount + bias)).sin()
            }
            Block::Drive { mode, lp } => {
                let x = v[0].clamp(-4.0, 4.0) * v[1].clamp(1.0, 40.0);
                let y = match mode {
                    1 => x.clamp(-1.0, 1.0),
                    2 => (x + 0.2 * x * x).tanh() - 0.05,
                    3 => x.abs().tanh() * 2.0 - 1.0,
                    _ => x.tanh(),
                };
                let fc = 500.0 * 32f32.powf(v[2].clamp(0.0, 1.0));
                *lp += (y - *lp) * onepole_coef(1.0 / (TAU * fc), c.inv_sr);
                *lp
            }
            Block::Chorus { ring, phase } => {
                let x = v[0];
                ring.push(x);
                *phase += v[1].clamp(0.0, 10.0) * c.inv_sr;
                *phase -= phase.floor();
                let depth = v[2].clamp(0.0, 1.0);
                let mut wet = 0.0;
                for k in 0..3 {
                    let lfo = (TAU * (*phase + k as f32 / 3.0)).sin();
                    let d = (0.012 + depth * 0.008 * lfo) * c.sr;
                    wet += ring.read(d);
                }
                let mix = v[3].clamp(0.0, 1.0);
                x * (1.0 - mix * 0.5) + wet / 3.0 * mix
            }
            Block::Modal(m) => {
                let key = [v[1], v[2], v[3], v[4]];
                let changed = key.iter().zip(m.last.iter()).any(|(a, b)| (a - b).abs() > 1e-3 * a.abs().max(1e-3));
                if changed {
                    m.retune(v[1].clamp(10.0, 12_000.0), v[2], v[3], v[4], c.sr);
                    m.last = key;
                }
                let x = v[0].clamp(-4.0, 4.0);
                let mut y = 0.0;
                for k in 0..MODES {
                    let o = m.b[k] * x + m.a1[k] * m.y1[k] - m.a2[k] * m.y2[k];
                    m.y2[k] = m.y1[k];
                    m.y1[k] = o;
                    y += o;
                }
                (y * 0.5).clamp(-8.0, 8.0)
            }
            Block::Grain(g) => {
                let n = g.ring.len();
                if v[6] <= 0.5 {
                    g.ring[g.w] = v[0].clamp(-4.0, 4.0);
                    g.w = (g.w + 1) % n;
                }
                let size = v[2].clamp(0.01, 0.5) * c.sr;
                let spread = v[5].clamp(0.0, 1.0);
                g.acc += v[3].clamp(0.0, 100.0) * c.inv_sr;
                while g.acc >= 1.0 {
                    g.acc -= 1.0;
                    if let Some(slot) = g.grains.iter_mut().find(|s| !s.on) {
                        let r1 = white(&mut g.rng);
                        let r2 = white(&mut g.rng);
                        let rate = 2f32.powf((v[4].clamp(-24.0, 24.0) + r2 * spread * 0.5) / 12.0);
                        // start far enough back that a faster-than-life
                        // grain never overtakes the write head
                        let need = size * rate.max(1.0) + 4.0;
                        let back = (v[1].clamp(0.0, 1.0) * (n as f32 - need - 8.0) + need + r1.abs() * spread * 0.2 * n as f32).min(n as f32 - 4.0);
                        *slot = Grain { on: true, pos: (g.w as f32 - back).rem_euclid(n as f32), rate, age: 0.0, len: size };
                    }
                }
                let mut y = 0.0;
                let mut active = 0.0f32;
                for gr in g.grains.iter_mut().filter(|s| s.on) {
                    let i0 = gr.pos as usize % n;
                    let f = gr.pos - gr.pos.floor();
                    let s = g.ring[i0] * (1.0 - f) + g.ring[(i0 + 1) % n] * f;
                    let ph = gr.age / gr.len;
                    let w = (PI * ph).sin();
                    y += s * w * w;
                    active += 1.0;
                    gr.pos = (gr.pos + gr.rate) % n as f32;
                    gr.age += 1.0;
                    if gr.age >= gr.len {
                        gr.on = false;
                    }
                }
                y / active.max(1.0).sqrt()
            }
            Block::Sampler { data, root, pos, playing, prev_trig } => {
                let Some(d) = data else { return 0.0 };
                let len = d.frames.len();
                let (a, b) = (v[2].clamp(0.0, 1.0), v[3].clamp(0.0, 1.0));
                let (start, end) = (a.min(b) * (len - 1) as f32, a.max(b) * (len - 1) as f32);
                let rev = v[4] > 0.5;
                if rising(prev_trig, v[1]) {
                    *pos = if rev { end as f64 } else { start as f64 };
                    *playing = true;
                }
                if !*playing || end - start < 2.0 {
                    return 0.0;
                }
                let root_hz = 440.0 * 2f32.powf((*root - 69.0) / 12.0);
                let rate = (v[0].clamp(0.0, 20_000.0) / root_hz) * d.rate * c.inv_sr;
                let i0 = (*pos as usize).min(len - 1);
                let f = (*pos - pos.floor()) as f32;
                let s = d.frames[i0] * (1.0 - f) + d.frames[(i0 + 1).min(len - 1)] * f;
                *pos += if rev { -(rate as f64) } else { rate as f64 };
                let (s0, e0) = (start as f64, end as f64);
                if *pos > e0 || *pos < s0 {
                    if v[5] > 0.5 {
                        *pos = if rev { e0 - (s0 - *pos).max(0.0) % (e0 - s0) } else { s0 + (*pos - e0).max(0.0) % (e0 - s0) };
                    } else {
                        *playing = false;
                    }
                }
                s
            }
            Block::Plaits { voice, engine, prev_gate } => {
                let hz = v[0].clamp(8.0, 12_000.0);
                let note = 69.0 + 12.0 * (hz / 440.0).log2();
                let gate = v[5] > 0.5;
                let _ = rising(prev_gate, v[5]);
                let params = crate::plaits_ffi::PlaitsParams {
                    engine: *engine,
                    note,
                    harmonics: v[1].clamp(0.0, 1.0),
                    timbre: v[2].clamp(0.0, 1.0),
                    morph: v[3].clamp(0.0, 1.0),
                    decay: v[4].clamp(0.0, 1.0),
                    lpg_colour: 0.5,
                    trigger: gate,
                };
                let mut one = [0.0f32];
                voice.render(&mut one, c.sr, &params);
                one[0]
            }
            Block::Pitch { lp, prev, last_cross, t, period, hz, env } => {
                // One-pole low-pass near the top of the range, then period
                // from rising zero crossings (with an envelope gate so
                // silence and hiss don't produce pitches).
                let max = v[2].clamp(50.0, 4000.0);
                let min = v[1].clamp(20.0, max * 0.9);
                let x = v[0].clamp(-4.0, 4.0);
                *lp += (x - *lp) * onepole_coef(1.0 / (TAU * max * 1.5), c.inv_sr);
                *env = (*env * 0.9995).max(lp.abs());
                *t += 1.0;
                if *prev <= 0.0 && *lp > 0.0 && *env > 0.01 {
                    let p = *t - *last_cross;
                    *last_cross = *t;
                    let f = c.sr / p.max(1.0);
                    if f >= min && f <= max {
                        // reject octave-jump outliers: follow only near the
                        // running period, or after a clear restart
                        if *period <= 0.0 || (p / *period - 1.0).abs() < 0.25 {
                            *period = if *period <= 0.0 { p } else { *period * 0.7 + p * 0.3 };
                            *hz = c.sr / *period;
                        } else {
                            *period = p;
                        }
                    }
                }
                *prev = *lp;
                if *t > 1e7 {
                    *last_cross -= *t;
                    *t = 0.0;
                }
                *hz
            }
            Block::Onset { fast, slow, hold, refractory } => {
                let x = v[0].clamp(-4.0, 4.0).abs();
                let fa = if x > *fast { onepole_coef(0.001, c.inv_sr) } else { onepole_coef(0.01, c.inv_sr) };
                *fast += (x - *fast) * fa;
                let sa = if x > *slow { onepole_coef(0.05, c.inv_sr) } else { onepole_coef(0.3, c.inv_sr) };
                *slow += (x - *slow) * sa;
                *hold = (*hold - c.inv_sr).max(0.0);
                *refractory = (*refractory - c.inv_sr).max(0.0);
                let ratio = 1.5 + (1.0 - v[1].clamp(0.0, 1.0)) * 3.0;
                if *refractory <= 0.0 && *fast > *slow * ratio + 0.01 {
                    *hold = 0.005;
                    *refractory = 0.06;
                }
                if *hold > 0.0 { 1.0 } else { 0.0 }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &'static [(&'static str, &'static str)]) -> Opts<'static> {
        Opts { get: Box::leak(Box::new(move |k: &str| pairs.iter().find(|p| p.0 == k).map(|p| p.1.to_string()))), values: vec![] }
    }

    fn run(kind: &str, o: &Opts, inputs: [f32; MAX_IN], n: usize) -> Vec<f32> {
        let mut b = Block::new(kind, o, 48_000.0).unwrap();
        let c = Ctx { sr: 48_000.0, inv_sr: 1.0 / 48_000.0, beat: 0.0 };
        let mut rng = 7;
        (0..n).map(|_| b.tick(&inputs, &c, &mut rng)).collect()
    }

    #[test]
    fn every_spec_builds_and_stays_finite_under_hostile_inputs() {
        let empty = opts(&[]);
        for s in SPECS {
            for hostile in [[0.0; MAX_IN], [1e9; MAX_IN], [-1e9; MAX_IN], [f32::MAX; MAX_IN], [1.0, 20_000.0, 1.0, 10.0, 1.0, 1.0, 0.0, 0.0]] {
                let out = run(s.name, &empty, hostile, 4000);
                assert!(out.iter().all(|x| x.is_finite()), "{} went non-finite with {:?}", s.name, hostile[0]);
            }
        }
    }

    #[test]
    fn oscillators_hit_their_range() {
        let saw = opts(&[("wave", "saw")]);
        let out = run("osc", &saw, [440.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0], 4800);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.8 && peak < 1.3, "saw peak {peak}");
    }

    #[test]
    fn euclid_5_of_16_hits_five_times_per_cycle() {
        let o = opts(&[]);
        let mut b = Block::new("euclid", &o, 48_000.0).unwrap();
        let c = Ctx { sr: 48_000.0, inv_sr: 1.0 / 48_000.0, beat: 0.0 };
        let mut rng = 1;
        let mut hits = 0;
        for _ in 0..16 {
            let on = b.tick(&[1.0, 16.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0], &c, &mut rng);
            if on > 0.5 {
                hits += 1;
            }
            b.tick(&[0.0, 16.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0], &c, &mut rng);
        }
        assert_eq!(hits, 5);
    }

    #[test]
    fn every_block_has_a_constructor_and_unique_input_names() {
        let o = opts(&[]);
        for s in SPECS {
            assert!(Block::new(s.name, &o, 48_000.0).is_some(), "{} has a spec but no constructor", s.name);
            assert!(s.inputs.len() <= MAX_IN);
            let mut names: Vec<_> = s.inputs.iter().map(|i| i.name).collect();
            names.dedup();
            assert_eq!(names.len(), s.inputs.len());
        }
    }

    fn ctx() -> Ctx {
        Ctx { sr: 48_000.0, inv_sr: 1.0 / 48_000.0, beat: 0.0 }
    }

    fn ins(v: &[f32]) -> [f32; MAX_IN] {
        let mut a = [0.0; MAX_IN];
        a[..v.len()].copy_from_slice(v);
        a
    }

    /// Energy at one frequency (single DFT bin), normalised.
    fn bin(x: &[f32], hz: f32) -> f32 {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, s) in x.iter().enumerate() {
            let ph = TAU * hz * n as f32 / 48_000.0;
            re += s * ph.cos();
            im += s * ph.sin();
        }
        (re * re + im * im).sqrt() / x.len() as f32
    }

    #[test]
    fn wavetable_position_morphs_from_sine_to_bright_and_stays_band_limited() {
        let o = opts(&[("table", "analog")]);
        let sine = run("wavetable", &o, ins(&[220.0, 0.0]), 4800);
        let saw = run("wavetable", &o, ins(&[220.0, 2.0 / 7.0]), 4800);
        assert!(bin(&sine, 660.0) < bin(&sine, 220.0) * 0.02, "frame 0 is a sine");
        assert!(bin(&saw, 660.0) > bin(&saw, 220.0) * 0.2, "frame 2 is saw-bright");
        // at 5 kHz only a few harmonics may exist: nothing near 22 kHz aliasing back
        let high = run("wavetable", &o, ins(&[5000.0, 2.0 / 7.0]), 4800);
        assert!(bin(&high, 5000.0) > 0.2);
        for t in WT_TABLES {
            let o = Opts { get: Box::leak(Box::new(move |k: &str| (k == "table").then(|| t.to_string()))), values: vec![] };
            let x = run("wavetable", &o, ins(&[110.0, 0.6, 0.0, 0.5]), 2400);
            let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.3 && peak <= 1.2, "{t}: {peak}");
        }
    }

    #[test]
    fn modal_rings_at_its_frequency_and_damping_shortens_it() {
        let o = opts(&[]);
        let ring = |damp: f32| {
            let mut b = Block::new("modal", &o, 48_000.0).unwrap();
            let mut rng = 3;
            (0..24_000).map(|n| b.tick(&ins(&[if n == 0 { 1.0 } else { 0.0 }, 330.0, 0.0, damp, 0.5]), &ctx(), &mut rng)).collect::<Vec<f32>>()
        };
        let long = ring(0.1);
        assert!(bin(&long, 330.0) > bin(&long, 400.0) * 5.0, "rings at 330 Hz");
        let tail = |x: &[f32]| x[18_000..].iter().map(|v| v * v).sum::<f32>();
        assert!(tail(&ring(0.9)) < tail(&long) * 0.01, "damping kills the ring");
    }

    #[test]
    fn grains_carry_the_input_and_freeze_holds_it() {
        let o = opts(&[]);
        let mut b = Block::new("grain", &o, 48_000.0).unwrap();
        let mut rng = 9;
        let tone = |n: usize| (TAU * 440.0 * n as f32 / 48_000.0).sin() * 0.5;
        let mut out = Vec::new();
        for n in 0..48_000 {
            out.push(b.tick(&ins(&[tone(n), 0.1, 0.08, 30.0, 0.0, 0.0, 0.0]), &ctx(), &mut rng));
        }
        assert!(bin(&out[24_000..], 440.0) > 0.03, "grains of the input");
        // freeze, then the input goes silent: the texture remains
        let mut frozen = Vec::new();
        for _ in 0..24_000 {
            frozen.push(b.tick(&ins(&[0.0, 0.1, 0.08, 30.0, 0.0, 0.0, 1.0]), &ctx(), &mut rng));
        }
        // (a frozen slice replays out of phase with the clock, so measure
        // level rather than one DFT bin)
        let rms = (frozen.iter().map(|v| v * v).sum::<f32>() / frozen.len() as f32).sqrt();
        assert!(rms > 0.05, "frozen texture keeps sounding ({rms})");
        let pitched: Vec<f32> = {
            let mut b = Block::new("grain", &o, 48_000.0).unwrap();
            (0..48_000).map(|n| b.tick(&ins(&[tone(n), 0.1, 0.08, 30.0, 12.0, 0.0, 0.0]), &ctx(), &mut rng)).collect()
        };
        assert!(bin(&pitched[24_000..], 880.0) > bin(&pitched[24_000..], 440.0), "pitch +12 moves it an octave up");
    }

    #[test]
    fn the_pitch_tracker_follows_a_hum_with_harmonics() {
        let o = opts(&[]);
        for hz in [110.0f32, 196.0, 330.0] {
            let mut b = Block::new("pitch", &o, 48_000.0).unwrap();
            let mut rng = 1;
            let mut last = 0.0;
            for n in 0..24_000 {
                let ph = TAU * hz * n as f32 / 48_000.0;
                let x = 0.5 * ph.sin() + 0.25 * (2.0 * ph).sin() + 0.12 * (3.0 * ph).sin();
                last = b.tick(&ins(&[x, 60.0, 1000.0]), &ctx(), &mut rng);
            }
            assert!((last / hz - 1.0).abs() < 0.02, "{hz} Hz tracked as {last}");
        }
    }

    #[test]
    fn onsets_fire_once_per_hit() {
        let o = opts(&[]);
        let mut b = Block::new("onset", &o, 48_000.0).unwrap();
        let mut rng = 1;
        let mut hits = 0;
        let mut prev = 0.0;
        for n in 0..96_000 {
            // a decaying click every 0.25 s
            let k = n % 12_000;
            let x = if k < 2000 { (-(k as f32) / 300.0).exp() * if k % 2 == 0 { 1.0 } else { -1.0 } * 0.8 } else { 0.0 };
            let y = b.tick(&ins(&[x, 0.5]), &ctx(), &mut rng);
            if y > 0.5 && prev <= 0.5 {
                hits += 1;
            }
            prev = y;
        }
        assert_eq!(hits, 8);
    }

    #[test]
    fn the_sampler_plays_a_wav_at_pitch_and_in_reverse() {
        let path = std::env::temp_dir().join(format!("pmx-sampler-{}.wav", std::process::id()));
        {
            let spec = hound::WavSpec { channels: 1, sample_rate: 48_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
            let mut w = hound::WavWriter::create(&path, spec).unwrap();
            for n in 0..48_000 {
                let s = (TAU * 261.63 * n as f32 / 48_000.0).sin() * 0.5 * (1.0 - n as f32 / 48_000.0);
                w.write_sample((s * 32767.0) as i16).unwrap();
            }
            w.finalize().unwrap();
        }
        let file: &'static str = Box::leak(path.to_string_lossy().to_string().into_boxed_str());
        let o = Opts { get: Box::leak(Box::new(move |k: &str| match k { "file" => Some(file.to_string()), "root" => Some("60".into()), _ => None })), values: vec![] };
        let play = |freq: f32, rev: f32| {
            let mut b = Block::new("sampler", &o, 48_000.0).unwrap();
            let mut rng = 1;
            (0..12_000).map(|n| b.tick(&ins(&[freq, if n < 10 { 1.0 } else { 0.0 }, 0.0, 1.0, rev, 0.0]), &ctx(), &mut rng)).collect::<Vec<f32>>()
        };
        let root = play(261.63, 0.0);
        assert!(bin(&root, 261.63) > 0.15, "root plays at its own pitch");
        let up = play(523.25, 0.0);
        assert!(bin(&up, 523.25) > bin(&up, 261.63) * 3.0, "an octave up");
        let back = play(261.63, 1.0);
        let early = |x: &[f32]| x[..2000].iter().map(|v| v.abs()).sum::<f32>();
        assert!(early(&back) < early(&root) * 0.2, "reverse starts from the quiet tail");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_plaits_node_sounds_when_struck() {
        let o = opts(&[("engine", "va")]);
        let mut b = Block::new("plaits", &o, 48_000.0).unwrap();
        let mut rng = 1;
        let x: Vec<f32> = (0..9600).map(|_| b.tick(&ins(&[220.0, 0.5, 0.5, 0.5, 0.5, 1.0]), &ctx(), &mut rng)).collect();
        assert!(x.iter().map(|v| v * v).sum::<f32>() > 1.0);
    }

    /// Calibration for `cost()`: run with
    /// `cargo test --bin portamax-sim block_costs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn block_costs() {
        let o = opts(&[]);
        let mut base = 0.0;
        for s in SPECS {
            let mut b = Block::new(s.name, &o, 48_000.0).unwrap();
            let mut rng = 1;
            let input = ins(&[220.0, 0.5, 0.3, 0.5, 0.5, 1.0, 0.0, 0.0]);
            let t = std::time::Instant::now();
            let mut acc = 0.0;
            for _ in 0..480_000 {
                acc += b.tick(&input, &ctx(), &mut rng);
            }
            let ns = t.elapsed().as_nanos() as f64 / 480_000.0;
            if s.name == "osc" {
                base = ns;
            }
            println!("{:<10} {:>7.1} ns/sample  {:>5.1}x osc  (cost table {:>4.1}) {}", s.name, ns, ns / base.max(1e-9), cost(s.name), if acc.is_finite() { "" } else { "!" });
        }
    }
}
