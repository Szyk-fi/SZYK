//! Independent, lazily constructed instruments from the portable collection.
//! Shared DSP/media primitives do not share transport, parameters, or buffers.
//! Disk and network operations run on workers; the audio callback only exchanges
//! bounded commands and telemetry. Inputs are explicit mono AudioBus routes.
use crate::app::music_scales::{self, ScaleInfo, SCALE_TYPES, ROOT_NAMES};
use crate::app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes, Throw};
use crate::led_output::PadColor;
use crate::spleen_fonts::SPLEEN_6X12;
use crate::{
    app::{App, CollectionExtra, Input, SlintExtra},
    audio::AudioProcessor,
    audio_bus::{cycle_source, AudioBus, NO_SOURCE},
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    paramlist::ParamList,
    util::{note_name, AtomicF32},
};
use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::Rgb565,
    prelude::{Drawable, Point},
    text::Text,
};
use rustfft::{num_complex::Complex, FftPlanner};
use std::{
    f32::consts::TAU,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
        Arc, Mutex,
    },
};
#[path = "collection_radio.rs"]
mod radio;
#[path = "collection_portal.rs"]
pub mod portal;
#[path = "collection_decode.rs"]
pub(crate) mod decode;
#[path="collection_visuals.rs"]
mod visuals;
const VOICE_NAMES: [&str; 8] = [
    "Pure sine",
    "Glass FM",
    "Reed",
    "Plucked string",
    "Drawbar organ",
    "Round bass",
    "Live input",
    "Audio sample",
];
const RATE: f32 = 48000.0;
const CAP: usize = 48000 * 8;
/// Studio's tracks: a minute each (Tape's old looper length; Studio
/// replaced Tape). Reserved up front so recording never allocates on the
/// audio thread; on the device this is PSRAM, streamed to the SD card.
const STUDIO_CAP: usize = 48000 * 60;
/// Longest file the players load: half an hour at 48 kHz (whole albums'
/// worth of FLAC tracks fit; a DJ mix may not). Held as 16-bit (`Pcm`,
/// 4 bytes a frame, ~350 MB at the limit) -- the sim decodes whole files;
/// the device will stream from the SD card instead.
const MEDIA_LIMIT: usize = 48000 * 60 * 30;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Orbit,
    Fracture,
    Ghosts,
    Swarm,
    Mutant,
    Constellation,
    TapeMachine,
    Dream,
    Portal,
    Reference,
    Field,
    SampleHunter,
    Studio,
    Scope,
    Vinyl,
    Practice,
    Radio,
    Master,
    Memories,
}
pub const APPS: &[(Kind, &str, &str)] = &[
    (Kind::Orbit, "orbit", "Orbit"),
    (Kind::Fracture, "fracture", "Fracture"),
    (Kind::Ghosts, "ghosts", "Ghosts"),
    (Kind::Swarm, "swarm", "Swarm"),
    (Kind::Mutant, "mutant", "Mutant"),
    (Kind::Constellation, "constellation", "Constellation"),
    (Kind::TapeMachine, "tape_machine", "Tape Machine"),
    (Kind::Dream, "dream", "Dream"),
    (Kind::Portal, "portal", "Portal"),
    (Kind::Reference, "reference", "Reference"),
    (Kind::Field, "field", "Field"),
    (Kind::SampleHunter, "sample_hunter", "Sample Hunter"),
    (Kind::Studio, "studio", "Studio"),
    (Kind::Scope, "scope", "Scope"),
    (Kind::Vinyl, "vinyl", "Vinyl"),
    (Kind::Practice, "practice", "Practice"),
    (Kind::Radio, "radio", "Radio"),
    (Kind::Master, "master", "Master"),
    (Kind::Memories, "memories", "Memories"),
];
impl Kind {
    fn synth(self) -> bool {
        matches!(
            self,
            Self::Orbit | Self::Swarm | Self::Mutant | Self::Constellation | Self::Dream
        )
    }
    fn media(self) -> bool {
        matches!(
            self,
            Self::Reference | Self::Vinyl | Self::Practice | Self::Radio | Self::Memories
        )
    }
    fn capture(self) -> bool {
        matches!(self, Self::Field | Self::SampleHunter | Self::Studio)
    }
    fn passive(self) -> bool {
        matches!(self, Self::Portal | Self::Scope | Self::Master)
    }
    /// Kinds that get a play view. Portal is a patch matrix and Scope an
    /// analyser: their pads route and inspect rather than make music, and
    /// neither has a knob worth performing, so they keep their menu-first
    /// screen untouched.
    fn playable(self) -> bool {
        !matches!(self, Self::Portal | Self::Scope)
    }
    /// Processors of another app's audio: these get a Throws layer.
    fn effect(self) -> bool {
        matches!(self, Self::Fracture | Self::Ghosts | Self::TapeMachine | Self::Master)
    }
    /// Kinds whose pads have two modes (the menu's "Pads" row). They map
    /// onto native layers 0 and 1, so F2 and the menu row stay in step.
    fn two_pad_modes(self) -> bool {
        self.synth() || matches!(self, Self::SampleHunter | Self::Studio)
    }
    fn id(self) -> &'static str {
        APPS.iter().find(|a| a.0 == self).unwrap().1
    }
    fn default_voice(self) -> usize {
        match self {
            Self::Orbit => 3,
            Self::Swarm => 2,
            Self::Mutant => 4,
            Self::Constellation => 4,
            Self::Dream => 1,
            _ => 0,
        }
    }
    fn default_pattern(self) -> usize {
        match self {
            Self::Swarm => 4,
            Self::Mutant => 3,
            Self::Constellation => 5,
            Self::Dream => 2,
            _ => 0,
        }
    }
    fn default_rhythm(self) -> usize {
        match self {
            Self::Swarm | Self::Dream => 2,
            Self::Mutant => 3,
            Self::Constellation => 1,
            _ => 0,
        }
    }
    /// One palette per family, so an instrument, an effect, a recorder and
    /// a player read as different things at a glance even though they
    /// share an engine.
    fn palette(self) -> kit::draw::Palette {
        let c = Rgb565::new;
        let (bg, ink, accent, dim, faint) = if self.synth() {
            // Night violet: the generative instruments.
            (c(2, 2, 6), c(28, 56, 31), c(23, 34, 31), c(13, 24, 20), c(5, 7, 11))
        } else if self.effect() {
            // Umber and amber: things that process another app's audio.
            (c(3, 3, 1), c(31, 58, 26), c(31, 42, 6), c(17, 28, 9), c(8, 9, 3))
        } else if self.capture() {
            // Coral on oxblood: the recorders, like a record lamp.
            (c(4, 1, 1), c(31, 56, 28), c(31, 22, 14), c(17, 22, 14), c(9, 5, 4))
        } else {
            // Teal: the library players.
            (c(1, 4, 4), c(26, 60, 30), c(8, 52, 26), c(9, 30, 19), c(3, 11, 10))
        };
        kit::draw::Palette { bg, ink, accent, dim, faint }
    }
    fn name(self) -> &'static str {
        APPS.iter().find(|a| a.0 == self).unwrap().2
    }
    fn labels(self) -> [&'static str; 6] {
        match self {
            Self::Orbit => [
                "Tempo",
                "Orbit ratio",
                "Gravity / pitch",
                "Gate length",
                "Probability",
                "Swing",
            ],
            Self::Fracture => [
                "Slice seconds",
                "Repeats",
                "Reverse",
                "Pitch semitones",
                "Scatter",
                "Wet",
            ],
            Self::Ghosts => [
                "Loop seconds",
                "Decay",
                "Drift",
                "Low pass",
                "Reverse",
                "Wet",
            ],
            Self::Swarm => [
                "Voices",
                "Cohesion",
                "Pitch spread",
                "Root note",
                "Mutation",
                "Release",
            ],
            Self::Mutant => [
                "Parent blend",
                "Mutation",
                "Harmonics",
                "Root note",
                "Brightness",
                "Release",
            ],
            Self::Constellation => [
                "Root note",
                "Octave spread",
                "Inversion",
                "Glide",
                "Tension",
                "Voices",
            ],
            Self::TapeMachine => [
                "Speed",
                "Head spacing",
                "Feedback",
                "Wow",
                "Flutter",
                "Drive",
            ],
            Self::Dream => [
                "Tempo",
                "Dwell bars",
                "Transition chance",
                "Complexity",
                "Tension",
                "Note chance",
            ],
            Self::Portal => [
                "Send gain",
                "Polarity invert",
                "High pass",
                "Low pass",
                "Delay ms",
                "Direct monitor",
            ],
            Self::Reference => [
                "Position %",
                "Gain dB",
                "Normalize peak",
                "Auto next",
                "Repeat",
                "Output",
            ],
            Self::Field => [
                "Input gain",
                "Pre-roll seconds",
                "Limiter",
                "Low cut Hz",
                "Monitor",
                "Output",
            ],
            Self::SampleHunter => [
                "Trim start %",
                "Trim end %",
                "Normalize",
                "Pitch semitones",
                "Gate threshold",
                "Monitor",
            ],
            Self::Studio => ["Track 1–8", "Track level", "Pan", "Mute", "Solo", "Monitor"],
            Self::Scope => [
                "Timebase ms",
                "Gain",
                "Trigger level",
                "FFT window",
                "Display mode",
                "Freeze",
            ],
            Self::Vinyl => [
                "Position %",
                "Speed",
                "Sort newest",
                "Auto next",
                "Repeat",
                "Output",
            ],
            Self::Practice => [
                "Speed",
                "Loop A %",
                "Loop B %",
                "Pitch semitones",
                "Click BPM",
                "Click mix",
            ],
            Self::Radio => [
                "Buffer ms",
                "Gain dB",
                "Limiter",
                "Auto next",
                "Repeat",
                "Output",
            ],
            Self::Master => [
                "A / B reference",
                "Auto match",
                "Bass dB",
                "Presence dB",
                "Drive",
                "Ceiling dB",
            ],
            Self::Memories => [
                "Position %",
                "Favorite",
                "Tag",
                "Auto next",
                "Repeat",
                "Output",
            ],
        }
    }
    fn defaults(self) -> [f32; 6] {
        match self {
            Self::Orbit => [110., 3., 48., 0.3, 1., 0.],
            Self::Fracture => [0.125, 3., 0., 0., 0., 0.8],
            Self::Ghosts => [2., 0.65, 0.1, 4000., 0., 0.7],
            Self::Swarm => [8., 0.7, 0.2, 48., 0.2, 0.4],
            Self::Mutant => [0.5, 0.2, 4., 48., 0.5, 0.4],
            Self::Constellation => [48., 1., 0., 0.05, 0., 4.],
            Self::TapeMachine => [1., 0.25, 0.5, 0.15, 0.1, 1.],
            Self::Dream => [100., 4., 0.75, 0.5, 0.3, 0.8],
            Self::Portal => [1., 0., 20., 18000., 0., 0.],
            Self::Reference => [0., 0., 0., 0., 0., 0.8],
            Self::Radio => [80., 0., 1., 0., 0., 0.8],
            Self::Field => [1., 1., 1., 40., 0., 0.8],
            Self::SampleHunter => [0., 100., 0., 0., 0., 0.],
            Self::Studio => [1., 0.8, 0., 0., 0., 0.],
            Self::Scope => [10., 1., 0., 1., 0., 0.],
            Self::Vinyl => [0., 1., 0., 0., 0., 0.8],
            Self::Practice => [1., 0., 100., 0., 0., 0.2],
            Self::Master => [0., 0., 0., 0., 1., -1.],
            Self::Memories => [0., 0., 0., 0., 0., 0.8],
        }
    }
    fn ranges(self) -> [(f32, f32, f32); 6] {
        match self {
            Self::Orbit => [
                (30., 240., 1.),
                (1., 8., 1.),
                (24., 84., 1.),
                (0.05, 0.95, 0.05),
                (0., 1., 0.05),
                (0., 0.45, 0.025),
            ],
            Self::Fracture => [
                (0.025, 1., 0.025),
                (1., 16., 1.),
                (0., 1., 1.),
                (-24., 24., 1.),
                (0., 1., 0.05),
                (0., 1., 0.05),
            ],
            Self::Ghosts => [
                (0.1, 4., 0.1),
                (0., 0.95, 0.05),
                (0., 1., 0.05),
                (100., 18000., 100.),
                (0., 1., 1.),
                (0., 1., 0.05),
            ],
            Self::Swarm => [
                (1., 16., 1.),
                (0., 1., 0.05),
                (0., 2., 0.05),
                (24., 84., 1.),
                (0., 1., 0.05),
                (0.05, 3., 0.05),
            ],
            Self::Mutant => [
                (0., 1., 0.05),
                (0., 1., 0.05),
                (1., 12., 1.),
                (24., 84., 1.),
                (0., 1., 0.05),
                (0.05, 3., 0.05),
            ],
            Self::Constellation => [
                (24., 84., 1.),
                (0., 3., 1.),
                (0., 3., 1.),
                (0., 0.5, 0.01),
                (0., 1., 0.05),
                (1., 8., 1.),
            ],
            Self::TapeMachine => [
                (0.25, 2., 0.05),
                (0.025, 1., 0.025),
                (0., 0.92, 0.02),
                (0., 1., 0.05),
                (0., 1., 0.05),
                (0.5, 4., 0.1),
            ],
            Self::Dream => [
                (30., 240., 1.),
                (1., 16., 1.),
                (0., 1., 0.05),
                (0., 1., 0.05),
                (0., 1., 0.05),
                (0., 1., 0.05),
            ],
            Self::Portal => [
                (0., 2., 0.05),
                (0., 1., 1.),
                (10., 2000., 10.),
                (200., 20000., 200.),
                (0., 1000., 10.),
                (0., 1., 1.),
            ],
            Self::Radio => [
                (20., 500., 10.),
                (-24., 12., 1.),
                (0., 1., 1.),
                (0., 1., 1.),
                (0., 1., 1.),
                (0., 1., 0.05),
            ],
            Self::Reference => [
                (0., 100., 1.),
                (-24., 12., 1.),
                (0., 1., 1.),
                (0., 1., 1.),
                (0., 1., 1.),
                (0., 1., 0.05),
            ],
            Self::Field => [
                (0., 4., 0.1),
                (0., 3., 0.25),
                (0., 1., 1.),
                (0., 200., 10.),
                (0., 1., 1.),
                (0., 1., 0.05),
            ],
            Self::SampleHunter => [
                (0., 99., 1.),
                (1., 100., 1.),
                (0., 1., 1.),
                (-24., 24., 1.),
                (0., 0.5, 0.01),
                (0., 1., 1.),
            ],
            Self::Studio => [
                (1., 8., 1.),
                (0., 1., 0.05),
                (-1., 1., 0.1),
                (0., 1., 1.),
                (0., 1., 1.),
                (0., 1., 1.),
            ],
            Self::Scope => [
                (1., 100., 1.),
                (0.1, 8., 0.1),
                (-1., 1., 0.05),
                (0., 1., 1.),
                (0., 9., 1.),
                (0., 1., 1.),
            ],
            Self::Vinyl => [
                (0., 100., 1.),
                (0.5, 2., 0.05),
                (0., 1., 1.),
                (0., 1., 1.),
                (0., 1., 1.),
                (0., 1., 0.05),
            ],
            Self::Practice => [
                (0.5, 1.5, 0.05),
                (0., 99., 1.),
                (1., 100., 1.),
                (-12., 12., 1.),
                (0., 240., 5.),
                (0., 1., 0.05),
            ],
            Self::Master => [
                (0., 1., 1.),
                (0., 1., 1.),
                (-12., 12., 1.),
                (-12., 12., 1.),
                (1., 4., 0.1),
                (-18., 0., 0.5),
            ],
            Self::Memories => [
                (0., 100., 1.),
                (0., 1., 1.),
                (0., 4., 1.),
                (0., 1., 1.),
                (0., 1., 1.),
                (0., 1., 0.05),
            ],
        }
    }
}
#[derive(Clone, Debug)]
/// Decoded stereo audio at 16 bits: half the memory of f32, so long files
/// fit, and 96 dB is more than playback needs.
#[derive(Default)]
pub(crate) struct Pcm(Vec<[i16; 2]>);

#[allow(dead_code)]
impl Pcm {
    pub(crate) fn with_capacity(n: usize) -> Self {
        Self(Vec::with_capacity(n))
    }
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn push(&mut self, f: [f32; 2]) {
        self.0.push(f.map(|x| (x.clamp(-1.0, 1.0) * 32767.0).round() as i16));
    }
    /// Frame `i` as floats (panics past the end, like indexing).
    pub(crate) fn at(&self, i: usize) -> [f32; 2] {
        self.0[i].map(|x| x as f32 / 32767.0)
    }
    pub(crate) fn get(&self, i: usize) -> Option<[f32; 2]> {
        self.0.get(i).map(|f| f.map(|x| x as f32 / 32767.0))
    }
    /// The first `max` frames as floats (tools that analyse a clip).
    pub(crate) fn frames(&self, max: usize) -> Vec<[f32; 2]> {
        self.0.iter().take(max).map(|f| f.map(|x| x as f32 / 32767.0)).collect()
    }
    /// Peak per bucket, for thumbnails (see `take_overview`).
    pub(crate) fn overview(&self, count: usize) -> Vec<f32> {
        let n = self.0.len();
        (0..count)
            .map(|i| {
                let start = i * n / count;
                let end = ((i + 1) * n / count).max(start + 1).min(n);
                if start >= end {
                    return 0.0;
                }
                self.0[start..end].iter().step_by(((end - start) / 16).max(1)).map(|f| f[0].unsigned_abs().max(f[1].unsigned_abs()) as f32 / 32767.0).fold(0f32, f32::max)
            })
            .collect()
    }
}

impl From<Vec<[f32; 2]>> for Pcm {
    fn from(v: Vec<[f32; 2]>) -> Self {
        let mut p = Pcm::with_capacity(v.len());
        for f in v {
            p.push(f);
        }
        p
    }
}

pub(crate) struct Clip {
    pub(crate) samples: Arc<Pcm>,
    pub(crate) rate: f32,
    pub(crate) name: String,
    pub(crate) peak: f32,
}
// Bounded peak sampling avoids zero-crossing aliasing in take thumbnails.
fn take_overview(samples: &[[f32; 2]], count: usize) -> Vec<f32> {
    (0..count)
        .map(|i| {
            let start = i * samples.len() / count;
            let end = ((i + 1) * samples.len() / count)
                .max(start + 1)
                .min(samples.len());
            samples[start..end]
                .iter()
                .step_by(((end - start) / 16).max(1))
                .map(|f| f[0].abs().max(f[1].abs()))
                .fold(0f32, f32::max)
        })
        .collect()
}
const PATTERNS: [&str;6] = ["Up", "Down", "Bounce", "Random", "Random walk", "Chords"];
const RHYTHMS: [&str;5] = ["Straight", "Offbeat", "Euclidean 5/8", "Sparse", "Bursts"];
fn rhythm_gate(rhythm:usize,step:usize)->bool {
    match rhythm%5 {0=>true,1=>step%4==2,2=>(step*5)%8<5,3=>[0,3,10].contains(&(step%16)),_=>step%8<3}
}
fn pattern_mask(pattern:usize,step:usize,count:usize,choice:usize,walk:&mut usize)->usize {
    let count=count.clamp(1,16);
    let index=match pattern%6 {0=>step%count,1=>count-1-step%count,2=>{let p=step%((count-1)*2).max(1);if p<count {p}else{(count-1)*2-p}},3=>choice%count,4=>{*walk=(*walk+count+if choice.is_multiple_of(3){count-1}else if choice%3==1{0}else{1})%count;*walk},_=>step/4%count};
    if pattern%6==5 { (1<<index)|(1<<((index+2)%count))|(1<<((index+4)%count)) } else {1<<index}
}
struct Shared {
    values: [AtomicF32; 6],
    modulation: [Arc<AtomicF32>; 6],
    color_modulation: Arc<AtomicF32>,
    voice: AtomicUsize,
    color: AtomicF32,
    attack: AtomicF32,
    release: AtomicF32,
    scale: AtomicUsize,
    dream_root: AtomicUsize,
    pattern: AtomicUsize,
    rhythm: AtomicUsize,
    octave: AtomicF32,
    motion_rate: AtomicF32,
    motion_depth: AtomicF32,
    alternate: AtomicBool,
    freeze: AtomicBool,
    variation: AtomicUsize,
    source: AtomicUsize,
    reference: AtomicUsize,
    play: AtomicBool,
    record: AtomicBool,
    pads: AtomicUsize,
    step: AtomicUsize,
    position: AtomicF32,
    duration: AtomicF32,
    ended: AtomicBool,
    audible: AtomicBool,
    loaded: AtomicBool,
    export_busy: AtomicBool,
    view: Mutex<View>,
    radio: Mutex<Option<Arc<radio::RadioStream>>>,
    out: Arc<Mutex<Vec<f32>>>,
    mix: Arc<AtomicF32>,
    ext: Arc<AtomicF32>,
}
#[derive(Default, Clone)]
#[allow(dead_code)] // not used by the main binary
struct View {
    wave: Vec<f32>,
    spectrum: Vec<f32>,
    levels: Vec<f32>,
    peak: f32,
    rms: f32,
    buffer: Vec<f32>,
    tracks: Vec<f32>,
    phases: Vec<f32>,
    notes: Vec<f32>,
    genes: Vec<f32>,
}
enum Command {
    Load(Clip),
    Seek(f32),
    Audition(i32),
    Clear,
    Save,
    Marker,
    Track(usize, [f32; 4]),
    Breed,
    Recycle(Vec<[f32; 2]>),
}
struct Take {
    samples: Vec<[f32; 2]>,
    rate: u32,
    normalize: bool,
    marker: usize,
}
struct WorkerResult {
    clip: Option<Clip>,
    files: Option<Vec<PathBuf>>,
    message: String,
    recycle: Option<Vec<[f32; 2]>>,
    journal: Option<(PathBuf, bool, usize)>,
}
#[allow(dead_code)] // not used by the main binary
pub struct CollectionApp {
    terrain: Vec<f32>,
    kind: Kind,
    p: Arc<Shared>,
    bus: Arc<AudioBus>,
    own: usize,
    nav: Arc<AtomicF32>,
    list: ParamList,
    held: [bool; 16],
    tx: SyncSender<Command>,
    rx: Option<Receiver<Command>>,
    takes: Receiver<Take>,
    take_tx: Option<SyncSender<Take>>,
    worker: Receiver<WorkerResult>,
    work_tx: Sender<WorkerResult>,
    files: Vec<PathBuf>,
    file: usize,
    status: String,
    busy: bool,
    tracks: [[f32; 4]; 8],
    last_track: usize,
    favorite: bool,
    tag: usize,
    auto_start: bool,
    loaded_file: Option<PathBuf>,
    /// The shared play view (play_kit.rs); unused by Portal and Scope.
    kit: PlayKit,
    /// The pad mode last agreed between the kit's native layer and
    /// `Shared::alternate`, so a change on either side (F2, or the menu's
    /// Pads row) can be told apart and followed by the other.
    kit_alt: bool,
    /// Orbit's gate pattern while its pads are being played as keys:
    /// otherwise every trip through the KEYS layer would erase it.
    gates: usize,
    /// A library step that arrived while a load was still in flight,
    /// with whether playback should start when it lands.
    pending_load: Option<bool>,
    /// Synth kinds: where the generated notes go (own voices, another
    /// app, or nowhere) -- see note_bus.rs. The sending end goes to the
    /// audio thread with the processor.
    note_route: crate::note_bus::NoteRoute,
    note_out: Option<crate::note_bus::NoteOut>,
}

