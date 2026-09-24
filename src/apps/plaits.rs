//! The real Mutable Instruments Plaits voice (see src/plaits_ffi.rs and
//! vendor/eurorack), not a simplified approximation. Verified against
//! vendor/eurorack/plaits/dsp/voice.cc's Voice::Render: Harmonics/Timbre/
//! Morph really are 0..1 -- but Note is clamped to -119..120 semitones
//! (~20 octaves). Harmonics/Timbre/Morph also mean something different
//! per engine -- ENGINE_PARAM_NAMES labels them per engine from Mutable's
//! own descriptions; treat engines 0-7 (the newer "engine2" bank) as
//! lower-confidence than 8-23 (widely documented).
//!
//! Layout: settings grouped into collapsible dropdowns on the left, a
//! hidable/resizable piano-roll strip along the bottom, and a
//! context-sensitive visualizer panel on the right. That panel shows a
//! live mini spectrum of Plaits' own output by default, but switches to
//! a live view of whatever you're currently navigating: the ADSR shape
//! (with a moving dot tracking the actual envelope stage/level) while
//! anywhere inside the Envelope group, or a scrolling waveform of
//! whatever modulator you're looking at while inside the Modulators
//! group -- the same modulator kinds Off/Harmonics/Timbre/Morph/Decay
//! can be routed to. Modulators run and are visible in the panel even
//! while their target is "Off", so you can preview one before wiring it.
//!
//! Control surface, no F-buttons:
//!   - knob1: move the selection up/down the (flattened) list of group
//!     headers, and for expanded groups, their children -- modulator
//!     slots are their own sub-group (a "subfolder" per modulator), each
//!     expanding further into that modulator's Target/Rate/Depth.
//!   - knob1 press: toggle expand/collapse on a group or modulator-slot
//!     header; no-op on a leaf row.
//!   - knob2: edit the selected leaf row's value; no-op on a header.
//!   - knob2 press: reset the selected leaf row to a sensible default.
//!   - grid: 16 notes (mono: last-pressed wins; poly: each held key is
//!     its own voice).
//!   - F1-F4: not handled here -- they're global OS-level quick-nav now
//!     (Settings/Apps, always visible; see os.rs), consumed before this
//!     app ever sees them. The piano roll's "sits above the 4 app
//!     buttons" framing is literally true now that bar exists.
//!
//! Voice mode: Mono (single voice, optionally a chord) or Poly (up to 16
//! simultaneous voices, one per held grid key; chord mode is ignored in
//! Poly to avoid a 16x4-voice explosion).
//!
//! Chord mode (Mono only): a grid key triggers a full chord -- root plus
//! the selected chord type's intervals -- through extra voice instances,
//! separate from Plaits' own built-in "Chord" engine (index 14).
//!
//! Modulation: 9 assignable slots, each independently targeting
//! Off/Harmonics/Timbre/Morph/Decay, or another modulator's own Rate --
//! letting one modulator speed up or slow down another (or itself, for
//! a self-FM/chaos patch). Rate targets shift the target's rate by up
//! to +/-RATE_MOD_OCTAVES octaves at full depth, using that target's
//! *previous* audio block's output so the result doesn't depend on
//! which slot happens to run first within a block --
//!   - LFO 1-4: sine wave at an adjustable rate.
//!   - S&H: freshly randomized value, held at an adjustable rate.
//!   - Follower: tracks the gate (smoothed) -- not true audio amplitude,
//!     which isn't exposed by the FFI bridge.
//!   - Ramp: a sawtooth wave at an adjustable rate.
//!   - Drift: a slow, smooth random walk -- rate controls how fast it
//!     wanders.
//!   - Sequencer: an 8-step repeating pattern of random values, stepped
//!     through at an adjustable rate.
//! Each has its own live visualizer in the right-hand panel (see
//! `draw_mod_panel`) -- the ADSR gets one too (`draw_adsr_panel`).

