//! Compiles a `Patch` into a runnable `Engine`, and runs it.
//!
//! `compile` does everything that can fail or allocate (validation,
//! expression compilation, buffer allocation, voice cloning) on the calling
//! thread -- the AI worker or the UI -- and returns either a ready engine or
//! a list of human-readable errors (which are fed straight back to the AI
//! for self-repair). `Engine::process` then runs on the audio thread with
//! no allocation, no locks and no panicking paths.

use super::blocks::{self, Block, Ctx, Opts, MAX_IN};
use super::expr::{Expr, Resolved, Sym};
use super::patch::*;
use std::collections::{BTreeMap, HashSet};

// Global registers.
const G_TIME: u16 = 0;
const G_SR: u16 = 1;
const G_IN: u16 = 2;
const G_VOICES: u16 = 3;
const G_PADNOTE: u16 = 4;
const G_PADGATE: u16 = 5;
const G_PADTRIG: u16 = 6;
const G_BPM: u16 = 7;
const G_BEAT: u16 = 8;
const G_PLAYING: u16 = 9;
const G_MACRO0: u16 = 10;
const G_PARAM0: u16 = G_MACRO0 + MAX_MACROS as u16;

// Voice registers.
const V_PITCH: u16 = 0;
const V_NOTE: u16 = 1;
const V_GATE: u16 = 2;
const V_VEL: u16 = 3;
const V_TRIG: u16 = 4;
const V_VTIME: u16 = 5;
const V_INDEX: u16 = 6;
const V_AMPENV: u16 = 7;
const V_NODE0: u16 = 8;

/// Built-in names and what they mean, for validation and the AI prompt.
pub const GLOBAL_NAMES: &[(&str, &str)] = &[
    ("time", "seconds since the patch loaded"),
    ("sr", "sample rate in Hz"),
    ("in", "audio input (the Source app, or silence) -1..1"),
    ("voices", "sum of all voice_out values (instruments), headroom-scaled"),
    ("padnote", "MIDI note of the last pad pressed"),
    ("padgate", "1 while any pad is held, else 0"),
    ("padtrig", "1 for one sample when a pad is pressed"),
    ("bpm", "Tempo in BPM"),
    ("beat", "song position in beats (advances while playing)"),
    ("playing", "1 while the transport runs (F3 Play/Stop, generator patches)"),
    ("m1..m8", "macro knob values 0..1"),
];
pub const VOICE_NAMES: &[(&str, &str)] = &[
    ("pitch", "note frequency in Hz (with glide and transpose)"),
    ("note", "MIDI note number"),
    ("gate", "1 while the pad is held"),
    ("vel", "velocity 0..1"),
    ("trig", "1 for one sample at note on"),
    ("vtime", "seconds since note on"),
    ("vindex", "which voice this is, 0..voices-1"),
    ("ampenv", "the built-in amp envelope 0..1"),
];

const RESERVED_WORDS: &[&str] = &["pi", "tau", "e", "noise", "true", "false"];

