//! Oracle -- describe a sound, get an instrument.
//!
//! A language model designs a patch (a synth, drum voice, effect or
//! self-playing generator) as JSON in Oracle's patch language; Oracle
//! validates and compiles it into a real-time audio graph built from a fixed
//! library of DSP blocks plus small safe expressions. Nothing the model
//! writes can crash the device or block audio: bad patches are sent back
//! to the model with the compiler's errors until they work.
//!
//! On top of whatever the model makes, the player gets:
//!   - up to 64 named parameters on 4 pages, 8 macros, per-param locks
//!   - built-in voice/glide/scale/ADSR/tempo/dry-wet controls
//!   - 16 snapshot slots on the pads, with morphing between any two
//!   - local randomize / mutate / breed (no AI needed)
//!   - AI refine / vary / explain / breed, and full undo history
//!   - a library of starter patches plus saved JSON patches
//!   - every macro, the morph and the first 16 params as ModBus targets
//!
//! Controls (Oracle opens on the shared play view, see play_kit.rs):
//!   knobs  the patch's macros (then its params) in pairs; press knob1 for
//!          the next pair, knob2 to reset the pair
//!   D-pad  up/down steps through the patch library
//!   R1     the full menu, where:
//!     knob1  browse menu     press: expand/collapse group, or lock a param
//!     knob2  edit value      press: reset / run the selected action
//!   F2     pad layers: PLAY (notes), SNAP (snapshots), Controls, Moments.
//!          SNAP: tap an empty pad to store, tap a stored pad to recall,
//!          hold 0.6 s to overwrite, hold two pads to morph between them.
//!          (The Play > Pads row flips PLAY/SNAP too.)
//!   F3     Start/Stop a generator's transport (clocks, sequencers, echoes
//!          sync to it)
//!
//! Prompts: pick words in "Ask the Oracle" and press Generate -- or type
//! free text in the terminal running the sim (type /help there).
//!
//! Real-time design: the audio thread only ever runs a compiled `Engine`.
//! New engines are built off the audio thread and handed over through a
//! try-locked slot; the old one crossfades out and is handed back to be
//! freed on the UI thread.

pub mod blocks;
pub mod engine;
pub mod evolve;
pub mod expr;
pub mod library;
pub mod llm;
pub mod patch;
pub mod voice;

use crate::app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes};
use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::{cycle_source, AudioBus, NO_SOURCE};
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::{ParamList, ACCENT};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16};
use crate::util::{accelerate, note_name, AtomicF32};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use engine::{pad_note, Controls, Engine, GraphNode, SCALES};
use evolve::Rng;
use library::Entry;
use llm::{Job, Provider, Reply, Worker};
use patch::{format_value, Kind, ParamMap, Patch, State, MAX_MACROS, MAX_PARAMS};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

const APP_NAME: &str = "Oracle";
const EXT_PARAMS: usize = 16;
const SCOPE_LEN: usize = 128;
const FADE_SAMPLES: usize = 2048;
const HOLD_TO_STORE_S: f32 = 0.6;
const HISTORY_MAX: usize = 50;

// ------------------------------------------------------------ built-ins

#[derive(Clone, Copy, PartialEq, Debug)]
enum B {
    Level,
    Voices,
    Glide,
    Transpose,
    Root,
    Scale,
    Hold,
    Attack,
    Decay,
    Sustain,
    Release,
    Velocity,
    Tempo,
    InputGain,
    DryWet,
}
const NB: usize = 15;
/// Every built-in, in `Shared::builtins` order.
const ALL_B: [B; NB] = [
    B::Level,
    B::Voices,
    B::Glide,
    B::Transpose,
    B::Root,
    B::Scale,
    B::Hold,
    B::Attack,
    B::Decay,
    B::Sustain,
    B::Release,
    B::Velocity,
    B::Tempo,
    B::InputGain,
    B::DryWet,
];

struct BSpec {
    name: &'static str,
    min: f32,
    max: f32,
    default: f32,
    /// 0 = continuous, else step size
    step: f32,
    exp: bool,
}

fn bspec(b: B) -> BSpec {
    let s = |name, min, max, default, step, exp| BSpec { name, min, max, default, step, exp };
    match b {
        B::Level => s("Level", -30.0, 6.0, 0.0, 0.0, false),
        B::Voices => s("Voices", 1.0, 16.0, 8.0, 1.0, false),
        B::Glide => s("Glide", 0.0, 2.0, 0.0, 0.0, false),
        B::Transpose => s("Transpose", -24.0, 24.0, 0.0, 1.0, false),
        B::Root => s("Root", 24.0, 84.0, 48.0, 1.0, false),
        B::Scale => s("Scale", 0.0, (SCALES.len() - 1) as f32, 2.0, 1.0, false),
        B::Hold => s("Hold", 0.0, 1.0, 0.0, 1.0, false),
        B::Attack => s("Attack", 0.001, 4.0, 0.01, 0.0, true),
        B::Decay => s("Decay", 0.01, 4.0, 0.4, 0.0, true),
        B::Sustain => s("Sustain", 0.0, 1.0, 0.8, 0.0, false),
        B::Release => s("Release", 0.01, 8.0, 0.6, 0.0, true),
        B::Velocity => s("Velocity", 0.05, 1.0, 0.8, 0.0, false),
        B::Tempo => s("Tempo", 40.0, 240.0, 110.0, 1.0, false),
        B::InputGain => s("Input Gain", 0.0, 2.0, 1.0, 0.0, false),
        B::DryWet => s("Dry/Wet", 0.0, 1.0, 1.0, 0.0, false),
    }
}

fn bformat(b: B, v: f32) -> String {
    match b {
        B::Level => format!("{v:+.1} dB"),
        B::Voices | B::Tempo => format!("{v:.0}{}", if b == B::Tempo { " bpm" } else { "" }),
        B::Transpose => format!("{v:+.0} st"),
        B::Root => note_name(v as i32),
        B::Scale => SCALES.get(v as usize).map(|s| s.0).unwrap_or("?").to_string(),
        B::Hold => (if v >= 0.5 { "on (pads latch)" } else { "off" }).into(),
        B::Glide | B::Attack | B::Decay | B::Release => {
            if v < 1.0 {
                format!("{:.0} ms", v * 1000.0)
            } else {
                format!("{v:.2} s")
            }
        }
        _ => format!("{v:.2}"),
    }
}

// ---------------------------------------------------------- word banks

const KINDS: &[&str] = &["Pad", "Bass", "Lead", "Pluck", "Keys", "Bell", "Drums", "Drone", "Texture", "Effect", "Generator"];
const MOODS: &[&str] = &["Dark", "Bright", "Warm", "Cold", "Dreamy", "Aggressive", "Melancholy", "Playful", "Eerie", "Lush", "Gritty", "Pristine"];
const TEXTURES: &[&str] = &["Glassy", "Metallic", "Wooden", "Airy", "Dusty", "Liquid", "Crystalline", "Rubbery", "Vocal", "Fuzzy", "Granular", "Choral"];
const MOTIONS: &[&str] = &["Static", "Breathing", "Pulsing", "Evolving", "Chaotic", "Rhythmic", "Drifting", "Stuttering", "Swelling", "Orbiting"];
const REFINES: &[&str] = &[
    "Darker",
    "Brighter",
    "More movement",
    "Calmer",
    "More chaos",
    "Simpler",
    "Richer",
    "Wider stereo",
    "Punchier",
    "Longer tail",
    "Add a sub",
    "More grit",
    "Cleaner",
    "Weirder",
    "Add a rhythmic element",
    "More dramatic macros",
    "Add a new page of params",
];

fn compose_prompt(kind: usize, mood: usize, texture: usize, motion: usize) -> String {
    let (k, m, t, mo) = (KINDS[kind], MOODS[mood].to_lowercase(), TEXTURES[texture].to_lowercase(), MOTIONS[motion].to_lowercase());
    match k {
        "Effect" => format!("A {m}, {t}, {mo} audio effect that transforms whatever is fed into it (kind: effect)."),
        "Generator" => format!("A self-playing {m}, {t}, {mo} generative patch that makes music on its own (kind: generator)."),
        _ => format!("A {m}, {t}, {mo} {} instrument played from the pads (kind: instrument).", k.to_lowercase()),
    }
}

// ---------------------------------------------------- shared UI<->audio

struct Shared {
    norms: [AtomicF32; MAX_PARAMS],
    macros: [AtomicF32; MAX_MACROS],
    builtins: [AtomicF32; NB],
    held: [AtomicBool; 16],
    playing: AtomicBool,
    source: AtomicUsize,
    morph: AtomicF32,
    morph_on: AtomicBool,
    snap_a: [AtomicF32; MAX_PARAMS],
    snap_b: [AtomicF32; MAX_PARAMS],
    ext_params: [Arc<AtomicF32>; EXT_PARAMS],
    ext_macros: [Arc<AtomicF32>; MAX_MACROS],
    ext_morph: Arc<AtomicF32>,
    pending: Mutex<Option<Box<Engine>>>,
    retired: Mutex<Option<Box<Engine>>>,
    sample_rate: Arc<AtomicU32>,
    activity: [AtomicF32; 64],
    scope: [AtomicF32; SCOPE_LEN],
    scope_pos: AtomicUsize,
    peak: AtomicF32,
    load: AtomicF32,
    active_voices: AtomicUsize,
    unstable: AtomicBool,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Shared {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register(APP_NAME, modbus);
        let all = ALL_B;
        Self {
            norms: std::array::from_fn(|_| AtomicF32::new(0.0)),
            macros: std::array::from_fn(|_| AtomicF32::new(0.0)),
            builtins: std::array::from_fn(|i| AtomicF32::new(bspec(all[i]).default)),
            held: std::array::from_fn(|_| AtomicBool::new(false)),
            playing: AtomicBool::new(true),
            source: AtomicUsize::new(NO_SOURCE),
            morph: AtomicF32::new(0.0),
            morph_on: AtomicBool::new(false),
            snap_a: std::array::from_fn(|_| AtomicF32::new(0.0)),
            snap_b: std::array::from_fn(|_| AtomicF32::new(0.0)),
            ext_params: std::array::from_fn(|i| modbus.register(format!("{APP_NAME}: P{}", i + 1))),
            ext_macros: std::array::from_fn(|i| modbus.register(format!("{APP_NAME}: Macro {}", i + 1))),
            ext_morph: modbus.register(format!("{APP_NAME}: Morph")),
            pending: Mutex::new(None),
            retired: Mutex::new(None),
            sample_rate: Arc::new(AtomicU32::new(48_000f32.to_bits())),
            activity: std::array::from_fn(|_| AtomicF32::new(0.0)),
            scope: std::array::from_fn(|_| AtomicF32::new(0.0)),
            scope_pos: AtomicUsize::new(0),
            peak: AtomicF32::new(0.0),
            load: AtomicF32::new(0.0),
            active_voices: AtomicUsize::new(0),
            unstable: AtomicBool::new(false),
            bus_out: audio_bus.register(APP_NAME),
            mix_level,
            ext_mix_level,
        }
    }

    fn b(&self, b: B) -> f32 {
        self.builtins[b as usize].get()
    }

    fn set_b(&self, b: B, v: f32) {
        self.builtins[b as usize].set(v);
    }

    /// Snapshot everything the engine needs this block.
    fn controls(&self) -> Controls {
        let morph_on = self.morph_on.load(Ordering::Relaxed);
        let m = (self.morph.get() + self.ext_morph.get()).clamp(0.0, 1.0);
        let mut c = Controls {
            norms: std::array::from_fn(|i| {
                let base = if morph_on {
                    let (a, b) = (self.snap_a[i].get(), self.snap_b[i].get());
                    a + (b - a) * m
                } else {
                    self.norms[i].get()
                };
                base + if i < EXT_PARAMS { self.ext_params[i].get() } else { 0.0 }
            }),
            macros: std::array::from_fn(|k| (self.macros[k].get() + self.ext_macros[k].get()).clamp(0.0, 1.0)),
            held: std::array::from_fn(|i| self.held[i].load(Ordering::Relaxed)),
            ..Controls::default()
        };
        c.gain = 10f32.powf(self.b(B::Level) / 20.0);
        c.voices_limit = self.b(B::Voices).round().max(1.0) as usize;
        c.glide = self.b(B::Glide);
        c.transpose = self.b(B::Transpose).round() as i32;
        c.root = self.b(B::Root).round() as i32;
        c.scale = self.b(B::Scale).round().max(0.0) as usize;
        c.attack = self.b(B::Attack);
        c.decay = self.b(B::Decay);
        c.sustain = self.b(B::Sustain);
        c.release = self.b(B::Release);
        c.velocity = self.b(B::Velocity);
        c.bpm = self.b(B::Tempo);
        c.playing = self.playing.load(Ordering::Relaxed);
        c.dry_wet = self.b(B::DryWet);
        c.input_gain = self.b(B::InputGain);
        c
    }
}

// ---------------------------------------------------------- the processor

struct OracleProcessor {
    shared: Arc<Shared>,
    audio_bus: Arc<AudioBus>,
    current: Option<Box<Engine>>,
    fading: Option<Box<Engine>>,
    fade: usize,
    input: Vec<f32>,
    l: Vec<f32>,
    r: Vec<f32>,
    fl: Vec<f32>,
    fr: Vec<f32>,
    mono: Vec<f32>,
}