use super::plaits_layout::LayoutWatcher;
use crate::app::{App, Input};
use crate::arpeggiator::{Arpeggiator, PATTERN_NAMES as ARP_PATTERN_NAMES};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::plaits_ffi::{PlaitsParams, PlaitsVoice};
use crate::util::{accelerate, note_name, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub const NUM_ENGINES: u32 = 24;
const BASE_NOTE: i32 = 48; // roughly C3 in Plaits' MIDI-note-like units
const NOTE_MIN: i32 = -119;
const NOTE_MAX: i32 = 120;
const LPG_COLOUR: f32 = 0.5; // fixed default -- no control assigned to it yet
const MAX_CHORD_VOICES: usize = 3; // supports up to 4-note chords (root + 3)
const NUM_MOD_SLOTS: usize = 9;
const SEQ_STEPS: usize = 8;
const MOD_HISTORY_LEN: usize = 150;
const ANALYZER_FFT_SIZE: usize = 256;
const ANALYZER_BARS: usize = 24;
const MIN_DB: f32 = -50.0;
const MAX_DB: f32 = 0.0;
const MIN_ROLL_HEIGHT: i32 = 20;
const MAX_ROLL_HEIGHT: i32 = 140;
/// Pitch classes that are "black keys" on a standard keyboard, for the
/// piano roll's coloring (0=C, 1=C#, ...).
const BLACK_KEYS: [bool; 12] = [
    false, true, false, true, false, false, true, false, true, false, true, false,
];

pub const ENGINE_NAMES: [&str; NUM_ENGINES as usize] = [
    "VA VCF", "Phase Dist", "6-OP FM A", "6-OP FM B", "6-OP FM C", "Wave Terrain",
    "String Machine", "Chiptune", "Virtual Analog", "Waveshaper", "FM", "Grain",
    "Additive", "Wavetable", "Chord", "Speech", "Swarm", "Noise", "Particle",
    "String", "Modal", "Bass Drum", "Snare Drum", "Hi-Hat",
];

/// [Harmonics, Timbre, Morph] labels per engine, same order as ENGINE_NAMES.
const ENGINE_PARAM_NAMES: [[&str; 3]; NUM_ENGINES as usize] = [
    ["Detune", "Cutoff", "Resonance"],
    ["Mod Shape", "PD Amount", "Asymmetry"],
    ["Patch", "Index", "Mod"],
    ["Patch", "Index", "Mod"],
    ["Patch", "Index", "Mod"],
    ["Path Shape", "Eccentricity", "Center"],
    ["Chord", "Ensemble", "Filter"],
    ["Waveform", "Arp Pattern", "Envelope"],
    ["Detune", "Pulse W/Sync", "Tri->Saw"],
    ["Waveform", "Fold Amount", "Asymmetry"],
    ["Ratio", "Index", "Feedback"],
    ["Formant", "Density", "Shape"],
    ["Partial Count", "Tilt", "Organ/Formant"],
    ["Bank", "Row", "Column"],
    ["Chord Type", "Voicing", "Wave/Timbre"],
    ["Phoneme/Word", "Species", "Prosody/Speed"],
    ["Swarm Size", "Detune", "Sync"],
    ["Filter Type", "Cutoff", "Resonance"],
    ["Density", "Filter Freq", "Spread"],
    ["Inharmonicity", "Brightness", "Position"],
    ["Inharmonicity", "Brightness", "Position"],
    ["Tone/Overdrive", "Attack/Punch", "Decay"],
    ["Tone Balance", "Snappy/Noise", "Decay"],
    ["Noise Type", "Tone/Metallic", "Decay"],
];

/// The 24 engines as 3 banks of 8, echoing the real Plaits hardware's
/// column of 8 LEDs that changes color to page through banks -- red,
/// green, and yellow here, each covering 8 consecutive engine indices.
const NUM_BANKS: usize = 3;
const BANK_SIZE: usize = 8;
const BANK_NAMES: [&str; NUM_BANKS] = ["Bank 1 (1-8)", "Bank 2 (9-16)", "Bank 3 (17-24)"];
const BANK_COLORS: [Rgb565; NUM_BANKS] = [
    Rgb565::new(0, 50, 4),   // green -- Bank 1
    Rgb565::new(27, 3, 3),   // red -- Bank 2
    Rgb565::new(31, 41, 0),  // orange -- Bank 3
];

/// Chord types: name + extra intervals beyond the root (root is always 0).
const CHORD_TYPES: [(&str, &[i32]); 7] = [
    ("Major", &[4, 7]),
    ("Minor", &[3, 7]),
    ("Maj7", &[4, 7, 11]),
    ("Min7", &[3, 7, 10]),
    ("Dim", &[3, 6]),
    ("Sus4", &[5, 7]),
    ("Sus2", &[2, 7]),
];

pub const ROOT_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// Scale name + its degrees as semitones from the root, one octave's
/// worth. `note_for` wraps the 16 pads through this list (not raw
/// chromatic semitones) once a non-Chromatic scale is picked -- pad
/// rank 0 is scale degree 1, rank `len` is degree 1 an octave up, etc.
/// Chromatic (all 12 semitones) is first/default so the pads are fully
/// chromatic until you deliberately pick a scale.
pub const SCALE_TYPES: [(&str, &[i32]); 8] = [
    ("Chromatic", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
    ("Major", &[0, 2, 4, 5, 7, 9, 11]),
    ("Nat Minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("Dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("Mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    ("Harm Minor", &[0, 2, 3, 5, 7, 8, 11]),
    ("Maj Pentatonic", &[0, 2, 4, 7, 9]),
    ("Min Pentatonic", &[0, 3, 5, 7, 10]),
];

const ANALYZER_TYPE_NAMES: [&str; 4] = ["Spectrum", "Oscilloscope", "Level Meter", "Pitch Detect"];
/// Sentinel for "no pitch confidently detected" in `Params.detected_note`.
const NO_PITCH: i32 = i32::MIN;

const NUM_FIXED_TARGETS: u32 = 5; // Off, Harmonics, Timbre, Morph, Decay
/// Every modulator's own Rate is also selectable as a target -- one
/// modulator can speed up or slow down another (or, self-targeted,
/// itself -- a deliberate self-FM/chaos patch, not a bug). Off through
/// Decay are the low target values; a target at or past
/// NUM_FIXED_TARGETS addresses `Rate` on slot `target - NUM_FIXED_TARGETS`.
const TARGET_COUNT: u32 = NUM_FIXED_TARGETS + NUM_MOD_SLOTS as u32;
const MOD_SLOT_NAMES: [&str; NUM_MOD_SLOTS] =
    ["LFO 1", "LFO 2", "LFO 3", "LFO 4", "S&H", "Follower", "Ramp", "Drift", "Sequencer"];
/// How far a fully-deep rate-modulation can swing the target's rate, in
/// octaves either way -- rates span a ~400x range (0.05-20 Hz), so this
/// is multiplicative, not a flat +/- Hz offset.
const RATE_MOD_OCTAVES: f32 = 3.0;

fn target_name(target: u32) -> String {
    match target {
        0 => "Off".to_string(),
        1 => "Harmonics".to_string(),
        2 => "Timbre".to_string(),
        3 => "Morph".to_string(),
        4 => "Decay".to_string(),
        t => match rate_target_slot(t) {
            Some(slot) => format!("{} Rate", MOD_SLOT_NAMES[slot]),
            None => "Off".to_string(),
        },
    }
}

#[derive(Clone, Copy)]
enum ModKind {
    Lfo,
    SampleHold,
    EnvFollower,
    Ramp,
    Drift,
    Sequencer,
}
const MOD_SLOT_KINDS: [ModKind; NUM_MOD_SLOTS] = [
    ModKind::Lfo, ModKind::Lfo, ModKind::Lfo, ModKind::Lfo,
    ModKind::SampleHold, ModKind::EnvFollower, ModKind::Ramp, ModKind::Drift, ModKind::Sequencer,
];

/// Off/Harmonics/Timbre/Morph/Decay targets only -- see `rate_target_slot`
/// for the "another modulator's Rate" targets.
fn target_slot(target: u32) -> Option<usize> {
    match target {
        1 => Some(0),
        2 => Some(1),
        3 => Some(2),
        4 => Some(3),
        _ => None,
    }
}

/// If `target` addresses a modulator's Rate, the slot index it targets.
fn rate_target_slot(target: u32) -> Option<usize> {
    if (NUM_FIXED_TARGETS..TARGET_COUNT).contains(&target) {
        Some((target - NUM_FIXED_TARGETS) as usize)
    } else {
        None
    }
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.05).clamp(0.0, 1.0);
    value.set(next);
}

/// Every *editable* row (always a leaf inside some group, or inside a
/// modulator sub-group).
#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Engine,
    Harmonics,
    Timbre,
    Morph,
    Decay,
    Octave,
    VoiceMode,
    ChordMode,
    ChordType,
    Attack,
    Sustain,
    Release,
    ModTarget(usize),
    ModRate(usize),
    ModDepth(usize),
    RollVisible,
    RollHeight,
    RootNote,
    ScaleType,
    AnalyzerType,
    ArpOn,
    ArpPattern,
    ArpRate,
}

const NUM_GROUPS: usize = 7;
const ENGINE_GROUP: usize = 0;
const ENVELOPE_GROUP: usize = 1;
const MOD_GROUP: usize = 3;
const ANALYZER_GROUP: usize = 5;
const ARP_GROUP: usize = 6;

fn group_name(g: usize) -> &'static str {
    match g {
        0 => "Engine",
        1 => "Envelope",
        2 => "Voice",
        3 => "Modulators",
        4 => "Piano Roll",
        5 => "Analyzer",
        _ => "Arp",
    }
}

/// Leaves for every group except Modulators, whose children are the
/// per-slot sub-groups built directly in `visible_rows` instead.
fn group_leaves(g: usize) -> Vec<Selection> {
    match g {
        0 => vec![Selection::Engine, Selection::Harmonics, Selection::Timbre, Selection::Morph],
        1 => vec![
            Selection::Attack,
            Selection::Decay,
            Selection::Sustain,
            Selection::Release,
            Selection::Octave,
        ],
        2 => vec![Selection::VoiceMode, Selection::ChordMode, Selection::ChordType],
        4 => vec![Selection::RollVisible, Selection::RollHeight, Selection::RootNote, Selection::ScaleType],
        5 => vec![Selection::AnalyzerType],
        6 => vec![Selection::ArpOn, Selection::ArpPattern, Selection::ArpRate],
        _ => Vec::new(),
    }
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    /// A modulator's own sub-group header (its "subfolder") -- expands
    /// into that slot's Target/Rate/Depth leaves.
    ModSlot(usize),
    Leaf(Selection),
}

/// What the right-hand panel currently shows, driven entirely by where
/// the selection is -- no separate control for it. `Analyzer` covers all
/// 4 analyzer types (see ANALYZER_TYPE_NAMES) -- which one is just a
/// setting, not a separate PanelMode, so it's both the always-available
/// default panel and what the Analyzer group itself previews.
enum PanelMode {
    Analyzer,
    Adsr,
    Mod(usize),
    Engine,
    Key,
}

struct Params {
    engine: AtomicU32,
    harmonics: AtomicF32,
    timbre: AtomicF32,
    morph: AtomicF32,
    decay: AtomicF32,
    attack: AtomicF32,
    sustain: AtomicF32,
    release: AtomicF32,
    octave: AtomicI32,
    poly_mode: AtomicBool,
    chord_mode: AtomicBool,
    chord_type: AtomicU32,
    mod_target: [AtomicU32; NUM_MOD_SLOTS],
    mod_rate: [AtomicF32; NUM_MOD_SLOTS],
    mod_depth: [AtomicF32; NUM_MOD_SLOTS],
    /// Current output value of each modulator (pre-depth), for the
    /// panel's numeric readout.
    mod_value: [AtomicF32; NUM_MOD_SLOTS],
    /// Recent output history of each modulator, for the panel's
    /// scrolling waveform -- updated once per audio block regardless of
    /// whether that slot's target is "Off".
    mod_history: [Mutex<VecDeque<f32>>; NUM_MOD_SLOTS],
    /// Live outer-ADSR level/stage (see AdsrState), for the Envelope
    /// panel's moving dot. In Poly mode this mirrors voice 0 only -- a
    /// deliberate simplification, since 16 independent envelopes can't
    /// all be shown at once.
    env_level: AtomicF32,
    env_stage: AtomicU32,
    roll_visible: AtomicBool,
    roll_height: AtomicI32,
    /// Which key the 16 pads are constrained to -- see `note_for` and
    /// SCALE_TYPES. `root_note` is a pitch class (0=C..11=B).
    root_note: AtomicU32,
    scale_type: AtomicU32,
    note: AtomicF32,
    gate: AtomicBool,
    held: Mutex<[bool; 16]>,
    /// Which of ANALYZER_TYPE_NAMES the right panel shows by default
    /// (and what the Analyzer group's own leaf previews).
    analyzer_type: AtomicU32,
    /// Fixed-size (not `Vec`) so publishing any of these from the audio
    /// thread every block never allocates.
    spectrum: Mutex<[f32; ANALYZER_BARS]>,
    waveform: Mutex<[f32; ANALYZER_FFT_SIZE]>,
    peak_db: AtomicF32,
    rms_db: AtomicF32,
    detected_note: AtomicI32,
    /// External modulation inputs -- see modbus.rs. Added into the same
    /// `offsets[]` computation as the 9 internal modulators, on top of
    /// the base knob value; a source app (Pam's) writes into these,
    /// and they're live regardless of whether Plaits' own screen is
    /// the one on screen, since every app's processor always runs now.
    ext_harmonics: Arc<AtomicF32>,
    ext_timbre: Arc<AtomicF32>,
    ext_morph: Arc<AtomicF32>,
    ext_decay: Arc<AtomicF32>,
    /// This app's rendered mono output, republished every block for
    /// another app (Clouds) to tap -- see audio_bus.rs.
    bus_out: Arc<Mutex<Vec<f32>>>,
    /// This app's channel fader in the Mixer app, plus its own
    /// modulation input -- see mixer_bus.rs. Multiplied into the
    /// final output right before it's written to the device buffer.
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    /// Real, sample-block-stepped arpeggiator -- see arpeggiator.rs.
    /// Stepped once per audio block inside `PlaitsProcessor::process`
    /// (the real-time thread), not `tick` -- see there for how its
    /// output overrides the note/gate that would otherwise play.
    arp: Arpeggiator,
}

impl Params {
    fn new(default_roll_height: i32, modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Plaits", modbus);
        Self {
            engine: AtomicU32::new(8), // start on "Virtual Analog"
            harmonics: AtomicF32::new(0.5),
            timbre: AtomicF32::new(0.5),
            morph: AtomicF32::new(0.5),
            decay: AtomicF32::new(0.5),
            attack: AtomicF32::new(0.0),
            sustain: AtomicF32::new(1.0),
            release: AtomicF32::new(0.3),
            octave: AtomicI32::new(0),
            poly_mode: AtomicBool::new(false),
            chord_mode: AtomicBool::new(false),
            chord_type: AtomicU32::new(0),
            mod_target: std::array::from_fn(|_| AtomicU32::new(0)),
            mod_rate: std::array::from_fn(|_| AtomicF32::new(1.0)),
            mod_depth: std::array::from_fn(|_| AtomicF32::new(0.0)),
            mod_value: std::array::from_fn(|_| AtomicF32::new(0.0)),
            mod_history: std::array::from_fn(|_| Mutex::new(VecDeque::with_capacity(MOD_HISTORY_LEN))),
            env_level: AtomicF32::new(0.0),
            env_stage: AtomicU32::new(0),
            roll_visible: AtomicBool::new(true),
            roll_height: AtomicI32::new(default_roll_height),
            root_note: AtomicU32::new(0),
            scale_type: AtomicU32::new(0),
            note: AtomicF32::new(BASE_NOTE as f32),
            gate: AtomicBool::new(false),
            held: Mutex::new([false; 16]),
            analyzer_type: AtomicU32::new(1), // Oscilloscope ("scope") -- the default view
            spectrum: Mutex::new([MIN_DB; ANALYZER_BARS]),
            waveform: Mutex::new([0.0; ANALYZER_FFT_SIZE]),
            peak_db: AtomicF32::new(MIN_DB),
            rms_db: AtomicF32::new(MIN_DB),
            detected_note: AtomicI32::new(NO_PITCH),
            ext_harmonics: modbus.register("Plaits: Harmonics"),
            ext_timbre: modbus.register("Plaits: Timbre"),
            ext_morph: modbus.register("Plaits: Morph"),
            ext_decay: modbus.register("Plaits: Decay"),
            bus_out: audio_bus.register("Plaits"),
            mix_level,
            ext_mix_level,
            arp: Arpeggiator::new(),
        }
    }

    /// `rank` is 0=lowest..15=highest, NOT the physical pad index -- see
    /// `pad_rank` for the physical-index -> rank conversion. Wraps
    /// through the selected scale's degrees rather than raw chromatic
    /// semitones -- see SCALE_TYPES; Chromatic (the default) makes this
    /// identical to the old fully-chromatic behavior.
    fn note_for(&self, rank: i32) -> i32 {
        let octave = self.octave.load(Ordering::Relaxed);
        let root = self.root_note.load(Ordering::Relaxed) as i32;
        let scale_idx = self.scale_type.load(Ordering::Relaxed) as usize % SCALE_TYPES.len();
        let intervals = SCALE_TYPES[scale_idx].1;
        let len = intervals.len() as i32;
        let degree = rank.rem_euclid(len);
        let octave_offset = rank.div_euclid(len);
        let semitone = intervals[degree as usize];
        (BASE_NOTE + root + (octave + octave_offset) * 12 + semitone).clamp(NOTE_MIN, NOTE_MAX)
    }
}

/// `input.grid`/`held` are row-major with row 0 on top (see app.rs) --
/// physical pad 0 is top-left, pad 15 is bottom-right. Musically we want
/// the opposite vertical sense (lowest note bottom-left, highest
/// top-right), so this flips the row while keeping the column, mapping
/// a physical pad index to its pitch rank (0=lowest..15=highest). The
/// flip is its own inverse, so the same function converts rank back to
/// physical index too.
fn pad_rank(index: i32) -> i32 {
    let row = index / 4;
    let col = index % 4;
    (3 - row) * 4 + col
}

pub struct PlaitsApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    layout: LayoutWatcher,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    mod_expanded: [bool; NUM_MOD_SLOTS],
    current_index: Option<usize>,
}

