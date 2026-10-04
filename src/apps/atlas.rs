//! Atlas -- the meta-synth. A curated library of sound designs built on
//! the shared synthesis platform (src/synthesis): each preset is a patch --
//! a graph of generators (wavetable, supersaw, FM, Plaits, modal, granular,
//! sampler, noise...) feeding filters, shapers and effects, with
//! expressions wiring them into each other -- that Atlas plays with the
//! same eight performance controls every time:
//!
//!   CHARACTER  COLOR  MOTION  SPACE  SHAPE  ENERGY  TEXTURE  MORPH
//!
//! The first seven are the patch's macros (each moves several parameters
//! by its own amounts); MORPH glides through the patch's stored states,
//! A -> B -> C -> D, every parameter at once. Same knobs, any sound: the
//! player learns one surface, the presets supply the meaning.
//!
//! Progressive depth (the "Depth" row): Play shows the performance
//! controls and the basics; Edit adds every parameter by page, the
//! envelope, storing states, mutation and saving; Deep adds the compiled
//! graph (each block with its live activity) and the CPU budget.
//!
//! Real-time design is Oracle's (both use the same compiled `Engine`):
//! presets compile on the UI thread and are handed to the audio thread
//! through a try-locked slot; the old engine crossfades out and comes back
//! to be freed off the audio thread. An estimated cost per voice (see
//! `synth::blocks::cost`) caps polyphony to the CPU budget, so an
//! expensive patch plays fewer notes rather than glitching.
//!
//! Presets: the factory set lives in assets/atlas/presets (compiled in);
//! saved ones are plain JSON in saves/atlas/presets, shareable and
//! hand-editable (the format is docs/SYNTH_PLATFORM.md).

use crate::app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes};
use crate::app::{App, AtlasExtra, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::spleen_fonts::{SPLEEN_6X12, SPLEEN_8X16};
use crate::synthesis::engine::{compile, pad_note, Controls, Engine, GraphNode, SCALES};
use crate::synthesis::evolve::{self, Rng};
use crate::synthesis::patch::{format_value, MacroSpec, ParamMap, Patch, State, CURRENT_VERSION, MAX_MACROS, MAX_PARAMS, MAX_STATES};
use crate::util::{note_name, AtomicF32};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const APP_NAME: &str = "Atlas";

/// The seven macro slots, in knob order. A preset's macros are matched to
/// these by name, so CHARACTER is always the same knob whatever the sound.
pub const MACRO_NAMES: [&str; 7] = ["CHARACTER", "COLOR", "MOTION", "SPACE", "SHAPE", "ENERGY", "TEXTURE"];
const N_MACROS: usize = MACRO_NAMES.len();
const STATE_NAMES: [&str; MAX_STATES] = ["A", "B", "C", "D"];
pub const DEPTHS: [&str; 3] = ["Play", "Edit", "Deep"];

/// Factory presets, compiled in.
pub const FACTORY: &[(&str, &str)] = &[
    ("evolving_glass", include_str!("../../assets/atlas/presets/evolving_glass.json")),
    ("init", include_str!("../../assets/atlas/presets/init.json")),
    ("sub_pressure", include_str!("../../assets/atlas/presets/sub_pressure.json")),
    ("hum_choir", include_str!("../../assets/atlas/presets/hum_choir.json")),
    ("modal_garden", include_str!("../../assets/atlas/presets/modal_garden.json")),
    ("ghost_strings", include_str!("../../assets/atlas/presets/ghost_strings.json")),
    ("dust_drone", include_str!("../../assets/atlas/presets/dust_drone.json")),
];

/// Default CPU budget, in units of one plain oscillator voice-sample (see
/// `synth::blocks::cost`). Generous enough that every factory preset gets
/// its designed polyphony on the simulator; the Deep page lowers it to
/// model a slower target.
const DEFAULT_BUDGET: f32 = 400.0;
const FADE_SAMPLES: usize = 2048;
/// Samples kept for the scope / spectrum.
const WAVE_LEN: usize = 1024;
const SPECTRUM_BANDS: usize = 24;
#[allow(dead_code)] // Slint GUI only
const SCOPE_POINTS: usize = 128;

// Own palette: chart ink on midnight blue, brass accents.
const BG: Rgb565 = Rgb565::new(2, 5, 9);
const INK: Rgb565 = Rgb565::new(27, 56, 30);
const ACCENT: Rgb565 = Rgb565::new(30, 45, 10);
const DIM: Rgb565 = Rgb565::new(11, 24, 18);
const FAINT: Rgb565 = Rgb565::new(5, 10, 13);

// ------------------------------------------------------------- controls

const C_MORPH: usize = 7;
const C_PRESET: usize = 8;
const C_LEVEL: usize = 9;
const C_VOICES: usize = 10;
const C_DEPTH: usize = 11;
/// ENERGY: what pad pressure pushes until the player binds it elsewhere.
const C_ENERGY: usize = 5;
/// The native pad layer where the bottom row of pads recalls the states.
const STATES_LAYER: u8 = 1;
/// How much of the remaining distance MORPH covers each frame (~60 Hz)
/// when a state pad glides to its state: about a second to settle.
const MORPH_GLIDE: f32 = 0.06;
const N_CONTROLS: usize = 12;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "atlas",
        layers: vec![Layer::Native(0, "PLAY"), Layer::Native(STATES_LAYER, "STATES"), Layer::Controls, Layer::Moments],
        // CHARACTER/COLOR, MOTION/TEXTURE, SPACE/SHAPE, ENERGY/MORPH
        hero: vec![[0, 1], [2, 6], [3, 4], [5, C_MORPH]],
        browse: Some(C_PRESET),
        // the stick morphs and colours; the depth sensors play MOTION and SPACE
        routes: Routes { stick_x: Some(C_MORPH), stick_y: Some(1), hand_l: Some(2), hand_r: Some(3) },
        throws: Vec::new(),
        midi_to_pads: true,
        own_expression: false,
    }
}

// ------------------------------------------------------------- library

#[derive(Clone, Debug)]
pub enum Source {
    Factory(usize),
    File(PathBuf),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub category: String,
    pub source: Source,
}

pub fn user_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves/atlas/presets"))
}