impl AudioProcessor for OracleProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let start = Instant::now();
        let channels = channels.max(1);
        let frames = buffer.len() / channels;
        let s = &self.shared;
        s.sample_rate.store(sample_rate.to_bits(), Ordering::Relaxed);

        // 1. Take a new engine if one is waiting (never blocks).
        if self.fading.is_none() {
            if let Ok(mut slot) = s.pending.try_lock() {
                if slot.is_some() {
                    self.fading = self.current.take();
                    self.current = slot.take();
                    self.fade = if self.fading.is_some() { FADE_SAMPLES } else { 0 };
                }
            }
        }
        // 2. Hand a fully faded-out engine back to the UI thread to free.
        if self.fading.is_some() && self.fade == 0 {
            if let Ok(mut slot) = s.retired.try_lock() {
                if slot.is_none() {
                    *slot = self.fading.take();
                }
            }
        }

        for v in [&mut self.input, &mut self.l, &mut self.r, &mut self.fl, &mut self.fr, &mut self.mono] {
            v.clear();
            v.resize(frames, 0.0);
        }

        // 3. Input from the chosen source app (never ourselves).
        let src = s.source.load(Ordering::Relaxed);
        if src != NO_SOURCE {
            if let Some(buf) = self.audio_bus.get(src) {
                if !Arc::ptr_eq(&buf, &s.bus_out) {
                    if let Ok(b) = buf.try_lock() {
                        for (d, x) in self.input.iter_mut().zip(b.iter()) {
                            *d = *x;
                        }
                    }
                }
            }
        }

        // 4. Render (plus the outgoing engine while crossfading).
        let ctl = s.controls();
        if let Some(e) = self.current.as_mut() {
            e.process(&ctl, &self.input, &mut self.l, &mut self.r, sample_rate);
        }
        if self.fade > 0 {
            if let Some(old) = self.fading.as_mut() {
                old.process(&ctl, &self.input, &mut self.fl, &mut self.fr, sample_rate);
            }
            for i in 0..frames {
                let t = if self.fade > 0 { self.fade as f32 / FADE_SAMPLES as f32 } else { 0.0 };
                self.fade = self.fade.saturating_sub(1);
                let (gin, gout) = ((1.0 - t).sqrt(), t.sqrt());
                self.l[i] = self.l[i] * gin + self.fl[i] * gout;
                self.r[i] = self.r[i] * gin + self.fr[i] * gout;
            }
        }

        // 5. Telemetry.
        if let Some(e) = self.current.as_ref() {
            for (a, v) in s.activity.iter().zip(e.activity.iter()) {
                a.set(*v);
            }
            s.active_voices.store(e.active_voices, Ordering::Relaxed);
            if e.unstable {
                s.unstable.store(true, Ordering::Relaxed);
            }
        }
        let mut peak = 0.0f32;
        for i in 0..frames {
            self.mono[i] = (self.l[i] + self.r[i]) * 0.5;
            peak = peak.max(self.mono[i].abs());
        }
        s.peak.set(peak.max(s.peak.get() * 0.85));
        for half in 0..2 {
            let seg = &self.mono[(half * frames / 2)..((half + 1) * frames / 2).max(half * frames / 2)];
            let v = seg.iter().copied().fold(0.0f32, |m, x| if x.abs() > m.abs() { x } else { m });
            let pos = s.scope_pos.fetch_add(1, Ordering::Relaxed) % SCOPE_LEN;
            s.scope[pos].set(v);
        }

        // 6. Publish pre-fader, then write post-fader stereo.
        if let Ok(mut bus) = s.bus_out.try_lock() {
            bus.clear();
            bus.extend_from_slice(&self.mono);
        }
        let g = (s.mix_level.get() + s.ext_mix_level.get()).clamp(0.0, 2.0);
        for (i, frame) in buffer.chunks_mut(channels).enumerate() {
            for (ch, out) in frame.iter_mut().enumerate() {
                *out = g * match ch {
                    0 => self.l[i],
                    1 => self.r[i],
                    _ => self.mono[i],
                };
            }
        }
        let budget = frames as f32 / sample_rate.max(1.0);
        s.load.set(start.elapsed().as_secs_f32() / budget.max(1e-6));
    }
}

// --------------------------------------------------------- the menu model

#[derive(Clone, Copy, PartialEq, Debug)]
enum Sel {
    AskKind,
    AskMood,
    AskTexture,
    AskMotion,
    AskGo,
    SpeakNew,
    RefineWord,
    RefineGo,
    SpeakChange,
    Vary,
    Explain,
    AiInfo,
    Param(usize),
    Macro(usize),
    Builtin(B),
    Source,
    PadMode,
    Morph,
    RandStrength,
    Randomize,
    Mutation,
    Mutate,
    Breed,
    AiBreed,
    ClearSnaps,
    Lib(usize),
    Save,
    Undo,
    Redo,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Group(usize),
    Leaf(Sel),
}

const G_ORACLE: usize = 0;
const G_PAGE0: usize = 1; // 1..=4
const G_MACROS: usize = 5;
const G_PLAY: usize = 6;
const G_EVOLVE: usize = 7;
const G_LIBRARY: usize = 8;
const NUM_GROUPS: usize = 9;

#[derive(Clone)]
struct Snap {
    norms: Vec<f32>,
    macros: [f32; MAX_MACROS],
}

pub struct OracleApp {
    shared: Arc<Shared>,
    audio_bus: Arc<AudioBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    patch: Patch,
    maps: Vec<ParamMap>,
    graph: Vec<GraphNode>,
    out_reads_voices: bool,
    out_reads_in: bool,
    locks: [bool; MAX_PARAMS],
    history: Vec<(Patch, Vec<f32>)>,
    hist_pos: usize,
    snaps: Vec<Option<Snap>>,
    snap_a: Option<usize>,
    snap_b: Option<usize>,
    library: Vec<Entry>,
    worker: Worker,
    busy_since: Option<Instant>,
    voice: Option<voice::Take>,
    stt: Option<voice::SttProvider>,
    status: String,
    explain: Option<String>,
    ask: [usize; 4],
    refine: usize,
    rand_strength: f32,
    mutation: f32,
    rng: Rng,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    snap_mode: bool,
    prev_grid: [bool; 16],
    pad_down_at: [Option<Instant>; 16],
    pad_morphed: [bool; 16],
    latched: [bool; 16],
    stdin: Option<Receiver<String>>,
    paste: Option<String>,
    frame: u64,
    breed_partner: usize,
    /// The shared play view (play_kit.rs).
    kit: PlayKit,
}

/// One play-view control. Oracle's controls aren't fixed: they're
/// whatever the current patch exposes, so the list is rebuilt from the
/// patch (see `OracleApp::kit_controls`) rather than being a const table.
#[derive(Clone, Copy, PartialEq, Debug)]
enum K {
    S(Sel),
    /// Steps through the library (the D-pad's browse).
    Patch,
}

/// The library browser always sits on the last Controls pad so the
/// D-pad's `browse` index never moves when the patch changes.
const C_PATCH: usize = 15;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "oracle",
        // Oracle's own Play/Snap pad modes are its first two layers, so F2
        // reaches them exactly as it did before, then the shared ones.
        layers: vec![Layer::Native(0, "PLAY"), Layer::Native(1, "SNAP"), Layer::Controls, Layer::Moments],
        hero: vec![[0, 1], [2, 3], [4, 5], [6, 7]],
        // D-pad up/down = next/previous library patch: auditioning patches
        // is the move a player makes most once the AI isn't involved.
        browse: Some(C_PATCH),
        // Macros are what a patch designer (human or model) intends to be
        // performed, and they apply even while morphing, so the stick and
        // hands push the first four controls (macros first).
        routes: Routes { stick_x: Some(0), stick_y: Some(1), hand_l: Some(2), hand_r: Some(3) },
        throws: Vec::new(),
        midi_to_pads: true,
        own_expression: false,
    }
}

static STDIN_RX: OnceLock<Mutex<Option<Receiver<String>>>> = OnceLock::new();