pub const SCALES: &[(&str, &[i32])] = &[
    ("Chromatic", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
    ("Major", &[0, 2, 4, 5, 7, 9, 11]),
    ("Minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("Dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("Phrygian", &[0, 1, 3, 5, 7, 8, 10]),
    ("Lydian", &[0, 2, 4, 6, 7, 9, 11]),
    ("Mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    ("Harm Minor", &[0, 2, 3, 5, 7, 8, 11]),
    ("Pent Major", &[0, 2, 4, 7, 9]),
    ("Pent Minor", &[0, 3, 5, 7, 10]),
    ("Blues", &[0, 3, 5, 6, 7, 10]),
    ("Whole Tone", &[0, 2, 4, 6, 8, 10]),
    ("Hirajoshi", &[0, 2, 3, 7, 8]),
];

/// Pad index (row-major, row 0 = top) to scale degree, lowest bottom-left.
pub fn pad_degree(pad: usize) -> i32 {
    let (row, col) = (pad / 4, pad % 4);
    ((3 - row) * 4 + col) as i32
}

pub fn pad_note(pad: usize, root: i32, scale: usize, transpose: i32) -> i32 {
    let steps = SCALES.get(scale).map(|s| s.1).unwrap_or(SCALES[0].1);
    let d = pad_degree(pad);
    let len = steps.len() as i32;
    root + transpose + 12 * (d / len) + steps[(d % len) as usize]
}

/// Everything the engine reads from the UI each block.
#[derive(Clone, Copy)]
pub struct Controls {
    pub norms: [f32; MAX_PARAMS],
    pub macros: [f32; MAX_MACROS],
    pub held: [bool; 16],
    pub gain: f32,
    pub voices_limit: usize,
    pub glide: f32,
    pub transpose: i32,
    pub root: i32,
    pub scale: usize,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub velocity: f32,
    pub bpm: f32,
    pub playing: bool,
    pub dry_wet: f32,
    pub input_gain: f32,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            norms: [0.0; MAX_PARAMS],
            macros: [0.0; MAX_MACROS],
            held: [false; 16],
            gain: 1.0,
            voices_limit: MAX_VOICES,
            glide: 0.0,
            transpose: 0,
            root: 48,
            scale: 0,
            attack: 0.005,
            decay: 0.3,
            sustain: 0.8,
            release: 0.4,
            velocity: 0.8,
            bpm: 120.0,
            playing: true,
            dry_wet: 1.0,
            input_gain: 1.0,
        }
    }
}

enum In {
    Const(f32),
    Block(Expr, f32),
    Sample(Expr),
}

struct CNode {
    block: Block,
    inputs: Vec<In>,
    out: u16,
    tele: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum EnvStage {
    Idle,
    Attack,
    Decay,
    Release,
}

struct Voice {
    regs: Vec<f32>,
    nodes: Vec<CNode>,
    order: Vec<usize>,
    active: bool,
    gate: bool,
    pad: usize,
    note: i32,
    pitch: f32,
    target: f32,
    trig: bool,
    vtime: f32,
    env: f32,
    stage: EnvStage,
    quiet: f32,
    age: u64,
}

/// Node summary for the graph view: (id, type, scope is voice, deps).
#[derive(Clone, Debug)]
#[allow(dead_code)] // `kind` is for tools/UI that want to show block types
pub struct GraphNode {
    pub id: String,
    pub kind: String,
    pub voice: bool,
    pub deps: Vec<usize>,
    /// Reads the summed voices / the audio input.
    pub reads_voices: bool,
    pub reads_in: bool,
    /// Feeds voice_out (voice nodes) or out (global nodes) directly.
    pub to_out: bool,
}

pub struct Engine {
    pub kind: Kind,
    pub graph: Vec<GraphNode>,
    pub out_reads_voices: bool,
    pub out_reads_in: bool,
    amp_env: bool,
    poly: usize,
    g: Vec<f32>,
    gnodes: Vec<CNode>,
    gorder: Vec<usize>,
    voices: Vec<Voice>,
    voice_out: Expr,
    out_l: Expr,
    out_r: Option<Expr>,
    maps: Vec<ParamMap>,
    /// Per-param smoothing time, seconds.
    smooth_s: Vec<f32>,
    macro_targets: Vec<Vec<(usize, f32)>>,
    smoothed: Vec<f32>,
    /// Estimated cost, in units of one oscillator voice-sample: per voice,
    /// and for the global graph (see `blocks::cost`).
    pub cost_voice: f32,
    pub cost_global: f32,
    first_block: bool,
    held_prev: [bool; 16],
    padtrig: bool,
    last_pitch: f32,
    age: u64,
    beat: f64,
    time: f64,
    rng: u32,
    dc: [(f32, f32); 2],
    /// Telemetry, written each block: mean |output| per graph node.
    pub activity: Vec<f32>,
    pub active_voices: usize,
    /// Set when some node blew up (non-finite) and was reset this block.
    pub unstable: bool,
    tele_acc: Vec<f32>,
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_lowercase() || ch == '_')
        && s.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
        && s.len() <= 24
}

fn builtin(name: &str) -> bool {
    GLOBAL_NAMES.iter().any(|(n, _)| *n == name)
        || VOICE_NAMES.iter().any(|(n, _)| *n == name)
        || RESERVED_WORDS.contains(&name)
        || (name.len() == 2 && name.starts_with('m') && name[1..].parse::<u8>().is_ok_and(|k| (1..=8).contains(&k)))
}

/// Validate and compile. On failure returns every problem found, phrased
/// so a language model (or a person) can fix the patch.
pub fn compile(patch: &Patch, sr: f32) -> Result<Engine, Vec<String>> {
    let mut errs: Vec<String> = Vec::new();
    let sr = if sr.is_finite() && sr > 1000.0 { sr } else { 48_000.0 };

    // ---- structure
    if patch.name.trim().is_empty() {
        errs.push("\"name\" must not be empty".into());
    }
    if patch.params.len() > MAX_PARAMS {
        errs.push(format!("too many params: {} (max {MAX_PARAMS})", patch.params.len()));
    }
    if patch.macros.len() > MAX_MACROS {
        errs.push(format!("too many macros: {} (max {MAX_MACROS})", patch.macros.len()));
    }
    if !(1..=MAX_VOICES as u32).contains(&patch.voices) {
        errs.push(format!("\"voices\" must be 1..{MAX_VOICES}"));
    }
    match patch.kind {
        Kind::Instrument => {
            if patch.voice_out.is_none() {
                errs.push("an instrument needs \"voice_out\": the per-voice output expression".into());
            }
        }
        _ => {
            if !patch.voice.is_empty() {
                errs.push(format!(
                    "a {} has no voices: move the \"voice\" nodes into \"global\" (use padgate/padtrig/padnote for pads)",
                    patch.kind.label()
                ));
            }
        }
    }
    for (scope, list) in [("voice", &patch.voice), ("global", &patch.global)] {
        if list.len() > MAX_NODES {
            errs.push(format!("too many {scope} nodes: {} (max {MAX_NODES})", list.len()));
        }
    }

    // ---- names
    let mut seen: HashSet<&str> = HashSet::new();
    for n in patch.voice.iter().chain(patch.global.iter()) {
        if !is_ident(&n.id) {
            errs.push(format!("node id \"{}\" must be lowercase letters, digits, _ (max 24)", n.id));
        } else if builtin(&n.id) {
            errs.push(format!("node id \"{}\" is a built-in name; rename it", n.id));
        } else if !seen.insert(&n.id) {
            errs.push(format!("duplicate id \"{}\"", n.id));
        }
    }
    for p in &patch.params {
        if !is_ident(&p.id) {
            errs.push(format!("param id \"{}\" must be lowercase letters, digits, _ (max 24)", p.id));
        } else if builtin(&p.id) {
            errs.push(format!("param id \"{}\" is a built-in name; rename it", p.id));
        } else if !seen.insert(&p.id) {
            errs.push(format!("duplicate id \"{}\" (params and nodes share one namespace)", p.id));
        }
        if !(p.min.is_finite() && p.max.is_finite() && p.default.is_finite()) {
            errs.push(format!("param \"{}\" has a non-finite min/max/default", p.id));
        }
        if p.curve == "exp" && (p.min <= 0.0 || p.max <= 0.0) {
            errs.push(format!("param \"{}\": curve \"exp\" needs min and max > 0", p.id));
        }
        if p.curve == "choice" && p.options.len() < 2 {
            errs.push(format!("param \"{}\": curve \"choice\" needs 2+ \"options\"", p.id));
        }
        if !["lin", "exp", "int", "toggle", "bool", "choice"].contains(&p.curve.as_str()) {
            errs.push(format!("param \"{}\": unknown curve \"{}\" (lin, exp, int, toggle, choice)", p.id, p.curve));
        }
    }
    let param_idx: BTreeMap<&str, usize> = patch.params.iter().enumerate().map(|(i, p)| (p.id.as_str(), i)).collect();
    let mut macro_targets = Vec::new();
    for m in patch.macros.iter().take(MAX_MACROS) {
        let mut t = Vec::new();
        for tg in &m.targets {
            match param_idx.get(tg.param.as_str()) {
                Some(&i) => t.push((i, tg.amount.clamp(-1.0, 1.0))),
                None => errs.push(format!("macro \"{}\" targets unknown param \"{}\"", m.name, tg.param)),
            }
        }
        macro_targets.push(t);
    }

    // ---- node blocks and options
    for n in patch.voice.iter().chain(patch.global.iter()) {
        let Some(spec) = blocks::spec(&n.kind) else {
            let known: Vec<&str> = blocks::SPECS.iter().map(|s| s.name).collect();
            errs.push(format!("node \"{}\": unknown type \"{}\" (known: {})", n.id, n.kind, known.join(", ")));
            continue;
        };
        for k in n.inputs.keys() {
            if !spec.inputs.iter().any(|i| i.name == k) {
                let names: Vec<&str> = spec.inputs.iter().map(|i| i.name).collect();
                errs.push(format!("node \"{}\" ({}): no input \"{k}\" (inputs: {})", n.id, n.kind, names.join(", ")));
            }
        }
        for (k, v) in &n.opts {
            let Some(o) = spec.opts.iter().find(|o| o.name == k) else {
                let names: Vec<&str> = spec.opts.iter().map(|o| o.name).collect();
                errs.push(format!(
                    "node \"{}\" ({}): unknown option \"{k}\"{} -- signal connections go inside \"in\"",
                    n.id,
                    n.kind,
                    if names.is_empty() { String::new() } else { format!(" (options: {})", names.join(", ")) }
                ));
                continue;
            };
            if k == "values" {
                match v.as_array() {
                    Some(a) if !a.is_empty() && a.len() <= 32 && a.iter().all(|x| x.as_f64().is_some_and(f64::is_finite)) => {}
                    _ => errs.push(format!("node \"{}\": \"values\" must be a list of 1..32 numbers", n.id)),
                }
            } else if o.choices.is_empty() {
                if v.as_f64().is_none() && v.as_str().and_then(|s| s.parse::<f64>().ok()).is_none() {
                    errs.push(format!("node \"{}\": option \"{k}\" must be a number", n.id));
                }
            } else {
                let s = opt_string(v);
                if !o.choices.contains(&s.as_str()) {
                    errs.push(format!("node \"{}\": option {k}=\"{s}\" must be one of {}", n.id, o.choices.join(", ")));
                }
            }
        }
    }
    for n in patch.voice.iter().chain(patch.global.iter()).filter(|n| n.kind == "sampler") {
        match n.opts.get("file").map(opt_string) {
            None => errs.push(format!("node \"{}\" (sampler): needs a \"file\" (a WAV in samples/)", n.id)),
            Some(f) => {
                if let Err(e) = blocks::load_sample(&f) {
                    errs.push(format!("node \"{}\": {e}", n.id));
                }
            }
        }
    }
    let delay_mem: f32 = patch
        .voice
        .iter()
        .map(|n| delay_seconds(n) * patch.voices as f32)
        .chain(patch.global.iter().map(delay_seconds))
        .sum();
    if delay_mem > 40.0 {
        errs.push(format!("too much delay memory ({delay_mem:.0} s total across voices; max 40): lower \"max\" or use global delays"));
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    // ---- register allocation
    let voice_ids: BTreeMap<&str, u16> =
        patch.voice.iter().enumerate().map(|(i, n)| (n.id.as_str(), V_NODE0 + i as u16)).collect();
    let g_node0 = G_PARAM0 + patch.params.len() as u16;
    let global_ids: BTreeMap<&str, u16> =
        patch.global.iter().enumerate().map(|(i, n)| (n.id.as_str(), g_node0 + i as u16)).collect();

    let global_resolve = |name: &str| -> Option<Resolved> {
        let d = |sym, dynamic| Some(Resolved { sym, dynamic });
        match name {
            "time" => d(Sym::Global(G_TIME), true),
            "sr" => d(Sym::Global(G_SR), false),
            "in" => d(Sym::Global(G_IN), true),
            "voices" => d(Sym::Global(G_VOICES), true),
            "padnote" => d(Sym::Global(G_PADNOTE), true),
            "padgate" => d(Sym::Global(G_PADGATE), true),
            "padtrig" => d(Sym::Global(G_PADTRIG), true),
            "bpm" => d(Sym::Global(G_BPM), false),
            "beat" => d(Sym::Global(G_BEAT), true),
            "playing" => d(Sym::Global(G_PLAYING), false),
            _ => {
                if let Some(k) = name.strip_prefix('m').and_then(|k| k.parse::<u16>().ok()) {
                    if (1..=8).contains(&k) {
                        return d(Sym::Global(G_MACRO0 + k - 1), false);
                    }
                }
                if let Some(&i) = param_idx.get(name) {
                    return d(Sym::Global(G_PARAM0 + i as u16), false);
                }
                global_ids.get(name).map(|&r| Resolved { sym: Sym::Global(r), dynamic: true })
            }
        }
    };
    let voice_resolve = |name: &str| -> Option<Resolved> {
        let d = |sym, dynamic| Some(Resolved { sym, dynamic });
        match name {
            "pitch" => d(Sym::Voice(V_PITCH), true),
            "note" => d(Sym::Voice(V_NOTE), true),
            "gate" => d(Sym::Voice(V_GATE), true),
            "vel" => d(Sym::Voice(V_VEL), true),
            "trig" => d(Sym::Voice(V_TRIG), true),
            "vtime" => d(Sym::Voice(V_VTIME), true),
            "vindex" => d(Sym::Voice(V_INDEX), false),
            "ampenv" => d(Sym::Voice(V_AMPENV), true),
            _ => voice_ids.get(name).map(|&r| Resolved { sym: Sym::Voice(r), dynamic: true }).or_else(|| global_resolve(name)),
        }
    };

    let compile_expr = |src: &str, voice: bool, what: &str, errs: &mut Vec<String>| -> Option<Expr> {
        let r = if voice { Expr::compile(src, &voice_resolve) } else { Expr::compile(src, &global_resolve) };
        match r {
            Ok(e) => Some(e),
            Err(e) => {
                let mut msg = format!("{what}: {e} in \"{src}\"");
                if !voice && VOICE_NAMES.iter().any(|(n, _)| msg.contains(&format!("unknown name '{n}'"))) {
                    msg.push_str(" (voice names like pitch/gate/trig only exist in \"voice\" nodes and voice_out; global nodes use padnote/padgate/padtrig)");
                } else if voice_ids.keys().any(|v| msg.contains(&format!("unknown name '{v}'"))) {
                    msg.push_str(" (global nodes cannot read voice nodes; sum voices with voice_out and read \"voices\")");
                }
                errs.push(msg);
                None
            }
        }
    };

    // Build a template for each node: (block ctor args, compiled inputs, deps).
    struct Tmpl {
        inputs: Vec<(Expr, Option<f32>)>,
        deps: Vec<usize>,
        names: Vec<String>,
    }
    let build_list = |list: &[NodeSpec], voice: bool, ids: &BTreeMap<&str, u16>, errs: &mut Vec<String>| -> Vec<Tmpl> {
        list.iter()
            .map(|n| {
                let spec = blocks::spec(&n.kind).expect("validated above");
                let mut inputs = Vec::new();
                let mut deps = Vec::new();
                let mut all_names: Vec<String> = Vec::new();
                for ispec in spec.inputs {
                    let src = n
                        .inputs
                        .get(ispec.name)
                        .map(ExprSrc::text)
                        .unwrap_or_else(|| (if voice { ispec.voice_default } else { ispec.global_default }).to_string());
                    let what = format!("node \"{}\" input \"{}\"", n.id, ispec.name);
                    if let Some(e) = compile_expr(&src, voice, &what, errs) {
                        all_names.extend(e.names.iter().cloned());
                        for name in &e.names {
                            if let Some(pos) = list.iter().position(|m| &m.id == name) {
                                if !deps.contains(&pos) && list[pos].id != n.id {
                                    deps.push(pos);
                                }
                            }
                        }
                        let c = e.as_const();
                        inputs.push((e, c));
                    } else {
                        inputs.push((Expr::constant(0.0), Some(0.0)));
                    }
                }
                let _ = ids;
                Tmpl { inputs, deps, names: all_names }
            })
            .collect()
    };
    let vt = build_list(&patch.voice, true, &voice_ids, &mut errs);
    let gt = build_list(&patch.global, false, &global_ids, &mut errs);

    let voice_out = match patch.kind {
        Kind::Instrument => compile_expr(&patch.voice_out.as_ref().map(ExprSrc::text).unwrap_or_default(), true, "voice_out", &mut errs),
        _ => Some(Expr::constant(0.0)),
    };
    let (out_l, out_r) = match &patch.out {
        OutSpec::Mono(e) => (compile_expr(&e.text(), false, "out", &mut errs), None),
        OutSpec::Stereo { left, right } => (
            compile_expr(&left.text(), false, "out.left", &mut errs),
            compile_expr(&right.text(), false, "out.right", &mut errs).map(Some),
        ),
    };
    if !errs.is_empty() {
        return Err(errs);
    }

    let order_of = |t: &[Tmpl]| topo(t.iter().map(|x| x.deps.clone()).collect());
    let vorder = order_of(&vt);
    let gorder = order_of(&gt);

    let make_nodes = |list: &[NodeSpec], t: &[Tmpl], out0: u16, tele0: usize| -> Vec<CNode> {
        list.iter()
            .zip(t)
            .enumerate()
            .map(|(k, (n, tm))| {
                let get = |key: &str| n.opts.get(key).map(opt_string);
                let values: Vec<f32> = n
                    .opts
                    .get("values")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect())
                    .unwrap_or_default();
                let block = Block::new(&n.kind, &Opts { get: &get, values }, sr).expect("validated above");
                let inputs = tm
                    .inputs
                    .iter()
                    .map(|(e, c)| match c {
                        Some(v) => In::Const(*v),
                        None if e.dynamic => In::Sample(e.clone()),
                        None => In::Block(e.clone(), 0.0),
                    })
                    .collect();
                CNode { block, inputs, out: out0 + k as u16, tele: tele0 + k }
            })
            .collect()
    };

    let poly = if patch.kind == Kind::Instrument { patch.voices as usize } else { 0 };
    let voices: Vec<Voice> = (0..poly)
        .map(|vi| {
            let mut regs = vec![0.0; V_NODE0 as usize + patch.voice.len()];
            regs[V_INDEX as usize] = vi as f32;
            Voice {
                regs,
                nodes: make_nodes(&patch.voice, &vt, V_NODE0, 0),
                order: vorder.clone(),
                active: false,
                gate: false,
                pad: usize::MAX,
                note: 60,
                pitch: 261.6,
                target: 261.6,
                trig: false,
                vtime: 0.0,
                env: 0.0,
                stage: EnvStage::Idle,
                quiet: 0.0,
                age: 0,
            }
        })
        .collect();
    let gnodes = make_nodes(&patch.global, &gt, g_node0, patch.voice.len());

    let vout_names: Vec<String> = voice_out.as_ref().map(|e| e.names.clone()).unwrap_or_default();
    let mut out_names: Vec<String> = out_l.as_ref().map(|e| e.names.clone()).unwrap_or_default();
    if let Some(Some(r)) = out_r.as_ref() {
        out_names.extend(r.names.iter().cloned());
    }
    let has = |names: &[String], n: &str| names.iter().any(|x| x == n);
    let mut graph: Vec<GraphNode> = patch
        .voice
        .iter()
        .zip(&vt)
        .map(|(n, t)| GraphNode {
            id: n.id.clone(),
            kind: n.kind.clone(),
            voice: true,
            deps: t.deps.clone(),
            reads_voices: false,
            reads_in: has(&t.names, "in"),
            to_out: has(&vout_names, &n.id),
        })
        .collect();
    let off = graph.len();
    graph.extend(patch.global.iter().zip(&gt).map(|(n, t)| GraphNode {
        id: n.id.clone(),
        kind: n.kind.clone(),
        voice: false,
        deps: t.deps.iter().map(|d| d + off).collect(),
        reads_voices: has(&t.names, "voices"),
        reads_in: has(&t.names, "in"),
        to_out: has(&out_names, &n.id),
    }));
    let out_reads_voices = has(&out_names, "voices");
    let out_reads_in = has(&out_names, "in");
    let n_tele = graph.len();

    let mut g = vec![0.0; g_node0 as usize + patch.global.len()];
    g[G_SR as usize] = sr;
    g[G_PADNOTE as usize] = 60.0;
    let maps: Vec<ParamMap> = patch.params.iter().map(ParamMap::from_spec).collect();

    Ok(Engine {
        kind: patch.kind,
        graph,
        out_reads_voices,
        out_reads_in,
        amp_env: patch.amp_env,
        poly,
        g,
        gnodes,
        gorder,
        voices,
        voice_out: voice_out.unwrap_or_else(|| Expr::constant(0.0)),
        out_l: out_l.unwrap_or_else(|| Expr::constant(0.0)),
        out_r: out_r.flatten(),
        smoothed: vec![0.0; maps.len()],
        smooth_s: patch.params.iter().map(|p| p.smooth.unwrap_or(DEFAULT_SMOOTH_S).clamp(0.0, 10.0)).collect(),
        cost_voice: patch.voice.iter().map(node_cost).sum::<f32>() + expr_cost(patch.voice_out.as_ref()),
        cost_global: patch.global.iter().map(node_cost).sum(),
        maps,
        macro_targets,
        first_block: true,
        held_prev: [false; 16],
        padtrig: false,
        last_pitch: 261.6,
        age: 0,
        beat: 0.0,
        time: 0.0,
        rng: 0x2545_F491,
        dc: [(0.0, 0.0); 2],
        activity: vec![0.0; n_tele],
        active_voices: 0,
        unstable: false,
        tele_acc: vec![0.0; n_tele],
    })
}

/// Default smoothing for param changes (knobs, macros, morph), seconds.
pub const DEFAULT_SMOOTH_S: f32 = 0.01;

/// A node's estimated cost: its block plus expression evaluation for
/// each input that isn't a constant.
fn node_cost(n: &NodeSpec) -> f32 {
    blocks::cost(&n.kind) + n.inputs.values().map(|e| expr_cost(Some(e))).sum::<f32>()
}

fn expr_cost(e: Option<&ExprSrc>) -> f32 {
    match e {
        Some(ExprSrc::Text(s)) => 0.05 + 0.02 * s.len().min(200) as f32 / 8.0,
        _ => 0.0,
    }
}

impl Engine {
    /// Total estimated cost with `voices` voices sounding.
    #[allow(dead_code)] // API for tools and future apps; Atlas reads the parts
    pub fn cost(&self, voices: usize) -> f32 {
        self.cost_global + self.cost_voice * voices as f32
    }

    /// The most voices that fit in `budget` (at least 1), so an expensive
    /// patch plays fewer notes rather than dropping out.
    pub fn voices_within(&self, budget: f32) -> usize {
        if self.poly == 0 {
            return 0;
        }
        let per = self.cost_voice.max(1e-3);
        (((budget - self.cost_global) / per).floor().max(1.0) as usize).min(self.poly)
    }

    pub fn polyphony(&self) -> usize {
        self.poly
    }
}

fn opt_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn delay_seconds(n: &NodeSpec) -> f32 {
    match n.kind.as_str() {
        "delay" => n.opts.get("max").and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok())).unwrap_or(1.0).clamp(0.01, 4.0) as f32,
        "reverb" => 0.5,
        "shift" => 0.45,
        "comb" => 0.05,
        "allpass" => 0.1,
        "grain" => 2.0,
        "chorus" => 0.05,
        // pre-delay + diffusers + the tank at its largest size
        "plateau" => 1.4,
        "spring" => 0.1,
        "rotary" => 0.01,
        "tape" => 0.012,
        _ => 0.0,
    }
}

