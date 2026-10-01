//! The Oracle's link to a language model.
//!
//! A background thread takes `Job`s, talks to the configured provider,
//! validates what comes back by compiling it, and -- if the compile fails --
//! sends the errors straight back to the model to fix (up to
//! `MAX_REPAIRS` times) before giving up. The UI and audio threads never
//! wait on the network.
//!
//! Provider, from the environment (checked in this order):
//!   ANTHROPIC_API_KEY              -> Claude (model: ORACLE_MODEL, default below)
//!   ORACLE_LLM_URL [+ ORACLE_API_KEY] -> any OpenAI-compatible endpoint
//!                                     (Ollama, LM Studio, llama.cpp...), ORACLE_MODEL required
//!   OPENAI_API_KEY                 -> OpenAI, ORACLE_MODEL required
//!   none                           -> manual mode: /prompt prints the prompt to paste
//!                                     into any chatbot, /paste loads its reply

use super::blocks::SPECS;
use super::engine::{self, Engine, GLOBAL_NAMES, VOICE_NAMES};
use super::patch::{Patch, MAX_MACROS, MAX_NODES, MAX_PARAMS};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_CLAUDE_MODEL: &str = "claude-sonnet-5-5";
const MAX_REPAIRS: u32 = 2;
const MAX_TOKENS: u32 = 8192;

/// Reads one Oracle setting: the process environment first, then the
/// project's `.env` file (`KEY=value` lines, `#` comments). `.env` is
/// gitignored, so API keys kept there never reach the repository -- the
/// whole point of supporting it instead of editing a tracked config file.
pub fn config_var(key: &str) -> Option<String> {
    if let Some(v) = std::env::var(key).ok().filter(|v| !v.trim().is_empty()) {
        return Some(v);
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".env");
    parse_dotenv(&std::fs::read_to_string(path).ok()?, key)
}

/// `KEY=value` lookup in `.env` text; tolerates `export `, quotes and
/// trailing comments after quoted values.
pub fn parse_dotenv(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else { continue };
        if k.trim() != key {
            continue;
        }
        let v = v.trim();
        let v = if let Some(rest) = v.strip_prefix('"') {
            rest.split('"').next().unwrap_or("")
        } else if let Some(rest) = v.strip_prefix('\'') {
            rest.split('\'').next().unwrap_or("")
        } else {
            v.split(" #").next().unwrap_or("").trim()
        };
        return (!v.is_empty()).then(|| v.to_string());
    }
    None
}

#[derive(Clone, Debug)]
pub enum Provider {
    Anthropic { key: String, model: String },
    OpenAi { url: String, key: Option<String>, model: String },
    Manual,
}

impl Provider {
    pub fn from_env() -> Provider {
        let var = config_var;
        if let Some(key) = var("ANTHROPIC_API_KEY") {
            return Provider::Anthropic { key, model: var("ORACLE_MODEL").unwrap_or_else(|| DEFAULT_CLAUDE_MODEL.into()) };
        }
        if let Some(url) = var("ORACLE_LLM_URL") {
            return Provider::OpenAi { url, key: var("ORACLE_API_KEY"), model: var("ORACLE_MODEL").unwrap_or_else(|| "local-model".into()) };
        }
        if let Some(key) = var("OPENAI_API_KEY") {
            if let Some(model) = var("ORACLE_MODEL") {
                return Provider::OpenAi { url: "https://api.openai.com/v1".into(), key: Some(key), model };
            }
        }
        Provider::Manual
    }

    pub fn label(&self) -> String {
        match self {
            Provider::Anthropic { model, .. } => format!("Claude ({model})"),
            Provider::OpenAi { url, model, .. } if url.contains("openai.com") => format!("OpenAI ({model})"),
            Provider::OpenAi { model, .. } => format!("Local ({model})"),
            Provider::Manual => "Manual (no API key)".into(),
        }
    }

    pub fn is_manual(&self) -> bool {
        matches!(self, Provider::Manual)
    }
}

pub enum Job {
    Generate { prompt: String },
    Refine { patch: String, instruction: String },
    Vary { patch: String },
    Explain { patch: String },
    Breed { a: String, b: String },
}

