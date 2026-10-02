//! Pulsar -- a one-press beat machine.
//!
//! Pick a genre (30 + "???"), press Generate, get a complete, humanized,
//! genre-correct drum pattern and kit. Lock the lanes you love and keep
//! regenerating the rest. Then shape it: 8 lanes of synthesized drums or
//! your own samples, swing and humanize, rolls and ghost notes, 8 pattern
//! slots with chaining and automatic fills, per-step velocity / chance /
//! roll / nudge, a fatten-glue-filter-room master chain, finger drumming
//! with live recording, and MIDI / WAV / stem export.
//!
//! Controls (Pulsar opens on the shared play view, see play_kit.rs):
//!   knobs  DJ filter / swing, fatten / room, the selected lane's decay /
//!          tone, tempo / glue (press knob1 for the next pair)
//!   D-pad  up/down picks the pattern slot (switches at the bar line)
//!   stick  X sweeps the DJ filter, Y opens the room; hands push fatten
//!          and swing -- all spring back on release
//!   R1     the full menu, where:
//!     knob1  browse menu      press: open/close group (on a lane row: lock lane)
//!     knob2  edit value       press: reset / run the selected action
//!   F3     Start / Stop
//!   F2     pad layers: PERFORM, STEPS, SLOTS (the old pad modes, also set
//!          on Pads & Slots > Pads), then Controls, Moments, Throws
//!   pads   on Pulsar's own layers:
//!          Perform  top 2 rows play lanes 1-8 (records when Record is on),
//!                   bottom 2 rows mute/unmute lanes 1-8
//!          Steps    16 steps of the selected lane in the shown bar
//!          Slots    top 2 rows pick pattern slots A-H (switches at the
//!                   bar line), bottom 2 rows regenerate lanes 1-8

pub mod engine;
pub mod export;
pub mod genres;
pub mod samples;

use crate::app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes, Throw};
use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::{ParamList, ACCENT};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16};
use crate::util::{accelerate, AtomicF32};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use engine::{Engine, LaneSettings, Model, Pattern, SampleBuf, Settings, Step, BANK, FILL, LANES, LANE_NAMES, LANE_SHORT, MAX_STEPS, MODELS, SLOTS};
use genres::{GenParams, Rng, GENRES};
use samples::SampleFile;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const APP_NAME: &str = "Pulsar";
const SLOT_NAMES: [&str; SLOTS] = ["A", "B", "C", "D", "E", "F", "G", "H"];
const LENGTHS: [usize; 4] = [16, 32, 48, 64];
const FILL_EVERY: [u32; 4] = [0, 4, 8, 16];
const UNDO_MAX: usize = 30;

// ------------------------------------------------------------ shared

struct LaneAtoms {
    model: AtomicU32,
    /// index into the sample list, -1 = synth
    sample: AtomicI32,
    level: AtomicF32,
    pan: AtomicF32,
    tune: AtomicF32,
    decay: AtomicF32,
    tone: AtomicF32,
    punch: AtomicF32,
    send: AtomicF32,
    mute: AtomicBool,
    solo: AtomicBool,
    chance: AtomicF32,
}

impl LaneAtoms {
    fn new() -> Self {
        Self {
            model: AtomicU32::new(0),
            sample: AtomicI32::new(-1),
            level: AtomicF32::new(0.8),
            pan: AtomicF32::new(0.0),
            tune: AtomicF32::new(0.0),
            decay: AtomicF32::new(0.5),
            tone: AtomicF32::new(0.5),
            punch: AtomicF32::new(0.5),
            send: AtomicF32::new(0.0),
            mute: AtomicBool::new(false),
            solo: AtomicBool::new(false),
            chance: AtomicF32::new(1.0),
        }
    }

    fn get(&self) -> LaneSettings {
        LaneSettings {
            model: Model::from_index(self.model.load(Ordering::Relaxed) as usize),
            use_sample: self.sample.load(Ordering::Relaxed) >= 0,
            level: self.level.get(),
            pan: self.pan.get(),
            tune: self.tune.get(),
            decay: self.decay.get(),
            tone: self.tone.get(),
            punch: self.punch.get(),
            send: self.send.get(),
            mute: self.mute.load(Ordering::Relaxed),
            solo: self.solo.load(Ordering::Relaxed),
            chance: self.chance.get(),
        }
    }

    fn set(&self, l: &LaneSettings) {
        self.model.store(l.model.index() as u32, Ordering::Relaxed);
        self.level.set(l.level);
        self.pan.set(l.pan);
        self.tune.set(l.tune);
        self.decay.set(l.decay);
        self.tone.set(l.tone);
        self.punch.set(l.punch);
        self.send.set(l.send);
        self.mute.store(l.mute, Ordering::Relaxed);
        self.solo.store(l.solo, Ordering::Relaxed);
        self.chance.set(l.chance);
    }
}

struct Shared {
    lanes: [LaneAtoms; LANES],
    bpm: AtomicF32,
    swing: AtomicF32,
    swing8: AtomicBool,
    human_time: AtomicF32,
    human_vel: AtomicF32,
    fatten: AtomicF32,
    tone_shift: AtomicF32,
    glue: AtomicF32,
    filter: AtomicF32,
    room: AtomicF32,
    level: AtomicF32,
    playing: AtomicBool,
    slot: AtomicUsize,
    chain: AtomicBool,
    fill_every: AtomicU32,
    audition: [AtomicU8; LANES],
    bank: Mutex<Box<[Pattern; BANK]>>,
    bank_version: AtomicU32,
    samples: Mutex<[Option<Arc<SampleBuf>>; LANES]>,
    samples_version: AtomicU32,
    ext_density: Arc<AtomicF32>,
    ext_filter: Arc<AtomicF32>,
    ext_fatten: Arc<AtomicF32>,
    ext_swing: Arc<AtomicF32>,
    ext_lane_level: [Arc<AtomicF32>; LANES],
    // telemetry
    step: AtomicUsize,
    bar: AtomicU32,
    playing_slot: AtomicUsize,
    in_fill: AtomicBool,
    flashes: [AtomicF32; LANES],
    peak: AtomicF32,
    load: AtomicF32,
    sample_rate: AtomicU32,
    bus_out: Arc<Mutex<Vec<f32>>>,
    kick_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Shared {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register(APP_NAME, modbus);
        Self {
            lanes: std::array::from_fn(|_| LaneAtoms::new()),
            bpm: AtomicF32::new(124.0),
            swing: AtomicF32::new(0.0),
            swing8: AtomicBool::new(false),
            human_time: AtomicF32::new(0.15),
            human_vel: AtomicF32::new(0.2),
            fatten: AtomicF32::new(0.2),
            tone_shift: AtomicF32::new(0.0),
            glue: AtomicF32::new(0.2),
            filter: AtomicF32::new(0.0),
            room: AtomicF32::new(0.3),
            level: AtomicF32::new(0.85),
            playing: AtomicBool::new(false),
            slot: AtomicUsize::new(0),
            chain: AtomicBool::new(false),
            fill_every: AtomicU32::new(0),
            audition: std::array::from_fn(|_| AtomicU8::new(0)),
            bank: Mutex::new(Box::new([Pattern::default(); BANK])),
            bank_version: AtomicU32::new(1),
            samples: Mutex::new(Default::default()),
            samples_version: AtomicU32::new(1),
            ext_density: modbus.register(format!("{APP_NAME}: Lane Chance")),
            ext_filter: modbus.register(format!("{APP_NAME}: DJ Filter")),
            ext_fatten: modbus.register(format!("{APP_NAME}: Fatten")),
            ext_swing: modbus.register(format!("{APP_NAME}: Swing")),
            ext_lane_level: std::array::from_fn(|i| modbus.register(format!("{APP_NAME}: {} Level", LANE_NAMES[i]))),
            step: AtomicUsize::new(0),
            bar: AtomicU32::new(0),
            playing_slot: AtomicUsize::new(0),
            in_fill: AtomicBool::new(false),
            flashes: std::array::from_fn(|_| AtomicF32::new(0.0)),
            peak: AtomicF32::new(0.0),
            load: AtomicF32::new(0.0),
            sample_rate: AtomicU32::new(48_000f32.to_bits()),
            bus_out: audio_bus.register(APP_NAME),
            kick_out: audio_bus.register(format!("{APP_NAME} Kick")),
            mix_level,
            ext_mix_level,
        }
    }

    /// Everything the engine reads, as plain values (with modulation).
    fn settings(&self, with_audition: bool) -> Settings {
        let mut lanes: [LaneSettings; LANES] = std::array::from_fn(|i| self.lanes[i].get());
        for (i, l) in lanes.iter_mut().enumerate() {
            l.level = (l.level + self.ext_lane_level[i].get()).clamp(0.0, 1.5);
            l.chance = (l.chance + self.ext_density.get()).clamp(0.0, 1.0);
        }
        Settings {
            lanes,
            bpm: self.bpm.get(),
            swing: (self.swing.get() + self.ext_swing.get()).clamp(0.0, 1.0),
            swing8: self.swing8.load(Ordering::Relaxed),
            human_time: self.human_time.get(),
            human_vel: self.human_vel.get(),
            fatten: (self.fatten.get() + self.ext_fatten.get()).clamp(0.0, 1.0),
            tone_shift: self.tone_shift.get(),
            glue: self.glue.get(),
            filter: (self.filter.get() + self.ext_filter.get()).clamp(-1.0, 1.0),
            room: self.room.get(),
            level: self.level.get(),
            playing: self.playing.load(Ordering::Relaxed),
            slot: self.slot.load(Ordering::Relaxed),
            chain: self.chain.load(Ordering::Relaxed),
            fill_every: self.fill_every.load(Ordering::Relaxed),
            audition: if with_audition { std::array::from_fn(|i| self.audition[i].swap(0, Ordering::Relaxed)) } else { [0; LANES] },
        }
    }
}

// --------------------------------------------------------- processor

struct PulsarProcessor {
    shared: Arc<Shared>,
    engine: Engine,
    bank: Box<[Pattern; BANK]>,
    bank_version: u32,
    samples: [Option<Arc<SampleBuf>>; LANES],
    samples_version: u32,
    l: Vec<f32>,
    r: Vec<f32>,
    mono: Vec<f32>,
}