/// One play-view control. `Param(j)` is one of the kind's six engine
/// parameters (`Kind::labels`); the rest are the shared rows below them in
/// the menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctl {
    Param(usize),
    Voice,
    Color,
    Attack,
    Tail,
    Scale,
    Octave,
    MotionRate,
    MotionDepth,
    /// Dream's root lives outside its six parameters.
    DreamRoot,
    Pattern,
    Rhythm,
    Freeze,
    Scene,
    /// The selected library file (track, station, memory).
    File,
}

/// Each kind's play-view controls, most important first: 0-7 are the knob
/// pairs (continuous where the kind has them), all of them the Controls
/// layer. Everything not listed stays in the menu.
fn controls(kind: Kind) -> &'static [Ctl] {
    use Ctl::*;
    match kind {
        // A polyrhythm sequencer: tempo and probability are what you ride.
        Kind::Orbit => &[
            Param(0), Param(4), Color, MotionDepth, Param(3), Param(5), Attack, MotionRate,
            Param(2), Param(1), Octave, Scale, Pattern, Rhythm, Voice, Scene,
        ],
        // Cohesion against spread is the swarm's whole character.
        Kind::Swarm => &[
            Param(1), Param(2), Color, Param(4), MotionDepth, Param(5), Attack, MotionRate,
            Param(3), Param(0), Rhythm, Octave, Scale, Pattern, Voice, Scene,
        ],
        // Brightness and parent blend reshape the partials live; Mutation
        // only matters at the next Breed, so it sits further down.
        Kind::Mutant => &[
            Param(4), Param(0), Color, MotionDepth, Param(5), Attack, MotionRate, Param(1),
            Param(2), Param(3), Rhythm, Octave, Scale, Pattern, Voice, Scene,
        ],
        Kind::Constellation => &[
            Param(4), Param(3), Color, MotionDepth, Attack, Tail, MotionRate, Param(2),
            Param(0), Param(1), Param(5), Scale, Pattern, Rhythm, Voice, Scene,
        ],
        Kind::Dream => &[
            Param(4), Param(3), Color, Param(5), Param(2), Param(0), MotionDepth, Tail,
            Param(1), DreamRoot, Rhythm, Octave, Scale, Pattern, Voice, Scene,
        ],
        Kind::Fracture => &[Param(5), Param(4), Param(0), Param(3), Param(1), Param(2), Freeze, Scene],
        Kind::Ghosts => &[Param(1), Param(5), Param(0), Param(3), Param(2), Param(4), Freeze, Scene],
        Kind::TapeMachine => &[Param(2), Param(0), Param(1), Param(5), Param(3), Param(4), Freeze, Scene],
        Kind::Master => &[Param(4), Param(5), Param(2), Param(3), Param(0), Param(1), Freeze, Scene],
        Kind::Reference => &[Param(5), Param(1), Param(0), Param(2), Param(4), Param(3), File, Scene],
        Kind::Vinyl => &[Param(1), Param(5), Param(0), Param(4), Param(3), Param(2), File, Scene],
        Kind::Practice => &[Param(0), Param(3), Param(1), Param(2), Param(4), Param(5), File, Scene],
        Kind::Radio => &[Param(5), Param(1), Param(0), Param(2), Param(4), Param(3), File, Scene],
        Kind::Memories => &[Param(5), Param(0), Param(1), Param(2), Param(4), Param(3), File, Scene],
        Kind::Field => &[Param(0), Param(5), Param(3), Param(1), Param(2), Param(4), Scene],
        Kind::SampleHunter => &[Param(0), Param(1), Param(3), Param(4), Param(2), Param(5), Scene],
        Kind::Studio => &[Param(1), Param(2), Param(0), Param(3), Param(4), Param(5), Scene],
        Kind::Portal | Kind::Scope => &[],
    }
}

fn kit_config(kind: Kind) -> KitConfig {
    let app_id = kind.id();
    if !kind.playable() {
        return KitConfig { app_id, ..Default::default() };
    }
    let list = controls(kind);
    let at = |c: Ctl| list.iter().position(|&x| x == c);
    let mut layers = match kind {
        Kind::Orbit => vec![Layer::Native(0, "GATES"), Layer::Native(1, "KEYS")],
        Kind::Dream => vec![Layer::Native(0, "STATES"), Layer::Native(1, "KEYS")],
        Kind::Swarm | Kind::Mutant | Kind::Constellation => vec![Layer::Native(0, "KEYS"), Layer::Native(1, "SCENES")],
        Kind::SampleHunter => vec![Layer::Native(0, "RECORD"), Layer::Native(1, "KEYS")],
        Kind::Studio => vec![Layer::Native(0, "RECORD"), Layer::Native(1, "MUTE/SOLO")],
        Kind::Field => vec![Layer::Native(0, "RECORD")],
        Kind::Fracture => vec![Layer::Native(0, "SLICES")],
        Kind::Ghosts => vec![Layer::Native(0, "ECHOES")],
        Kind::TapeMachine => vec![Layer::Native(0, "HEADS")],
        Kind::Master => vec![Layer::Native(0, "A/B")],
        Kind::Radio => vec![Layer::Native(0, "STATIONS")],
        Kind::Practice => vec![Layer::Native(0, "PRACTICE")],
        _ => vec![Layer::Native(0, "PLAYER")],
    };
    layers.extend([Layer::Controls, Layer::Moments]);
    if kind.effect() {
        layers.push(Layer::Throws);
    }
    // Four pairs on the 16-control instruments; the shorter lists end in
    // Freeze/Scene/File, which are steps, not something to turn.
    let pairs = if list.len() >= 16 { 4 } else { 3 };
    let hero = (0..pairs).map(|p| [2 * p, 2 * p + 1]).collect();
    // D-pad: a player steps through its library, Studio through its
    // tracks; everything else steps its four scenes, which are the
    // closest thing these apps have to presets.
    let browse = if kind.media() {
        at(Ctl::File)
    } else if kind == Kind::Studio {
        at(Ctl::Param(0))
    } else {
        at(Ctl::Scene)
    };
    let p = |j: usize| at(Ctl::Param(j));
    let routes = match kind {
        Kind::Orbit => Routes { stick_x: at(Ctl::Color), stick_y: at(Ctl::MotionDepth), hand_l: p(4), hand_r: p(3) },
        Kind::Swarm => Routes { stick_x: p(2), stick_y: at(Ctl::Color), hand_l: p(1), hand_r: at(Ctl::MotionDepth) },
        Kind::Mutant => Routes { stick_x: p(4), stick_y: p(0), hand_l: at(Ctl::Color), hand_r: at(Ctl::MotionDepth) },
        Kind::Constellation => Routes { stick_x: p(4), stick_y: at(Ctl::Color), hand_l: p(3), hand_r: at(Ctl::MotionDepth) },
        Kind::Dream => Routes { stick_x: p(4), stick_y: at(Ctl::Color), hand_l: p(5), hand_r: p(3) },
        Kind::Fracture => Routes { stick_x: p(0), stick_y: p(3), hand_l: p(5), hand_r: p(4) },
        Kind::Ghosts => Routes { stick_x: p(3), stick_y: p(1), hand_l: p(2), hand_r: p(5) },
        Kind::TapeMachine => Routes { stick_x: p(0), stick_y: p(2), hand_l: p(3), hand_r: p(5) },
        // A hand raising the ceiling would mean *less* limiting, the
        // opposite of what reaching in suggests, so the right hand is free.
        Kind::Master => Routes { stick_x: p(3), stick_y: p(2), hand_l: p(4), hand_r: None },
        // Pushing the platter: the stick is a pitch-bend on the speed.
        Kind::Vinyl => Routes { stick_x: p(1), stick_y: p(5), ..Default::default() },
        Kind::Practice => Routes { stick_x: p(0), stick_y: p(3), ..Default::default() },
        // Position is never routed: it would seek every frame.
        Kind::Reference | Kind::Radio | Kind::Memories => Routes { stick_y: p(5), ..Default::default() },
        Kind::Field => Routes { stick_x: p(3), stick_y: p(0), ..Default::default() },
        Kind::SampleHunter => Routes { stick_x: p(3), stick_y: p(0), ..Default::default() },
        Kind::Studio => Routes { stick_x: p(2), stick_y: p(1), ..Default::default() },
        Kind::Portal | Kind::Scope => Routes::default(),
    };
    let t = |c: Ctl, to: f32, label: &'static str| at(c).map(|control| Throw { control, to, label });
    let throws: Vec<Throw> = match kind {
        Kind::Fracture => vec![
            t(Ctl::Param(5), 1.0, "WET"),
            t(Ctl::Param(2), 1.0, "REVERSE"),
            t(Ctl::Freeze, 1.0, "FREEZE"),
            t(Ctl::Param(0), 0.0, "STUTTER"), // 25 ms slices
            t(Ctl::Param(3), 0.75, "+12"),
            t(Ctl::Param(3), 0.25, "-12"),
            t(Ctl::Param(4), 1.0, "SCATTER"),
            t(Ctl::Param(1), 1.0, "REPEAT"), // 16 repeats
        ],
        Kind::Ghosts => vec![
            t(Ctl::Param(1), 1.0, "HOLD"), // decay 0.95: the longest memory
            t(Ctl::Param(5), 1.0, "WET"),
            t(Ctl::Freeze, 1.0, "FREEZE"),
            t(Ctl::Param(4), 1.0, "REVERSE"),
            t(Ctl::Param(3), 0.03, "DARK"), // ~640 Hz low pass
            t(Ctl::Param(3), 1.0, "BRIGHT"),
            t(Ctl::Param(2), 1.0, "DRIFT"),
            t(Ctl::Param(0), 0.0, "SHORT"), // 0.1 s loop
        ],
        Kind::TapeMachine => vec![
            t(Ctl::Param(2), 1.0, "DUB"), // the bounded 0.92 feedback ceiling
            t(Ctl::Param(0), 0.25 / 1.75, "HALF"), // speed 0.5
            t(Ctl::Param(0), 1.0, "DOUBLE"),
            t(Ctl::Freeze, 1.0, "FREEZE"),
            t(Ctl::Param(3), 1.0, "WOW"),
            t(Ctl::Param(4), 1.0, "FLUTTER"),
            t(Ctl::Param(5), 1.0, "DRIVE"),
            t(Ctl::Param(1), 0.0, "SLAP"), // heads 25 ms apart
        ],
        Kind::Master => vec![
            t(Ctl::Param(0), 1.0, "REF B"),
            t(Ctl::Freeze, 1.0, "BYPASS"),
            t(Ctl::Param(4), 1.0, "DRIVE"),
            t(Ctl::Param(5), 1.0 / 3.0, "LIMIT"), // ceiling -12 dB
            t(Ctl::Param(2), 0.75, "BASS+6"),
            t(Ctl::Param(2), 0.25, "BASS-6"),
            t(Ctl::Param(3), 0.75, "PRES+6"),
            t(Ctl::Param(1), 1.0, "MATCH"), // only acts while B is selected
        ],
        _ => Vec::new(),
    }
    .into_iter()
    .flatten()
    .collect();
    KitConfig { app_id, layers, hero, browse, routes, throws, midi_to_pads: true, own_expression: false }
}

impl CollectionApp {
    pub fn new(
        kind: Kind,
        bus: Arc<AudioBus>,
        mods: Arc<ModBus>,
        mixer: Arc<MixerBus>,
        nav: Arc<AtomicF32>,
    ) -> Self {
        let out = bus.register(kind.name());
        let own=bus.index_of(kind.name()).unwrap();
        let (mix, ext) = mixer.register(kind.name(), &mods);
        let (tx, rx) = mpsc::sync_channel(16);
        let (take_tx, takes) = mpsc::sync_channel(2);
        let (work_tx, worker) = mpsc::channel();
        Self {
            kind,
            p: Arc::new(Shared {
                values: kind.defaults().map(AtomicF32::new),
                modulation: std::array::from_fn(|i| if kind.synth() || matches!(kind,Kind::Fracture|Kind::Ghosts|Kind::TapeMachine|Kind::Master) {mods.register(format!("{}: {}",kind.name(),kind.labels()[i]))} else {Arc::new(AtomicF32::new(0.))}),
                color_modulation: if kind.synth() {mods.register(format!("{}: Tone color",kind.name()))} else {Arc::new(AtomicF32::new(0.))},
                voice: AtomicUsize::new(kind.default_voice()),
                color: AtomicF32::new(0.5),
                attack: AtomicF32::new(0.008),
                release: AtomicF32::new(0.45),
                scale: AtomicUsize::new(1),
                dream_root: AtomicUsize::new(48),
                pattern: AtomicUsize::new(kind.default_pattern()),
                rhythm: AtomicUsize::new(kind.default_rhythm()),
                octave: AtomicF32::new(0.),
                motion_rate: AtomicF32::new(0.25),
                motion_depth: AtomicF32::new(0.),
                alternate: AtomicBool::new(false),
                freeze: AtomicBool::new(false),
                variation: AtomicUsize::new(0),
                source: AtomicUsize::new(NO_SOURCE),
                reference: AtomicUsize::new(NO_SOURCE),
                play: AtomicBool::new(false),
                record: AtomicBool::new(false),
                pads: AtomicUsize::new(if kind == Kind::Orbit { 0x1111 } else { 0 }),
                step: AtomicUsize::new(0),
                position: AtomicF32::new(0.),
                duration: AtomicF32::new(0.),
                ended: AtomicBool::new(false),
                audible: AtomicBool::new(false),
                loaded: AtomicBool::new(false),
                export_busy: AtomicBool::new(false),
                view: Mutex::new(View::default()),
                radio: Mutex::new(None),
                out,
                mix,
                ext,
            }),
            bus,
            terrain: Vec::new(),
            own,
            nav,
            list: ParamList::new(),
            held: [false; 16],
            tx,
            rx: Some(rx),
            takes,
            take_tx: Some(take_tx),
            worker,
            work_tx,
            files: vec![],
            file: 0,
            status: if kind.synth() {
                "Pads play • F3 starts the generator"
            } else if kind.media() {
                "R1 on Library to scan media/"
            } else {
                "Choose any installed audio source"
            }
            .into(),
            busy: false,
            tracks: [[0.8, 0., 0., 0.]; 8],
            last_track: 0,
            favorite: false,
            tag: 0,
            auto_start: false,
            loaded_file: None,
            kit: PlayKit::new(kit_config(kind), !cfg!(test) && kind.playable()),
            kit_alt: false,
            gates: 0,
            pending_load: None,
            note_route: crate::note_bus::NoteRoute::new(None, kind.name(), kind.id(), true).0,
            note_out: None,
        }
    }