/// Topological order; cycles (feedback) fall back to declared order.
fn topo(deps: Vec<Vec<usize>>) -> Vec<usize> {
    let n = deps.len();
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    loop {
        let mut progressed = false;
        for i in 0..n {
            if !done[i] && deps[i].iter().all(|&d| done[d]) {
                done[i] = true;
                order.push(i);
                progressed = true;
            }
        }
        if order.len() == n {
            return order;
        }
        if !progressed {
            // break the cycle at the first unfinished node
            if let Some(i) = (0..n).find(|&i| !done[i]) {
                done[i] = true;
                order.push(i);
            }
        }
    }
}

#[inline]
fn eval_inputs(node: &mut CNode, g: &[f32], v: &[f32], rng: &mut u32) -> [f32; MAX_IN] {
    let mut out = [0.0; MAX_IN];
    for (slot, inp) in out.iter_mut().zip(node.inputs.iter()) {
        *slot = match inp {
            In::Const(c) => *c,
            In::Block(_, cached) => *cached,
            In::Sample(e) => e.eval(g, v, rng),
        };
    }
    out
}

fn refresh_block_inputs(nodes: &mut [CNode], g: &[f32], v: &[f32], rng: &mut u32) {
    for n in nodes.iter_mut() {
        for inp in n.inputs.iter_mut() {
            if let In::Block(e, cached) = inp {
                *cached = e.eval(g, v, rng);
            }
        }
    }
}