// --- Plaits' own palette: warm cream and ink, not a device-wide
// theme -- the real Mutable Instruments panel look (a light body,
// dark screen-printed text), for the sim's flagship voice. Light-on-
// dark's usual "bright = lit" convention inverts here since the
// ground is light: PLAITS_ACCENT is a deep, saturated teal rather
// than a bright neon, so it still reads as bold against cream instead
// of washing out. `BANK_COLORS` (real per-engine-bank hues) are left
// untouched -- they're already this app's own distinct identity. ---

const PLAITS_BG: Rgb565 = Rgb565::new(23, 46, 22);
const PLAITS_TITLE: Rgb565 = Rgb565::new(3, 5, 2);
const PLAITS_ACCENT: Rgb565 = Rgb565::new(4, 22, 10);
const PLAITS_DIM: Rgb565 = Rgb565::new(9, 17, 7);
/// Frame/axis outlines -- a light-medium warm grey line, distinct
/// enough from `PLAITS_BG` to read as a thin outline on cream.
const PLAITS_OUTLINE: Rgb565 = Rgb565::new(22, 42, 19);
/// A dot/state that's off/inactive -- barely darker than the
/// background, same "nearly invisible" intent the old dark-on-black
/// version had, just inverted for a light ground.
const PLAITS_FAINT: Rgb565 = Rgb565::new(25, 50, 22);
/// A middle brightness step between `PLAITS_FAINT` and `PLAITS_ACCENT`
/// -- used where a 3-step (off/mid/lit) ramp existed before.
const PLAITS_MID: Rgb565 = Rgb565::new(13, 35, 15);