/// One stdin reader for the whole program; the first Oracle takes it.
fn take_stdin() -> Option<Receiver<String>> {
    if cfg!(test) || std::env::var("PORTAMAX_RENDER_WAV").is_ok() || std::env::var("PORTAMAX_RENDER_PNG").is_ok() {
        return None;
    }
    let cell = STDIN_RX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("oracle-stdin".into())
            .spawn(move || {
                let stdin = std::io::stdin();
                let mut line = String::new();
                loop {
                    line.clear();
                    match stdin.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            if tx.send(line.trim_end_matches(['\n', '\r']).to_string()).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
            .ok();
        Mutex::new(Some(rx))
    });
    cell.lock().ok().and_then(|mut g| g.take())
}

impl OracleApp {
    pub fn new(
        sensitivity: Arc<AtomicF32>,
        nav_speed: Arc<AtomicF32>,
        modbus: Arc<ModBus>,
        audio_bus: Arc<AudioBus>,
        mixer_bus: Arc<MixerBus>,
    ) -> Self {
        let shared = Arc::new(Shared::new(&modbus, &audio_bus, &mixer_bus));
        let provider = Provider::from_env();
        let worker = Worker::spawn(provider, Arc::clone(&shared.sample_rate));
        let first: Patch = serde_json::from_str(library::STARTERS[0].1).expect("starter 0 is valid (tested)");
        let mut app = Self {
            shared,
            audio_bus,
            sensitivity,
            nav_speed,
            patch: first.clone(),
            maps: Vec::new(),
            graph: Vec::new(),
            out_reads_voices: false,
            out_reads_in: false,
            locks: [false; MAX_PARAMS],
            history: Vec::new(),
            hist_pos: 0,
            snaps: vec![None; 16],
            snap_a: None,
            snap_b: None,
            library: library::scan(),
            worker,
            busy_since: None,
            voice: None,
            stt: voice::SttProvider::from_env(),
            status: String::new(),
            explain: None,
            ask: [0, 4, 0, 1],
            refine: 0,
            rand_strength: 0.4,
            mutation: 0.3,
            rng: Rng::seeded(),
            list: ParamList::new(),
            expanded: [true, true, false, false, false, false, false, false, false],
            snap_mode: false,
            prev_grid: [false; 16],
            pad_down_at: [None; 16],
            pad_morphed: [false; 16],
            latched: [false; 16],
            stdin: take_stdin(),
            paste: None,
            frame: 0,
            breed_partner: 1,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
        };
        app.install(first, None, Carry::Fresh, "Loaded");
        app.status = format!("Oracle ready -- {}. Pick words and Generate, or type in the terminal.", app.worker.provider.label());
        if app.stdin.is_some() {
            println!("\n[Oracle] AI: {}. Type a sound description here and press Enter (/help for commands).", app.worker.provider.label());
        }
        app
    }

    // ------------------------------------------------------------ patches

    fn sr(&self) -> f32 {
        f32::from_bits(self.shared.sample_rate.load(Ordering::Relaxed))
    }

    fn norms(&self) -> Vec<f32> {
        (0..self.maps.len()).map(|i| self.shared.norms[i].get()).collect()
    }

    fn macros(&self) -> [f32; MAX_MACROS] {
        std::array::from_fn(|k| self.shared.macros[k].get())
    }

    /// The current patch with the knob positions baked into `state`.
    fn patch_with_state(&self) -> Patch {
        let mut p = self.patch.clone();
        let params = p
            .params
            .iter()
            .enumerate()
            .map(|(i, spec)| (spec.id.clone(), self.maps[i].value(self.shared.norms[i].get())))
            .collect();
        p.state = Some(State { params, macros: self.macros().to_vec() });
        p
    }

    fn patch_json_for_ai(&self) -> String {
        let mut p = self.patch_with_state();
        p.state = None;
        serde_json::to_string(&p).unwrap_or_default()
    }

    /// Compile (if needed) and hand a patch to the audio thread.
    fn install(&mut self, patch: Patch, engine: Option<Box<Engine>>, carry: Carry, verb: &str) -> bool {
        let engine = match engine {
            Some(e) => e,
            None => match engine::compile(&patch, self.sr()) {
                Ok(e) => Box::new(e),
                Err(errs) => {
                    self.set_status(format!("Patch failed: {}", errs.first().cloned().unwrap_or_default()));
                    if self.stdin.is_some() {
                        println!("[Oracle] patch \"{}\" failed to compile:\n  - {}", patch.name, errs.join("\n  - "));
                    }
                    return false;
                }
            },
        };
        // Only generators expose Play/Stop (see `running`), so a patch
        // without a transport must never inherit a stopped one.
        if patch.kind != Kind::Generator {
            self.shared.playing.store(true, Ordering::Relaxed);
        }
        let old_vals: Vec<(String, f32)> = self
            .patch
            .params
            .iter()
            .enumerate()
            .filter_map(|(i, p)| self.maps.get(i).map(|m| (p.id.clone(), m.value(self.shared.norms[i].get()))))
            .collect();
        let old_locks: Vec<String> =
            self.patch.params.iter().enumerate().filter(|(i, _)| self.locks[*i]).map(|(_, p)| p.id.clone()).collect();

        let maps: Vec<ParamMap> = patch.params.iter().map(ParamMap::from_spec).collect();
        for (i, (spec, map)) in patch.params.iter().zip(&maps).enumerate().take(MAX_PARAMS) {
            let from_state = patch.state.as_ref().and_then(|s| s.params.get(&spec.id)).copied();
            let carried = match carry {
                Carry::KeepMatching => old_vals.iter().find(|(id, _)| *id == spec.id).map(|(_, v)| *v),
                Carry::Fresh => None,
            };
            let v = carried.or(from_state).unwrap_or(spec.default);
            self.shared.norms[i].set(map.norm(v));
            self.locks[i] = carry == Carry::KeepMatching && old_locks.contains(&spec.id);
        }
        for i in patch.params.len()..MAX_PARAMS {
            self.locks[i] = false;
        }
        if carry == Carry::Fresh {
            let saved = patch.state.as_ref().map(|s| s.macros.clone()).unwrap_or_default();
            for k in 0..MAX_MACROS {
                self.shared.macros[k].set(saved.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0));
            }
            self.shared.set_b(B::Voices, patch.voices as f32);
        } else {
            self.shared.set_b(B::Voices, self.shared.b(B::Voices).clamp(1.0, patch.voices as f32));
        }

        self.graph = engine.graph.clone();
        self.out_reads_voices = engine.out_reads_voices;
        self.out_reads_in = engine.out_reads_in;
        self.maps = maps;
        self.patch = patch;
        self.shared.morph_on.store(false, Ordering::Relaxed);
        self.snaps = vec![None; 16];
        self.snap_a = None;
        self.snap_b = None;
        self.shared.unstable.store(false, Ordering::Relaxed);
        if let Ok(mut slot) = self.shared.pending.lock() {
            *slot = Some(engine); // a not-yet-taken older engine is dropped here, on the UI thread
        }
        self.push_history();
        let name = self.patch.name.clone();
        self.set_status(format!("{verb} \"{name}\" ({}, {} params)", self.patch.kind.label(), self.patch.params.len()));
        if self.stdin.is_some() {
            println!("[Oracle] {verb} \"{name}\" -- {}", self.patch.description);
        }
        true
    }

    fn push_history(&mut self) {
        let entry = (self.patch.clone(), self.norms());
        self.history.truncate(self.hist_pos.saturating_add(1).min(self.history.len()));
        if self.history.last().map(|h| h.0 == entry.0).unwrap_or(false) {
            return;
        }
        self.history.push(entry);
        if self.history.len() > HISTORY_MAX {
            self.history.remove(0);
        }
        self.hist_pos = self.history.len() - 1;
    }

    fn step_history(&mut self, dir: i32) {
        let target = self.hist_pos as i32 + dir;
        if target < 0 || target as usize >= self.history.len() {
            self.set_status(if dir < 0 { "Nothing to undo" } else { "Nothing to redo" }.into());
            return;
        }
        let (patch, norms) = self.history[target as usize].clone();
        let pos = target as usize;
        match engine::compile(&patch, self.sr()) {
            Ok(e) => {
                self.graph = e.graph.clone();
                self.out_reads_voices = e.out_reads_voices;
                self.out_reads_in = e.out_reads_in;
                self.maps = patch.params.iter().map(ParamMap::from_spec).collect();
                for (i, v) in norms.iter().enumerate().take(MAX_PARAMS) {
                    self.shared.norms[i].set(*v);
                }
                self.patch = patch;
                self.snaps = vec![None; 16];
                self.shared.morph_on.store(false, Ordering::Relaxed);
                if let Ok(mut slot) = self.shared.pending.lock() {
                    *slot = Some(Box::new(e));
                }
                self.hist_pos = pos;
                self.set_status(format!("{} -> \"{}\" ({}/{})", if dir < 0 { "Undo" } else { "Redo" }, self.patch.name, pos + 1, self.history.len()));
            }
            Err(errs) => self.set_status(format!("History entry failed: {}", errs.join("; "))),
        }
    }

    fn set_status(&mut self, s: String) {
        self.status = s;
    }

    // --------------------------------------------------------------- AI

    /// Press once to talk, again to stop (a pause also stops it). The
    /// words then go to the Oracle as a new patch or a change.
    fn toggle_voice(&mut self, intent: voice::Intent) {
        if let Some(t) = &self.voice {
            if t.recording() {
                t.finish();
                self.set_status("Got it -- transcribing...".into());
            } else {
                self.set_status("Still transcribing the last take...".into());
            }
            return;
        }
        if self.worker.busy.is_some() {
            self.set_status("The Oracle is still thinking...".into());
            return;
        }
        if intent == voice::Intent::Change && self.patch.params.is_empty() && self.patch.name.is_empty() {
            self.set_status("Nothing to change yet -- speak a new patch first".into());
            return;
        }
        self.voice = Some(voice::Take::start(intent, self.stt.clone()));
        self.set_status(match intent {
            voice::Intent::New => "Listening... describe the sound you want. Press again to stop.".into(),
            voice::Intent::Change => "Listening... say what to change. Press again to stop.".into(),
        });
    }

    fn poll_voice(&mut self) {
        let Some(result) = self.voice.as_ref().and_then(|t| t.poll()) else { return };
        let intent = self.voice.take().map_or(voice::Intent::New, |t| t.intent);
        match result {
            voice::VoiceResult::Text(words) => {
                println!("[Oracle] heard: \"{words}\"");
                match intent {
                    voice::Intent::New => self.submit(Job::Generate { prompt: words.clone() }),
                    voice::Intent::Change => {
                        let patch = self.patch_json_for_ai();
                        self.submit(Job::Refine { patch, instruction: words.clone() });
                    }
                }
                if self.worker.busy.is_some() {
                    self.set_status(format!("Heard: \"{}\"", fit(&words, 90)));
                }
            }
            voice::VoiceResult::Saved(path) => self.set_status(format!(
                "Recorded to {path}. Set ORACLE_STT_URL (local whisper server) or OPENAI_API_KEY to turn speech into text."
            )),
            voice::VoiceResult::Error(e) => self.set_status(e),
        }
    }

    fn submit(&mut self, job: Job) {
        let label = job.label();
        if self.worker.busy.is_some() {
            self.set_status("The Oracle is still thinking...".into());
            return;
        }
        if self.worker.provider.is_manual() {
            let text = format!("=== SYSTEM ===\n{}\n\n=== USER ===\n{}\n", llm::system_prompt(), job.user_message());
            let path = library::patch_dir().join("../prompt.txt");
            let saved = std::fs::create_dir_all(library::patch_dir()).and_then(|_| std::fs::write(&path, &text)).is_ok();
            self.set_status(
                "No AI key set. Prompt written to apps/oracle/prompt.txt: paste it into any chatbot, then paste its JSON reply into the terminal."
                    .into(),
            );
            if self.stdin.is_some() {
                println!(
                    "[Oracle] Manual mode (set ANTHROPIC_API_KEY for automatic). {} -- paste the reply JSON here.",
                    if saved { "Prompt saved to apps/oracle/prompt.txt" } else { "Couldn't save prompt.txt" }
                );
            }
            return;
        }
        if self.worker.submit(job) {
            self.busy_since = Some(Instant::now());
            self.explain = None;
            self.set_status(format!("{label}..."));
        }
    }

    fn poll_worker(&mut self) {
        let Some(reply) = self.worker.poll() else { return };
        let secs = self.busy_since.take().map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
        match reply {
            Reply::Patch { patch, engine, repairs } => {
                let carry = if patch.params.iter().any(|p| self.patch.params.iter().any(|q| q.id == p.id)) {
                    Carry::KeepMatching
                } else {
                    Carry::Fresh
                };
                let fixed = if repairs > 0 { format!(", {repairs} self-repair{}", if repairs > 1 { "s" } else { "" }) } else { String::new() };
                self.install(patch, Some(engine), carry, &format!("Summoned in {secs:.0}s{fixed}:"));
            }
            Reply::Text(t) => {
                if self.stdin.is_some() {
                    println!("[Oracle] {t}");
                }
                self.explain = Some(t);
                self.set_status("Explanation ready (select Explain to read it)".into());
            }
            Reply::Error(e) => {
                if self.stdin.is_some() {
                    println!("[Oracle] error: {e}");
                }
                self.set_status(format!("Error: {e}"));
            }
        }
    }

    // ------------------------------------------------------------ terminal

    fn poll_stdin(&mut self) {
        let mut lines = Vec::new();
        if let Some(rx) = self.stdin.as_ref() {
            while let Ok(l) = rx.try_recv() {
                lines.push(l);
            }
        }
        for line in lines {
            self.command(&line);
        }
    }

    fn command(&mut self, raw: &str) {
        if let Some(buf) = self.paste.as_mut() {
            buf.push_str(raw);
            buf.push('\n');
            if let Some(json) = patch::extract_json(buf).map(str::to_string) {
                self.paste = None;
                match serde_json::from_str::<Patch>(&json) {
                    Ok(p) => {
                        self.install(p, None, Carry::KeepMatching, "Pasted");
                    }
                    Err(e) => println!("[Oracle] pasted JSON isn't a valid patch: {e}"),
                }
            }
            return;
        }
        let line = raw.trim();
        if line.is_empty() {
            return;
        }
        if line.starts_with('{') {
            self.paste = Some(String::new());
            self.command(raw);
            return;
        }
        let (cmd, arg) = match line.split_once(' ') {
            Some((c, a)) => (c, a.trim()),
            None => (line, ""),
        };
        match cmd {
            "/help" => println!(
                "[Oracle] commands:\n  <text>            generate a new patch from a description\n  + <text>          refine the current patch (also /refine <text>)\n  /vary /explain    AI variation / explanation of the current patch\n  /breed <name>     AI-breed the current patch with a library patch\n  /random [0-1]     randomize unlocked params     /undo /redo\n  /save [name]      save to apps/oracle/patches   /load <name>   /list\n  /json             print the current patch       /paste  or paste JSON directly\n  /model            show the AI provider"
            ),
            "+" | "/refine" => {
                if arg.is_empty() {
                    println!("[Oracle] usage: + make it darker");
                } else {
                    let patch = self.patch_json_for_ai();
                    self.submit(Job::Refine { patch, instruction: arg.to_string() });
                }
            }
            "/vary" => {
                let patch = self.patch_json_for_ai();
                self.submit(Job::Vary { patch });
            }
            "/explain" => {
                let patch = self.patch_json_for_ai();
                self.submit(Job::Explain { patch });
            }
            "/breed" => match self.find_library(arg) {
                Some(i) => self.ai_breed(i),
                None => println!("[Oracle] no library patch matching \"{arg}\" (try /list)"),
            },
            "/random" => {
                let s = arg.parse::<f32>().unwrap_or(self.rand_strength);
                self.randomize(s);
            }
            "/undo" => self.step_history(-1),
            "/redo" => self.step_history(1),
            "/save" => {
                if !arg.is_empty() {
                    self.patch.name = arg.to_string();
                }
                self.save();
                println!("[Oracle] {}", self.status);
            }
            "/load" => match self.find_library(arg) {
                Some(i) => self.load_library(i),
                None => println!("[Oracle] no library patch matching \"{arg}\""),
            },
            "/list" => {
                for (i, e) in self.library.iter().enumerate() {
                    println!("  {:2}. {} ({})", i + 1, e.name, e.kind);
                }
            }
            "/json" => println!("{}", self.patch_with_state().to_json()),
            "/paste" => {
                self.paste = Some(String::new());
                println!("[Oracle] paste the patch JSON now");
            }
            "/model" => println!("[Oracle] {}", self.worker.provider.label()),
            c if c.starts_with('/') => println!("[Oracle] unknown command {c} (/help)"),
            _ => self.submit(Job::Generate { prompt: line.to_string() }),
        }
    }

    fn find_library(&mut self, needle: &str) -> Option<usize> {
        self.library = library::scan();
        let n = needle.to_lowercase();
        if n.is_empty() {
            return None;
        }
        if let Ok(k) = n.parse::<usize>() {
            if k >= 1 && k <= self.library.len() {
                return Some(k - 1);
            }
        }
        self.library.iter().position(|e| e.name.to_lowercase().contains(&n))
    }

    fn load_library(&mut self, i: usize) {
        let Some(entry) = self.library.get(i).cloned() else { return };
        match library::load(&entry) {
            Ok(p) => {
                self.install(p, None, Carry::Fresh, "Loaded");
            }
            Err(e) => self.set_status(format!("Load failed: {e}")),
        }
    }

    fn save(&mut self) {
        match library::save(&self.patch_with_state()) {
            Ok(path) => {
                self.library = library::scan();
                self.set_status(format!("Saved {}", path.file_name().and_then(|f| f.to_str()).unwrap_or("patch")));
            }
            Err(e) => self.set_status(format!("Save failed: {e}")),
        }
    }

    fn ai_breed(&mut self, partner: usize) {
        let Some(entry) = self.library.get(partner).cloned() else { return };
        match library::load(&entry) {
            Ok(mut other) => {
                other.state = None;
                let a = self.patch_json_for_ai();
                let b = serde_json::to_string(&other).unwrap_or_default();
                self.submit(Job::Breed { a, b });
            }
            Err(e) => self.set_status(format!("Breed failed: {e}")),
        }
    }

    // ----------------------------------------------------- local evolve

    fn randomize(&mut self, strength: f32) {
        let mut v = self.norms();
        let n = v.len();
        evolve::randomize(&mut v, &self.locks[..n], strength, &mut self.rng);
        for (i, x) in v.iter().enumerate() {
            self.shared.norms[i].set(*x);
        }
        self.shared.morph_on.store(false, Ordering::Relaxed);
        self.set_status(format!("Randomized {} params ({:.0}%)", v.len() - self.locks.iter().take(v.len()).filter(|l| **l).count(), strength * 100.0));
    }

    fn fill_variants(&mut self, children: Vec<Vec<f32>>, what: &str) {
        let m = self.macros();
        for (k, c) in children.into_iter().enumerate() {
            self.snaps[12 + k] = Some(Snap { norms: c, macros: m });
        }
        self.snap_mode = true;
        self.kit.set_native(1);
        self.set_status(format!("{what}: 4 variants on the bottom-right pads (Snap mode)"));
    }

    fn mutate(&mut self) {
        let base = self.norms();
        let locks = self.locks;
        let kids = (0..4).map(|_| evolve::mutate(&base, &locks[..base.len()], self.mutation, &mut self.rng)).collect();
        self.fill_variants(kids, "Mutated");
    }

    fn breed(&mut self) {
        let (Some(a), Some(b)) = (self.snap_a.and_then(|i| self.snaps[i].clone()), self.snap_b.and_then(|i| self.snaps[i].clone())) else {
            self.set_status("Breed needs two snapshots: hold two stored pads in Snap mode first".into());
            return;
        };
        let locks = self.locks;
        let n = a.norms.len();
        let kids = (0..4).map(|_| evolve::breed(&a.norms, &b.norms, &locks[..n], self.mutation, &mut self.rng)).collect();
        self.fill_variants(kids, "Bred A x B");
    }

    fn recall(&mut self, slot: usize) {
        if let Some(s) = self.snaps[slot].clone() {
            for (i, v) in s.norms.iter().enumerate().take(MAX_PARAMS) {
                self.shared.norms[i].set(*v);
            }
            for k in 0..MAX_MACROS {
                self.shared.macros[k].set(s.macros[k]);
            }
            self.shared.morph_on.store(false, Ordering::Relaxed);
            self.set_status(format!("Recalled snapshot {}", slot + 1));
        }
    }

    fn store(&mut self, slot: usize) {
        self.snaps[slot] = Some(Snap { norms: self.norms(), macros: self.macros() });
        self.set_status(format!("Stored snapshot {}", slot + 1));
    }

    fn set_morph_pair(&mut self, a: usize, b: usize) {
        let (Some(sa), Some(sb)) = (self.snaps[a].clone(), self.snaps[b].clone()) else {
            self.set_status("Morph needs two stored pads".into());
            return;
        };
        for i in 0..MAX_PARAMS {
            self.shared.snap_a[i].set(sa.norms.get(i).copied().unwrap_or(0.0));
            self.shared.snap_b[i].set(sb.norms.get(i).copied().unwrap_or(0.0));
        }
        self.snap_a = Some(a);
        self.snap_b = Some(b);
        self.shared.morph_on.store(true, Ordering::Relaxed);
        self.set_status(format!("Morphing {} <-> {}: turn Evolve > Morph", a + 1, b + 1));
    }

    // ---------------------------------------------------------- the menu

    fn group_name(&self, g: usize) -> String {
        match g {
            G_ORACLE => "Ask the Oracle".into(),
            G_MACROS => "Macros".into(),
            G_PLAY => "Play".into(),
            G_EVOLVE => "Morph & Evolve".into(),
            G_LIBRARY => "Library".into(),
            p => format!("Page {}", p - G_PAGE0 + 1),
        }
    }

    fn group_leaves(&self, g: usize) -> Vec<Sel> {
        let inst = self.patch.kind == Kind::Instrument;
        match g {
            G_ORACLE => vec![
                Sel::AskKind,
                Sel::AskMood,
                Sel::AskTexture,
                Sel::AskMotion,
                Sel::AskGo,
                Sel::SpeakNew,
                Sel::RefineWord,
                Sel::RefineGo,
                Sel::SpeakChange,
                Sel::Vary,
                Sel::Explain,
                Sel::AiInfo,
            ],
            G_MACROS => (0..self.patch.macros.len().min(MAX_MACROS)).map(Sel::Macro).collect(),
            G_PLAY => {
                let mut v = vec![Sel::Builtin(B::Level), Sel::PadMode];
                if inst {
                    v.extend([Sel::Builtin(B::Voices), Sel::Builtin(B::Glide)]);
                }
                v.extend([Sel::Builtin(B::Root), Sel::Builtin(B::Scale), Sel::Builtin(B::Transpose), Sel::Builtin(B::Hold)]);
                if inst && self.patch.amp_env {
                    v.extend([Sel::Builtin(B::Attack), Sel::Builtin(B::Decay), Sel::Builtin(B::Sustain), Sel::Builtin(B::Release)]);
                }
                if inst {
                    v.push(Sel::Builtin(B::Velocity));
                }
                v.extend([Sel::Builtin(B::Tempo), Sel::Source, Sel::Builtin(B::InputGain)]);
                if self.patch.kind == Kind::Effect {
                    v.push(Sel::Builtin(B::DryWet));
                }
                v
            }
            G_EVOLVE => vec![
                Sel::Morph,
                Sel::Randomize,
                Sel::RandStrength,
                Sel::Mutate,
                Sel::Breed,
                Sel::Mutation,
                Sel::AiBreed,
                Sel::ClearSnaps,
            ],
            G_LIBRARY => {
                let mut v = vec![Sel::Save, Sel::Undo, Sel::Redo];
                v.extend((0..self.library.len()).map(Sel::Lib));
                v
            }
            p => {
                let page = p - G_PAGE0;
                (0..self.patch.params.len().min(MAX_PARAMS)).filter(|&i| self.patch.page_of(i) == page).map(Sel::Param).collect()
            }
        }
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for g in 0..NUM_GROUPS {
            let leaves = self.group_leaves(g);
            if leaves.is_empty() && g != G_ORACLE {
                continue;
            }
            rows.push(Row::Group(g));
            if self.expanded[g] {
                rows.extend(leaves.into_iter().map(Row::Leaf));
            }
        }
        rows
    }

    fn leaf_name(&self, sel: Sel) -> String {
        match sel {
            Sel::AskKind => "Kind".into(),
            Sel::AskMood => "Mood".into(),
            Sel::AskTexture => "Texture".into(),
            Sel::AskMotion => "Motion".into(),
            Sel::AskGo => ">> Generate".into(),
            Sel::RefineWord => "Refine".into(),
            Sel::RefineGo => ">> Apply refine".into(),
            Sel::SpeakNew => ">> Speak a new patch".into(),
            Sel::SpeakChange => ">> Speak a change".into(),
            Sel::Vary => ">> AI variation".into(),
            Sel::Explain => ">> Explain".into(),
            Sel::AiInfo => "AI".into(),
            Sel::Param(i) => {
                let name = self.patch.params.get(i).map(|p| p.name.as_str()).unwrap_or("?");
                format!("{}{}", if self.locks[i] { "*" } else { "" }, name)
            }
            Sel::Macro(k) => self.patch.macros.get(k).map(|m| m.name.clone()).unwrap_or_default(),
            Sel::Builtin(b) => bspec(b).name.into(),
            Sel::Source => "Source".into(),
            Sel::PadMode => "Pads".into(),
            Sel::Morph => "Morph".into(),
            Sel::RandStrength => "Random Amount".into(),
            Sel::Randomize => ">> Randomize".into(),
            Sel::Mutation => "Mutation".into(),
            Sel::Mutate => ">> Mutate x4".into(),
            Sel::Breed => ">> Breed A x B".into(),
            Sel::AiBreed => ">> AI breed with".into(),
            Sel::ClearSnaps => ">> Clear snapshots".into(),
            Sel::Lib(i) => self.library.get(i).map(|e| e.name.clone()).unwrap_or_default(),
            Sel::Save => ">> Save current".into(),
            Sel::Undo => ">> Undo".into(),
            Sel::Redo => ">> Redo".into(),
        }
    }

    fn leaf_value(&self, sel: Sel) -> String {
        let s = &self.shared;
        match sel {
            Sel::AskKind => KINDS[self.ask[0]].into(),
            Sel::AskMood => MOODS[self.ask[1]].into(),
            Sel::AskTexture => TEXTURES[self.ask[2]].into(),
            Sel::AskMotion => MOTIONS[self.ask[3]].into(),
            Sel::AskGo | Sel::RefineGo | Sel::Vary | Sel::Explain => {
                if self.worker.busy.is_some() {
                    "thinking...".into()
                } else {
                    "press knob2".into()
                }
            }
            Sel::SpeakNew | Sel::SpeakChange => {
                let want = if sel == Sel::SpeakNew { voice::Intent::New } else { voice::Intent::Change };
                match &self.voice {
                    Some(t) if t.intent == want && t.recording() => format!("listening {:.0}s", t.elapsed()),
                    Some(t) if t.intent == want => "transcribing...".into(),
                    _ if self.worker.busy.is_some() => "thinking...".into(),
                    _ => match &self.stt {
                        Some(p) => p.label(),
                        None => "record only".into(),
                    },
                }
            }
            Sel::RefineWord => REFINES[self.refine].into(),
            Sel::AiInfo => self.worker.provider.label(),
            Sel::Param(i) => match (self.patch.params.get(i), self.maps.get(i)) {
                (Some(spec), Some(map)) => format_value(spec, map, map.value(s.norms[i].get())),
                _ => String::new(),
            },
            Sel::Macro(k) => format!("{:.0}%", s.macros[k].get() * 100.0),
            Sel::Builtin(b) => bformat(b, s.b(b)),
            Sel::Source => self.audio_bus.source_name(s.source.load(Ordering::Relaxed)),
            Sel::PadMode => (if self.snap_mode { "Snap (snapshots)" } else { "Play (notes)" }).into(),
            Sel::Morph => {
                if s.morph_on.load(Ordering::Relaxed) {
                    format!("{:.0}%  ({} > {})", s.morph.get() * 100.0, self.snap_a.map_or(0, |a| a + 1), self.snap_b.map_or(0, |b| b + 1))
                } else {
                    "hold 2 pads (Snap)".into()
                }
            }
            Sel::RandStrength => format!("{:.0}%", self.rand_strength * 100.0),
            Sel::Mutation => format!("{:.0}%", self.mutation * 100.0),
            Sel::Randomize | Sel::Mutate | Sel::ClearSnaps => "press knob2".into(),
            Sel::Breed => {
                if self.snap_a.is_some() && self.snap_b.is_some() {
                    "press knob2".into()
                } else {
                    "needs A & B".into()
                }
            }
            Sel::AiBreed => self.library.get(self.breed_partner).map(|e| e.name.clone()).unwrap_or_default(),
            Sel::Lib(i) => self.library.get(i).map(|e| e.kind.to_string()).unwrap_or_default(),
            Sel::Save => "press knob2".into(),
            Sel::Undo | Sel::Redo => format!("{}/{}", self.hist_pos + 1, self.history.len()),
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            G_ORACLE => {
                if self.worker.busy.is_some() {
                    "thinking...".into()
                } else if self.worker.provider.is_manual() {
                    "manual".into()
                } else {
                    "ready".into()
                }
            }
            G_MACROS => format!("{}", self.patch.macros.len()),
            G_PLAY => format!("{} {}", note_name(self.shared.b(B::Root) as i32), SCALES.get(self.shared.b(B::Scale) as usize).map_or("", |s| s.0)),
            G_EVOLVE => format!("{} snaps", self.snaps.iter().filter(|s| s.is_some()).count()),
            G_LIBRARY => format!("{}", self.library.len()),
            p => format!("{} params", self.group_leaves(p).len()),
        }
    }

    fn edit(&mut self, sel: Sel, delta: i32) {
        if delta == 0 {
            return;
        }
        let sens = self.sensitivity.get();
        let step = delta.signum();
        let cyc = |cur: usize, n: usize| (cur as i32 + step).rem_euclid(n as i32) as usize;
        match sel {
            Sel::AskKind => self.ask[0] = cyc(self.ask[0], KINDS.len()),
            Sel::AskMood => self.ask[1] = cyc(self.ask[1], MOODS.len()),
            Sel::AskTexture => self.ask[2] = cyc(self.ask[2], TEXTURES.len()),
            Sel::AskMotion => self.ask[3] = cyc(self.ask[3], MOTIONS.len()),
            Sel::RefineWord => self.refine = cyc(self.refine, REFINES.len()),
            Sel::Param(i) => {
                let Some(map) = self.maps.get(i) else { return };
                let cur = self.shared.norms[i].get();
                let next = match map.steps() {
                    Some(n) => cur + step as f32 / (n.saturating_sub(1).max(1)) as f32,
                    None => cur + accelerate(delta) * sens * 0.05,
                };
                self.shared.norms[i].set(next.clamp(0.0, 1.0));
                self.shared.morph_on.store(false, Ordering::Relaxed);
            }
            Sel::Macro(k) => {
                let v = (self.shared.macros[k].get() + accelerate(delta) * sens * 0.05).clamp(0.0, 1.0);
                self.shared.macros[k].set(v);
            }
            Sel::Builtin(b) => {
                let spec = bspec(b);
                let mut max = spec.max;
                if b == B::Voices {
                    max = self.patch.voices as f32;
                }
                let cur = self.shared.b(b);
                let next = if spec.step > 0.0 {
                    cur + step as f32 * spec.step
                } else if spec.exp {
                    cur * 1.08f32.powf(accelerate(delta) * sens * 10.0)
                } else {
                    cur + accelerate(delta) * sens * (spec.max - spec.min) * 0.05
                };
                self.shared.set_b(b, next.clamp(spec.min, max));
                if b == B::Hold && next < 0.5 {
                    self.latched = [false; 16];
                }
            }
            Sel::Source => {
                let next = cycle_source(self.shared.source.load(Ordering::Relaxed), step, self.audio_bus.len());
                self.shared.source.store(next, Ordering::Relaxed);
            }
            Sel::PadMode => self.flip_pad_mode(),
            Sel::Morph => {
                let v = (self.shared.morph.get() + accelerate(delta) * sens * 0.05).clamp(0.0, 1.0);
                self.shared.morph.set(v);
            }
            Sel::RandStrength => self.rand_strength = (self.rand_strength + step as f32 * 0.05).clamp(0.05, 1.0),
            Sel::Mutation => self.mutation = (self.mutation + step as f32 * 0.05).clamp(0.0, 1.0),
            Sel::AiBreed => self.breed_partner = cyc(self.breed_partner, self.library.len().max(1)),
            _ => {}
        }
    }

    fn press(&mut self, sel: Sel) {
        match sel {
            Sel::AskGo => {
                let prompt = compose_prompt(self.ask[0], self.ask[1], self.ask[2], self.ask[3]);
                self.submit(Job::Generate { prompt });
            }
            Sel::RefineGo => {
                let patch = self.patch_json_for_ai();
                self.submit(Job::Refine { patch, instruction: REFINES[self.refine].to_string() });
            }
            Sel::SpeakNew => self.toggle_voice(voice::Intent::New),
            Sel::SpeakChange => self.toggle_voice(voice::Intent::Change),
            Sel::Vary => {
                let patch = self.patch_json_for_ai();
                self.submit(Job::Vary { patch });
            }
            Sel::Explain => {
                let patch = self.patch_json_for_ai();
                self.submit(Job::Explain { patch });
            }
            Sel::Param(i) => {
                if let (Some(spec), Some(map)) = (self.patch.params.get(i), self.maps.get(i)) {
                    self.shared.norms[i].set(map.norm(spec.default));
                }
            }
            Sel::Macro(k) => self.shared.macros[k].set(0.0),
            Sel::Builtin(b) => {
                let d = if b == B::Voices { self.patch.voices as f32 } else { bspec(b).default };
                self.shared.set_b(b, d);
            }
            Sel::Source => self.shared.source.store(NO_SOURCE, Ordering::Relaxed),
            Sel::PadMode => self.flip_pad_mode(),
            Sel::Morph => self.shared.morph.set(0.0),
            Sel::Randomize => self.randomize(self.rand_strength),
            Sel::Mutate => self.mutate(),
            Sel::Breed => self.breed(),
            Sel::AiBreed => self.ai_breed(self.breed_partner),
            Sel::ClearSnaps => {
                self.snaps = vec![None; 16];
                self.snap_a = None;
                self.snap_b = None;
                self.shared.morph_on.store(false, Ordering::Relaxed);
                self.set_status("Snapshots cleared".into());
            }
            Sel::Lib(i) => self.load_library(i),
            Sel::Save => self.save(),
            Sel::Undo => self.step_history(-1),
            Sel::Redo => self.step_history(1),
            _ => {}
        }
    }

    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match *row {
                Row::Group(g) => {
                    let arrow = if self.expanded[g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(g)), self.group_summary(g), true)
                }
                Row::Leaf(sel) => (self.leaf_name(sel), self.leaf_value(sel), false),
            })
            .collect()
    }

    #[allow(dead_code)] // Slint GUI only; see app.rs
    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        self.list.selected = self.list.selected.min(rows.len() - 1);
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        (rows[start..end].to_vec(), self.list.selected - start, start > 0, end < rows.len())
    }

    // --------------------------------------------------------------- pads

    fn handle_pads(&mut self, grid: &[bool; 16]) {
        let hold = self.shared.b(B::Hold) >= 0.5;
        if !self.snap_mode {
            for i in 0..16 {
                if hold && grid[i] && !self.prev_grid[i] {
                    self.latched[i] = !self.latched[i];
                }
                let on = if hold { self.latched[i] } else { grid[i] };
                self.shared.held[i].store(on, Ordering::Relaxed);
            }
            self.prev_grid = *grid;
            return;
        }
        // Snap mode: pads are snapshot slots, never notes (unless latched).
        for i in 0..16 {
            self.shared.held[i].store(hold && self.latched[i], Ordering::Relaxed);
        }
        let now = Instant::now();
        for i in 0..16 {
            if grid[i] && !self.prev_grid[i] {
                self.pad_down_at[i] = Some(now);
                self.pad_morphed[i] = false;
                // A second pad while one is held: morph between them.
                if let Some(j) = (0..16).find(|&j| j != i && grid[j] && self.prev_grid[j]) {
                    self.pad_morphed[i] = true;
                    self.pad_morphed[j] = true;
                    self.set_morph_pair(j, i);
                }
            }
            if !grid[i] && self.prev_grid[i] {
                let held_for = self.pad_down_at[i].take().map(|t| now.duration_since(t).as_secs_f32()).unwrap_or(0.0);
                if !self.pad_morphed[i] {
                    if held_for >= HOLD_TO_STORE_S || self.snaps[i].is_none() {
                        self.store(i);
                    } else {
                        self.recall(i);
                    }
                }
            }
        }
        self.prev_grid = *grid;
    }

    // ------------------------------------------------------------ drawing

    /// Level (row) of every graph node, top to bottom: IN, voice nodes by
    /// dependency depth, the voice sum, global nodes by depth, then OUT.
    fn graph_levels(&self) -> (Vec<usize>, Option<usize>, Option<usize>, usize) {
        let n = self.graph.len();
        // Longest path from a source, ignoring back edges (feedback loops).
        fn visit(i: usize, g: &[GraphNode], state: &mut [u8], depth: &mut [usize]) -> usize {
            if state[i] == 2 {
                return depth[i];
            }
            if state[i] == 1 {
                return 0; // back edge: part of a feedback loop
            }
            state[i] = 1;
            let mut d = 0;
            for &p in &g[i].deps {
                if p < g.len() && state[p] != 1 {
                    d = d.max(visit(p, g, state, depth) + 1);
                }
            }
            state[i] = 2;
            depth[i] = d;
            d
        }
        let mut depth = vec![0usize; n];
        let mut state = vec![0u8; n];
        for i in 0..n {
            visit(i, &self.graph, &mut state, &mut depth);
        }
        let has_in = self.graph.iter().any(|g| g.reads_in) || self.out_reads_in;
        let in_level = if has_in { Some(0) } else { None };
        let base = usize::from(has_in);
        let vdepth = self.graph.iter().zip(&depth).filter(|(g, _)| g.voice).map(|(_, d)| d + 1).max().unwrap_or(0);
        let vox_level = if vdepth > 0 { Some(base + vdepth) } else { None };
        let gbase = base + vdepth + usize::from(vdepth > 0);
        let levels: Vec<usize> =
            (0..n).map(|i| if self.graph[i].voice { base + depth[i] } else { gbase + depth[i] }).collect();
        let out_level = levels.iter().copied().max().map_or(gbase, |m| (m + 1).max(gbase)).max(vox_level.map_or(0, |v| v + 1));
        (levels, in_level, vox_level, out_level)
    }

    /// Lays the signal graph out top-down inside (x0, y0, w, h): one
    /// box per block (plus IN / voices / OUT), and the wiring between
    /// them. Shared by the device screen and the Slint panel.
    fn graph_layout(&self, x0: i32, y0: i32, w: i32, h: i32) -> GraphLayout {
        let (levels, in_level, vox_level, out_level) = self.graph_levels();
        let nlevels = out_level + 1;
        #[derive(Clone, Copy)]
        enum Item {
            In,
            Vox,
            Node(usize),
            Out,
        }
        let mut rows: Vec<Vec<Item>> = vec![Vec::new(); nlevels];
        if let Some(l) = in_level {
            rows[l].push(Item::In);
        }
        for (i, l) in levels.iter().enumerate() {
            rows[*l].push(Item::Node(i));
        }
        if let Some(l) = vox_level {
            rows[l].push(Item::Vox);
        }
        rows[out_level].push(Item::Out);

        let level_h = (h / nlevels as i32).clamp(12, 40);
        let box_h = (level_h - 6).clamp(8, 14);
        let top = y0 + (h - level_h * nlevels as i32).max(0) / 2;
        let key = |it: &Item| match it {
            Item::In => usize::MAX - 1,
            Item::Vox => usize::MAX - 2,
            Item::Out => usize::MAX - 3,
            Item::Node(i) => *i,
        };
        let mut rect = std::collections::HashMap::new(); // item key -> (x, y, w)
        let mut boxes = Vec::new();
        for (l, row) in rows.iter().enumerate() {
            let m = row.len().max(1) as i32;
            let slot = w / m;
            let bw = (slot - 6).clamp(10, 70);
            for (k, it) in row.iter().enumerate() {
                let x = x0 + k as i32 * slot + (slot - bw) / 2;
                let y = top + l as i32 * level_h;
                rect.insert(key(it), (x, y, bw));
                let (label, activity, voice) = match it {
                    Item::In => ("IN".to_string(), 0.35, false),
                    Item::Out => ("OUT".to_string(), self.shared.peak.get() * 1.5, false),
                    Item::Vox => {
                        let v = self.shared.active_voices.load(Ordering::Relaxed);
                        (format!("voices {v}"), (v as f32 / 3.0).min(1.0), true)
                    }
                    Item::Node(i) => (self.graph[*i].id.clone(), self.shared.activity.get(*i).map_or(0.0, |a| a.get()), self.graph[*i].voice),
                };
                boxes.push(GraphBox { x, y, w: bw, label, activity: activity.clamp(0.0, 1.0), voice });
            }
        }
        let bottom = |k: usize| rect.get(&k).map(|&(x, y, bw)| Point::new(x + bw / 2, y + box_h));
        let topc = |k: usize| rect.get(&k).map(|&(x, y, bw)| Point::new(x + bw / 2, y));
        let mut edges = Vec::new();
        let mut edge = |from: usize, to: usize| {
            if let (Some(a), Some(b)) = (bottom(from), topc(to)) {
                edges.push((a, b, b.y > a.y));
            }
        };
        let (k_in, k_vox, k_out) = (usize::MAX - 1, usize::MAX - 2, usize::MAX - 3);
        for (i, g) in self.graph.iter().enumerate() {
            for &d in &g.deps {
                edge(d, i);
            }
            if g.reads_in {
                edge(k_in, i);
            }
            if g.reads_voices {
                edge(k_vox, i);
            }
            if g.to_out {
                edge(i, if g.voice { k_vox } else { k_out });
            }
        }
        if self.out_reads_voices {
            edge(k_vox, k_out);
        }
        if self.out_reads_in {
            edge(k_in, k_out);
        }
        GraphLayout { boxes, box_h, edges }
    }

    fn draw_graph(&self, fb: &mut FrameBuffer, x0: i32, y0: i32, w: i32, h: i32) {
        let layout = self.graph_layout(x0, y0, w, h);
        let box_h = layout.box_h;
        let fwd = PrimitiveStyle::with_stroke(Rgb565::new(5, 16, 8), 1);
        let back = PrimitiveStyle::with_stroke(Rgb565::new(10, 8, 4), 1);
        for &(a, b, forward) in &layout.edges {
            Line::new(a, b).into_styled(if forward { fwd } else { back }).draw(fb).ok();
        }
        let label = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(20, 44, 22));
        let dark = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::BLACK);
        let border = Rgb565::new(6, 12, 6);
        for b in &layout.boxes {
            let lvl = b.activity.sqrt();
            let fill = if b.voice {
                Rgb565::new((2.0 + lvl * 6.0) as u8, (6.0 + lvl * 50.0) as u8, (4.0 + lvl * 10.0) as u8)
            } else {
                Rgb565::new((2.0 + lvl * 4.0) as u8, (6.0 + lvl * 34.0) as u8, (6.0 + lvl * 24.0) as u8)
            };
            let r = Rectangle::new(Point::new(b.x, b.y), Size::new(b.w as u32, box_h as u32));
            r.into_styled(PrimitiveStyle::with_fill(fill)).draw(fb).ok();
            r.into_styled(PrimitiveStyle::with_stroke(border, 1)).draw(fb).ok();
            if box_h >= 10 {
                let chars = ((b.w - 4) / 6).max(1) as usize;
                let t: String = b.label.chars().take(chars).collect();
                let tx = b.x + (b.w - t.len() as i32 * 6) / 2;
                Text::new(&t, Point::new(tx, b.y + box_h - 3), if lvl > 0.55 { dark } else { label }).draw(fb).ok();
            }
        }
    }

    #[allow(dead_code)] // Slint GUI only; see app.rs
    /// Everything the Slint home screen's Oracle panel shows -- the same
    /// state the device screen's right panel draws.
    fn panel_extra(&self) -> crate::app::OracleExtra {
        use crate::app::{CurveSegments, OracleExtra, OracleNode};
        let (gw, gh) = (290, 128);
        let layout = self.graph_layout(0, 0, gw, gh);
        let mut edges = CurveSegments::default();
        let mut ordered: Vec<_> = layout.edges.iter().filter(|e| e.2).collect();
        let forward_edges = ordered.len();
        ordered.extend(layout.edges.iter().filter(|e| !e.2));
        for (a, b, _) in ordered {
            let (dx, dy) = ((b.x - a.x) as f32, (b.y - a.y) as f32);
            edges.mid_x.push((a.x + b.x) as f32 / 2.0);
            edges.mid_y.push((a.y + b.y) as f32 / 2.0);
            edges.length.push((dx * dx + dy * dy).sqrt().max(0.5));
            edges.angle_deg.push(dy.atan2(dx).to_degrees());
        }
        let nodes = layout
            .boxes
            .into_iter()
            .map(|b| OracleNode { x: b.x as f32, y: b.y as f32, w: b.w as f32, h: layout.box_h as f32, label: b.label, activity: b.activity, voice: b.voice })
            .collect();
        let pos = self.shared.scope_pos.load(Ordering::Relaxed);
        let scope = (0..SCOPE_LEN).map(|k| self.shared.scope[(pos + k) % SCOPE_LEN].get().clamp(-1.0, 1.0)).collect();
        let snaps = std::array::from_fn(|i| {
            if Some(i) == self.snap_a {
                2
            } else if Some(i) == self.snap_b {
                3
            } else {
                u8::from(self.snaps[i].is_some())
            }
        });
        let morph = self.shared.morph_on.load(Ordering::Relaxed).then(|| (self.shared.morph.get() + self.shared.ext_morph.get()).clamp(0.0, 1.0));
        let selected = self.visible_rows().get(self.list.selected).copied();
        let listening = self.voice.as_ref().is_some_and(|t| t.recording());
        let transcribing = self.voice.as_ref().is_some_and(|t| t.transcribing());
        let mode = if listening {
            4
        } else if transcribing || self.busy_since.is_some() {
            2
        } else if self.snap_mode {
            1
        } else if matches!(selected, Some(Row::Leaf(Sel::Explain)) | Some(Row::Leaf(Sel::AiInfo))) && self.explain.is_some() {
            3
        } else {
            0
        };
        let thinking = match self.busy_since {
            _ if transcribing => format!("Transcribing{}", ".".repeat((self.frame / 15 % 4) as usize)),
            _ if listening => self.voice.as_ref().map_or(String::new(), |t| format!("{:.0}s / {:.0}s", t.elapsed(), voice::MAX_SECONDS)),
            Some(since) => {
                let label = self.worker.busy.map(|b| b.1).unwrap_or("Thinking");
                format!("{label}{}  {:.0}s", ".".repeat((self.frame / 15 % 4) as usize), since.elapsed().as_secs_f32())
            }
            None => String::new(),
        };
        let voices = self.shared.active_voices.load(Ordering::Relaxed);
        let info = format!(
            "{}  load {:.0}%{}",
            if self.patch.kind == Kind::Instrument { format!("{voices}/{} voices", self.shared.b(B::Voices) as usize) } else { self.patch.kind.label().into() },
            self.shared.load.get() * 100.0,
            if self.shared.playing.load(Ordering::Relaxed) { "" } else { "  stopped" }
        );
        OracleExtra {
            patch_name: fit(&self.patch.name, 28),
            kind_label: self.patch.kind.label().to_uppercase(),
            mode,
            nodes,
            edges,
            forward_edges,
            scope,
            snaps,
            morph,
            info,
            status: self.status.clone(),
            unstable: self.shared.unstable.load(Ordering::Relaxed),
            explain: self.explain.clone().unwrap_or_default(),
            thinking,
            mic_level: self.voice.as_ref().map_or(0.0, |t| t.level()),
        }
    }

    fn draw_snap_grid(&self, fb: &mut FrameBuffer, x0: i32, y0: i32) {
        let (cell, gap) = (50, 6);
        let label = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(20, 44, 22));
        let dark = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::BLACK);
        for i in 0..16 {
            let (r, c) = ((i / 4) as i32, (i % 4) as i32);
            let (x, y) = (x0 + c * (cell + gap), y0 + r * (cell + gap));
            let stored = self.snaps[i].is_some();
            let fill = if Some(i) == self.snap_a {
                Rgb565::new(28, 56, 0)
            } else if Some(i) == self.snap_b {
                Rgb565::new(0, 36, 30)
            } else if stored {
                Rgb565::new(4, 28, 8)
            } else {
                Rgb565::new(3, 6, 3)
            };
            Rectangle::new(Point::new(x, y), Size::new(cell as u32, cell as u32))
                .into_styled(PrimitiveStyle::with_fill(fill))
                .draw(fb)
                .ok();
            let tag = if Some(i) == self.snap_a {
                "A".to_string()
            } else if Some(i) == self.snap_b {
                "B".to_string()
            } else {
                format!("{}", i + 1)
            };
            let style = if stored { dark } else { label };
            Text::new(&tag, Point::new(x + 4, y + 13), style).draw(fb).ok();
        }
        if self.shared.morph_on.load(Ordering::Relaxed) {
            let y = y0 + 4 * (cell + gap) + 2;
            let m = (self.shared.morph.get() + self.shared.ext_morph.get()).clamp(0.0, 1.0);
            Rectangle::new(Point::new(x0, y), Size::new(218, 5)).into_styled(PrimitiveStyle::with_fill(Rgb565::new(3, 6, 3))).draw(fb).ok();
            Rectangle::new(Point::new(x0, y), Size::new((218.0 * m) as u32, 5)).into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(fb).ok();
        }
    }

    fn draw_scope(&self, fb: &mut FrameBuffer, x0: i32, y0: i32, w: i32, h: i32) {
        let pos = self.shared.scope_pos.load(Ordering::Relaxed);
        let style = PrimitiveStyle::with_stroke(ACCENT, 1);
        let mid = y0 + h / 2;
        Line::new(Point::new(x0, mid), Point::new(x0 + w, mid)).into_styled(PrimitiveStyle::with_stroke(Rgb565::new(4, 8, 4), 1)).draw(fb).ok();
        let mut prev: Option<Point> = None;
        for k in 0..SCOPE_LEN {
            let v = self.shared.scope[(pos + k) % SCOPE_LEN].get().clamp(-1.0, 1.0);
            let p = Point::new(x0 + (k as i32 * w) / SCOPE_LEN as i32, mid - (v * (h / 2) as f32) as i32);
            if let Some(q) = prev {
                Line::new(q, p).into_styled(style).draw(fb).ok();
            }
            prev = Some(p);
        }
    }
}