    /// Lets a synth kind's generator play other apps (see note_bus.rs).
    pub fn with_notes(mut self, bus: Option<Arc<crate::note_bus::NoteBus>>) -> Self {
        if self.kind.synth() {
            let (route, out) = crate::note_bus::NoteRoute::new(bus, self.kind.name(), self.kind.id(), true);
            self.note_route = route;
            self.note_out = Some(out);
        }
        self
    }
    fn send(&mut self, c: Command) {
        if self.tx.try_send(c).is_err() {
            self.status = "Busy — try again".into();
        }
    }
    fn offset(&self) -> usize {
        if self.kind.synth() {
            // Plays, the settings of the instrument it plays, Instrument, Source
            3 + self.note_route.settings().len()
        } else if self.kind == Kind::Master {
            2
        } else {
            1
        }
    }
    fn rows(&self) -> Vec<(String, String, bool)> {
        let mut r = vec![];
        if self.kind.synth() {
            // First, like every note source: where the generator's notes go.
            r.push(("Plays".into(), self.note_route.label(), false));
            // The instrument it plays, dialled in right under Plays.
            r.extend(self.note_route.settings().into_iter().map(|s| (format!("  {}", s.label), s.value, false)));
            r.push((
                "Instrument".into(),
                VOICE_NAMES[self.p.voice.load(Ordering::Relaxed).min(7)].into(),
                false,
            ));
            r.push((
                "Source / live input".into(),
                self.bus.source_name(self.p.source.load(Ordering::Relaxed)),
                false,
            ));
        }
        if !self.kind.synth() {
            if self.kind.media() {
                r.push((
                    "Library / R1 load".into(),
                    self.files
                        .get(self.file)
                        .and_then(|p| p.file_stem())
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Scan media/".into()),
                    false,
                ));
            } else {
                r.push((
                    "Source".into(),
                    self.bus.source_name(self.p.source.load(Ordering::Relaxed)),
                    false,
                ));
            }
        }
        if self.kind == Kind::Master {
            r.push((
                "Reference source".into(),
                self.bus
                    .source_name(self.p.reference.load(Ordering::Relaxed)),
                false,
            ));
        }
        for (i, label) in self.kind.labels().iter().enumerate() {
            r.push(((*label).into(), self.value_text(i), false));
        }
        if self.kind.capture() {
            r.push(("Save WAV".into(), "R1 exports".into(), false));
            r.push(("Clear take".into(), "R1 clears".into(), false));
        }
        if self.kind == Kind::Mutant {
            r.push(("Action / R1".into(), "Breed patch".into(), false));
        }
        if self.kind.media() {
            r.push(("Action / R1".into(), "Rescan library".into(), false));
        }
        r.push((
            "Recall scene / R1".into(),
            self.scene_names()[self.p.variation.load(Ordering::Relaxed) % 4].into(),
            false,
        ));
        if self.kind.synth() {
            r.extend([
                (
                    "Tone color".into(),
                    format!("{:.0}%", self.p.color.get() * 100.),
                    false,
                ),
                (
                    "Attack".into(),
                    format!("{:.0} ms", self.p.attack.get() * 1000.),
                    false,
                ),
                (
                    "Tail length".into(),
                    format!("{:.2}×", self.p.release.get() / 0.45),
                    false,
                ),
                (
                    "Main scale".into(),
                    SCALE_TYPES[self.p.scale.load(Ordering::Relaxed) % SCALE_TYPES.len()].0.into(),
                    false,
                ),
                (
                    "Octave".into(),
                    format!("{:+.0}", self.p.octave.get()),
                    false,
                ),
                (
                    "Motion rate".into(),
                    format!("{:.2} Hz", self.p.motion_rate.get()),
                    false,
                ),
                (
                    "Motion depth".into(),
                    format!("{:.0}%", self.p.motion_depth.get() * 100.),
                    false,
                ),
                (
                    "Pads".into(),
                    if self.p.alternate.load(Ordering::Relaxed) {
                        if matches!(self.kind, Kind::Orbit | Kind::Dream) {
                            "Keys"
                        } else {
                            "Scenes"
                        }
                    } else {
                        if self.kind == Kind::Orbit {
                            "Orbit gates"
                        } else if self.kind == Kind::Dream {
                            "States"
                        } else {
                            "Keys"
                        }
                    }
                    .into(),
                    false,
                ),
                (
                    "Sample instrument / R1".into(),
                    self.files
                        .get(self.file)
                        .and_then(|p| p.file_stem())
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Scan media/".into()),
                    false,
                ),
                (
                    "Rescan instruments / R1".into(),
                    "Audio library".into(),
                    false,
                ),
            ]);
            r.push(("Root note".into(), ROOT_NAMES[self.root_note() as usize % 12].into(),false));
            r.push(("Pattern".into(),PATTERNS[self.p.pattern.load(Ordering::Relaxed)%6].into(),false));
            r.push(("Rhythm".into(),RHYTHMS[self.p.rhythm.load(Ordering::Relaxed)%5].into(),false));
        } else if matches!(self.kind, Kind::Fracture | Kind::Ghosts | Kind::TapeMachine) {
            r.push((
                "Freeze input buffer".into(),
                if self.p.freeze.load(Ordering::Relaxed) {
                    "Frozen"
                } else {
                    "Live"
                }
                .into(),
                false,
            ));
        }
        if matches!(self.kind, Kind::SampleHunter | Kind::Studio) {
            r.push((
                "Pads".into(),
                if self.p.alternate.load(Ordering::Relaxed) {
                    if self.kind == Kind::Studio {
                        "Mute / solo"
                    } else {
                        "Chromatic keys"
                    }
                } else {
                    "Record controls"
                }
                .into(),
                false,
            ));
        }
        if self.kind == Kind::Master {
            r.push((
                "Bypass processing".into(),
                if self.p.freeze.load(Ordering::Relaxed) {
                    "Bypass"
                } else {
                    "Process"
                }
                .into(),
                false,
            ));
        }
        if self.kind == Kind::Portal {
            r.push((
                "Second source".into(),
                self.bus
                    .source_name(self.p.reference.load(Ordering::Relaxed)),
                false,
            ));
            r.push((
                "Crossfade A → B".into(),
                format!("{:.0}%", self.p.color.get() * 100.),
                false,
            ));
        }
        r
    }
    /// One engine parameter as the menu shows it (shared with the play view).
    fn value_text(&self, i: usize) -> String {
        let v = self.p.values[i].get();
        if self.kind == Kind::Memories && i == 2 {
            ["Unsorted", "Nature", "Voice", "Music", "Idea"][(v as usize).min(4)].into()
        } else if self.kind == Kind::Scope && i == 3 {
            if v > 0. { "Hann" } else { "Rectangle" }.into()
        } else if self.kind == Kind::Scope && i == 4 {
            visuals::MODES[(v as usize).min(9)].into()
        } else if self.kind == Kind::Master && i == 0 {
            if v > 0. { "B / reference" } else { "A / mix" }.into()
        } else if self.kind.ranges()[i].2 == 1. && self.kind.ranges()[i].1 == 1. {
            if v > 0. { "on" } else { "off" }.into()
        } else if self.kind.ranges()[i].2 >= 1. {
            format!("{v:.0}")
        } else {
            format!("{v:.2}")
        }
    }
    /// Steps one engine parameter by `d` of its own step, exactly as the
    /// menu row does, side effects included.
    fn edit_value(&mut self, j: usize, d: i32) {
        let (lo, hi, step) = self.kind.ranges()[j];
        let v = (self.p.values[j].get() + d as f32 * step).clamp(lo, hi);
        self.p.values[j].set(v);
        self.value_changed(j);
    }
    /// What a parameter change has to do beyond storing the value: seek
    /// the player, keep Studio's per-track settings in step, re-sort
    /// Vinyl's library, write a memory's journal sidecar.
    fn value_changed(&mut self, j: usize) {
        let v = self.p.values[j].get();
        if self.kind.media() && j == 0 && self.kind != Kind::Practice && self.kind != Kind::Radio {
            self.send(Command::Seek(v / 100.));
        }
        if self.kind == Kind::Studio {
            let track = (self.p.values[0].get() as usize - 1).min(7);
            if track != self.last_track {
                self.last_track = track;
                for k in 0..4 {
                    self.p.values[k + 1].set(self.tracks[track][k]);
                }
            } else {
                for k in 0..4 {
                    self.tracks[track][k] = self.p.values[k + 1].get();
                }
                self.send(Command::Track(track, self.tracks[track]));
            }
        }
        if self.kind == Kind::Vinyl && j == 2 {
            if v > 0. {
                self.files.sort_by_key(|p| std::cmp::Reverse(p.metadata().and_then(|m| m.modified()).ok()));
            } else {
                self.files.sort();
            }
            self.file = 0;
        }
        if self.kind == Kind::Memories && (j == 1 || j == 2) {
            self.favorite = self.p.values[1].get() > 0.;
            self.tag = self.p.values[2].get() as usize;
            if let Some(file) = self.loaded_file.clone() {
                let text = format!("favorite = {}\ntag = {}\n", self.favorite, self.tag);
                let tx = self.work_tx.clone();
                std::thread::spawn(move || {
                    let result = std::fs::write(file.with_extension("journal.toml"), text);
                    let _ = tx.send(WorkerResult {
                        recycle: None,
                        journal: None,
                        clip: None,
                        files: None,
                        message: if result.is_ok() {
                            "Journal metadata saved".into()
                        } else {
                            "Unable to save journal metadata".into()
                        },
                    });
                });
            }
        }
    }
    /// The menu's Instrument row.
    fn edit_voice(&mut self, d: i32) {
        let voice = (self.p.voice.load(Ordering::Relaxed) as i32 + d.signum()).rem_euclid(8) as usize;
        self.p.voice.store(voice, Ordering::Relaxed);
        self.status = match voice {
            6 => "Choose Source for a live audio wavetable",
            7 => "Scroll to WAV instrument; R1 loads your sample",
            _ => "Pads play the selected instrument",
        }
        .into();
    }
    /// The instruments' performance rows, `j` counted from Recall scene.
    fn edit_perf(&mut self, j: usize, d: i32) {
        match j {
            1 => self.p.color.set((self.p.color.get() + d as f32 * 0.05).clamp(0., 1.)),
            2 => self.p.attack.set((self.p.attack.get() + d as f32 * 0.005).clamp(0.001, 1.)),
            3 => self.p.release.set((self.p.release.get() + d as f32 * 0.045).clamp(0.09, 1.8)),
            4 => self.p.scale.store(
                (self.p.scale.load(Ordering::Relaxed) as i32 + d.signum()).rem_euclid(SCALE_TYPES.len() as i32) as usize,
                Ordering::Relaxed,
            ),
            5 => self.p.octave.set((self.p.octave.get() + d as f32).clamp(-2., 2.)),
            6 => self.p.motion_rate.set((self.p.motion_rate.get() + d as f32 * 0.05).clamp(0.05, 8.)),
            7 => self.p.motion_depth.set((self.p.motion_depth.get() + d as f32 * 0.05).clamp(0., 1.)),
            8 => self.set_alternate(d > 0),
            9 => {
                if !self.files.is_empty() {
                    self.file = (self.file as i32 + d.signum()).rem_euclid(self.files.len() as i32) as usize;
                }
            }
            11 => {
                let old = self.root_note();
                self.set_root(old / 12 * 12 + (old + d.signum()).rem_euclid(12));
            }
            12 => self.p.pattern.store((self.p.pattern.load(Ordering::Relaxed) as i32 + d.signum()).rem_euclid(6) as usize, Ordering::Relaxed),
            13 => self.p.rhythm.store((self.p.rhythm.load(Ordering::Relaxed) as i32 + d.signum()).rem_euclid(5) as usize, Ordering::Relaxed),
            _ => {}
        }
    }
    /// Switches the pads' mode (menu Pads row, F2 native layers). Held
    /// key bits must not survive into gate mode or vice versa, so the
    /// mask is cleared -- except that Orbit's gate pattern is put aside
    /// and given back, since it's a composition, not a held chord.
    fn set_alternate(&mut self, alt: bool) {
        if self.p.alternate.swap(alt, Ordering::Relaxed) == alt {
            return;
        }
        if self.kind == Kind::Orbit {
            if alt {
                self.gates = self.p.pads.swap(0, Ordering::Relaxed);
            } else {
                self.p.pads.store(self.gates, Ordering::Relaxed);
            }
        } else if self.kind.synth() {
            self.p.pads.store(0, Ordering::Relaxed);
        }
    }
    /// D-pad on a player: select the neighbouring file and load it,
    /// carrying on playing if it was. A step during a load is queued
    /// rather than dropped, so the loaded file always ends up matching
    /// the one shown.
    fn browse_file(&mut self, d: i32) {
        if self.files.is_empty() {
            self.scan();
            return;
        }
        self.file = (self.file as i32 + d.signum()).rem_euclid(self.files.len() as i32) as usize;
        let start = self.auto_start || self.p.play.load(Ordering::Relaxed);
        if self.busy {
            // The in-flight load is superseded: don't let it start playing.
            self.auto_start = false;
            self.pending_load = Some(start);
        } else {
            self.auto_start = start;
            self.load();
        }
    }
    fn ctl(&self, i: usize) -> Ctl {
        controls(self.kind).get(i).copied().unwrap_or(Ctl::Scene)
    }
    fn ctl_knob(&self, c: Ctl) -> Knob<'_> {
        let p = &self.p;
        match c {
            Ctl::Param(j) => {
                let (lo, hi, _) = self.kind.ranges()[j];
                Knob::F(&p.values[j], lo, hi)
            }
            Ctl::Color => Knob::F(&p.color, 0., 1.),
            Ctl::Attack => Knob::F(&p.attack, 0.001, 1.),
            Ctl::Tail => Knob::F(&p.release, 0.09, 1.8),
            Ctl::MotionRate => Knob::F(&p.motion_rate, 0.05, 8.),
            Ctl::MotionDepth => Knob::F(&p.motion_depth, 0., 1.),
            Ctl::Freeze => Knob::B(&p.freeze),
            _ => Knob::None,
        }
    }
    /// Controls that pick *where* rather than *how it sounds*: a moment
    /// leaves them alone, exactly as scene recall already does (it keeps a
    /// player's position, Studio's track and a memory's journal tags).
    fn outside_a_moment(&self, c: Ctl) -> bool {
        match c {
            Ctl::File | Ctl::Scene => true,
            Ctl::Param(0) => (self.kind.media() && !matches!(self.kind, Kind::Practice | Kind::Radio)) || self.kind == Kind::Studio,
            Ctl::Param(j) => (self.kind == Kind::Memories && (j == 1 || j == 2)) || (self.kind == Kind::Vinyl && j == 2),
            _ => false,
        }
    }
    /// Whether the pads are pitched right now (the KEYS layers).
    fn keys_layer(&self) -> bool {
        let alt = self.p.alternate.load(Ordering::Relaxed);
        match self.kind {
            Kind::Orbit | Kind::Dream | Kind::SampleHunter => alt,
            Kind::Swarm | Kind::Mutant | Kind::Constellation => !alt,
            _ => false,
        }
    }
    /// The MIDI note a synth pad plays. The engine's own per-voice pitch
    /// once it has run (it includes modulation); before that, the same
    /// formula `Processor::synth` uses, without modulation.
    fn pad_note(&self, pad: usize) -> Option<i32> {
        if !self.kind.synth() || pad >= 16 {
            return None;
        }
        if let Ok(view) = self.p.view.try_lock() {
            if view.notes.len() == 16 {
                return Some(view.notes[pad].round() as i32);
            }
        }
        let v = |j: usize| {
            let (_, _, step) = self.kind.ranges()[j];
            let x = self.p.values[j].get();
            if step >= 1. { x.round() } else { x }
        };
        let (degree, octave) = match self.kind {
            Kind::Constellation => (
                (pad % 4) * 2 + v(2) as usize + if pad % 4 == 3 { (v(4) * 2.).round() as usize } else { 0 },
                (pad / 4) as f32 * v(1) * 12.,
            ),
            Kind::Dream => {
                let state = self.p.step.load(Ordering::Relaxed) % 4;
                (pad % ((v(3) * 12.) as usize + 1) + (state as f32 * v(4) * 3.).round() as usize, 0.)
            }
            _ => (pad, 0.),
        };
        let scale = self.p.scale.load(Ordering::Relaxed);
        Some(self.root_note() + music_scales::degree(scale, degree) + (octave + self.p.octave.get() * 12.).round() as i32)
    }
    /// The play view's hint line: what the knobs, D-pad, F2, F3 and R1 do here.
    fn play_hint(&self) -> String {
        let browse = self.kit.cfg.browse.map(|b| self.kit_label(b).to_lowercase()).unwrap_or_default();
        let f3 = if self.kind.synth() {
            "   F3: generator"
        } else if matches!(self.kind, Kind::Fracture | Kind::Ghosts | Kind::TapeMachine) {
            "   F3: process"
        } else if self.kind.capture() {
            "   F3: rec/play"
        } else if self.kind.media() {
            "   F3: play"
        } else {
            ""
        };
        format!(
            "L/R: {} / {}   U/D: {browse}   F2: pads{f3}   R1: menu",
            self.kit_label(0).to_lowercase(),
            self.kit_label(1).to_lowercase()
        )
    }
    fn root_note(&self) -> i32 {
        match self.kind {Kind::Orbit=>self.p.values[2].get() as i32,Kind::Swarm|Kind::Mutant=>self.p.values[3].get() as i32,Kind::Constellation=>self.p.values[0].get() as i32,_=>self.p.dream_root.load(Ordering::Relaxed) as i32}
    }
    fn set_root(&self,root:i32) {
        match self.kind {Kind::Orbit=>self.p.values[2].set(root as f32),Kind::Swarm|Kind::Mutant=>self.p.values[3].set(root as f32),Kind::Constellation=>self.p.values[0].set(root as f32),_=>self.p.dream_root.store(root as usize,Ordering::Relaxed)}
    }
    fn performance_base(&self) -> usize {
        self.offset()
            + 6
            + if self.kind.capture() {
                2
            } else if self.kind.media() || self.kind == Kind::Mutant {
                1
            } else {
                0
            }
    }
    fn scene_names(&self) -> [&'static str; 4] {
        match self.kind {
            Kind::Orbit => [
                "Solar plucks",
                "Glass satellites",
                "Bass moons",
                "Slow strings",
            ],
            Kind::Swarm => ["Reed colony", "Glass cloud", "Soft strings", "Low swarm"],
            Kind::Mutant => [
                "Organ genome",
                "Metal offspring",
                "Reed hybrid",
                "Bass organism",
            ],
            Kind::Constellation => ["Open organ", "Glass seventh", "Wide strings", "Minor reed"],
            Kind::Dream => ["Glass dawn", "Dark pulse", "String drift", "Reed release"],
            Kind::Fracture => [
                "Clean cuts",
                "Reverse shards",
                "Octave scatter",
                "Stutter cloud",
            ],
            Kind::Ghosts => [
                "Short memory",
                "Long shadows",
                "Reverse ghosts",
                "Dark echoes",
            ],
            Kind::TapeMachine => ["Warm heads", "Dub echoes", "Slow warble", "Bright flutter"],
            Kind::Portal => ["Clean send", "Telephone", "Inverted echo", "Dark send"],
            Kind::Field => ["Natural", "Quiet detail", "Loud room", "Low-cut voice"],
            Kind::SampleHunter => ["Whole take", "Middle slice", "Octave up", "Tail texture"],
            Kind::Studio => ["Center mix", "Wide left", "Wide right", "Quiet take"],
            Kind::Scope => ["Wave close", "Spectrum", "Slow cycle", "Small signal"],
            Kind::Practice => [
                "Normal speed",
                "Slow practice",
                "Half-speed",
                "Semitone study",
            ],
            Kind::Master => ["Neutral", "Warm", "Presence", "Gentle ceiling"],
            Kind::Vinyl => ["33 / original", "Slow spin", "Fast spin", "Endless side"],
            Kind::Memories => ["One memory", "Continuous", "Repeat memory", "Quiet listen"],
            Kind::Radio => [
                "Low latency",
                "Steady buffer",
                "Quiet station",
                "Repeat station",
            ],
            Kind::Reference => ["Original", "Peak matched", "Continuous", "Quiet reference"],
        }
    }
    fn recall_scene(&mut self, index: usize) {
        let index = index % 4;
        self.p.variation.store(index, Ordering::Relaxed);
        let mut values = self.kind.defaults();
        if index > 0 {
            match self.kind {
                Kind::Orbit => {
                    values[0] = [110., 145., 82., 58.][index];
                    values[1] = [3., 5., 2., 7.][index];
                    values[4] = [1., 0.75, 0.95, 0.6][index];
                }
                Kind::Swarm => {
                    values[0] = [8., 16., 6., 12.][index];
                    values[1] = [0.7, 0.25, 0.9, 0.5][index];
                    values[2] = [0.2, 0.8, 0.1, 0.4][index];
                }
                Kind::Mutant => {
                    values[0] = [0.5, 0.9, 0.3, 0.65][index];
                    values[2] = [4., 9., 6., 3.][index];
                    values[4] = [0.5, 0.85, 0.65, 0.3][index];
                }
                Kind::Constellation => {
                    values[1] = [1., 2., 3., 1.][index];
                    values[2] = index as f32;
                    values[4] = index as f32 * 0.25;
                }
                Kind::Dream => {
                    values[0] = [100., 135., 65., 88.][index];
                    values[1] = [4., 2., 8., 6.][index];
                    values[3] = [0.5, 0.9, 0.25, 0.65][index];
                }
                Kind::Fracture => {
                    values = [
                        [0.125, 3., 0., 0., 0., 0.8],
                        [0.2, 2., 1., 0., 0.3, 0.9],
                        [0.08, 4., 0., 12., 0.8, 1.],
                        [0.04, 12., 0., -12., 0.5, 0.85],
                    ][index];
                }
                Kind::Ghosts => {
                    values = [
                        [2., 0.65, 0.1, 4000., 0., 0.7],
                        [1.7, 0.9, 0.35, 6500., 0., 0.9],
                        [0.8, 0.75, 0.2, 3000., 1., 0.85],
                        [1.2, 0.82, 0.6, 650., 0., 0.95],
                    ][index];
                }
                Kind::TapeMachine => {
                    values = [
                        [1., 0.25, 0.5, 0.15, 0.1, 1.],
                        [0.8, 0.35, 0.8, 0.25, 0.1, 1.5],
                        [0.5, 0.5, 0.65, 0.8, 0.4, 2.],
                        [1.3, 0.12, 0.4, 0.2, 0.8, 1.2],
                    ][index];
                }
                Kind::Portal => {
                    if index == 1 {
                        values[2] = 450.;
                        values[3] = 2800.;
                    }
                    if index == 2 {
                        values[1] = 1.;
                        values[4] = 180.;
                    }
                    if index == 3 {
                        values[3] = 900.;
                        values[0] = 0.7;
                    }
                }
                Kind::Field => {
                    values[0] = [1., 2., 0.5, 1.3][index];
                    values[3] = [40., 20., 80., 160.][index];
                }
                Kind::SampleHunter => {
                    values[0] = [0., 25., 0., 65.][index];
                    values[1] = [100., 65., 100., 100.][index];
                    values[3] = [0., 0., 12., -12.][index];
                }
                Kind::Studio => {
                    values[0] = self.p.values[0].get();
                    values[2] = [0., -0.8, 0.8, 0.][index];
                    values[1] = [0.8, 0.8, 0.8, 0.4][index];
                }
                Kind::Scope => {
                    values[0] = [10., 10., 70., 3.][index];
                    values[1] = [1., 1., 1., 4.][index];
                    values[4] = if index == 1 { 1. } else { 0. };
                }
                Kind::Practice => {
                    values[0] = [1., 0.75, 0.5, 0.9][index];
                    values[3] = if index == 3 { 1. } else { 0. };
                    values[4] = [0., 80., 60., 90.][index];
                }
                Kind::Master => {
                    values[2] = [0., 3., -1., 0.][index];
                    values[3] = [0., -2., 3., 0.][index];
                    values[5] = [-1., -2., -2., -6.][index];
                }
                Kind::Vinyl => {
                    values[1] = [1., 0.75, 1.35, 1.][index];
                    values[4] = if index == 3 { 1. } else { 0. };
                }
                Kind::Radio => {
                    values[0] = [80., 350., 150., 150.][index];
                    values[5] = if index == 2 { 0.4 } else { 0.8 };
                    values[4] = if index == 3 { 1. } else { 0. };
                }
                Kind::Reference => {
                    values[2] = if index == 1 { 1. } else { 0. };
                    values[3] = if index == 2 { 1. } else { 0. };
                    values[5] = if index == 3 { 0.4 } else { 0.8 };
                }
                Kind::Memories => {
                    values[1] = self.p.values[1].get();
                    values[2] = self.p.values[2].get();
                    values[3] = if index == 1 { 1. } else { 0. };
                    values[4] = if index == 2 { 1. } else { 0. };
                    values[5] = if index == 3 { 0.35 } else { 0.8 };
                }
            }
        }
        if self.kind.media() && !matches!(self.kind, Kind::Practice | Kind::Radio) {
            values[0] = self.p.values[0].get();
        }
        for (i, v) in values.into_iter().enumerate() {
            self.p.values[i].set(v);
        }
        if self.kind.synth() {
            let voices = match self.kind {
                Kind::Orbit => [3, 1, 5, 3],
                Kind::Swarm => [2, 1, 3, 5],
                Kind::Mutant => [4, 1, 2, 5],
                Kind::Constellation => [4, 1, 3, 2],
                _ => [1, 5, 3, 2],
            };
            self.p.voice.store(voices[index], Ordering::Relaxed);
            self.p.color.set([0.5, 0.85, 0.3, 0.65][index]);
            self.p.motion_depth.set([0., 0.35, 0.15, 0.5][index]);
            self.p.release.set([0.45, 0.8, 1.5, 0.25][index]);
            if self.kind == Kind::Mutant {
                self.send(Command::Breed);
            }
        }
        if self.kind == Kind::Studio {
            let t = (values[0] as usize - 1).min(7);
            self.tracks[t] = [values[1], values[2], values[3], values[4]];
            self.send(Command::Track(t, self.tracks[t]));
        }
        self.status = format!("{} • scene recalled", self.scene_names()[index]);
    }
    fn scan(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Scanning media/…".into();
        let tx = self.work_tx.clone();
        let radio = self.kind == Kind::Radio;
        std::thread::spawn(move || {
            let media = media_root();
            let root = media.as_path();
            let mut files = vec![];
            scan_files(root, &mut files, 0, radio);
            files.sort();
            let message = if files.is_empty() {
                if radio {
                    "Add HTTP(S) WAV URLs as media/radio/*.url"
                } else {
                    "Add FLAC, WAV, MP3, Ogg, M4A or AIFF to media/"
                }
                .into()
            } else {
                format!("{} files • select then R1 to load", files.len())
            };
            let _ = tx.send(WorkerResult {
                recycle: None,
                journal: None,
                clip: None,
                files: Some(files),
                message,
            });
        });
    }
    fn load(&mut self) {
        if self.busy {
            return;
        }
        let Some(path) = self.files.get(self.file).cloned() else {
            self.scan();
            return;
        };
        if self.kind == Kind::Radio {
            let stream = radio::RadioStream::start(path);
            if let Some(old) = self.p.radio.lock().unwrap().replace(stream) {
                old.cancel.store(true, Ordering::Release);
            }
            self.busy = true;
            self.p.loaded.store(false, Ordering::Relaxed);
            self.p.play.store(false, Ordering::Relaxed);
            self.p.position.set(0.);
            self.p.duration.set(0.);
            self.status = "Connecting to WAV stream…".into();
            return;
        }
        self.busy = true;
        self.p.play.store(false, Ordering::Relaxed);
        self.p.loaded.store(false, Ordering::Relaxed);
        self.status = "Loading…".into();
        let tx = self.work_tx.clone();
        std::thread::spawn(move || {
            let result = decode::load(&path);
            let (clip, message) = match result {
                Ok(c) => {
                    let s = format!(
                        "{} • {:.1}s • ready",
                        c.name,
                        c.samples.len() as f32 / c.rate
                    );
                    (Some(c), s)
                }
                Err(e) => (None, e),
            };
            let _ = tx.send(WorkerResult {
                recycle: None,
                journal: Some({
                    let (favorite, tag) = load_journal(&path);
                    (path, favorite, tag)
                }),
                clip,
                files: None,
                message,
            });
        });
    }
    fn poll(&mut self) {
        if self.kind == Kind::Radio {
            let stream = self.p.radio.lock().unwrap().clone();
            if let Some(stream) = stream {
                if let Some(error) = stream.error.lock().unwrap().take() {
                    self.status = error;
                    self.busy = false;
                    self.auto_start = false;
                }
                if self.busy && stream.ready.load(Ordering::Acquire) {
                    self.busy = false;
                    self.p.loaded.store(true, Ordering::Relaxed);
                    self.status = "WAV stream ready • F3 Play / Stop".into();
                    if self.auto_start {
                        self.p.play.store(true, Ordering::Relaxed);
                        self.auto_start = false;
                    }
                }
            }
        }
        while let Ok(result) = self.worker.try_recv() {
            if let Some(buffer) = result.recycle {
                self.send(Command::Recycle(buffer));
                self.p.export_busy.store(false, Ordering::Relaxed);
            }
            if let Some((path, favorite, tag)) = result.journal {
                self.loaded_file = Some(path);
                if self.kind == Kind::Memories {
                    self.favorite = favorite;
                    self.tag = tag;
                    self.p.values[1].set(favorite as u8 as f32);
                    self.p.values[2].set(tag as f32);
                }
            }
            self.busy = false;
            self.status = result.message;
            if let Some(files) = result.files {
                self.files = files;
                self.file = self.file.min(self.files.len().saturating_sub(1));
            }
            if let Some(c) = result.clip {
                if self.kind.synth() {
                    self.p.voice.store(7, Ordering::Relaxed);
                    self.status = format!("{} • pads play at C4 root", c.name);
                }
                self.p.loaded.store(true, Ordering::Relaxed);
                self.send(Command::Load(c));
                if self.auto_start {
                    self.p.play.store(true, Ordering::Relaxed);
                    self.auto_start = false;
                }
            }
        }
        while let Ok(take) = self.takes.try_recv() {
            let tx = self.work_tx.clone();
            let name = self.kind.name().replace(' ', "-").to_lowercase();
            self.status = "Saving recording…".into();
            std::thread::spawn(move || {
                let mut take = take;
                let result = save_take(&name, &mut take);
                let message = match result {
                    Ok(p) => format!("Saved {}", p.display()),
                    Err(e) => format!("Save failed: {e}"),
                };
                let _ = tx.send(WorkerResult {
                    recycle: Some(take.samples),
                    journal: None,
                    clip: None,
                    files: None,
                    message,
                });
            });
        }
        if self.p.ended.swap(false, Ordering::Relaxed) {
            if self.kind == Kind::Radio && self.p.values[4].get() > 0. {
                self.load();
                self.auto_start = true;
            } else if self.kind.media()
                && self.kind != Kind::Practice
                && self.p.values[3].get() > 0.
                && !self.files.is_empty()
            {
                self.file = (self.file + 1) % self.files.len();
                self.load();
                self.auto_start = true;
            } else {
                if self.kind == Kind::Radio {
                    self.p.loaded.store(false, Ordering::Relaxed);
                }
                self.status = if self.kind.capture() {
                    "Take ready • F3 plays • R1 Save WAV"
                } else {
                    "Playback complete"
                }
                .into();
            }
        }
        if !self.busy {
            if let Some(start) = self.pending_load.take() {
                self.auto_start = start;
                self.load();
            }
        }
    }
    /// The synth kinds' "Plays" row (always first).
    fn plays_row(&self) -> Option<usize> {
        self.kind.synth().then_some(0)
    }
    fn action(&mut self) {
        let row = self.list.selected;
        if Some(row) == self.plays_row() {
            self.note_route.reset();
            return;
        }
        let base = self.performance_base();
        if row == base {
            self.recall_scene(self.p.variation.load(Ordering::Relaxed));
            return;
        }
        if self.kind.synth() && row == base + 9 {
            self.load();
            return;
        }
        if self.kind.synth() && row == base + 10 {
            self.scan();
            return;
        }
        let off = self.offset();
        if self.kind.media() && (row == 0 || row == off + 6) {
            if row == 0 {
                self.load()
            } else {
                self.scan()
            }
            return;
        }
        if row == off + 6 {
            if self.kind.capture() {
                self.send(Command::Save)
            } else if self.kind == Kind::Mutant {
                self.send(Command::Breed);
                self.status = "New offspring • pads audition the patch".into();
            }
        }
        if row == off + 7 && self.kind.capture() {
            self.p.record.store(false, Ordering::Relaxed);
            self.p.play.store(false, Ordering::Relaxed);
            self.send(Command::Clear);
        }
    }
}
impl App for CollectionApp {
    fn instrument_settings(&self) -> Vec<crate::app::Setting> {
        crate::app::play_kit::settings_of(self)
    }
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        crate::app::play_kit::adjust_in(self, index, delta)
    }
    fn on_enter(&mut self) {
        if self.kind.media() && self.files.is_empty() {
            self.scan();
        }
    }
    fn on_exit(&mut self) {
        if self.kind == Kind::Radio && !self.p.play.load(Ordering::Relaxed) {
            if let Some(stream) = self.p.radio.lock().unwrap().take() {
                stream.cancel.store(true, Ordering::Release);
            }
            self.p.loaded.store(false, Ordering::Relaxed);
            self.busy = false;
        }
    }
    fn tick(&mut self, input: &Input) {
        // The play view takes the knobs and D-pad first; in the menu they
        // pass straight through. Pads reach the app only on its own layers.
        let step_input;
        let input = if self.kind.playable() {
            let two = self.kind.two_pad_modes();
            let alt = self.p.alternate.load(Ordering::Relaxed);
            if two && alt != self.kit_alt {
                // The menu's Pads row changed the mode: the kit follows.
                self.kit.set_native(alt as u8);
                self.kit_alt = alt;
            }
            let mut play = std::mem::take(&mut self.kit);
            let step = play.tick(self, input);
            self.kit = play;
            if two {
                // F2 changed the layer: the app's pad mode follows.
                if let Some(id) = step.native {
                    self.set_alternate(id == 1);
                    self.kit_alt = id == 1;
                }
            }
            step_input = step.input;
            &step_input
        } else {
            input
        };
        self.poll();
        let rows = self.rows().len();
        self.list.navigate_input(input, rows, self.nav.get() as i32);
        let d = input.knob2;
        let off = self.offset();
        if d != 0 && Some(self.list.selected) == self.plays_row() {
            self.note_route.step(d);
        }
        if d != 0 {
            let i = self.list.selected;
            if i < off {
                let n = off.saturating_sub(3); // settings listed under Plays (synth kinds)
                if self.kind.synth() && i == 0 {
                    // Plays: stepped above
                } else if self.kind.synth() && i <= n {
                    self.note_route.adjust(i - 1, d.signum());
                } else if self.kind.synth() && i == n + 1 {
                    self.edit_voice(d);
                } else if self.kind.media() {
                    if !self.files.is_empty() {
                        self.file = (self.file as i32 + d.signum())
                            .rem_euclid(self.files.len() as i32)
                            as usize;
                        self.status = "R1 loads the selected file".into();
                    }
                } else {
                    let p = if i == 1 && !self.kind.synth() {
                        &self.p.reference
                    } else {
                        &self.p.source
                    };
                    let mut next =
                        cycle_source(p.load(Ordering::Relaxed), d.signum(), self.bus.len());
                    if next == self.own {
                        next = cycle_source(next, d.signum(), self.bus.len());
                    }
                    p.store(next, Ordering::Relaxed);
                    if self.kind.synth() && next != NO_SOURCE {
                        self.p.voice.store(6, Ordering::Relaxed);
                    }
                    self.status = if next == NO_SOURCE {
                        "Choose any installed audio source"
                    } else {
                        "Live input connected"
                    }
                    .into();
                }
            } else if i < off + 6 {
                self.edit_value(i - off, d);
            }
        }
        if d != 0 && self.list.selected >= self.performance_base() {
            let j = self.list.selected - self.performance_base();
            if j == 0 {
                let n = (self.p.variation.load(Ordering::Relaxed) as i32 + d.signum()).rem_euclid(4)
                    as usize;
                self.p.variation.store(n, Ordering::Relaxed);
                self.status = "R1 recalls the selected scene".into();
            } else if self.kind.synth() {
                self.edit_perf(j, d);
            } else if j == 1
                && matches!(self.kind, Kind::Fracture | Kind::Ghosts | Kind::TapeMachine)
            {
                self.p.freeze.store(d > 0, Ordering::Relaxed);
            }
        }
        if d != 0 {
            let j = self.list.selected as isize - self.performance_base() as isize;
            if j == 1 && matches!(self.kind, Kind::Studio | Kind::SampleHunter) {
                self.p.alternate.store(d > 0, Ordering::Relaxed);
            }
            if j == 1 && self.kind == Kind::Master {
                self.p.freeze.store(d > 0, Ordering::Relaxed);
            }
            if self.kind == Kind::Portal {
                if j == 1 {
                    let mut next = cycle_source(
                        self.p.reference.load(Ordering::Relaxed),
                        d.signum(),
                        self.bus.len(),
                    );
                    if next == self.own {
                        next = cycle_source(next, d.signum(), self.bus.len());
                    }
                    self.p.reference.store(next, Ordering::Relaxed);
                } else if j == 2 {
                    self.p
                        .color
                        .set((self.p.color.get() + d as f32 * 0.05).clamp(0., 1.));
                }
            }
        }
        if input.knob1_press || input.knob2_press {
            self.action();
        }
        let mut mask = self.p.pads.load(Ordering::Relaxed);
        for i in 0..16 {
            let edge = input.grid[i] && !self.held[i];
            if self.kind.synth()
                && self.p.alternate.load(Ordering::Relaxed)
                && !matches!(self.kind, Kind::Orbit | Kind::Dream)
            {
                if edge {
                    self.recall_scene(i % 4);
                }
            } else if self.kind == Kind::Orbit && !self.p.alternate.load(Ordering::Relaxed) {
                if edge {
                    mask ^= 1 << i;
                }
            } else if self.kind == Kind::Dream && !self.p.alternate.load(Ordering::Relaxed) {
                if edge {
                    self.p.step.store(i % 4, Ordering::Relaxed);
                }
            } else if self.kind == Kind::SampleHunter && self.p.alternate.load(Ordering::Relaxed) {
                if edge {
                    self.send(Command::Audition(i as i32));
                }
            } else if self.kind == Kind::Studio && self.p.alternate.load(Ordering::Relaxed) {
                if edge {
                    let track = i % 8;
                    let field = if i < 8 { 2 } else { 3 };
                    self.tracks[track][field] = 1. - self.tracks[track][field];
                    self.send(Command::Track(track, self.tracks[track]));
                    if track == self.last_track {
                        self.p.values[field + 1].set(self.tracks[track][field]);
                    }
                }
            } else if self.kind.capture() {
                if edge {
                    match i {
                        0 => {
                            if self.p.source.load(Ordering::Relaxed) == NO_SOURCE {
                                self.status = "Select a source before recording".into();
                                continue;
                            }
                            if self.p.export_busy.load(Ordering::Relaxed) {
                                self.status = "Finish saving before recording".into();
                                continue;
                            }
                            self.p
                                .record
                                .store(!self.p.record.load(Ordering::Relaxed), Ordering::Relaxed);
                            self.p.play.store(false, Ordering::Relaxed);
                        }
                        1 => {
                            self.p.record.store(false, Ordering::Relaxed);
                            self.p
                                .play
                                .store(!self.p.play.load(Ordering::Relaxed), Ordering::Relaxed);
                        }
                        2 => self.send(Command::Save),
                        3 => self.send(Command::Marker),
                        4..=11 if self.kind == Kind::Studio => {
                            self.p.values[0].set((i - 3) as f32);
                            self.last_track = i - 4;
                            for k in 0..4 {
                                self.p.values[k + 1].set(self.tracks[i - 4][k]);
                            }
                        }
                        4 if self.kind == Kind::Field => {
                            let peak = self.p.view.lock().unwrap().peak;
                            if peak > 0.001 {
                                self.p.values[0]
                                    .set((self.p.values[0].get() * 0.8 / peak).clamp(0.1, 4.));
                            }
                        }
                        5 if self.kind == Kind::Field => {
                            self.p.values[4].set(1. - self.p.values[4].get())
                        }
                        6 if self.kind == Kind::Field => {
                            self.p.values[2].set(1. - self.p.values[2].get())
                        }
                        7 if self.kind == Kind::Field => {
                            self.p.values[3].set(if self.p.values[3].get() > 0. {
                                0.
                            } else {
                                100.
                            })
                        }
                        _ => {}
                    }
                }
            } else if self.kind.media() {
                if edge {
                    match (self.kind, i) {
                        (Kind::Practice, 0) => {
                            let f = (self.p.position.get() / self.p.duration.get().max(0.001)
                                * 100.)
                                .clamp(0., 99.);
                            self.p.values[1].set(f);
                        }
                        (Kind::Practice, 1) => {
                            let f = (self.p.position.get() / self.p.duration.get().max(0.001)
                                * 100.)
                                .clamp(self.p.values[1].get() + 1., 100.);
                            self.p.values[2].set(f);
                        }
                        (Kind::Practice, 2) => {
                            self.p.values[0].set((self.p.values[0].get() - 0.05).max(0.5))
                        }
                        (Kind::Practice, 3) => {
                            self.p.values[0].set((self.p.values[0].get() + 0.05).min(1.5))
                        }
                        (_, 0..=11) => {
                            if i < self.files.len() {
                                self.file = i;
                                self.load();
                                self.auto_start = true;
                            }
                        }
                        (_, 12) => self.toggle_running(),
                        (_, 13) => {
                            self.p.values[4].set((self.p.values[4].get()+1.)%10.);
                        }
                        (_, 14) => self.send(Command::Seek(0.)),
                        (_, 15) => self.scan(),
                        _ => {}
                    }
                }
            } else if self.kind == Kind::Portal {
                if edge {
                    let sources = (0..self.bus.len())
                        .filter(|n| *n != self.own)
                        .collect::<Vec<_>>();
                    if let Some(source) = sources.get(i) {
                        self.p.source.store(*source, Ordering::Relaxed);
                        self.status = format!("Patched {}", self.bus.source_name(*source));
                    }
                }
            } else if self.kind == Kind::Scope {
                if edge {
                    if i == 0 {
                        self.p.values[5].set(if self.p.values[5].get() > 0. { 0. } else { 1. });
                    } else if i == 1 {
                        self.p.values[4].set((self.p.values[4].get()+1.)%10.);
                    } else if i == 2 {
                        self.p.values[1]
                            .set((0.8 / self.p.view.lock().unwrap().peak.max(0.1)).clamp(0.1, 8.));
                    }
                }
            } else if self.kind == Kind::Master {
                if edge {
                    match i {
                        0 | 1 => self.p.values[0].set(i as f32),
                        2 => self.p.values[1].set(1. - self.p.values[1].get()),
                        3 => {
                            self.p.freeze.fetch_xor(true, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
            } else {
                if input.grid[i] {
                    mask |= 1 << i;
                } else {
                    mask &= !(1 << i);
                }
            }
        }
        self.held = input.grid;
        self.p.pads.store(mask, Ordering::Relaxed);
    }
    fn background_tick(&mut self) {
        self.poll();
    }
    fn needs_background_audio(&self) -> bool {
        self.p.record.load(Ordering::Relaxed)
            || (self.kind != Kind::Scope
                && !self.kind.synth()
                && !self.kind.media()
                && self.p.source.load(Ordering::Relaxed) != NO_SOURCE
                && (self.kind.passive() || self.kind == Kind::Field))
            || self.busy
            || self.p.audible.load(Ordering::Relaxed)
            || self.p.export_busy.load(Ordering::Relaxed)
    }
    fn supports_pad_lock(&self) -> bool {
        self.kind.synth() || self.kind == Kind::Fracture
    }
    fn running(&self) -> Option<bool> {
        if self.kind.passive() {
            None
        } else {
            Some(self.p.play.load(Ordering::Relaxed) || self.p.record.load(Ordering::Relaxed))
        }
    }
    fn transport_action(&self) -> Option<&'static str> {
        if self.kind.passive() {
            None
        } else if self.kind == Kind::Radio && !self.p.loaded.load(Ordering::Relaxed) {
            Some("CONNECT")
        } else if self.p.record.load(Ordering::Relaxed) {
            Some("STOP REC")
        } else if self.p.play.load(Ordering::Relaxed) {
            Some("STOP")
        } else if self.kind.capture() && self.p.duration.get() == 0. {
            Some("RECORD")
        } else {
            Some("PLAY")
        }
    }
    fn toggle_running(&mut self) {
        if self.kind.passive() {
            return;
        }
        if self.kind == Kind::Radio && !self.p.loaded.load(Ordering::Relaxed) {
            self.load();
            self.auto_start = true;
            return;
        }
        if self.kind == Kind::Radio && self.p.play.load(Ordering::Relaxed) {
            self.p.play.store(false, Ordering::Relaxed);
            if let Some(stream) = self.p.radio.lock().unwrap().take() {
                stream.cancel.store(true, Ordering::Release);
            }
            self.p.loaded.store(false, Ordering::Relaxed);
            self.status = "Stopped • F3 reconnects".into();
            return;
        }
        if self.kind.media() && !self.p.loaded.load(Ordering::Relaxed) {
            self.status = "Select a WAV file and press R1 to load".into();
            return;
        }
        if self.kind.capture() && self.p.export_busy.load(Ordering::Relaxed) {
            self.status = "Saving take…".into();
            return;
        }
        if self.p.record.swap(false, Ordering::Relaxed) {
            self.status = "Take ready • F3 plays • pad 1 records".into();
        } else if self.kind.capture() && self.p.duration.get() == 0. {
            if self.p.source.load(Ordering::Relaxed) == NO_SOURCE {
                self.status = "Select a source before recording".into();
                return;
            }
            self.p.record.store(true, Ordering::Relaxed);
        } else {
            self.p.play.fetch_xor(true, Ordering::Relaxed);
        }
    }
    fn draw(&mut self, fb: &mut FrameBuffer) {
        if let Some(col) = self.play_column() {
            let pal = self.kind.palette();
            kit::draw::column(fb, &col, 16, 40, 350, 280, pal);
            Text::new(&self.play_hint(), Point::new(16, 337), MonoTextStyle::new(&SPLEEN_6X12, pal.dim)).draw(fb).ok();
            return;
        }
        let rows = self
            .rows()
            .into_iter()
            .map(|(a, b, _)| (a, b))
            .collect::<Vec<_>>();
        self.list.draw(fb, 16, 44, 24, rows.len(), &rows);
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_pointer_pick(&mut self, x: f32, y: f32) {
        if self.kind==Kind::Constellation && (-72. ..=-70.).contains(&x) {
            let slot=match x as i32 {-70=>0,-71=>2,_=>1};let (lo,hi,step)=self.kind.ranges()[slot];
            self.p.values[slot].set(((lo+y.clamp(0.,1.)*(hi-lo))/step).round()*step);
        } else if x == -1. && self.kind == Kind::Mutant {
            self.send(Command::Breed);
            self.status = "Bred a new harmonic genome".into();
        } else if x <= -50. && x > -66. && self.kind == Kind::SampleHunter {
            self.send(Command::Audition((-50. - x) as i32));
        } else if x <= -70. && x > -74. && self.kind == Kind::Dream {
            self.p.step.store((-70. - x) as usize, Ordering::Relaxed);
        } else if x <= -30. && x > -42. && self.kind == Kind::Constellation {
            self.p.values[0].set(48. + (-30. - x));
        } else if (x == -20. || x == -21.)
            && matches!(self.kind, Kind::SampleHunter | Kind::Practice)
        {
            let first = if self.kind == Kind::Practice { 1 } else { 0 };
            let slot = first + if x == -20. { 0 } else { 1 };
            let value = y.clamp(0., 1.) * 100.;
            let value = if slot == first {
                value.min(self.p.values[first + 1].get() - 1.)
            } else {
                value.max(self.p.values[first].get() + 1.)
            };
            self.p.values[slot].set(value.clamp(0., 100.));
        } else if x <= -10. && x > -18. {
            let i = (-10. - x) as usize;
            if self.kind == Kind::Portal {
                if let Some(source) = (0..self.bus.len()).filter(|s| *s != self.own).nth(i) {
                    self.p.source.store(source, Ordering::Relaxed);
                }
            } else if self.kind == Kind::Studio {
                self.last_track = i;
                self.p.values[0].set((i + 1) as f32);
                for k in 0..4 {
                    self.p.values[k + 1].set(self.tracks[i][k]);
                }
            } else if self.kind.media() && self.file + i < self.files.len() {
                self.file += i;
                self.auto_start = true;
                self.load();
            }
        }
    }
    fn slint_windowed_rows(
        &mut self,
        visible: usize,
    ) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.rows();
        let (start, end) = self.list.centered_scroll_window(visible.max(1), rows.len());
        (
            rows[start..end].to_vec(),
            self.list.selected.saturating_sub(start),
            start > 0,
            end < rows.len(),
        )
    }
    /// Orbit's gates, Dream's state, Studio's tracks and mutes: all now
    /// in `kit_pad_color`, so they show on the app's own layers as before.
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        if self.kind.playable() { self.kit.led_overlay(self) } else { [PadColor::Off; 16] }
    }
    fn play_surface(&self) -> bool {
        self.kind.playable()
    }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (self.kind.playable() && !self.kit.menu).then(|| self.kit.column(self))
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        self.kind.playable().then(|| self.kit.layer_label())
    }
    /// The pad-mode sync with `Shared::alternate` happens in `tick`, from
    /// the layer the kit reports.
    fn toggle_grid_mode(&mut self) {
        if self.kind.playable() {
            self.kit.next_layer();
        }
    }
    fn slint_scale_info(&self)->Option<ScaleInfo> {
        if self.kind.synth() && [self.performance_base()+4,self.performance_base()+11].contains(&self.list.selected) {
            Some(ScaleInfo::new(self.p.scale.load(Ordering::Relaxed),self.root_note()))
        } else {None}
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let v = self.p.view.lock().unwrap().clone();
        if self.kind==Kind::Scope && !v.spectrum.is_empty() && self.p.values[5].get()==0. {
            if self.terrain.len()>=12*32 {self.terrain.drain(..32);}
            self.terrain.extend((0..32).map(|i|v.spectrum[i*v.spectrum.len()/32]));
        }
        let mode=match self.kind {Kind::Scope=>self.p.values[4].get() as usize,Kind::Ghosts=>2,Kind::Fracture=>3,Kind::Constellation=>4,_=>0};
        let visual_lines=visuals::geometry(mode,&v.wave,&v.spectrum,&self.terrain,if self.kind==Kind::Ghosts {&v.tracks}else if self.kind==Kind::Fracture{&v.buffer}else{&[]},if self.kind==Kind::Constellation{&v.notes}else{&[]},&v.levels,self.kind==Kind::Fracture&&self.p.values[2].get()>0.);
        SlintExtra::Collection(CollectionExtra {
            visual_lines,
            terrain:self.terrain.clone(),
            controls: (0..6)
                .map(|i| {
                    let (lo, hi, _) = self.kind.ranges()[i];
                    (self.p.values[i].get() - lo) / (hi - lo).max(0.001)
                })
                .collect(),
            buffer: v.buffer,
            tracks: v.tracks,
            voice_phases: v.phases,
            notes: v.notes,
            genes: v.genes,
            files: self
                .files
                .iter()
                .skip(self.file)
                .take(4)
                .map(|p| {
                    p.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect(),
            source_names: (0..self.bus.len())
                .filter(|i| *i != self.own)
                .take(8)
                .map(|i| self.bus.source_name(i))
                .collect(),
            reference: self
                .bus
                .source_name(self.p.reference.load(Ordering::Relaxed)),
            instrument: if self.kind.synth() {
                VOICE_NAMES[self.p.voice.load(Ordering::Relaxed).min(7)].into()
            } else {
                self.scene_names()[self.p.variation.load(Ordering::Relaxed) % 4].into()
            },
            frozen: self.p.freeze.load(Ordering::Relaxed),
            alternate: self.p.alternate.load(Ordering::Relaxed),
            kind: self.kind as i32 + 1,
            status: self.status.clone(),
            wave: v.wave,
            spectrum: v.spectrum,
            levels: v.levels,
            peak: v.peak,
            rms: v.rms,
            phase: self.p.position.get(),
            duration: self.p.duration.get(),
            step: self.p.step.load(Ordering::Relaxed) as i32,
            pads: (0..16)
                .map(|i| {
                    if self.kind == Kind::Studio && self.p.alternate.load(Ordering::Relaxed) {
                        self.tracks[i % 8][if i < 8 { 2 } else { 3 }] > 0.
                    } else if self.kind.capture() {
                        self.held[i]
                    } else {
                        self.p.pads.load(Ordering::Relaxed) & (1 << i) != 0
                    }
                })
                .collect(),
            recording: self.p.record.load(Ordering::Relaxed),
            playing: self.p.play.load(Ordering::Relaxed),
            source: self.bus.source_name(self.p.source.load(Ordering::Relaxed)),
            hint: if self.kind.playable() && !self.kit.menu { self.play_hint() } else { (match self.kind {
                Kind::Orbit => "Pads: gates / keys • Instrument selects sound • scenes below",
                Kind::Swarm => "Play keys or switch Pads to Scenes • Motion moves the tone",
                Kind::Mutant => "Breed changes real partials • load WAV or live instruments",
                Kind::Constellation => "Keys voice chords • Scale / octave / instrument below",
                Kind::Dream => "Pads choose states / keys • scenes change the musical arc",
                Kind::Fracture => "Hold pads to punch slices • Freeze holds the input buffer",
                Kind::Ghosts => "Hold pads 1–4 to solo generations • Freeze holds memory",
                Kind::TapeMachine => "Hold pads 1–4 to solo heads • Freeze holds the tape",
                Kind::Portal => "Pads patch sources • second input + crossfade below",
                Kind::Field => "1 Rec · 2 Play · 3 Save · 4 Mark · 5 Auto gain · 6 Monitor",
                Kind::SampleHunter => "Record, trim, then switch Pads to Chromatic keys",
                Kind::Studio => "5–12 select tracks • switch Pads to Mute / solo",
                Kind::Scope => "Pad 1 Freeze · 2 Next visual · 3 Auto gain",
                Kind::Master => "Pads 1 A · 2 B · 3 Match · 4 Bypass",
                Kind::Practice => "Pads 1 set A · 2 set B · 3 slower · 4 faster · 13 play",
                Kind::Radio => "Pads 1–12 select stations · 13 connect/stop · 14 repeat",
                _ => "Pads 1–12 load tracks · 13 play · 14 repeat · 15 rewind",
            })
            .into() },
        })
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let mut processor = Processor::new(self.kind, self.p.clone(), self.bus.clone(), self.rx.take()?, self.take_tx.take()?);
        if let Some(out) = self.note_out.take() {
            processor.notes = out;
        }
        Some(Box::new(processor))
    }
}
impl PlayHost for CollectionApp {
    fn kit_control_count(&self) -> usize {
        controls(self.kind).len()
    }
    fn kit_label(&self, i: usize) -> String {
        let label = match self.ctl(i) {
            Ctl::Param(j) => self.kind.labels()[j],
            Ctl::Voice => "Instrument",
            Ctl::Color => "Tone color",
            Ctl::Attack => "Attack",
            Ctl::Tail => "Tail length",
            Ctl::Scale => "Scale",
            Ctl::Octave => "Octave",
            Ctl::MotionRate => "Motion rate",
            Ctl::MotionDepth => "Motion depth",
            Ctl::DreamRoot => "Root note",
            Ctl::Pattern => "Pattern",
            Ctl::Rhythm => "Rhythm",
            Ctl::Freeze if self.kind == Kind::Master => "Bypass",
            Ctl::Freeze => "Freeze",
            Ctl::Scene => "Scene",
            Ctl::File if self.kind == Kind::Radio => "Station",
            Ctl::File if self.kind == Kind::Memories => "Memory",
            Ctl::File => "Track",
        };
        label.to_string()
    }
    fn kit_value(&self, i: usize) -> String {
        let p = &self.p;
        match self.ctl(i) {
            Ctl::Param(j) => self.value_text(j),
            Ctl::Voice => VOICE_NAMES[p.voice.load(Ordering::Relaxed).min(7)].into(),
            Ctl::Color => format!("{:.0}%", p.color.get() * 100.),
            Ctl::Attack => format!("{:.0} ms", p.attack.get() * 1000.),
            Ctl::Tail => format!("{:.2}×", p.release.get() / 0.45),
            Ctl::Scale => SCALE_TYPES[p.scale.load(Ordering::Relaxed) % SCALE_TYPES.len()].0.into(),
            Ctl::Octave => format!("{:+.0}", p.octave.get()),
            Ctl::MotionRate => format!("{:.2} Hz", p.motion_rate.get()),
            Ctl::MotionDepth => format!("{:.0}%", p.motion_depth.get() * 100.),
            Ctl::DreamRoot => note_name(self.root_note()),
            Ctl::Pattern => PATTERNS[p.pattern.load(Ordering::Relaxed) % 6].into(),
            Ctl::Rhythm => RHYTHMS[p.rhythm.load(Ordering::Relaxed) % 5].into(),
            Ctl::Freeze => {
                let on = p.freeze.load(Ordering::Relaxed);
                match (self.kind == Kind::Master, on) {
                    (true, true) => "Bypass",
                    (true, false) => "Process",
                    (false, true) => "Frozen",
                    (false, false) => "Live",
                }
                .into()
            }
            Ctl::Scene => self.scene_names()[p.variation.load(Ordering::Relaxed) % 4].into(),
            Ctl::File => self
                .files
                .get(self.file)
                .and_then(|f| f.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Scan media/".into()),
        }
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        let p = &self.p;
        let frac = |v: usize, n: usize| Some(v.min(n.saturating_sub(1)) as f32 / n.saturating_sub(1).max(1) as f32);
        match self.ctl(i) {
            Ctl::Voice => frac(p.voice.load(Ordering::Relaxed), 8),
            Ctl::Scale => frac(p.scale.load(Ordering::Relaxed) % SCALE_TYPES.len(), SCALE_TYPES.len()),
            Ctl::Octave => Some(((p.octave.get() + 2.) / 4.).clamp(0., 1.)),
            Ctl::DreamRoot => Some(((self.root_note() as f32 - 24.) / 60.).clamp(0., 1.)),
            Ctl::Pattern => frac(p.pattern.load(Ordering::Relaxed) % 6, 6),
            Ctl::Rhythm => frac(p.rhythm.load(Ordering::Relaxed) % 5, 5),
            Ctl::Scene => frac(p.variation.load(Ordering::Relaxed) % 4, 4),
            Ctl::File => frac(self.file, self.files.len()),
            c => self.ctl_knob(c).norm(),
        }
    }
    /// Small sets of whole numbers (repeats, inversion, a mode, an on/off)
    /// are choices: shown on dials and thrown, never pushed by the stick.
    /// Wide integer ranges (tempo, a cutoff in 100 Hz steps, semitones)
    /// are left continuous; the engine rounds them anyway.
    fn kit_stepped(&self, i: usize) -> bool {
        match self.ctl(i) {
            Ctl::Param(j) => {
                let (lo, hi, step) = self.kind.ranges()[j];
                step >= 1. && (hi - lo) / step <= 16.
            }
            Ctl::Color | Ctl::Attack | Ctl::Tail | Ctl::MotionRate | Ctl::MotionDepth => false,
            _ => true,
        }
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match self.ctl(i) {
            Ctl::Param(j) => self.edit_value(j, delta),
            Ctl::Voice => self.edit_voice(delta),
            Ctl::Color => self.edit_perf(1, delta),
            Ctl::Attack => self.edit_perf(2, delta),
            Ctl::Tail => self.edit_perf(3, delta),
            Ctl::Scale => self.edit_perf(4, delta),
            Ctl::Octave => self.edit_perf(5, delta),
            Ctl::MotionRate => self.edit_perf(6, delta),
            Ctl::MotionDepth => self.edit_perf(7, delta),
            Ctl::DreamRoot => self.edit_perf(11, delta),
            Ctl::Pattern => self.edit_perf(12, delta),
            Ctl::Rhythm => self.edit_perf(13, delta),
            Ctl::Freeze => self.p.freeze.store(delta > 0, Ordering::Relaxed),
            // On the play view a scene step recalls it at once: the menu's
            // select-then-press is two hands for one move.
            Ctl::Scene => {
                let n = (self.p.variation.load(Ordering::Relaxed) as i32 + delta.signum()).rem_euclid(4) as usize;
                self.recall_scene(n);
            }
            Ctl::File => self.browse_file(delta),
        }
    }
    fn kit_reset(&mut self, i: usize) {
        let p = self.p.clone();
        match self.ctl(i) {
            Ctl::Param(j) => {
                p.values[j].set(self.kind.defaults()[j]);
                self.value_changed(j);
            }
            Ctl::Voice => p.voice.store(self.kind.default_voice(), Ordering::Relaxed),
            Ctl::Color => p.color.set(0.5),
            Ctl::Attack => p.attack.set(0.008),
            Ctl::Tail => p.release.set(0.45),
            Ctl::Scale => p.scale.store(1, Ordering::Relaxed),
            Ctl::Octave => p.octave.set(0.),
            Ctl::MotionRate => p.motion_rate.set(0.25),
            Ctl::MotionDepth => p.motion_depth.set(0.),
            Ctl::DreamRoot => p.dream_root.store(48, Ordering::Relaxed),
            Ctl::Pattern => p.pattern.store(self.kind.default_pattern(), Ordering::Relaxed),
            Ctl::Rhythm => p.rhythm.store(self.kind.default_rhythm(), Ordering::Relaxed),
            Ctl::Freeze => p.freeze.store(false, Ordering::Relaxed),
            // Re-recall the current scene: the menu's press on that row.
            Ctl::Scene => self.recall_scene(p.variation.load(Ordering::Relaxed)),
            Ctl::File => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0., 1.);
        let p = self.p.clone();
        let pick = |n: usize| (v * n.saturating_sub(1) as f32).round() as usize;
        match self.ctl(i) {
            Ctl::Param(j) => {
                let (lo, hi, step) = self.kind.ranges()[j];
                let mut x = lo + v * (hi - lo);
                if step >= 1. {
                    x = ((x - lo) / step).round() * step + lo;
                }
                let x = x.clamp(lo, hi);
                // Only real changes: each one may seek, message the engine
                // or write a journal file.
                if p.values[j].get() != x {
                    p.values[j].set(x);
                    self.value_changed(j);
                }
            }
            Ctl::Voice => p.voice.store(pick(8), Ordering::Relaxed),
            Ctl::Scale => p.scale.store(pick(SCALE_TYPES.len()), Ordering::Relaxed),
            // Whole octaves only: the engine adds octave * 12 semitones, so
            // a fraction would detune rather than transpose.
            Ctl::Octave => p.octave.set((-2. + v * 4.).round()),
            Ctl::DreamRoot => self.set_root(24 + (v * 60.).round() as i32),
            Ctl::Pattern => p.pattern.store(pick(6), Ordering::Relaxed),
            Ctl::Rhythm => p.rhythm.store(pick(5), Ordering::Relaxed),
            Ctl::Scene => {
                if pick(4) != p.variation.load(Ordering::Relaxed) % 4 {
                    self.recall_scene(pick(4));
                }
            }
            // Selects without loading (loading is the D-pad's job).
            Ctl::File => {
                if !self.files.is_empty() {
                    self.file = pick(self.files.len());
                }
            }
            c => self.ctl_knob(c).set(v),
        }
    }
    fn kit_snapshot(&self) -> serde_json::Value {
        serde_json::Value::Array(
            (0..self.kit_control_count())
                .map(|i| match self.kit_norm(i) {
                    Some(v) if !self.outside_a_moment(self.ctl(i)) => serde_json::json!(v),
                    _ => serde_json::Value::Null,
                })
                .collect(),
        )
    }
    fn kit_line(&self) -> String {
        self.status.clone()
    }
    /// A key plays the pad with its pitch on the KEYS layers (folding by
    /// octave when no pad has the exact note); elsewhere the shell's old
    /// note % 16, so a keyboard still taps the transport/slice pads it
    /// always did.
    fn kit_midi_pad(&self, note: u8) -> Option<usize> {
        let note = note as i32;
        if !self.keys_layer() {
            return (note >= 21).then_some(note as usize % 16);
        }
        if self.kind == Kind::SampleHunter {
            // Pad n auditions the take n semitones up; C4 is the take as
            // recorded (plus its own Pitch setting).
            let d = note - 60;
            let pad = if (0..16).contains(&d) { d } else { d.rem_euclid(12) };
            return Some(pad as usize);
        }
        let notes: Vec<Option<i32>> = (0..16).map(|i| self.pad_note(i)).collect();
        notes
            .iter()
            .position(|&n| n == Some(note))
            .or_else(|| notes.iter().position(|&n| n.is_some_and(|n| n.rem_euclid(12) == note.rem_euclid(12))))
    }
    fn kit_pad_label(&self, layer: u8, pad: usize) -> String {
        let alt = layer == 1;
        let pick = |names: &[&str]| names.get(pad).copied().unwrap_or("").to_string();
        if self.keys_layer() {
            return if self.kind == Kind::SampleHunter {
                format!("+{pad}")
            } else {
                self.pad_note(pad).map(note_name).unwrap_or_default()
            };
        }
        match self.kind {
            Kind::Orbit => {
                // Row r fires every 1 + r * ratio steps (Processor::synth).
                let every = 1 + (pad / 4) * self.p.values[1].get().round() as usize;
                let on = self.p.pads.load(Ordering::Relaxed) & (1 << pad) != 0;
                if on { format!("● /{every}") } else { format!("/{every}") }
            }
            Kind::Dream => ["Calm", "Tension", "Chaos", "Release"][pad % 4].to_string(),
            Kind::Swarm | Kind::Mutant | Kind::Constellation => {
                self.scene_names()[pad % 4].split(' ').next().unwrap_or("").to_string()
            }
            Kind::Fracture => format!("S{}", pad + 1),
            Kind::Ghosts if pad < 4 => format!("Gen {}", pad + 1),
            Kind::TapeMachine if pad < 4 => format!("Head {}", pad + 1),
            Kind::Master => pick(&["A", "B", "MATCH", "BYPASS"]),
            Kind::Field => pick(&["REC", "PLAY", "SAVE", "MARK", "AUTO", "MON", "LIMIT", "LOCUT"]),
            Kind::SampleHunter => pick(&["REC", "PLAY", "SAVE", "MARK"]),
            Kind::Studio if alt => format!("{}{}", if pad < 8 { "M" } else { "S" }, pad % 8 + 1),
            Kind::Studio => match pad {
                0..=3 => pick(&["REC", "PLAY", "SAVE", "MARK"]),
                4..=11 => format!("T{}", pad - 3),
                _ => String::new(),
            },
            k if k.media() => match (k, pad) {
                (Kind::Practice, 0..=3) => pick(&["SET A", "SET B", "SLOWER", "FASTER"]),
                (_, 0..=11) => self
                    .files
                    .get(pad)
                    .and_then(|f| f.file_stem())
                    .map(|s| s.to_string_lossy().chars().take(8).collect())
                    .unwrap_or_default(),
                (Kind::Practice, 13) => "CLICK".into(),
                (_, 12) => "PLAY".into(),
                (_, 13) => "REPEAT".into(),
                (_, 14) => "REWIND".into(),
                _ => "SCAN".into(),
            },
            _ => String::new(),
        }
    }
    fn kit_pad_color(&self, layer: u8, pad: usize, held: bool) -> PadColor {
        let alt = layer == 1;
        let bit = |m: usize| m & (1 << pad) != 0;
        let p = &self.p;
        if self.kind == Kind::Orbit && !alt {
            return if bit(p.pads.load(Ordering::Relaxed)) { PadColor::Green } else { PadColor::Off };
        }
        if self.kind == Kind::Studio && alt {
            return if self.tracks[pad % 8][if pad < 8 { 2 } else { 3 }] > 0. {
                if pad < 8 { PadColor::Red } else { PadColor::Green }
            } else {
                PadColor::Off
            };
        }
        if held {
            return PadColor::Green;
        }
        let playing = p.play.load(Ordering::Relaxed);
        match self.kind {
            Kind::Studio if (4..12).contains(&pad) && pad - 4 == self.last_track => PadColor::Blue,
            Kind::Dream if !alt && pad == p.step.load(Ordering::Relaxed) % 4 => PadColor::Blue,
            Kind::Swarm | Kind::Mutant | Kind::Constellation if alt && pad % 4 == p.variation.load(Ordering::Relaxed) % 4 => PadColor::Blue,
            _ if self.keys_layer() && self.kind != Kind::SampleHunter => {
                if self.pad_note(pad).is_some_and(|n| n.rem_euclid(12) == self.root_note().rem_euclid(12)) { PadColor::Blue } else { PadColor::Off }
            }
            k if k.capture() && !self.keys_layer() && pad == 0 && p.record.load(Ordering::Relaxed) => PadColor::Red,
            k if k.capture() && !self.keys_layer() && pad == 1 && playing => PadColor::Yellow,
            k if k.media() && pad == 12 && playing => PadColor::Yellow,
            Kind::Master if pad < 2 && (p.values[0].get() > 0.) == (pad == 1) => PadColor::Blue,
            Kind::Master if pad == 2 && p.values[1].get() > 0. => PadColor::Yellow,
            Kind::Master if pad == 3 && p.freeze.load(Ordering::Relaxed) => PadColor::Red,
            _ => PadColor::Off,
        }
    }
}

fn load_journal(path: &Path) -> (bool, usize) {
    let Ok(text) = std::fs::read_to_string(path.with_extension("journal.toml")) else {
        return (false, 0);
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return (false, 0);
    };
    (
        value
            .get("favorite")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        value
            .get("tag")
            .and_then(|v| v.as_integer())
            .unwrap_or(0)
            .clamp(0, 4) as usize,
    )
}
fn media_root() -> PathBuf {
    std::env::var_os("PORTAMAX_MEDIA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/media")))
}
fn scan_files(root: &Path, out: &mut Vec<PathBuf>, depth: usize, radio: bool) {
    if depth > 4 || out.len() >= 1024 {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                scan_files(&p, out, depth + 1, radio)
            } else if if radio {p.extension().and_then(|s|s.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("url"))} else {decode::supported(&p)}
            {
                out.push(p);
            }
            if out.len() >= 1024 {
                break;
            }
        }
    }
}
#[allow(dead_code)] // not used by the main binary
fn load_wav(path: &Path) -> Result<Clip, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("WAV: {e}"))?;
    let s = r.spec();
    if s.channels == 0 || s.channels > 2 || s.sample_rate == 0 {
        return Err("Use mono or stereo PCM/float WAV".into());
    }
    if r.duration() as usize > MEDIA_LIMIT {
        return Err("Clip too long: the limit is 30 minutes".into());
    }
    let raw: Result<Vec<f32>, _> = if s.sample_format == hound::SampleFormat::Float {
        r.samples::<f32>().collect()
    } else {
        let div = 2f32.powi(s.bits_per_sample as i32 - 1);
        r.samples::<i32>()
            .map(|v| v.map(|x| x as f32 / div))
            .collect()
    };
    let raw = raw.map_err(|e| e.to_string())?;
    let samples = raw
        .chunks(s.channels as usize)
        .map(|f| {
            [f[0], *f.get(1).unwrap_or(&f[0])].map(|v| {
                if v.is_finite() {
                    v.clamp(-1., 1.)
                } else {
                    0.
                }
            })
        })
        .collect::<Vec<_>>();
    let peak = samples.iter().flatten().fold(0f32, |a, v| a.max(v.abs()));
    Ok(Clip {
        samples: Arc::new(Pcm::from(samples)),
        rate: s.sample_rate as f32,
        name: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        peak,
    })
}
fn save_take(name: &str, take: &mut Take) -> Result<PathBuf, String> {
    if take.samples.is_empty() {
        return Err("No recorded audio".into());
    }
    let recordings = media_root().join("recordings");
    let dir = recordings.as_path();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = dir.join(format!("{name}-{stamp}.wav"));
    let mut w = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 2,
            sample_rate: take.rate,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|e| e.to_string())?;
    let gain = if take.normalize {
        0.98 / take
            .samples
            .iter()
            .flatten()
            .fold(0.001f32, |a, v| a.max(v.abs()))
    } else {
        1.
    };
    for frame in &take.samples {
        for s in frame {
            let s = *s * gain;
            w.write_sample((s.clamp(-1., 1.) * 8388607.) as i32)
                .map_err(|e| e.to_string())?
        }
    }
    w.finalize().map_err(|e| e.to_string())?;
    if take.marker > 0 {
        std::fs::write(
            path.with_extension("markers.txt"),
            format!("{:.6}\n", take.marker as f32 / take.rate as f32),
        )
        .map_err(|e| e.to_string())?;
    }
    take.samples.clear();
    Ok(path)
}
struct Processor {
    kind: Kind,
    p: Arc<Shared>,
    bus: Arc<AudioBus>,
    rx: Receiver<Command>,
    takes: SyncSender<Take>,
    ring: Vec<f32>,
    write: usize,
    filled: usize,
    phases: [f32; 16],
    env: [f32; 16],
    freq: [f32; 16],
    clock: f32,
    beat: usize,
    rng: u32,
    genes: [f32; 12],
    low: f32,
    high: f32,
    match_a: f32,
    match_b: f32,
    read: f64,
    clip: Option<Clip>,
    recordings: Vec<Vec<[f32; 2]>>,
    track_settings: [[f32; 4]; 8],
    was_record: bool,
    record_pos: usize,
    rate: f32,
    input: Vec<f32>,
    reference: Vec<f32>,
    mono: Vec<f32>,
    view_clock: usize,
    fft: Arc<dyn rustfft::Fft<f32>>,
    fft_buf: Vec<Complex<f32>>,
    fft_scratch: Vec<Complex<f32>>,
    pending_save: bool,
    marker: usize,
    stretch_clock: usize,
    export: Vec<[f32; 2]>,
    export_at: usize,
    export_end: usize,
    export_values: [f32; 6],
    export_tracks: [[f32; 4]; 8],
    recorded_peak: f32,
    grain_origin: f32,
    slice_pad: usize,
    walk: usize,
    radio_frames: Vec<[f32; 2]>,
    radio_phase: f32,
    voice_amplitude: [f32; 16],
    voice_tone: [f32; 16],
    sample_position: [f64; 16],
    previous_manual: [bool; 16],
    note_values: [f32; 16],
    motion_clock: f32,
    perf: [f32; 8],
    audition: Option<i32>,
    /// Where generated notes go (synth kinds; see note_bus.rs).
    notes: crate::note_bus::NoteOut,
}
impl Processor {
    /// How long one take may be (frames).
    fn take_cap(&self) -> usize {
        if self.kind == Kind::Studio { STUDIO_CAP } else { CAP }
    }
    fn new(
        kind: Kind,
        p: Arc<Shared>,
        bus: Arc<AudioBus>,
        rx: Receiver<Command>,
        takes: SyncSender<Take>,
    ) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(512);
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        Self {
            kind,
            p,
            bus,
            rx,
            takes,
            ring: if kind.synth() || kind.media() {
                vec![]
            } else {
                vec![0.; CAP]
            },
            write: 0,
            filled: 0,
            phases: [0.; 16],
            env: [0.; 16],
            freq: [110.; 16],
            clock: 0.,
            beat: 0,
            rng: 0x12875123,
            genes: std::array::from_fn(|i| 1. / (i + 1) as f32),
            low: 0.,
            high: 0.,
            match_a: 0.,
            match_b: 0.,
            read: 0.,
            clip: None,
            recordings: if kind.capture() {
                (0..if kind == Kind::Studio { 8 } else { 1 })
                    .map(|_| Vec::with_capacity(if kind == Kind::Studio { STUDIO_CAP } else { CAP }))
                    .collect()
            } else {
                vec![]
            },
            track_settings: [[0.8, 0., 0., 0.]; 8],
            was_record: false,
            record_pos: 0,
            rate: RATE,
            input: Vec::with_capacity(4096),
            reference: Vec::with_capacity(4096),
            mono: Vec::with_capacity(4096),
            view_clock: 0,
            fft,
            fft_buf: vec![Complex::default(); 512],
            fft_scratch: scratch,
            pending_save: false,
            marker: 0,
            stretch_clock: 0,
            export: if kind.capture() {
                Vec::with_capacity(if kind == Kind::Studio { STUDIO_CAP } else { CAP })
            } else {
                vec![]
            },
            export_at: 0,
            export_end: 0,
            export_values: kind.defaults(),
            export_tracks: [[0.8, 0., 0., 0.]; 8],
            recorded_peak: 0.,
            grain_origin: 0.,
            slice_pad: 0,
            walk: 0,
            radio_frames: Vec::with_capacity(4096),
            radio_phase: 0.,
            voice_amplitude: [0.; 16],
            voice_tone: [0.; 16],
            sample_position: [0.; 16],
            previous_manual: [false; 16],
            note_values: [0.; 16],
            motion_clock: 0.,
            perf: [0.; 8],
            audition: None,
            notes: crate::note_bus::NoteOut::detached(),
        }
    }
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }
    fn delay(&self, delay: f32) -> f32 {
        if self.ring.is_empty() {
            return 0.;
        }
        let pos = (self.write as f32 - delay.clamp(1., (self.ring.len() - 2) as f32))
            .rem_euclid(self.ring.len() as f32);
        let i = pos as usize;
        let f = pos - i as f32;
        self.ring[i] * (1. - f) + self.ring[(i + 1) % self.ring.len()] * f
    }
    fn commands(&mut self) {
        while let Ok(c) = self.rx.try_recv() {
            match c {
                Command::Load(c) => {
                    self.p.duration.set(c.samples.len() as f32 / c.rate);
                    self.clip = Some(c);
                    self.read = 0.;
                    self.p.position.set(0.);
                }
                Command::Audition(note) => {
                    self.audition = Some(note);
                    self.read = 0.;
                    self.p.record.store(false, Ordering::Relaxed);
                    self.p.play.store(true, Ordering::Relaxed);
                }
                Command::Seek(f) => {
                    if let Some(c) = &self.clip {
                        self.read = f as f64 * c.samples.len().saturating_sub(1) as f64
                    }
                }
                Command::Clear => {
                    if self.p.export_busy.load(Ordering::Relaxed) {
                        continue;
                    }
                    for t in &mut self.recordings {
                        t.clear()
                    }
                    self.read = 0.;
                    self.record_pos = 0;
                    self.p.duration.set(0.);
                }
                Command::Save => {
                    if !self.p.record.load(Ordering::Relaxed)
                        && !self.pending_save
                        && self.export.capacity() >= self.take_cap()
                    {
                        self.pending_save = true;
                        self.p.export_busy.store(true, Ordering::Relaxed);
                        self.export.clear();
                        self.export_values = std::array::from_fn(|i| self.p.values[i].get());
                        self.export_tracks = self.track_settings;
                        let len = self.recordings.iter().map(Vec::len).max().unwrap_or(0);
                        self.export_at = if self.kind == Kind::SampleHunter {
                            (self.export_values[0] / 100. * len as f32) as usize
                        } else {
                            0
                        };
                        self.export_end = if self.kind == Kind::SampleHunter && len > 0 {
                            ((self.export_values[1] / 100. * len as f32) as usize)
                                .clamp(self.export_at + 1, len)
                        } else {
                            len
                        };
                    }
                }
                Command::Recycle(mut buffer) => {
                    buffer.clear();
                    self.export = buffer;
                }
                Command::Marker => self.marker = self.record_pos,
                Command::Track(i, v) => self.track_settings[i] = v,
                Command::Breed => {
                    let mutation = self.p.values[1].get();
                    for i in 0..12 {
                        let r = self.random();
                        self.genes[i] = (self.genes[i] * (1. - mutation) + r * mutation)
                            / (1. + i as f32 * 0.05);
                    }
                }
            }
        }
    }
    fn synth(&mut self, v: [f32; 6], pads: usize, play: bool, rate: f32) -> f32 {
        let tempo = if matches!(self.kind, Kind::Orbit | Kind::Dream) {
            v[0]
        } else {
            110.
        };
        let mut trigger = false;
        let swing = if self.kind == Kind::Orbit { v[5] } else { 0. };
        let beat_len = 60. * rate / tempo / 4.
            * if self.beat.is_multiple_of(2) {
                1. + swing
            } else {
                1. - swing
            };
        if play {
            self.clock += 1.;
            if self.clock >= beat_len {
                self.clock -= beat_len;
                self.beat += 1;
                trigger = true;
            }
        }
        let root = match self.kind {
            Kind::Orbit => v[2],
            Kind::Swarm | Kind::Mutant => v[3],
            Kind::Constellation => v[0],
            _ => self.p.dream_root.load(Ordering::Relaxed) as f32,
        };
        if self.kind == Kind::Dream
            && trigger
            && self.beat.is_multiple_of(v[1] as usize * 16)
            && self.random() < v[2]
        {
            let n = (self.p.step.load(Ordering::Relaxed) + 1) % 4;
            self.p.step.store(n, Ordering::Relaxed);
        }
        if self.kind != Kind::Dream {
            self.p.step.store(self.beat % 16, Ordering::Relaxed);
        }
        let state = self.p.step.load(Ordering::Relaxed) % 4;
        let count = match self.kind {
            Kind::Swarm => v[0] as usize,
            Kind::Constellation => v[5] as usize,
            _ => 16,
        }
        .clamp(1, 16);
        let generated = if trigger && rhythm_gate(self.p.rhythm.load(Ordering::Relaxed),self.beat) {
            let choice=(self.random()*65536.) as usize;
            if self.kind == Kind::Constellation && self.p.pattern.load(Ordering::Relaxed)==5 {(1<<count)-1} else {pattern_mask(self.p.pattern.load(Ordering::Relaxed),self.beat,count,choice,&mut self.walk)}
        } else {0};
        let perf = self.perf;
        self.motion_clock = (self.motion_clock + perf[6] / rate).fract();
        let color = (perf[1] + (self.motion_clock * TAU).sin() * perf[7] * 0.4).clamp(0., 1.);
        let color = if self.kind == Kind::Mutant {
            (color + v[0] * (self.genes[1] - 0.5) * 0.5).clamp(0., 1.)
        } else {
            color
        };
        // Tone and motion also shape imported/live audio and the sine voice.
        let tone_alpha = (TAU * (200. * 2f32.powf(color * 6.)) / rate).min(1.);
        let mut result = 0.;
        for i in 0..16 {
            let keys = !matches!(self.kind, Kind::Orbit | Kind::Dream)
                || self.p.alternate.load(Ordering::Relaxed);
            let manual = pads & (1 << i) != 0 && keys;
            let manual_edge = manual && !self.previous_manual[i];
            self.previous_manual[i] = manual;
            let fire = if !trigger || generated == 0 {
                false
            } else {
                match self.kind {
                    Kind::Orbit => {
                        pads & (1 << i) != 0
                            && (self.p.pattern.load(Ordering::Relaxed)==0 || generated & (1<<i)!=0)
                            && self.beat.is_multiple_of(1 + (i / 4) * v[1] as usize)
                            && self.random() < v[4]
                    }
                    Kind::Dream => {
                        generated & (1<<i)!=0 && self.random() < v[5]
                    }
                    _ => generated & (1<<i)!=0,
                }
            };
            // Generated notes sound here, go to another app, or (route
            // None) nowhere; the pads always play here.
            if manual_edge || (fire && self.notes.internal()) {
                self.env[i] = 1.;
                self.sample_position[i] = 0.;
            }

            if manual {
                self.env[i] = (self.env[i] + 0.004).min(1.);
            }
            let scale=perf[4] as usize;
            let degree = if self.kind == Kind::Constellation {
                (i%4)*2 + v[2] as usize + if i%4==3 {(v[4]*2.).round() as usize} else {0}
            } else if self.kind == Kind::Dream {
                i % ((v[3]*12.) as usize+1) + (state as f32*v[4]*3.).round() as usize
            } else {i};
            let octave = if self.kind==Kind::Constellation {(i/4) as f32*v[1]*12.}else{0.};
            let note=root + music_scales::degree(scale,degree) as f32 + octave + perf[5]*12.;
            self.note_values[i] = note;
            if fire && self.notes.external() {
                self.notes.trigger(note.round().clamp(0., 127.) as u8, 100, (beat_len * 1.5) as u32);
            }
            if self.env[i] < 0.00001 && self.voice_amplitude[i] < 0.00001 {
                self.voice_amplitude[i] = 0.;
                continue;
            }
            let target = 440. * 2f32.powf((note - 69.) / 12.);
            let glide = if self.kind == Kind::Constellation {
                v[3]
            } else {
                0.
            };
            self.freq[i] += (target - self.freq[i]) * (1. / (1. + glide * rate));
            let detune = if self.kind == Kind::Swarm {
                1. + (i as f32 - 7.5) * v[2] * 0.003 * (1. - v[1])
                    + ((self.beat + i) as f32 * 0.713).sin() * v[4] * 0.01
            } else {
                1.
            };
            self.phases[i] = (self.phases[i] + self.freq[i] * detune / rate).fract();
            let phase = self.phases[i] * TAU;
            let voice = perf[0] as usize;
            let mut osc = 0.;
            if voice == 6 {
                if !self.input.is_empty() {
                    let index = self.phases[i] * self.input.len() as f32;
                    let a = index as usize % self.input.len();
                    let b = (a + 1) % self.input.len();
                    osc = self.input[a] * (1. - index.fract()) + self.input[b] * index.fract();
                }
            } else if voice == 7 {
                if let Some(clip) = &self.clip {
                    let index = self.sample_position[i] as usize;
                    if let Some(frame) = clip.samples.get(index) {
                        let next = clip.samples.get(index + 1).unwrap_or(frame);
                        let (frame, next) = (&frame, &next);
                        let f = self.sample_position[i].fract() as f32;
                        osc =
                            (frame[0] + frame[1]) * (1. - f) * 0.5 + (next[0] + next[1]) * f * 0.5;
                        self.sample_position[i] +=
                            self.freq[i] as f64 / 261.6256 * clip.rate as f64 / rate as f64;
                    }
                }
            } else if voice == 1 {
                osc = (phase + (phase * 2.).sin() * color * 4. * self.env[i]).sin();
            } else {
                let harmonics = if voice == 0 { 1 } else { 8 };
                let mut norm = 0.;
                for h in 1..=harmonics {
                    if self.freq[i] * h as f32 >= rate * 0.45 {
                        continue;
                    }
                    let hf = h as f32;
                    let mut amp = match voice {
                        0 => 1.,
                        2 => {
                            if h % 2 == 1 {
                                1. / hf
                            } else {
                                color * 0.18 / hf
                            }
                        }
                        3 => (-hf * (1. - color) * 0.5).exp() / hf,
                        4 => {
                            if [1, 2, 3, 4, 6, 8].contains(&h) {
                                1. / hf.powf(2. - color * 1.7)
                            } else {
                                0.
                            }
                        }
                        5 => {
                            if h < 4 {
                                1. / (hf * hf)
                            } else {
                                color * 0.1 / hf
                            }
                        }
                        _ => 1. / hf,
                    };
                    if self.kind == Kind::Mutant {
                        amp *= ((1. - v[0]) / hf + v[0] * self.genes[h - 1])
                            * (0.1 + v[4] * 0.9).powi((h - 1) as i32);
                        if h > v[2] as usize {
                            amp = 0.;
                        }
                    }
                    osc += (phase * hf).sin() * amp;
                    norm += amp;
                }
                osc /= norm.max(1.);
            }
            self.voice_tone[i] += tone_alpha * (osc - self.voice_tone[i]);
            osc = self.voice_tone[i];
            let attack = (1. / (perf[2].max(0.001) * rate)).min(1.);
            self.voice_amplitude[i] += (self.env[i] - self.voice_amplitude[i])
                * if self.env[i] > self.voice_amplitude[i] {
                    attack
                } else {
                    0.02
                };
            if i < count || self.kind == Kind::Orbit || self.kind == Kind::Dream || manual {
                result += osc * self.voice_amplitude[i];
            }
            let release = match self.kind {
                Kind::Orbit => v[3] * 60. / tempo,
                Kind::Swarm | Kind::Mutant => v[5],
                _ => 0.45,
            } * (perf[3] / 0.45).clamp(0.2, 4.);
            if !manual {
                self.env[i] *= (-1. / (release.max(0.01) * rate)).exp();
                if self.env[i] < 0.00001 {
                    self.env[i] = 0.;
                }
            }
        }
        self.p
            .position
            .set((self.beat as f32 + self.clock / beat_len) / 16.);
        result * 0.12 / (count as f32).sqrt()
    }
    fn media_frame(&mut self, v: [f32; 6], rate: f32) -> [f32; 2] {
        let Some(c) = &self.clip else { return [0.; 2] };
        if c.samples.is_empty() {
            return [0.; 2];
        }
        let len = c.samples.len();
        let practice = self.kind == Kind::Practice;
        let a = if practice {
            (v[1] / 100. * len as f32) as usize
        } else {
            0
        }
        .min(len - 1);
        let b = if practice {
            (v[2] / 100. * len as f32) as usize
        } else {
            len
        }
        .clamp(a + 1, len);
        if self.read < a as f64 {
            self.read = a as f64
        }
        if self.read >= b as f64 {
            if practice || v[4] > 0. {
                self.read = a as f64;
            } else {
                self.p.play.store(false, Ordering::Relaxed);
                self.p.ended.store(true, Ordering::Relaxed);
                return [0.; 2];
            }
        }
        let speed = if practice {
            v[0]
        } else if self.kind == Kind::Vinyl {
            v[1]
        } else {
            1.
        };
        let pitch = if practice { 2f32.powf(v[3] / 12.) } else { 1. };
        let sample = |pos: f64| {
            let pos = a as f64 + (pos - a as f64).rem_euclid((b - a) as f64);
            let i = pos as usize;
            let j = if i + 1 < b { i + 1 } else { a };
            let f = (pos - i as f64) as f32;
            let (si, sj) = (c.samples.at(i), c.samples.at(j));
            [si[0] * (1. - f) + sj[0] * f, si[1] * (1. - f) + sj[1] * f]
        };
        // Two overlapped 40ms grains decouple practice speed from pitch. This
        // lightweight granular stretch is deliberately not a phase-vocoder claim.
        let mut out = if practice {
            let size = (rate * 0.04) as usize;
            let p = self.stretch_clock % size;
            let q = (p + size / 2) % size;
            let w = 0.5 - 0.5 * (TAU * p as f32 / size as f32).cos();
            let delta = (pitch - speed) * c.rate / rate;
            let x = sample(self.read + (p as f32 - size as f32 * 0.5) as f64 * delta as f64);
            let y = sample(self.read + (q as f32 - size as f32 * 0.5) as f64 * delta as f64);
            [x[0] * w + y[0] * (1. - w), x[1] * w + y[1] * (1. - w)]
        } else {
            sample(self.read)
        };
        self.read += speed as f64 * c.rate as f64 / rate as f64;
        self.stretch_clock += 1;
        let gain = if matches!(self.kind, Kind::Reference | Kind::Radio) {
            10f32.powf(v[1] / 20.)
                * if v[2] > 0. {
                    0.9 / c.peak.max(0.001)
                } else {
                    1.
                }
                * v[5]
        } else if practice {
            0.8
        } else {
            v[5]
        };
        let click = if practice && v[4] > 0. {
            let period = rate * 60. / v[4];
            let t = self.stretch_clock as f32 % period;
            if t < rate * 0.02 {
                (TAU * 1000. * t / rate).sin() * (1. - t / (rate * 0.02)) * v[5] * 0.3
            } else {
                0.
            }
        } else {
            0.
        };
        out[0] = out[0] * gain + click;
        out[1] = out[1] * gain + click;
        self.p.position.set(self.read as f32 / c.rate);
        out
    }
    fn capture_frame(
        &mut self,
        x: f32,
        v: [f32; 6],
        rate: f32,
        record: bool,
        play: bool,
    ) -> [f32; 2] {
        let studio = self.kind == Kind::Studio;
        let track = if studio {
            (v[0] as usize - 1).min(7)
        } else {
            0
        };
        if record && !self.was_record {
            self.record_pos = 0;
            self.read = 0.;
            if !studio {
                self.recordings[0].clear();
                self.recorded_peak = 0.;
                if self.kind == Kind::Field {
                    let n = (v[1] * rate) as usize;
                    let n = n.min(self.filled).min(CAP);
                    for i in (1..=n).rev() {
                        let s = self.delay(i as f32);
                        self.recordings[0].push([s, s]);
                    }
                    self.record_pos = self.recordings[0].len();
                }
            }
        }
        self.was_record = record;
        let mut input = x;
        if self.kind == Kind::Field {
            input *= v[0];
            let alpha = (TAU * v[3] / rate).min(1.);
            self.high += alpha * (input - self.high);
            input -= self.high;
            if v[2] > 0. {
                input = input.clamp(-0.98, 0.98);
            }
        } else if self.kind == Kind::SampleHunter && x.abs() < v[4] {
            input = 0.;
        }
        if record {
            self.recorded_peak = self.recorded_peak.max(input.abs());
            if self.record_pos < self.take_cap() {
                let t = &mut self.recordings[track];
                if self.record_pos < t.len() {
                    t[self.record_pos] = [input, input];
                } else {
                    t.resize(self.record_pos + 1, [0.; 2]);
                    t[self.record_pos] = [input, input];
                }
                self.record_pos += 1;
                self.read = self.record_pos as f64;
            } else {
                self.p.record.store(false, Ordering::Relaxed);
                self.p.ended.store(true, Ordering::Relaxed);
            }
        }
        let duration = self.recordings.iter().map(Vec::len).max().unwrap_or(0);
        self.p.duration.set(duration as f32 / rate);
        let mut out = [0.; 2];
        if (play || (record && studio)) && duration > 0 {
            let (start, end, speed) = if self.kind == Kind::SampleHunter {
                let a = (v[0] / 100. * duration as f32) as usize;
                let b = ((v[1] / 100. * duration as f32) as usize).clamp(a + 1, duration);
                (
                    a,
                    b,
                    2f32.powf((v[3] + self.audition.unwrap_or(0) as f32) / 12.),
                )
            } else {
                (0, duration, 1.)
            };
            if !record && self.read >= end as f64 && self.audition.is_some() {
                self.p.play.store(false, Ordering::Relaxed);
                self.audition = None;
                return [0.; 2];
            }
            if !record && (self.read < start as f64 || self.read >= end as f64) {
                self.read = start as f64;
            }

            let solo = self.track_settings.iter().any(|t| t[3] > 0.);
            for (i, t) in self.recordings.iter().enumerate() {
                if record && studio && i == track {
                    continue;
                }
                if let Some(s) = t.get(if record {
                    self.record_pos.saturating_sub(1)
                } else {
                    self.read as usize
                }) {
                    let cfg = self.track_settings[i];
                    if studio && (cfg[2] > 0. || (solo && cfg[3] == 0.)) {
                        continue;
                    }
                    let level = if studio {
                        cfg[0]
                    } else if self.kind == Kind::Field {
                        v[5]
                    } else if v[2] > 0. {
                        0.98 / self.recorded_peak.max(0.001)
                    } else {
                        0.8
                    };
                    let pan = if studio { cfg[1] } else { 0. };
                    out[0] += s[0] * level * (1. - pan).min(1.);
                    out[1] += s[1] * level * (1. + pan).min(1.);
                }
            }
            if !record {
                self.read += speed as f64;
            }
        }
        let monitor = if self.kind == Kind::Field {
            v[4] > 0.
        } else {
            v[5] > 0.
        };
        if monitor {
            out[0] += input * 0.5;
            out[1] += input * 0.5;
        }
        self.p.position.set(self.read as f32 / rate);
        out
    }
    fn effect(&mut self, x: f32, b: f32, v: [f32; 6], pads: usize, rate: f32) -> f32 {
        match self.kind {
            Kind::Fracture => {
                let length = (v[0] * rate).max(1.);
                if pads != self.slice_pad {
                    self.clock = 0.;
                    self.slice_pad = pads;
                }
                if self.clock == 0. {
                    let slice = if pads != 0 {
                        pads.trailing_zeros() as f32
                    } else {
                        (self.beat % 16) as f32 * v[4]
                    };
                    self.grain_origin =
                        (self.write as f32 - length * (1. + slice)).rem_euclid(CAP as f32);
                }
                let n = self.clock % length;
                let pitch = 2f32.powf(v[3] / 12.);
                let position = (self.grain_origin
                    + if v[2] > 0. {
                        length - n * pitch
                    } else {
                        n * pitch
                    })
                .rem_euclid(CAP as f32);
                let delay = (self.write as f32 - position).rem_euclid(CAP as f32);
                // Short edge fades prevent discontinuities at slice boundaries.
                let fade = (n / 32.).min((length - n) / 32.).clamp(0., 1.);
                let wet = self.delay(delay) * fade;
                self.clock += 1.;
                if self.clock >= length * v[1] {
                    self.clock = 0.;
                    self.beat += 1;
                }
                x * (1. - v[5]) + wet * v[5]
            }
            Kind::Ghosts => {
                let mut sum = 0.;
                for i in 1..=4 {
                    if pads & 15 != 0 && pads & (1 << (i - 1)) == 0 {
                        continue;
                    }
                    let drift =
                        (self.clock / rate * (0.11 * i as f32) * TAU).sin() * v[2] * rate * 0.01;
                    let time = v[0] * rate * i as f32 + drift;
                    let time = if v[4] > 0. {
                        time + (self.clock % (v[0] * rate)) * 2.
                    } else {
                        time
                    };
                    sum += self.delay(time) * v[1].powi(i);
                }
                self.clock += 1.;
                self.low += (sum - self.low) * (TAU * v[3] / rate).min(1.);
                x * (1. - v[5]) + self.low * v[5] * 0.5
            }
            Kind::TapeMachine => {
                let wow = (self.clock / rate * 0.6 * TAU).sin() * v[3] * 0.01;
                let flutter = (self.clock / rate * 8. * TAU).sin() * v[4] * 0.001;
                self.clock += 1.;
                let mut wet = 0.;
                for i in 1..=4 {
                    if pads & 15 != 0 && pads & (1 << (i - 1)) == 0 {
                        continue;
                    }
                    wet += self.delay((v[1] * i as f32 / v[0] + wow + flutter) * rate) / 4.;
                }
                let y = (x + wet * v[2]).tanh();
                if !self.p.freeze.load(Ordering::Relaxed) {
                    self.ring[self.write] = y;
                }
                ((x * 0.5 + wet) * v[5]).tanh() / v[5].sqrt()
            }
            Kind::Portal => {
                let x = if self.p.reference.load(Ordering::Relaxed) != NO_SOURCE {
                    x * (1. - self.p.color.get()) + b * self.p.color.get()
                } else {
                    x
                };
                self.high += (x - self.high) * (TAU * v[2] / rate).min(1.);
                let hp = x - self.high;
                self.low += (hp - self.low) * (TAU * v[3] / rate).min(1.);
                let s = if v[4] > 0. {
                    self.delay(v[4] * rate / 1000.)
                } else {
                    self.low
                };
                s * v[0] * if v[1] > 0. { -1. } else { 1. }
            }
            Kind::Master => {
                self.match_a = self.match_a * 0.9999 + x * x * 0.0001;
                self.match_b = self.match_b * 0.9999 + b * b * 0.0001;
                let selected = if v[0] > 0. { b } else { x };
                if self.p.freeze.load(Ordering::Relaxed) {
                    return selected;
                }
                let gain = if v[1] > 0. && v[0] > 0. {
                    (self.match_a / (self.match_b + 1e-8))
                        .sqrt()
                        .clamp(0.25, 4.)
                } else {
                    1.
                };
                self.low += (selected - self.low) * (TAU * 160. / rate).min(1.);
                self.high += (selected - self.high) * (TAU * 2500. / rate).min(1.);
                let eq = selected
                    + self.low * (10f32.powf(v[2] / 20.) - 1.)
                    + (selected - self.high) * (10f32.powf(v[3] / 20.) - 1.);
                let ceiling = 10f32.powf(v[5] / 20.);
                (eq * gain * v[4]).tanh().clamp(-ceiling, ceiling)
            }
            _ => 0.,
        }
    }
}
impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.);
        if channels == 0 || !rate.is_finite() || rate <= 0. {
            return;
        }
        self.rate = rate;
        self.commands();
        self.input.clear();
        self.reference.clear();
        if let Some(b) = self.bus.get(self.p.source.load(Ordering::Relaxed)) {
            if let Ok(b) = b.try_lock() {
                self.input.extend_from_slice(&b);
            }
        }
        if let Some(b) = self.bus.get(self.p.reference.load(Ordering::Relaxed)) {
            if let Ok(b) = b.try_lock() {
                self.reference.extend_from_slice(&b);
            }
        }
        let v = std::array::from_fn(|i| {let (lo,hi,step)=self.kind.ranges()[i];let value=(self.p.values[i].get()+self.p.modulation[i].get()*(hi-lo)).clamp(lo,hi);if step>=1.{value.round()}else{value}});
        self.perf = [
            self.p.voice.load(Ordering::Relaxed) as f32,
            (self.p.color.get()+self.p.color_modulation.get()).clamp(0.,1.),
            self.p.attack.get(),
            self.p.release.get(),
            self.p.scale.load(Ordering::Relaxed) as f32,
            self.p.octave.get(),
            self.p.motion_rate.get(),
            self.p.motion_depth.get(),
        ];
        let pads = self.p.pads.load(Ordering::Relaxed);
        let play = self.p.play.load(Ordering::Relaxed);
        let record = self.p.record.load(Ordering::Relaxed);
        let mix = (self.p.mix.get() + self.p.ext.get()).clamp(0., 2.);
        // notes sent to another app end on time (or all at once on stop)
        if play {
            self.notes.advance((out.len() / channels) as u32);
        } else {
            self.notes.all_off();
        }
        self.mono.clear();
        let radio = self.p.radio.try_lock().ok().and_then(|s| s.clone());
        let mut radio_ended = false;
        if let Some(stream) = &radio {
            if play {
                radio_ended = stream.render(
                    &mut self.radio_frames,
                    out.len() / channels,
                    rate,
                    &mut self.radio_phase,
                    v[0],
                );
            }
        }
        for (i, frame) in out.chunks_mut(channels).enumerate() {
            let x = self.input.get(i).copied().unwrap_or(0.);
            let b = self.reference.get(i).copied().unwrap_or(0.);
            let mut y = if self.kind.synth() {
                let s = self.synth(v, pads, play, rate);
                [s, s]
            } else if self.kind.media() {
                if self.p.play.load(Ordering::Relaxed) {
                    if self.kind == Kind::Radio && radio.is_some() {
                        let frame = self.radio_frames.get(i).copied().unwrap_or([0.; 2]);
                        let gain = 10f32.powf(v[1] / 20.) * v[5];
                        self.p.position.set(self.p.position.get() + 1. / rate);
                        frame.map(|s| {
                            if v[2] > 0. {
                                (s * gain).clamp(-0.95, 0.95)
                            } else {
                                s * gain
                            }
                        })
                    } else {
                        self.media_frame(v, rate)
                    }
                } else {
                    [0.; 2]
                }
            } else if self.kind.capture() {
                self.capture_frame(
                    x,
                    v,
                    rate,
                    self.p.record.load(Ordering::Relaxed),
                    self.p.play.load(Ordering::Relaxed),
                )
            } else if self.kind == Kind::Scope {
                [0.; 2]
            } else if play || self.kind.passive() {
                let s = self.effect(x, b, v, pads, rate);
                [s, s]
            } else {
                [0.; 2]
            };
            if !self.ring.is_empty() {
                if (self.kind != Kind::TapeMachine || !play)
                    && !self.p.freeze.load(Ordering::Relaxed)
                {
                    self.ring[self.write] = x;
                }
                self.write = (self.write + 1) % self.ring.len();
                self.filled = (self.filled + 1).min(self.ring.len());
            }
            let mono = if self.kind == Kind::Scope {
                x
            } else {
                (y[0] + y[1]) * 0.5
            };
            self.mono.push(if mono.is_finite() {
                mono.clamp(-4., 4.)
            } else {
                0.
            });
            if self.kind == Kind::Portal && v[5] == 0. {
                y = [0.; 2];
            }
            for (c, s) in frame.iter_mut().enumerate() {
                let sample = y[c % 2] * mix;
                *s = if sample.is_finite() {
                    sample.clamp(-1., 1.)
                } else {
                    0.
                };
            }
        }
        if radio_ended {
            self.p.play.store(false, Ordering::Relaxed);
            self.p.ended.store(true, Ordering::Relaxed);
        }
        if let Ok(mut bus) = self.p.out.try_lock() {
            bus.clear();
            bus.extend_from_slice(&self.mono);
        }
        self.view_clock += out.len() / channels;
        if self.view_clock >= 1600 && !(self.kind == Kind::Scope && v[5] > 0.) {
            self.view_clock = 0;
            let measurement = if self.kind.capture() && record {
                &self.input
            } else {
                &self.mono
            };
            let peak = measurement.iter().fold(0f32, |a, v| a.max(v.abs()));
            let rms = (measurement.iter().map(|v| v * v).sum::<f32>()
                / measurement.len().max(1) as f32)
                .sqrt();
            let scope = self.kind == Kind::Scope;
            let gain = if scope { v[1] } else { 1. };
            let span = if scope {
                (v[0] * rate / 1000.) as usize
            } else {
                self.mono.len()
            };
            let span = span.min(CAP - 1024);
            let lookback = span + 512;
            let mut start = 0;
            if scope {
                for i in 1..512 {
                    if self.delay((lookback - i + 1) as f32) < v[2]
                        && self.delay((lookback - i) as f32) >= v[2]
                    {
                        start = i;
                        break;
                    }
                }
            }
            let wave = (0..96)
                .map(|i| {
                    if scope {
                        self.delay((lookback - start - i * span.max(1) / 96) as f32) * gain
                    } else {
                        measurement
                            .get(i * measurement.len().max(1) / 96)
                            .copied()
                            .unwrap_or(0.)
                    }
                })
                .collect();
            for i in 0..512 {
                let sample = if scope {
                    self.delay((512 - i) as f32)
                } else {
                    measurement.get(i).copied().unwrap_or(0.)
                };
                let window = if scope && v[3] == 0. {
                    1.
                } else {
                    0.5 - 0.5 * (TAU * i as f32 / 512.).cos()
                };
                self.fft_buf[i] = Complex::new(sample * window, 0.);
            }
            self.fft
                .process_with_scratch(&mut self.fft_buf, &mut self.fft_scratch);
            let spectrum: Vec<f32> = (0..48)
                .map(|i| {
                    let a = 1 + i * 5;
                    self.fft_buf[a..(a + 5).min(256)]
                        .iter()
                        .map(|c| c.norm() / 128.)
                        .fold(0f32, f32::max)
                })
                .collect();
            let levels = if self.kind == Kind::Studio {
                (0..8)
                    .map(|i| {
                        let index = self.read as usize;
                        let end = index.min(self.recordings[i].len());
                        self.recordings[i][end.saturating_sub(512)..end]
                            .iter()
                            .map(|f| f[0].abs() * self.track_settings[i][0])
                            .fold(0f32, f32::max)
                    })
                    .collect()
            } else {
                self.env.to_vec()
            };
            let buffer: Vec<f32> = if self.kind==Kind::Fracture {
                (0..128).map(|i|self.delay(v[0]*rate*((i/8+1) as f32-(i%8) as f32/8.))).collect()
            } else if let Some(clip) = &self.clip {
                clip.samples.overview(128)
            } else if self.kind.capture() {
                let track = if self.kind == Kind::Studio {
                    (v[0] as usize - 1).min(7)
                } else {
                    0
                };
                let take = &self.recordings[track];
                take_overview(take, 128)
            } else if !self.ring.is_empty() {
                (0..128)
                    .map(|i| {
                        (0..16)
                            .map(|j| {
                                self.delay((1 + i * self.filled.max(1) / 128 + j) as f32)
                                    .abs()
                            })
                            .fold(0f32, f32::max)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let tracks: Vec<f32> = if self.kind==Kind::Ghosts {
                (0..4).flat_map(|layer| (0..64).map(move |i|(layer,i))).map(|(layer,i)|{
                    let n=(layer+1) as f32;
                    let drift=(self.clock/rate*(0.11*n)*TAU).sin()*v[2]*rate*0.01;
                    let reverse=if v[4]>0.{(self.clock%(v[0]*rate))*2.}else{0.};
                    self.delay((v[0]*rate*n+drift+reverse-i as f32*8.).max(1.))*v[1].powi(layer+1)
                }).collect()
            } else if self.kind == Kind::Studio {
                self.recordings
                    .iter()
                    .flat_map(|take| take_overview(take, 32))
                    .collect()
            } else {
                Vec::new()
            };
            if let Ok(mut view) = self.p.view.try_lock() {
                *view = View {
                    buffer,
                    tracks,
                    phases: self.phases.to_vec(),
                    notes: self.note_values.to_vec(),
                    genes: self.genes.to_vec(),
                    wave,
                    spectrum,
                    levels,
                    peak,
                    rms,
                };
            }
        }
        if self.pending_save && !record {
            // At most 512 frames copied per callback. No bulk take clone or disk
            // operation on the real-time thread. Worker returns the spare buffer.
            let end = (self.export_at + 512).min(self.export_end);
            let solo = self.export_tracks.iter().any(|c| c[3] > 0.);
            for index in self.export_at..end {
                let mut frame = [0.; 2];
                for (track, cfg) in self.recordings.iter().zip(self.export_tracks) {
                    if self.kind == Kind::Studio && (cfg[2] > 0. || (solo && cfg[3] == 0.)) {
                        continue;
                    }
                    if let Some(s) = track.get(index) {
                        let level = if self.kind == Kind::Studio {
                            cfg[0]
                        } else {
                            1.
                        };
                        let pan = if self.kind == Kind::Studio {
                            cfg[1]
                        } else {
                            0.
                        };
                        frame[0] += s[0] * level * (1. - pan).min(1.);
                        frame[1] += s[1] * level * (1. + pan).min(1.);
                    }
                }
                self.export.push(frame);
            }
            self.export_at = end;
            if end == self.export_end {
                let take = Take {
                    samples: std::mem::take(&mut self.export),
                    rate: rate as u32,
                    normalize: self.kind == Kind::SampleHunter && self.export_values[2] > 0.,
                    marker: self.marker,
                };
                match self.takes.try_send(take) {
                    Ok(()) => self.pending_save = false,
                    Err(TrySendError::Full(take)) => self.export = take.samples,
                    Err(TrySendError::Disconnected(take)) => {
                        self.export = take.samples;
                        self.pending_save = false;
                        self.p.export_busy.store(false, Ordering::Relaxed);
                    }
                }
            }
        }
        self.p.audible.store(
            self.kind.synth() && self.env.iter().any(|v| *v > 0.0001),
            Ordering::Relaxed,
        );
        if matches!(self.kind, Kind::TapeMachine | Kind::Fracture | Kind::Ghosts) {
            self.p.position.set(self.clock / rate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patched_scope_does_not_request_background_processing() {
        let (app,mut dsp,input)=fixture(Kind::Scope);signal(&input);app.p.source.store(0,Ordering::Relaxed);
        dsp.process(&mut [0.;1024],2,48000.);
        assert!(!app.needs_background_audio(),"an invisible analyzer should sleep unless its output is consumed");
    }

    #[test]
    fn musical_patterns_change_direction_and_density() {
        let sequence=|pattern|{let mut walk=0;(0..8).map(|step|pattern_mask(pattern,step,4,step*17+2,&mut walk)).collect::<Vec<_>>()};
        assert_eq!(sequence(0),vec![1,2,4,8,1,2,4,8]);
        assert_eq!(sequence(1),vec![8,4,2,1,8,4,2,1]);
        assert_eq!(sequence(2),vec![1,2,4,8,4,2,1,2]);
        assert_ne!(sequence(3),sequence(0));assert_ne!(sequence(4),sequence(0));
        for count in 1..=16 {for pattern in 0..6 {let mut walk=0;for step in 0..100 {let mask=pattern_mask(pattern,step,count,step*11,&mut walk);assert!(mask>0 && mask<(1<<count));}}}
        let densities:Vec<_>=(0..5).map(|r|(0..16).filter(|s|rhythm_gate(r,*s)).count()).collect();
        assert_eq!(densities,vec![16,4,10,3,6]);
    }
    #[test]
    fn spectral_history_is_bounded_and_freeze_preserves_it() {
        let(mut app,mut dsp,input)=fixture(Kind::Scope);signal(&input);app.p.source.store(0,Ordering::Relaxed);app.p.values[4].set(1.);
        let mut out=[0.;1024];for _ in 0..50{dsp.process(&mut out,2,48000.);let _=app.slint_extra();}
        assert_eq!(app.terrain.len(),12*32);assert!(app.terrain.iter().any(|x|*x>0.));let before=app.terrain.clone();app.p.values[5].set(1.);for _ in 0..20 {dsp.process(&mut out,2,48000.);let _=app.slint_extra();}assert_eq!(app.terrain,before);
    }
    fn fixture(kind: Kind) -> (CollectionApp, Box<dyn AudioProcessor>, Arc<Mutex<Vec<f32>>>) {
        let bus = Arc::new(AudioBus::new());
        let input = bus.register("Test input");
        let mut app = CollectionApp::new(
            kind,
            bus,
            Arc::new(ModBus::new()),
            Arc::new(MixerBus::new()),
            Arc::new(AtomicF32::new(3.)),
        );
        let dsp = app.audio_processor().unwrap();
        (app, dsp, input)
    }
    fn signal(input: &Arc<Mutex<Vec<f32>>>) {
        *input.lock().unwrap() = (0..512)
            .map(|i| (TAU * i as f32 / 32.).sin() * 0.25)
            .collect();
    }
    fn energy(output: &[f32]) -> f32 {
        output.iter().map(|v| v * v).sum::<f32>() / output.len() as f32
    }
    fn voice_render(voice: usize, motion: f32, sample: bool) -> Vec<f32> {
        let (mut app, mut dsp, input) = fixture(Kind::Swarm);
        signal(&input);
        app.p.source.store(0, Ordering::Relaxed);
        app.p.voice.store(voice, Ordering::Relaxed);
        app.p.pads.store(1, Ordering::Relaxed);
        app.p.motion_depth.set(motion);
        app.p.motion_rate.set(4.);
        if sample {
            app.send(Command::Load(Clip {
                samples: Arc::new(Pcm::from(
                    (0..48000)
                        .map(|i| {
                            let v = (TAU * i as f32 / 87.).sin() * 0.7;
                            [v, v]
                        })
                        .collect::<Vec<_>>(),
                )),
                rate: RATE,
                name: "test instrument".into(),
                peak: 0.7,
            }));
        }
        let mut result = Vec::new();
        let mut out = [0.; 1024];
        for _ in 0..24 {
            dsp.process(&mut out, 2, RATE);
            result.extend_from_slice(&out);
        }
        result
    }
    #[test]
    fn every_performance_row_is_reachable_in_a_bounded_menu_window() {
        for &(kind, _, _) in APPS {
            let (mut app, _, _) = fixture(kind);
            let all = app.rows();
            for i in 0..all.len() {
                app.list.selected = i;
                let (rows, selected, above, below) = app.slint_windowed_rows(10);
                assert!(rows.len() <= 10);
                assert_eq!(rows[selected], all[i]);
                if i == all.len() - 1 {
                    assert!(!below);
                    assert_eq!(above, all.len() > 10);
                }
            }
        }
    }
    #[test]
    fn fracture_pads_choose_different_slices_even_with_zero_scatter() {
        let render = |pad| {
            let (app, mut dsp, input) = fixture(Kind::Fracture);
            app.p.source.store(0, Ordering::Relaxed);
            app.p.play.store(true, Ordering::Relaxed);
            app.p.values[5].set(1.);
            let mut out = [0.; 1024];
            for i in 0..40 {
                *input.lock().unwrap() = vec![i as f32 / 80.; 512];
                dsp.process(&mut out, 2, RATE);
            }
            app.p.pads.store(pad, Ordering::Relaxed);
            dsp.process(&mut out, 2, RATE);
            out
        };
        let a = render(1);
        let b = render(2);
        assert!(energy(&a.iter().zip(b).map(|(a, b)| a - b).collect::<Vec<_>>()) > 1e-4);
    }
    #[test]
    fn take_thumbnail_preserves_signal_at_zero_crossing_boundaries() {
        let samples: Vec<_> = (0..4096)
            .map(|i| {
                let v = (TAU * i as f32 / 32.).sin();
                [v, v]
            })
            .collect();
        assert!(take_overview(&samples, 128).iter().all(|v| *v > 0.9));
        assert_eq!(take_overview(&[], 128), vec![0.; 128]);
    }
    #[test]
    fn voice_bank_has_distinct_audio_and_motion_changes_timbre() {
        let voices: Vec<_> = (0..8).map(|i| voice_render(i, 0., true)).collect();
        for (i, a) in voices.iter().enumerate() {
            assert!(energy(a) > 1e-7, "voice {i} silent");
            for (j, b) in voices.iter().enumerate().take(i) {
                let difference: Vec<_> = a.iter().zip(b).map(|(a, b)| a - b).collect();
                assert!(energy(&difference) > 1e-8, "voices {i} and {j} identical");
            }
        }
        for i in 0..8 {
            let moving = voice_render(i, 1., true);
            let difference: Vec<_> = voices[i].iter().zip(moving).map(|(a, b)| a - b).collect();
            assert!(
                energy(&difference) > 1e-9,
                "voice {i} motion does not affect audio"
            );
        }
        assert_eq!(
            energy(&voice_render(7, 0., false)),
            0.,
            "WAV voice must need a real sample"
        );
    }
    #[test]
    fn scene_recall_changes_sound_controls_without_changing_routes() {
        for &(kind, _, _) in APPS {
            let (mut app, _, _) = fixture(kind);
            app.p.source.store(0, Ordering::Relaxed);
            let before: Vec<_> = app.p.values.iter().map(|v| v.get()).collect();
            app.recall_scene(1);
            let after: Vec<_> = app.p.values.iter().map(|v| v.get()).collect();
            assert_ne!(before, after, "{kind:?} inert scene");
            assert_eq!(app.p.source.load(Ordering::Relaxed), 0);
            for (i, value) in after.iter().enumerate() {
                let (lo, hi, _) = kind.ranges()[i];
                assert!(
                    *value >= lo && *value <= hi,
                    "{kind:?} parameter {i} out of range"
                );
            }
        }
    }
    #[test]
    fn visual_actions_edit_real_trim_root_state_and_tracks() {
        let (mut hunter, _, _) = fixture(Kind::SampleHunter);
        hunter.slint_pointer_pick(-20., 0.25);
        hunter.slint_pointer_pick(-21., 0.75);
        assert_eq!(hunter.p.values[0].get(), 25.);
        assert_eq!(hunter.p.values[1].get(), 75.);
        let (mut practice, _, _) = fixture(Kind::Practice);
        practice.slint_pointer_pick(-20., 0.3);
        practice.slint_pointer_pick(-21., 0.8);
        assert_eq!(practice.p.values[0].get(), 1.);
        assert!((practice.p.values[1].get() - 30.).abs() < 0.001);
        assert_eq!(practice.p.values[2].get(), 80.);
        let (mut chord, _, _) = fixture(Kind::Constellation);
        chord.slint_pointer_pick(-37., 0.);
        assert_eq!(chord.p.values[0].get(), 55.);
        let (mut dream, _, _) = fixture(Kind::Dream);
        dream.slint_pointer_pick(-72., 0.);
        assert_eq!(dream.p.step.load(Ordering::Relaxed), 2);
        let (mut studio, _, _) = fixture(Kind::Studio);
        studio.slint_pointer_pick(-17., 0.);
        assert_eq!(studio.p.values[0].get(), 8.);
        studio.p.alternate.store(true, Ordering::Relaxed);
        let mut input = Input::default();
        input.grid[7] = true;
        input.grid[15] = true;
        studio.tick(&input);
        assert_eq!(studio.tracks[7][2], 1.);
        assert_eq!(studio.tracks[7][3], 1.);
    }
    #[test]
    fn chromatic_audition_is_a_pitched_one_shot_and_stops_within_block() {
        for note in [0, 12] {
            let (mut app, mut dsp, input) = fixture(Kind::SampleHunter);
            signal(&input);
            app.p.source.store(0, Ordering::Relaxed);
            app.p.record.store(true, Ordering::Relaxed);
            let mut out = [0.; 1024];
            for _ in 0..4 {
                dsp.process(&mut out, 2, RATE);
            }
            app.p.record.store(false, Ordering::Relaxed);
            app.slint_pointer_pick(-50. - note as f32, 0.);
            let mut blocks = 0;
            while blocks < 8 {
                dsp.process(&mut out, 2, RATE);
                blocks += 1;
                if !app.p.play.load(Ordering::Relaxed) {
                    break;
                }
            }
            assert_eq!(blocks, if note == 0 { 5 } else { 3 });
            assert!(
                out.iter().all(|v| *v == 0.),
                "one-shot restarted in its ending block"
            );
        }
    }
    #[test]
    fn portal_crossfade_selects_both_real_sources() {
        let (app, mut dsp, input) = fixture(Kind::Portal);
        signal(&input);
        let second = app.bus.register("second");
        *second.lock().unwrap() = vec![0.; 512];
        app.p.source.store(0, Ordering::Relaxed);
        app.p.reference.store(app.bus.len() - 1, Ordering::Relaxed);
        app.p.color.set(0.);
        let mut out = [0.; 1024];
        for _ in 0..8 {
            dsp.process(&mut out, 2, RATE);
        }
        assert!(energy(&app.p.out.lock().unwrap()) > 1e-5);
        app.p.color.set(1.);
        for _ in 0..40 {
            dsp.process(&mut out, 2, RATE);
        }
        assert!(energy(&app.p.out.lock().unwrap()) < 1e-9);
    }
    #[test]
    fn all_apps_are_independent_silent_at_rest_and_finite_at_parameter_extremes() {
        for &(kind, _, _) in APPS {
            let (mut app, mut dsp, input) = fixture(kind);
            app.kit.menu = true; // drives the list with the D-pad below
            let mut out = [0.; 1024];
            for _ in 0..4 {
                dsp.process(&mut out, 2, RATE);
            }
            assert!(out.iter().all(|v| *v == 0.), "{kind:?} idle output");
            signal(&input);
            app.p.source.store(0, Ordering::Relaxed);
            app.p.reference.store(0, Ordering::Relaxed);
            app.p.play.store(true, Ordering::Relaxed);
            app.p.pads.store(65535, Ordering::Relaxed);
            for high in [false, true] {
                for (i, (lo, hi, _)) in kind.ranges().into_iter().enumerate() {
                    app.p.values[i].set(if high { hi } else { lo });
                }
                for _ in 0..16 {
                    dsp.process(&mut out, 2, RATE);
                    assert!(
                        out.iter().all(|v| v.is_finite() && v.abs() <= 1.),
                        "{kind:?} unstable"
                    );
                }
            }
            assert_eq!(app.slint_rows().len(), app.rows().len());
            app.tick(&Input {
                navigation_steps: 1,
                ..Default::default()
            });
            assert_eq!(app.slint_selected(), 1);
        }
    }
    #[test]
    fn instruments_make_audio_from_pads_or_their_own_transport() {
        for kind in [
            Kind::Orbit,
            Kind::Swarm,
            Kind::Mutant,
            Kind::Constellation,
            Kind::Dream,
        ] {
            let (app, mut dsp, _) = fixture(kind);
            app.p.play.store(true, Ordering::Relaxed);
            if kind != Kind::Orbit {
                app.p.pads.store(1, Ordering::Relaxed);
            }
            let mut out = [0.; 1024];
            let mut maximum = 0f32;
            for _ in 0..120 {
                dsp.process(&mut out, 2, RATE);
                maximum = maximum.max(energy(&out));
            }
            assert!(maximum > 0.00001, "{kind:?} produced no sound");
        }
    }
    #[test]
    fn source_selectors_skip_own_output_and_unpatch_cleanly() {
        for kind in [
            Kind::Fracture,
            Kind::Portal,
            Kind::Field,
            Kind::Scope,
            Kind::Master,
        ] {
            let (mut app, _, _) = fixture(kind);
            app.kit.menu = true; // knob 2 on the Source row
            app.tick(&Input {
                knob2: 1,
                ..Default::default()
            });
            assert_eq!(app.p.source.load(Ordering::Relaxed), 0);
            app.tick(&Input {
                knob2: 1,
                ..Default::default()
            });
            assert_eq!(app.p.source.load(Ordering::Relaxed), NO_SOURCE);
        }
    }
    #[test]
    fn effects_require_real_audio_and_portal_does_not_double_monitor() {
        for kind in [
            Kind::Fracture,
            Kind::Ghosts,
            Kind::TapeMachine,
            Kind::Portal,
            Kind::Master,
        ] {
            let (app, mut dsp, input) = fixture(kind);
            app.p.play.store(true, Ordering::Relaxed);
            let mut out = [0.; 1024];
            for _ in 0..5 {
                dsp.process(&mut out, 2, RATE);
            }
            assert_eq!(energy(&out), 0.);
            signal(&input);
            app.p.source.store(0, Ordering::Relaxed);
            for _ in 0..240 {
                dsp.process(&mut out, 2, RATE);
            }
            let bus = app.p.out.lock().unwrap();
            assert!(energy(&bus) > 0.000001, "{kind:?} failed route");
            if kind == Kind::Portal {
                assert_eq!(energy(&out), 0.);
            } else {
                assert!(energy(&out) > 0.000001);
            }
        }
    }
    #[test]
    fn capture_playback_and_bounded_export_preserve_real_samples() {
        for kind in [Kind::Field, Kind::SampleHunter, Kind::Studio] {
            let (mut app, mut dsp, input) = fixture(kind);
            signal(&input);
            app.p.source.store(0, Ordering::Relaxed);
            app.p.record.store(true, Ordering::Relaxed);
            let mut out = [0.; 1024];
            for _ in 0..20 {
                dsp.process(&mut out, 2, RATE);
            }
            app.p.record.store(false, Ordering::Relaxed);
            app.p.play.store(true, Ordering::Relaxed);
            for _ in 0..3 {
                dsp.process(&mut out, 2, RATE);
            }
            assert!(energy(&out) > 0.00001, "{kind:?} playback");
            app.send(Command::Save);
            for _ in 0..30 {
                dsp.process(&mut out, 2, RATE);
            }
            let take = app.takes.try_recv().expect("bounded export completed");
            assert!(take.samples.len() >= 10240);
            assert!(take.samples.iter().any(|s| s[0].abs() > 0.1));
            assert_eq!(take.rate, 48000);
        }
    }
    #[test]
    fn media_playback_has_real_position_repeat_and_end_state() {
        for kind in [
            Kind::Reference,
            Kind::Vinyl,
            Kind::Practice,
            Kind::Radio,
            Kind::Memories,
        ] {
            let (mut app, mut dsp, _) = fixture(kind);
            let samples: Vec<[f32; 2]> = (0..4800)
                .map(|i| {
                    let s = (TAU * i as f32 / 32.).sin() * 0.2;
                    [s, s]
                })
                .collect();
            app.send(Command::Load(Clip {
                samples: Arc::new(Pcm::from(samples)),
                rate: RATE,
                name: "fixture".into(),
                peak: 0.2,
            }));
            app.p.play.store(true, Ordering::Relaxed);
            let mut out = [0.; 1024];
            dsp.process(&mut out, 2, RATE);
            assert!(energy(&out) > 0.0001, "{kind:?}");
            assert!(app.p.position.get() > 0.);
            for _ in 0..20 {
                dsp.process(&mut out, 2, RATE);
            }
            if kind == Kind::Practice {
                assert!(app.p.play.load(Ordering::Relaxed));
            } else {
                assert!(!app.p.play.load(Ordering::Relaxed));
                assert!(app.p.ended.load(Ordering::Relaxed));
            }
        }
    }
    #[test]
    fn scope_measures_without_monitoring_and_freezes_telemetry() {
        let (app, mut dsp, input) = fixture(Kind::Scope);
        signal(&input);
        app.p.source.store(0, Ordering::Relaxed);
        let mut out = [0.; 1024];
        for _ in 0..4 {
            dsp.process(&mut out, 2, RATE);
        }
        assert_eq!(energy(&out), 0.);
        let peak = app.p.view.lock().unwrap().peak;
        assert!((peak - 0.25).abs() < 0.001);
        app.p.values[5].set(1.);
        input.lock().unwrap().fill(0.);
        for _ in 0..4 {
            dsp.process(&mut out, 2, RATE);
        }
        assert_eq!(app.p.view.lock().unwrap().peak, peak);
    }
    #[test]
    fn wav_round_trip_preserves_stereo_and_sample_rate() {
        let mut take = Take {
            samples: vec![[0.25, -0.5]; 128],
            rate: 44100,
            normalize: false,
            marker: 0,
        };
        let path = save_take("test-roundtrip", &mut take).unwrap();
        let clip = load_wav(&path).unwrap();
        assert_eq!(clip.rate, 44100.);
        assert_eq!(clip.samples.len(), 128);
        assert!((clip.samples.at(0)[1] + 0.5).abs() < 1e-4);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod play_view_tests {
    use super::*;
    use kit::rank_pad;

    fn app(kind: Kind) -> CollectionApp {
        let bus = Arc::new(AudioBus::new());
        let input = bus.register("Test input");
        *input.lock().unwrap() = vec![0.1; 512];
        CollectionApp::new(kind, bus, Arc::new(ModBus::new()), Arc::new(MixerBus::new()), Arc::new(AtomicF32::new(3.)))
    }
    fn pad(i: usize) -> Input {
        Input { grid: std::array::from_fn(|k| k == i), ..Default::default() }
    }
    fn musical() -> impl Iterator<Item = Kind> {
        APPS.iter().map(|a| a.0).filter(|k| k.playable())
    }

    #[test]
    fn musical_kinds_open_playable_and_utilities_keep_their_menu() {
        for &(kind, id, _) in APPS {
            let a = app(kind);
            assert_eq!(a.play_column().is_some(), kind.playable(), "{kind:?}");
            assert_eq!(a.play_surface(), kind.playable());
            assert_eq!(a.kit.cfg.app_id, id, "moments saved under the app's own id");
            if kind.playable() {
                let n = a.kit_control_count();
                assert!((7..=16).contains(&n), "{kind:?}");
                for i in 0..n {
                    assert!(!a.kit_label(i).is_empty() && !a.kit_value(i).is_empty());
                }
                let r = a.kit.cfg.routes;
                for c in [r.stick_x, r.stick_y, r.hand_l, r.hand_r].into_iter().flatten() {
                    assert!(!a.kit_stepped(c), "{kind:?} routes a stepped control");
                }
            } else {
                assert_eq!(a.grid_mode_label(), None);
            }
        }
        let mut s = app(Kind::Scope);
        s.tick(&Input { navigation_steps: 1, ..Default::default() });
        assert_eq!(s.slint_selected(), 1, "Scope's D-pad still walks its menu");
    }

    #[test]
    fn knob_1_turns_hero_control_0_and_r1_opens_the_menu() {
        for kind in musical() {
            let mut a = app(kind);
            let before = a.kit_norm(0).unwrap();
            a.tick(&Input { knob1: 1, ..Default::default() });
            assert!(a.kit_norm(0).unwrap() > before, "{kind:?}: knob 1 = {}", a.kit_label(0));
            assert_eq!(a.slint_selected(), 0, "the list didn't move");
            a.tick(&Input { shoulder_press: [false, true], ..Default::default() });
            assert!(a.play_column().is_none(), "{kind:?}: R1 opens the menu");
            a.tick(&Input { navigation_steps: 1, ..Default::default() });
            assert_eq!(a.slint_selected(), 1, "{kind:?}: the menu has the D-pad back");
        }
    }

    #[test]
    fn native_layers_do_what_the_pads_did() {
        // Orbit: a pad toggles its gate.
        let mut orbit = app(Kind::Orbit);
        let gates = orbit.p.pads.load(Ordering::Relaxed);
        orbit.tick(&pad(1));
        orbit.tick(&Input::default());
        assert_eq!(orbit.p.pads.load(Ordering::Relaxed), gates ^ 2);
        // F2 to KEYS: gates set aside, pads play; back to GATES restores them.
        orbit.toggle_grid_mode();
        orbit.tick(&pad(3));
        assert!(orbit.p.alternate.load(Ordering::Relaxed), "KEYS layer = the menu's Keys mode");
        assert_eq!(orbit.p.pads.load(Ordering::Relaxed), 1 << 3);
        for _ in 0..3 {
            orbit.toggle_grid_mode(); // Controls, Moments, Gates
        }
        orbit.tick(&Input::default());
        assert!(!orbit.p.alternate.load(Ordering::Relaxed));
        assert_eq!(orbit.p.pads.load(Ordering::Relaxed), gates ^ 2, "gate pattern survived");

        // Swarm: held pads are held keys; Fracture: held pads punch slices.
        for kind in [Kind::Swarm, Kind::Fracture] {
            let mut a = app(kind);
            a.tick(&pad(5));
            assert_eq!(a.p.pads.load(Ordering::Relaxed), 1 << 5, "{kind:?}");
            a.tick(&Input::default());
            assert_eq!(a.p.pads.load(Ordering::Relaxed), 0);
        }

        // Field: pad 1 records once a source is chosen.
        let mut field = app(Kind::Field);
        field.p.source.store(0, Ordering::Relaxed);
        field.tick(&pad(0));
        assert!(field.p.record.load(Ordering::Relaxed));

        // Studio: the menu's Pads row and F2 stay in step.
        let mut studio = app(Kind::Studio);
        studio.toggle_grid_mode();
        studio.tick(&Input::default());
        assert!(studio.p.alternate.load(Ordering::Relaxed));
        assert_eq!(studio.grid_mode_label(), Some("MUTE/SOLO"));
        studio.tick(&pad(2));
        assert_eq!(studio.tracks[2][2], 1., "mute pad");
        studio.p.alternate.store(false, Ordering::Relaxed);
        studio.tick(&Input::default());
        assert_eq!(studio.grid_mode_label(), Some("RECORD"));
    }

    #[test]
    fn effect_throws_spring_back_exactly() {
        let mut a = app(Kind::Fracture);
        for _ in 0..3 {
            a.toggle_grid_mode();
        }
        assert_eq!(a.grid_mode_label(), Some("THROWS"));
        let wet = a.p.values[5].get();
        a.tick(&pad(rank_pad(0)));
        assert_eq!(a.p.values[5].get(), 1., "WET throw");
        a.tick(&Input::default());
        assert!((a.p.values[5].get() - wet).abs() < 1e-5);
        a.tick(&pad(rank_pad(2)));
        assert!(a.p.freeze.load(Ordering::Relaxed), "FREEZE throw");
        a.tick(&Input::default());
        assert!(!a.p.freeze.load(Ordering::Relaxed));

        let mut tape = app(Kind::TapeMachine);
        let speed = tape.p.values[0].get();
        tape.tick(&Input { stick: [1.0, 0.0], ..Default::default() });
        assert!(tape.p.values[0].get() > speed, "stick X pushes speed");
        tape.tick(&Input::default());
        assert!((tape.p.values[0].get() - speed).abs() < 1e-5, "and returns to the knob");
    }

    #[test]
    fn midi_keys_play_pitched_pads_and_d_pad_browses() {
        let mut a = app(Kind::Swarm);
        let note = a.pad_note(3).unwrap();
        let mut keys = crate::app::MidiKeys::default();
        keys.0[note as usize] = 100;
        a.tick(&Input { midi_keys: keys, ..Default::default() });
        assert_ne!(a.p.pads.load(Ordering::Relaxed) & (1 << 3), 0, "a key presses the pad with its pitch");

        let mut f = app(Kind::Fracture);
        f.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(f.p.variation.load(Ordering::Relaxed), 1, "D-pad up recalls the next scene");
        assert_eq!(f.p.values[1].get(), 2., "Reverse shards' repeats");

        let mut s = app(Kind::Studio);
        s.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(s.p.values[0].get(), 2., "Studio's D-pad steps tracks");
        assert_eq!(s.last_track, 1);
    }

    #[test]
    fn moments_leave_where_you_are_alone() {
        let a = app(Kind::Reference);
        let snap = a.kit_snapshot();
        let at = |c| controls(Kind::Reference).iter().position(|&x| x == c).unwrap();
        assert!(snap[at(Ctl::Param(0))].is_null(), "position is not part of a sound");
        assert!(snap[at(Ctl::File)].is_null() && snap[at(Ctl::Scene)].is_null());
        assert!(snap[at(Ctl::Param(5))].is_number(), "output level is");
    }
}

#[cfg(test)]
mod media_state_regressions {
    use super::*;
    #[test]
    fn journal_sidecars_round_trip_and_clamp_invalid_tags() {
        let path =
            std::env::temp_dir().join(format!("portamax-journal-{}.wav", std::process::id()));
        let sidecar = path.with_extension("journal.toml");
        std::fs::write(&sidecar, "favorite = true\ntag = 3\n").unwrap();
        assert_eq!(load_journal(&path), (true, 3));
        std::fs::write(&sidecar, "favorite = false\ntag = 999\n").unwrap();
        assert_eq!(load_journal(&path), (false, 4));
        std::fs::remove_file(sidecar).unwrap();
    }
    #[test]
    fn studio_overdubs_align_and_export_obeys_pan_and_mute() {
        let bus = Arc::new(AudioBus::new());
        let input = bus.register("fixture");
        *input.lock().unwrap() = vec![0.25; 512];
        let mut app = CollectionApp::new(
            Kind::Studio,
            bus,
            Arc::new(ModBus::new()),
            Arc::new(MixerBus::new()),
            Arc::new(AtomicF32::new(3.)),
        );
        let mut dsp = app.audio_processor().unwrap();
        let mut output = [0.; 1024];
        app.p.source.store(0, Ordering::Relaxed);
        for track in 1..=2 {
            app.p.values[0].set(track as f32);
            app.p.record.store(true, Ordering::Relaxed);
            for _ in 0..8 {
                dsp.process(&mut output, 2, RATE);
            }
            app.p.record.store(false, Ordering::Relaxed);
            dsp.process(&mut output, 2, RATE);
        }
        assert!(
            (app.p.duration.get() - 4096. / RATE).abs() < 0.0001,
            "takes must align at time zero"
        );
        app.send(Command::Track(0, [1., -1., 0., 0.]));
        app.send(Command::Track(1, [1., 1., 1., 0.]));
        app.send(Command::Save);
        for _ in 0..10 {
            dsp.process(&mut output, 2, RATE);
        }
        let take = app.takes.try_recv().unwrap();
        assert_eq!(take.samples.len(), 4096);
        assert!((take.samples[0][0] - 0.25).abs() < 1e-6);
        assert_eq!(take.samples[0][1], 0.);
    }
}

impl Drop for CollectionApp {
    fn drop(&mut self) {
        if let Ok(radio) = self.p.radio.lock() {
            if let Some(stream) = radio.as_ref() {
                stream.cancel.store(true, Ordering::Release);
            }
        }
    }
}

#[cfg(test)] mod visual_interaction_tests {
 use super::*;
 #[test] fn scope_cycles_all_ten_modes_and_freezes_all_geometry(){let bus=Arc::new(AudioBus::new());let input=bus.register("Signal");*input.lock().unwrap()=(0..512).map(|i|(i as f32*0.2).sin()*0.3).collect();let mut app=CollectionApp::new(Kind::Scope,bus,Arc::new(ModBus::new()),Arc::new(MixerBus::new()),Arc::new(AtomicF32::new(1.)));app.p.source.store(0,Ordering::Relaxed);let mut dsp=app.audio_processor().unwrap();let mut out=[0.;1024];for mode in 0..10 {app.p.values[4].set(mode as f32);app.p.values[5].set(0.);for _ in 0..4{dsp.process(&mut out,2,48000.);}let SlintExtra::Collection(before)=app.slint_extra()else{panic!()};app.p.values[5].set(1.);for _ in 0..4{dsp.process(&mut out,2,48000.);}let SlintExtra::Collection(after)=app.slint_extra()else{panic!()};assert_eq!(before.visual_lines,after.visual_lines);assert_eq!(before.terrain,after.terrain);assert!(out.iter().all(|s|*s==0.));}app.p.values[4].set(9.);let mut input=Input::default();input.grid[1]=true;app.tick(&input);assert_eq!(app.p.values[4].get(),0.);}
}

/// Builds one of the Collection apps (or Portal) for the registry: the
/// manifest's id picks the kind, so every Collection app shares this
/// one entry point (`module = "collection"` in its manifest).
pub fn create(ctx: &crate::app::AppContext, id: &str) -> Box<dyn crate::app::App> {
    let kind = APPS.iter().find(|(_, app, _)| *app == id).map(|(k, _, _)| *k).unwrap_or(Kind::Orbit);
    if kind == Kind::Portal {
        Box::new(portal::PortalApp::new(ctx.get(), ctx.get(), ctx.get(), ctx.named("nav_speed")).with_notes(ctx.try_get()))
    } else {
        Box::new(CollectionApp::new(kind, ctx.get(), ctx.get(), ctx.get(), ctx.named("nav_speed")).with_notes(ctx.try_get()))
    }
}