impl AudioProcessor for PulsarProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let start = Instant::now();
        let channels = channels.max(1);
        let frames = buffer.len() / channels;
        let s = &self.shared;
        s.sample_rate.store(sample_rate.to_bits(), Ordering::Relaxed);

        // pick up edits (copy, never block)
        let v = s.bank_version.load(Ordering::Acquire);
        if v != self.bank_version {
            if let Ok(b) = s.bank.try_lock() {
                *self.bank = **b;
                self.bank_version = v;
            }
        }
        let sv = s.samples_version.load(Ordering::Acquire);
        if sv != self.samples_version {
            if let Ok(smp) = s.samples.try_lock() {
                for i in 0..LANES {
                    // The UI keeps its own reference to every buffer it hands
                    // over, so dropping ours here never frees memory on the
                    // audio thread.
                    self.samples[i] = smp[i].clone();
                }
                self.samples_version = sv;
            }
        }

        for v in [&mut self.l, &mut self.r, &mut self.mono] {
            v.clear();
            v.resize(frames, 0.0);
        }
        let settings = s.settings(true);
        let rep = self.engine.process(&settings, &self.bank, &self.samples, &mut self.l, &mut self.r, sample_rate);

        s.step.store(rep.step, Ordering::Relaxed);
        s.bar.store(rep.bar, Ordering::Relaxed);
        s.playing_slot.store(rep.slot, Ordering::Relaxed);
        s.in_fill.store(rep.in_fill, Ordering::Relaxed);
        for i in 0..LANES {
            s.flashes[i].set(rep.flashes[i].max(s.flashes[i].get() * 0.8));
        }
        s.peak.set(rep.peak.max(s.peak.get() * 0.85));

        for i in 0..frames {
            self.mono[i] = (self.l[i] + self.r[i]) * 0.5;
        }
        if let Ok(mut b) = s.bus_out.try_lock() {
            b.clear();
            b.extend_from_slice(&self.mono);
        }
        if let Ok(mut b) = s.kick_out.try_lock() {
            b.clear();
            b.extend_from_slice(&self.engine.kick_tap);
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
        s.load.set(start.elapsed().as_secs_f32() / (frames as f32 / sample_rate.max(1.0)).max(1e-6));
    }
}

// ------------------------------------------------------------- menu

#[derive(Clone, Copy, PartialEq, Debug)]
enum PadMode {
    Perform,
    Steps,
    Slots,
}

impl PadMode {
    /// The kit's native layer id for each pad mode (F2 order).
    fn layer(self) -> u8 {
        match self {
            PadMode::Perform => 0,
            PadMode::Steps => 1,
            PadMode::Slots => 2,
        }
    }

    fn from_layer(id: u8) -> PadMode {
        match id {
            1 => PadMode::Steps,
            2 => PadMode::Slots,
            _ => PadMode::Perform,
        }
    }