impl Engine {
    fn note_on(&mut self, pad: usize, note: i32, ctl: &Controls) {
        if self.poly == 0 {
            return;
        }
        let limit = ctl.voices_limit.clamp(1, self.poly);
        self.age += 1;
        let target = 440.0 * 2f32.powf((note - 69) as f32 / 12.0);
        // Mono legato: keep the voice, glide to the new note.
        let idx = if limit == 1 {
            0
        } else if let Some(i) = (0..limit).find(|&i| !self.voices[i].active) {
            i
        } else if let Some(i) = (0..limit).filter(|&i| !self.voices[i].gate).min_by_key(|&i| self.voices[i].age) {
            i
        } else {
            (0..limit).min_by_key(|&i| self.voices[i].age).unwrap_or(0)
        };
        let glide = ctl.glide > 0.0;
        let last = self.last_pitch;
        let v = &mut self.voices[idx];
        let legato = limit == 1 && v.gate;
        if !v.active {
            for n in v.nodes.iter_mut() {
                n.block.reset();
            }
            v.regs.iter_mut().skip(V_NODE0 as usize).for_each(|r| *r = 0.0);
        }
        v.pad = pad;
        v.note = note;
        v.target = target;
        if !glide || (!v.active && !legato && last <= 0.0) {
            v.pitch = target;
        } else if !v.active {
            v.pitch = last;
        }
        if !legato {
            v.trig = true;
            v.vtime = 0.0;
            v.stage = EnvStage::Attack;
            for n in v.nodes.iter_mut() {
                n.block.note_on();
            }
        }
        v.gate = true;
        v.active = true;
        v.quiet = 0.0;
        v.age = self.age;
        self.last_pitch = target;
    }