impl OracleApp {
    /// Play/Snap with the side effects the old F2 toggle had: entering
    /// Snap releases un-latched notes so a pad held for a note doesn't
    /// hang once the pads become snapshot slots.
    fn set_snap_mode(&mut self, on: bool) {
        self.snap_mode = on;
        if on && self.shared.b(B::Hold) < 0.5 {
            for h in &self.shared.held {
                h.store(false, Ordering::Relaxed);
            }
        }
        self.set_status(if on {
            "Snap mode: tap empty pad = store, tap = recall, hold 2 = morph".into()
        } else {
            "Play mode: pads play notes".into()
        });
    }

    /// The menu's Play > Pads row: flips PLAY/SNAP and moves the kit's
    /// pad layer with it (F2 now cycles the kit's layers instead).
    fn flip_pad_mode(&mut self) {
        let on = !self.snap_mode;
        self.set_snap_mode(on);
        self.kit.set_native(u8::from(on));
    }

    /// Next/previous library patch from wherever the current one sits.
    /// A patch that isn't in the library (AI-made, pasted) starts from the
    /// first entry going up and the last going down.
    fn step_patch(&mut self, delta: i32) {
        let n = self.library.len() as i32;
        if n == 0 || delta == 0 {
            return;
        }
        let dir = delta.signum();
        let cur = self.library.iter().position(|e| e.name == self.patch.name).map_or(if dir > 0 { -1 } else { 0 }, |p| p as i32);
        self.load_library((cur + dir).rem_euclid(n) as usize);
    }