impl PlaitsApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let layout = LayoutWatcher::new();
        Self {
            params: Arc::new(Params::new(layout.get().default_roll_height, &modbus, &audio_bus, &mixer_bus)),
            sensitivity,
            nav_speed,
            layout,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
            mod_expanded: [false; NUM_MOD_SLOTS],
            current_index: None,
        }
    }

    /// Flat `(name, value, is_group)` rows -- the same data `draw()`
    /// builds for its own `ParamList`, just without handing out the
    /// private `Row`/`Selection` types themselves. `pub(crate)` (not
    /// fully private) specifically so an alternate *renderer* for
    /// this same real app/state -- e.g. a Slint screen driving the
    /// real `PlaitsApp` instead of `embedded_graphics` -- can read it
    /// without needing its own reimplementation of this list's
    /// structure. See `examples/slint_plaits_live.rs`.
    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", group_name(*g)), self.group_summary(*g), true)
                }
                Row::Leaf(sel) => (self.leaf_name(*sel), self.leaf_value(*sel), false),
                Row::ModSlot(slot) => {
                    let arrow = if self.mod_expanded[*slot] { "v" } else { ">" };
                    (format!("{arrow} Mod {}", slot + 1), self.mod_slot_summary(*slot), true)
                }
            })
            .collect()
    }

    /// Which row `display_rows` should show as selected.
    pub(crate) fn selected_row(&self) -> usize {
        self.list.selected
    }

    /// `display_rows`, windowed to at most `visible` rows around the
    /// current selection -- same sticky scrolling `ParamList::draw`
    /// already does for the real firmware's on-screen list (see
    /// `ParamList::centered_scroll_window`), just handed back as data instead
    /// of drawn. Returns `(window, selected_index_in_window,
    /// has_more_above, has_more_below)`.
    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }

    /// The current engine's display name (see `ENGINE_NAMES`).
    pub(crate) fn engine_name(&self) -> &'static str {
        ENGINE_NAMES[self.params.engine.load(Ordering::Relaxed) as usize % ENGINE_NAMES.len()]
    }

    /// Whether a note is actually sounding right now (mono voice
    /// gated on) -- real, not a guess: the same `current_index` the
    /// audio processor itself reads.
    pub(crate) fn is_sounding(&self) -> bool {
        self.current_index.is_some()
    }

    /// The current engine's index within its bank (0..BANK_SIZE) and
    /// its bank index (0..NUM_BANKS) -- same split `draw_engine_panel`
    /// uses for the real firmware's bank/LED display.
    pub(crate) fn engine_bank_and_led(&self) -> (usize, usize) {
        let engine = self.params.engine.load(Ordering::Relaxed) as usize % ENGINE_NAMES.len();
        (engine / BANK_SIZE, engine % BANK_SIZE)
    }

    /// The real FFT spectrum bars of Plaits' own output, normalized to
    /// 0..1 -- same source `draw_spectrum_panel` reads (`Params.
    /// spectrum`, filled by the audio thread), just handed back as
    /// plain floats instead of drawn directly.
    pub(crate) fn spectrum_levels(&self) -> Vec<f32> {
        self.params
            .spectrum
            .lock()
            .unwrap()
            .iter()
            .map(|db| ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0))
            .collect()
    }

    /// Which of `ANALYZER_TYPE_NAMES` the right panel should currently
    /// show (0=Spectrum, 1=Oscilloscope, 2=Level Meter, 3=Pitch
    /// Detect) plus its display name -- same selector
    /// `draw_current_analyzer` reads (`Params.analyzer_type`).
    pub(crate) fn analyzer_kind(&self) -> (u32, &'static str) {
        let idx = self.params.analyzer_type.load(Ordering::Relaxed) % ANALYZER_TYPE_NAMES.len() as u32;
        (idx, ANALYZER_TYPE_NAMES[idx as usize])
    }

    /// Real time-domain waveform, normalized -1..1 -- same source
    /// `draw_oscilloscope_panel` reads (`Params.waveform`).
    pub(crate) fn waveform_samples(&self) -> Vec<f32> {
        self.params.waveform.lock().unwrap().iter().map(|v| v.clamp(-1.0, 1.0)).collect()
    }

    /// Real peak/RMS level in dB, normalized to 0..1 (same MIN_DB/
    /// MAX_DB range `draw_level_panel` uses) -- `(peak, rms)`.
    pub(crate) fn level_meters(&self) -> (f32, f32) {
        let norm = |db: f32| ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
        (norm(self.params.peak_db.get()), norm(self.params.rms_db.get()))
    }

    /// The autocorrelation-detected note name, or "--" if nothing is
    /// confidently detected -- same source `draw_pitch_panel` reads.
    pub(crate) fn detected_note_name(&self) -> String {
        let note = self.params.detected_note.load(Ordering::Relaxed);
        if note == NO_PITCH { "--".to_string() } else { note_name(note) }
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for g in 0..NUM_GROUPS {
            rows.push(Row::Group(g));
            if !self.expanded[g] {
                continue;
            }
            if g == MOD_GROUP {
                for slot in 0..NUM_MOD_SLOTS {
                    rows.push(Row::ModSlot(slot));
                    if self.mod_expanded[slot] {
                        rows.push(Row::Leaf(Selection::ModTarget(slot)));
                        rows.push(Row::Leaf(Selection::ModRate(slot)));
                        rows.push(Row::Leaf(Selection::ModDepth(slot)));
                    }
                }
            } else {
                for sel in group_leaves(g) {
                    rows.push(Row::Leaf(sel));
                }
            }
        }
        rows
    }

    /// What the right-hand panel should show for the current selection.
    fn panel_mode(&self, rows: &[Row]) -> PanelMode {
        match rows.get(self.list.selected) {
            Some(Row::Group(g)) if *g == ENGINE_GROUP => PanelMode::Engine,
            Some(Row::Leaf(Selection::Engine | Selection::Harmonics | Selection::Timbre | Selection::Morph)) => {
                PanelMode::Engine
            }
            Some(Row::Group(g)) if *g == ENVELOPE_GROUP => PanelMode::Adsr,
            Some(Row::Leaf(Selection::Attack | Selection::Decay | Selection::Sustain | Selection::Release)) => {
                PanelMode::Adsr
            }
            Some(Row::ModSlot(slot)) => PanelMode::Mod(*slot),
            Some(Row::Leaf(Selection::ModTarget(slot) | Selection::ModRate(slot) | Selection::ModDepth(slot))) => {
                PanelMode::Mod(*slot)
            }
            Some(Row::Leaf(Selection::RootNote | Selection::ScaleType)) => PanelMode::Key,
            Some(Row::Group(g)) if *g == ANALYZER_GROUP => PanelMode::Analyzer,
            _ => PanelMode::Analyzer,
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Engine => {
                ENGINE_NAMES[self.params.engine.load(Ordering::Relaxed) as usize % ENGINE_NAMES.len()].to_string()
            }
            Selection::Harmonics => format!("{:.2}", self.params.harmonics.get()),
            Selection::Timbre => format!("{:.2}", self.params.timbre.get()),
            Selection::Morph => format!("{:.2}", self.params.morph.get()),
            Selection::Decay => format!("{:.2}", self.params.decay.get()),
            Selection::Attack => format!("{:.2}", self.params.attack.get()),
            Selection::Sustain => format!("{:.2}", self.params.sustain.get()),
            Selection::Release => format!("{:.2}", self.params.release.get()),
            Selection::Octave => format!("{:+}", self.params.octave.load(Ordering::Relaxed)),
            Selection::VoiceMode => {
                if self.params.poly_mode.load(Ordering::Relaxed) { "Poly".into() } else { "Mono".into() }
            }
            Selection::ChordMode => {
                if self.params.poly_mode.load(Ordering::Relaxed) {
                    "n/a (Poly)".into()
                } else if self.params.chord_mode.load(Ordering::Relaxed) {
                    "ON".into()
                } else {
                    "off".into()
                }
            }
            Selection::ChordType => {
                CHORD_TYPES[self.params.chord_type.load(Ordering::Relaxed) as usize % CHORD_TYPES.len()].0.to_string()
            }
            Selection::ModTarget(slot) => target_name(self.params.mod_target[slot].load(Ordering::Relaxed)),
            Selection::ModRate(slot) => format!("{:.2} Hz", self.params.mod_rate[slot].get()),
            Selection::ModDepth(slot) => format!("{:.2}", self.params.mod_depth[slot].get()),
            Selection::RollVisible => {
                if self.params.roll_visible.load(Ordering::Relaxed) { "shown".into() } else { "hidden".into() }
            }
            Selection::RollHeight => format!("{}px", self.params.roll_height.load(Ordering::Relaxed)),
            Selection::RootNote => ROOT_NAMES[self.params.root_note.load(Ordering::Relaxed) as usize % 12].to_string(),
            Selection::ScaleType => {
                SCALE_TYPES[self.params.scale_type.load(Ordering::Relaxed) as usize % SCALE_TYPES.len()].0.to_string()
            }
            Selection::AnalyzerType => {
                let idx = self.params.analyzer_type.load(Ordering::Relaxed) as usize % ANALYZER_TYPE_NAMES.len();
                ANALYZER_TYPE_NAMES[idx].to_string()
            }
            Selection::ArpOn => {
                if self.params.arp.enabled.load(Ordering::Relaxed) { "On".into() } else { "Off".into() }
            }
            Selection::ArpPattern => {
                let idx = self.params.arp.pattern.load(Ordering::Relaxed) as usize % ARP_PATTERN_NAMES.len();
                ARP_PATTERN_NAMES[idx].to_string()
            }
            Selection::ArpRate => format!("{:.1} Hz", self.params.arp.rate_hz.get()),
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Engine => "Engine".into(),
            Selection::Harmonics | Selection::Timbre | Selection::Morph => {
                let idx = self.params.engine.load(Ordering::Relaxed) as usize % ENGINE_NAMES.len();
                let names = ENGINE_PARAM_NAMES[idx];
                match sel {
                    Selection::Harmonics => names[0].into(),
                    Selection::Timbre => names[1].into(),
                    _ => names[2].into(),
                }
            }
            Selection::Decay => "Decay".into(),
            Selection::Attack => "Attack".into(),
            Selection::Sustain => "Sustain".into(),
            Selection::Release => "Release".into(),
            Selection::Octave => "Octave".into(),
            Selection::VoiceMode => "Voice Mode".into(),
            Selection::ChordMode => "Chord Mode".into(),
            Selection::ChordType => "Chord Type".into(),
            Selection::ModTarget(slot) => format!("{} Target", MOD_SLOT_NAMES[slot]),
            Selection::ModRate(slot) => format!("{} Rate", MOD_SLOT_NAMES[slot]),
            Selection::ModDepth(slot) => format!("{} Depth", MOD_SLOT_NAMES[slot]),
            Selection::RollVisible => "Show".into(),
            Selection::RollHeight => "Height".into(),
            Selection::RootNote => "Root".into(),
            Selection::ScaleType => "Scale".into(),
            Selection::AnalyzerType => "Type".into(),
            Selection::ArpOn => "On/Off".into(),
            Selection::ArpPattern => "Pattern".into(),
            Selection::ArpRate => "Rate".into(),
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => self.leaf_value(Selection::Engine),
            1 => format!("Decay {:.2}", self.params.decay.get()),
            2 => self.leaf_value(Selection::VoiceMode),
            3 => {
                let active = (0..NUM_MOD_SLOTS)
                    .filter(|&s| self.params.mod_target[s].load(Ordering::Relaxed) != 0)
                    .count();
                format!("{active} active")
            }
            6 => self.leaf_value(Selection::ArpOn),
            _ => self.leaf_value(Selection::RollVisible),
        }
    }

    fn mod_slot_summary(&self, slot: usize) -> String {
        let t = self.params.mod_target[slot].load(Ordering::Relaxed);
        if t == 0 { "off".into() } else { format!("-> {}", target_name(t)) }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::Engine => {
                let cur = self.params.engine.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(NUM_ENGINES as i32);
                self.params.engine.store(next as u32, Ordering::Relaxed);
            }
            Selection::Harmonics => bump(&self.params.harmonics, delta, sensitivity),
            Selection::Timbre => bump(&self.params.timbre, delta, sensitivity),
            Selection::Morph => bump(&self.params.morph, delta, sensitivity),
            Selection::Decay => bump(&self.params.decay, delta, sensitivity),
            Selection::Attack => bump(&self.params.attack, delta, sensitivity),
            Selection::Sustain => bump(&self.params.sustain, delta, sensitivity),
            Selection::Release => bump(&self.params.release, delta, sensitivity),
            Selection::Octave => {
                let cur = self.params.octave.load(Ordering::Relaxed);
                self.params.octave.store(cur + step, Ordering::Relaxed);
            }
            Selection::VoiceMode => self.params.poly_mode.store(delta > 0, Ordering::Relaxed),
            Selection::ChordMode => self.params.chord_mode.store(delta > 0, Ordering::Relaxed),
            Selection::ChordType => {
                let cur = self.params.chord_type.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(CHORD_TYPES.len() as i32);
                self.params.chord_type.store(next as u32, Ordering::Relaxed);
            }
            Selection::ModTarget(slot) => {
                let cur = self.params.mod_target[slot].load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(TARGET_COUNT as i32);
                self.params.mod_target[slot].store(next as u32, Ordering::Relaxed);
            }
            Selection::ModRate(slot) => {
                let cur = self.params.mod_rate[slot].get();
                let next = (cur + accelerate(delta) * sensitivity * 0.2).clamp(0.05, 20.0);
                self.params.mod_rate[slot].set(next);
            }
            Selection::ModDepth(slot) => bump(&self.params.mod_depth[slot], delta, sensitivity),
            Selection::RollVisible => {
                self.params.roll_visible.store(delta > 0, Ordering::Relaxed);
            }
            Selection::RollHeight => {
                let cur = self.params.roll_height.load(Ordering::Relaxed);
                let next = (cur + step * 5).clamp(MIN_ROLL_HEIGHT, MAX_ROLL_HEIGHT);
                self.params.roll_height.store(next, Ordering::Relaxed);
            }
            Selection::RootNote => {
                let cur = self.params.root_note.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(12);
                self.params.root_note.store(next as u32, Ordering::Relaxed);
            }
            Selection::ScaleType => {
                let cur = self.params.scale_type.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(SCALE_TYPES.len() as i32);
                self.params.scale_type.store(next as u32, Ordering::Relaxed);
            }
            Selection::AnalyzerType => {
                let cur = self.params.analyzer_type.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(ANALYZER_TYPE_NAMES.len() as i32);
                self.params.analyzer_type.store(next as u32, Ordering::Relaxed);
            }
            Selection::ArpOn => self.params.arp.enabled.store(delta > 0, Ordering::Relaxed),
            Selection::ArpPattern => {
                let cur = self.params.arp.pattern.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(ARP_PATTERN_NAMES.len() as i32);
                self.params.arp.pattern.store(next as u32, Ordering::Relaxed);
            }
            Selection::ArpRate => {
                let cur = self.params.arp.rate_hz.get();
                let next = (cur + accelerate(delta) * sensitivity * 0.2).clamp(0.5, 30.0);
                self.params.arp.rate_hz.set(next);
            }
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Harmonics => self.params.harmonics.set(0.5),
            Selection::Timbre => self.params.timbre.set(0.5),
            Selection::Morph => self.params.morph.set(0.5),
            Selection::Decay => self.params.decay.set(0.5),
            Selection::Attack => self.params.attack.set(0.0),
            Selection::Sustain => self.params.sustain.set(1.0),
            Selection::Release => self.params.release.set(0.3),
            Selection::Octave => self.params.octave.store(0, Ordering::Relaxed),
            Selection::VoiceMode => self.params.poly_mode.store(false, Ordering::Relaxed),
            Selection::ChordMode => self.params.chord_mode.store(false, Ordering::Relaxed),
            Selection::ChordType => self.params.chord_type.store(0, Ordering::Relaxed),
            Selection::ModTarget(slot) => self.params.mod_target[slot].store(0, Ordering::Relaxed),
            Selection::ModRate(slot) => self.params.mod_rate[slot].set(1.0),
            Selection::ModDepth(slot) => self.params.mod_depth[slot].set(0.0),
            Selection::RollVisible => self.params.roll_visible.store(true, Ordering::Relaxed),
            Selection::RollHeight => {
                self.params.roll_height.store(self.layout.get().default_roll_height, Ordering::Relaxed)
            }
            Selection::RootNote => self.params.root_note.store(0, Ordering::Relaxed),
            Selection::ScaleType => self.params.scale_type.store(0, Ordering::Relaxed),
            Selection::AnalyzerType => self.params.analyzer_type.store(1, Ordering::Relaxed), // Oscilloscope
            Selection::ArpOn => self.params.arp.enabled.store(false, Ordering::Relaxed),
            Selection::ArpPattern => self.params.arp.pattern.store(0, Ordering::Relaxed), // Up
            Selection::ArpRate => self.params.arp.rate_hz.set(8.0),
            Selection::Engine => {} // no sensible single "default" engine to reset to
        }
    }

    /// The 24 engines as 3 banks of 8, echoing the real Plaits module's
    /// LED indicator that changes color per bank -- a legend swatch
    /// per bank up top (the active one lit, the others dim), then the
    /// 8 LEDs in a single row centered below the legend (only the
    /// current engine's LED lit, in its bank's color), and the engine
    /// name right under that. Whatever vertical space is left below
    /// all of that is real estate this static indicator was never
    /// using, so it's handed to a live view of the current analyzer
    /// (`draw_current_analyzer`) -- picking an engine while still
    /// seeing the spectrum/scope/levels react to it, rather than
    /// having to leave this panel to check.
    fn draw_engine_panel(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, dim: MonoTextStyle<Rgb565>) {
        let engine = self.params.engine.load(Ordering::Relaxed) as usize % ENGINE_NAMES.len();
        let bank = engine / BANK_SIZE;
        let led_index = engine % BANK_SIZE;

        let legend_row_h = 14;
        for (i, name) in BANK_NAMES.iter().enumerate() {
            let sy = y + i as i32 * legend_row_h;
            let active = i == bank;
            let color = if active { BANK_COLORS[i] } else { PLAITS_FAINT };
            let text_style = if active { MonoTextStyle::new(&SPLEEN_6X12, BANK_COLORS[i]) } else { dim };
            Circle::new(Point::new(x, sy), 8).into_styled(PrimitiveStyle::with_fill(color)).draw(fb).ok();
            Text::new(name, Point::new(x + 14, sy + 8), text_style).draw(fb).ok();
        }

        // 8 LEDs in a horizontal row, centered under the legend --
        // compact on purpose, so most of the panel's height is left
        // for the analyzer below rather than a tall, mostly-empty
        // column.
        let led_d = 14i32;
        let led_gap = 10i32;
        let row_w = BANK_SIZE as i32 * led_d + (BANK_SIZE as i32 - 1) * led_gap;
        let row_x = x + (w - row_w) / 2;
        let leds_y = y + BANK_NAMES.len() as i32 * legend_row_h + 14;
        for i in 0..BANK_SIZE {
            let lx = row_x + i as i32 * (led_d + led_gap);
            let lit = i == led_index;
            let color = if lit { BANK_COLORS[bank] } else { PLAITS_FAINT };
            Circle::new(Point::new(lx, leds_y), led_d as u32).into_styled(PrimitiveStyle::with_fill(color)).draw(fb).ok();
        }

        let name_y = leds_y + led_d + 16;
        let name_text = format!("{} ({}/{})", ENGINE_NAMES[engine], engine + 1, ENGINE_NAMES.len());
        let name_w = name_text.len() as i32 * 6; // SPLEEN_6X12's advance width
        Text::new(&name_text, Point::new(x + (w - name_w) / 2, name_y), dim).draw(fb).ok();

        let analyzer_y = name_y + 14;
        let analyzer_h = (y + h - analyzer_y).max(0);
        if analyzer_h > 20 {
            self.draw_current_analyzer(fb, x, analyzer_y, w, analyzer_h, dim);
        }
    }

    /// The classic ADSR trapezoid, dimly drawn in full, with the segment
    /// matching the live stage picked out in accent and a dot riding at
    /// the actual live level -- see Params::env_level/env_stage.
    fn draw_adsr_panel(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, dim: MonoTextStyle<Rgb565>) {
        let a = adsr_time(self.params.attack.get(), 1.0);
        let d = adsr_time(self.params.decay.get(), 2.0);
        let s = self.params.sustain.get().clamp(0.0, 1.0);
        let r = adsr_time(self.params.release.get(), 2.0);
        // Sustain has no real "duration" -- give it a representative
        // slice of the plot just so the shape reads as ADSR, not ADR.
        let sustain_visual = (a + d + r) * 0.4;
        let total = (a + d + sustain_visual + r).max(0.001);

        let plot_h = (h - 16).max(10);
        let base_y = y + plot_h;
        let aw = (a / total * w as f32) as i32;
        let dw = (d / total * w as f32) as i32;
        let sw = (sustain_visual / total * w as f32) as i32;
        let rw = ((r / total * w as f32) as i32).max(1);

        let p0 = Point::new(x, base_y);
        let p1 = Point::new(x + aw, y);
        let p2 = Point::new(x + aw + dw, y + ((1.0 - s) * plot_h as f32) as i32);
        let p3 = Point::new(x + aw + dw + sw, p2.y);
        let p4 = Point::new((x + aw + dw + sw + rw).min(x + w), base_y);

        let dim_line = PrimitiveStyle::with_stroke(PLAITS_OUTLINE, 1);
        let accent_line = PrimitiveStyle::with_stroke(PLAITS_ACCENT, 2);

        let segs = [
            (p0, p1, AdsrStage::Attack),
            (p1, p2, AdsrStage::Decay),
            (p2, p3, AdsrStage::Sustain),
            (p3, p4, AdsrStage::Release),
        ];
        let live_stage = adsr_stage_from_code(self.params.env_stage.load(Ordering::Relaxed));
        for (from, to, stage) in segs {
            let style = if stage == live_stage { accent_line } else { dim_line };
            Line::new(from, to).into_styled(style).draw(fb).ok();
        }

        if live_stage != AdsrStage::Idle {
            if let Some((seg_from, seg_to, _)) = segs.iter().find(|(_, _, st)| *st == live_stage) {
                let mid_x = (seg_from.x + seg_to.x) / 2;
                let level = self.params.env_level.get().clamp(0.0, 1.0);
                let dot_y = base_y - (level * plot_h as f32) as i32;
                Circle::new(Point::new(mid_x - 3, dot_y - 3), 6)
                    .into_styled(PrimitiveStyle::with_fill(PLAITS_ACCENT))
                    .draw(fb)
                    .ok();
            }
        }

        Text::new(&format!("A {a:.2}s  D {d:.2}s  S {s:.2}  R {r:.2}s"), Point::new(x, y + h - 4), dim)
            .draw(fb)
            .ok();
    }

    /// A scrolling waveform of one modulator's recent output, plus its
    /// current target and live value -- works the same for every
    /// modulator kind, LFO/S&H/Follower/Ramp/Drift/Sequencer alike.
    fn draw_mod_panel(
        &self,
        fb: &mut FrameBuffer,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        slot: usize,
        dim: MonoTextStyle<Rgb565>,
    ) {
        let target = self.params.mod_target[slot].load(Ordering::Relaxed);
        let value = self.params.mod_value[slot].get();
        Text::new(&format!("-> {}   {:.2}", target_name(target), value), Point::new(x, y + h - 4), dim)
            .draw(fb)
            .ok();

        let plot_h = (h - 16).max(10);
        let mid_y = y + plot_h / 2;
        Line::new(Point::new(x, mid_y), Point::new(x + w, mid_y))
            .into_styled(PrimitiveStyle::with_stroke(PLAITS_OUTLINE, 1))
            .draw(fb)
            .ok();

        let history = self.params.mod_history[slot].lock().unwrap();
        if history.len() >= 2 {
            let step = w as f32 / (history.len() - 1) as f32;
            let style = PrimitiveStyle::with_stroke(PLAITS_ACCENT, 1);
            for i in 0..history.len() - 1 {
                let v0 = history[i].clamp(-1.0, 1.0);
                let v1 = history[i + 1].clamp(-1.0, 1.0);
                let p0 = Point::new(x + (i as f32 * step) as i32, mid_y - (v0 * plot_h as f32 / 2.0) as i32);
                let p1 = Point::new(x + ((i + 1) as f32 * step) as i32, mid_y - (v1 * plot_h as f32 / 2.0) as i32);
                Line::new(p0, p1).into_styled(style).draw(fb).ok();
            }
        }
    }

    /// Dispatches to whichever of the 4 analyzer views `AnalyzerType`
    /// currently selects -- shared by the Analyzer panel itself and
    /// `draw_engine_panel`'s bottom half (see its doc comment).
    fn draw_current_analyzer(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, dim: MonoTextStyle<Rgb565>) {
        let analyzer_idx = self.params.analyzer_type.load(Ordering::Relaxed) as usize % ANALYZER_TYPE_NAMES.len();
        match analyzer_idx {
            0 => self.draw_spectrum_panel(fb, x, y, w, h),
            1 => self.draw_oscilloscope_panel(fb, x, y, w, h),
            2 => self.draw_level_panel(fb, x, y, w, h, dim),
            _ => self.draw_pitch_panel(fb, x, y, w, h, dim),
        }
    }

    /// FFT magnitude bars of Plaits' own output -- the original/default
    /// analyzer view.
    fn draw_spectrum_panel(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32) {
        let bars = *self.params.spectrum.lock().unwrap();
        let bar_w = (w / ANALYZER_BARS as i32).max(1);
        for (i, db) in bars.iter().enumerate() {
            let t = ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
            let bh = (t * h as f32) as u32;
            if bh == 0 {
                continue;
            }
            let bx = x + i as i32 * bar_w;
            let by = y + h - bh as i32;
            Rectangle::new(Point::new(bx, by), Size::new((bar_w - 1).max(1) as u32, bh))
                .into_styled(PrimitiveStyle::with_fill(PLAITS_ACCENT))
                .draw(fb)
                .ok();
        }
    }

    /// A scrolling time-domain waveform of Plaits' own output, straight
    /// off `Params.waveform` (the same raw samples the spectrum's FFT
    /// runs on, just not frequency-transformed).
    fn draw_oscilloscope_panel(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32) {
        let mid_y = y + h / 2;
        Line::new(Point::new(x, mid_y), Point::new(x + w, mid_y))
            .into_styled(PrimitiveStyle::with_stroke(PLAITS_OUTLINE, 1))
            .draw(fb)
            .ok();

        let wf = self.params.waveform.lock().unwrap();
        let step = w as f32 / (wf.len() - 1) as f32;
        let style = PrimitiveStyle::with_stroke(PLAITS_ACCENT, 1);
        for i in 0..wf.len() - 1 {
            let v0 = wf[i].clamp(-1.0, 1.0);
            let v1 = wf[i + 1].clamp(-1.0, 1.0);
            let p0 = Point::new(x + (i as f32 * step) as i32, mid_y - (v0 * h as f32 / 2.0) as i32);
            let p1 = Point::new(x + ((i + 1) as f32 * step) as i32, mid_y - (v1 * h as f32 / 2.0) as i32);
            Line::new(p0, p1).into_styled(style).draw(fb).ok();
        }
    }

    /// Peak and RMS level meters, updated every block (not gated on the
    /// FFT window filling) so it reacts as fast as possible.
    fn draw_level_panel(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, _h: i32, dim: MonoTextStyle<Rgb565>) {
        let peak = self.params.peak_db.get();
        let rms = self.params.rms_db.get();
        let bar_h = 24;
        let gap = 16;

        for (i, (label, db)) in [("Peak", peak), ("RMS", rms)].into_iter().enumerate() {
            let by = y + i as i32 * (bar_h + gap);
            let t = ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0);
            let filled = (t * w as f32) as u32;
            Rectangle::new(Point::new(x, by), Size::new(w as u32, bar_h as u32))
                .into_styled(PrimitiveStyle::with_stroke(PLAITS_OUTLINE, 1))
                .draw(fb)
                .ok();
            if filled > 0 {
                Rectangle::new(Point::new(x, by), Size::new(filled.min(w as u32), bar_h as u32))
                    .into_styled(PrimitiveStyle::with_fill(PLAITS_ACCENT))
                    .draw(fb)
                    .ok();
            }
            Text::new(&format!("{label}: {db:.1} dB"), Point::new(x, by + bar_h + 12), dim).draw(fb).ok();
        }
    }

    /// The autocorrelation-detected fundamental, as a note name -- the
    /// same technique the old standalone Analyzer app used, just fed by
    /// Plaits' real output instead of a demo signal.
    fn draw_pitch_panel(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, dim: MonoTextStyle<Rgb565>) {
        let note = self.params.detected_note.load(Ordering::Relaxed);
        let big = MonoTextStyle::new(&SPLEEN_16X32, PLAITS_ACCENT);
        let text = if note == NO_PITCH { "--".to_string() } else { note_name(note) };
        Text::new(&text, Point::new(x + w / 2 - 24, y + h / 2), big).draw(fb).ok();
        Text::new("(autocorrelation)", Point::new(x, y + h - 4), dim).draw(fb).ok();
    }

    /// A pitch-class wheel (C at top, clockwise) -- the root lit
    /// brightest, other scale members dimmer-lit, non-members outline
    /// only. Answers "which keys will be selected for that key/scale"
    /// before you commit to it.
    fn draw_key_panel(&self, fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, dim: MonoTextStyle<Rgb565>) {
        let root = self.params.root_note.load(Ordering::Relaxed) as i32;
        let scale_idx = self.params.scale_type.load(Ordering::Relaxed) as usize % SCALE_TYPES.len();
        let (scale_name, intervals) = SCALE_TYPES[scale_idx];
        let members: Vec<i32> = intervals.iter().map(|iv| (root + iv).rem_euclid(12)).collect();

        let label_h = 16;
        let cx = x + w / 2;
        let cy = y + (h - label_h) / 2;
        let radius = ((w.min(h - label_h)) / 2 - 14).max(20) as f32;

        for pc in 0..12i32 {
            let angle = (pc as f32 * 30.0 - 90.0).to_radians();
            let px = cx + (radius * angle.cos()) as i32;
            let py = cy + (radius * angle.sin()) as i32;
            let is_root = pc == root;
            let is_member = members.contains(&pc);
            let (color, d) = if is_root {
                (PLAITS_ACCENT, 12)
            } else if is_member {
                (PLAITS_MID, 9)
            } else {
                (PLAITS_FAINT, 6)
            };
            Circle::new(Point::new(px - d / 2, py - d / 2), d as u32)
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(fb)
                .ok();
            let lx = px + if angle.cos() >= 0.0 { d / 2 + 2 } else { -d - 8 };
            Text::new(ROOT_NAMES[pc as usize], Point::new(lx, py + 4), dim).draw(fb).ok();
        }

        Text::new(&format!("{} {}", ROOT_NAMES[root as usize], scale_name), Point::new(x, y + h - 4), dim)
            .draw(fb)
            .ok();
    }
}

