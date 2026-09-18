//! A multi-track step sequencer, in the spirit of Sugar Bytes'
//! DrumComputer -- not a clone of it. DrumComputer is a full commercial
//! drum-synth workstation (8 three-layer synth engines, full mod matrix,
//! mixer/FX chain, 16 patterns with per-track tempo/direction, rolls,
//! probability, humanize, MIDI export...); matching that 1:1 is a
//! multi-week build. This is the rhythm-machine core of it, built for
//! what this sim actually has: 4 independent tracks, 16 steps each,
//! swing, and per-track choice of instrument --
//!   - Drum: a small built-in synth voice (Kick/Snare/Hat/Clap).
//!   - Plaits: a real Mutable Plaits voice (see plaits_ffi.rs) running
//!     any of its 24 engines -- the same DSP the Plaits app uses, just
//!     triggered from a pattern instead of held notes.
//!   - Sample: a one-shot WAV loaded from the `samples/` folder at the
//!     project root (decoded once at startup via the `hound` crate; new
//!     files need an app/sim restart to be picked up -- no live-reload).
//!
//! Explicitly not included yet: per-step probability/rolls/delay,
//! humanize, pattern banks/chaining, mute/choke groups, a mod matrix,
//! remix/auto-fill, MIDI export. All real DrumComputer features
//! (confirmed against sugar-bytes.de/drumcomputer), all natural
//! follow-ups, being added slowly one at a time. First one in:
//! per-track trigger Probability -- a track-wide chance (not yet
//! per-step) that an active step actually sounds, DrumComputer's
//! "probability sequencer" simplified to one knob for now.
//!
//! Control surface: knob1 browses the menu (Global BPM/Swing, or one of
//! the 4 Track groups -- each track's own leaf set depends on its
//! current Instrument, same idea as Plaits' per-engine parameter
//! labels). knob2 edits the selected leaf; both knobs reset on press.
//! The 4x4 grid always shows/edits the 16 steps of whichever track
//! you're currently browsing in the menu (or last were, while on
//! Global) -- not a fixed track, and not flattened into a strip, same
//! physical-layout mirroring Plaits' piano roll uses.
//!
//! The step clock lives in the audio processor, sample-accurate rather
//! than tied to the UI frame rate -- the same technique Plaits' LFOs
//! and the original single-track version of this app already used.

use crate::app::{App, Input};
use crate::apps::plaits::{ENGINE_NAMES, NUM_ENGINES};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::plaits_ffi::{PlaitsParams, PlaitsVoice};
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NUM_TRACKS: usize = 4;
const NUM_STEPS: usize = 16;
const MIN_BPM: f32 = 40.0;
const MAX_BPM: f32 = 300.0;
const DEFAULT_BPM: f32 = 120.0;
const MAX_SWING: f32 = 0.6; // higher starts to feel broken rather than "groovy"
const NUM_GROUPS: usize = 1 + NUM_TRACKS; // Global, then one per track

/// (label, steps-per-beat). Index 3 ("1/16") is the default, matching
/// this app's original fixed behavior exactly, so existing patterns
/// don't change tempo/feel unless this is touched.
const SUBDIVISIONS: [(&str, f32); 7] = [("1/4", 1.0), ("1/8", 2.0), ("1/8T", 3.0), ("1/16", 4.0), ("1/16T", 6.0), ("1/32", 8.0), ("1/32T", 12.0)];
const DEFAULT_SUBDIVISION: usize = 3;
const MIN_ROLLS: u32 = 1;
const MAX_ROLLS: u32 = 4;
const DEFAULT_ROLLS: u32 = 1;
/// How far a step's own trigger can be nudged, as a fraction of the
/// step's own (post-swing) duration -- +/-40%, so a step can shift
/// noticeably without ever landing on top of its neighbor.
const MAX_STEP_DELAY_FRAC: f32 = 0.4;
/// Global timing/velocity randomization amount, 0..1 -- 1.0 nudges
/// timing by up to +/-15% of a step's duration and velocity by up to
/// +/-20%.
const MAX_HUMANIZE_TIME_FRAC: f32 = 0.15;
const MAX_HUMANIZE_VELOCITY_FRAC: f32 = 0.2;

const INSTRUMENT_NAMES: [&str; 3] = ["Drum", "Plaits", "Sample"];
const DRUM_KIND_NAMES: [&str; 4] = ["Kick", "Snare", "Hat", "Clap"];
const SAMPLES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/samples");

/// Shortens `s` to at most `max` characters (plus a "..." marker when
/// it actually had to cut something) -- needed for sample pack/type/
/// file names, which come straight from real names on disk and,
/// unlike most other strings this app displays, have no length this
/// codebase controls. Character-counted, not byte-counted, so it
/// can't panic slicing into the middle of a multi-byte UTF-8 character.
fn truncate_display(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max).collect::<String>())
    }
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.05).clamp(0.0, 1.0);
    value.set(next);
}

/// A one-shot sample: mono, kept at its native rate once decoded.
/// Playback resamples live with the same linear-interpolation
/// technique PlaitsVoice's own resampler already uses, so no per-load
/// resampling pass is needed.
///
/// Decoding is lazy, not eager: scanning `samples/` to build the name
/// and the pack/type tree only touches the filesystem (`read_dir` +
/// filenames), which is fast even for hundreds of files -- it's
/// *decoding* hundreds of real WAVs up front that was slow enough to
/// add double-digit seconds to every single launch, whether or not a
/// track ever plays that pack. `decoded()` defers the actual decode
/// to the first time a slot is actually needed, and caches it after
/// that -- `OnceLock` makes that safe to call from any thread
/// (including the audio thread, on a track's first trigger) without
/// a decode ever running twice for the same slot.
struct SampleSlot {
    name: String,
    path: std::path::PathBuf,
    decoded: std::sync::OnceLock<(Vec<f32>, f32)>,
}

impl SampleSlot {
    fn new(path: std::path::PathBuf) -> Self {
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string();
        Self { name, path, decoded: std::sync::OnceLock::new() }
    }

    /// (data, native rate), decoding on first call and reusing the
    /// cached result afterward. A file that fails to decode resolves
    /// to an empty, silent slot rather than panicking or retrying
    /// every call.
    fn decoded(&self) -> (&[f32], f32) {
        let (data, rate) = self.decoded.get_or_init(|| match decode_wav(&self.path) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("sequencer: couldn't load {}: {e}", self.path.display());
                (Vec::new(), 44100.0)
            }
        });
        (data.as_slice(), *rate)
    }
}

/// One sample-pack folder directly under `samples/` (e.g. "808 Pack"),
/// holding one or more type subfolders (e.g. "Kick", "Snare") -- the
/// `samples/<pack>/<type>/<file>.wav` layout. A pack's `types` is
/// never empty (packs with none are dropped while scanning).
struct SamplePack {
    name: String,
    types: Vec<SampleType>,
}

/// One type subfolder within a pack. `sample_indices` are positions
/// into the flat `Params::samples` Vec, in the order files were found
/// -- keeps the audio side (which only ever needs "one flat list, one
/// index") completely unaware this tree exists.
struct SampleType {
    name: String,
    sample_indices: Vec<usize>,
}

fn is_wav(path: &Path) -> bool {
    path.is_file() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("wav"))
}

fn sorted_dir_entries(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out: Vec<_> = std::fs::read_dir(dir).map(|e| e.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    out.sort();
    out
}

/// Scans every .wav found under SAMPLES_DIR -- filenames and folder
/// structure only, no decoding (see `SampleSlot`). Builds both the
/// flat list the audio side plays from and the pack/type tree the UI
/// browses, in one pass so their indices stay consistent by
/// construction. Loose .wav files sitting directly in `samples/` or
/// directly in a pack folder (no type subfolder) are grouped under a
/// synthetic "(loose)" type, so the old flat layout still works
/// without requiring the new folder structure.
fn load_samples(root: &Path) -> (Vec<SampleSlot>, Vec<SamplePack>) {
    let mut slots = Vec::new();
    let mut packs = Vec::new();

    let mut load_wavs_into = |paths: Vec<std::path::PathBuf>| -> Vec<usize> {
        let mut indices = Vec::new();
        for path in paths {
            if !is_wav(&path) {
                continue;
            }
            indices.push(slots.len());
            slots.push(SampleSlot::new(path));
        }
        indices
    };

    let root_entries = sorted_dir_entries(root);

    // Loose .wav files directly under samples/ -- a synthetic pack so
    // the pre-existing flat layout keeps working.
    let root_loose = load_wavs_into(root_entries.iter().filter(|p| is_wav(p)).cloned().collect());
    if !root_loose.is_empty() {
        packs.push(SamplePack { name: "(root)".into(), types: vec![SampleType { name: "(loose)".into(), sample_indices: root_loose }] });
    }

    for pack_path in root_entries.iter().filter(|p| p.is_dir()) {
        let pack_name = pack_path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
        let pack_entries = sorted_dir_entries(pack_path);
        let mut types = Vec::new();

        let loose = load_wavs_into(pack_entries.iter().filter(|p| is_wav(p)).cloned().collect());
        if !loose.is_empty() {
            types.push(SampleType { name: "(loose)".into(), sample_indices: loose });
        }

        for type_path in pack_entries.iter().filter(|p| p.is_dir()) {
            let type_name = type_path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
            let indices = load_wavs_into(sorted_dir_entries(type_path));
            if !indices.is_empty() {
                types.push(SampleType { name: type_name, sample_indices: indices });
            }
        }

        if !types.is_empty() {
            packs.push(SamplePack { name: pack_name, types });
        }
    }

    (slots, packs)
}

/// The flat sample index a track's chosen (pack, type, file-position)
/// resolves to, or None if any of the three is out of range (e.g. no
/// packs found at all).
fn resolve_sample(packs: &[SamplePack], pack: usize, ty: usize, file: usize) -> Option<usize> {
    packs.get(pack)?.types.get(ty)?.sample_indices.get(file).copied()
}

fn decode_wav(path: &Path) -> Result<(Vec<f32>, f32), Box<dyn std::error::Error>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let max = (1i64 << spec.bits_per_sample.saturating_sub(1).max(1)).max(1) as f32;
            reader.samples::<i32>().collect::<Result<Vec<i32>, _>>()?.into_iter().map(|s| s as f32 / max).collect()
        }
    };
    let mono = if channels > 1 {
        raw.chunks(channels).map(|c| c.iter().sum::<f32>() / channels as f32).collect()
    } else {
        raw
    };
    Ok((mono, spec.sample_rate as f32))
}

/// Every editable row, always scoped to a track index except the two
/// Global ones.
#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Running,
    Bpm,
    Swing,
    Subdivision,
    Humanize,
    Instrument(usize),
    /// Toggles Grid Edit mode for this track (see `SequencerApp::tick`'s
    /// `grid_edit_track` branch) -- while on, the grid stops toggling
    /// steps on/off and both knobs are repurposed for browsing every
    /// step and setting its note instead.
    GridEdit(usize),
    DrumKind(usize),
    PlaitsEngine(usize),
    PlaitsHarmonics(usize),
    PlaitsTimbre(usize),
    SamplePack(usize),
    SampleType(usize),
    SampleFile(usize),
    Pitch(usize),
    /// Pitch offset (semitones, added on top of the track's own
    /// `Pitch`) for whichever step was most recently touched on the
    /// grid -- see `SequencerApp::last_touched_step`. One shared leaf
    /// rather than 16 per track, so touching a pad and turning knob2
    /// is how you tune that pad, without a 16-entry submenu.
    StepPitch(usize),
    /// How many times the focused step retriggers within its own
    /// duration -- same "last touched pad" convention as StepPitch.
    StepRolls(usize),
    /// How far into its own duration the focused step's hit(s) are
    /// pushed -- same "last touched pad" convention as StepPitch.
    StepDelay(usize),
    Decay(usize),
    Volume(usize),
    Probability(usize),
    Mute(usize),
    /// If any track is soloed, only soloed tracks are audible -- see
    /// `SequencerProcessor::process`'s render loop.
    Solo(usize),
    /// How many of this track's 16 steps play before wrapping back to
    /// 0 -- see `TrackParams::length`'s doc comment for the polymeter
    /// effect a length shorter than another track's creates.
    Length(usize),
    /// Refills this track's whole 16-step pattern with a fresh random
    /// on/off pattern -- press-only action, same "press knob2" idiom
    /// as Bloom/Madness's Randomize.
    RandomizePattern(usize),
    /// Turns every one of this track's 16 steps off -- press-only
    /// action, same idiom as `RandomizePattern`.
    ClearPattern(usize),
}