    fn next(self) -> PadMode {
        match self {
            PadMode::Perform => PadMode::Steps,
            PadMode::Steps => PadMode::Slots,
            PadMode::Slots => PadMode::Perform,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Sel {
    Genre,
    Complexity,
    Density,
    Variation,
    Ghosts,
    Rolls,
    Length,
    KitFollows,
    Generate,
    NewKit,
    RegenLane,
    MakeFill,
    Tempo,
    Swing,
    SwingGrid,
    HumanTime,
    HumanVel,
    AutoFill,
    Chain,
    Lane,
    Lock,
    Mute,
    Solo,
    Sound,
    Level,
    Pan,
    Tune,
    Decay,
    Tone,
    Punch,
    Send,
    Chance,
    ClearLane,
    RandSound,
    Cursor,
    StepVel,
    StepChance,
    StepRoll,
    StepNudge,
    StepToggle,
    Fatten,
    ToneShift,
    Glue,
    Filter,
    Room,
    Master,
    Pads,
    Record,
    Slot,
    CopySlot,
    ClearSlot,
    Undo,
    RandAmount,
    RandSounds,
    RandGroove,
    RandAll,
    Mutate,
    SaveBeat,
    Beat(usize),
    ExportMidi,
    ExportWav,
    ExportStems,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Group(usize),
    Leaf(Sel),
}

const GROUPS: [&str; 8] = ["Generate", "Groove", "Lane", "Step Edit", "Mix FX", "Pads & Slots", "Randomize", "Library & Export"];

pub struct PulsarApp {
    shared: Arc<Shared>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    genre: usize,
    gen: GenParams,
    kit_follows: bool,
    locks: [bool; LANES],
    lane: usize,
    cursor: usize,
    pad_mode: PadMode,
    record: bool,
    rand_amount: f32,
    rng: Rng,
    undo: Vec<Box<[Pattern; BANK]>>,
    sample_list: Vec<SampleFile>,
    lane_samples: [Vec<usize>; LANES],
    loaded: [Option<Arc<SampleBuf>>; LANES],
    graveyard: Vec<Arc<SampleBuf>>,
    beats: Vec<(String, PathBuf)>,
    list: ParamList,
    expanded: [bool; 8],
    prev_grid: [bool; 16],
    status: String,
    beat_name: String,
    /// The shared play view (play_kit.rs).
    kit: PlayKit,
}

/// The play view's controls, most important first. The master chain and
/// groove are what a drum machine is performed with, so they take the
/// knobs; 4/5 and 8-11 follow the selected lane (a Perform pad hit
/// selects its lane), so hitting the snare and turning knob 1 shapes the
/// snare. Slot and genre close the list for the D-pad and the pads.
const CONTROLS: [(Sel, &str); 16] = [
    (Sel::Filter, "DJ Filter"),
    (Sel::Swing, "Swing"),
    (Sel::Fatten, "Fatten"),
    (Sel::Room, "Room"),
    (Sel::Decay, "Decay"),
    (Sel::Tone, "Tone"),
    (Sel::Tempo, "Tempo"),
    (Sel::Glue, "Glue"),
    (Sel::Tune, "Tune"),
    (Sel::Punch, "Punch"),
    (Sel::Level, "Level"),
    (Sel::Chance, "Chance"),
    (Sel::HumanTime, "Human Time"),
    (Sel::HumanVel, "Human Vel"),
    (Sel::Slot, "Slot"),
    (Sel::Genre, "Genre"),
];
const C_FILTER: usize = 0;
const C_SWING: usize = 1;
const C_FATTEN: usize = 2;
const C_ROOM: usize = 3;
const C_GLUE: usize = 7;
const C_SLOT: usize = 14;
const C_GENRE: usize = 15;

/// Controls that act on the selected lane rather than the whole kit.
fn lane_control(i: usize) -> bool {
    matches!(i, 4 | 5 | 8..=11)
}

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "pulsar",
        // The old pad modes are Pulsar's own layers, so F2 reaches them
        // exactly as before, then the shared ones.
        layers: vec![
            Layer::Native(0, "PERFORM"),
            Layer::Native(1, "STEPS"),
            Layer::Native(2, "SLOTS"),
            Layer::Controls,
            Layer::Moments,
            Layer::Throws,
        ],
        hero: vec![[0, 1], [2, 3], [4, 5], [6, 7]],
        // D-pad up/down = next/previous pattern slot: the song-arranging
        // move, and audible at the next bar line (genre only changes what
        // the next Generate makes).
        browse: Some(C_SLOT),
        // The DJ filter on stick X is the classic groovebox sweep (centre
        // = off, left = low-pass, right = high-pass); Y opens the room.
        // Hands: drive (fatten) and push the swing. All kit-wide, never
        // per-lane, so changing lanes mid-gesture can't strand an offset.
        routes: Routes { stick_x: Some(C_FILTER), stick_y: Some(C_ROOM), hand_l: Some(C_FATTEN), hand_r: Some(C_SWING) },
        // The master chain is effectively an effect over the beat, so it
        // gets the effects' momentary moves.
        throws: vec![
            // Filter -1..1: 0.1 = LP 80%, 0.9 = HP 80%.
            Throw { control: C_FILTER, to: 0.1, label: "LP SWEEP" },
            Throw { control: C_FILTER, to: 0.9, label: "HP SWEEP" },
            Throw { control: C_ROOM, to: 1.0, label: "ROOM WASH" },
            Throw { control: C_ROOM, to: 0.0, label: "DRY" },
            Throw { control: C_FATTEN, to: 1.0, label: "FATTEN" },
            Throw { control: C_GLUE, to: 1.0, label: "SQUASH" },
            Throw { control: C_SWING, to: 1.0, label: "MAX SWING" },
        ],
        midi_to_pads: true,
        own_expression: false,
    }
}

impl PulsarApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let shared = Arc::new(Shared::new(&modbus, &audio_bus, &mixer_bus));
        let sample_list = samples::scan(&samples::samples_dir());
        let lane_samples = std::array::from_fn(|l| samples::for_lane(&sample_list, l));
        let mut app = Self {
            shared,
            sensitivity,
            nav_speed,
            genre: 0,
            gen: GenParams::default(),
            kit_follows: true,
            locks: [false; LANES],
            lane: 0,
            cursor: 0,
            pad_mode: PadMode::Perform,
            record: false,
            rand_amount: 0.4,
            rng: Rng::seeded(),
            undo: Vec::new(),
            sample_list,
            lane_samples,
            loaded: Default::default(),
            graveyard: Vec::new(),
            beats: export::list_beats(),
            list: ParamList::new(),
            expanded: [true, false, false, false, false, false, false, false],
            prev_grid: [false; 16],
            status: String::new(),
            beat_name: String::new(),
            kit: PlayKit::new(kit_config(), !cfg!(test)),
        };
        app.generate(true);
        app.status = format!("{} beat ready. F3 to play, Generate for another.", GENRES[app.genre].name);
        app
    }

    // ----------------------------------------------------------- bank

    fn bank(&self) -> Box<[Pattern; BANK]> {
        self.shared.bank.lock().map(|b| b.clone()).unwrap_or_else(|_| Box::new([Pattern::default(); BANK]))
    }

    fn edit_bank(&mut self, f: impl FnOnce(&mut [Pattern; BANK])) {
        if let Ok(mut b) = self.shared.bank.lock() {
            f(&mut b);
        }
        self.shared.bank_version.fetch_add(1, Ordering::Release);
    }

    fn push_undo(&mut self) {
        let b = self.bank();
        self.undo.push(b);
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
    }

    fn slot(&self) -> usize {
        self.shared.slot.load(Ordering::Relaxed)
    }

    fn pattern(&self) -> Pattern {
        self.bank()[self.slot()]
    }

    /// Bar shown in the grid: follows the playhead while playing, the
    /// cursor while editing.
    fn view_bar(&self) -> usize {
        let p = self.pattern();
        let bars = (p.length / 16).max(1);
        if self.shared.playing.load(Ordering::Relaxed) && self.pad_mode != PadMode::Steps && self.shared.playing_slot.load(Ordering::Relaxed) == self.slot() {
            (self.shared.step.load(Ordering::Relaxed) / 16).min(bars - 1)
        } else {
            (self.cursor / 16).min(bars - 1)
        }
    }

    // ------------------------------------------------------ generation

    fn genre_bpm(&self) -> f32 {
        let g = &GENRES[self.genre];
        if g.bpm > 0.0 {
            g.bpm
        } else {
            80.0 + self.rng_peek() * 90.0
        }
    }

    fn rng_peek(&self) -> f32 {
        Rng::new(self.rng.0 ^ 0xABCD).next()
    }

    fn generate(&mut self, with_kit: bool) {
        self.push_undo();
        let g = &GENRES[self.genre];
        let slot = self.slot();
        let cur = self.bank()[slot];
        let p = genres::generate(g, &self.gen, &self.locks, &cur, &mut self.rng);
        let fill = genres::fill_bar(g, &p, self.gen.complexity, &mut self.rng);
        self.edit_bank(|b| {
            b[slot] = p;
            b[FILL] = fill;
        });
        if with_kit && self.kit_follows {
            self.apply_genre_kit();
        }
        let n: usize = (0..LANES).map(|l| p.hits(l)).sum();
        self.status = format!("{} pattern in slot {} ({} bars, {n} hits)", g.name, SLOT_NAMES[slot], p.length / 16);
    }

    fn apply_genre_kit(&mut self) {
        let g = &GENRES[self.genre];
        let mut lanes: [LaneSettings; LANES] = std::array::from_fn(|i| self.shared.lanes[i].get());
        genres::apply_kit(g, &mut lanes, &self.locks);
        if self.genre == genres::chaos_index() {
            for (i, l) in lanes.iter_mut().enumerate() {
                if !self.locks[i] {
                    l.model = MODELS[self.rng.range(MODELS.len())];
                    l.tune = (self.rng.next() - 0.5) * 14.0;
                    l.decay = self.rng.next();
                    l.tone = self.rng.next();
                    l.punch = self.rng.next();
                }
            }
        }
        for i in 0..LANES {
            if !self.locks[i] {
                self.shared.lanes[i].set(&lanes[i]);
                self.set_sample(i, None);
            }
        }
        self.shared.bpm.set(self.genre_bpm().round());
        self.shared.swing.set(g.swing);
        self.shared.swing8.store(g.swing8, Ordering::Relaxed);
    }

    fn regen_lane(&mut self, lane: usize) {
        if self.locks[lane] {
            self.status = format!("{} is locked", LANE_NAMES[lane]);
            return;
        }
        self.push_undo();
        let mut locks = [true; LANES];
        locks[lane] = false;
        let slot = self.slot();
        let cur = self.bank()[slot];
        let p = genres::generate(&GENRES[self.genre], &GenParams { length: cur.length, ..self.gen }, &locks, &cur, &mut self.rng);
        self.edit_bank(|b| b[slot].steps[lane] = p.steps[lane]);
        self.status = format!("New {} part", LANE_NAMES[lane]);
    }

    fn make_fill(&mut self) {
        let p = self.pattern();
        let fill = genres::fill_bar(&GENRES[self.genre], &p, self.gen.complexity, &mut self.rng);
        self.edit_bank(|b| b[FILL] = fill);
        if self.shared.fill_every.load(Ordering::Relaxed) == 0 {
            self.shared.fill_every.store(4, Ordering::Relaxed);
        }
        self.status = "New fill -- plays on the last bar of every 4 (Groove > Auto Fill)".into();
    }

    // ------------------------------------------------------- samples

    fn set_sample(&mut self, lane: usize, idx: Option<usize>) {
        let buf = match idx.and_then(|i| self.sample_list.get(i)) {
            Some(f) => match samples::load(&f.path) {
                Ok(b) => Some(Arc::new(b)),
                Err(e) => {
                    self.status = format!("Sample failed: {e}");
                    None
                }
            },
            None => None,
        };
        self.shared.lanes[lane].sample.store(if buf.is_some() { idx.map_or(-1, |i| i as i32) } else { -1 }, Ordering::Relaxed);
        if let Ok(mut s) = self.shared.samples.lock() {
            s[lane] = buf.clone();
        }
        if let Some(old) = std::mem::replace(&mut self.loaded[lane], buf) {
            self.graveyard.push(old);
        }
        self.shared.samples_version.fetch_add(1, Ordering::Release);
    }

    /// Sound choices for a lane: every synth model, then matching samples.
    fn sound_count(&self, lane: usize) -> usize {
        MODELS.len() + self.lane_samples[lane].len()
    }

    fn sound_index(&self, lane: usize) -> usize {
        let la = &self.shared.lanes[lane];
        let s = la.sample.load(Ordering::Relaxed);
        if s >= 0 {
            MODELS.len() + self.lane_samples[lane].iter().position(|&i| i as i32 == s).unwrap_or(0)
        } else {
            la.model.load(Ordering::Relaxed) as usize
        }
    }

    fn set_sound(&mut self, lane: usize, k: usize) {
        if k < MODELS.len() {
            self.shared.lanes[lane].model.store(k as u32, Ordering::Relaxed);
            self.set_sample(lane, None);
        } else if let Some(&si) = self.lane_samples[lane].get(k - MODELS.len()) {
            self.set_sample(lane, Some(si));
        }
    }

    fn sound_name(&self, lane: usize) -> String {
        let k = self.sound_index(lane);
        if k < MODELS.len() {
            MODELS[k].name().to_string()
        } else {
            self.loaded[lane].as_ref().map(|b| format!("~{}", b.name)).unwrap_or_else(|| "sample".into())
        }
    }

    // ---------------------------------------------------- randomize

    fn rand_sounds(&mut self, lane_only: Option<usize>) {
        let a = self.rand_amount;
        for i in 0..LANES {
            if self.locks[i] || lane_only.is_some_and(|l| l != i) {
                continue;
            }
            let la = &self.shared.lanes[i];
            let nudge = |rng: &mut Rng, v: f32, lo: f32, hi: f32| (v + (rng.next() - 0.5) * a * (hi - lo)).clamp(lo, hi);
            la.tune.set(nudge(&mut self.rng, la.tune.get(), -12.0, 12.0));
            la.decay.set(nudge(&mut self.rng, la.decay.get(), 0.0, 1.0));
            la.tone.set(nudge(&mut self.rng, la.tone.get(), 0.0, 1.0));
            la.punch.set(nudge(&mut self.rng, la.punch.get(), 0.0, 1.0));
            la.pan.set(nudge(&mut self.rng, la.pan.get(), -0.6, 0.6));
            if self.rng.next() < a * 0.5 {
                let n = self.sound_count(i);
                if self.rng.next() < 0.5 && n > MODELS.len() {
                    let k = MODELS.len() + self.rng.range(n - MODELS.len());
                    self.set_sound(i, k);
                } else if let Some(m) = genres::GENRES.get(self.rng.range(GENRES.len() - 1)).map(|g| g.kit[i].model) {
                    self.shared.lanes[i].model.store(m.index() as u32, Ordering::Relaxed);
                    self.set_sample(i, None);
                }
            }
        }
        self.status = "Sounds randomized (locked lanes kept)".into();
    }

    fn rand_groove(&mut self) {
        let a = self.rand_amount;
        let s = &self.shared;
        let nudge = |rng: &mut Rng, v: f32| (v + (rng.next() - 0.5) * a).clamp(0.0, 1.0);
        s.swing.set(nudge(&mut self.rng, s.swing.get()));
        s.human_time.set(nudge(&mut self.rng, s.human_time.get()));
        s.human_vel.set(nudge(&mut self.rng, s.human_vel.get()));
        self.gen.complexity = nudge(&mut self.rng, self.gen.complexity);
        self.gen.density = nudge(&mut self.rng, self.gen.density);
        self.status = "Groove randomized".into();
    }

    fn mutate(&mut self) {
        self.push_undo();
        let slot = self.slot();
        let a = self.rand_amount;
        let mut p = self.pattern();
        let mut changed = 0;
        for lane in 0..LANES {
            if self.locks[lane] {
                continue;
            }
            for i in 0..p.length {
                if self.rng.next() < a * 0.08 {
                    let st = &mut p.steps[lane][i];
                    if st.on() && !(lane == 0 && i % 4 == 0) {
                        *st = Step::OFF;
                    } else if !st.on() {
                        *st = Step::hit(60 + self.rng.range(60) as u8);
                    }
                    changed += 1;
                } else if p.steps[lane][i].on() && self.rng.next() < a * 0.2 {
                    let v = p.steps[lane][i].vel as i32 + ((self.rng.next() - 0.5) * 40.0) as i32;
                    p.steps[lane][i].vel = v.clamp(20, 127) as u8;
                }
            }
        }
        self.edit_bank(|b| b[slot] = p);
        self.status = format!("Mutated {changed} steps");
    }

    // --------------------------------------------------- export / save

    fn beat_file(&self) -> export::BeatFile {
        let s = &self.shared;
        let bank = self.bank();
        export::BeatFile {
            name: if self.beat_name.is_empty() { format!("{} {}", GENRES[self.genre].name, s.bpm.get().round()) } else { self.beat_name.clone() },
            genre: GENRES[self.genre].name.into(),
            bpm: s.bpm.get(),
            swing: s.swing.get(),
            swing8: s.swing8.load(Ordering::Relaxed),
            human_time: s.human_time.get(),
            human_vel: s.human_vel.get(),
            fatten: s.fatten.get(),
            tone_shift: s.tone_shift.get(),
            glue: s.glue.get(),
            filter: s.filter.get(),
            room: s.room.get(),
            level: s.level.get(),
            lanes: (0..LANES)
                .map(|i| {
                    let l = s.lanes[i].get();
                    export::LaneFile {
                        model: l.model.name().into(),
                        sample: self.loaded[i].as_ref().and_then(|_| {
                            let si = s.lanes[i].sample.load(Ordering::Relaxed);
                            self.sample_list.get(si.max(0) as usize).map(|f| f.path.strip_prefix(samples::samples_dir()).unwrap_or(&f.path).to_string_lossy().to_string())
                        }),
                        level: l.level,
                        pan: l.pan,
                        tune: l.tune,
                        decay: l.decay,
                        tone: l.tone,
                        punch: l.punch,
                        send: l.send,
                        mute: l.mute,
                        chance: l.chance,
                    }
                })
                .collect(),
            slots: bank[..SLOTS].iter().map(export::pattern_to_file).collect(),
        }
    }

    fn load_beat_file(&mut self, b: &export::BeatFile) {
        self.push_undo();
        let s = &self.shared;
        s.bpm.set(b.bpm);
        s.swing.set(b.swing);
        s.swing8.store(b.swing8, Ordering::Relaxed);
        s.human_time.set(b.human_time);
        s.human_vel.set(b.human_vel);
        s.fatten.set(b.fatten);
        s.tone_shift.set(b.tone_shift);
        s.glue.set(b.glue);
        s.filter.set(b.filter);
        s.room.set(b.room);
        s.level.set(b.level);
        if let Some(i) = GENRES.iter().position(|g| g.name == b.genre) {
            self.genre = i;
        }
        for (i, lf) in b.lanes.iter().enumerate().take(LANES) {
            let l = LaneSettings {
                model: export::model_from_name(&lf.model),
                use_sample: false,
                level: lf.level,
                pan: lf.pan,
                tune: lf.tune,
                decay: lf.decay,
                tone: lf.tone,
                punch: lf.punch,
                send: lf.send,
                mute: lf.mute,
                solo: false,
                chance: lf.chance,
            };
            self.shared.lanes[i].set(&l);
            let idx = lf.sample.as_ref().and_then(|rel| self.sample_list.iter().position(|f| f.path.ends_with(rel)));
            self.set_sample(i, idx);
        }
        let mut bank = [Pattern::default(); BANK];
        for (k, pf) in b.slots.iter().enumerate().take(SLOTS) {
            bank[k] = export::pattern_from_file(pf);
        }
        let fill = genres::fill_bar(&GENRES[self.genre], &bank[0], self.gen.complexity, &mut self.rng);
        bank[FILL] = fill;
        self.edit_bank(|b| *b = bank);
        self.shared.slot.store(0, Ordering::Relaxed);
        self.beat_name = b.name.clone();
        self.status = format!("Loaded \"{}\"", b.name);
    }

    fn save_beat(&mut self) {
        let b = self.beat_file();
        match export::save_beat(&b) {
            Ok(p) => {
                self.beats = export::list_beats();
                self.status = format!("Saved apps/pulsar/beats/{}", p.file_name().and_then(|f| f.to_str()).unwrap_or(""));
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    fn export(&mut self, what: Sel) {
        let dir = export::export_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.status = format!("Export failed: {e}");
            return;
        }
        let settings = self.shared.settings(false);
        let bank = self.bank();
        let slot = self.slot();
        let base = format!("{}_{}", export::slug(&self.beat_file().name), SLOT_NAMES[slot].to_lowercase());
        let sr = 48_000u32;
        let loops = (4 * 16 / bank[slot].length.max(16)).max(1);
        let result: Result<String, String> = match what {
            Sel::ExportMidi => {
                let path = export::fresh_path(&dir, &base, "mid");
                std::fs::write(&path, export::midi_bytes(&bank[slot], &settings, loops)).map(|_| path.display().to_string()).map_err(|e| e.to_string())
            }
            Sel::ExportWav => {
                let (l, r) = export::render(&bank, slot, &settings, &self.loaded, loops, None, sr as f32);
                let path = export::fresh_path(&dir, &base, "wav");
                export::write_wav(&path, &l, &r, sr).map(|_| path.display().to_string())
            }
            _ => {
                let mut n = 0;
                let mut err = None;
                for lane in 0..LANES {
                    if bank[slot].hits(lane) == 0 || settings.lanes[lane].mute {
                        continue;
                    }
                    let (l, r) = export::render(&bank, slot, &settings, &self.loaded, loops, Some(lane), sr as f32);
                    let path = export::fresh_path(&dir, &format!("{base}_{}", LANE_SHORT[lane].to_lowercase()), "wav");
                    match export::write_wav(&path, &l, &r, sr) {
                        Ok(()) => n += 1,
                        Err(e) => err = Some(e),
                    }
                }
                match err {
                    Some(e) => Err(e),
                    None => Ok(format!("{n} stems")),
                }
            }
        };
        self.status = match result {
            Ok(p) => format!("Exported {} to exports/pulsar", p.rsplit('/').next().unwrap_or(&p)),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    // ------------------------------------------------------- the menu

    fn group_leaves(&self, g: usize) -> Vec<Sel> {
        use Sel::*;
        match g {
            0 => vec![Genre, Generate, Complexity, Density, Variation, Ghosts, Rolls, Length, KitFollows, NewKit, RegenLane, MakeFill],
            1 => vec![Tempo, Swing, SwingGrid, HumanTime, HumanVel, AutoFill, Chain],
            2 => vec![Lane, Lock, Mute, Solo, Sound, Level, Pan, Tune, Decay, Tone, Punch, Send, Chance, RandSound, ClearLane],
            3 => vec![Lane, Cursor, StepToggle, StepVel, StepChance, StepRoll, StepNudge],
            4 => vec![Fatten, ToneShift, Glue, Filter, Room, Master],
            5 => vec![Pads, Record, Slot, CopySlot, ClearSlot, Undo],
            6 => vec![RandAmount, RandSounds, RandGroove, RandAll, Mutate],
            _ => {
                let mut v = vec![SaveBeat, ExportMidi, ExportWav, ExportStems];
                v.extend((0..self.beats.len()).map(Beat));
                v
            }
        }
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for g in 0..GROUPS.len() {
            rows.push(Row::Group(g));
            if self.expanded[g] {
                rows.extend(self.group_leaves(g).into_iter().map(Row::Leaf));
            }
        }
        rows
    }

    fn leaf_name(&self, sel: Sel) -> String {
        use Sel::*;
        match sel {
            Genre => "Genre".into(),
            Complexity => "Complexity".into(),
            Density => "Density".into(),
            Variation => "Variation".into(),
            Ghosts => "Ghost Notes".into(),
            Rolls => "Rolls".into(),
            Length => "Length".into(),
            KitFollows => "Kit Follows Genre".into(),
            Generate => ">> GENERATE".into(),
            NewKit => ">> New kit only".into(),
            RegenLane => ">> New part for lane".into(),
            MakeFill => ">> Make fill".into(),
            Tempo => "Tempo".into(),
            Swing => "Swing".into(),
            SwingGrid => "Swing Grid".into(),
            HumanTime => "Humanize Time".into(),
            HumanVel => "Humanize Velocity".into(),
            AutoFill => "Auto Fill".into(),
            Chain => "Chain Slots".into(),
            Lane => "Lane".into(),
            Lock => "Lock".into(),
            Mute => "Mute".into(),
            Solo => "Solo".into(),
            Sound => "Sound".into(),
            Level => "Level".into(),
            Pan => "Pan".into(),
            Tune => "Tune".into(),
            Decay => "Decay".into(),
            Tone => "Tone".into(),
            Punch => "Punch".into(),
            Send => "Room Send".into(),
            Chance => "Lane Chance".into(),
            ClearLane => ">> Clear lane".into(),
            RandSound => ">> Randomize sound".into(),
            Cursor => "Step".into(),
            StepVel => "Velocity".into(),
            StepChance => "Chance".into(),
            StepRoll => "Roll".into(),
            StepNudge => "Nudge".into(),
            StepToggle => ">> Toggle step".into(),
            Fatten => "Fatten".into(),
            ToneShift => "Tone Shift".into(),
            Glue => "Glue".into(),
            Filter => "DJ Filter".into(),
            Room => "Room".into(),
            Master => "Master".into(),
            Pads => "Pads".into(),
            Record => "Record".into(),
            Slot => "Slot".into(),
            CopySlot => ">> Copy to next slot".into(),
            ClearSlot => ">> Clear slot".into(),
            Undo => ">> Undo".into(),
            RandAmount => "Amount".into(),
            RandSounds => ">> Randomize sounds".into(),
            RandGroove => ">> Randomize groove".into(),
            RandAll => ">> Randomize everything".into(),
            Mutate => ">> Mutate pattern".into(),
            SaveBeat => ">> Save beat".into(),
            Beat(i) => self.beats.get(i).map(|b| b.0.clone()).unwrap_or_default(),
            ExportMidi => ">> Export MIDI".into(),
            ExportWav => ">> Export WAV".into(),
            ExportStems => ">> Export stems".into(),
        }
    }

    fn pct(v: f32) -> String {
        format!("{:.0}%", v * 100.0)
    }

    fn leaf_value(&self, sel: Sel) -> String {
        use Sel::*;
        let s = &self.shared;
        let la = &s.lanes[self.lane];
        let p = self.pattern();
        let st = p.steps[self.lane][self.cursor.min(MAX_STEPS - 1)];
        match sel {
            Genre => GENRES[self.genre].name.into(),
            Complexity => Self::pct(self.gen.complexity),
            Density => Self::pct(self.gen.density),
            Variation => Self::pct(self.gen.variation),
            Ghosts => Self::pct(self.gen.ghosts),
            Rolls => Self::pct(self.gen.rolls),
            Length => format!("{} bars", self.gen.length / 16),
            KitFollows => (if self.kit_follows { "on" } else { "off" }).into(),
            Generate => "press knob2".into(),
            NewKit | MakeFill | ClearLane | RandSound | StepToggle | CopySlot | ClearSlot | RandSounds | RandGroove | RandAll | Mutate | SaveBeat | ExportMidi | ExportWav | ExportStems => "press".into(),
            RegenLane => LANE_NAMES[self.lane].into(),
            Tempo => format!("{:.0} bpm", s.bpm.get()),
            Swing => format!("{:.0}%", 50.0 + s.swing.get() * 25.0),
            SwingGrid => (if s.swing8.load(Ordering::Relaxed) { "1/8" } else { "1/16" }).into(),
            HumanTime => Self::pct(s.human_time.get()),
            HumanVel => Self::pct(s.human_vel.get()),
            AutoFill => match s.fill_every.load(Ordering::Relaxed) {
                0 => "off".into(),
                n => format!("every {n} bars"),
            },
            Chain => (if s.chain.load(Ordering::Relaxed) { "on" } else { "off" }).into(),
            Lane => format!("{} {}", self.lane + 1, LANE_NAMES[self.lane]),
            Lock => (if self.locks[self.lane] { "locked" } else { "off" }).into(),
            Mute => (if la.mute.load(Ordering::Relaxed) { "muted" } else { "off" }).into(),
            Solo => (if la.solo.load(Ordering::Relaxed) { "solo" } else { "off" }).into(),
            Sound => self.sound_name(self.lane),
            Level => Self::pct(la.level.get()),
            Pan => {
                let v = la.pan.get();
                if v.abs() < 0.02 {
                    "C".into()
                } else if v < 0.0 {
                    format!("L{:.0}", -v * 100.0)
                } else {
                    format!("R{:.0}", v * 100.0)
                }
            }
            Tune => format!("{:+.1} st", la.tune.get()),
            Decay => Self::pct(la.decay.get()),
            Tone => Self::pct(la.tone.get()),
            Punch => Self::pct(la.punch.get()),
            Send => Self::pct(la.send.get()),
            Chance => Self::pct(la.chance.get()),
            Cursor => format!("{} (bar {})", self.cursor % 16 + 1, self.cursor / 16 + 1),
            StepVel => {
                if st.on() {
                    format!("{}", st.vel)
                } else {
                    "off".into()
                }
            }
            StepChance => format!("{}%", st.prob),
            StepRoll => format!("x{}", st.ratchet.max(1)),
            StepNudge => format!("{:+}%", st.micro),
            Fatten => Self::pct(s.fatten.get()),
            ToneShift => format!("{:+.0} st", s.tone_shift.get()),
            Glue => Self::pct(s.glue.get()),
            Filter => {
                let f = s.filter.get();
                if f.abs() < 0.02 {
                    "off".into()
                } else if f < 0.0 {
                    format!("LP {:.0}%", -f * 100.0)
                } else {
                    format!("HP {:.0}%", f * 100.0)
                }
            }
            Room => Self::pct(s.room.get()),
            Master => Self::pct(s.level.get()),
            Pads => match self.pad_mode {
                PadMode::Perform => "Perform",
                PadMode::Steps => "Steps",
                PadMode::Slots => "Slots",
            }
            .into(),
            Record => (if self.record { "REC" } else { "off" }).into(),
            Slot => {
                let playing = s.playing_slot.load(Ordering::Relaxed);
                if playing != self.slot() && s.playing.load(Ordering::Relaxed) {
                    format!("{} (queued)", SLOT_NAMES[self.slot()])
                } else {
                    SLOT_NAMES[self.slot()].into()
                }
            }
            Undo => format!("{} steps", self.undo.len()),
            RandAmount => Self::pct(self.rand_amount),
            Beat(_) => "load".into(),
        }
    }

    fn group_summary(&self, g: usize) -> String {
        let s = &self.shared;
        match g {
            0 => GENRES[self.genre].name.into(),
            1 => format!("{:.0} bpm", s.bpm.get()),
            2 | 3 => LANE_NAMES[self.lane].into(),
            4 => format!("fat {}", Self::pct(s.fatten.get())),
            5 => format!("slot {}", SLOT_NAMES[self.slot()]),
            6 => Self::pct(self.rand_amount),
            _ => format!("{} beats", self.beats.len()),
        }
    }

    fn bump(v: &AtomicF32, delta: i32, sens: f32, lo: f32, hi: f32) {
        v.set((v.get() + accelerate(delta) * sens * (hi - lo) * 0.05).clamp(lo, hi));
    }

    fn bumpf(v: &mut f32, delta: i32, sens: f32) {
        *v = (*v + accelerate(delta) * sens * 0.05).clamp(0.0, 1.0);
    }

    fn edit_step(&mut self, f: impl FnOnce(&mut Step)) {
        let (slot, lane, c) = (self.slot(), self.lane, self.cursor.min(MAX_STEPS - 1));
        self.edit_bank(|b| f(&mut b[slot].steps[lane][c]));
    }

    fn edit(&mut self, sel: Sel, delta: i32) {
        use Sel::*;
        if delta == 0 {
            return;
        }
        let sens = self.sensitivity.get();
        let step = delta.signum();
        let cyc = |cur: usize, n: usize| (cur as i32 + step).rem_euclid(n.max(1) as i32) as usize;
        let s = Arc::clone(&self.shared);
        let la = &s.lanes[self.lane];
        match sel {
            Genre => self.genre = cyc(self.genre, GENRES.len()),
            Complexity => Self::bumpf(&mut self.gen.complexity, delta, sens),
            Density => Self::bumpf(&mut self.gen.density, delta, sens),
            Variation => Self::bumpf(&mut self.gen.variation, delta, sens),
            Ghosts => Self::bumpf(&mut self.gen.ghosts, delta, sens),
            Rolls => Self::bumpf(&mut self.gen.rolls, delta, sens),
            Length => {
                let i = LENGTHS.iter().position(|l| *l == self.gen.length).unwrap_or(1);
                self.gen.length = LENGTHS[(i as i32 + step).clamp(0, 3) as usize];
            }
            KitFollows => self.kit_follows = !self.kit_follows,
            RegenLane | Lane => self.lane = cyc(self.lane, LANES),
            Tempo => s.bpm.set((s.bpm.get() + step as f32 * if delta.abs() > 2 { 5.0 } else { 1.0 }).clamp(40.0, 240.0)),
            Swing => Self::bump(&s.swing, delta, sens, 0.0, 1.0),
            SwingGrid => s.swing8.store(!s.swing8.load(Ordering::Relaxed), Ordering::Relaxed),
            HumanTime => Self::bump(&s.human_time, delta, sens, 0.0, 1.0),
            HumanVel => Self::bump(&s.human_vel, delta, sens, 0.0, 1.0),
            AutoFill => {
                let cur = s.fill_every.load(Ordering::Relaxed);
                let i = FILL_EVERY.iter().position(|f| *f == cur).unwrap_or(0);
                s.fill_every.store(FILL_EVERY[cyc(i, FILL_EVERY.len())], Ordering::Relaxed);
            }
            Chain => s.chain.store(!s.chain.load(Ordering::Relaxed), Ordering::Relaxed),
            Lock => self.locks[self.lane] = !self.locks[self.lane],
            Mute => la.mute.store(!la.mute.load(Ordering::Relaxed), Ordering::Relaxed),
            Solo => la.solo.store(!la.solo.load(Ordering::Relaxed), Ordering::Relaxed),
            Sound => {
                let n = self.sound_count(self.lane);
                let k = cyc(self.sound_index(self.lane), n);
                self.set_sound(self.lane, k);
            }
            Level => Self::bump(&la.level, delta, sens, 0.0, 1.5),
            Pan => Self::bump(&la.pan, delta, sens, -1.0, 1.0),
            Tune => Self::bump(&la.tune, delta, sens, -24.0, 24.0),
            Decay => Self::bump(&la.decay, delta, sens, 0.0, 1.0),
            Tone => Self::bump(&la.tone, delta, sens, 0.0, 1.0),
            Punch => Self::bump(&la.punch, delta, sens, 0.0, 1.0),
            Send => Self::bump(&la.send, delta, sens, 0.0, 1.0),
            Chance => Self::bump(&la.chance, delta, sens, 0.0, 1.0),
            Cursor => {
                let len = self.pattern().length.max(1);
                self.cursor = (self.cursor as i32 + step).rem_euclid(len as i32) as usize;
            }
            StepVel => self.edit_step(|st| {
                let v = if st.on() { st.vel as i32 } else { 0 };
                st.vel = (v + step * 8).clamp(0, 127) as u8;
            }),
            StepChance => self.edit_step(|st| st.prob = (st.prob as i32 + step * 5).clamp(0, 100) as u8),
            StepRoll => self.edit_step(|st| st.ratchet = (st.ratchet as i32 + step).clamp(1, 4) as u8),
            StepNudge => self.edit_step(|st| st.micro = (st.micro as i32 + step * 5).clamp(-50, 50) as i8),
            Fatten => Self::bump(&s.fatten, delta, sens, 0.0, 1.0),
            ToneShift => s.tone_shift.set((s.tone_shift.get() + step as f32).clamp(-12.0, 12.0)),
            Glue => Self::bump(&s.glue, delta, sens, 0.0, 1.0),
            Filter => Self::bump(&s.filter, delta, sens, -1.0, 1.0),
            Room => Self::bump(&s.room, delta, sens, 0.0, 1.0),
            Master => Self::bump(&s.level, delta, sens, 0.0, 1.5),
            Pads => {
                let m = match (self.pad_mode, step > 0) {
                    (PadMode::Perform, true) | (PadMode::Slots, false) => PadMode::Steps,
                    (PadMode::Steps, true) | (PadMode::Perform, false) => PadMode::Slots,
                    _ => PadMode::Perform,
                };
                self.pad_mode = m;
                self.kit.set_native(m.layer());
            }
            Record => self.record = !self.record,
            Slot => self.shared.slot.store(cyc(self.slot(), SLOTS), Ordering::Relaxed),
            RandAmount => Self::bumpf(&mut self.rand_amount, delta, sens),
            _ => {}
        }
    }

    fn press(&mut self, sel: Sel) {
        use Sel::*;
        let la = &self.shared.lanes[self.lane];
        match sel {
            Generate => self.generate(true),
            NewKit => {
                self.apply_genre_kit();
                self.status = format!("{} kit loaded", GENRES[self.genre].name);
            }
            RegenLane => self.regen_lane(self.lane),
            MakeFill => self.make_fill(),
            KitFollows | SwingGrid | Chain | Lock | Mute | Solo | Record => self.edit(sel, 1),
            Pads => {
                // F2 now cycles the kit's layers; this row still cycles
                // only the three pad modes, moving the layer with them.
                let m = self.pad_mode.next();
                self.set_pad_mode(m);
                self.kit.set_native(m.layer());
            }
            Complexity => self.gen.complexity = 0.4,
            Density => self.gen.density = 0.5,
            Variation => self.gen.variation = 0.3,
            Ghosts => self.gen.ghosts = 0.3,
            Rolls => self.gen.rolls = 0.4,
            Tempo => self.shared.bpm.set(self.genre_bpm().round()),
            Swing => self.shared.swing.set(GENRES[self.genre].swing),
            HumanTime => self.shared.human_time.set(0.15),
            HumanVel => self.shared.human_vel.set(0.2),
            Level => la.level.set(0.8),
            Pan => la.pan.set(0.0),
            Tune => la.tune.set(GENRES[self.genre].kit[self.lane].tune),
            Decay => la.decay.set(GENRES[self.genre].kit[self.lane].decay),
            Tone => la.tone.set(GENRES[self.genre].kit[self.lane].tone),
            Punch => la.punch.set(GENRES[self.genre].kit[self.lane].punch),
            Send => la.send.set(0.0),
            Chance => la.chance.set(1.0),
            Sound => {
                let m = GENRES[self.genre].kit[self.lane].model.index();
                self.set_sound(self.lane, m);
            }
            ClearLane => {
                self.push_undo();
                let (slot, lane) = (self.slot(), self.lane);
                self.edit_bank(|b| b[slot].steps[lane] = [Step::OFF; MAX_STEPS]);
                self.status = format!("{} cleared", LANE_NAMES[lane]);
            }
            RandSound => self.rand_sounds(Some(self.lane)),
            StepToggle => {
                self.push_undo();
                self.edit_step(|st| *st = if st.on() { Step::OFF } else { Step::hit(100) });
            }
            StepVel => self.edit_step(|st| st.vel = if st.on() { 100 } else { 0 }),
            StepChance => self.edit_step(|st| st.prob = 100),
            StepRoll => self.edit_step(|st| st.ratchet = 1),
            StepNudge => self.edit_step(|st| st.micro = 0),
            Fatten => self.shared.fatten.set(0.0),
            ToneShift => self.shared.tone_shift.set(0.0),
            Glue => self.shared.glue.set(0.0),
            Filter => self.shared.filter.set(0.0),
            Room => self.shared.room.set(0.0),
            Master => self.shared.level.set(0.85),
            CopySlot => {
                let from = self.slot();
                let bank = self.bank();
                let to = (1..SLOTS).map(|k| (from + k) % SLOTS).find(|&k| bank[k].is_empty()).unwrap_or((from + 1) % SLOTS);
                self.push_undo();
                self.edit_bank(|b| b[to] = b[from]);
                self.shared.slot.store(to, Ordering::Relaxed);
                self.status = format!("Copied {} to {}", SLOT_NAMES[from], SLOT_NAMES[to]);
            }
            ClearSlot => {
                self.push_undo();
                let slot = self.slot();
                let len = self.pattern().length;
                self.edit_bank(|b| b[slot] = Pattern { length: len, ..Pattern::default() });
                self.status = format!("Slot {} cleared", SLOT_NAMES[slot]);
            }
            Undo => match self.undo.pop() {
                Some(b) => {
                    self.edit_bank(|bank| *bank = *b);
                    self.status = "Undone".into();
                }
                None => self.status = "Nothing to undo".into(),
            },
            RandSounds => self.rand_sounds(None),
            RandGroove => self.rand_groove(),
            RandAll => {
                self.genre = self.rng.range(GENRES.len());
                self.rand_groove();
                self.generate(true);
                self.rand_sounds(None);
                self.status = format!("Surprise: {}", GENRES[self.genre].name);
            }
            Mutate => self.mutate(),
            SaveBeat => self.save_beat(),
            Beat(i) => {
                if let Some((_, path)) = self.beats.get(i).cloned() {
                    match export::load_beat(&path) {
                        Ok(b) => self.load_beat_file(&b),
                        Err(e) => self.status = e,
                    }
                }
            }
            ExportMidi | ExportWav | ExportStems => self.export(sel),
            _ => {}
        }
    }

    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|r| match *r {
                Row::Group(g) => (format!("{} {}", if self.expanded[g] { "v" } else { ">" }, GROUPS[g]), self.group_summary(g), true),
                Row::Leaf(s) => (self.leaf_name(s), self.leaf_value(s), false),
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
        let (a, b) = self.list.centered_scroll_window(visible, rows.len());
        (rows[a..b].to_vec(), self.list.selected - a, a > 0, b < rows.len())
    }

    // ------------------------------------------------------------ pads

    fn handle_pads(&mut self, grid: &[bool; 16]) {
        for i in 0..16 {
            let pressed = grid[i] && !self.prev_grid[i];
            if !pressed {
                continue;
            }
            match self.pad_mode {
                PadMode::Perform => {
                    if i < 8 {
                        let lane = i;
                        self.shared.audition[lane].store(110, Ordering::Relaxed);
                        self.lane = lane;
                        if self.record && self.shared.playing.load(Ordering::Relaxed) {
                            // quantize to the nearest step
                            let slot = self.slot();
                            let len = self.pattern().length.max(1);
                            let step = (self.shared.step.load(Ordering::Relaxed) + 1) % len;
                            self.edit_bank(|b| b[slot].steps[lane][step] = Step::hit(105));
                        }
                    } else {
                        let lane = i - 8;
                        let m = &self.shared.lanes[lane].mute;
                        m.store(!m.load(Ordering::Relaxed), Ordering::Relaxed);
                    }
                }
                PadMode::Steps => {
                    let bar = self.view_bar();
                    let step = bar * 16 + i;
                    self.cursor = step;
                    let (slot, lane) = (self.slot(), self.lane);
                    self.edit_bank(|b| {
                        let st = &mut b[slot].steps[lane][step];
                        *st = if st.on() { Step::OFF } else { Step::hit(100) };
                    });
                }
                PadMode::Slots => {
                    if i < 8 {
                        self.shared.slot.store(i, Ordering::Relaxed);
                        self.status = format!("Slot {}{}", SLOT_NAMES[i], if self.shared.playing.load(Ordering::Relaxed) { " queued" } else { "" });
                    } else {
                        self.lane = i - 8;
                        self.regen_lane(i - 8);
                    }
                }
            }
        }
        self.prev_grid = *grid;
    }

    // ---------------------------------------------------------- drawing

    fn draw_grid(&self, fb: &mut FrameBuffer, x0: i32, y0: i32) {
        let bank = self.bank();
        let slot = self.slot();
        let p = bank[slot];
        let bar = self.view_bar();
        let playing = self.shared.playing.load(Ordering::Relaxed) && self.shared.playing_slot.load(Ordering::Relaxed) == slot;
        let play_step = self.shared.step.load(Ordering::Relaxed);
        let (cw, ch, label_w) = (12, 15, 14);
        let lbl = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));
        let lbl_sel = MonoTextStyle::new(&SPLEEN_6X12, ACCENT);
        let lock = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(31, 40, 0));
        for lane in 0..LANES {
            let y = y0 + lane as i32 * ch;
            let muted = self.shared.lanes[lane].mute.load(Ordering::Relaxed);
            let style = if self.locks[lane] {
                lock
            } else if lane == self.lane {
                lbl_sel
            } else {
                lbl
            };
            Text::new(LANE_SHORT[lane], Point::new(x0, y + 10), style).draw(fb).ok();
            let flash = self.shared.flashes[lane].get().clamp(0.0, 1.0);
            for i in 0..16 {
                let step = bar * 16 + i;
                let x = x0 + label_w + i as i32 * (cw + 1);
                let st = p.steps[lane][step.min(MAX_STEPS - 1)];
                let beat_shade = if i % 4 == 0 { 5 } else { 3 };
                let mut fill = Rgb565::new(beat_shade, beat_shade * 2, beat_shade);
                if step < p.length && st.on() {
                    let v = st.vel as f32 / 127.0;
                    let g = (18.0 + v * 45.0) as u8;
                    fill = if muted {
                        Rgb565::new(6, 12, 6)
                    } else if st.prob < 100 {
                        Rgb565::new(4, g / 2 + 8, (g / 2) as u8)
                    } else {
                        Rgb565::new((v * 8.0) as u8, g, (v * 10.0) as u8)
                    };
                    if st.ratchet > 1 {
                        fill = Rgb565::new(20, g, 4);
                    }
                }
                if playing && step == play_step {
                    fill = if st.on() && !muted { Rgb565::new((16.0 + flash * 15.0) as u8, 63, 20) } else { Rgb565::new(10, 20, 12) };
                }
                Rectangle::new(Point::new(x, y + 1), Size::new(cw as u32, (ch - 3) as u32)).into_styled(PrimitiveStyle::with_fill(fill)).draw(fb).ok();
                if self.pad_mode == PadMode::Steps && lane == self.lane && step == self.cursor {
                    Rectangle::new(Point::new(x - 1, y), Size::new(cw as u32 + 2, (ch - 1) as u32))
                        .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(31, 50, 0), 1))
                        .draw(fb)
                        .ok();
                }
            }
        }
        // bar / slot strip
        let y = y0 + LANES as i32 * ch + 6;
        let bars = (p.length / 16).max(1);
        for b in 0..bars {
            let x = x0 + label_w + b as i32 * 52;
            let c = if b == bar { ACCENT } else { Rgb565::new(5, 10, 5) };
            Rectangle::new(Point::new(x, y), Size::new(48, 4)).into_styled(PrimitiveStyle::with_fill(c)).draw(fb).ok();
        }
        let y = y + 10;
        let cur_play = self.shared.playing_slot.load(Ordering::Relaxed);
        for k in 0..SLOTS {
            let x = x0 + k as i32 * 27;
            let empty = bank[k].is_empty();
            let fill = if k == cur_play && self.shared.playing.load(Ordering::Relaxed) {
                Rgb565::new(28, 56, 0)
            } else if k == slot {
                ACCENT
            } else if empty {
                Rgb565::new(3, 6, 3)
            } else {
                Rgb565::new(5, 22, 8)
            };
            Rectangle::new(Point::new(x, y), Size::new(24, 14)).into_styled(PrimitiveStyle::with_fill(fill)).draw(fb).ok();
            let t = MonoTextStyle::new(&SPLEEN_6X12, if empty && k != slot { Rgb565::new(14, 28, 14) } else { Rgb565::BLACK });
            Text::new(SLOT_NAMES[k], Point::new(x + 9, y + 11), t).draw(fb).ok();
        }
    }
}