    fn note_off(&mut self, pad: usize) {
        for v in self.voices.iter_mut() {
            if v.gate && v.pad == pad {
                v.gate = false;
                v.stage = EnvStage::Release;
            }
        }
    }

    /// Render one block. `input` is mono (may be shorter than the block;
    /// missing samples read as silence). Never allocates.
    pub fn process(&mut self, ctl: &Controls, input: &[f32], out_l: &mut [f32], out_r: &mut [f32], sr: f32) {
        let frames = out_l.len().min(out_r.len());
        let sr = if sr > 1000.0 { sr } else { 48_000.0 };
        let inv_sr = 1.0 / sr;
        self.unstable = false;

        // ---- parameters (once per block, smoothed per param)
        let block_s = frames as f32 * inv_sr;
        self.g[G_SR as usize] = sr;
        self.g[G_BPM as usize] = ctl.bpm;
        self.g[G_PLAYING as usize] = if ctl.playing { 1.0 } else { 0.0 };
        for k in 0..MAX_MACROS {
            self.g[(G_MACRO0 as usize) + k] = ctl.macros[k].clamp(0.0, 1.0);
        }
        for i in 0..self.maps.len() {
            let mut n = ctl.norms[i];
            for (m, targets) in self.macro_targets.iter().enumerate() {
                for &(p, amt) in targets {
                    if p == i {
                        n += ctl.macros[m] * amt;
                    }
                }
            }
            let target = self.maps[i].value(n);
            let discrete = self.maps[i].steps().is_some();
            if self.first_block || discrete {
                self.smoothed[i] = target;
            } else {
                let s = self.smooth_s[i];
                let coef = if s <= 0.0 { 1.0 } else { 1.0 - (-block_s / s).exp() };
                self.smoothed[i] += (target - self.smoothed[i]) * coef;
            }
            self.g[G_PARAM0 as usize + i] = self.smoothed[i];
        }
        self.first_block = false;

        // ---- pads
        for pad in 0..16 {
            let (now, before) = (ctl.held[pad], self.held_prev[pad]);
            if now && !before {
                let note = pad_note(pad, ctl.root, ctl.scale, ctl.transpose);
                self.g[G_PADNOTE as usize] = note as f32;
                self.padtrig = true;
                self.note_on(pad, note, ctl);
            } else if !now && before {
                self.note_off(pad);
            }
        }
        self.held_prev = ctl.held;
        self.g[G_PADGATE as usize] = if ctl.held.iter().any(|h| *h) { 1.0 } else { 0.0 };

        // ---- block-rate inputs
        let Engine { g, gnodes, voices, rng, .. } = self;
        refresh_block_inputs(gnodes, g, &[], rng);
        for v in voices.iter_mut().filter(|v| v.active) {
            refresh_block_inputs(&mut v.nodes, g, &v.regs, rng);
        }

        let limit = ctl.voices_limit.clamp(1, self.poly.max(1));
        let voice_norm = 1.0 / (limit as f32).sqrt();
        let glide_coef = if ctl.glide > 0.0 { 1.0 - (-inv_sr / (ctl.glide * 0.3)).exp() } else { 1.0 };
        let att = inv_sr / ctl.attack.max(5e-4);
        let dec = 1.0 - (-inv_sr / (ctl.decay.max(1e-3) * 0.3)).exp();
        let rel = 1.0 - (-inv_sr / (ctl.release.max(1e-3) * 0.3)).exp();
        let sus = ctl.sustain.clamp(0.0, 1.0);
        let beat_inc = if ctl.playing { (ctl.bpm.clamp(20.0, 400.0) / 60.0 * inv_sr) as f64 } else { 0.0 };
        let wet = ctl.dry_wet.clamp(0.0, 1.0);
        let dc_r = 1.0 - 25.0 * inv_sr;
        self.tele_acc.iter_mut().for_each(|a| *a = 0.0);

        for n in 0..frames {
            let x_in = input.get(n).copied().unwrap_or(0.0) * ctl.input_gain;
            let ctx = Ctx { sr, inv_sr, beat: self.beat };
            self.g[G_TIME as usize] = self.time as f32;
            self.g[G_BEAT as usize] = self.beat as f32;
            self.g[G_IN as usize] = x_in;
            self.g[G_PADTRIG as usize] = if self.padtrig { 1.0 } else { 0.0 };
            self.padtrig = false;

            // voices
            let mut sum = 0.0;
            let mut active = 0;
            {
                let Engine { g, voices, voice_out, rng, amp_env, tele_acc, unstable, .. } = self;
                for v in voices.iter_mut().filter(|v| v.active) {
                    active += 1;
                    v.pitch += (v.target - v.pitch) * glide_coef;
                    match v.stage {
                        EnvStage::Attack => {
                            v.env += att;
                            if v.env >= 1.0 {
                                v.env = 1.0;
                                v.stage = EnvStage::Decay;
                            }
                        }
                        EnvStage::Decay => v.env += (sus - v.env) * dec,
                        EnvStage::Release => {
                            v.env -= v.env * rel;
                            if v.env < 1e-5 {
                                v.env = 0.0;
                                v.stage = EnvStage::Idle;
                            }
                        }
                        EnvStage::Idle => v.env = 0.0,
                    }
                    let r = &mut v.regs;
                    r[V_PITCH as usize] = v.pitch;
                    r[V_NOTE as usize] = v.note as f32;
                    r[V_GATE as usize] = if v.gate { 1.0 } else { 0.0 };
                    r[V_VEL as usize] = ctl.velocity;
                    r[V_TRIG as usize] = if v.trig { 1.0 } else { 0.0 };
                    r[V_VTIME as usize] = v.vtime;
                    r[V_AMPENV as usize] = v.env;
                    v.trig = false;
                    v.vtime += inv_sr;

                    for &k in v.order.iter() {
                        let Some(node) = v.nodes.get_mut(k) else { continue };
                        let ins = eval_inputs(node, g, &v.regs, rng);
                        let mut y = node.block.tick(&ins, &ctx, rng);
                        if !y.is_finite() {
                            node.block.reset();
                            y = 0.0;
                            *unstable = true;
                        }
                        let y = y.clamp(-1e5, 1e5);
                        if let Some(slot) = v.regs.get_mut(node.out as usize) {
                            *slot = y;
                        }
                        if let Some(a) = tele_acc.get_mut(node.tele) {
                            *a += y.abs();
                        }
                    }
                    let mut vo = voice_out.eval(g, &v.regs, rng);
                    if !vo.is_finite() {
                        vo = 0.0;
                    }
                    if *amp_env {
                        vo *= v.env;
                    }
                    sum += vo;
                    // retire silent released voices
                    if !v.gate {
                        if (*amp_env && v.stage == EnvStage::Idle) || (!*amp_env && vo.abs() < 1e-4) {
                            v.quiet += inv_sr;
                        } else {
                            v.quiet = 0.0;
                        }
                        if v.quiet > if *amp_env { 0.0 } else { 0.3 } {
                            v.active = false;
                        }
                    }
                }
            }
            self.active_voices = active;
            self.g[G_VOICES as usize] = sum * voice_norm;

            // global graph
            {
                let Engine { g, gnodes, gorder, rng, tele_acc, unstable, .. } = self;
                for &k in gorder.iter() {
                    let Some(node) = gnodes.get_mut(k) else { continue };
                    let ins = eval_inputs(node, g, &[], rng);
                    let mut y = node.block.tick(&ins, &ctx, rng);
                    if !y.is_finite() {
                        node.block.reset();
                        y = 0.0;
                        *unstable = true;
                    }
                    let y = y.clamp(-1e5, 1e5);
                    if let Some(slot) = g.get_mut(node.out as usize) {
                        *slot = y;
                    }
                    if let Some(a) = tele_acc.get_mut(node.tele) {
                        *a += y.abs();
                    }
                }
            }

            let mut l = self.out_l.eval(&self.g, &[], &mut self.rng);
            let mut r = match &self.out_r {
                Some(e) => e.eval(&self.g, &[], &mut self.rng),
                None => l,
            };
            if self.kind == Kind::Effect {
                l = x_in + (l - x_in) * wet;
                r = x_in + (r - x_in) * wet;
            }
            for (s, dc) in [(&mut l, 0usize), (&mut r, 1usize)] {
                if !s.is_finite() {
                    *s = 0.0;
                    self.unstable = true;
                }
                // DC blocker, gain, soft clip
                let (x1, y1) = self.dc[dc];
                let y = *s - x1 + dc_r * y1;
                self.dc[dc] = (*s, if y.is_finite() { y } else { 0.0 });
                *s = (y * ctl.gain).tanh();
            }
            out_l[n] = l;
            out_r[n] = r;

            self.beat += beat_inc;
            if self.beat > 1e7 {
                self.beat -= 1e7;
            }
            self.time += inv_sr as f64;
        }

        let scale = 1.0 / frames.max(1) as f32;
        for (a, acc) in self.activity.iter_mut().zip(self.tele_acc.iter()) {
            *a = acc * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn render(e: &mut Engine, ctl: &Controls, blocks: usize, input: f32) -> Vec<f32> {
        let mut l = vec![0.0; 256];
        let mut r = vec![0.0; 256];
        let inp = vec![input; 256];
        let mut out = Vec::new();
        for _ in 0..blocks {
            e.process(ctl, &inp, &mut l, &mut r, 48_000.0);
            out.extend_from_slice(&l);
        }
        out
    }

    fn patch(json: &str) -> Patch {
        serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}\n{json}"))
    }

    const SIMPLE: &str = r#"{
        "name": "Test Saw", "kind": "instrument", "voices": 4,
        "params": [{"id":"cutoff","name":"Cutoff","min":100,"max":8000,"default":2000,"curve":"exp","unit":"Hz"}],
        "voice": [
            {"id":"o","type":"osc","wave":"saw","in":{"freq":"pitch"}},
            {"id":"f","type":"filter","in":{"in":"o","cutoff":"cutoff"}}
        ],
        "voice_out": "f * 0.5",
        "out": "voices"
    }"#;