    fn pad_tuning(&self) -> (i32, usize, i32) {
        let s = &self.shared;
        (s.b(B::Root).round() as i32, s.b(B::Scale).round().max(0.0) as usize, s.b(B::Transpose).round() as i32)
    }

    /// The built-ins that matter most for this kind of patch, after the
    /// knobs: level and morph always, then the envelope for enveloped
    /// instruments, the wet/dry and input for effects, tempo for
    /// generators.
    fn kit_tail(&self) -> Vec<K> {
        let b = |x: B| K::S(Sel::Builtin(x));
        let mut v = vec![b(B::Level), K::S(Sel::Morph)];
        match self.patch.kind {
            Kind::Instrument if self.patch.amp_env => v.extend([b(B::Attack), b(B::Release), b(B::Glide), b(B::Transpose), b(B::Scale)]),
            Kind::Instrument => v.extend([b(B::Velocity), b(B::Glide), b(B::Transpose), b(B::Root), b(B::Scale)]),
            Kind::Effect => v.extend([b(B::DryWet), b(B::InputGain), b(B::Tempo), b(B::Transpose), b(B::Scale)]),
            Kind::Generator => v.extend([b(B::Tempo), b(B::Transpose), b(B::Root), b(B::Scale), b(B::Hold)]),
        }
        v
    }