impl PulsarApp {
    fn set_pad_mode(&mut self, m: PadMode) {
        self.pad_mode = m;
        self.status = match m {
            PadMode::Perform => "Pads: top rows play lanes, bottom rows mute".into(),
            PadMode::Steps => format!("Pads: steps of {} (Lane row picks the lane)", LANE_NAMES[self.lane]),
            PadMode::Slots => "Pads: top rows pick slots A-H, bottom rows regenerate lanes".into(),
        };
    }

    fn knob(&self, i: usize) -> Knob<'_> {
        let s = &self.shared;
        let la = &s.lanes[self.lane];
        match CONTROLS[i % 16].0 {
            Sel::Filter => Knob::F(&s.filter, -1.0, 1.0),
            Sel::Swing => Knob::F(&s.swing, 0.0, 1.0),
            Sel::Fatten => Knob::F(&s.fatten, 0.0, 1.0),
            Sel::Room => Knob::F(&s.room, 0.0, 1.0),
            Sel::Decay => Knob::F(&la.decay, 0.0, 1.0),
            Sel::Tone => Knob::F(&la.tone, 0.0, 1.0),
            Sel::Tempo => Knob::F(&s.bpm, 40.0, 240.0),
            Sel::Glue => Knob::F(&s.glue, 0.0, 1.0),
            Sel::Tune => Knob::F(&la.tune, -24.0, 24.0),
            Sel::Punch => Knob::F(&la.punch, 0.0, 1.0),
            Sel::Level => Knob::F(&la.level, 0.0, 1.5),
            Sel::Chance => Knob::F(&la.chance, 0.0, 1.0),
            Sel::HumanTime => Knob::F(&s.human_time, 0.0, 1.0),
            Sel::HumanVel => Knob::F(&s.human_vel, 0.0, 1.0),
            // Slot (AtomicUsize) and genre (a plain field) have no Knob
            // variant; PlayHost handles them by hand.
            _ => Knob::None,
        }
    }

    /// The colours the old LED overlay showed for each pad mode.
    fn mode_pad_color(&self, mode: PadMode, i: usize) -> PadColor {
        let p = self.pattern();
        match mode {
            PadMode::Perform => {
                if i < 8 {
                    if p.hits(i) > 0 {
                        PadColor::Green
                    } else {
                        PadColor::Off
                    }
                } else if self.shared.lanes[i - 8].mute.load(Ordering::Relaxed) {
                    PadColor::Red
                } else {
                    PadColor::Green
                }
            }
            PadMode::Steps => {
                let step = self.view_bar() * 16 + i;
                let playing = self.shared.playing.load(Ordering::Relaxed) && self.shared.step.load(Ordering::Relaxed) == step;
                if playing {
                    PadColor::Red
                } else if step == self.cursor {
                    PadColor::Yellow
                } else if p.steps[self.lane][step.min(MAX_STEPS - 1)].on() {
                    PadColor::Green
                } else {
                    PadColor::Off
                }
            }
            PadMode::Slots => {
                if i < 8 {
                    let filled = self.shared.bank.lock().map(|b| b.get(i).is_some_and(|p| !p.is_empty())).unwrap_or(false);
                    if i == self.shared.playing_slot.load(Ordering::Relaxed) && self.shared.playing.load(Ordering::Relaxed) {
                        PadColor::Yellow
                    } else if i == self.slot() {
                        PadColor::Blue
                    } else if filled {
                        PadColor::Green
                    } else {
                        PadColor::Off
                    }
                } else if self.locks[i - 8] {
                    PadColor::Yellow
                } else {
                    PadColor::Blue
                }
            }
        }
    }
}