impl Selection {
    /// The track this leaf belongs to, or None for the Global leaves.
    fn track(&self) -> Option<usize> {
        match *self {
            Selection::Running | Selection::Bpm | Selection::Swing | Selection::Subdivision | Selection::Humanize => None,
            Selection::Instrument(t)
            | Selection::GridEdit(t)
            | Selection::DrumKind(t)
            | Selection::PlaitsEngine(t)
            | Selection::PlaitsHarmonics(t)
            | Selection::PlaitsTimbre(t)
            | Selection::SamplePack(t)
            | Selection::SampleType(t)
            | Selection::SampleFile(t)
            | Selection::Pitch(t)
            | Selection::StepPitch(t)
            | Selection::StepRolls(t)
            | Selection::StepDelay(t)
            | Selection::Decay(t)
            | Selection::Volume(t)
            | Selection::Probability(t)
            | Selection::Mute(t)
            | Selection::Solo(t)
            | Selection::Length(t)
            | Selection::RandomizePattern(t)
            | Selection::ClearPattern(t) => Some(t),
        }
    }
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

struct TrackParams {
    steps: [AtomicBool; NUM_STEPS],
    instrument: AtomicU32,
    drum_kind: AtomicU32,
    plaits_engine: AtomicU32,
    plaits_harmonics: AtomicF32,
    plaits_timbre: AtomicF32,
    /// Which pack/type/file this track's Sample instrument is set to
    /// -- `sample_file` is a position within that (pack, type)'s
    /// list, not a flat index (see `resolve_sample`).
    sample_pack: AtomicUsize,
    sample_type: AtomicUsize,
    sample_file: AtomicUsize,
    pitch: AtomicI32,
    /// Per-step pitch offset (semitones), added on top of `pitch` --
    /// defaults to 0, so an untouched pattern plays at the track's
    /// base pitch exactly as before this existed.
    step_pitch: [AtomicI32; NUM_STEPS],
    /// How many evenly-spaced times this step retriggers within its
    /// own (post-swing) duration -- 1 (default) means just the one
    /// normal hit, matching every pattern that hasn't touched this.
    step_rolls: [AtomicU32; NUM_STEPS],
    /// How far into its own (post-swing) duration this step's hit(s)
    /// are pushed, as a fraction 0..MAX_STEP_DELAY_FRAC -- 0.0
    /// (default) means right on the beat, exactly as before this
    /// existed. Forward-only: nudging a step *earlier* would need
    /// look-ahead scheduling this sim doesn't have.
    step_delay: [AtomicF32; NUM_STEPS],
    decay: AtomicF32,
    volume: AtomicF32,
    /// Track-wide chance (0..1) that an active step actually sounds --
    /// rolled fresh per step, independent of every other track.
    probability: AtomicF32,
    mute: AtomicBool,
    /// If any track is soloed, only soloed tracks reach the output
    /// (mute is ignored while that's true) -- same convention as most
    /// DAWs/grooveboxes. See `SequencerProcessor::process`'s render loop.
    solo: AtomicBool,
    /// How many of this track's 16 steps actually play before
    /// wrapping back to step 0 -- 16 (default) is the whole pattern,
    /// same as every pattern that hasn't touched this. A shorter
    /// length here than another track's creates a polymeter: the two
    /// tracks' loops drift in and out of phase with each other
    /// instead of always realigning every bar, since they wrap at
    /// different points against the *same* shared clock (see
    /// `Params::pulse`). Steps past `length` stay editable (toggling,
    /// Step Pitch, etc. all still work on them) -- only *playback*
    /// skips them, so shortening and later lengthening a track
    /// doesn't lose anything you'd already programmed.
    length: AtomicUsize,
    /// External modulation input (see modbus.rs) -- added on top of
    /// `volume` every block, live regardless of whether this app's own
    /// screen is the one showing, since every app's processor always
    /// runs now.
    ext_volume: Arc<AtomicF32>,
}

impl TrackParams {
    fn new(default_kind: u32, track_num: usize, modbus: &ModBus) -> Self {
        Self {
            steps: std::array::from_fn(|_| AtomicBool::new(false)),
            instrument: AtomicU32::new(0),
            drum_kind: AtomicU32::new(default_kind),
            plaits_engine: AtomicU32::new(0),
            plaits_harmonics: AtomicF32::new(0.5),
            plaits_timbre: AtomicF32::new(0.5),
            sample_pack: AtomicUsize::new(0),
            sample_type: AtomicUsize::new(0),
            sample_file: AtomicUsize::new(0),
            pitch: AtomicI32::new(0),
            step_pitch: std::array::from_fn(|_| AtomicI32::new(0)),
            step_rolls: std::array::from_fn(|_| AtomicU32::new(DEFAULT_ROLLS)),
            step_delay: std::array::from_fn(|_| AtomicF32::new(0.0)),
            decay: AtomicF32::new(0.5),
            volume: AtomicF32::new(0.8),
            probability: AtomicF32::new(1.0),
            mute: AtomicBool::new(false),
            solo: AtomicBool::new(false),
            length: AtomicUsize::new(NUM_STEPS),
            ext_volume: modbus.register(format!("Sequencer: Track {track_num} Volume")),
        }
    }
}

struct Params {
    bpm: AtomicF32,
    swing: AtomicF32,
    subdivision: AtomicUsize,
    /// Global 0..1 amount of subtle per-hit timing/velocity
    /// randomization -- 0.0 (default) changes nothing.
    humanize: AtomicF32,
    /// The transport: false (default -- see the module doc comment's
    /// "every app starts stopped" rule) until explicitly started.
    /// Stopping again pauses the step clock exactly where it is (no
    /// new steps fire) rather than resetting it, and still lets
    /// whatever's already decaying ring out -- see
    /// `SequencerProcessor::process`.
    running: AtomicBool,
    /// The shared clock's raw, monotonically-increasing tick count --
    /// *not* wrapped at 16 anymore (that's what let a single value
    /// mean "the current step" back when every track shared one
    /// length). Each track now derives its own playing step as
    /// `pulse % that track's length` (see `TrackParams::length`), so
    /// two tracks with different lengths drift against this same
    /// shared `pulse` instead of both wrapping in lockstep.
    pulse: AtomicUsize,
    tracks: [TrackParams; NUM_TRACKS],
    /// A one-shot "preview this step's sound now" pulse per track --
    /// set from the UI thread by Grid Edit mode's pad presses (see
    /// `SequencerApp::tick`) and consumed once by the audio thread
    /// (see `SequencerProcessor::process`), independent of the
    /// transport and this step's own on/off/probability/mute gating.
    /// Auditioning a note shouldn't require turning the step -- or
    /// the whole sequencer -- on first.
    audition_pending: [AtomicBool; NUM_TRACKS],
    audition_step: [AtomicUsize; NUM_TRACKS],
    /// Loaded once at construction and never mutated after -- safe to
    /// share with the audio thread as a plain `Vec` (no `Mutex`) since
    /// there's no live-reload.
    samples: Vec<SampleSlot>,
    /// The pack/type tree browsed via `Selection::SamplePack/Type/File`
    /// -- indices into `samples` (see `resolve_sample`), built once
    /// alongside it, same lifetime and sharing rules.
    sample_packs: Vec<SamplePack>,
    /// This app's rendered mono output, republished every block for
    /// another app (Clouds) to tap -- see audio_bus.rs.
    bus_out: Arc<Mutex<Vec<f32>>>,
    /// This app's channel fader in the Mixer app, plus its own
    /// modulation input -- see mixer_bus.rs.
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (samples, sample_packs) = load_samples(Path::new(SAMPLES_DIR));
        let (mix_level, ext_mix_level) = mixer_bus.register("Sequencer", modbus);
        Self {
            bpm: AtomicF32::new(DEFAULT_BPM),
            swing: AtomicF32::new(0.0),
            subdivision: AtomicUsize::new(DEFAULT_SUBDIVISION),
            humanize: AtomicF32::new(0.0),
            running: AtomicBool::new(false),
            pulse: AtomicUsize::new(0),
            tracks: std::array::from_fn(|i| TrackParams::new(i.min(3) as u32, i + 1, modbus)),
            audition_pending: std::array::from_fn(|_| AtomicBool::new(false)),
            audition_step: std::array::from_fn(|_| AtomicUsize::new(0)),
            samples,
            sample_packs,
            bus_out: audio_bus.register("Sequencer"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct SequencerApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    /// Which track the grid edits while browsing Global (whose leaves
    /// aren't track-scoped) -- keeps the grid from going blank.
    last_track: usize,
    /// The step index a grid press most recently touched, per track --
    /// what `Selection::StepPitch` edits (see its doc comment).
    last_touched_step: [usize; NUM_TRACKS],
    prev_grid: [bool; 16],
    /// `Some(track)` while Grid Edit mode is active for that track --
    /// see `Selection::GridEdit` and `tick()`'s early-return branch.
    /// `None` (the default) is the normal menu/grid-toggle behavior.
    grid_edit_track: Option<usize>,
    /// Tick accumulator for scrolling the focused step with knob1
    /// while Grid Edit is active -- same accumulate-until-nav_speed
    /// algorithm `ParamList::navigate` uses, just applied to
    /// `last_touched_step` instead of a menu row.
    grid_edit_accum: i32,
    /// UI-thread-only RNG for `Selection::RandomizePattern` -- doesn't
    /// need to be shared with the audio thread, unlike the per-track
    /// RNGs `TrackVoice` uses for Humanize/probability.
    rng: u32,
}

impl SequencerApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
            last_track: 0,
            last_touched_step: [0; NUM_TRACKS],
            prev_grid: [false; 16],
            grid_edit_track: None,
            grid_edit_accum: 0,
            rng: 0x9E3779B9,
        }
    }

    fn next_rand01(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    /// Refills a track's whole pattern with a fresh random on/off
    /// choice per step -- ~40% density, a reasonable "sounds like a
    /// pattern, not noise" starting point rather than a coin-flip 50%.
    fn randomize_pattern(&mut self, track: usize) {
        const DENSITY: f32 = 0.4;
        for i in 0..NUM_STEPS {
            let on = self.next_rand01() < DENSITY;
            self.params.tracks[track].steps[i].store(on, Ordering::Relaxed);
        }
    }

    fn group_name(&self, g: usize) -> String {
        if g == 0 {
            "Global".into()
        } else {
            format!("Track {g}")
        }
    }

    /// Track groups' leaves depend on that track's current Instrument --
    /// same idea as Plaits relabeling Harmonics/Timbre/Morph per engine,
    /// just changing leaf *count* here instead of just labels.
    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        if g == 0 {
            return vec![Selection::Running, Selection::Bpm, Selection::Swing, Selection::Subdivision, Selection::Humanize];
        }
        let t = g - 1;
        let mut leaves = vec![Selection::Instrument(t), Selection::GridEdit(t)];
        match self.params.tracks[t].instrument.load(Ordering::Relaxed) {
            1 => {
                leaves.push(Selection::PlaitsEngine(t));
                leaves.push(Selection::PlaitsHarmonics(t));
                leaves.push(Selection::PlaitsTimbre(t));
                leaves.push(Selection::Pitch(t));
                leaves.push(Selection::StepPitch(t));
                leaves.push(Selection::StepRolls(t));
                leaves.push(Selection::StepDelay(t));
                leaves.push(Selection::Decay(t));
            }
            2 => {
                leaves.push(Selection::SamplePack(t));
                leaves.push(Selection::SampleType(t));
                leaves.push(Selection::SampleFile(t));
                leaves.push(Selection::Pitch(t));
                leaves.push(Selection::StepPitch(t));
                leaves.push(Selection::StepRolls(t));
                leaves.push(Selection::StepDelay(t));
            }
            _ => {
                leaves.push(Selection::DrumKind(t));
                leaves.push(Selection::Pitch(t));
                leaves.push(Selection::StepPitch(t));
                leaves.push(Selection::StepRolls(t));
                leaves.push(Selection::StepDelay(t));
                leaves.push(Selection::Decay(t));
            }
        }
        leaves.push(Selection::Length(t));
        leaves.push(Selection::Volume(t));
        leaves.push(Selection::Probability(t));
        leaves.push(Selection::Mute(t));
        leaves.push(Selection::Solo(t));
        leaves.push(Selection::RandomizePattern(t));
        leaves.push(Selection::ClearPattern(t));
        leaves
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for g in 0..NUM_GROUPS {
            rows.push(Row::Group(g));
            if self.expanded[g] {
                for sel in self.group_leaves(g) {
                    rows.push(Row::Leaf(sel));
                }
            }
        }
        rows
    }