impl App for PlaitsApp {
    fn supports_pad_lock(&self) -> bool { true }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.selected_row()
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        self.windowed_rows(visible)
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let (engine_bank, engine_led) = self.engine_bank_and_led();
        let (analyzer_kind, analyzer_name) = self.analyzer_kind();
        let mut spectrum = Vec::new();
        let mut waveform = crate::app::CurveSegments::default();
        let mut peak_level = 0.0;
        let mut rms_level = 0.0;
        let mut pitch_name = String::new();
        match analyzer_kind {
            0 => spectrum = self.spectrum_levels(),
            1 => {
                // Preserve every captured sample, including the final endpoint.
                // SignalTrace scales this reference geometry to the Slint panel.
                const PANEL_W: f32 = 260.0;
                const PANEL_H: f32 = 190.0;
                let raw = self.waveform_samples();
                if raw.len() >= 2 {
                    let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(&raw, PANEL_W, PANEL_H, true);
                    waveform = crate::app::CurveSegments { mid_x, mid_y, length, angle_deg };
                }
            }
            2 => (peak_level, rms_level) = self.level_meters(),
            _ => pitch_name = self.detected_note_name(),
        }
        crate::app::SlintExtra::Plaits(crate::app::PlaitsExtra {
            engine_name: self.engine_name().to_string(),
            engine_bank,
            engine_led,
            analyzer_kind,
            analyzer_name: analyzer_name.to_string(),
            spectrum,
            waveform,
            peak_level,
            rms_level,
            pitch_name,
        })
    }

    fn tick(&mut self, input: &Input) {
        self.layout.poll();
        let rows = self.visible_rows();
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();

        if input.knob1_press {
            match current {
                Some(Row::Group(g)) => self.expanded[g] = !self.expanded[g],
                Some(Row::ModSlot(slot)) => self.mod_expanded[slot] = !self.mod_expanded[slot],
                _ => {}
            }
        }
        if let Some(Row::Leaf(sel)) = current {
            self.edit(sel, input.knob2);
            if input.knob2_press {
                self.reset(sel);
            }
        }

        let poly = self.params.poly_mode.load(Ordering::Relaxed);

        let mut newly_pressed = None;
        {
            let mut held = self.params.held.lock().unwrap();
            for (i, pressed) in input.grid.iter().enumerate() {
                if *pressed && !held[i] {
                    newly_pressed = Some(i);
                }
                held[i] = *pressed;
            }
        }

        if poly {
            let held = self.params.held.lock().unwrap();
            self.current_index = held.iter().position(|&h| h);
            return;
        }

        // Mono: last-pressed-and-still-held key wins (matching real
        // Plaits, a single-voice module).
        if let Some(i) = newly_pressed {
            self.current_index = Some(i);
            self.params.note.set(self.params.note_for(pad_rank(i as i32)) as f32);
            self.params.gate.store(true, Ordering::Relaxed);
        } else {
            let held = self.params.held.lock().unwrap();
            let current_still_held = self.current_index.map(|i| held[i]).unwrap_or(false);
            if !current_still_held {
                if let Some(i) = held.iter().position(|&h| h) {
                    self.current_index = Some(i);
                    self.params.note.set(self.params.note_for(pad_rank(i as i32)) as f32);
                } else {
                    self.current_index = None;
                    self.params.gate.store(false, Ordering::Relaxed);
                }
            }
        }
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(ANALYZER_FFT_SIZE);
        Some(Box::new(PlaitsProcessor {
            params: Arc::clone(&self.params),
            voice: PlaitsVoice::new(),
            chord_voices: std::array::from_fn(|_| PlaitsVoice::new()),
            poly_voices: std::array::from_fn(|_| PlaitsVoice::new()),
            mod_runtime: std::array::from_fn(|i| ModRuntime::new(i as u32)),
            mono_adsr: AdsrState::default(),
            poly_adsr: [AdsrState::default(); 16],
            fft,
            history: VecDeque::with_capacity(ANALYZER_FFT_SIZE),
            fft_buf: [Complex::new(0.0, 0.0); ANALYZER_FFT_SIZE],
            mono_buf: Vec::new(),
            scratch_buf: Vec::new(),
            extra_buf: Vec::new(),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(PLAITS_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, PLAITS_TITLE);
        Text::new("Plaits", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_8X16, PLAITS_ACCENT);
        let small_dim = MonoTextStyle::new(&SPLEEN_6X12, PLAITS_DIM);

        if let Some(i) = self.current_index {
            Text::new(
                &format!("Playing: {}", note_name(self.params.note_for(pad_rank(i as i32)))),
                Point::new(360, 24),
                accent,
            )
            .draw(fb)
            .ok();
        }

        // The roll/content area's bottom edge stops well above the
        // hint line at y=337 (a real gap, not just "less than 340") --
        // it used to end flush at 340, which put the roll strip's
        // bottom ~10px directly under the hint text's ascent, so the
        // hint was rendering right on top of the roll every time it
        // was visible (caught by actually rendering and looking, not
        // by any test -- nothing here measures cross-app text/roll
        // collisions the way the list-vs-panel overlap tests do).
        const ROLL_BOTTOM: i32 = 322;
        let roll_visible = self.params.roll_visible.load(Ordering::Relaxed);
        let roll_height = self.params.roll_height.load(Ordering::Relaxed);
        let content_bottom = ROLL_BOTTOM - if roll_visible { roll_height + 10 } else { 0 };

        // --- Left: the dropdown menu ---
        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", group_name(*g)), self.group_summary(*g))
                }
                Row::ModSlot(slot) => {
                    let arrow = if self.mod_expanded[*slot] { "v" } else { ">" };
                    (format!("  {arrow} {}", MOD_SLOT_NAMES[*slot]), self.mod_slot_summary(*slot))
                }
                Row::Leaf(sel) => {
                    let indent = if matches!(sel, Selection::ModTarget(_) | Selection::ModRate(_) | Selection::ModDepth(_)) {
                        "        "
                    } else {
                        "    "
                    };
                    (format!("{indent}{}", self.leaf_name(*sel)), self.leaf_value(*sel))
                }
            })
            .collect();
        let layout = self.layout.get();
        const MENU_ROW_H: i32 = 24; // taller row to fit ParamList's SPLEEN_8X16 -- see paramlist.rs
        let visible_menu_rows = (((content_bottom - layout.menu_y) / MENU_ROW_H).max(1)) as usize;
        self.list.draw_themed(fb, layout.menu_x, layout.menu_y, MENU_ROW_H, visible_menu_rows, &display_rows, PLAITS_BG, PLAITS_DIM, PLAITS_ACCENT);

        // --- Right: context-sensitive visualizer -- Plaits' output
        // spectrum by default, or a live view of the ADSR/modulator
        // currently under the cursor (see panel_mode).
        let panel_x = layout.panel_x;
        let panel_y = layout.panel_y;
        let panel_w = layout.panel_w;
        let panel_h = (content_bottom - panel_y).max(20);
        let panel_mode = self.panel_mode(&rows);
        let analyzer_idx = self.params.analyzer_type.load(Ordering::Relaxed) as usize % ANALYZER_TYPE_NAMES.len();
        let panel_title = match panel_mode {
            PanelMode::Analyzer => ANALYZER_TYPE_NAMES[analyzer_idx].to_string(),
            PanelMode::Adsr => "Envelope".to_string(),
            PanelMode::Mod(slot) => MOD_SLOT_NAMES[slot].to_string(),
            PanelMode::Engine => "Model".to_string(),
            PanelMode::Key => "Key".to_string(),
        };
        Text::new(&panel_title, Point::new(panel_x, panel_y - 4), small_dim).draw(fb).ok();
        match panel_mode {
            PanelMode::Analyzer => self.draw_current_analyzer(fb, panel_x, panel_y, panel_w, panel_h, small_dim),
            PanelMode::Adsr => self.draw_adsr_panel(fb, panel_x, panel_y, panel_w, panel_h, small_dim),
            PanelMode::Mod(slot) => self.draw_mod_panel(fb, panel_x, panel_y, panel_w, panel_h, slot, small_dim),
            PanelMode::Engine => self.draw_engine_panel(fb, panel_x, panel_y, panel_w, panel_h, small_dim),
            PanelMode::Key => self.draw_key_panel(fb, panel_x, panel_y, panel_w, panel_h, small_dim),
        }

        // --- Bottom: piano roll, hidable and resizable (both settings
        // live under the "Piano Roll" dropdown group above) ---
        if roll_visible {
            let roll_y = ROLL_BOTTOM - roll_height;
            let held = *self.params.held.lock().unwrap();
            let key_w = 640 / 16;
            for i in 0..16 {
                let note = self.params.note_for(i);
                let is_black = BLACK_KEYS[note.rem_euclid(12) as usize];
                // The strip is drawn in ascending-pitch (rank) order, but
                // `held` is indexed by physical pad -- pad_rank converts.
                let on = held[pad_rank(i) as usize];
                let fill = if on {
                    PLAITS_ACCENT
                } else if is_black {
                    PLAITS_FAINT
                } else {
                    PLAITS_MID
                };
                Rectangle::new(Point::new(i * key_w, roll_y), Size::new((key_w - 1) as u32, roll_height as u32))
                    .into_styled(PrimitiveStyle::with_fill(fill))
                    .draw(fb)
                    .ok();
            }
        }

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) | Some(Row::ModSlot(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), small_dim).draw(fb).ok();
    }
}