/// General MIDI drum notes to Pulsar's lanes, so a drum pad controller
/// or a GM-mapped keyboard plays the matching sound on PERFORM.
fn gm_lane(note: u8) -> Option<usize> {
    Some(match note {
        35 | 36 => 0,                // kicks
        38 | 40 => 1,                // snares
        39 => 2,                     // hand clap
        42 | 44 => 3,                // closed / pedal hat
        46 => 4,                     // open hat
        37 | 41 | 43 | 45 => 5,      // side stick, low toms
        47 | 48 | 50 | 56 => 6,      // high toms, cowbell
        49 | 52 | 55 | 57 => 7,      // crashes, china, splash
        _ => return None,
    })
}

impl PlayHost for PulsarApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS.len()
    }
    fn kit_label(&self, i: usize) -> String {
        let name = CONTROLS[i % 16].1;
        if lane_control(i % 16) { format!("{} {name}", LANE_SHORT[self.lane]) } else { name.to_string() }
    }
    fn kit_value(&self, i: usize) -> String {
        self.leaf_value(CONTROLS[i % 16].0)
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        match i % 16 {
            C_SLOT => Some(self.slot() as f32 / (SLOTS - 1) as f32),
            C_GENRE => Some(self.genre as f32 / (GENRES.len().max(2) - 1) as f32),
            _ => self.knob(i).norm(),
        }
    }
    fn kit_stepped(&self, i: usize) -> bool {
        match i % 16 {
            C_SLOT | C_GENRE => true,
            _ => self.knob(i).stepped(),
        }
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit(CONTROLS[i % 16].0, delta);
    }
    fn kit_reset(&mut self, i: usize) {
        // `press` resets every continuous control here; slot and genre
        // have no reset and fall through to its no-op arm.
        self.press(CONTROLS[i % 16].0);
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i % 16 {
            C_SLOT => self.shared.slot.store((v * (SLOTS - 1) as f32).round() as usize, Ordering::Relaxed),
            C_GENRE => self.genre = ((v * (GENRES.len().max(2) - 1) as f32).round() as usize).min(GENRES.len() - 1),
            _ => self.knob(i).set(v),
        }
    }
    /// A moment is the sound and groove: the lane controls are stored
    /// with the lane they belong to (and recalled onto it, whatever lane
    /// is selected now), and slot/genre are left out so recalling a sound
    /// never jumps the arrangement.
    fn kit_snapshot(&self) -> serde_json::Value {
        let v: Vec<serde_json::Value> = (0..CONTROLS.len())
            .map(|i| if i == C_SLOT || i == C_GENRE { serde_json::Value::Null } else { self.kit_norm(i).map_or(serde_json::Value::Null, |x| serde_json::json!(x)) })
            .collect();
        serde_json::json!({ "lane": self.lane, "v": v })
    }
    fn kit_recall(&mut self, m: &serde_json::Value) {
        let keep = self.lane;
        if let Some(l) = m.get("lane").and_then(|l| l.as_u64()) {
            self.lane = (l as usize).min(LANES - 1);
        }
        if let Some(items) = m.get("v").and_then(|v| v.as_array()) {
            for (i, item) in items.iter().enumerate().take(CONTROLS.len()) {
                if let Some(x) = item.as_f64() {
                    self.kit_set_norm(i, (x as f32).clamp(0.0, 1.0));
                }
            }
        }
        self.lane = keep;
    }
    fn kit_line(&self) -> String {
        let s = &self.shared;
        let playing = s.playing.load(Ordering::Relaxed);
        format!(
            "{} {}  bar {}  {}{}{}",
            SLOT_NAMES[self.slot()],
            if playing { ">" } else { "||" },
            self.view_bar() + 1,
            LANE_SHORT[self.lane],
            if playing && s.in_fill.load(Ordering::Relaxed) { "  FILL" } else { "" },
            if self.record { "  REC" } else { "" }
        )
    }
    fn kit_pad_label(&self, layer: u8, pad: usize) -> String {
        match PadMode::from_layer(layer) {
            PadMode::Perform => {
                if pad < 8 {
                    LANE_SHORT[pad].to_string()
                } else if self.shared.lanes[pad - 8].mute.load(Ordering::Relaxed) {
                    format!("{} MUTED", LANE_SHORT[pad - 8])
                } else {
                    format!("{} mute", LANE_SHORT[pad - 8])
                }
            }
            PadMode::Steps => format!("{}", self.view_bar() * 16 + pad + 1),
            PadMode::Slots => {
                if pad < 8 {
                    SLOT_NAMES[pad].to_string()
                } else {
                    format!("new {}", LANE_SHORT[pad - 8])
                }
            }
        }
    }
    fn kit_pad_color(&self, layer: u8, pad: usize, _held: bool) -> PadColor {
        self.mode_pad_color(PadMode::from_layer(layer), pad)
    }
    /// On PERFORM a GM drum note plays its lane; any other key plays lane
    /// note % 8, so a keyboard never lands on the mute row. On STEPS and
    /// SLOTS keys press pads the way the shell always mapped them.
    fn kit_midi_pad(&self, note: u8) -> Option<usize> {
        // Controllers send encoder-touch notes below 21; never pads.
        if note < 21 {
            return None;
        }
        Some(match self.pad_mode {
            PadMode::Perform => gm_lane(note).unwrap_or(note as usize % 8),
            _ => note as usize % 16,
        })
    }
}