impl Job {
    pub fn label(&self) -> &'static str {
        match self {
            Job::Generate { .. } => "Generating",
            Job::Refine { .. } => "Refining",
            Job::Vary { .. } => "Varying",
            Job::Explain { .. } => "Explaining",
            Job::Breed { .. } => "Breeding",
        }
    }

    /// The user message for this job (also what /prompt prints).
    pub fn user_message(&self) -> String {
        match self {
            Job::Generate { prompt } => format!(
                "Design a new Oracle patch for this request:\n\n{prompt}\n\nReply with the complete patch JSON only."
            ),
            Job::Refine { patch, instruction } => format!(
                "Here is the current patch:\n\n{patch}\n\nChange it as follows, keeping everything the request doesn't touch \
                 (same param ids where they still make sense, so the user's knob positions carry over):\n\n{instruction}\n\n\
                 Reply with the complete updated patch JSON only."
            ),
            Job::Vary { patch } => format!(
                "Here is a patch:\n\n{patch}\n\nCreate a variation: keep its core character and param ids, but change its \
                 structure in one or two interesting ways (a new modulation path, a different filter or oscillator type, an \
                 added effect). Give it a new name. Reply with the complete patch JSON only."
            ),
            Job::Explain { patch } => format!(
                "Here is a patch:\n\n{patch}\n\nExplain to a musician, in under 120 words of plain prose (no JSON, no markdown \
                 headings): what it sounds like, how the signal flows, and which two or three controls are most worth exploring."
            ),
            Job::Breed { a, b } => format!(
                "Here are two parent patches.\n\nPARENT A:\n{a}\n\nPARENT B:\n{b}\n\nBreed them into one offspring patch that \
                 combines the most distinctive ideas of each (for example A's sound source with B's modulation or effects). \
                 Invent a name that blends both. Reply with the complete patch JSON only."
            ),
        }
    }

    fn wants_patch(&self) -> bool {
        !matches!(self, Job::Explain { .. })
    }
}

pub enum Reply {
    Patch { patch: Patch, engine: Box<Engine>, repairs: u32 },
    Text(String),
    Error(String),
}

pub struct Worker {
    tx: Sender<(u64, Job)>,
    rx: Receiver<(u64, Reply)>,
    next_id: u64,
    pub busy: Option<(u64, &'static str)>,
    pub provider: Provider,
}

impl Worker {
    pub fn spawn(provider: Provider, sample_rate: Arc<AtomicU32>) -> Worker {
        let (tx, job_rx) = channel::<(u64, Job)>();
        let (reply_tx, rx) = channel::<(u64, Reply)>();
        let p = provider.clone();
        std::thread::Builder::new()
            .name("oracle-llm".into())
            .spawn(move || {
                while let Ok((id, job)) = job_rx.recv() {
                    let sr = f32::from_bits(sample_rate.load(Ordering::Relaxed));
                    let reply = run_job(&p, &job, sr);
                    if reply_tx.send((id, reply)).is_err() {
                        break;
                    }
                }
            })
            .ok();
        Worker { tx, rx, next_id: 1, busy: None, provider }
    }