/// Deterministic xorshift32 step, factored out of ModRuntime so it can
/// also seed a modulator's initial state (e.g. Sequencer's pattern)
/// before `Self` exists to call a method on.
fn next_rand_from(rng: &mut u32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    (*rng as f32 / u32::MAX as f32) * 2.0 - 1.0
}

struct ModRuntime {
    phase: f32,
    sh_value: f32,
    sh_timer: f32,
    follower: f32,
    drift: f32,
    seq_pattern: [f32; SEQ_STEPS],
    seq_step: usize,
    seq_timer: f32,
    rng: u32,
}

impl ModRuntime {
    fn new(seed: u32) -> Self {
        let mut rng = 0x9E3779B9 ^ (seed.wrapping_mul(0x85EBCA6B) | 1);
        let mut seq_pattern = [0.0; SEQ_STEPS];
        for v in seq_pattern.iter_mut() {
            *v = next_rand_from(&mut rng);
        }
        Self {
            phase: 0.0,
            sh_value: 0.0,
            sh_timer: 0.0,
            follower: 0.0,
            drift: 0.0,
            seq_pattern,
            seq_step: 0,
            seq_timer: 0.0,
            rng,
        }
    }

    fn next_rand(&mut self) -> f32 {
        next_rand_from(&mut self.rng)
    }
}