/// Factory presets first, then saved ones (by name). Unreadable files are
/// skipped rather than failing the whole library.
pub fn scan() -> Vec<Entry> {
    let mut out: Vec<Entry> = FACTORY
        .iter()
        .enumerate()
        .filter_map(|(i, (_, json))| {
            let p = Patch::from_json(json).ok()?;
            Some(Entry { name: p.name, category: p.category, source: Source::Factory(i) })
        })
        .collect();
    let mut files: Vec<Entry> = std::fs::read_dir(user_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| {
            let p = Patch::from_json(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            Some(Entry { name: p.name, category: p.category, source: Source::File(e.path()) })
        })
        .collect();
    files.sort_by_key(|e| e.name.to_lowercase());
    out.extend(files);
    out
}

pub fn load_entry(e: &Entry) -> Result<Patch, String> {
    let text = match &e.source {
        Source::Factory(i) => FACTORY.get(*i).map(|f| f.1.to_string()).ok_or("missing preset")?,
        Source::File(p) => std::fs::read_to_string(p).map_err(|err| format!("{}: {err}", p.display()))?,
    };
    Patch::from_json(&text)
}

/// Reorder a patch's macros into the seven named slots. Macros under
/// other names have no knob; they're dropped and reported.
pub fn canonical_macros(patch: &mut Patch) -> Vec<String> {
    let mut dropped = Vec::new();
    let mut slots: Vec<MacroSpec> = MACRO_NAMES.iter().map(|n| MacroSpec { name: (*n).into(), targets: Vec::new() }).collect();
    for m in patch.macros.drain(..) {
        match MACRO_NAMES.iter().position(|n| n.eq_ignore_ascii_case(m.name.trim())) {
            Some(k) => slots[k] = MacroSpec { name: MACRO_NAMES[k].into(), targets: m.targets },
            None => dropped.push(m.name),
        }
    }
    patch.macros = slots;
    dropped
}

fn slug(name: &str) -> String {
    let s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let s = s.trim_matches('_').chars().take(40).collect::<String>();
    if s.is_empty() {
        "preset".into()
    } else {
        s
    }
}

// ------------------------------------------------------- shared UI<->audio

/// Built-in controls kept in `Shared::builtins`.
#[derive(Clone, Copy, PartialEq, Debug)]
enum B {
    Level,
    Voices,
    Glide,
    Transpose,
    Root,
    Scale,
    Attack,
    Decay,
    Sustain,
    Release,
    Velocity,
    Budget,
}
const NB: usize = 12;

struct BSpec {
    name: &'static str,
    min: f32,
    max: f32,
    default: f32,
    step: f32,
}

fn bspec(b: B) -> BSpec {
    let s = |name, min, max, default, step| BSpec { name, min, max, default, step };
    match b {
        B::Level => s("Level", -40.0, 6.0, -3.0, 0.5),
        B::Voices => s("Voices", 1.0, 16.0, 16.0, 1.0),
        B::Glide => s("Glide", 0.0, 1.0, 0.0, 0.01),
        B::Transpose => s("Transpose", -24.0, 24.0, 0.0, 1.0),
        B::Root => s("Root", 24.0, 72.0, 48.0, 1.0),
        B::Scale => s("Scale", 0.0, (SCALES.len() - 1) as f32, 1.0, 1.0),
        B::Attack => s("Attack", 0.001, 4.0, 0.005, 0.0),
        B::Decay => s("Decay", 0.01, 4.0, 0.3, 0.0),
        B::Sustain => s("Sustain", 0.0, 1.0, 0.8, 0.02),
        B::Release => s("Release", 0.01, 8.0, 0.4, 0.0),
        B::Velocity => s("Velocity", 0.05, 1.0, 0.8, 0.05),
        B::Budget => s("CPU Budget", 20.0, 1000.0, DEFAULT_BUDGET, 10.0),
    }
}

/// `settings` keys a preset can set, and which built-in each sets.
const SETTINGS: [(&str, B); 11] = [
    ("level", B::Level),
    ("voices", B::Voices),
    ("glide", B::Glide),
    ("transpose", B::Transpose),
    ("root", B::Root),
    ("scale", B::Scale),
    ("attack", B::Attack),
    ("decay", B::Decay),
    ("sustain", B::Sustain),
    ("release", B::Release),
    ("velocity", B::Velocity),
];

struct Shared {
    /// Each morph state's parameter positions, normalized 0..1.
    states: [[AtomicF32; MAX_PARAMS]; MAX_STATES],
    n_states: AtomicUsize,
    macros: [AtomicF32; MAX_MACROS],
    morph: AtomicF32,
    ext_macros: [Arc<AtomicF32>; N_MACROS],
    ext_morph: Arc<AtomicF32>,
    builtins: [AtomicF32; NB],
    held: [AtomicBool; 16],
    pending: Mutex<Option<Box<Engine>>>,
    retired: Mutex<Option<Box<Engine>>>,
    sample_rate: AtomicU32,
    /// Latest WAVE_LEN output samples (mono), oldest first.
    wave: Mutex<Vec<f32>>,
    activity: [AtomicF32; 64],
    peak: AtomicF32,
    load: AtomicF32,
    active_voices: AtomicUsize,
    /// How many voices the CPU budget allows for the current patch.
    voice_fit: AtomicUsize,
    unstable: AtomicBool,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Shared {
    fn new(name: &str, modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register(name, modbus);
        let all = all_b();
        Self {
            states: std::array::from_fn(|_| std::array::from_fn(|_| AtomicF32::new(0.0))),
            n_states: AtomicUsize::new(1),
            macros: std::array::from_fn(|_| AtomicF32::new(0.0)),
            morph: AtomicF32::new(0.0),
            ext_macros: std::array::from_fn(|k| modbus.register(format!("{name}: {}", title_case(MACRO_NAMES[k])))),
            ext_morph: modbus.register(format!("{name}: Morph")),
            builtins: std::array::from_fn(|i| AtomicF32::new(bspec(all[i]).default)),
            held: std::array::from_fn(|_| AtomicBool::new(false)),
            pending: Mutex::new(None),
            retired: Mutex::new(None),
            sample_rate: AtomicU32::new(48_000f32.to_bits()),
            wave: Mutex::new(vec![0.0; WAVE_LEN]),
            activity: std::array::from_fn(|_| AtomicF32::new(0.0)),
            peak: AtomicF32::new(0.0),
            load: AtomicF32::new(0.0),
            active_voices: AtomicUsize::new(0),
            voice_fit: AtomicUsize::new(0),
            unstable: AtomicBool::new(false),
            bus_out: audio_bus.register(name),
            mix_level,
            ext_mix_level,
        }
    }

    fn b(&self, b: B) -> f32 {
        self.builtins[b as usize].get()
    }

    fn set_b(&self, b: B, v: f32) {
        let s = bspec(b);
        self.builtins[b as usize].set(v.clamp(s.min, s.max));
    }

    fn morph(&self) -> f32 {
        (self.morph.get() + self.ext_morph.get()).clamp(0.0, 1.0)
    }

    /// Where MORPH sits among the states: (lower state, upper, blend).
    fn morph_pos(&self) -> (usize, usize, f32) {
        let n = self.n_states.load(Ordering::Relaxed).clamp(1, MAX_STATES);
        let pos = self.morph() * (n - 1) as f32;
        let i = (pos.floor() as usize).min(n - 1);
        (i, (i + 1).min(n - 1), pos - i as f32)
    }

    /// The parameter positions MORPH is currently at.
    fn norm(&self, p: usize) -> f32 {
        let (i, j, t) = self.morph_pos();
        let (a, b) = (self.states[i][p].get(), self.states[j][p].get());
        a + (b - a) * t
    }

    fn controls(&self) -> Controls {
        let mut c = Controls {
            norms: std::array::from_fn(|p| self.norm(p)),
            macros: std::array::from_fn(|k| (self.macros[k].get() + if k < N_MACROS { self.ext_macros[k].get() } else { 0.0 }).clamp(0.0, 1.0)),
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
        c
    }
}

fn all_b() -> [B; NB] {
    [B::Level, B::Voices, B::Glide, B::Transpose, B::Root, B::Scale, B::Attack, B::Decay, B::Sustain, B::Release, B::Velocity, B::Budget]
}

fn title_case(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut c = lower.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

// ------------------------------------------------------------ processor

struct AtlasProcessor {
    s: Arc<Shared>,
    current: Option<Box<Engine>>,
    fading: Option<Box<Engine>>,
    fade: usize,
    l: Vec<f32>,
    r: Vec<f32>,
    fl: Vec<f32>,
    fr: Vec<f32>,
    mono: Vec<f32>,
    ring: Vec<f32>,
    ring_pos: usize,
}

impl AudioProcessor for AtlasProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let start = Instant::now();
        let channels = channels.max(1);
        let frames = buffer.len() / channels;
        let s = Arc::clone(&self.s);
        s.sample_rate.store(sample_rate.to_bits(), Ordering::Relaxed);

        // A new engine? (never blocks)
        if self.fading.is_none() {
            if let Ok(mut slot) = s.pending.try_lock() {
                if slot.is_some() {
                    self.fading = self.current.take();
                    self.current = slot.take();
                    self.fade = if self.fading.is_some() { FADE_SAMPLES } else { 0 };
                }
            }
        }
        // A faded-out engine goes back to the UI thread to be freed.
        if self.fading.is_some() && self.fade == 0 {
            if let Ok(mut slot) = s.retired.try_lock() {
                if slot.is_none() {
                    *slot = self.fading.take();
                }
            }
        }
        for v in [&mut self.l, &mut self.r, &mut self.fl, &mut self.fr, &mut self.mono] {
            v.clear();
            v.resize(frames, 0.0);
        }

        let mut ctl = s.controls();
        let budget = s.b(B::Budget);
        if let Some(e) = self.current.as_mut() {
            let fit = e.voices_within(budget);
            s.voice_fit.store(fit, Ordering::Relaxed);
            ctl.voices_limit = ctl.voices_limit.min(fit.max(1));
            e.process(&ctl, &[], &mut self.l, &mut self.r, sample_rate);
        }
        if self.fade > 0 {
            if let Some(old) = self.fading.as_mut() {
                old.process(&ctl, &[], &mut self.fl, &mut self.fr, sample_rate);
            }
            for i in 0..frames {
                let t = self.fade as f32 / FADE_SAMPLES as f32;
                self.fade = self.fade.saturating_sub(1);
                let (gin, gout) = ((1.0 - t).sqrt(), t.sqrt());
                self.l[i] = self.l[i] * gin + self.fl[i] * gout;
                self.r[i] = self.r[i] * gin + self.fr[i] * gout;
            }
        }

        // Telemetry.
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
            let m = (self.l[i] + self.r[i]) * 0.5;
            self.mono[i] = m;
            peak = peak.max(m.abs());
            self.ring[self.ring_pos] = m;
            self.ring_pos = (self.ring_pos + 1) % WAVE_LEN;
        }
        s.peak.set(peak.max(s.peak.get() * 0.85));
        if let Ok(mut w) = s.wave.try_lock() {
            let (a, b) = self.ring.split_at(self.ring_pos);
            w[..b.len()].copy_from_slice(b);
            w[b.len()..].copy_from_slice(a);
        }

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
        let span = frames as f32 / sample_rate.max(1.0);
        s.load.set(start.elapsed().as_secs_f32() / span.max(1e-6));
    }
}

// ------------------------------------------------------------ the menu

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Header(&'static str),
    PageHeader(usize),
    Preset,
    Depth,
    Macro(usize),
    Morph,
    Builtin(B),
    Param(usize),
    Store(usize),
    Mutate,
    Revert,
    Save,
    Cost,
    Node(usize),
}

// ------------------------------------------------------------ the app

pub struct AtlasApp {
    s: Arc<Shared>,
    sensitivity: Arc<AtomicF32>,
    nav: Arc<AtomicF32>,
    kit: PlayKit,
    list: ParamList,
    library: Vec<Entry>,
    preset: usize,
    patch: Patch,
    maps: Vec<ParamMap>,
    graph: Vec<GraphNode>,
    cost_voice: f32,
    cost_global: f32,
    poly: usize,
    depth: usize,
    status: String,
    rng: Rng,
    prev_grid: [bool; 16],
    /// Where a state pad is gliding MORPH to, if it is.
    morph_target: Option<f32>,
    states_prev: [bool; 16],
}

impl AtlasApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self::build(APP_NAME, "atlas", scan(), sensitivity, nav, modbus, audio_bus, mixer_bus)
    }

    /// A cartridge: one patch file as its own app, under its own name
    /// (its own mixer channel, modulation inputs and moments) -- an
    /// instrument added by dropping a folder in apps/, no code. See
    /// `create`.
    #[allow(clippy::too_many_arguments)]
    pub fn cartridge(name: &str, id: &str, patch: std::path::PathBuf, sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let entry = Entry { name: name.into(), category: String::new(), source: Source::File(patch) };
        Self::build(name, id, vec![entry], sensitivity, nav, modbus, audio_bus, mixer_bus)
    }

    #[allow(clippy::too_many_arguments)]
    fn build(name: &str, id: &str, library: Vec<Entry>, sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let s = Arc::new(Shared::new(name, &modbus, &audio_bus, &mixer_bus));
        let mut cfg = kit_config();
        cfg.app_id = if id == "atlas" { "atlas" } else { Box::leak(id.to_string().into_boxed_str()) };
        let mut app = Self {
            s,
            sensitivity,
            nav,
            kit: PlayKit::new(cfg, !cfg!(test)),
            list: ParamList::new(),
            library,
            preset: 0,
            patch: Patch::from_json(FACTORY[0].1).expect("factory preset parses"),
            maps: Vec::new(),
            graph: Vec::new(),
            cost_voice: 0.0,
            cost_global: 0.0,
            poly: 0,
            depth: 0,
            status: String::new(),
            rng: Rng::seeded(),
            prev_grid: [false; 16],
            morph_target: None,
            states_prev: [false; 16],
        };
        app.kit.suggest_pressure_route(C_ENERGY);
        app.load(0);
        app
    }

    fn sr(&self) -> f32 {
        f32::from_bits(self.s.sample_rate.load(Ordering::Relaxed))
    }

    /// Load library entry `idx`: compile it, then (only if that worked)
    /// take over its parameters, states, macros and settings.
    fn load(&mut self, idx: usize) {
        let Some(entry) = self.library.get(idx).cloned() else { return };
        let mut patch = match load_entry(&entry) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("{}: {e}", entry.name);
                return;
            }
        };
        let dropped = canonical_macros(&mut patch);
        let engine = match compile(&patch, self.sr()) {
            Ok(e) => e,
            Err(errs) => {
                self.status = format!("{} won't compile: {}", entry.name, errs.first().cloned().unwrap_or_default());
                return;
            }
        };
        self.release_all();
        self.morph_target = None;
        self.preset = idx;
        self.maps = patch.params.iter().map(ParamMap::from_spec).collect();
        self.graph = engine.graph.clone();
        self.cost_voice = engine.cost_voice;
        self.cost_global = engine.cost_global;
        self.poly = engine.polyphony();

        let defaults: Vec<f32> = patch.params.iter().zip(&self.maps).map(|(p, m)| m.norm(p.default)).collect();
        let states: Vec<State> = if patch.states.is_empty() { vec![State::default()] } else { patch.states.iter().take(MAX_STATES).cloned().collect() };
        for (k, st) in states.iter().enumerate() {
            for (i, spec) in patch.params.iter().enumerate().take(MAX_PARAMS) {
                let v = st.params.get(&spec.id).map(|v| self.maps[i].norm(*v)).unwrap_or(defaults[i]);
                self.s.states[k][i].set(v);
            }
        }
        self.s.n_states.store(states.len(), Ordering::Relaxed);
        for k in 0..MAX_MACROS {
            self.s.macros[k].set(states[0].macros.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0));
        }
        self.s.morph.set(0.0);
        for b in all_b() {
            if b != B::Budget {
                self.s.set_b(b, bspec(b).default);
            }
        }
        // all the voices the patch has, unless it says otherwise
        self.s.set_b(B::Voices, self.poly.max(1) as f32);
        for (key, b) in SETTINGS {
            if let Some(v) = patch.settings.get(key) {
                self.s.set_b(b, *v);
            }
        }
        self.s.unstable.store(false, Ordering::Relaxed);
        if let Ok(mut slot) = self.s.pending.lock() {
            *slot = Some(Box::new(engine)); // a never-played pending engine is simply dropped here
        }
        self.status = if dropped.is_empty() { String::new() } else { format!("no knob for macro {}", dropped.join(", ")) };
        self.patch = patch;
    }

    fn n_params(&self) -> usize {
        self.patch.params.len().min(MAX_PARAMS)
    }

    /// The state an edit lands in: whichever MORPH is nearest.
    fn edit_state(&self) -> usize {
        let (i, j, t) = self.s.morph_pos();
        if t < 0.5 {
            i
        } else {
            j
        }
    }

    fn param_value(&self, i: usize) -> f32 {
        self.maps[i].value(self.s.norm(i))
    }

    fn edit_param(&mut self, i: usize, d: i32) {
        if d == 0 || i >= self.n_params() {
            return;
        }
        let k = self.edit_state();
        let a = &self.s.states[k][i];
        let step = match self.maps[i].steps() {
            Some(n) => d.signum() as f32 / (n.max(2) - 1) as f32,
            None => d as f32 * 0.005 * self.sensitivity.get().max(0.01) * 10.0,
        };
        a.set((a.get() + step).clamp(0.0, 1.0));
    }

    fn reset_param(&mut self, i: usize) {
        if let (Some(spec), Some(map)) = (self.patch.params.get(i), self.maps.get(i)) {
            self.s.states[self.edit_state()][i].set(map.norm(spec.default));
        }
    }

    /// Copy where MORPH is now into state `k` (adding states up to it).
    fn store_state(&mut self, k: usize) {
        let k = k.min(MAX_STATES - 1);
        let now: Vec<f32> = (0..MAX_PARAMS).map(|p| self.s.norm(p)).collect();
        let n = self.s.n_states.load(Ordering::Relaxed);
        // new states between the old last one and k start as copies of it
        for fill in n..k {
            for p in 0..MAX_PARAMS {
                self.s.states[fill][p].set(self.s.states[n - 1][p].get());
            }
        }
        for (p, v) in now.iter().enumerate() {
            self.s.states[k][p].set(*v);
        }
        self.s.n_states.store(n.max(k + 1), Ordering::Relaxed);
        self.status = format!("stored state {}", STATE_NAMES[k]);
    }

    fn mutate(&mut self) {
        let k = self.edit_state();
        let n = self.n_params();
        let cur: Vec<f32> = (0..n).map(|p| self.s.states[k][p].get()).collect();
        let next = evolve::mutate(&cur, &vec![false; n], 0.15, &mut self.rng);
        for (p, v) in next.iter().enumerate() {
            self.s.states[k][p].set(*v);
        }
        self.status = format!("mutated state {}", STATE_NAMES[k]);
    }

    /// The patch as it stands: every state, the macros, the settings.
    fn current_patch(&self) -> Patch {
        let mut p = self.patch.clone();
        p.version = CURRENT_VERSION;
        let n = self.s.n_states.load(Ordering::Relaxed).clamp(1, MAX_STATES);
        p.states = (0..n)
            .map(|k| State {
                params: p.params.iter().enumerate().take(MAX_PARAMS).map(|(i, spec)| (spec.id.clone(), self.maps[i].value(self.s.states[k][i].get()))).collect(),
                macros: if k == 0 { (0..N_MACROS).map(|m| self.s.macros[m].get()).collect() } else { Vec::new() },
            })
            .collect();
        p.state = None;
        for (key, b) in SETTINGS {
            p.settings.insert(key.into(), self.s.b(b));
        }
        p
    }

    fn save(&mut self) {
        let p = self.current_patch();
        let dir = user_dir();
        let res = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|_| {
            let base = slug(&p.name);
            let mut path = dir.join(format!("{base}.json"));
            let mut k = 2;
            while path.exists() {
                path = dir.join(format!("{base}_{k}.json"));
                k += 1;
            }
            std::fs::write(&path, p.to_json()).map_err(|e| e.to_string()).map(|_| path)
        });
        match res {
            Ok(path) => {
                if self.kit.cfg.app_id == "atlas" {
                    self.library = scan();
                }
                if let Some(i) = self.library.iter().position(|e| matches!(&e.source, Source::File(f) if *f == path)) {
                    self.preset = i;
                }
                self.status = format!("saved {}", path.file_name().and_then(|f| f.to_str()).unwrap_or(""));
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }

    fn release_all(&mut self) {
        for h in &self.s.held {
            h.store(false, Ordering::Relaxed);
        }
        self.prev_grid = [false; 16];
    }

    fn handle_pads(&mut self, grid: &[bool; 16]) {
        for (i, (h, g)) in self.s.held.iter().zip(grid.iter()).enumerate() {
            if *g != self.prev_grid[i] {
                h.store(*g, Ordering::Relaxed);
            }
        }
        self.prev_grid = *grid;
    }

    /// STATES layer: the bottom row of pads (A B C D, left to right) glide
    /// MORPH to that state. No dial needed: a tap is the whole gesture.
    fn state_pads(&mut self, grid: &[bool; 16]) {
        let n = self.s.n_states.load(Ordering::Relaxed).clamp(1, MAX_STATES);
        for k in 0..MAX_STATES {
            let pad = kit::rank_pad(k);
            if grid[pad] && !self.states_prev[pad] && k < n {
                if n < 2 {
                    self.status = "This sound has one state: store more in Edit".into();
                } else {
                    self.morph_target = Some(k as f32 / (n - 1) as f32);
                    self.status = format!("Gliding to state {}", STATE_NAMES[k]);
                }
            }
        }
        self.states_prev = *grid;
    }

    /// Moves MORPH toward a state pad's target, a little each frame.
    fn glide_morph(&mut self) {
        let Some(target) = self.morph_target else { return };
        let now = self.s.morph.get();
        let next = now + (target - now) * MORPH_GLIDE;
        if (target - next).abs() < 0.002 {
            self.s.morph.set(target);
            self.morph_target = None;
        } else {
            self.s.morph.set(next);
        }
    }

    fn pad_midi(&self, pad: usize) -> i32 {
        let s = &self.s;
        pad_note(pad, s.b(B::Root).round() as i32, s.b(B::Scale).round().max(0.0) as usize, s.b(B::Transpose).round() as i32)
    }

    /// The macro slots this preset actually uses.
    fn macro_used(&self, k: usize) -> bool {
        self.patch.macros.get(k).is_some_and(|m| !m.targets.is_empty())
    }

    fn morph_label(&self) -> String {
        let n = self.s.n_states.load(Ordering::Relaxed);
        if n <= 1 {
            return "A only".into();
        }
        let (i, j, t) = self.s.morph_pos();
        if t < 0.02 || i == j {
            STATE_NAMES[i].into()
        } else if t > 0.98 {
            STATE_NAMES[j].into()
        } else {
            format!("{}→{} {:.0}%", STATE_NAMES[i], STATE_NAMES[j], t * 100.0)
        }
    }

    fn fit(&self) -> usize {
        let f = self.s.voice_fit.load(Ordering::Relaxed);
        if f > 0 {
            return f;
        }
        // before the audio thread has reported, estimate it the same way
        let per = self.cost_voice.max(1e-3);
        (((self.s.b(B::Budget) - self.cost_global) / per).floor().max(1.0) as usize).min(self.poly.max(1))
    }

    // ---- kit controls

    fn knob(&self, c: usize) -> Knob<'_> {
        match c {
            k if k < N_MACROS => Knob::F(&self.s.macros[k], 0.0, 1.0),
            C_MORPH => Knob::F(&self.s.morph, 0.0, 1.0),
            C_LEVEL => Knob::F(&self.s.builtins[B::Level as usize], bspec(B::Level).min, bspec(B::Level).max),
            _ => Knob::None,
        }
    }

    fn label(&self, c: usize) -> String {
        match c {
            k if k < N_MACROS => MACRO_NAMES[k].into(),
            C_MORPH => "MORPH".into(),
            C_PRESET => "Preset".into(),
            C_LEVEL => "Level".into(),
            C_VOICES => "Voices".into(),
            _ => "Depth".into(),
        }
    }

    fn value(&self, c: usize) -> String {
        match c {
            k if k < N_MACROS => {
                if self.macro_used(k) {
                    format!("{:.0}%", self.s.macros[k].get() * 100.0)
                } else {
                    "—".into()
                }
            }
            C_MORPH => self.morph_label(),
            C_PRESET => self.library.get(self.preset).map(|e| e.name.clone()).unwrap_or_default(),
            C_LEVEL => format!("{:.1} dB", self.s.b(B::Level)),
            C_VOICES => {
                let want = self.s.b(B::Voices).round() as usize;
                let can = self.fit().min(self.poly.max(1));
                if want > can {
                    format!("{want} (fits {can})")
                } else {
                    format!("{want}")
                }
            }
            _ => DEPTHS[self.depth].into(),
        }
    }

    fn edit_control(&mut self, c: usize, d: i32) {
        if d == 0 {
            return;
        }
        let sens = self.sensitivity.get().max(0.01) * 10.0;
        match c {
            k if k < N_MACROS || k == C_MORPH => {
                let a = if k == C_MORPH { &self.s.morph } else { &self.s.macros[k] };
                a.set((a.get() + d as f32 * 0.01 * sens).clamp(0.0, 1.0));
            }
            C_PRESET => {
                let n = self.library.len().max(1) as i32;
                let next = (self.preset as i32 + d.signum()).rem_euclid(n) as usize;
                self.load(next);
            }
            C_LEVEL => self.edit_b(B::Level, d),
            C_VOICES => self.edit_b(B::Voices, d.signum()),
            _ => self.depth = (self.depth as i32 + d.signum()).clamp(0, DEPTHS.len() as i32 - 1) as usize,
        }
    }

    fn edit_b(&mut self, b: B, d: i32) {
        let s = bspec(b);
        let v = self.s.b(b);
        let next = if s.step > 0.0 {
            v + s.step * d as f32
        } else {
            // times: proportional steps feel even from 1 ms to 8 s
            v * 1.06f32.powi(d)
        };
        if matches!(b, B::Root | B::Scale | B::Transpose) {
            self.release_all(); // a held pad would strand its old note
        }
        self.s.set_b(b, next);
    }

    // ---- menu

    fn rows(&self) -> Vec<Row> {
        let mut r = vec![Row::Preset, Row::Depth, Row::Header("Perform")];
        r.extend((0..N_MACROS).filter(|&k| self.macro_used(k)).map(Row::Macro));
        r.push(Row::Morph);
        r.extend([Row::Builtin(B::Level), Row::Builtin(B::Voices), Row::Builtin(B::Root), Row::Builtin(B::Scale), Row::Builtin(B::Transpose)]);
        if self.depth >= 1 {
            let pages = self.patch.params.iter().map(|p| p.page).max().unwrap_or(1).max(1);
            for page in 1..=pages {
                let on_page: Vec<usize> = (0..self.n_params()).filter(|&i| self.patch.params[i].page.max(1) == page).collect();
                if !on_page.is_empty() {
                    r.push(Row::PageHeader(page as usize));
                    r.extend(on_page.into_iter().map(Row::Param));
                }
            }
            r.push(Row::Header("Voice"));
            r.extend([B::Attack, B::Decay, B::Sustain, B::Release, B::Glide, B::Velocity].map(Row::Builtin));
            r.push(Row::Header("States"));
            let n = self.s.n_states.load(Ordering::Relaxed);
            r.extend((0..(n + 1).min(MAX_STATES)).map(Row::Store));
            r.extend([Row::Mutate, Row::Revert, Row::Save]);
        }
        if self.depth >= 2 {
            r.push(Row::Header("Engine"));
            r.extend([Row::Builtin(B::Budget), Row::Cost]);
            r.extend((0..self.graph.len()).map(Row::Node));
        }
        r
    }

    fn row_text(&self, row: Row) -> (String, String, bool) {
        let s = &self.s;
        match row {
            Row::Header(h) => (h.into(), String::new(), true),
            Row::PageHeader(p) => (self.patch.page_names.get(p - 1).cloned().unwrap_or_else(|| format!("Page {p}")), String::new(), true),
            Row::Preset => {
                let cat = self.library.get(self.preset).map(|e| e.category.clone()).unwrap_or_default();
                ("Preset".into(), format!("{} · {cat} ({}/{})", self.value(C_PRESET), self.preset + 1, self.library.len()), false)
            }
            Row::Depth => ("Depth".into(), DEPTHS[self.depth].into(), false),
            Row::Macro(k) => (format!("  {}", MACRO_NAMES[k]), self.value(k), false),
            Row::Morph => ("  MORPH".into(), self.morph_label(), false),
            Row::Builtin(b) => {
                let v = s.b(b);
                let text = match b {
                    B::Level => format!("{v:.1} dB"),
                    B::Voices => self.value(C_VOICES),
                    B::Root => note_name(v.round() as i32),
                    B::Scale => SCALES.get(v.round() as usize).map_or("?", |x| x.0).into(),
                    B::Transpose => format!("{:+} st", v.round() as i32),
                    B::Attack | B::Decay | B::Release | B::Glide => {
                        if v < 1.0 {
                            format!("{:.0} ms", v * 1000.0)
                        } else {
                            format!("{v:.2} s")
                        }
                    }
                    B::Sustain | B::Velocity => format!("{:.0}%", v * 100.0),
                    B::Budget => format!("{v:.0} units"),
                };
                (format!("  {}", bspec(b).name), text, false)
            }
            Row::Param(i) => {
                let spec = &self.patch.params[i];
                (format!("  {}", spec.name), format_value(spec, &self.maps[i], self.param_value(i)), false)
            }
            Row::Store(k) => {
                let n = s.n_states.load(Ordering::Relaxed);
                let what = if k < n { "overwrite" } else { "add" };
                (format!("  Store → {}", STATE_NAMES[k]), format!("press: {what}"), false)
            }
            Row::Mutate => ("  Mutate".into(), format!("press: state {}", STATE_NAMES[self.edit_state()]), false),
            Row::Revert => ("  Revert".into(), "press: reload".into(), false),
            Row::Save => ("  Save".into(), "press: save copy".into(), false),
            Row::Cost => {
                let fit = self.fit();
                (format!("  Cost ({} voices)", fit.min(self.poly.max(1))), format!("{:.0} + {:.0}/voice", self.cost_global, self.cost_voice), false)
            }
            Row::Node(i) => {
                let n = &self.graph[i];
                let act = s.activity.get(i).map_or(0.0, |a| a.get());
                (format!("  {}{} ({})", if n.voice { "" } else { "◆ " }, n.id, n.kind), format!("{:.0}%", (act * 200.0).min(100.0)), false)
            }
        }
    }

    fn edit_row(&mut self, row: Row, d: i32) {
        if d == 0 {
            return;
        }
        match row {
            Row::Preset => self.edit_control(C_PRESET, d),
            Row::Depth => self.edit_control(C_DEPTH, d),
            Row::Macro(k) => self.edit_control(k, d),
            Row::Morph => self.edit_control(C_MORPH, d),
            Row::Builtin(b) => self.edit_b(b, d),
            Row::Param(i) => self.edit_param(i, d),
            _ => {}
        }
    }

    fn press_row(&mut self, row: Row) {
        match row {
            Row::Macro(k) => self.s.macros[k].set(0.0),
            Row::Morph => self.s.morph.set(0.0),
            Row::Builtin(b) => self.s.set_b(b, self.patch_setting(b)),
            Row::Param(i) => self.reset_param(i),
            Row::Store(k) => self.store_state(k),
            Row::Mutate => self.mutate(),
            Row::Revert => self.load(self.preset),
            Row::Save => self.save(),
            _ => {}
        }
    }

    /// A built-in's value as the preset set it (or the default).
    fn patch_setting(&self, b: B) -> f32 {
        let default = if b == B::Voices { self.poly.max(1) as f32 } else { bspec(b).default };
        SETTINGS.iter().find(|(_, x)| *x == b).and_then(|(k, _)| self.patch.settings.get(*k).copied()).unwrap_or(default)
    }

    // ---- visuals

    /// The latest output, oldest first.
    fn wave(&self) -> Vec<f32> {
        self.s.wave.lock().map(|w| w.clone()).unwrap_or_default()
    }

    /// Coarse log-spaced spectrum of the latest output, 0..1 per band
    /// (Hann window, one Goertzel per band centre; -72..0 dBFS).
    pub fn spectrum(&self) -> Vec<f32> {
        let w = self.wave();
        let sr = self.sr();
        let n = w.len().max(1);
        let win: Vec<f32> = w.iter().enumerate().map(|(i, x)| x * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos())).collect();
        (0..SPECTRUM_BANDS)
            .map(|b| {
                let f = 40.0 * (16_000.0f32 / 40.0).powf(b as f32 / (SPECTRUM_BANDS - 1) as f32);
                let coef = 2.0 * (std::f32::consts::TAU * f / sr).cos();
                let (mut s1, mut s2) = (0.0f32, 0.0f32);
                for x in &win {
                    let s0 = x + coef * s1 - s2;
                    s2 = s1;
                    s1 = s0;
                }
                let mag = (s1 * s1 + s2 * s2 - coef * s1 * s2).max(0.0).sqrt() / (n as f32 * 0.25);
                ((20.0 * mag.max(1e-6).log10() + 72.0) / 72.0).clamp(0.0, 1.0)
            })
            .collect()
    }

    #[allow(dead_code)] // Slint GUI only
    fn scope(&self) -> Vec<f32> {
        let w = self.wave();
        // start at a rising zero crossing so the trace holds still
        let half = w.len() / 2;
        let start = (1..half).find(|&i| w[i - 1] <= 0.0 && w[i] > 0.0).unwrap_or(0);
        let span = &w[start..(start + half).min(w.len())];
        let peak = span.iter().fold(0.05f32, |m, x| m.max(x.abs()));
        (0..SCOPE_POINTS).map(|i| span.get(i * span.len() / SCOPE_POINTS).copied().unwrap_or(0.0) / peak).collect()
    }
}