    /// Queue a job. Returns false in manual mode (nothing to send to).
    pub fn submit(&mut self, job: Job) -> bool {
        if self.provider.is_manual() {
            return false;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.busy = Some((id, job.label()));
        self.tx.send((id, job)).is_ok()
    }

    pub fn poll(&mut self) -> Option<Reply> {
        match self.rx.try_recv() {
            Ok((id, reply)) => {
                if self.busy.map(|b| b.0) == Some(id) {
                    self.busy = None;
                }
                Some(reply)
            }
            Err(_) => None,
        }
    }
}

fn run_job(p: &Provider, job: &Job, sr: f32) -> Reply {
    let system = system_prompt();
    let mut messages: Vec<(&'static str, String)> = vec![("user", job.user_message())];
    let mut repairs = 0;
    loop {
        let text = match chat(p, &system, &messages) {
            Ok(t) => t,
            Err(e) => return Reply::Error(e),
        };
        if !job.wants_patch() {
            return Reply::Text(text.trim().to_string());
        }
        let problems = match Patch::from_llm_text(&text) {
            Ok(patch) => match engine::compile(&patch, sr) {
                Ok(engine) => return Reply::Patch { patch, engine: Box::new(engine), repairs },
                Err(errs) => errs,
            },
            Err(e) => vec![e],
        };
        if repairs >= MAX_REPAIRS {
            return Reply::Error(format!("the model's patch still had problems after {MAX_REPAIRS} repairs: {}", problems.join("; ")));
        }
        repairs += 1;
        messages.push(("assistant", text));
        messages.push((
            "user",
            format!(
                "That patch failed to compile:\n- {}\n\nFix every problem and reply with the complete corrected patch JSON only.",
                problems.join("\n- ")
            ),
        ));
    }
}

fn chat(p: &Provider, system: &str, messages: &[(&str, String)]) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(240)).build();
    let msgs: Vec<serde_json::Value> =
        messages.iter().map(|(role, content)| serde_json::json!({ "role": role, "content": content })).collect();
    let (req, body) = match p {
        Provider::Anthropic { key, model } => (
            agent
                .post("https://api.anthropic.com/v1/messages")
                .set("x-api-key", key)
                .set("anthropic-version", "2023-06-01")
                .set("content-type", "application/json"),
            serde_json::json!({ "model": model, "max_tokens": MAX_TOKENS, "system": system, "messages": msgs }),
        ),
        Provider::OpenAi { url, key, model } => {
            let endpoint = if url.ends_with("/chat/completions") {
                url.clone()
            } else {
                format!("{}/chat/completions", url.trim_end_matches('/'))
            };
            let mut all = vec![serde_json::json!({ "role": "system", "content": system })];
            all.extend(msgs);
            let mut r = agent.post(&endpoint).set("content-type", "application/json");
            if let Some(k) = key {
                r = r.set("authorization", &format!("Bearer {k}"));
            }
            (r, serde_json::json!({ "model": model, "messages": all }))
        }
        Provider::Manual => return Err("no AI provider configured (manual mode: use /prompt and /paste)".into()),
    };
    let resp = req.send_string(&body.to_string());
    let text = match resp {
        Ok(r) => r.into_string().map_err(|e| format!("reading reply: {e}"))?,
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            return Err(format!("provider returned HTTP {code}: {}", body.chars().take(300).collect::<String>()));
        }
        Err(e) => return Err(format!("network error: {e}")),
    };
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("bad JSON from provider: {e}"))?;
    let out = match p {
        Provider::Anthropic { .. } => v["content"]
            .as_array()
            .map(|a| a.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("")),
        _ => v["choices"][0]["message"]["content"].as_str().map(str::to_string),
    };
    out.filter(|s| !s.is_empty()).ok_or_else(|| format!("unexpected reply shape: {}", text.chars().take(200).collect::<String>()))
}

/// The system prompt: the whole patch language, generated from the block
/// specs and built-in name tables so it can never drift from the engine.
pub fn system_prompt() -> String {
    let mut s = String::new();
    s.push_str(PROMPT_HEAD);
    s.push_str("\n## Blocks (node \"type\")\n\n");
    for b in SPECS {
        s.push_str(&format!("### {} -- {} (output {})\n", b.name, b.summary, b.output));
        if b.inputs.is_empty() {
            s.push_str("inputs: none\n");
        }
        for i in b.inputs {
            let default = if i.voice_default == i.global_default {
                i.voice_default.to_string()
            } else {
                format!("{} in voice / {} in global", i.voice_default, i.global_default)
            };
            s.push_str(&format!("- in.{}: {} (default {})\n", i.name, i.doc, default));
        }
        for o in b.opts {
            let allowed = if o.choices.is_empty() { String::new() } else { format!(" one of: {}", o.choices.join(" | ")) };
            let doc = if o.doc.is_empty() { String::new() } else { format!(" -- {}", o.doc) };
            s.push_str(&format!("- option \"{}\":{} (default {}){}\n", o.name, allowed, o.default, doc));
        }
        s.push('\n');
    }
    s.push_str("## Built-in names\n\nEverywhere:\n");
    for (n, d) in GLOBAL_NAMES {
        s.push_str(&format!("- {n}: {d}\n"));
    }
    s.push_str("\nOnly in \"voice\" nodes and voice_out:\n");
    for (n, d) in VOICE_NAMES {
        s.push_str(&format!("- {n}: {d}\n"));
    }
    s.push_str(PROMPT_TAIL);
    s.replace("{MAX_PARAMS}", &MAX_PARAMS.to_string())
        .replace("{MAX_MACROS}", &MAX_MACROS.to_string())
        .replace("{MAX_NODES}", &MAX_NODES.to_string())
}

const PROMPT_HEAD: &str = r#"You are the Oracle, the sound designer inside Portamax, a handheld music machine with 16 pads, two knobs and a small screen. You design patches: synthesizers, drum voices, effects and self-playing generators, written as JSON in the Oracle patch language below. The device compiles your JSON into a real-time audio graph. Reply with ONE JSON object and nothing else unless asked for prose.