#[derive(Default, Clone, Copy, PartialEq)]
enum AdsrStage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

fn adsr_stage_code(stage: AdsrStage) -> u32 {
    match stage {
        AdsrStage::Idle => 0,
        AdsrStage::Attack => 1,
        AdsrStage::Decay => 2,
        AdsrStage::Sustain => 3,
        AdsrStage::Release => 4,
    }
}

fn adsr_stage_from_code(code: u32) -> AdsrStage {
    match code {
        1 => AdsrStage::Attack,
        2 => AdsrStage::Decay,
        3 => AdsrStage::Sustain,
        4 => AdsrStage::Release,
        _ => AdsrStage::Idle,
    }
}

/// A standard block-rate ADSR, applied as an outer amplitude envelope on
/// top of whatever Plaits' own internal envelope/LPG already does --
/// this is a separate stage, not a replacement for it. One instance
/// drives the mono/chord voices together (they share one gate); Poly
/// mode gives each of the 16 voices its own, since they gate
/// independently.
#[derive(Default, Clone, Copy)]
struct AdsrState {
    stage: AdsrStage,
    level: f32,
    prev_gate: bool,
}

impl AdsrState {
    /// `attack`/`decay`/`release` in seconds, `sustain` a 0..1 level.
    fn step(&mut self, gate: bool, attack: f32, decay: f32, sustain: f32, release: f32, dt: f32) -> f32 {
        if gate && !self.prev_gate {
            self.stage = AdsrStage::Attack;
        } else if !gate && self.prev_gate {
            self.stage = AdsrStage::Release;
        }
        self.prev_gate = gate;

        match self.stage {
            AdsrStage::Idle => self.level = 0.0,
            AdsrStage::Attack => {
                self.level = (self.level + dt / attack.max(0.001)).min(1.0);
                if self.level >= 1.0 {
                    self.stage = AdsrStage::Decay;
                }
            }
            AdsrStage::Decay => {
                self.level = (self.level - dt * (1.0 - sustain) / decay.max(0.001)).max(sustain);
                if self.level <= sustain {
                    self.stage = AdsrStage::Sustain;
                }
            }
            AdsrStage::Sustain => self.level = sustain,
            AdsrStage::Release => {
                self.level = (self.level - dt / release.max(0.001)).max(0.0);
                if self.level <= 0.0 {
                    self.stage = AdsrStage::Idle;
                }
            }
        }
        self.level
    }
}

/// Maps a 0..1 knob value to a musically useful time range.
fn adsr_time(knob_value: f32, max_seconds: f32) -> f32 {
    0.001 + knob_value * max_seconds
}

struct PlaitsProcessor {
    params: Arc<Params>,
    voice: PlaitsVoice,
    chord_voices: [PlaitsVoice; MAX_CHORD_VOICES],
    poly_voices: [PlaitsVoice; 16],
    mod_runtime: [ModRuntime; NUM_MOD_SLOTS],
    mono_adsr: AdsrState,
    poly_adsr: [AdsrState; 16],
    fft: Arc<dyn Fft<f32>>,
    history: VecDeque<f32>,
    // Reused block-to-block instead of being allocated fresh every call
    // -- a `vec![...]` (or `.collect()`) inside the audio callback is a
    // classic source of the jitter/glitches heard once enough of these
    // piled up (9 modulators, poly voices, chord voices, FFT analysis,
    // all added incrementally without revisiting this).
    fft_buf: [Complex<f32>; ANALYZER_FFT_SIZE],
    mono_buf: Vec<f32>,
    scratch_buf: Vec<f32>,
    extra_buf: Vec<f32>,
}

impl PlaitsProcessor {
    /// Everything here needs the FFT window full, hence the early
    /// return -- spectrum, oscilloscope snapshot, and pitch detection
    /// all read from the same ANALYZER_FFT_SIZE-sample window. Peak/RMS
    /// (see `process`) don't wait on this since they only need the
    /// current block.
    fn analyze(&mut self, sample_rate: f32) {
        if self.history.len() < ANALYZER_FFT_SIZE {
            return;
        }
        for (i, s) in self.history.iter().enumerate() {
            let w = 0.5 - 0.5 * (TAU * i as f32 / (ANALYZER_FFT_SIZE - 1) as f32).cos();
            self.fft_buf[i] = Complex::new(*s * w, 0.0);
        }
        self.fft.process(&mut self.fft_buf);

        let bins_per_bar = (ANALYZER_FFT_SIZE / 2) / ANALYZER_BARS;
        let mut bars = self.params.spectrum.lock().unwrap();
        for (bar, group) in bars.iter_mut().zip(self.fft_buf[..ANALYZER_FFT_SIZE / 2].chunks(bins_per_bar.max(1))) {
            let mag = group.iter().map(|c| c.norm()).fold(0.0f32, f32::max);
            *bar = (20.0 * (mag / ANALYZER_FFT_SIZE as f32 + 1e-9).log10()).clamp(MIN_DB, MAX_DB);
        }
        drop(bars);

        {
            let mut wf = self.params.waveform.lock().unwrap();
            for (i, s) in self.history.iter().enumerate() {
                wf[i] = *s;
            }
        }

        // Autocorrelation pitch detection over the same window --
        // `make_contiguous` rearranges the VecDeque's existing storage
        // in place rather than collecting a fresh Vec, so this stays
        // allocation-free on the audio thread.
        let samples = self.history.make_contiguous();
        let energy: f32 = samples.iter().map(|s| s * s).sum();
        let min_lag = (sample_rate / 2000.0).max(2.0) as usize;
        let max_lag = ((sample_rate / 40.0) as usize).min(samples.len() - 1);
        let mut best_lag = 0;
        let mut best_corr = 0.0f32;
        if max_lag > min_lag {
            for lag in min_lag..max_lag {
                let mut corr = 0.0;
                for i in 0..(samples.len() - lag) {
                    corr += samples[i] * samples[i + lag];
                }
                if corr > best_corr {
                    best_corr = corr;
                    best_lag = lag;
                }
            }
        }
        if best_lag > 0 && energy > 1e-6 && best_corr / energy > 0.35 {
            let freq = sample_rate / best_lag as f32;
            let note = (12.0 * (freq / 440.0).log2() + 69.0).round() as i32;
            self.params.detected_note.store(note, Ordering::Relaxed);
        } else {
            self.params.detected_note.store(NO_PITCH, Ordering::Relaxed);
        }
    }
}