    /// Continuous built-ins that fill the knobs when a patch has fewer
    /// than eight macros + params, so the hero pairs never land on a
    /// stepped choice that expression can't push.
    fn kit_spare(&self) -> Vec<K> {
        let b = |x: B| K::S(Sel::Builtin(x));
        match self.patch.kind {
            Kind::Instrument if self.patch.amp_env => vec![b(B::Attack), b(B::Release), b(B::Decay), b(B::Sustain), b(B::Glide), b(B::Velocity)],
            Kind::Instrument => vec![b(B::Glide), b(B::Velocity)],
            Kind::Effect => vec![b(B::DryWet), b(B::InputGain)],
            Kind::Generator => vec![b(B::Tempo)],
        }
    }

    /// The 16 play-view controls for the current patch, most important
    /// first: its macros, then its continuous params, then stepped ones
    /// (knobs 0-7); then the built-ins for its kind; the library browser
    /// last. Always exactly 16 (Morph + 15 built-ins are 16 distinct
    /// fillers), so hero/browse indexes stay valid for any patch.
    fn kit_controls(&self) -> Vec<K> {
        fn add(v: &mut Vec<K>, k: K) {
            if !v.contains(&k) {
                v.push(k);
            }
        }
        let mut v: Vec<K> = Vec::with_capacity(16);
        for k in 0..self.patch.macros.len().min(MAX_MACROS) {
            add(&mut v, K::S(Sel::Macro(k)));
        }
        let n = self.maps.len().min(self.patch.params.len()).min(MAX_PARAMS);
        let continuous = (0..n).filter(|&i| self.maps[i].steps().is_none());
        let stepped = (0..n).filter(|&i| self.maps[i].steps().is_some());
        for i in continuous.chain(stepped) {
            if v.len() >= 8 {
                break;
            }
            add(&mut v, K::S(Sel::Param(i)));
        }
        v.truncate(8);
        for k in self.kit_spare() {
            if v.len() >= 8 {
                break;
            }
            add(&mut v, k);
        }
        for k in self.kit_tail() {
            add(&mut v, k);
        }
        for b in ALL_B {
            if v.len() >= C_PATCH {
                break;
            }
            add(&mut v, K::S(Sel::Builtin(b)));
        }
        add(&mut v, K::S(Sel::Morph));
        v.truncate(C_PATCH);
        v.push(K::Patch);
        v
    }

    fn kit_control(&self, i: usize) -> Option<K> {
        self.kit_controls().get(i).copied()
    }

    /// A built-in's range as `edit` clamps it (Voices tops out at what
    /// the patch allocated).
    fn builtin_range(&self, b: B) -> (f32, f32) {
        let s = bspec(b);
        (s.min, if b == B::Voices { self.patch.voices as f32 } else { s.max })
    }

    /// Exponential built-ins (attack, decay, release) sit on the dial
    /// logarithmically, the way `edit` turns them.
    fn builtin_norm(&self, b: B) -> f32 {
        let spec = bspec(b);
        let (lo, hi) = self.builtin_range(b);
        let v = self.shared.b(b);
        if hi <= lo {
            0.0
        } else if spec.exp && lo > 0.0 {
            ((v.max(lo) / lo).ln() / (hi / lo).ln()).clamp(0.0, 1.0)
        } else {
            ((v - lo) / (hi - lo)).clamp(0.0, 1.0)
        }
    }

    fn set_builtin_norm(&mut self, b: B, n: f32) {
        let spec = bspec(b);
        let (lo, hi) = self.builtin_range(b);
        if hi <= lo {
            return;
        }
        let n = n.clamp(0.0, 1.0);
        let mut v = if spec.exp && lo > 0.0 { lo * (hi / lo).powf(n) } else { lo + n * (hi - lo) };
        if spec.step > 0.0 {
            v = lo + ((v - lo) / spec.step).round() * spec.step;
        }
        let v = v.clamp(lo, hi);
        self.shared.set_b(b, v);
        if b == B::Hold && v < 0.5 {
            self.latched = [false; 16];
        }
    }
}