    fn current_track(&self, rows: &[Row]) -> usize {
        match rows.get(self.list.selected) {
            Some(Row::Group(g)) if *g >= 1 => *g - 1,
            Some(Row::Leaf(sel)) => sel.track().unwrap_or(self.last_track),
            _ => self.last_track,
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Running => "Running".into(),
            Selection::Bpm => "BPM".into(),
            Selection::Swing => "Swing".into(),
            Selection::Subdivision => "Subdivision".into(),
            Selection::Humanize => "Humanize".into(),
            Selection::Instrument(_) => "Instrument".into(),
            Selection::GridEdit(_) => "Grid Edit".into(),
            Selection::DrumKind(_) => "Kind".into(),
            Selection::PlaitsEngine(_) => "Engine".into(),
            Selection::PlaitsHarmonics(_) => "Harmonics".into(),
            Selection::PlaitsTimbre(_) => "Timbre".into(),
            Selection::SamplePack(_) => "Pack".into(),
            Selection::SampleType(_) => "Type".into(),
            Selection::SampleFile(_) => "Sample".into(),
            Selection::Pitch(_) => "Pitch".into(),
            Selection::StepPitch(t) => format!("Step {} Pitch", self.last_touched_step[t] + 1),
            Selection::StepRolls(t) => format!("Step {} Rolls", self.last_touched_step[t] + 1),
            Selection::StepDelay(t) => format!("Step {} Delay", self.last_touched_step[t] + 1),
            Selection::Decay(_) => "Decay".into(),
            Selection::Volume(_) => "Volume".into(),
            Selection::Probability(_) => "Probability".into(),
            Selection::Mute(_) => "Mute".into(),
            Selection::Solo(_) => "Solo".into(),
            Selection::Length(_) => "Length".into(),
            Selection::RandomizePattern(_) => "Randomize Pattern".into(),
            Selection::ClearPattern(_) => "Clear Pattern".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Running => {
                if self.params.running.load(Ordering::Relaxed) { "running".into() } else { "stopped".into() }
            }
            Selection::Bpm => format!("{:.0}", self.params.bpm.get()),
            Selection::Swing => format!("{:.2}", self.params.swing.get()),
            Selection::Subdivision => {
                let idx = self.params.subdivision.load(Ordering::Relaxed) % SUBDIVISIONS.len();
                SUBDIVISIONS[idx].0.to_string()
            }
            Selection::Humanize => format!("{:.0}%", self.params.humanize.get() * 100.0),
            Selection::Instrument(t) => {
                let idx = self.params.tracks[t].instrument.load(Ordering::Relaxed) as usize % INSTRUMENT_NAMES.len();
                INSTRUMENT_NAMES[idx].to_string()
            }
            Selection::GridEdit(t) => {
                if self.grid_edit_track == Some(t) { "ON -- press knob1 to exit".into() } else { "off".into() }
            }
            Selection::DrumKind(t) => {
                let idx = self.params.tracks[t].drum_kind.load(Ordering::Relaxed) as usize % DRUM_KIND_NAMES.len();
                DRUM_KIND_NAMES[idx].to_string()
            }
            Selection::PlaitsEngine(t) => {
                let idx = self.params.tracks[t].plaits_engine.load(Ordering::Relaxed) as usize % ENGINE_NAMES.len();
                ENGINE_NAMES[idx].to_string()
            }
            Selection::PlaitsHarmonics(t) => format!("{:.2}", self.params.tracks[t].plaits_harmonics.get()),
            Selection::PlaitsTimbre(t) => format!("{:.2}", self.params.tracks[t].plaits_timbre.get()),
            Selection::SamplePack(t) => {
                let idx = self.params.tracks[t].sample_pack.load(Ordering::Relaxed);
                match self.params.sample_packs.get(idx) {
                    Some(pack) => truncate_display(&pack.name, 18),
                    None => "(none found)".into(),
                }
            }
            Selection::SampleType(t) => {
                let packs = &self.params.sample_packs;
                let pack_idx = self.params.tracks[t].sample_pack.load(Ordering::Relaxed);
                let type_idx = self.params.tracks[t].sample_type.load(Ordering::Relaxed);
                match packs.get(pack_idx).and_then(|p| p.types.get(type_idx)) {
                    Some(ty) => truncate_display(&ty.name, 18),
                    None => "(none found)".into(),
                }
            }
            Selection::SampleFile(t) => {
                let packs = &self.params.sample_packs;
                let pack_idx = self.params.tracks[t].sample_pack.load(Ordering::Relaxed);
                let type_idx = self.params.tracks[t].sample_type.load(Ordering::Relaxed);
                let file_idx = self.params.tracks[t].sample_file.load(Ordering::Relaxed);
                match resolve_sample(packs, pack_idx, type_idx, file_idx).and_then(|i| self.params.samples.get(i)) {
                    Some(slot) => truncate_display(&slot.name, 18),
                    None => "(none found)".into(),
                }
            }
            Selection::Pitch(t) => format!("{:+}", self.params.tracks[t].pitch.load(Ordering::Relaxed)),
            Selection::StepPitch(t) => {
                format!("{:+}", self.params.tracks[t].step_pitch[self.last_touched_step[t]].load(Ordering::Relaxed))
            }
            Selection::StepRolls(t) => {
                format!("{}x", self.params.tracks[t].step_rolls[self.last_touched_step[t]].load(Ordering::Relaxed))
            }
            Selection::StepDelay(t) => {
                format!("{:.0}%", self.params.tracks[t].step_delay[self.last_touched_step[t]].get() * 100.0)
            }
            Selection::Decay(t) => format!("{:.2}", self.params.tracks[t].decay.get()),
            Selection::Volume(t) => format!("{:.2}", self.params.tracks[t].volume.get()),
            Selection::Probability(t) => format!("{:.0}%", self.params.tracks[t].probability.get() * 100.0),
            Selection::Mute(t) => {
                if self.params.tracks[t].mute.load(Ordering::Relaxed) { "MUTED".into() } else { "on".into() }
            }
            Selection::Solo(t) => {
                if self.params.tracks[t].solo.load(Ordering::Relaxed) { "SOLO".into() } else { "off".into() }
            }
            Selection::Length(t) => format!("{} steps", self.params.tracks[t].length.load(Ordering::Relaxed)),
            Selection::RandomizePattern(_) => "press knob2".into(),
            Selection::ClearPattern(_) => "press knob2".into(),
        }
    }

    fn group_summary(&self, g: usize) -> String {
        if g == 0 {
            return format!("{:.0} BPM", self.params.bpm.get());
        }
        let t = g - 1;
        let inst_idx = self.params.tracks[t].instrument.load(Ordering::Relaxed) as usize % INSTRUMENT_NAMES.len();
        let detail = match inst_idx {
            1 => self.leaf_value(Selection::PlaitsEngine(t)),
            2 => self.leaf_value(Selection::SampleFile(t)),
            _ => self.leaf_value(Selection::DrumKind(t)),
        };
        // Deliberately compact tags (`[S]`/`[M]`, no parens, no
        // "steps") -- this row's text is competing with `grid_x`'s
        // step grid for the same screen width once
        // `paramlist::TEXT_SCALE` is applied, and a real sample/
        // engine/drum-kit name plus every modifier stacked at once
        // was enough to run past it (see `list_text_never_reaches_
        // the_step_grid`).
        let prob = self.params.tracks[t].probability.get();
        let detail = if prob < 0.999 { format!("{detail} {:.0}%", prob * 100.0) } else { detail };
        let detail = if self.params.tracks[t].solo.load(Ordering::Relaxed) {
            format!("{detail} [S]")
        } else if self.params.tracks[t].mute.load(Ordering::Relaxed) {
            format!("{detail} [M]")
        } else {
            detail
        };
        let len = self.params.tracks[t].length.load(Ordering::Relaxed);
        if len != NUM_STEPS { format!("{detail} {len}st") } else { detail }
    }