    #[test]
    fn simple_instrument_is_silent_until_a_pad_then_sounds() {
        let mut e = compile(&patch(SIMPLE), 48_000.0).unwrap();
        let mut ctl = Controls::default();
        assert!(render(&mut e, &ctl, 10, 0.0).iter().all(|x| x.abs() < 1e-6));
        ctl.held[12] = true;
        let out = render(&mut e, &ctl, 40, 0.0);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.05 && peak <= 1.0, "peak {peak}");
        assert_eq!(e.active_voices, 1);
        ctl.held[12] = false;
        render(&mut e, &ctl, 400, 0.0);
        assert_eq!(e.active_voices, 0, "released voice should retire");
    }

    #[test]
    fn validation_reports_many_problems_at_once_with_hints() {
        let bad = patch(
            r#"{"name":"", "kind":"effect", "params":[{"id":"time","name":"T"}],
                "voice":[{"id":"x","type":"osc"}],
                "global":[{"id":"d","type":"dellay"},{"id":"f","type":"filter","wave":"saw","in":{"inn":"in"}}],
                "out":"f"}"#,
        );
        let errs = compile(&bad, 48_000.0).err().expect("should fail");
        let all = errs.join("\n");
        for needle in ["name", "has no voices", "built-in", "unknown type \"dellay\"", "no input \"inn\"", "unknown option \"wave\""] {
            assert!(all.contains(needle), "missing '{needle}' in:\n{all}");
        }
    }

    #[test]
    fn expression_errors_name_the_node_and_explain_scope() {
        let bad = patch(r#"{"name":"G","kind":"generator","global":[{"id":"o","type":"osc","in":{"freq":"pitch * 2"}}],"out":"o"}"#);
        let all = compile(&bad, 48_000.0).err().unwrap().join("\n");
        assert!(all.contains("node \"o\" input \"freq\""), "{all}");
        assert!(all.contains("only exist in \"voice\" nodes"), "{all}");
    }

    #[test]
    fn feedback_loops_compile_and_stay_bounded() {
        let p = patch(
            r#"{"name":"FB","kind":"generator",
                "global":[
                    {"id":"a","type":"osc","in":{"freq":"200 + b * 400"}},
                    {"id":"b","type":"filter","mode":"bp","in":{"in":"a + b * 0.99","cutoff":800,"res":0.99}}
                ],
                "out":"b * 4"}"#,
        );
        let mut e = compile(&p, 48_000.0).unwrap();
        let out = render(&mut e, &Controls::default(), 200, 0.0);
        assert!(out.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
    }

    #[test]
    fn effect_processes_input_and_dry_wet_works() {
        let p = patch(r#"{"name":"Fx","kind":"effect","global":[{"id":"c","type":"crush","in":{"in":"in","bits":3}}],"out":"c"}"#);
        let mut e = compile(&p, 48_000.0).unwrap();
        let mut ctl = Controls::default();
        let wet = render(&mut e, &ctl, 20, 0.3);
        assert!(wet.iter().skip(1000).any(|x| x.abs() > 0.05));
        ctl.dry_wet = 0.0;
        let dry = render(&mut e, &ctl, 400, 0.3);
        // dry path is the input through the DC blocker, so it decays to ~0
        assert!(dry.last().unwrap().abs() < 0.05);
    }

    #[test]
    fn macros_and_params_drive_registers() {
        let p = patch(
            r#"{"name":"M","kind":"generator",
                "params":[{"id":"lvl","name":"Level","min":0,"max":1,"default":0}],
                "macros":[{"name":"Open","targets":[{"param":"lvl","amount":1.0}]}],
                "global":[{"id":"o","type":"osc","in":{"freq":220}}],
                "out":"o * lvl"}"#,
        );
        let mut e = compile(&p, 48_000.0).unwrap();
        let mut ctl = Controls::default();
        assert!(render(&mut e, &ctl, 10, 0.0).iter().all(|x| x.abs() < 1e-3));
        ctl.macros[0] = 1.0;
        let out = render(&mut e, &ctl, 20, 0.0);
        assert!(out.iter().any(|x| x.abs() > 0.3));
    }

    #[test]
    fn pad_mapping_is_a_scale_from_bottom_left() {
        assert_eq!(pad_degree(12), 0);
        assert_eq!(pad_degree(3), 15);
        assert_eq!(pad_note(12, 48, 1, 0), 48); // C major, lowest pad = root
        assert_eq!(pad_note(13, 48, 1, 0), 50);
        assert_eq!(pad_note(8, 48, 1, 0), 55); // degree 4 -> G
    }
}