impl PlayHost for OracleApp {
    fn kit_control_count(&self) -> usize {
        self.kit_controls().len()
    }
    fn kit_label(&self, i: usize) -> String {
        match self.kit_control(i) {
            Some(K::S(sel)) => {
                let l = self.leaf_name(sel);
                if l.is_empty() { format!("Control {}", i + 1) } else { l }
            }
            Some(K::Patch) => "Patch".into(),
            None => String::new(),
        }
    }
    fn kit_value(&self, i: usize) -> String {
        match self.kit_control(i) {
            Some(K::S(sel)) => self.leaf_value(sel),
            Some(K::Patch) => fit(&self.patch.name, 24),
            None => String::new(),
        }
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        match self.kit_control(i)? {
            K::S(Sel::Param(p)) => (p < self.maps.len()).then(|| self.shared.norms[p].get()),
            K::S(Sel::Macro(k)) => Some(self.shared.macros[k].get()),
            K::S(Sel::Builtin(b)) => Some(self.builtin_norm(b)),
            K::S(Sel::Morph) => Some(self.shared.morph.get()),
            _ => None,
        }
    }
    fn kit_stepped(&self, i: usize) -> bool {
        match self.kit_control(i) {
            Some(K::S(Sel::Param(p))) => self.maps.get(p).map_or(true, |m| m.steps().is_some()),
            Some(K::S(Sel::Builtin(b))) => bspec(b).step > 0.0,
            Some(K::S(Sel::Macro(_) | Sel::Morph)) => false,
            _ => true,
        }
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match self.kit_control(i) {
            Some(K::S(sel)) => self.edit(sel, delta),
            Some(K::Patch) => self.step_patch(delta),
            None => {}
        }
    }
    fn kit_reset(&mut self, i: usize) {
        // `press` resets these four; on any other row it runs an action,
        // which a knob-2 reset must never do.
        if let Some(K::S(sel @ (Sel::Param(_) | Sel::Macro(_) | Sel::Builtin(_) | Sel::Morph))) = self.kit_control(i) {
            self.press(sel);
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match self.kit_control(i) {
            Some(K::S(Sel::Param(p))) => {
                if let Some(map) = self.maps.get(p) {
                    let v = match map.steps() {
                        Some(n) => {
                            let k = n.saturating_sub(1).max(1) as f32;
                            (v * k).round() / k
                        }
                        None => v,
                    };
                    // While morphing the engine reads the A/B snapshots,
                    // not these; the macros (first on the knobs) still
                    // apply, which is why they come first.
                    self.shared.norms[p].set(v);
                }
            }
            Some(K::S(Sel::Macro(k))) => self.shared.macros[k].set(v),
            Some(K::S(Sel::Builtin(b))) => self.set_builtin_norm(b, v),
            Some(K::S(Sel::Morph)) => self.shared.morph.set(v),
            _ => {}
        }
    }
    /// A moment only means something on the patch it was stored on (the
    /// controls are that patch's macros and params), so it carries the
    /// patch name and won't land on a different patch.
    fn kit_snapshot(&self) -> serde_json::Value {
        let v: Vec<serde_json::Value> = (0..self.kit_control_count()).map(|i| self.kit_norm(i).map_or(serde_json::Value::Null, |x| serde_json::json!(x))).collect();
        serde_json::json!({ "patch": self.patch.name, "v": v })
    }
    fn kit_recall(&mut self, m: &serde_json::Value) {
        let stored = m.get("patch").and_then(|p| p.as_str()).unwrap_or("");
        if stored != self.patch.name {
            self.set_status(format!("That moment belongs to \"{}\" -- load it first (D-pad)", fit(stored, 24)));
            return;
        }
        if let Some(items) = m.get("v").and_then(|v| v.as_array()) {
            for (i, item) in items.iter().enumerate().take(self.kit_control_count()) {
                if let Some(x) = item.as_f64() {
                    self.kit_set_norm(i, (x as f32).clamp(0.0, 1.0));
                }
            }
        }
        // Like recalling a snapshot: the recalled knobs are what plays.
        self.shared.morph_on.store(false, Ordering::Relaxed);
    }
    fn kit_line(&self) -> String {
        if let Some(t) = self.voice.as_ref() {
            return if t.recording() { format!("Listening {:.0}s", t.elapsed()) } else { "Transcribing...".into() };
        }
        if self.busy_since.is_some() {
            return format!("{}...", self.worker.busy.map(|b| b.1).unwrap_or("Thinking"));
        }
        if self.snap_mode {
            return match (self.shared.morph_on.load(Ordering::Relaxed), self.snap_a, self.snap_b) {
                (true, Some(a), Some(b)) => format!("Morph {}>{} {:.0}%", a + 1, b + 1, self.shared.morph.get() * 100.0),
                _ => format!("{} snapshots", self.snaps.iter().filter(|s| s.is_some()).count()),
            };
        }
        let (root, scale, tr) = self.pad_tuning();
        let notes: Vec<String> = (0..16).filter(|&i| self.shared.held[i].load(Ordering::Relaxed)).take(4).map(|i| note_name(pad_note(i, root, scale, tr))).collect();
        if notes.is_empty() { fit(&self.patch.name, 18) } else { notes.join(" ") }
    }
    fn kit_pad_label(&self, layer: u8, pad: usize) -> String {
        if layer == 1 {
            return if Some(pad) == self.snap_a {
                "A".into()
            } else if Some(pad) == self.snap_b {
                "B".into()
            } else if self.snaps[pad].is_some() {
                format!("{} snap", pad + 1)
            } else {
                format!("{} -", pad + 1)
            };
        }
        let (root, scale, tr) = self.pad_tuning();
        note_name(pad_note(pad, root, scale, tr))
    }
    /// The colours the old LED overlay showed, per pad mode, plus the
    /// held pad on PLAY.
    fn kit_pad_color(&self, layer: u8, pad: usize, held: bool) -> PadColor {
        if layer == 1 {
            if Some(pad) == self.snap_a {
                PadColor::Yellow
            } else if Some(pad) == self.snap_b {
                PadColor::Blue
            } else if self.snaps[pad].is_some() {
                PadColor::Green
            } else {
                PadColor::Off
            }
        } else if self.latched[pad] || held {
            PadColor::Green
        } else {
            PadColor::Off
        }
    }
    /// On PLAY a key presses the pad that sounds its pitch (pads follow
    /// the scale, so keys outside it fold to the nearest pad of the same
    /// pitch class, or are ignored if the scale doesn't have it). On SNAP
    /// keys pick slots the way the shell always mapped them (note % 16).
    fn kit_midi_pad(&self, note: u8) -> Option<usize> {
        // Controllers send encoder-touch notes below 21; never pads.
        if note < 21 {
            return None;
        }
        if self.snap_mode {
            return Some(note as usize % 16);
        }
        let (root, scale, tr) = self.pad_tuning();
        let n = note as i32;
        (0..16).find(|&p| pad_note(p, root, scale, tr) == n).or_else(|| {
            (0..16).filter(|&p| (pad_note(p, root, scale, tr) - n).rem_euclid(12) == 0).min_by_key(|&p| (pad_note(p, root, scale, tr) - n).abs())
        })
    }
}

struct GraphBox {
    x: i32,
    y: i32,
    w: i32,
    label: String,
    activity: f32,
    voice: bool,
}

struct GraphLayout {
    boxes: Vec<GraphBox>,
    box_h: i32,
    /// (from, to, forward) -- feedback edges point upward.
    edges: Vec<(Point, Point, bool)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Carry {
    Fresh,
    KeepMatching,
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let word: String = word.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).collect();
        if line.len() + word.len() + 1 > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word.chars().take(width).collect::<String>());
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn fit(s: &str, max: usize) -> String {
    let clean: String = s.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).collect();
    if clean.chars().count() <= max {
        clean
    } else {
        let mut t: String = clean.chars().take(max.saturating_sub(1)).collect();
        t.push('~');
        t
    }
}

impl App for OracleApp {
    fn tick(&mut self, input: &Input) {
        // The play view takes the knobs and D-pad first; in the menu they
        // pass straight through. Pads reach handle_pads only on PLAY/SNAP.
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let input = &step.input;
        if let Some(id) = step.native {
            if (id == 1) != self.snap_mode {
                self.set_snap_mode(id == 1);
            }
        }
        self.frame = self.frame.wrapping_add(1);
        self.handle_pads(&input.grid);

        let rows = self.visible_rows();
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();
        if input.knob1_press {
            match current {
                Some(Row::Group(g)) => self.expanded[g] = !self.expanded[g],
                Some(Row::Leaf(Sel::Param(i))) => {
                    self.locks[i] = !self.locks[i];
                    let name = self.patch.params.get(i).map(|p| p.name.clone()).unwrap_or_default();
                    self.set_status(format!("{} {}", name, if self.locks[i] { "locked" } else { "unlocked" }));
                }
                _ => {}
            }
        }
        if let Some(Row::Leaf(sel)) = current {
            self.edit(sel, input.knob2);
            if input.knob2_press {
                self.press(sel);
            }
        }
    }

    fn on_exit(&mut self) {
        // Leaving with Hold on keeps a drone running; otherwise release.
        if self.shared.b(B::Hold) < 0.5 {
            for h in &self.shared.held {
                h.store(false, Ordering::Relaxed);
            }
        }
        self.prev_grid = [false; 16];
    }

    fn background_tick(&mut self) {
        self.poll_voice();
        self.poll_worker();
        self.poll_stdin();
        if let Ok(mut slot) = self.shared.retired.try_lock() {
            slot.take(); // free the faded-out engine here, off the audio thread
        }
    }

    /// Only a generator has a transport: it plays on its own, so it has
    /// to keep running off-screen. Instruments and effects behave like
    /// Plaits/Tonestack -- they sound while played, patched or audible
    /// (see `needs_background_audio`), and need no Play/Stop.
    fn running(&self) -> Option<bool> {
        (self.patch.kind == Kind::Generator).then(|| self.shared.playing.load(Ordering::Relaxed))
    }

    fn supports_pad_lock(&self) -> bool {
        self.patch.kind == Kind::Instrument
    }