    /// Decodes whichever sample a track's Pack/Type/File selection
    /// now resolves to, on this (UI) thread rather than the audio
    /// thread -- called right after that selection changes, so
    /// browsing to a sample is usually what pays its one-time decode
    /// cost, not that track's next trigger. Harmless no-op once
    /// cached (see `SampleSlot::decoded`), and if nothing resolves
    /// yet (e.g. no packs found) there's simply nothing to warm.
    fn warm_selected_sample(&self, t: usize) {
        let pack_idx = self.params.tracks[t].sample_pack.load(Ordering::Relaxed);
        let type_idx = self.params.tracks[t].sample_type.load(Ordering::Relaxed);
        let file_idx = self.params.tracks[t].sample_file.load(Ordering::Relaxed);
        if let Some(slot) =
            resolve_sample(&self.params.sample_packs, pack_idx, type_idx, file_idx).and_then(|i| self.params.samples.get(i))
        {
            slot.decoded();
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::Running => self.params.running.store(delta > 0, Ordering::Relaxed),
            Selection::Bpm => {
                let next = (self.params.bpm.get() + accelerate(delta) * sensitivity * 2.0).clamp(MIN_BPM, MAX_BPM);
                self.params.bpm.set(next);
            }
            Selection::Swing => {
                let next = (self.params.swing.get() + accelerate(delta) * sensitivity * 0.05).clamp(0.0, MAX_SWING);
                self.params.swing.set(next);
            }
            Selection::Subdivision => {
                let cur = self.params.subdivision.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(SUBDIVISIONS.len() as i32);
                self.params.subdivision.store(next as usize, Ordering::Relaxed);
            }
            Selection::Humanize => bump(&self.params.humanize, delta, sensitivity),
            Selection::Instrument(t) => {
                let cur = self.params.tracks[t].instrument.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(INSTRUMENT_NAMES.len() as i32);
                self.params.tracks[t].instrument.store(next as u32, Ordering::Relaxed);
            }
            Selection::GridEdit(t) => self.grid_edit_track = if delta > 0 { Some(t) } else { None },
            Selection::DrumKind(t) => {
                let cur = self.params.tracks[t].drum_kind.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(DRUM_KIND_NAMES.len() as i32);
                self.params.tracks[t].drum_kind.store(next as u32, Ordering::Relaxed);
            }
            Selection::PlaitsEngine(t) => {
                let cur = self.params.tracks[t].plaits_engine.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(NUM_ENGINES as i32);
                self.params.tracks[t].plaits_engine.store(next as u32, Ordering::Relaxed);
            }
            Selection::PlaitsHarmonics(t) => bump(&self.params.tracks[t].plaits_harmonics, delta, sensitivity),
            Selection::PlaitsTimbre(t) => bump(&self.params.tracks[t].plaits_timbre, delta, sensitivity),
            Selection::SamplePack(t) => {
                let n = self.params.sample_packs.len();
                if n > 0 {
                    let cur = self.params.tracks[t].sample_pack.load(Ordering::Relaxed) as i32;
                    let next = (cur + step).rem_euclid(n as i32);
                    self.params.tracks[t].sample_pack.store(next as usize, Ordering::Relaxed);
                    // Changing pack can invalidate the current type/file
                    // position (a different pack has different types).
                    self.params.tracks[t].sample_type.store(0, Ordering::Relaxed);
                    self.params.tracks[t].sample_file.store(0, Ordering::Relaxed);
                    self.warm_selected_sample(t);
                }
            }
            Selection::SampleType(t) => {
                let pack_idx = self.params.tracks[t].sample_pack.load(Ordering::Relaxed);
                let n = self.params.sample_packs.get(pack_idx).map(|p| p.types.len()).unwrap_or(0);
                if n > 0 {
                    let cur = self.params.tracks[t].sample_type.load(Ordering::Relaxed) as i32;
                    let next = (cur + step).rem_euclid(n as i32);
                    self.params.tracks[t].sample_type.store(next as usize, Ordering::Relaxed);
                    self.params.tracks[t].sample_file.store(0, Ordering::Relaxed);
                    self.warm_selected_sample(t);
                }
            }
            Selection::SampleFile(t) => {
                let pack_idx = self.params.tracks[t].sample_pack.load(Ordering::Relaxed);
                let type_idx = self.params.tracks[t].sample_type.load(Ordering::Relaxed);
                let n = self
                    .params
                    .sample_packs
                    .get(pack_idx)
                    .and_then(|p| p.types.get(type_idx))
                    .map(|t| t.sample_indices.len())
                    .unwrap_or(0);
                if n > 0 {
                    let cur = self.params.tracks[t].sample_file.load(Ordering::Relaxed) as i32;
                    let next = (cur + step).rem_euclid(n as i32);
                    self.params.tracks[t].sample_file.store(next as usize, Ordering::Relaxed);
                    self.warm_selected_sample(t);
                }
            }
            Selection::Pitch(t) => {
                let cur = self.params.tracks[t].pitch.load(Ordering::Relaxed);
                self.params.tracks[t].pitch.store(cur + step, Ordering::Relaxed);
            }
            Selection::StepPitch(t) => {
                let s = self.last_touched_step[t];
                let cur = self.params.tracks[t].step_pitch[s].load(Ordering::Relaxed);
                self.params.tracks[t].step_pitch[s].store(cur + step, Ordering::Relaxed);
            }
            Selection::StepRolls(t) => {
                let s = self.last_touched_step[t];
                let cur = self.params.tracks[t].step_rolls[s].load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_ROLLS as i32, MAX_ROLLS as i32);
                self.params.tracks[t].step_rolls[s].store(next as u32, Ordering::Relaxed);
            }
            Selection::StepDelay(t) => {
                let s = self.last_touched_step[t];
                let next = (self.params.tracks[t].step_delay[s].get() + accelerate(delta) * sensitivity * 0.05 * MAX_STEP_DELAY_FRAC)
                    .clamp(0.0, MAX_STEP_DELAY_FRAC);
                self.params.tracks[t].step_delay[s].set(next);
            }
            Selection::Decay(t) => bump(&self.params.tracks[t].decay, delta, sensitivity),
            Selection::Volume(t) => bump(&self.params.tracks[t].volume, delta, sensitivity),
            Selection::Probability(t) => bump(&self.params.tracks[t].probability, delta, sensitivity),
            Selection::Mute(t) => self.params.tracks[t].mute.store(delta > 0, Ordering::Relaxed),
            Selection::Solo(t) => self.params.tracks[t].solo.store(delta > 0, Ordering::Relaxed),
            Selection::Length(t) => {
                let cur = self.params.tracks[t].length.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(1, NUM_STEPS as i32);
                self.params.tracks[t].length.store(next as usize, Ordering::Relaxed);
            }
            Selection::RandomizePattern(_) | Selection::ClearPattern(_) => {} // press-only, see `reset`
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Running => {} // no sensible single default -- same convention Bloom/Madness use
            Selection::Bpm => self.params.bpm.set(DEFAULT_BPM),
            Selection::Swing => self.params.swing.set(0.0),
            Selection::Subdivision => self.params.subdivision.store(DEFAULT_SUBDIVISION, Ordering::Relaxed),
            Selection::Humanize => self.params.humanize.set(0.0),
            Selection::PlaitsHarmonics(t) => self.params.tracks[t].plaits_harmonics.set(0.5),
            Selection::PlaitsTimbre(t) => self.params.tracks[t].plaits_timbre.set(0.5),
            Selection::Pitch(t) => self.params.tracks[t].pitch.store(0, Ordering::Relaxed),
            Selection::StepPitch(t) => self.params.tracks[t].step_pitch[self.last_touched_step[t]].store(0, Ordering::Relaxed),
            Selection::StepRolls(t) => self.params.tracks[t].step_rolls[self.last_touched_step[t]].store(DEFAULT_ROLLS, Ordering::Relaxed),
            Selection::StepDelay(t) => self.params.tracks[t].step_delay[self.last_touched_step[t]].set(0.0),
            Selection::Decay(t) => self.params.tracks[t].decay.set(0.5),
            Selection::Volume(t) => self.params.tracks[t].volume.set(0.8),
            Selection::Probability(t) => self.params.tracks[t].probability.set(1.0),
            Selection::Mute(t) => self.params.tracks[t].mute.store(false, Ordering::Relaxed),
            Selection::Solo(t) => self.params.tracks[t].solo.store(false, Ordering::Relaxed),
            Selection::Length(t) => self.params.tracks[t].length.store(NUM_STEPS, Ordering::Relaxed),
            Selection::RandomizePattern(t) => self.randomize_pattern(t),
            Selection::ClearPattern(t) => {
                for i in 0..NUM_STEPS {
                    self.params.tracks[t].steps[i].store(false, Ordering::Relaxed);
                }
            }
            Selection::GridEdit(_) => self.grid_edit_track = None,
            // No sensible single "default" to reset these to.
            Selection::Instrument(_)
            | Selection::DrumKind(_)
            | Selection::PlaitsEngine(_)
            | Selection::SamplePack(_)
            | Selection::SampleType(_)
            | Selection::SampleFile(_) => {}
        }
    }

    /// Moves `last_touched_step[track]` by one step once enough knob1
    /// ticks have accumulated -- the exact same "accumulate until
    /// `nav_speed` ticks, then move one" algorithm `ParamList::navigate`
    /// uses for menu rows, just driving the focused grid step instead.
    fn nudge_focused_step(&mut self, track: usize, delta: i32) {
        if delta == 0 {
            return;
        }
        let ticks_per_step = (self.nav_speed.get() as i32).max(1);
        if (delta > 0 && self.grid_edit_accum < 0) || (delta < 0 && self.grid_edit_accum > 0) {
            self.grid_edit_accum = 0;
        }
        self.grid_edit_accum += delta.signum();
        if self.grid_edit_accum.abs() >= ticks_per_step {
            let cur = self.last_touched_step[track] as i32;
            let next = if self.grid_edit_accum > 0 { cur + 1 } else { cur - 1 };
            self.last_touched_step[track] = next.rem_euclid(NUM_STEPS as i32) as usize;
            self.grid_edit_accum = 0;
        }
    }

    /// Grid Edit mode's entire input handling, replacing the normal
    /// menu-navigation + grid-toggle `tick()` body while active (see
    /// `grid_edit_track`) -- knob1 scrolls which step is focused,
    /// knob2 sets that step's note, a grid press jumps focus straight
    /// to that pad and previews its sound (never toggling it on/off),
    /// and pressing knob1 exits back to the normal menu. This is the
    /// one thing on screen that isn't the usual browse/edit menu, so
    /// it gets its own method rather than being folded into `tick()`.
    fn tick_grid_edit(&mut self, track: usize, input: &Input) {
        self.nudge_focused_step(track, input.knob1);

        if input.knob2 != 0 {
            let s = self.last_touched_step[track];
            let step = input.knob2.signum();
            let cur = self.params.tracks[track].step_pitch[s].load(Ordering::Relaxed);
            self.params.tracks[track].step_pitch[s].store(cur + step, Ordering::Relaxed);
        }
        if input.knob2_press {
            let s = self.last_touched_step[track];
            self.params.tracks[track].step_pitch[s].store(0, Ordering::Relaxed);
        }
        if input.knob1_press {
            self.grid_edit_track = None;
        }

        for i in 0..16 {
            if input.grid[i] && !self.prev_grid[i] {
                self.last_touched_step[track] = i;
                self.params.audition_step[track].store(i, Ordering::Relaxed);
                self.params.audition_pending[track].store(true, Ordering::Relaxed);
            }
        }
        self.prev_grid = input.grid;
    }
}

impl App for SequencerApp {
    /// The transport -- global to the whole app, unlike Bloom/
    /// Madness's per-shape Running, so there's no "which one" to pick.
    fn running(&self) -> Option<bool> {
        Some(self.params.running.load(Ordering::Relaxed))
    }

    fn toggle_running(&mut self) {
        let cur = self.params.running.load(Ordering::Relaxed);
        self.params.running.store(!cur, Ordering::Relaxed);
    }

    /// Mirrors the currently-browsed track's step grid on a physical
    /// controller: green for a step that's programmed on ("activated"
    /// -- same steady color regardless of playhead position), red for
    /// the one step actually sounding right now (the playhead, but
    /// only while it's sitting on an active step *and* the transport
    /// is running -- a frozen "phantom" pad with no sound behind it
    /// would be misleading). Off for everything else, same as the
    /// on-screen grid's own active/playhead treatment (`draw()`).
    fn grid_led_overlay(&self) -> [crate::led_output::PadColor; 16] {
        use crate::led_output::PadColor;

        let rows = self.visible_rows();
        let track = self.current_track(&rows);
        let track_len = self.params.tracks[track].length.load(Ordering::Relaxed).clamp(1, NUM_STEPS);
        let running = self.params.running.load(Ordering::Relaxed);
        let playhead_step = self.params.pulse.load(Ordering::Relaxed) % track_len;

        std::array::from_fn(|i| {
            let active = self.params.tracks[track].steps[i].load(Ordering::Relaxed);
            if !active {
                PadColor::Off
            } else if running && i == playhead_step {
                PadColor::Red
            } else {
                PadColor::Green
            }
        })
    }

    fn tick(&mut self, input: &Input) {
        if let Some(track) = self.grid_edit_track {
            self.tick_grid_edit(track, input);
            return;
        }

        let rows = self.visible_rows();
        self.list.navigate(input.knob1, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();

        if input.knob1_press {
            if let Some(Row::Group(g)) = current {
                self.expanded[g] = !self.expanded[g];
            }
        }
        if let Some(Row::Leaf(sel)) = current {
            self.edit(sel, input.knob2);
            if input.knob2_press {
                self.reset(sel);
            }
        }

        self.last_track = self.current_track(&rows);
        let track = self.last_track;

        // Toggle on a fresh press (rising edge), not while held -- a
        // step is a pattern you set once, not a note you sustain.
        // Touching a pad also focuses it for Step Pitch, regardless of
        // whether this press turned it on or off.
        for i in 0..16 {
            if input.grid[i] && !self.prev_grid[i] {
                self.last_touched_step[track] = i;
                let cur = self.params.tracks[track].steps[i].load(Ordering::Relaxed);
                self.params.tracks[track].steps[i].store(!cur, Ordering::Relaxed);
            }
        }
        self.prev_grid = input.grid;
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(SequencerProcessor {
            params: Arc::clone(&self.params),
            step_timer: 0.0,
            voices: std::array::from_fn(|i| TrackVoice::new(i as u32)),
            mono_buf: Vec::new(),
            pending_fires: std::array::from_fn(|_| Vec::new()),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let title = MonoTextStyle::new(&SPLEEN_16X32, Rgb565::WHITE);
        Text::new("Sequencer", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(0, 63, 10));
        let dim = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));

        // --- Left: the dropdown menu (same shape as Plaits') ---
        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", self.leaf_name(*sel)), self.leaf_value(*sel)),
            })
            .collect();
        self.list.draw(fb, 16, 44, 24, 10, &display_rows);