## Patch JSON

{
  "name": "Short Evocative Name",
  "kind": "instrument" | "effect" | "generator",
  "description": "one sentence",
  "voices": 1-16,            // instrument only. 1 = mono/legato
  "amp_env": true,           // instrument: multiply voice_out by the built-in amp ADSR (keeps notes click-free). false = you shape amplitude yourself
  "params": [                // up to {MAX_PARAMS} knobs the player edits, 16 per page
    {"id": "cutoff", "name": "Cutoff", "min": 60, "max": 12000, "default": 1400, "curve": "exp", "unit": "Hz", "page": 1}
  ],
  "macros": [                // up to {MAX_MACROS} performance macros, each moves several params (amount -1..1 of the param's range)
    {"name": "Bloom", "targets": [{"param": "cutoff", "amount": 0.6}, {"param": "verb", "amount": 0.4}]}
  ],
  "voice": [ nodes ],        // instrument only: a copy runs per voice
  "global": [ nodes ],       // runs once, after the voices
  "voice_out": "expression", // instrument only: each voice's output
  "out": "expression"  or  {"left": "expression", "right": "expression"}
}

A node: {"id": "filt", "type": "filter", "mode": "lp", "in": {"in": "osc1 + osc2", "cutoff": "cutoff * (1 + env * 4)", "res": "res"}}
- "id": lowercase snake_case, unique across nodes AND params.
- Block options ("wave", "mode", "values", "max"...) sit at the top level of the node. Signal and control connections go in "in".
- Every input is an expression (or a number). Omitted inputs take the default listed below.

## Expressions

Math like C: + - * / % ^(power) , comparisons (== != < > <= >=, giving 1 or 0), && || !, ternary a ? b : c, parentheses.
Functions: sin cos tan tanh abs sqrt exp log log2 floor ceil fract round sign min max pow clamp(x,lo,hi) mix(a,b,t) step(edge,x) smoothstep(a,b,x) fold(x) (wavefolder) wrap(x) sat(x) (soft clip) crush(x,bits) mtof(note) ftom(hz) db(decibels->gain) tri(phase) saw(phase) sqr(phase) noise() ; constants pi tau e.
Names: param ids (their value in units), node ids (their current output), built-ins below. Reading a node declared later, or the node itself, gives its previous sample: that is how you build feedback.
No assignments, no statements, no strings: one formula per input.

## Signal levels

Oscillators are -1..1. Aim for peaks around 0.5 at the output: scale with a gain param or constants. The device soft-clips above 1, so hot patches sound squashed. "voices" is already scaled by 1/sqrt(voice count).
Frequencies are Hz. Times are seconds. Resonance 0..1.
"#;

const PROMPT_TAIL: &str = r#"

## Rules

- instrument: needs "voice" nodes and "voice_out"; "out" usually processes "voices" (e.g. through global reverb). Pads play notes on a scale chosen by the player; use pitch, gate, trig.
- effect: no "voice" nodes; process "in". The device adds a dry/wet control, so "out" is the wet signal.
- generator: no "voice" nodes; makes sound on its own from clock/seq/euclid/chaos blocks. Pads can still play it via padnote, padgate, padtrig.
- At most {MAX_NODES} voice nodes and {MAX_NODES} global nodes. Delay memory is limited: prefer global delays and reverbs over per-voice ones.
- Give every param a human name, a sensible range and unit, and a musical default. Group related params on the same page: page 1 = the essentials.
- Always make at least 4 macros with evocative names that move several params in musically coherent ways.
- Prefer rich, playable, surprising designs: modulation (lfo, chaos, envelopes into cutoff/pitch/fold), movement, stereo width via different left/right delay times or modulation. Make every param audibly matter.
- Keep feedback below 1 and resonance below 0.98 unless you want self-oscillation on purpose.

## Complete example