    fn play_surface(&self) -> bool {
        true
    }

    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (!self.kit.menu).then(|| self.kit.column(self))
    }

    /// Keep DSP (and `background_tick`) alive while something is still
    /// happening: notes ringing out or held by Hold, an effect with a
    /// source patched in, an AI request in flight, or a voice take.
    fn needs_background_audio(&self) -> bool {
        self.shared.active_voices.load(Ordering::Relaxed) > 0
            || (self.patch.kind == Kind::Effect && self.shared.source.load(Ordering::Relaxed) != NO_SOURCE)
            || self.worker.busy.is_some()
            || self.voice.is_some()
    }

    fn toggle_running(&mut self) {
        let p = !self.shared.playing.load(Ordering::Relaxed);
        self.shared.playing.store(p, Ordering::Relaxed);
    }

    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }

    /// F2 cycles PLAY, SNAP, Controls, Moments. Landing on PLAY/SNAP
    /// switches Oracle's own pad mode right away (not a frame later in
    /// tick) so a MIDI key that same frame already maps the new way.
    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
        if let Layer::Native(id, _) = self.kit.layer() {
            if (id == 1) != self.snap_mode {
                self.set_snap_mode(id == 1);
            }
        }
    }

    fn grid_led_overlay(&self) -> [PadColor; 16] {
        self.kit.led_overlay(self)
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(OracleProcessor {
            shared: Arc::clone(&self.shared),
            audio_bus: Arc::clone(&self.audio_bus),
            current: None,
            fading: None,
            fade: 0,
            input: Vec::with_capacity(4096),
            l: Vec::with_capacity(4096),
            r: Vec::with_capacity(4096),
            fl: Vec::with_capacity(4096),
            fr: Vec::with_capacity(4096),
            mono: Vec::with_capacity(4096),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let title = MonoTextStyle::new(&SPLEEN_16X32, Rgb565::WHITE);
        Text::new(APP_NAME, Point::new(16, 30), title).draw(fb).ok();
        let accent = MonoTextStyle::new(&SPLEEN_6X12, ACCENT);
        let accent_big = MonoTextStyle::new(&SPLEEN_8X16, ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));
        let warn = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(31, 20, 0));

        let name = fit(&self.patch.name, 26);
        Text::new(&name, Point::new(124, 27), accent_big).draw(fb).ok();
        let kx = 124 + name.len() as i32 * 8 + 8;
        Text::new(self.patch.kind.label(), Point::new(kx, 27), dim).draw(fb).ok();

        // Left: menu. Keep rows clear of the right panel (x 400).
        let rows: Vec<(String, String)> = self
            .display_rows()
            .into_iter()
            .map(|(n, v, g)| {
                let n = if g { n } else { format!("    {n}") };
                let room = 44usize.saturating_sub(n.chars().count().min(30) + 4);
                (fit(&n, 30), fit(&v, room.max(6)))
            })
            .collect();
        if self.kit.menu {
            self.list.draw(fb, 16, 68, 24, 10, &rows);
        } else if let Some(col) = self.play_column() {
            // Oracle's green-on-black, kept clear of the right panel (x 400).
            let pal = kit::draw::Palette { bg: Rgb565::BLACK, ink: Rgb565::new(20, 44, 22), accent: ACCENT, dim: Rgb565::new(18, 36, 18), faint: Rgb565::new(3, 6, 3) };
            kit::draw::column(fb, &col, 16, 40, 370, 284, pal);
        }

        // Right panel.
        let (px, py, pw) = (400, 44, 220);
        let selected = self.visible_rows().get(self.list.selected).copied();
        if let Some(take) = self.voice.as_ref().filter(|t| t.recording()) {
            Text::new("Listening", Point::new(px, py + 4), accent_big).draw(fb).ok();
            Text::new(&format!("{:.0}s / {:.0}s -- press again to stop", take.elapsed(), voice::MAX_SECONDS), Point::new(px, py + 24), dim).draw(fb).ok();
            // live input level as a row of bars
            let lvl = take.level();
            for k in 0..20 {
                let on = (k as f32) < lvl * 20.0;
                let h = 10 + ((k as f32 * 0.9 + self.frame as f32 * 0.3).sin().abs() * 30.0 * if on { 1.0 } else { 0.2 }) as i32;
                Rectangle::new(Point::new(px + 10 + k * 10, py + 110 - h / 2), Size::new(6, h as u32))
                    .into_styled(PrimitiveStyle::with_fill(if on { ACCENT } else { Rgb565::new(3, 8, 4) }))
                    .draw(fb)
                    .ok();
            }
        } else if let Some(take) = self.voice.as_ref().filter(|t| t.transcribing()) {
            let _ = take;
            let dots = ".".repeat((self.frame / 15 % 4) as usize);
            Text::new(&format!("Transcribing{dots}"), Point::new(px, py + 4), accent_big).draw(fb).ok();
        } else if let Some(since) = self.busy_since {
            let dots = ".".repeat((self.frame / 15 % 4) as usize);
            let label = self.worker.busy.map(|b| b.1).unwrap_or("Thinking");
            Text::new(&format!("{label}{dots}"), Point::new(px, py + 4), accent_big).draw(fb).ok();
            Text::new(&format!("{:.0}s -- the Oracle is designing", since.elapsed().as_secs_f32()), Point::new(px, py + 24), dim)
                .draw(fb)
                .ok();
            // orbiting glyph
            let t = self.frame as f32 * 0.08;
            for k in 0..6 {
                let a = t + k as f32 * 1.047;
                let (cx, cy) = (px + 110 + (a.cos() * 50.0) as i32, py + 110 + (a.sin() * 50.0) as i32);
                Rectangle::new(Point::new(cx - 3, cy - 3), Size::new(6, 6))
                    .into_styled(PrimitiveStyle::with_fill(Rgb565::new(4, (20 + k * 7) as u8, 10)))
                    .draw(fb)
                    .ok();
            }
        } else if self.snap_mode {
            Text::new("Snapshots", Point::new(px, py - 4), accent).draw(fb).ok();
            self.draw_snap_grid(fb, px, py + 2);
        } else if matches!(selected, Some(Row::Leaf(Sel::Explain)) | Some(Row::Leaf(Sel::AiInfo))) && self.explain.is_some() {
            Text::new("The Oracle says", Point::new(px, py - 4), accent).draw(fb).ok();
            for (k, l) in wrap(self.explain.as_deref().unwrap_or(""), 36).iter().take(21).enumerate() {
                Text::new(l, Point::new(px, py + 12 + k as i32 * 13), dim).draw(fb).ok();
            }
        } else {
            Text::new("Signal graph", Point::new(px, py - 4), accent).draw(fb).ok();
            self.draw_graph(fb, px, py + 2, pw, 186);
            self.draw_scope(fb, px, py + 196, pw, 30);
        }

        // Info + status (right, under the panel).
        let voices = self.shared.active_voices.load(Ordering::Relaxed);
        let info = format!(
            "{}  load {:.0}%  {}",
            if self.patch.kind == Kind::Instrument { format!("{voices}/{} voices", self.shared.b(B::Voices) as usize) } else { self.patch.kind.label().into() },
            self.shared.load.get() * 100.0,
            if self.shared.playing.load(Ordering::Relaxed) { "" } else { "stopped" }
        );
        Text::new(&info, Point::new(px, py + 244), dim).draw(fb).ok();
        if self.shared.unstable.load(Ordering::Relaxed) {
            Text::new("! a node blew up and was reset", Point::new(px, py + 258), warn).draw(fb).ok();
        }
        for (k, l) in wrap(&self.status, 36).iter().take(3).enumerate() {
            Text::new(l, Point::new(px, py + 272 + k as i32 * 12), accent).draw(fb).ok();
        }

        let hint = match selected {
            _ if !self.kit.menu => "knobs: macros   D-pad: patch   F2: pads   F3: play/stop   R1: menu",
            Some(Row::Group(_)) => "knob1: browse   press knob1: open/close   F3: play/stop",
            Some(Row::Leaf(Sel::Param(_))) => "knob2: edit   press knob2: reset   press knob1: lock",
            Some(Row::Leaf(Sel::SpeakNew | Sel::SpeakChange)) => "press knob2: talk   press again: stop (a pause stops too)",
            Some(Row::Leaf(s)) if self.leaf_name(s).starts_with(">>") => "press knob2 to run",
            _ => "knob2: change   press knob2: reset",
        };
        Text::new(hint, Point::new(16, 337), dim).draw(fb).ok();
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        self.windowed_rows(visible)
    }
    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        crate::app::SlintExtra::Oracle(self.panel_extra())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> (OracleApp, Arc<ModBus>, Arc<AudioBus>, Arc<MixerBus>) {
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let app = OracleApp::new(
            Arc::new(AtomicF32::new(0.1)),
            Arc::new(AtomicF32::new(1.0)),
            Arc::clone(&modbus),
            Arc::clone(&audio_bus),
            Arc::clone(&mixer_bus),
        );
        (app, modbus, audio_bus, mixer_bus)
    }

    fn render(p: &mut dyn AudioProcessor, blocks: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let mut buf = vec![0.0f32; 512 * 2];
        for _ in 0..blocks {
            p.process(&mut buf, 2, 48_000.0);
            out.extend_from_slice(&buf);
        }
        out
    }

    #[test]
    fn plays_the_default_patch_from_the_pads() {
        let (mut app, ..) = make();
        let mut proc = app.audio_processor().unwrap();
        assert!(render(proc.as_mut(), 10).iter().all(|s| s.abs() < 1e-4), "silent before any pad");
        let mut input = Input::default();
        input.grid[12] = true;
        app.tick(&input);
        let out = render(proc.as_mut(), 60);
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(peak > 0.02 && peak < 1.0, "peak {peak}");
    }

    #[test]
    fn registers_on_every_bus() {
        let (_app, modbus, audio_bus, mixer_bus) = make();
        assert!(audio_bus.names().iter().any(|n| n == APP_NAME));
        assert!(mixer_bus.names().iter().any(|n| n == APP_NAME));
        let names = modbus.names();
        assert!(names.iter().any(|n| n == "Oracle: Macro 1"));
        assert!(names.iter().any(|n| n == "Oracle: Morph"));
        assert!(names.iter().any(|n| n == "Oracle: P16"));
    }

    #[test]
    fn loading_every_library_patch_swaps_engines_cleanly() {
        let (mut app, ..) = make();
        let mut proc = app.audio_processor().unwrap();
        render(proc.as_mut(), 4);
        let n = app.library.len();
        assert!(n >= 10);
        for i in 0..n {
            app.load_library(i);
            let mut input = Input::default();
            input.grid[12] = true;
            app.tick(&input);
            let out = render(proc.as_mut(), 30);
            assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.5), "{}", app.patch.name);
            app.background_tick(); // frees retired engines
        }
        assert_eq!(app.history.len(), n.min(HISTORY_MAX));
    }

    #[test]
    fn snapshots_store_recall_and_morph() {
        let (mut app, ..) = make();
        app.toggle_grid_mode();
        let tap = |app: &mut OracleApp, pads: &[usize]| {
            let mut i = Input::default();
            for &p in pads {
                i.grid[p] = true;
            }
            app.tick(&i);
        };
        app.shared.norms[0].set(0.1);
        tap(&mut app, &[0]);
        tap(&mut app, &[]);
        assert!(app.snaps[0].is_some(), "tap on empty pad stores");
        app.shared.norms[0].set(0.9);
        tap(&mut app, &[1]);
        tap(&mut app, &[]);
        tap(&mut app, &[0]);
        tap(&mut app, &[]);
        assert!((app.shared.norms[0].get() - 0.1).abs() < 1e-6, "tap on stored pad recalls");
        tap(&mut app, &[0]);
        tap(&mut app, &[0, 1]);
        tap(&mut app, &[]);
        assert!(app.shared.morph_on.load(Ordering::Relaxed));
        app.shared.morph.set(1.0);
        let c = app.shared.controls();
        assert!((c.norms[0] - 0.9).abs() < 1e-6, "morph fully to B");
        assert!(app.snaps[0].is_some() && app.snaps[1].is_some(), "morphing doesn't overwrite");
    }

    #[test]
    fn locks_survive_randomize_and_mutate_fills_variants() {
        let (mut app, ..) = make();
        app.locks[0] = true;
        let before = app.shared.norms[0].get();
        app.randomize(1.0);
        assert_eq!(app.shared.norms[0].get(), before);
        app.mutate();
        assert!(app.snaps[12..16].iter().all(|s| s.is_some()));
        assert!(app.snap_mode);
    }

    #[test]
    fn undo_redo_walks_history() {
        let (mut app, ..) = make();
        let first = app.patch.name.clone();
        app.load_library(1);
        let second = app.patch.name.clone();
        assert_ne!(first, second);
        app.step_history(-1);
        assert_eq!(app.patch.name, first);
        app.step_history(1);
        assert_eq!(app.patch.name, second);
    }

    #[test]
    fn pasted_json_in_the_terminal_installs_a_patch() {
        let (mut app, ..) = make();
        app.command(r#"{"name":"Pasted Tone","kind":"generator","#);
        assert!(app.paste.is_some());
        app.command(r#""global":[{"id":"o","type":"osc","in":{"freq":330}}],"out":"o*0.3"}"#);
        assert!(app.paste.is_none());
        assert_eq!(app.patch.name, "Pasted Tone");
    }

    #[test]
    fn every_row_edits_presses_and_draws_without_panicking() {
        let (mut app, ..) = make();
        app.expanded = [true; NUM_GROUPS];
        let rows = app.visible_rows();
        for row in rows.iter() {
            if let Row::Leaf(sel) = row {
                // skip rows that hit the network or the filesystem
                if matches!(sel, Sel::AskGo | Sel::RefineGo | Sel::Vary | Sel::Explain | Sel::AiBreed | Sel::Save | Sel::Lib(_)) {
                    continue;
                }
                for d in [1, -1, 7, -7] {
                    app.edit(*sel, d);
                }
                app.press(*sel);
            }
        }
        for snap in [false, true] {
            app.snap_mode = snap;
            let mut fb = FrameBuffer::new();
            app.draw(&mut fb);
            assert!(fb.buffer().iter().any(|&p| p != 0));
        }
        app.explain = Some("A warm pad. ".repeat(40));
        app.list.selected = 9; // Explain row area
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }

    #[test]
    fn prompt_builder_composes_by_kind() {
        assert!(compose_prompt(0, 0, 0, 0).contains("pad instrument"));
        assert!(compose_prompt(KINDS.len() - 2, 0, 0, 0).contains("kind: effect"));
        assert!(compose_prompt(KINDS.len() - 1, 0, 0, 0).contains("kind: generator"));
    }

    #[test]
    fn opens_on_the_play_view_knob1_turns_the_first_control_pads_play_and_r1_opens_the_menu() {
        let (mut app, ..) = make();
        assert!(app.play_column().is_some(), "play view first");
        assert_eq!(app.kit_control_count(), 16);
        let before = app.kit_norm(0).expect("knob 1 has a position");
        app.tick(&Input { knob1: if before > 0.5 { -5 } else { 5 }, ..Default::default() });
        assert!((app.kit_norm(0).unwrap() - before).abs() > 1e-4, "knob 1 turns control 0 ({})", app.kit_label(0));
        let mut input = Input::default();
        input.grid[12] = true;
        app.tick(&input);
        assert!(app.shared.held[12].load(Ordering::Relaxed), "PLAY layer plays the pads");
        app.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(app.play_column().is_none(), "R1 opens the full menu");
    }

    #[test]
    fn f2_layers_keep_play_and_snap_in_step_and_kit_layers_keep_the_pads() {
        let (mut app, ..) = make();
        let tap = |app: &mut OracleApp, pad: usize| {
            let mut i = Input::default();
            i.grid[pad] = true;
            app.tick(&i);
            app.tick(&Input::default());
        };
        app.toggle_grid_mode();
        assert!(app.snap_mode && app.kit.layer_label() == "SNAP");
        tap(&mut app, 0);
        assert!(app.snaps[0].is_some(), "SNAP stores like the old Snap mode");
        app.toggle_grid_mode(); // Controls
        tap(&mut app, 5);
        assert!(app.snaps[5].is_none(), "kit layers don't reach the snapshot pads");
        app.toggle_grid_mode(); // Moments
        app.toggle_grid_mode(); // back to PLAY
        assert!(!app.snap_mode && app.kit.layer_label() == "PLAY");
        app.press(Sel::PadMode);
        assert!(app.snap_mode && app.kit.layer_label() == "SNAP", "the menu's Pads row moves the layer too");
        app.mutate();
        assert_eq!(app.kit.layer_label(), "SNAP");
    }

    #[test]
    fn d_pad_browses_the_library_and_moments_stay_with_their_patch() {
        let (mut app, ..) = make();
        let first = app.patch.name.clone();
        app.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_ne!(app.patch.name, first, "D-pad up loads the next library patch");
        let moment = app.kit_snapshot();
        let before = app.kit_norm(0);
        app.kit_set_norm(0, if before.unwrap_or(0.0) > 0.5 { 0.0 } else { 1.0 });
        app.kit_recall(&moment);
        assert_eq!(app.kit_norm(0), before, "recalls on its own patch");
        app.step_patch(1);
        let other = app.kit_norm(0);
        app.kit_recall(&moment);
        assert_eq!(app.kit_norm(0), other, "never lands on a different patch");
    }

    #[test]
    fn a_midi_key_presses_the_pad_with_its_pitch() {
        let (mut app, ..) = make();
        let (root, scale, tr) = app.pad_tuning();
        let mut keys = crate::app::MidiKeys::default();
        keys.0[pad_note(12, root, scale, tr) as usize] = 100;
        app.tick(&Input { midi_keys: keys, ..Default::default() });
        assert!(app.shared.held[12].load(Ordering::Relaxed));
    }

    #[test]
    fn text_helpers_are_ascii_and_bounded() {
        assert!(wrap("one two three four five six", 9).iter().all(|l| l.len() <= 9));
        assert_eq!(fit("A very long patch name indeed", 10).chars().count(), 10);
        assert_eq!(fit("naïve", 10), "nave");
    }
}

