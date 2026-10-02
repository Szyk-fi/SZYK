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

pub const MAX_IN: usize = 6;

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
];

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
    Ladder { s: [f32; 4] },
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
            "ladder" => Block::Ladder { s: [0.0; 4] },
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
            Block::Ladder { s } => *s = [0.0; 4],
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
            Block::Ladder { s } => {
                let fc = v[1].clamp(20.0, c.sr * 0.42);
                let g = 1.0 - (-TAU * fc * c.inv_sr).exp();
                let res = v[2].clamp(0.0, 1.1) * 4.0;
                let drive = v[3].clamp(0.1, 10.0);
                let x = (v[0] * drive - res * s[3]).tanh();
                s[0] += g * (x - s[0].tanh());
                s[1] += g * (s[0].tanh() - s[1].tanh());
                s[2] += g * (s[1].tanh() - s[2].tanh());
                s[3] += g * (s[2].tanh() - s[3].tanh());
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
            for hostile in [[0.0; MAX_IN], [1e9; MAX_IN], [-1e9; MAX_IN], [f32::MAX; MAX_IN], [1.0, 20_000.0, 1.0, 10.0, 1.0, 1.0]] {
                let out = run(s.name, &empty, hostile, 4000);
                assert!(out.iter().all(|x| x.is_finite()), "{} went non-finite with {:?}", s.name, hostile[0]);
            }
        }
    }

    #[test]
    fn oscillators_hit_their_range() {
        let saw = opts(&[("wave", "saw")]);
        let out = run("osc", &saw, [440.0, 0.0, 0.5, 0.0, 0.0, 0.0], 4800);
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
            let on = b.tick(&[1.0, 16.0, 5.0, 0.0, 0.0, 0.0], &c, &mut rng);
            if on > 0.5 {
                hits += 1;
            }
            b.tick(&[0.0, 16.0, 5.0, 0.0, 0.0, 0.0], &c, &mut rng);
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
}