impl PulsarApp {
    #[allow(dead_code)] // Slint GUI only; see app.rs
    /// Everything the Slint home screen's Pulsar panel shows -- the same
    /// state `draw_grid` puts on the device screen.
    fn panel_extra(&self) -> crate::app::PulsarExtra {
        let s = &self.shared;
        let bank = self.bank();
        let slot = self.slot();
        let p = bank[slot];
        let bar = self.view_bar();
        let playing = s.playing.load(Ordering::Relaxed);
        let play_slot = s.playing_slot.load(Ordering::Relaxed);
        let play_step = s.step.load(Ordering::Relaxed);
        let mut cell_vel = Vec::with_capacity(LANES * 16);
        let mut cell_mark = Vec::with_capacity(LANES * 16);
        for lane in 0..LANES {
            for i in 0..16 {
                let step = bar * 16 + i;
                let st = p.steps[lane][step.min(MAX_STEPS - 1)];
                let on = step < p.length && st.on();
                cell_vel.push(if on { st.vel as f32 / 127.0 } else { 0.0 });
                cell_mark.push(if !on {
                    0
                } else if st.ratchet > 1 {
                    2
                } else if st.prob < 100 {
                    1
                } else {
                    0
                });
            }
        }
        let playhead = (playing && play_slot == slot && play_step / 16 == bar).then_some(play_step % 16);
        let cursor = (self.pad_mode == PadMode::Steps && self.cursor / 16 == bar).then_some((self.lane, self.cursor % 16));
        crate::app::PulsarExtra {
            genre_name: GENRES[self.genre].name.to_string(),
            bpm: s.bpm.get(),
            slot,
            playing_slot: playing.then_some(play_slot),
            slot_filled: std::array::from_fn(|k| !bank[k].is_empty()),
            in_fill: playing && s.in_fill.load(Ordering::Relaxed),
            bar,
            bars: (p.length / 16).max(1),
            pad_mode: match self.pad_mode {
                PadMode::Perform => "PERFORM",
                PadMode::Steps => "STEPS",
                PadMode::Slots => "SLOTS",
            }
            .into(),
            record: self.record,
            lane_names: LANE_SHORT,
            lane: self.lane,
            lane_locked: self.locks,
            lane_muted: std::array::from_fn(|l| s.lanes[l].mute.load(Ordering::Relaxed)),
            lane_flash: std::array::from_fn(|l| s.flashes[l].get().clamp(0.0, 1.0)),
            cell_vel,
            cell_mark,
            playhead,
            cursor,
            swing_pct: 50.0 + s.swing.get() * 25.0,
            peak: s.peak.get().clamp(0.0, 1.0),
            status: self.status.clone(),
        }
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for w in text.split_whitespace() {
        if line.len() + w.len() + 1 > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&w.chars().take(width).collect::<String>());
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

impl App for PulsarApp {
    fn tick(&mut self, input: &Input) {
        // The play view takes the knobs and D-pad first; in the menu they
        // pass straight through. Pads reach handle_pads only on Pulsar's
        // own layers, and the layer showing is the pad mode.
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let input = &step.input;
        if let Some(id) = step.native {
            let m = PadMode::from_layer(id);
            if m != self.pad_mode {
                self.set_pad_mode(m);
            }
        }
        self.handle_pads(&input.grid);
        let rows = self.visible_rows();
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();
        if input.knob1_press {
            match current {
                Some(Row::Group(g)) => self.expanded[g] = !self.expanded[g],
                Some(Row::Leaf(Sel::Lane)) | Some(Row::Leaf(Sel::RegenLane)) => {
                    self.locks[self.lane] = !self.locks[self.lane];
                    self.status = format!("{} {}", LANE_NAMES[self.lane], if self.locks[self.lane] { "locked" } else { "unlocked" });
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
        self.prev_grid = [false; 16];
    }

    fn background_tick(&mut self) {
        // free sample buffers the audio thread no longer holds
        self.graveyard.retain(|b| Arc::strong_count(b) > 1);
    }

    fn running(&self) -> Option<bool> {
        Some(self.shared.playing.load(Ordering::Relaxed))
    }

    fn supports_pad_lock(&self) -> bool {
        true
    }

    fn play_surface(&self) -> bool {
        true
    }

    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (!self.kit.menu).then(|| self.kit.column(self))
    }

    fn toggle_running(&mut self) {
        let p = !self.shared.playing.load(Ordering::Relaxed);
        self.shared.playing.store(p, Ordering::Relaxed);
    }

    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }

    /// F2 cycles PERFORM, STEPS, SLOTS, Controls, Moments, Throws. Landing
    /// on one of Pulsar's own layers switches the pad mode right away (not
    /// a frame later in tick) so a MIDI key that frame maps the new way.
    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
        if let Layer::Native(id, _) = self.kit.layer() {
            let m = PadMode::from_layer(id);
            if m != self.pad_mode {
                self.set_pad_mode(m);
            }
        }
    }

    fn grid_led_overlay(&self) -> [PadColor; 16] {
        self.kit.led_overlay(self)
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let sr = f32::from_bits(self.shared.sample_rate.load(Ordering::Relaxed));
        Some(Box::new(PulsarProcessor {
            shared: Arc::clone(&self.shared),
            engine: Engine::new(sr),
            bank: Box::new([Pattern::default(); BANK]),
            bank_version: 0,
            samples: Default::default(),
            samples_version: 0,
            l: Vec::with_capacity(4096),
            r: Vec::with_capacity(4096),
            mono: Vec::with_capacity(4096),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let title = MonoTextStyle::new(&SPLEEN_16X32, Rgb565::WHITE);
        Text::new(APP_NAME, Point::new(16, 30), title).draw(fb).ok();
        let accent = MonoTextStyle::new(&SPLEEN_6X12, ACCENT);
        let big = MonoTextStyle::new(&SPLEEN_8X16, ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));
        let s = &self.shared;
        let head = format!("{}  {:.0} bpm", GENRES[self.genre].name, s.bpm.get());
        Text::new(&fit(&head, 32), Point::new(124, 27), big).draw(fb).ok();

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
            // Pulsar's green-on-black; stops short of the step grid (x 394).
            let pal = kit::draw::Palette { bg: Rgb565::BLACK, ink: Rgb565::new(24, 48, 24), accent: ACCENT, dim: Rgb565::new(18, 36, 18), faint: Rgb565::new(3, 6, 3) };
            kit::draw::column(fb, &col, 16, 40, 366, 284, pal);
        }

        let (px, py) = (400, 44);
        let mode = match self.pad_mode {
            PadMode::Perform => "Perform",
            PadMode::Steps => "Steps",
            PadMode::Slots => "Slots",
        };
        let bar = self.view_bar() + 1;
        let fill = if s.in_fill.load(Ordering::Relaxed) && s.playing.load(Ordering::Relaxed) { "  FILL" } else { "" };
        Text::new(&format!("Slot {}  bar {bar}  pads: {mode}{fill}", SLOT_NAMES[self.slot()]), Point::new(px, py - 4), accent).draw(fb).ok();
        self.draw_grid(fb, px - 6, py);

        let info = format!(
            "{}  swing {:.0}%  load {:.0}%{}",
            if s.playing.load(Ordering::Relaxed) { "playing" } else { "stopped" },
            50.0 + s.swing.get() * 25.0,
            s.load.get() * 100.0,
            if self.record { "  REC" } else { "" }
        );
        Text::new(&info, Point::new(px, py + 170), dim).draw(fb).ok();
        // meter
        let pk = s.peak.get().clamp(0.0, 1.0);
        Rectangle::new(Point::new(px, py + 178), Size::new(220, 4)).into_styled(PrimitiveStyle::with_fill(Rgb565::new(3, 6, 3))).draw(fb).ok();
        Rectangle::new(Point::new(px, py + 178), Size::new((220.0 * pk) as u32, 4)).into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(fb).ok();
        for (k, l) in wrap(&self.status, 36).iter().take(4).enumerate() {
            Text::new(l, Point::new(px, py + 198 + k as i32 * 13), accent).draw(fb).ok();
        }

        let hint = match self.visible_rows().get(self.list.selected) {
            _ if !self.kit.menu => "knobs: filter/swing..   D-pad: slot   F2: pads   F3: play/stop   R1: menu",
            Some(Row::Group(_)) => "knob1: browse   press knob1: open/close   F3: play/stop",
            Some(Row::Leaf(Sel::Lane)) => "knob2: pick lane   press knob1: lock lane",
            Some(Row::Leaf(s)) if self.leaf_name(*s).starts_with(">>") => "press knob2 to run",
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
        crate::app::SlintExtra::Pulsar(self.panel_extra())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> (PulsarApp, Arc<ModBus>, Arc<AudioBus>, Arc<MixerBus>) {
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let app = PulsarApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(1.0)), Arc::clone(&modbus), Arc::clone(&audio_bus), Arc::clone(&mixer_bus));
        (app, modbus, audio_bus, mixer_bus)
    }

    fn render(p: &mut dyn AudioProcessor, blocks: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let mut buf = vec![0.0; 1024];
        for _ in 0..blocks {
            p.process(&mut buf, 2, 48_000.0);
            out.extend_from_slice(&buf);
        }
        out
    }

    #[test]
    fn starts_with_a_beat_and_plays_on_f2() {
        let (mut app, ..) = make();
        assert!(!app.pattern().is_empty());
        let mut proc = app.audio_processor().unwrap();
        assert!(render(proc.as_mut(), 20).iter().all(|x| x.abs() < 1e-4), "silent until started");
        app.toggle_running();
        let out = render(proc.as_mut(), 200);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(out.iter().all(|x| x.is_finite()));
        assert!(peak > 0.1 && peak <= 1.0, "peak {peak}");
    }

    #[test]
    fn registers_buses_including_a_kick_tap_for_sidechain() {
        let (_app, modbus, audio_bus, mixer_bus) = make();
        assert!(audio_bus.names().iter().any(|n| n == "Pulsar"));
        assert!(audio_bus.names().iter().any(|n| n == "Pulsar Kick"));
        assert!(mixer_bus.names().iter().any(|n| n == "Pulsar"));
        assert!(modbus.names().iter().any(|n| n == "Pulsar: DJ Filter"));
    }

    #[test]
    fn locks_survive_generate_and_every_genre_works() {
        let (mut app, ..) = make();
        app.locks[0] = true;
        let kick = app.pattern().steps[0];
        for g in 0..GENRES.len() {
            app.genre = g;
            app.generate(true);
            assert_eq!(app.pattern().steps[0], kick, "{}", GENRES[g].name);
        }
    }

    #[test]
    fn pads_perform_steps_and_slots() {
        let (mut app, ..) = make();
        let press = |app: &mut PulsarApp, i: usize| {
            let mut inp = Input::default();
            inp.grid[i] = true;
            app.tick(&inp);
            app.tick(&Input::default());
        };
        press(&mut app, 2);
        assert_eq!(app.shared.audition[2].load(Ordering::Relaxed), 110);
        press(&mut app, 9);
        assert!(app.shared.lanes[1].mute.load(Ordering::Relaxed));
        app.toggle_grid_mode(); // Steps
        app.lane = 5;
        let before = app.pattern().steps[5][3].on();
        press(&mut app, 3);
        assert_ne!(app.pattern().steps[5][3].on(), before);
        app.toggle_grid_mode(); // Slots
        press(&mut app, 4);
        assert_eq!(app.slot(), 4);
    }

    #[test]
    fn undo_restores_the_previous_pattern() {
        let (mut app, ..) = make();
        let before = app.pattern();
        app.generate(false);
        app.press(Sel::Undo);
        assert_eq!(app.pattern(), before);
    }

    #[test]
    fn every_row_edits_presses_and_draws() {
        let (mut app, ..) = make();
        app.expanded = [true; 8];
        for row in app.visible_rows() {
            if let Row::Leaf(sel) = row {
                if matches!(sel, Sel::SaveBeat | Sel::ExportMidi | Sel::ExportWav | Sel::ExportStems | Sel::Beat(_)) {
                    continue;
                }
                for d in [1, -1, 6, -6] {
                    app.edit(sel, d);
                }
                app.press(sel);
            }
        }
        for mode in 0..3 {
            let mut fb = FrameBuffer::new();
            app.draw(&mut fb);
            assert!(fb.buffer().iter().any(|&p| p != 0));
            let _ = app.grid_led_overlay();
            let _ = mode;
            app.toggle_grid_mode();
        }
    }

    #[test]
    fn opens_on_the_play_view_knob1_sweeps_the_filter_and_r1_opens_the_menu() {
        let (mut app, ..) = make();
        assert!(app.play_column().is_some(), "play view first");
        let f = app.shared.filter.get();
        app.tick(&Input { knob1: 5, ..Default::default() });
        assert!(app.shared.filter.get() > f, "knob 1 is the DJ filter on the play view");
        app.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(app.slot(), 1, "D-pad up = next slot");
        app.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(app.play_column().is_none(), "R1 opens the full menu");
    }

    #[test]
    fn f2_layers_are_the_pad_modes_and_kit_layers_keep_the_pads() {
        let (mut app, ..) = make();
        let press = |app: &mut PulsarApp, i: usize| {
            let mut inp = Input::default();
            inp.grid[i] = true;
            app.tick(&inp);
            app.tick(&Input::default());
        };
        assert_eq!(app.kit.layer_label(), "PERFORM");
        press(&mut app, 1);
        assert_eq!(app.shared.audition[1].load(Ordering::Relaxed), 110, "PERFORM plays lanes");
        app.toggle_grid_mode();
        assert_eq!(app.pad_mode, PadMode::Steps);
        app.toggle_grid_mode();
        assert_eq!(app.pad_mode, PadMode::Slots);
        app.toggle_grid_mode(); // Controls
        press(&mut app, 3);
        assert_eq!(app.slot(), 0, "kit layers keep the pads from the slot picker");
        app.press(Sel::Pads);
        assert!(app.pad_mode == PadMode::Perform && app.kit.layer_label() == "PERFORM", "the menu's Pads row moves the layer");
        app.edit(Sel::Pads, 1);
        assert!(app.pad_mode == PadMode::Steps && app.kit.layer_label() == "STEPS");
    }

    #[test]
    fn throws_and_the_stick_spring_back_and_moments_keep_their_lane() {
        let (mut app, ..) = make();
        for _ in 0..5 {
            app.toggle_grid_mode(); // ... Controls, Moments, Throws
        }
        assert_eq!(app.kit.layer_label(), "THROWS");
        let room = app.shared.room.get();
        let mut inp = Input::default();
        inp.grid[kit::rank_pad(2)] = true; // ROOM WASH
        app.tick(&inp);
        assert_eq!(app.shared.room.get(), 1.0, "held throw");
        app.tick(&Input::default());
        assert!((app.shared.room.get() - room).abs() < 1e-5, "springs back");
        app.tick(&Input { stick: [-1.0, 0.0], ..Default::default() });
        assert!(app.shared.filter.get() < -0.9, "stick left = low-pass");
        app.tick(&Input::default());
        assert!(app.shared.filter.get().abs() < 1e-5, "back to off");

        app.lane = 1;
        app.shared.lanes[1].decay.set(0.9);
        let moment = app.kit_snapshot();
        app.shared.lanes[1].decay.set(0.1);
        app.lane = 3;
        let hat = app.shared.lanes[3].decay.get();
        app.kit_recall(&moment);
        assert!((app.shared.lanes[1].decay.get() - 0.9).abs() < 1e-5, "recalled onto the snare");
        assert_eq!(app.shared.lanes[3].decay.get(), hat, "the selected lane is left alone");
        assert_eq!(app.lane, 3);
    }

    #[test]
    fn gm_drum_notes_play_their_lane_on_perform() {
        let (mut app, ..) = make();
        let mut keys = crate::app::MidiKeys::default();
        keys.0[38] = 100; // GM snare
        app.tick(&Input { midi_keys: keys, ..Default::default() });
        assert_eq!(app.shared.audition[1].load(Ordering::Relaxed), 110);
    }

    #[test]
    fn beat_save_format_round_trips_through_the_app() {
        let (mut app, ..) = make();
        app.shared.bpm.set(97.0);
        app.shared.lanes[2].tune.set(-3.0);
        let b = app.beat_file();
        let json = serde_json::to_string(&b).unwrap();
        let back: export::BeatFile = serde_json::from_str(&json).unwrap();
        let (mut other, ..) = make();
        other.load_beat_file(&back);
        assert_eq!(other.shared.bpm.get(), 97.0);
        assert_eq!(other.shared.lanes[2].tune.get(), -3.0);
        assert_eq!(other.bank()[0], app.bank()[0]);
    }
}