        // --- Right: the current track's 16 steps, as a 4x4 grid
        // matching the physical pad layout 1:1 (row-major, no flip --
        // unlike Plaits' pitch grid, step order has no "low/high"
        // sense that would call for one). ---
        let track = self.current_track(&rows);
        let header = if self.grid_edit_track == Some(track) {
            format!("Track {} -- GRID EDIT: knob1 pad, knob2 note", track + 1)
        } else {
            format!("Track {} -- grid edits this", track + 1)
        };
        Text::new(&header, Point::new(360, 60), accent).draw(fb).ok();

        let grid_x = 370;
        let grid_y = 78;
        let cell_w = 60;
        let cell_h = 42;
        let gap = 8;
        // Polymeter: this track plays through only its own first
        // `length` steps -- the playhead is that track's own wrapped
        // position against the shared pulse, and steps past `length`
        // are dimmed to show they're programmed-but-not-played rather
        // than looking identical to ones that just haven't fired yet.
        let track_len = self.params.tracks[track].length.load(Ordering::Relaxed).clamp(1, NUM_STEPS);
        let playhead_step = self.params.pulse.load(Ordering::Relaxed) % track_len;

        let focused_step = self.last_touched_step[track];
        for row in 0..4 {
            for col in 0..4 {
                let i = row * 4 + col;
                let active = self.params.tracks[track].steps[i].load(Ordering::Relaxed);
                let trimmed = i >= track_len;
                let is_playhead = i == playhead_step && !trimmed;
                let is_focused = i == focused_step;
                let x = grid_x + col as i32 * (cell_w + gap);
                let y = grid_y + row as i32 * (cell_h + gap);

                let fill = if trimmed {
                    Rgb565::new(1, 2, 1)
                } else if active {
                    Rgb565::new(0, 40, 6)
                } else {
                    Rgb565::new(3, 6, 3)
                };
                Rectangle::new(Point::new(x, y), Size::new(cell_w as u32, cell_h as u32))
                    .into_styled(PrimitiveStyle::with_fill(fill))
                    .draw(fb)
                    .ok();

                // Playhead takes priority (brighter/thicker); a
                // focused-but-not-playing step still gets a dim
                // outline so it's clear which step Step Pitch edits.
                let (border_color, border_weight) = if is_playhead {
                    (Rgb565::new(0, 63, 10), 2)
                } else if is_focused {
                    (Rgb565::new(20, 40, 63), 2)
                } else {
                    (Rgb565::new(10, 20, 10), 1)
                };
                Rectangle::new(Point::new(x, y), Size::new(cell_w as u32, cell_h as u32))
                    .into_styled(PrimitiveStyle::with_stroke(border_color, border_weight))
                    .draw(fb)
                    .ok();

                let trimmed_style = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(8, 12, 8));
                let label_style = if trimmed { trimmed_style } else if active || is_playhead { accent } else { dim };
                let step_pitch = self.params.tracks[track].step_pitch[i].load(Ordering::Relaxed);
                let rolls = self.params.tracks[track].step_rolls[i].load(Ordering::Relaxed);
                let mut label = format!("{}", i + 1);
                if step_pitch != 0 {
                    label.push_str(&format!(" {:+}", step_pitch));
                }
                if rolls > 1 {
                    label.push_str(&format!(" x{rolls}"));
                }
                Text::new(&label, Point::new(x + 6, y + cell_h - 8), label_style).draw(fb).ok();
            }
        }

        let hint = if self.grid_edit_track == Some(track) {
            format!(
                "knob1: scroll pad (Step {})   knob2: set note   pad: jump+preview   press knob1: exit",
                self.last_touched_step[track] + 1
            )
        } else {
            match rows.get(self.list.selected) {
                Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
                Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
                None => String::new(),
            }
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

/// A small built-in synth voice covering Kick/Snare/Hat/Clap -- not
/// Plaits' 3-layer engines, just enough to be a usable drum kit without
/// samples.
struct DrumVoice {
    phase: f32,
    env: f32,
    pitch_env: f32,
    lp: f32, // lowpass state, reused as the basis for Hat's crude highpass
    rng: u32,
}

impl DrumVoice {
    fn new(seed: u32) -> Self {
        Self { phase: 0.0, env: 0.0, pitch_env: 0.0, lp: 0.0, rng: 0x9E3779B9 ^ (seed.wrapping_mul(0x85EBCA6B) | 1) }
    }

    fn next_rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn trigger(&mut self) {
        self.env = 1.0;
        self.pitch_env = 1.0;
        self.phase = 0.0;
    }

    fn render(&mut self, kind: u32, decay: f32, pitch_semitones: i32, sample_rate: f32) -> f32 {
        let pitch_mult = 2f32.powf(pitch_semitones as f32 / 12.0);
        match kind {
            0 => {
                // Kick: sine with a fast downward pitch sweep on top of
                // the amplitude decay.
                let freq = 55.0 * pitch_mult * (1.0 + self.pitch_env * 2.5);
                self.phase = (self.phase + freq / sample_rate).fract();
                self.pitch_env *= (-1.0 / (sample_rate * 0.05)).exp();
                let coef = (-1.0 / (sample_rate * (0.05 + decay * 0.6))).exp();
                let out = (self.phase * std::f32::consts::TAU).sin() * self.env;
                self.env *= coef;
                out
            }
            1 => {
                // Snare: a decaying tone blended with noise.
                let tone_freq = 180.0 * pitch_mult;
                self.phase = (self.phase + tone_freq / sample_rate).fract();
                let noise = self.next_rand();
                let coef = (-1.0 / (sample_rate * (0.03 + decay * 0.4))).exp();
                let out = ((self.phase * std::f32::consts::TAU).sin() * 0.4 + noise * 0.6) * self.env;
                self.env *= coef;
                out
            }
            2 => {
                // Hat: crude highpass (noise minus its own lowpassed
                // copy), short decay.
                let noise = self.next_rand();
                self.lp += (noise - self.lp) * 0.5;
                let hp = noise - self.lp;
                let coef = (-1.0 / (sample_rate * (0.01 + decay * 0.15))).exp();
                let out = hp * self.env;
                self.env *= coef;
                out
            }
            _ => {
                // Clap: a noise burst, decay tuned a bit longer/looser
                // than the hat for a "splashy" feel.
                let noise = self.next_rand();
                let coef = (-1.0 / (sample_rate * (0.04 + decay * 0.3))).exp();
                let out = noise * self.env;
                self.env *= coef;
                out
            }
        }
    }
}

/// One-shot playback with the same linear-interpolation resampling
/// PlaitsVoice's own resampler uses, so a sample's native rate doesn't
/// have to match the device's.
#[derive(Default)]
struct SamplePlayer {
    pos: f32,
    playing: bool,
}

impl SamplePlayer {
    fn trigger(&mut self) {
        self.pos = 0.0;
        self.playing = true;
    }

    fn render(&mut self, data: &[f32], native_rate: f32, device_rate: f32, pitch_semitones: i32) -> f32 {
        if !self.playing || data.len() < 2 {
            return 0.0;
        }
        let ratio = (native_rate / device_rate) * 2f32.powf(pitch_semitones as f32 / 12.0);
        let i = self.pos as usize;
        if i + 1 >= data.len() {
            self.playing = false;
            return 0.0;
        }
        let frac = self.pos - i as f32;
        let sample = data[i] + (data[i + 1] - data[i]) * frac;
        self.pos += ratio.max(0.01);
        sample
    }
}

struct TrackVoice {
    drum: DrumVoice,
    plaits: PlaitsVoice,
    sample: SamplePlayer,
    /// Reused block-to-block for Plaits' batch-oriented `render` --
    /// same allocation-free pattern PlaitsProcessor uses for its own
    /// mix buffers.
    plaits_buf: Vec<f32>,
    /// Separate from DrumVoice's own internal noise RNG -- this one's
    /// rolled once per step to decide whether an active step actually
    /// sounds (see `probability`), so it shouldn't perturb (or be
    /// perturbed by) the drum synthesis noise sequence.
    rng: u32,
    /// The track's Pitch plus that step's own Step Pitch, latched at
    /// the moment a step actually triggers and held until the next
    /// one -- not read live every block, so a later step's Step Pitch
    /// can't bend a still-decaying earlier hit's tail.
    current_pitch: i32,
    /// Per-hit gain multiplier -- 1.0 normally, or Humanize's random
    /// velocity jitter for that particular hit, latched at trigger
    /// time same as `current_pitch`.
    current_gain: f32,
}

impl TrackVoice {
    fn new(seed: u32) -> Self {
        Self {
            drum: DrumVoice::new(seed),
            plaits: PlaitsVoice::new(),
            sample: SamplePlayer::default(),
            plaits_buf: Vec::new(),
            rng: 0x2545F491 ^ (seed.wrapping_mul(0xBF58476D) | 1),
            current_pitch: 0,
            current_gain: 1.0,
        }
    }

    fn next_rand01(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }
}

struct SequencerProcessor {
    params: Arc<Params>,
    step_timer: f32,
    voices: [TrackVoice; NUM_TRACKS],
    mono_buf: Vec<f32>,
    /// Per-track queue of not-yet-fired hits: (seconds remaining,
    /// gain for that hit). Every trigger -- the step's own on-beat
    /// hit and every Roll repeat -- goes through here, even ones
    /// scheduled for "right now" (0 seconds), so Rolls/Step
    /// Delay/Humanize are just different ways of populating the same
    /// queue rather than three separate mechanisms. Firing still only
    /// happens once per block (see `process`), so multiple entries
    /// landing inside one block will collapse to whichever fires
    /// last -- a real but minor precision limit at high BPM/roll
    /// counts, shared with Plaits' existing block-batched triggering.
    pending_fires: [Vec<(f32, f32)>; NUM_TRACKS],
}

impl AudioProcessor for SequencerProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        self.mono_buf.clear();
        self.mono_buf.resize(frames, 0.0);

        let bpm = self.params.bpm.get().max(1.0);
        let steps_per_beat = SUBDIVISIONS[self.params.subdivision.load(Ordering::Relaxed) % SUBDIVISIONS.len()].1;
        let base_step = (60.0 / bpm / steps_per_beat).max(0.001);
        let swing = self.params.swing.get().clamp(0.0, MAX_SWING);
        let dt = frames as f32 / sample_rate;

        // Plaits triggers are block-batched (see TrackVoice::plaits_buf),
        // so a mid-block step just marks "fire on this block's render
        // call" rather than triggering immediately like Drum/Sample do.
        let mut plaits_trigger = [false; NUM_TRACKS];

        // Grid Edit's audition request (see Params::audition_pending):
        // an unconditional, un-gated fire straight into the same
        // pending-fires queue every normal step hit goes through, so
        // it's picked up by the exact same instrument-dispatch code
        // below rather than needing its own copy of it.
        for t in 0..NUM_TRACKS {
            if self.params.audition_pending[t].swap(false, Ordering::Relaxed) {
                let step = self.params.audition_step[t].load(Ordering::Relaxed).min(NUM_STEPS - 1);
                let base_pitch = self.params.tracks[t].pitch.load(Ordering::Relaxed);
                let step_pitch = self.params.tracks[t].step_pitch[step].load(Ordering::Relaxed);
                self.voices[t].current_pitch = base_pitch + step_pitch;
                self.pending_fires[t].push((0.0, 1.0));
            }
        }

        // Stopped: the clock simply doesn't advance at all -- not
        // reset to step 0, not left to "catch up" in one burst on
        // resume, just paused exactly where it was (skipping the
        // whole block below, not just the decrement, matters here: a
        // `step_timer` that was already <= 0 the instant Stop was
        // pressed must not still fire that pending step). Whatever's
        // already in `pending_fires` (and the per-track render loop
        // below) still runs either way, so anything already
        // triggered keeps decaying naturally instead of being cut off.
        if self.params.running.load(Ordering::Relaxed) {
            self.step_timer -= dt;
        }
        while self.params.running.load(Ordering::Relaxed) && self.step_timer <= 0.0 {
            let next = self.params.pulse.load(Ordering::Relaxed).wrapping_add(1);
            self.params.pulse.store(next, Ordering::Relaxed);
            // Swing keyed off the shared pulse itself, not each
            // track's own wrapped step -- it's a feel applied to the
            // clock, the same for every track regardless of that
            // track's Length.
            let swung = if next % 2 == 1 { base_step * (1.0 + swing) } else { base_step * (1.0 - swing) };
            self.step_timer += swung.max(0.001);

            for t in 0..NUM_TRACKS {
                // Polymeter: this track plays through only its own
                // first `length` steps before wrapping, against the
                // *same* shared `next` every other track also
                // advances by -- see `TrackParams::length`.
                let track_len = self.params.tracks[t].length.load(Ordering::Relaxed).clamp(1, NUM_STEPS);
                let step = next % track_len;
                if !self.params.tracks[t].steps[step].load(Ordering::Relaxed) {
                    continue;
                }
                // Rolled once per active step -- at 100% (the
                // default) this always passes, so existing patterns
                // play exactly as before. Probability gates the whole
                // step (including every Roll repeat), not each hit
                // independently -- a ratchet shouldn't get random
                // silent gaps in the middle of it.
                let probability = self.params.tracks[t].probability.get();
                if self.voices[t].next_rand01() >= probability {
                    continue;
                }
                let base_pitch = self.params.tracks[t].pitch.load(Ordering::Relaxed);
                let step_pitch = self.params.tracks[t].step_pitch[step].load(Ordering::Relaxed);
                self.voices[t].current_pitch = base_pitch + step_pitch;

                // Schedule every hit -- the on-beat one and any Roll
                // repeats -- as (time remaining, gain) entries, rather
                // than firing directly. A plain, untouched step
                // (delay 0, 1 roll, no humanize) schedules one entry
                // at 0.0, which fires within this very block below,
                // matching the old immediate-fire behavior exactly.
                let delay_frac = self.params.tracks[t].step_delay[step].get().clamp(0.0, MAX_STEP_DELAY_FRAC);
                let rolls = self.params.tracks[t].step_rolls[step].load(Ordering::Relaxed).clamp(MIN_ROLLS, MAX_ROLLS);
                let humanize = self.params.humanize.get().clamp(0.0, 1.0);
                let roll_interval = swung / rolls as f32;
                for r in 0..rolls {
                    let base_offset = delay_frac * swung + roll_interval * r as f32;
                    let time_jitter = if humanize > 0.0 {
                        (self.voices[t].next_rand01() * 2.0 - 1.0) * MAX_HUMANIZE_TIME_FRAC * humanize * swung
                    } else {
                        0.0
                    };
                    // A negative jitter that would require firing in
                    // the past just fires immediately instead -- this
                    // sim has no look-ahead scheduling to actually
                    // move a hit earlier than "now".
                    let time_remaining = (base_offset + time_jitter).max(0.0);
                    let gain = if humanize > 0.0 {
                        (1.0 + (self.voices[t].next_rand01() * 2.0 - 1.0) * MAX_HUMANIZE_VELOCITY_FRAC * humanize).clamp(0.0, 1.5)
                    } else {
                        1.0
                    };
                    self.pending_fires[t].push((time_remaining, gain));
                }
            }
        }

        // Fire whatever's due. Same block-level granularity as the
        // old immediate trigger (DrumVoice/SamplePlayer start their
        // envelope at the top of whatever block `render` is next
        // called for, regardless of the exact sample offset within
        // it), so an untouched step's single 0.0-second entry above
        // fires here exactly as it used to fire inline.
        for t in 0..NUM_TRACKS {
            let mut i = 0;
            while i < self.pending_fires[t].len() {
                self.pending_fires[t][i].0 -= dt;
                if self.pending_fires[t][i].0 <= 0.0 {
                    self.voices[t].current_gain = self.pending_fires[t][i].1;
                    match self.params.tracks[t].instrument.load(Ordering::Relaxed) {
                        1 => plaits_trigger[t] = true,
                        2 => self.voices[t].sample.trigger(),
                        _ => self.voices[t].drum.trigger(),
                    }
                    self.pending_fires[t].remove(i);
                } else {
                    i += 1;
                }
            }
        }

        // If any track is soloed, only soloed tracks reach the
        // output -- Mute is ignored while that's true, same as most
        // DAWs/grooveboxes (soloing is a deliberate "just this one"
        // override, not something a track's own Mute should be able
        // to fight).
        let any_solo = (0..NUM_TRACKS).any(|t| self.params.tracks[t].solo.load(Ordering::Relaxed));
        for t in 0..NUM_TRACKS {
            let audible = if any_solo { self.params.tracks[t].solo.load(Ordering::Relaxed) } else { !self.params.tracks[t].mute.load(Ordering::Relaxed) };
            if !audible {
                continue;
            }
            let volume =
                (self.params.tracks[t].volume.get() + self.params.tracks[t].ext_volume.get()).clamp(0.0, 1.0) * self.voices[t].current_gain;
            let pitch = self.voices[t].current_pitch;
            let decay = self.params.tracks[t].decay.get();

            match self.params.tracks[t].instrument.load(Ordering::Relaxed) {
                1 => {
                    let engine = self.params.tracks[t].plaits_engine.load(Ordering::Relaxed) as i32;
                    let params = PlaitsParams {
                        engine,
                        note: 60.0 + pitch as f32,
                        harmonics: self.params.tracks[t].plaits_harmonics.get(),
                        timbre: self.params.tracks[t].plaits_timbre.get(),
                        morph: 0.5,
                        decay,
                        lpg_colour: 0.5,
                        trigger: plaits_trigger[t],
                    };
                    let voice = &mut self.voices[t];
                    voice.plaits_buf.clear();
                    voice.plaits_buf.resize(frames, 0.0);
                    voice.plaits.render(&mut voice.plaits_buf, sample_rate, &params);
                    for (m, s) in self.mono_buf.iter_mut().zip(voice.plaits_buf.iter()) {
                        *m += *s * volume;
                    }
                }
                2 => {
                    let pack_idx = self.params.tracks[t].sample_pack.load(Ordering::Relaxed);
                    let type_idx = self.params.tracks[t].sample_type.load(Ordering::Relaxed);
                    let file_idx = self.params.tracks[t].sample_file.load(Ordering::Relaxed);
                    let slot = resolve_sample(&self.params.sample_packs, pack_idx, type_idx, file_idx)
                        .and_then(|i| self.params.samples.get(i));
                    if let Some(slot) = slot {
                        // Decodes on this track's first-ever trigger
                        // of this particular sample, then reuses the
                        // cached result forever after -- see
                        // `SampleSlot::decoded`.
                        let (data, rate) = slot.decoded();
                        let voice = &mut self.voices[t];
                        for m in self.mono_buf.iter_mut() {
                            *m += voice.sample.render(data, rate, sample_rate, pitch) * volume;
                        }
                    }
                }
                _ => {
                    let kind = self.params.tracks[t].drum_kind.load(Ordering::Relaxed);
                    let voice = &mut self.voices[t];
                    for m in self.mono_buf.iter_mut() {
                        *m += voice.drum.render(kind, decay, pitch, sample_rate) * volume;
                    }
                }
            }
        }

        {
            let mut bus_out = self.params.bus_out.lock().unwrap();
            bus_out.clear();
            bus_out.extend_from_slice(&self.mono_buf);
        }

        // The Mixer app's channel fader for this app -- applied only
        // to what reaches the device, not to `bus_out` above (see
        // plaits.rs for the same pattern).
        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        for (frame, sample) in buffer.chunks_mut(channels).zip(self.mono_buf.iter()) {
            for out in frame.iter_mut() {
                *out = *sample * 0.6 * mix_level; // headroom -- up to 4 tracks summed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a trivially small, valid mono WAV file -- just enough
    /// for `decode_wav` to succeed; content doesn't matter for these
    /// tree-shape tests.
    fn write_test_wav(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let spec = hound::WavSpec { channels: 1, sample_rate: 44100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        writer.write_sample(0i16).unwrap();
        writer.finalize().unwrap();
    }

    /// A scratch directory under the OS temp dir, deleted on drop so
    /// a panicking assertion doesn't leave litter behind.
    struct ScratchDir(std::path::PathBuf);
    impl ScratchDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("portamax_seq_test_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The `samples/<pack>/<type>/<file>.wav` layout, plus loose files
    /// at both the root and pack levels, must all resolve into the
    /// expected pack/type tree with consistent flat indices.
    #[test]
    fn load_samples_builds_expected_pack_tree() {
        let scratch = ScratchDir::new("pack_tree");
        let root = &scratch.0;
        write_test_wav(&root.join("loose_root.wav"));
        write_test_wav(&root.join("PackA/Kick/k1.wav"));
        write_test_wav(&root.join("PackA/Kick/k2.wav"));
        write_test_wav(&root.join("PackA/Snare/s1.wav"));
        write_test_wav(&root.join("PackB/loose_in_pack.wav"));

        let (slots, packs) = load_samples(root);

        let pack_names: Vec<&str> = packs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(pack_names, vec!["(root)", "PackA", "PackB"], "unexpected pack ordering/names: {pack_names:?}");

        let root_pack = &packs[0];
        assert_eq!(root_pack.types.len(), 1);
        assert_eq!(root_pack.types[0].name, "(loose)");
        assert_eq!(root_pack.types[0].sample_indices.len(), 1);

        let pack_a = &packs[1];
        let type_names: Vec<&str> = pack_a.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(type_names, vec!["Kick", "Snare"], "unexpected type ordering/names: {type_names:?}");
        assert_eq!(pack_a.types[0].sample_indices.len(), 2, "Kick should have 2 files");
        assert_eq!(pack_a.types[1].sample_indices.len(), 1, "Snare should have 1 file");

        let pack_b = &packs[2];
        assert_eq!(pack_b.types.len(), 1);
        assert_eq!(pack_b.types[0].name, "(loose)", "a file loose in a pack folder (no type subfolder) should land here");

        // Every recorded index must actually resolve to a real slot.
        for pack in &packs {
            for ty in &pack.types {
                for &idx in &ty.sample_indices {
                    assert!(slots.get(idx).is_some(), "index {idx} in {}/{} out of range", pack.name, ty.name);
                }
            }
        }
        assert_eq!(slots.len(), 5, "expected all 5 wav files to be scanned");
    }

    /// Scanning must not decode anything -- that's the whole point of
    /// deferring it (see `SampleSlot`'s doc comment). Decoding must
    /// only happen the first time `decoded()` is actually called, and
    /// the result must then be cached rather than re-decoded.
    #[test]
    fn scanning_does_not_decode_and_decoding_is_cached() {
        let scratch = ScratchDir::new("lazy_decode");
        let root = &scratch.0;
        write_test_wav(&root.join("one.wav"));

        let (slots, _packs) = load_samples(root);
        assert_eq!(slots.len(), 1);
        assert!(slots[0].decoded.get().is_none(), "scanning must not have decoded the file yet");

        let (data_a, rate_a) = slots[0].decoded();
        assert!(slots[0].decoded.get().is_some(), "decoded() must cache its result");
        let (data_b, rate_b) = slots[0].decoded();
        assert_eq!(data_a, data_b, "a second call must return the same cached data, not re-decode");
        assert_eq!(rate_a, rate_b);
    }

    /// `resolve_sample` must map a (pack, type, file) selection to the
    /// right flat slot, and fail closed (None) for anything out of
    /// range rather than panicking -- important since these indices
    /// are plain user-adjustable knob values.
    #[test]
    fn resolve_sample_maps_selection_and_rejects_out_of_range() {
        let scratch = ScratchDir::new("resolve");
        let root = &scratch.0;
        write_test_wav(&root.join("PackA/Kick/k1.wav"));
        write_test_wav(&root.join("PackA/Kick/k2.wav"));
        let (_slots, packs) = load_samples(root);

        assert_eq!(resolve_sample(&packs, 0, 0, 0), Some(0));
        assert_eq!(resolve_sample(&packs, 0, 0, 1), Some(1));
        assert_eq!(resolve_sample(&packs, 0, 0, 2), None, "file position past the end");
        assert_eq!(resolve_sample(&packs, 0, 1, 0), None, "type index past the end");
        assert_eq!(resolve_sample(&packs, 1, 0, 0), None, "pack index past the end");
    }

    fn new_processor() -> (Arc<Params>, SequencerProcessor) {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        // Test-only convenience: the *production* default is stopped
        // (see Params::new's doc comment / the "every app starts
        // stopped" mandate), but almost every test built on this
        // helper exists specifically to exercise playback/trigger
        // behavior, so starting the transport running here saves
        // every one of them repeating the same line. Tests that
        // specifically care about the stopped state (e.g.
        // `stopped_transport_freezes_step_clock`) explicitly store
        // `false` themselves regardless.
        params.running.store(true, Ordering::Relaxed);
        let proc = SequencerProcessor {
            params: Arc::clone(&params),
            step_timer: 0.0,
            voices: std::array::from_fn(|i| TrackVoice::new(i as u32)),
            mono_buf: Vec::new(),
            pending_fires: std::array::from_fn(|_| Vec::new()),
        };
        (params, proc)
    }

    /// Rolls must schedule more than one pending hit for a step, not
    /// just fire once -- the "only one note triggers" shape of bug,
    /// this time for ratchets within a single step.
    #[test]
    fn rolls_schedule_multiple_hits() {
        let (params, mut proc) = new_processor();
        params.tracks[0].steps[1].store(true, Ordering::Relaxed);
        params.tracks[0].step_rolls[1].store(3, Ordering::Relaxed);

        let mut buffer = vec![0.0f32; 512 * 2];
        let mut saw_multiple_pending = false;
        for _ in 0..64 {
            proc.process(&mut buffer, 2, 48000.0);
            if proc.pending_fires[0].len() >= 2 {
                saw_multiple_pending = true;
            }
        }
        assert!(saw_multiple_pending, "3 rolls should leave more than one hit still pending right after the step fires");
    }

    /// A step with no Roll, Step Delay, or Humanize set must still
    /// fire in the very same block it's due -- Rolls/Step Delay/
    /// Humanize sharing one scheduling queue must not add latency to
    /// an untouched pattern.
    #[test]
    fn untouched_step_still_fires_immediately() {
        let (params, mut proc) = new_processor();
        params.tracks[0].steps[1].store(true, Ordering::Relaxed);

        let mut buffer = vec![0.0f32; 512 * 2];
        let mut fired_block = None;
        for block in 0..64 {
            proc.process(&mut buffer, 2, 48000.0);
            if proc.voices[0].drum.env > 0.0 && fired_block.is_none() {
                fired_block = Some(block);
            }
        }
        // Step 1 (index 1) at the default 1/16 subdivision and 120 BPM
        // is due after one 0.125s step -- well within a couple of
        // 512-sample (10.7ms) blocks -- and must never sit pending.
        assert!(matches!(fired_block, Some(b) if b <= 2), "expected an immediate fire within ~2 blocks, got {fired_block:?}");
        assert!(proc.pending_fires[0].is_empty(), "an untouched step must not leave anything queued after firing");
    }

    /// Stopping the transport must freeze the step clock exactly
    /// where it is -- no new steps advance while stopped, and nothing
    /// "catches up" in a burst once resumed.
    #[test]
    fn stopped_transport_freezes_step_clock() {
        let (params, mut proc) = new_processor();
        for i in 0..NUM_STEPS {
            params.tracks[0].steps[i].store(true, Ordering::Relaxed);
        }
        params.running.store(false, Ordering::Relaxed);

        let mut buffer = vec![0.0f32; 512 * 2];
        let frozen_at = params.pulse.load(Ordering::Relaxed);
        for _ in 0..64 {
            proc.process(&mut buffer, 2, 48000.0);
            assert_eq!(params.pulse.load(Ordering::Relaxed), frozen_at, "step clock must not advance while stopped");
        }

        params.running.store(true, Ordering::Relaxed);
        proc.process(&mut buffer, 2, 48000.0);
        // Resuming should continue from where it paused (one step
        // forward at most, at 120 BPM/1-16th over one 512-sample
        // block), not burst through every step it "missed" while
        // stopped. `pulse` is monotonic (no longer wraps at 16), so
        // this is now a plain difference, not a modulo trick.
        let after_resume = params.pulse.load(Ordering::Relaxed);
        let advanced = after_resume - frozen_at;
        assert!(advanced <= 1, "expected at most one step of catch-up on resume, advanced by {advanced}");
    }

    /// Step Delay must push a step's hit later rather than firing it
    /// immediately -- observable as the hit sitting in the pending
    /// queue for at least one block before it fires.
    #[test]
    fn step_delay_pushes_hit_later() {
        let (params, mut proc) = new_processor();
        params.tracks[0].steps[1].store(true, Ordering::Relaxed);
        params.tracks[0].step_delay[1].set(0.3);

        let mut buffer = vec![0.0f32; 512 * 2];
        let mut saw_pending = false;
        for _ in 0..64 {
            proc.process(&mut buffer, 2, 48000.0);
            if !proc.pending_fires[0].is_empty() {
                saw_pending = true;
            }
        }
        assert!(saw_pending, "a delayed step should sit in the pending queue for at least one block before firing");
    }

    /// At Humanize 0 (the default), velocity gain must always stay
    /// exactly 1.0 -- no behavior change for patterns that never touch
    /// this knob.
    #[test]
    fn zero_humanize_keeps_gain_at_one() {
        let (params, mut proc) = new_processor();
        params.tracks[0].steps[1].store(true, Ordering::Relaxed);

        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..64 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        assert_eq!(proc.voices[0].current_gain, 1.0);
    }

    /// At full Humanize, velocity gain must actually vary across
    /// triggers, while always staying within the documented clamp.
    #[test]
    fn full_humanize_varies_gain_within_bounds() {
        let (params, mut proc) = new_processor();
        for i in 0..NUM_STEPS {
            params.tracks[0].steps[i].store(true, Ordering::Relaxed);
        }
        params.humanize.set(1.0);

        let mut buffer = vec![0.0f32; 512 * 2];
        let mut saw_non_default_gain = false;
        for _ in 0..200 {
            proc.process(&mut buffer, 2, 48000.0);
            let g = proc.voices[0].current_gain;
            assert!((0.0..=1.5).contains(&g), "gain out of the documented clamp: {g}");
            if (g - 1.0).abs() > 0.01 {
                saw_non_default_gain = true;
            }
        }
        assert!(saw_non_default_gain, "expected humanize to vary velocity gain across many triggers");
    }

    /// At the default 100% probability, an active step must always
    /// trigger -- existing patterns should play exactly as they did
    /// before this feature existed.
    #[test]
    fn full_probability_always_triggers() {
        let (params, mut proc) = new_processor();
        params.tracks[0].steps[1].store(true, Ordering::Relaxed);

        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..64 {
            proc.process(&mut buffer, 2, 48000.0);
        }

        assert!(proc.voices[0].drum.env > 0.0, "step should have triggered the drum voice at 100% probability");
    }

    /// A step's own Step Pitch must combine with the track's base
    /// Pitch at the moment it triggers.
    #[test]
    fn step_pitch_combines_with_track_pitch_at_trigger() {
        let (params, mut proc) = new_processor();
        params.tracks[0].steps[1].store(true, Ordering::Relaxed);
        params.tracks[0].pitch.store(5, Ordering::Relaxed);
        params.tracks[0].step_pitch[1].store(-2, Ordering::Relaxed);

        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..64 {
            proc.process(&mut buffer, 2, 48000.0);
        }

        assert_eq!(proc.voices[0].current_pitch, 3, "expected base Pitch (5) + Step Pitch (-2) = 3");
    }

    /// A different step (not the one with a custom Step Pitch) must
    /// still trigger at the track's plain base pitch.
    #[test]
    fn untouched_step_uses_base_pitch_only() {
        let (params, mut proc) = new_processor();
        params.tracks[0].steps[1].store(true, Ordering::Relaxed);
        params.tracks[0].pitch.store(7, Ordering::Relaxed);
        // Step Pitch for step 1 left at its 0 default.

        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..64 {
            proc.process(&mut buffer, 2, 48000.0);
        }

        assert_eq!(proc.voices[0].current_pitch, 7);
    }

    /// A finer subdivision (more steps per beat) must advance through
    /// the pattern faster at the same BPM -- this is what makes
    /// triplet feels (1/8T, 1/16T, ...) actually differ from straight
    /// subdivisions rather than just being alternate labels.
    #[test]
    fn finer_subdivision_advances_steps_faster() {
        let (params, mut proc) = new_processor();
        params.subdivision.store(0, Ordering::Relaxed); // "1/4": 1 step/beat

        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..64 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        let coarse_step = params.pulse.load(Ordering::Relaxed);

        let (params2, mut proc2) = new_processor();
        params2.subdivision.store(6, Ordering::Relaxed); // "1/32T": 12 steps/beat
        for _ in 0..64 {
            proc2.process(&mut buffer, 2, 48000.0);
        }
        let fine_step = params2.pulse.load(Ordering::Relaxed);

        assert_ne!(coarse_step, fine_step, "different subdivisions should reach different steps in the same real time");
    }

    /// At 0% probability, an active step must never trigger.
    #[test]
    fn zero_probability_never_triggers() {
        let (params, mut proc) = new_processor();
        params.tracks[0].steps[1].store(true, Ordering::Relaxed);
        params.tracks[0].probability.set(0.0);

        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..64 {
            proc.process(&mut buffer, 2, 48000.0);
        }

        assert_eq!(proc.voices[0].drum.env, 0.0, "step must not trigger at 0% probability");
    }

    fn new_app() -> SequencerApp {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(1.0)); // 1 tick/step -- deterministic for these tests
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        SequencerApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus)
    }

    fn grid_press(pad: usize) -> Input {
        let mut grid = [false; 16];
        grid[pad] = true;
        Input { grid, ..Default::default() }
    }

    /// Regression test for "selecting a pad turns the sound on and
    /// off" -- entering Grid Edit mode for a track and then pressing
    /// a pad must focus/preview it without ever touching that step's
    /// on/off state.
    #[test]
    fn grid_edit_pad_press_focuses_and_previews_without_toggling() {
        let mut app = new_app();
        app.edit(Selection::GridEdit(0), 1); // turn on for track 0
        assert_eq!(app.grid_edit_track, Some(0));

        app.tick(&grid_press(5));
        assert!(!app.params.tracks[0].steps[5].load(Ordering::Relaxed), "Grid Edit's pad press must not toggle the step on");
        assert_eq!(app.last_touched_step[0], 5, "the pad press should still move focus");
        assert!(app.params.audition_pending[0].load(Ordering::Relaxed), "a pad press in Grid Edit mode should request an audition preview");
        assert_eq!(app.params.audition_step[0].load(Ordering::Relaxed), 5);

        // Pressing the same pad again must not toggle it off either.
        app.tick(&Input::default()); // release
        app.tick(&grid_press(5));
        assert!(!app.params.tracks[0].steps[5].load(Ordering::Relaxed), "a second press must still not toggle the step");
    }

    /// Turning knob1 while in Grid Edit mode must scroll the focused
    /// step (wrapping), not navigate the app's menu -- this is the
    /// "scroll through the pads with a knob" the mode exists for.
    #[test]
    fn grid_edit_knob1_scrolls_focused_step_and_wraps() {
        let mut app = new_app();
        app.edit(Selection::GridEdit(0), 1);
        assert_eq!(app.last_touched_step[0], 0);

        app.tick(&Input { knob1: 1, ..Default::default() });
        assert_eq!(app.last_touched_step[0], 1);

        app.tick(&Input { knob1: -1, ..Default::default() });
        assert_eq!(app.last_touched_step[0], 0);

        app.tick(&Input { knob1: -1, ..Default::default() });
        assert_eq!(app.last_touched_step[0], NUM_STEPS - 1, "scrolling back from 0 should wrap to the last step");
    }

    /// Turning knob2 while in Grid Edit mode must change the focused
    /// step's note (Step Pitch) directly -- no need to navigate to a
    /// specific menu leaf first.
    #[test]
    fn grid_edit_knob2_changes_focused_step_note() {
        let mut app = new_app();
        app.edit(Selection::GridEdit(1), 1);
        app.tick(&grid_press(3)); // focus step 3 on track 1

        app.tick(&Input { knob2: 1, ..Default::default() });
        app.tick(&Input { knob2: 1, ..Default::default() });
        app.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(app.params.tracks[1].step_pitch[3].load(Ordering::Relaxed), 3);

        // A different, unfocused step must be untouched.
        assert_eq!(app.params.tracks[1].step_pitch[0].load(Ordering::Relaxed), 0);
    }

    /// Pressing knob1 while in Grid Edit mode must exit back to
    /// normal menu/grid behavior -- a subsequent pad press should
    /// toggle on/off again, exactly like before entering the mode.
    #[test]
    fn grid_edit_knob1_press_exits_and_restores_normal_toggling() {
        let mut app = new_app();
        app.edit(Selection::GridEdit(2), 1);
        assert_eq!(app.grid_edit_track, Some(2));

        app.tick(&Input { knob1_press: true, ..Default::default() });
        assert_eq!(app.grid_edit_track, None, "knob1 press should exit Grid Edit mode");

        // Normal navigation runs again, landing back wherever it was;
        // what matters here is that a grid press now toggles on/off.
        app.tick(&grid_press(4));
        assert!(app.params.tracks[app.last_track].steps[4].load(Ordering::Relaxed), "after exiting, a pad press should toggle the step on again");
    }

    /// An audition request must fire regardless of the step's own
    /// on/off state and regardless of whether the transport is
    /// running -- auditioning a note is meant to work even on a step
    /// that's off, or while the sequencer is stopped.
    #[test]
    fn audition_fires_regardless_of_step_state_or_transport() {
        let (params, mut proc) = new_processor();
        params.running.store(false, Ordering::Relaxed);
        params.tracks[0].steps[7].store(false, Ordering::Relaxed); // deliberately off
        params.tracks[0].pitch.store(4, Ordering::Relaxed);
        params.tracks[0].step_pitch[7].store(-1, Ordering::Relaxed);
        params.audition_step[0].store(7, Ordering::Relaxed);
        params.audition_pending[0].store(true, Ordering::Relaxed);

        let mut buffer = vec![0.0f32; 512 * 2];
        proc.process(&mut buffer, 2, 48000.0);

        assert_eq!(proc.voices[0].current_pitch, 3, "expected base Pitch (4) + Step 7's Step Pitch (-1) = 3");
        assert!(!params.audition_pending[0].load(Ordering::Relaxed), "the one-shot audition pulse should be consumed after firing");
    }

    /// Two tracks with different Lengths must each wrap at their own
    /// length against the *same* shared clock (`Params::pulse`), not
    /// share one global step index -- that's the whole polymeter
    /// mechanism (see `TrackParams::length`). Verified by checking
    /// each track's `current_pitch` matches the step its own
    /// `pulse % length` formula predicts, independently.
    #[test]
    fn polymeter_tracks_wrap_at_their_own_length_against_shared_pulse() {
        let (params, mut proc) = new_processor();
        params.tracks[0].length.store(2, Ordering::Relaxed);
        params.tracks[0].steps[0].store(true, Ordering::Relaxed);
        params.tracks[0].steps[1].store(true, Ordering::Relaxed);
        params.tracks[0].step_pitch[0].store(10, Ordering::Relaxed);
        params.tracks[0].step_pitch[1].store(20, Ordering::Relaxed);

        params.tracks[1].length.store(4, Ordering::Relaxed);
        for i in 0..4 {
            params.tracks[1].steps[i].store(true, Ordering::Relaxed);
            params.tracks[1].step_pitch[i].store((i as i32 + 1) * 10, Ordering::Relaxed);
        }

        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..64 {
            proc.process(&mut buffer, 2, 48000.0);
        }

        let pulse = params.pulse.load(Ordering::Relaxed);
        assert!(pulse > 0, "clock should have advanced");
        let expected0 = (pulse % 2) as i32 * 10 + 10;
        let expected1 = (pulse % 4) as i32 * 10 + 10;
        assert_eq!(proc.voices[0].current_pitch, expected0, "track 0 (length 2) should be playing step {}", pulse % 2);
        assert_eq!(proc.voices[1].current_pitch, expected1, "track 1 (length 4) should be playing step {}, independently of track 0", pulse % 4);
    }

    /// Soloing a track must silence every other track regardless of
    /// their own Mute state, and un-soloing must restore normal
    /// mute-only gating.
    #[test]
    fn solo_silences_other_tracks_overriding_their_mute_state() {
        // Soloing track 0 should produce *exactly* the same output as
        // explicitly muting tracks 1-3 (with nothing soloed) -- both
        // are "only track 0 reaches the output," just via different
        // controls, and with probability/humanize at their
        // deterministic defaults there's no randomness to blur the
        // comparison. A naive "less energy than the 4-track total"
        // check doesn't actually prove this: track 0's own drum kit
        // might just be louder than the other 3 combined, which would
        // make a same-buffer or "under half" comparison pass or fail
        // for the wrong reason either way.
        let run = |solo_track0: bool, mute_others: bool| {
            let (params, mut proc) = new_processor();
            // Step 1, not 0 -- the shared clock's first tick lands on
            // index 1 (pulse starts at 0 and increments before its
            // first check), same convention every other test here uses.
            for t in 0..NUM_TRACKS {
                params.tracks[t].steps[1].store(true, Ordering::Relaxed);
            }
            params.tracks[0].solo.store(solo_track0, Ordering::Relaxed);
            if mute_others {
                for t in 1..NUM_TRACKS {
                    params.tracks[t].mute.store(true, Ordering::Relaxed);
                }
            }

            let mut buffer = vec![0.0f32; 512 * 2];
            let mut total_energy = 0.0f32;
            for _ in 0..64 {
                proc.process(&mut buffer, 2, 48000.0);
                total_energy += buffer.iter().map(|v| v * v).sum::<f32>();
            }
            total_energy
        };

        let energy_all = run(false, false);
        let energy_track0_muted_others = run(false, true);
        let energy_track0_soloed = run(true, false);

        assert!(energy_track0_muted_others > 0.0, "track 0 alone should still produce output");
        assert!(energy_all > energy_track0_muted_others, "sanity: all 4 tracks should produce more energy than just track 0");
        assert!(
            (energy_track0_soloed - energy_track0_muted_others).abs() < energy_track0_muted_others * 1e-4,
            "soloing track 0 should produce the same output as muting tracks 1-3: solo={energy_track0_soloed}, muted-others={energy_track0_muted_others}"
        );
    }

    /// Randomize Pattern must actually change the step pattern (not a
    /// no-op), and Clear Pattern must turn every step off -- including
    /// undoing whatever Randomize just did.
    #[test]
    fn randomize_and_clear_pattern_actions() {
        let mut app = new_app();
        let before: Vec<bool> = (0..NUM_STEPS).map(|i| app.params.tracks[0].steps[i].load(Ordering::Relaxed)).collect();
        assert!(before.iter().all(|&b| !b), "a fresh track should start with every step off");

        app.reset(Selection::RandomizePattern(0));
        let after_randomize: Vec<bool> = (0..NUM_STEPS).map(|i| app.params.tracks[0].steps[i].load(Ordering::Relaxed)).collect();
        assert!(after_randomize.iter().any(|&b| b), "Randomize Pattern should turn at least one step on");

        app.reset(Selection::ClearPattern(0));
        let after_clear: Vec<bool> = (0..NUM_STEPS).map(|i| app.params.tracks[0].steps[i].load(Ordering::Relaxed)).collect();
        assert!(after_clear.iter().all(|&b| !b), "Clear Pattern should turn every step off, including ones Randomize just set");
    }

    /// Turning Length's knob must clamp to 1..=16 and never go out of
    /// range in either direction.
    #[test]
    fn length_clamps_to_valid_range() {
        let mut app = new_app();
        for _ in 0..20 {
            app.edit(Selection::Length(0), -1);
        }
        assert_eq!(app.params.tracks[0].length.load(Ordering::Relaxed), 1, "should clamp at the minimum, not wrap or go negative");

        for _ in 0..40 {
            app.edit(Selection::Length(0), 1);
        }
        assert_eq!(app.params.tracks[0].length.load(Ordering::Relaxed), NUM_STEPS, "should clamp at the maximum (16)");
    }

    /// The freshly-constructed, untouched `Params` -- the exact state
    /// a brand new app instance starts in -- must default to
    /// stopped. With N apps all loaded (and someday possibly far
    /// more), an app that defaults to *running* is a real risk on
    /// its own, not just an inconvenience.
    #[test]
    fn defaults_to_stopped() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Params::new(&modbus, &audio_bus, &mixer_bus);
        assert!(!params.running.load(Ordering::Relaxed), "Sequencer must start stopped, not running");
    }

    /// The list column's text (at `paramlist::TEXT_SCALE`) must never
    /// reach `grid_x` (370), where the step grid starts -- sample pack
    /// names come straight from real folders on disk (see
    /// `load_samples`), so unlike most of this app's other strings,
    /// nothing here bounds their length. Uses the real, currently-
    /// checked-out `samples/` library (same as `load_samples_builds_
    /// expected_pack_tree`) plus every group expanded, so this checks
    /// the whole column against its real worst case, not a guess.
    #[test]
    fn list_text_never_reaches_the_step_grid() {
        let mut app = new_app();
        app.expanded = [true; NUM_GROUPS];
        for t in 0..NUM_TRACKS {
            app.params.tracks[t].probability.set(0.5); // shows "(50%)" in the group summary
            app.params.tracks[t].solo.store(true, Ordering::Relaxed); // "(solo)"
            app.params.tracks[t].length.store(NUM_STEPS - 1, Ordering::Relaxed); // "[N steps]"
        }

        let rows = app.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if app.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", app.group_name(*g)), app.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", app.leaf_name(*sel)), app.leaf_value(*sel)),
            })
            .collect();

        let mut list = ParamList::new();
        let mut fb = FrameBuffer::new();
        list.draw(&mut fb, 16, 44, 24, rows.len(), &display_rows);
        let mut max_x = 0i32;
        for (i, &p) in fb.buffer().iter().enumerate() {
            if p != 0 {
                max_x = max_x.max((i % crate::display::WIDTH) as i32);
            }
        }
        const GRID_X: i32 = 370;
        assert!(max_x < GRID_X, "list text reached x={max_x}, at or past grid_x ({GRID_X})");
    }

    /// Feeds a physical controller's LEDs (see `App::grid_led_overlay`
    /// and `Os::update_leds`): a programmed ("activated") step is
    /// green whether or not the transport is running; the one step
    /// actually sounding right now (the playhead, on an active step,
    /// while running) is red instead; an unprogrammed step is off
    /// even if the playhead happens to be sitting on it, and even
    /// while stopped nothing shows red (a lit red pad implies a beat
    /// that isn't actually playing).
    #[test]
    fn grid_led_overlay_shows_green_for_activated_steps_and_red_for_the_one_actually_playing() {
        use crate::led_output::PadColor;

        let mut app = new_app();
        app.params.tracks[0].steps[3].store(true, Ordering::Relaxed);
        app.params.tracks[0].steps[5].store(true, Ordering::Relaxed);
        app.params.pulse.store(5, Ordering::Relaxed);

        let stopped = app.grid_led_overlay();
        assert_eq!(stopped[3], PadColor::Green, "an activated step must be green even while stopped");
        assert_eq!(stopped[5], PadColor::Green, "the playhead's step must stay green (not red) while stopped -- nothing is actually playing");
        assert_eq!(stopped[0], PadColor::Off, "an unprogrammed step must be off");

        app.toggle_running();
        let running = app.grid_led_overlay();
        assert_eq!(running[3], PadColor::Green, "an activated step not under the playhead stays green");
        assert_eq!(running[5], PadColor::Red, "pulse=5 landing on activated step 5 must turn it red while running");
        assert_eq!(running[0], PadColor::Off, "an unprogrammed step under no playhead stays off");
    }
}