impl PlayHost for AtlasApp {
    fn kit_control_count(&self) -> usize {
        N_CONTROLS
    }
    fn kit_label(&self, i: usize) -> String {
        self.label(i % N_CONTROLS)
    }
    fn kit_value(&self, i: usize) -> String {
        self.value(i % N_CONTROLS)
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        match i {
            C_PRESET => Some(self.preset as f32 / (self.library.len().max(2) - 1) as f32),
            C_VOICES => Some((self.s.b(B::Voices) - 1.0) / 15.0),
            C_DEPTH => Some(self.depth as f32 / 2.0),
            _ => self.knob(i).norm(),
        }
    }
    fn kit_stepped(&self, i: usize) -> bool {
        matches!(i, C_PRESET | C_VOICES | C_DEPTH)
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit_control(i, delta);
    }
    fn kit_reset(&mut self, i: usize) {
        match i {
            k if k < N_MACROS => self.s.macros[k].set(0.0),
            C_MORPH => self.s.morph.set(0.0),
            C_LEVEL => self.s.set_b(B::Level, self.patch_setting(B::Level)),
            C_VOICES => self.s.set_b(B::Voices, self.patch_setting(B::Voices)),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i {
            C_PRESET => {
                let idx = (v * (self.library.len().max(2) - 1) as f32).round() as usize;
                if idx != self.preset {
                    self.load(idx);
                }
            }
            C_VOICES => self.s.set_b(B::Voices, 1.0 + (v * 15.0).round()),
            C_DEPTH => self.depth = (v * 2.0).round() as usize,
            _ => self.knob(i).set(v),
        }
    }
    /// A Moment is the whole sound: the preset, every state's parameters,
    /// the macros, MORPH and the settings.
    fn kit_snapshot(&self) -> serde_json::Value {
        let p = self.current_patch();
        serde_json::json!({
            "preset": self.library.get(self.preset).map(|e| e.name.clone()),
            "states": p.states,
            "macros": (0..N_MACROS).map(|k| self.s.macros[k].get()).collect::<Vec<_>>(),
            "morph": self.s.morph.get(),
            "settings": p.settings,
        })
    }
    fn kit_recall(&mut self, v: &serde_json::Value) {
        if let Some(name) = v.get("preset").and_then(|n| n.as_str()) {
            match self.library.iter().position(|e| e.name == name) {
                Some(i) if i != self.preset => self.load(i),
                Some(_) => {}
                None => {
                    self.status = format!("{name} isn't in the library");
                    return;
                }
            }
        }
        if let Some(states) = v.get("states").and_then(|s| serde_json::from_value::<Vec<State>>(s.clone()).ok()) {
            let n = states.len().clamp(1, MAX_STATES);
            for (k, st) in states.iter().take(n).enumerate() {
                for (i, spec) in self.patch.params.iter().enumerate().take(MAX_PARAMS) {
                    if let Some(val) = st.params.get(&spec.id) {
                        self.s.states[k][i].set(self.maps[i].norm(*val));
                    }
                }
            }
            self.s.n_states.store(n, Ordering::Relaxed);
        }
        if let Some(m) = v.get("macros").and_then(|m| m.as_array()) {
            for (k, x) in m.iter().enumerate().take(N_MACROS) {
                self.s.macros[k].set(x.as_f64().unwrap_or(0.0) as f32);
            }
        }
        if let Some(m) = v.get("morph").and_then(|m| m.as_f64()) {
            self.s.morph.set(m as f32);
        }
        if let Some(set) = v.get("settings").and_then(|s| s.as_object()) {
            for (key, b) in SETTINGS {
                if let Some(x) = set.get(key).and_then(|x| x.as_f64()) {
                    self.s.set_b(b, x as f32);
                }
            }
        }
    }
    fn kit_line(&self) -> String {
        let cat = &self.patch.category;
        format!("{}{}  ·  {} voices", if cat.is_empty() { String::new() } else { format!("{cat}  ·  ") }, self.morph_label(), self.s.active_voices.load(Ordering::Relaxed))
    }
    fn kit_pads_play(&self, layer: u8) -> bool {
        layer != STATES_LAYER
    }
    fn kit_pad_label(&self, layer: u8, pad: usize) -> String {
        if layer == STATES_LAYER {
            let n = self.s.n_states.load(Ordering::Relaxed).clamp(1, MAX_STATES);
            let k = kit::pad_rank(pad);
            return if k < n { STATE_NAMES[k].into() } else { String::new() };
        }
        note_name(self.pad_midi(pad).clamp(0, 127))
    }
    /// A key plays the pad of the same pitch (or pitch class).
    fn kit_midi_pad(&self, note: u8) -> Option<usize> {
        let n = note as i32;
        (0..16).find(|&p| self.pad_midi(p) == n).or_else(|| (0..16).find(|&p| self.pad_midi(p).rem_euclid(12) == n % 12))
    }
    fn kit_pad_color(&self, layer: u8, pad: usize, held: bool) -> PadColor {
        if layer == STATES_LAYER {
            let n = self.s.n_states.load(Ordering::Relaxed).clamp(1, MAX_STATES);
            let k = kit::pad_rank(pad);
            if k >= n {
                return PadColor::Off;
            }
            let at = if n > 1 { k as f32 / (n - 1) as f32 } else { 0.0 };
            return if (self.s.morph.get() - at).abs() < 0.03 {
                PadColor::Green
            } else if self.morph_target.is_some_and(|t| (t - at).abs() < 0.001) {
                PadColor::Yellow
            } else {
                PadColor::Blue
            };
        }
        if held || self.s.held[pad].load(Ordering::Relaxed) {
            PadColor::Green
        } else if (self.pad_midi(pad) - self.s.b(B::Root).round() as i32 - self.s.b(B::Transpose).round() as i32).rem_euclid(12) == 0 {
            PadColor::Blue
        } else {
            PadColor::Off
        }
    }
}

impl App for AtlasApp {
    fn play_surface(&self) -> bool {
        true
    }
    fn supports_pad_lock(&self) -> bool {
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
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        self.kit.led_overlay(self)
    }
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let input = &step.input;
        // STATES pads pick states instead of playing: no notes sound, and
        // switching layers releases anything held.
        if step.native == Some(STATES_LAYER) {
            self.handle_pads(&[false; 16]);
            self.state_pads(&input.grid);
        } else {
            self.handle_pads(&input.grid);
            self.states_prev = [false; 16];
        }
        self.glide_morph();
        if step.menu {
            let rows = self.rows();
            self.list.navigate_input(input, rows.len(), self.nav.get() as i32);
            self.list.selected = self.list.selected.min(rows.len().saturating_sub(1));
            let row = rows[self.list.selected];
            self.edit_row(row, input.knob2);
            // SELECT runs an action row; holding SELECT (reset) also
            // puts a value row back where the preset had it.
            let action = matches!(row, Row::Store(_) | Row::Mutate | Row::Revert | Row::Save);
            if input.knob2_press || (input.knob1_press && action) {
                self.press_row(row);
            }
        }
        if self.s.unstable.swap(false, Ordering::Relaxed) {
            self.status = "a block blew up and was reset".into();
        }
    }
    fn background_tick(&mut self) {
        // free engines the audio thread has finished with, off that thread
        if let Ok(mut slot) = self.s.retired.try_lock() {
            slot.take();
        }
    }
    fn on_exit(&mut self) {
        self.release_all();
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if !self.kit.menu {
            if let Some(col) = self.play_column() {
                let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
                kit::draw::column(f, &col, 16, 40, 350, 285, pal);
            }
        } else {
            let r: Vec<(String, String)> = self.rows().into_iter().map(|row| self.row_text(row)).map(|(a, b, _)| (a, b)).collect();
            self.list.draw(f, 16, 44, 24, r.len(), &r);
        }
        let big = MonoTextStyle::new(&SPLEEN_8X16, INK);
        let small = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        let x = 384;
        Text::new(&self.value(C_PRESET), Point::new(x, 52), big).draw(f).ok();
        Text::new(&self.patch.category, Point::new(x, 68), small).draw(f).ok();
        for k in 0..N_MACROS {
            let y = 84 + k as i32 * 16;
            let used = self.macro_used(k);
            Text::new(MACRO_NAMES[k], Point::new(x, y + 9), MonoTextStyle::new(&SPLEEN_6X12, if used { INK } else { FAINT })).draw(f).ok();
            Rectangle::new(Point::new(x + 64, y + 2), Size::new(80, 6)).into_styled(PrimitiveStyle::with_fill(FAINT)).draw(f).ok();
            if used {
                let w = (self.s.macros[k].get() * 80.0) as u32;
                Rectangle::new(Point::new(x + 64, y + 2), Size::new(w, 6)).into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(f).ok();
            }
        }
        let y = 84 + N_MACROS as i32 * 16 + 6;
        Text::new(&format!("MORPH {}", self.morph_label()), Point::new(x, y + 9), MonoTextStyle::new(&SPLEEN_6X12, INK)).draw(f).ok();
        Rectangle::new(Point::new(x, y + 14), Size::new(144, 4)).into_styled(PrimitiveStyle::with_fill(FAINT)).draw(f).ok();
        Rectangle::new(Point::new(x + (self.s.morph() * 140.0) as i32, y + 12), Size::new(4, 8)).into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(f).ok();
        // spectrum
        let base = 300;
        for (b, v) in self.spectrum().iter().enumerate() {
            let h = (v * 60.0) as u32;
            Rectangle::new(Point::new(x + b as i32 * 6, base - h as i32), Size::new(4, h.max(1))).into_styled(PrimitiveStyle::with_fill(INK)).draw(f).ok();
        }
        Text::new(&format!("{} / {}  ·  cpu {:.0}%", self.s.active_voices.load(Ordering::Relaxed), self.value(C_VOICES), self.s.load.get() * 100.0), Point::new(x, 318), small).draw(f).ok();
        let foot = if self.status.is_empty() { "pads: play   L/R: macros   U/D: preset   R1: menu" } else { self.status.as_str() };
        Text::new(foot, Point::new(16, 340), small).draw(f).ok();
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows().into_iter().map(|r| self.row_text(r)).collect()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        crate::app::SlintExtra::Atlas(AtlasExtra {
            name: self.value(C_PRESET),
            category: self.patch.category.to_uppercase(),
            description: self.patch.description.clone(),
            index: format!("{}/{}", self.preset + 1, self.library.len()),
            macro_names: MACRO_NAMES.iter().map(|s| title_case(s)).collect(),
            macro_values: (0..N_MACROS).map(|k| self.s.macros[k].get()).collect(),
            macro_used: (0..N_MACROS).map(|k| self.macro_used(k)).collect(),
            morph: self.s.morph(),
            states: self.s.n_states.load(Ordering::Relaxed),
            morph_label: self.morph_label(),
            spectrum_view: self.patch.visual != "scope",
            spectrum: self.spectrum(),
            scope: self.scope(),
            voices: format!("{} of {} voices", self.s.active_voices.load(Ordering::Relaxed), self.fit().min(self.s.b(B::Voices).round() as usize).min(self.poly.max(1))),
            load: self.s.load.get(),
            depth: DEPTHS[self.depth].into(),
            status: self.status.clone(),
        })
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(AtlasProcessor {
            s: Arc::clone(&self.s),
            current: None,
            fading: None,
            fade: 0,
            l: Vec::new(),
            r: Vec::new(),
            fl: Vec::new(),
            fr: Vec::new(),
            mono: Vec::new(),
            ring: vec![0.0; WAVE_LEN],
            ring_pos: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> AtlasApp {
        AtlasApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
    }

    fn pads(on: &[usize]) -> Input {
        Input { grid: std::array::from_fn(|k| on.contains(&k)), ..Default::default() }
    }

    fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> f32 {
        let mut e = 0.0;
        for _ in 0..blocks {
            let mut out = vec![0.0f32; 1024];
            p.process(&mut out, 2, 48_000.0);
            assert!(out.iter().all(|v| v.is_finite()), "non-finite output");
            e += out.iter().map(|v| v * v).sum::<f32>();
        }
        e / (blocks * 1024) as f32
    }

    /// Every factory preset parses, uses only the named macro slots,
    /// compiles, sounds when played, stays finite and bounded with every
    /// macro and MORPH at both extremes, and actually changes when MORPH
    /// moves.
    #[test]
    fn every_factory_preset_compiles_plays_and_morphs() {
        for (file, json) in FACTORY {
            let mut p = Patch::from_json(json).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(p.version, CURRENT_VERSION, "{file}");
            assert!(canonical_macros(&mut p).is_empty(), "{file}: macro without a slot");
            for st in &p.states {
                for id in st.params.keys() {
                    assert!(p.params.iter().any(|q| &q.id == id), "{file}: state sets unknown param {id}");
                }
            }
            for key in p.settings.keys() {
                assert!(SETTINGS.iter().any(|(k, _)| k == key), "{file}: unknown setting {key}");
            }
            let engine = compile(&p, 48_000.0).unwrap_or_else(|e| panic!("{file}: {e:?}"));
            assert!(engine.cost_voice > 0.0, "{file}");
        }
        let mut a = app();
        for idx in 0..FACTORY.len() {
            a.load(idx);
            assert!(a.status.is_empty(), "{}: {}", FACTORY[idx].0, a.status);
            let mut p = a.audio_processor().unwrap();
            a.tick(&pads(&[12, 14]));
            let base = render(&mut p, 40);
            assert!(base > 1e-6, "{}: silent ({base})", FACTORY[idx].0);
            for extreme in [0.0, 1.0] {
                for k in 0..N_MACROS {
                    a.s.macros[k].set(extreme);
                }
                a.s.morph.set(extreme);
                let e = render(&mut p, 30);
                assert!(e < 4.0, "{} at {extreme}: too loud ({e})", FACTORY[idx].0);
            }
            a.tick(&Input::default());
        }
    }

    fn pressed(on: usize, firmness: f32) -> Input {
        let mut input = pads(&[on]);
        input.pad_pressure[on] = firmness;
        input
    }

    /// With no dial to turn MORPH, the bottom row of pads (on the STATES
    /// layer) glides there: a tap is the whole gesture, nothing sounds,
    /// and the pads show where the sound is.
    #[test]
    fn state_pads_glide_morph_to_each_state_without_playing_notes() {
        let mut a = app();
        a.load(0); // Evolving Glass: states A and B
        a.toggle_grid_mode();
        assert_eq!(a.kit.layer_label(), "STATES");
        let b_pad = kit::rank_pad(1);
        a.tick(&pads(&[b_pad]));
        assert!(a.s.held.iter().all(|h| !h.load(Ordering::Relaxed)), "state pads don't sound notes");
        assert_eq!(a.morph_target, Some(1.0));
        assert!(a.status.contains("state B"), "{}", a.status);
        assert_eq!(a.kit_pad_color(STATES_LAYER, b_pad, false), PadColor::Yellow, "the target shows while it glides");
        assert_eq!(a.kit_pad_label(STATES_LAYER, b_pad), "B");
        assert_eq!(a.kit_pad_label(STATES_LAYER, kit::rank_pad(2)), "", "no state C in this sound");
        for _ in 0..200 {
            a.tick(&Input::default());
        }
        assert!((a.s.morph.get() - 1.0).abs() < 1e-3, "settled on B: {}", a.s.morph.get());
        assert!(a.morph_target.is_none());
        assert_eq!(a.kit_pad_color(STATES_LAYER, b_pad, false), PadColor::Green, "now at B");
        a.tick(&pads(&[kit::rank_pad(0)]));
        for _ in 0..200 {
            a.tick(&Input::default());
        }
        assert!(a.s.morph.get() < 1e-3, "and back to A: {}", a.s.morph.get());
    }

    /// Switching to the STATES layer lets go of any held note instead of
    /// leaving it hanging.
    #[test]
    fn leaving_the_play_layer_releases_held_notes() {
        let mut a = app();
        a.tick(&pads(&[3]));
        assert!(a.s.held[3].load(Ordering::Relaxed));
        a.toggle_grid_mode();
        a.tick(&pads(&[3]));
        assert!(!a.s.held[3].load(Ordering::Relaxed));
    }

    /// Pad pressure is Atlas's first dial-free modulation: out of the box
    /// it pushes ENERGY while pads are played, and lets go exactly.
    #[test]
    fn pad_pressure_pushes_energy_while_playing_and_not_on_the_state_pads() {
        let mut a = app();
        let base = a.s.macros[C_ENERGY].get();
        a.tick(&pressed(0, 0.5));
        let pushed = a.s.macros[C_ENERGY].get();
        assert!((pushed - (base + 0.3).min(1.0)).abs() < 1e-4, "0.5 pressure x 0.6 span: {base} -> {pushed}");
        a.tick(&pressed(0, 0.0));
        a.tick(&Input::default());
        assert!((a.s.macros[C_ENERGY].get() - base).abs() < 1e-4, "released");
        a.toggle_grid_mode(); // STATES: pads select, they don't play
        a.tick(&pressed(kit::rank_pad(1), 1.0));
        assert!((a.s.macros[C_ENERGY].get() - base).abs() < 1e-4, "selecting a state isn't musical pressure");
    }

    #[test]
    fn morph_interpolates_between_states() {
        let mut a = app();
        a.load(0); // Evolving Glass: A and B
        assert_eq!(a.s.n_states.load(Ordering::Relaxed), 2);
        let differs: Vec<usize> = (0..a.n_params()).filter(|&i| (a.s.states[0][i].get() - a.s.states[1][i].get()).abs() > 0.05).collect();
        assert!(!differs.is_empty(), "state B differs from A");
        let i = differs[0];
        let (va, vb) = (a.s.states[0][i].get(), a.s.states[1][i].get());
        a.s.morph.set(0.5);
        assert!((a.s.norm(i) - (va + vb) / 2.0).abs() < 1e-4);
        assert_eq!(a.morph_label(), "A→B 50%");
        // edits land in the nearest state
        a.s.morph.set(0.9);
        let before = a.s.states[1][i].get();
        a.edit_param(i, if before > 0.5 { -10 } else { 10 });
        assert!((a.s.states[1][i].get() - before).abs() > 1e-4);
        assert_eq!(a.s.states[0][i].get(), va, "state A untouched");
    }

    #[test]
    fn storing_adds_states_up_to_d() {
        let mut a = app();
        a.load(1); // Init: A and B
        assert_eq!(a.s.n_states.load(Ordering::Relaxed), 2);
        a.s.morph.set(1.0);
        a.edit_param(0, 20);
        let b0 = a.s.states[1][0].get();
        a.store_state(3);
        assert_eq!(a.s.n_states.load(Ordering::Relaxed), 4, "C filled in from B, D stored");
        assert_eq!(a.s.states[2][0].get(), b0);
        let p = a.current_patch();
        assert_eq!(p.states.len(), 4);
        let back = Patch::from_json(&p.to_json()).unwrap();
        assert_eq!(back.states.len(), 4, "states survive a save round trip");
        assert!(compile(&back, 48_000.0).is_ok());
    }

    #[test]
    fn macros_land_on_named_knobs_and_moments_restore_the_sound() {
        let mut a = app();
        assert!(a.play_column().is_some(), "opens on the play view");
        a.tick(&Input { knob1: 5, ..Default::default() });
        assert!(a.s.macros[0].get() > 0.0, "knob 1 is CHARACTER");
        let snap = a.kit_snapshot();
        let m0 = a.s.macros[0].get();
        a.edit_control(C_PRESET, 1);
        assert_eq!(a.preset, 1);
        a.kit_recall(&snap);
        assert_eq!(a.preset, 0, "the moment brings its preset back");
        assert!((a.s.macros[0].get() - m0).abs() < 1e-6);
    }

    #[test]
    fn the_cpu_budget_caps_polyphony() {
        let mut a = app();
        a.load(0);
        let mut p = a.audio_processor().unwrap();
        a.s.set_b(B::Budget, a.cost_global + a.cost_voice * 2.5);
        a.tick(&pads(&[0, 1, 2, 3, 4, 5]));
        render(&mut p, 10);
        assert_eq!(a.s.voice_fit.load(Ordering::Relaxed), 2);
        assert!(a.s.active_voices.load(Ordering::Relaxed) <= 2);
    }

    #[test]
    fn depth_reveals_more_rows() {
        let mut a = app();
        let play = a.rows().len();
        a.depth = 1;
        let edit = a.rows().len();
        a.depth = 2;
        let deep = a.rows().len();
        assert!(play < edit && edit < deep, "{play} {edit} {deep}");
        assert!(a.rows().iter().any(|r| matches!(r, Row::Node(_))));
    }

    /// Level / cost report for tuning presets:
    /// `cargo test --bin portamax-sim atlas::tests::preset_report -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preset_report() {
        let mut a = app();
        for idx in 0..FACTORY.len() {
            a.load(idx);
            let mut p = a.audio_processor().unwrap();
            a.tick(&pads(&[12, 14, 9]));
            let mut peak = 0.0f32;
            let mut e = 0.0;
            for _ in 0..90 {
                let mut out = vec![0.0f32; 1024];
                p.process(&mut out, 2, 48_000.0);
                peak = out.iter().fold(peak, |m, v| m.max(v.abs()));
                e += out.iter().map(|v| v * v).sum::<f32>();
            }
            let rms = (e / (90.0 * 1024.0)).sqrt();
            println!("{:16} rms {:.3} peak {:.3}  cost {:.0} + {:.0}/voice  poly {}", FACTORY[idx].0, rms, peak, a.cost_global, a.cost_voice, a.poly);
            a.tick(&Input::default());
        }
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, id: &str) -> Box<dyn crate::app::App> {
    // A manifest with a `data` patch is a cartridge: that sound, as its own app.
    if let Some(m) = ctx.try_named::<crate::manifest::AppManifest>("manifest") {
        if let Some(path) = m.data_path() {
            return Box::new(AtlasApp::cartridge(&m.name, id, path, ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()));
        }
    }
    Box::new(AtlasApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}