{
  "name": "Velvet Drift",
  "kind": "instrument",
  "description": "Warm detuned pad with a breathing filter and a wide shimmer tail.",
  "voices": 8,
  "amp_env": true,
  "params": [
    {"id": "detune", "name": "Detune", "min": 0, "max": 1, "default": 0.35, "page": 1},
    {"id": "cutoff", "name": "Cutoff", "min": 80, "max": 9000, "default": 1200, "curve": "exp", "unit": "Hz", "page": 1},
    {"id": "res", "name": "Resonance", "min": 0, "max": 0.95, "default": 0.25, "page": 1},
    {"id": "envamt", "name": "Filter Env", "min": 0, "max": 4, "default": 1.5, "page": 1},
    {"id": "lforate", "name": "Breath Rate", "min": 0.02, "max": 8, "default": 0.2, "curve": "exp", "unit": "Hz", "page": 2},
    {"id": "lfoamt", "name": "Breath Depth", "min": 0, "max": 1, "default": 0.4, "page": 2},
    {"id": "verb", "name": "Space", "min": 0, "max": 1, "default": 0.45, "page": 2},
    {"id": "size", "name": "Size", "min": 0.2, "max": 0.98, "default": 0.8, "page": 2}
  ],
  "macros": [
    {"name": "Open", "targets": [{"param": "cutoff", "amount": 0.6}, {"param": "envamt", "amount": 0.3}]},
    {"name": "Breathe", "targets": [{"param": "lfoamt", "amount": 0.6}, {"param": "lforate", "amount": 0.3}]},
    {"name": "Vast", "targets": [{"param": "verb", "amount": 0.5}, {"param": "size", "amount": 0.2}]},
    {"name": "Sour", "targets": [{"param": "detune", "amount": 0.6}, {"param": "res", "amount": 0.4}]}
  ],
  "voice": [
    {"id": "saws", "type": "supersaw", "in": {"freq": "pitch", "detune": "detune", "mix": 0.7}},
    {"id": "sub", "type": "osc", "wave": "sine", "in": {"freq": "pitch / 2"}},
    {"id": "fenv", "type": "adsr", "in": {"a": 0.4, "d": 1.2, "s": 0.3, "r": 1.5}},
    {"id": "breath", "type": "lfo", "shape": "smooth", "in": {"rate": "lforate"}},
    {"id": "filt", "type": "filter", "mode": "lp", "in": {"in": "saws + sub * 0.4", "cutoff": "clamp(cutoff * (1 + fenv * envamt) * (1 + breath * lfoamt), 40, 16000)", "res": "res"}}
  ],
  "voice_out": "filt * 0.6",
  "global": [
    {"id": "shim", "type": "shift", "in": {"in": "rev * 0.5", "semis": 12}},
    {"id": "rev", "type": "reverb", "in": {"in": "voices + shim * 0.25", "size": "size", "damp": 0.35}},
    {"id": "wob", "type": "lfo", "in": {"rate": 0.13}}
  ],
  "out": {"left": "voices + rev * verb", "right": "voices * 0.98 + rev * verb * (1 + wob * 0.1)"}
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_lines_parse() {
        let text = "# keys\nexport ANTHROPIC_API_KEY=\"abc123\" # mine\nORACLE_MODEL = some-model\nEMPTY=\nOTHER='q v'\n";
        assert_eq!(parse_dotenv(text, "ANTHROPIC_API_KEY").as_deref(), Some("abc123"));
        assert_eq!(parse_dotenv(text, "ORACLE_MODEL").as_deref(), Some("some-model"));
        assert_eq!(parse_dotenv(text, "OTHER").as_deref(), Some("q v"));
        assert_eq!(parse_dotenv(text, "EMPTY"), None);
        assert_eq!(parse_dotenv(text, "MISSING"), None);
    }

    #[test]
    fn system_prompt_covers_every_block_and_its_example_compiles() {
        let s = system_prompt();
        for b in SPECS {
            assert!(s.contains(&format!("### {} --", b.name)), "prompt misses block {}", b.name);
        }
        assert!(!s.contains("{MAX_"), "unreplaced placeholder");
        // The worked example in the prompt must itself be a valid patch.
        let ex = s.split("## Complete example").nth(1).unwrap();
        let patch = Patch::from_llm_text(ex).expect("example parses");
        engine::compile(&patch, 48_000.0).unwrap_or_else(|e| panic!("example fails: {e:?}"));
    }

    #[test]
    fn job_messages_embed_their_inputs() {
        let m = Job::Refine { patch: "{P}".into(), instruction: "darker".into() }.user_message();
        assert!(m.contains("{P}") && m.contains("darker"));
        assert!(Job::Explain { patch: String::new() }.user_message().contains("120 words"));
    }

    #[test]
    fn manual_mode_refuses_to_submit() {
        let mut w = Worker::spawn(Provider::Manual, Arc::new(AtomicU32::new(48_000f32.to_bits())));
        assert!(!w.submit(Job::Generate { prompt: "x".into() }));
        assert!(w.busy.is_none());
    }
}