impl AudioProcessor for PlaitsProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let engine = self.params.engine.load(Ordering::Relaxed) as i32;
        let harmonics_base = self.params.harmonics.get();
        let timbre_base = self.params.timbre.get();
        let morph_base = self.params.morph.get();
        let decay_base = self.params.decay.get();
        let poly = self.params.poly_mode.load(Ordering::Relaxed);
        let chord_mode = self.params.chord_mode.load(Ordering::Relaxed);
        let chord_type = self.params.chord_type.load(Ordering::Relaxed) as usize % CHORD_TYPES.len();

        let frames = buffer.len() / channels;
        let frames_f = frames as f32;

        // Real arpeggiator step -- sample-block-accurate (this is the
        // real-time thread), so it's stepped here rather than in
        // `tick`. Held pads are converted to pitch-rank order first
        // (see `pad_rank`) so Up/Down walk ascending/descending pitch,
        // not raw physical pad index.
        let held_raw = *self.params.held.lock().unwrap();
        let held_by_rank: [bool; 16] = std::array::from_fn(|r| held_raw[pad_rank(r as i32) as usize]);
        let arp_rank = self.params.arp.step(&held_by_rank, frames_f / sample_rate);

        // Mono: when the arp is on and something is held, it overrides
        // the last-pressed-wins note/gate `tick` already set; disabled
        // (or nothing held), behavior is unchanged.
        let (note, trigger) = if let Some(r) = arp_rank {
            (self.params.note_for(r as i32) as f32, true)
        } else {
            (self.params.note.get(), self.params.gate.load(Ordering::Relaxed))
        };
        // Poly: when the arp is on, only the current step's pad gates
        // -- a chord's worth of held pads becomes a rolling arpeggio
        // instead of all sounding at once; disabled, every held pad
        // still gates simultaneously (unchanged chord behavior).
        let effective_held: [bool; 16] = match arp_rank {
            Some(r) => {
                let idx = pad_rank(r as i32) as usize;
                std::array::from_fn(|i| i == idx)
            }
            None => held_raw,
        };

        let mut offsets = [0.0f32; 4]; // harmonics, timbre, morph, decay
        let any_gate = if poly { effective_held.iter().any(|h| *h) } else { trigger };
        // A modulator's target can be another modulator's Rate (see
        // rate_target_slot/target_name) -- snapshot every slot's output
        // from *last* block before this block overwrites any of them, so
        // the rate-modulation pass below uses a consistent set of values
        // regardless of which slot in the main loop runs first.
        let prev_values: [f32; NUM_MOD_SLOTS] = std::array::from_fn(|i| self.params.mod_value[i].get());
        let mut rate_octaves = [0.0f32; NUM_MOD_SLOTS];
        for j in 0..NUM_MOD_SLOTS {
            let target = self.params.mod_target[j].load(Ordering::Relaxed);
            if let Some(k) = rate_target_slot(target) {
                let depth = self.params.mod_depth[j].get();
                rate_octaves[k] += depth * prev_values[j] * RATE_MOD_OCTAVES;
            }
        }

        // Every modulator runs (and records history for its panel) even
        // when its target is "Off", so you can preview one before
        // wiring it up.
        for slot in 0..NUM_MOD_SLOTS {
            let base_rate = self.params.mod_rate[slot].get().max(0.01);
            // Rate modulation is multiplicative (octaves), not a flat Hz
            // offset -- rates span a ~400x range, so a fixed +/- offset
            // would be meaningless at the low end and huge at the high
            // end. Self-targeting (a slot modulating its own Rate) is
            // allowed on purpose -- a self-FM/chaos patch, not a bug.
            let rate = (base_rate * rate_octaves[slot].exp2()).clamp(0.05, 20.0);
            let depth = self.params.mod_depth[slot].get();
            let rt = &mut self.mod_runtime[slot];
            let value = match MOD_SLOT_KINDS[slot] {
                ModKind::Lfo => {
                    rt.phase = (rt.phase + rate * frames_f / sample_rate).fract();
                    (rt.phase * TAU).sin()
                }
                ModKind::Ramp => {
                    rt.phase = (rt.phase + rate * frames_f / sample_rate).fract();
                    rt.phase * 2.0 - 1.0
                }
                ModKind::SampleHold => {
                    rt.sh_timer -= frames_f / sample_rate;
                    if rt.sh_timer <= 0.0 {
                        rt.sh_value = rt.next_rand();
                        rt.sh_timer = 1.0 / rate;
                    }
                    rt.sh_value
                }
                ModKind::Sequencer => {
                    rt.seq_timer -= frames_f / sample_rate;
                    if rt.seq_timer <= 0.0 {
                        rt.seq_step = (rt.seq_step + 1) % SEQ_STEPS;
                        rt.seq_timer = 1.0 / rate;
                    }
                    rt.seq_pattern[rt.seq_step]
                }
                ModKind::EnvFollower => {
                    let target_level = if any_gate { 1.0 } else { 0.0 };
                    let coef = 1.0 - (-frames_f / sample_rate / (1.0 / rate)).exp();
                    rt.follower += (target_level - rt.follower) * coef;
                    rt.follower
                }
                ModKind::Drift => {
                    rt.drift += rt.next_rand() * rate * (frames_f / sample_rate) * 0.5;
                    rt.drift = rt.drift.clamp(-1.0, 1.0);
                    rt.drift
                }
            };

            self.params.mod_value[slot].set(value);
            {
                let mut hist = self.params.mod_history[slot].lock().unwrap();
                hist.push_back(value);
                if hist.len() > MOD_HISTORY_LEN {
                    hist.pop_front();
                }
            }

            let target = self.params.mod_target[slot].load(Ordering::Relaxed);
            if let Some(t) = target_slot(target) {
                offsets[t] += depth * value;
            }
        }
        // External modulation (see modbus.rs) stacks on top of the 9
        // internal modulators the same way -- just another offset.
        let harmonics = (harmonics_base + offsets[0] + self.params.ext_harmonics.get()).clamp(0.0, 1.0);
        let timbre = (timbre_base + offsets[1] + self.params.ext_timbre.get()).clamp(0.0, 1.0);
        let morph = (morph_base + offsets[2] + self.params.ext_morph.get()).clamp(0.0, 1.0);
        let decay = (decay_base + offsets[3] + self.params.ext_decay.get()).clamp(0.0, 1.0);

        // Outer ADSR (see AdsrState) -- block-rate, applied on top of
        // whatever Plaits' own internal envelope already did.
        let attack_s = adsr_time(self.params.attack.get(), 1.0);
        let decay_s = adsr_time(decay, 2.0);
        let sustain_level = self.params.sustain.get();
        let release_s = adsr_time(self.params.release.get(), 2.0);
        let dt = frames_f / sample_rate;

        let make_params = |note: f32, gate: bool| PlaitsParams {
            engine,
            note,
            harmonics,
            timbre,
            morph,
            decay,
            lpg_colour: LPG_COLOUR,
            trigger: gate,
        };

        // Reused block-to-block (see the PlaitsProcessor field comments)
        // instead of a fresh `vec![0.0; frames]` every call -- `resize`
        // is a no-op once capacity settles, since `frames` is constant
        // for a given device stream.
        self.mono_buf.clear();
        self.mono_buf.resize(frames, 0.0);

        if poly {
            self.scratch_buf.clear();
            self.scratch_buf.resize(frames, 0.0);
            let mut active = 0u32;
            for (i, voice) in self.poly_voices.iter_mut().enumerate() {
                let gate = effective_held[i];
                if gate {
                    active += 1;
                }
                let poly_note = self.params.note_for(pad_rank(i as i32)) as f32;
                voice.render(&mut self.scratch_buf, sample_rate, &make_params(poly_note, gate));
                let env = self.poly_adsr[i].step(gate, attack_s, decay_s, sustain_level, release_s, dt);
                for (m, s) in self.mono_buf.iter_mut().zip(self.scratch_buf.iter()) {
                    *m += *s * env;
                }
            }
            let headroom = active.max(1) as f32;
            for m in self.mono_buf.iter_mut() {
                *m /= headroom;
            }
            // Poly mode has 16 independent envelopes -- the panel can
            // only show one, so voice 0 stands in as a representative.
            self.params.env_level.set(self.poly_adsr[0].level);
            self.params.env_stage.store(adsr_stage_code(self.poly_adsr[0].stage), Ordering::Relaxed);
        } else {
            self.voice.render(&mut self.mono_buf, sample_rate, &make_params(note, trigger));

            if chord_mode {
                let intervals = CHORD_TYPES[chord_type].1;
                self.extra_buf.clear();
                self.extra_buf.resize(frames, 0.0);
                let mut voice_count = 1;
                for (voice, interval) in self.chord_voices.iter_mut().zip(intervals) {
                    voice.render(&mut self.extra_buf, sample_rate, &make_params(note + *interval as f32, trigger));
                    for (m, e) in self.mono_buf.iter_mut().zip(self.extra_buf.iter()) {
                        *m += *e;
                    }
                    voice_count += 1;
                }
                for m in self.mono_buf.iter_mut() {
                    *m /= voice_count as f32;
                }
            }

            // Chord voices share the root's gate, so one envelope
            // instance covers the whole mixed chord correctly.
            let env = self.mono_adsr.step(trigger, attack_s, decay_s, sustain_level, release_s, dt);
            self.params.env_level.set(env);
            self.params.env_stage.store(adsr_stage_code(self.mono_adsr.stage), Ordering::Relaxed);
            for m in self.mono_buf.iter_mut() {
                *m *= env;
            }
        }

        let mut peak = 0.0f32;
        let mut sum_sq = 0.0f32;
        for s in &self.mono_buf {
            peak = peak.max(s.abs());
            sum_sq += s * s;
            self.history.push_back(*s);
            if self.history.len() > ANALYZER_FFT_SIZE {
                self.history.pop_front();
            }
        }
        let rms = (sum_sq / self.mono_buf.len().max(1) as f32).sqrt();
        self.params.peak_db.set((20.0 * (peak + 1e-9).log10()).clamp(MIN_DB, MAX_DB));
        self.params.rms_db.set((20.0 * (rms + 1e-9).log10()).clamp(MIN_DB, MAX_DB));
        self.analyze(sample_rate);

        {
            let mut bus_out = self.params.bus_out.lock().unwrap();
            bus_out.clear();
            bus_out.extend_from_slice(&self.mono_buf);
        }

        // The Mixer app's channel fader for this app -- applied only
        // to what reaches the device, not to `bus_out` above, so
        // another app tapping this one via audio_bus.rs hears the
        // pre-fader signal regardless of where the Mixer fader sits.
        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        for (frame, sample) in buffer.chunks_mut(channels).zip(self.mono_buf.iter()) {
            for out in frame.iter_mut() {
                *out = *sample * mix_level;
            }
        }
    }
}
