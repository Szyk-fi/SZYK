//! Session: where a track comes together. Eight tracks, each playing any
//! instrument app (Plaits, Voltage, Atlas, Hum's notes... anything that
//! takes notes) or Session's own sounds, and a drum track on the built-in
//! kit. Each track has a clip in each of eight scenes; launch clips one
//! at a time or a whole scene at once, chain scenes into a song, record
//! from the pads or a MIDI keyboard, and save the whole thing as a
//! project.
//!
//! Five views, switched with F2:
//! - LAUNCH: the 8 x 8 clip grid. Pads 1-8 launch (or stop) each track's
//!   clip in the selected scene; pads 9-16 launch scenes 1-8. D-pad moves
//!   the selection. Launches wait for the next bar, so they land in time.
//! - STEP: the selected clip's steps, 16 to a page. Tap a pad to set or
//!   clear a step; hold one and use the D-pad to edit it (up/down picks
//!   note, velocity, length, chance or lock; left/right changes it).
//!   Left/right alone turns the page; up/down picks the track.
//! - PLAY: the pads are a keyboard (a drum kit on drum tracks) playing
//!   the selected track. SELECT records into the selected clip, rounded
//!   to the nearest step; hold SELECT to clear the clip.
//! - SONG: the arrangement, a list of scenes and how many bars each
//!   plays. Pads 1-8 add a scene, pad 16 removes the last; up/down picks
//!   a part, left/right changes its length; SELECT turns song mode on.
//! - SETUP: per-track routing (Plays), mute, octave and the lock lane's
//!   destination; tempo, swing; save and load projects (8 slots).
//!
//! Each track has a lock lane: a value per step sent to any modulation
//! input of any app -- a filter cutoff, a Plaits harmonic -- which is how
//! a step changes a sound, Elektron-style, across the whole device.
//!
//! The sequencer runs on the audio thread, on the device clock
//! (clock.rs; sample-accurate, with swing), so Skins, the Looper and MIDI
//! clock out play in time with it -- SETUP's Clock row switches Session to
//! its own tempo instead. Notes reach other apps through the note bus,
//! with timed note-offs.

use crate::app::{App, Input, SlintExtra};
use crate::apps::kids_kit::{self as kit, Drum, Ev, Extra, Note, Rng, Size2, Song, Sound, Tone};
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::modbus::{ModBus, Patch};
use crate::note_bus::{NoteBus, NoteOut, NoteRoute, INTERNAL};
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Session";
pub const TRACKS: usize = 8;
pub const SCENES: usize = 8;
pub const MAX_STEPS: usize = 64;
const DRUMS: [Drum; 11] = [Drum::Kick, Drum::Snare, Drum::Hat, Drum::OpenHat, Drum::Clap, Drum::Rim, Drum::Cowbell, Drum::Woodblock, Drum::HiTom, Drum::LowTom, Drum::Shaker];
/// Drum tracks send notes from here (General MIDI's kick) when they play
/// another app, so a drum machine there lines up.
const DRUM_BASE: u8 = 36;
/// Session's own sound for each track when "Plays" is "Own sound".
const TONES: [Tone; TRACKS] = [Tone::Bass, Tone::Bass, Tone::Soft, Tone::Pluck, Tone::Marimba, Tone::Organ, Tone::Flute, Tone::Chip];
const TRACK_COLORS: [(u8, u8, u8); TRACKS] = [(240, 90, 80), (250, 170, 60), (240, 220, 80), (130, 220, 110), (80, 210, 200), (90, 150, 240), (160, 120, 240), (230, 120, 200)];

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct Step {
    /// Empty: no note. Up to four (a chord).
    pub notes: Vec<u8>,
    pub vel: u8,
    /// In 16th notes.
    pub len: u8,
    /// Percent.
    pub chance: u8,
    /// A value for the track's lock lane, 0..1.
    pub lock: Option<f32>,
}

impl Default for Step {
    fn default() -> Self {
        Step { notes: Vec::new(), vel: 100, len: 1, chance: 100, lock: None }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct Clip {
    pub length: usize,
    pub steps: Vec<Step>,
}

impl Default for Clip {
    fn default() -> Self {
        Clip { length: 16, steps: vec![Step::default(); MAX_STEPS] }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum Kind {
    Notes,
    Drums,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Track {
    pub kind: Kind,
    pub clips: Vec<Option<Clip>>,
    pub mute: bool,
    pub octave: i8,
    /// The instrument it plays, by name ("" = Session's own sound).
    pub plays: String,
    /// The lock lane's destination, a modulation input by name.
    pub lock_to: String,
}

impl Default for Track {
    fn default() -> Self {
        Track { kind: Kind::Notes, clips: vec![None; SCENES], mute: false, octave: 0, plays: String::new(), lock_to: String::new() }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub struct Part {
    pub scene: usize,
    pub bars: u32,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Project {
    pub tempo: f32,
    pub swing: f32,
    pub tracks: Vec<Track>,
    pub song: Vec<Part>,
}

impl Default for Project {
    fn default() -> Self {
        Project { tempo: 110.0, swing: 0.0, tracks: (0..TRACKS).map(|_| Track::default()).collect(), song: Vec::new() }
    }
}

fn steps(pattern: &str, note: u8, vel: u8) -> Vec<Step> {
    let mut v = vec![Step::default(); MAX_STEPS];
    for (i, c) in pattern.chars().enumerate() {
        if c != '.' {
            v[i] = Step { notes: vec![note], vel: if c == 'X' { 120 } else { vel }, ..Step::default() };
        }
    }
    v
}

/// The project a new Session starts with: a beat, a bass line and some
/// chords in two scenes, so pressing play makes music at once.
pub fn demo() -> Project {
    let mut p = Project::default();
    p.tracks[0].kind = Kind::Drums;
    let mut drums = Clip::default();
    for (pat, d) in [("X.......X.x.....", 0u8), ("....x.......x...", 1), ("x.x.x.x.x.x.x.x.", 2)] {
        for (i, s) in steps(pat, DRUM_BASE + d, 100).into_iter().enumerate().take(16) {
            if !s.notes.is_empty() {
                drums.steps[i].notes.push(s.notes[0]);
                drums.steps[i].vel = s.vel.max(drums.steps[i].vel);
            }
        }
    }
    let mut drums2 = drums.clone();
    for i in [14, 15] {
        drums2.steps[i].notes.push(DRUM_BASE + 4);
    }
    p.tracks[0].clips[0] = Some(drums);
    p.tracks[0].clips[1] = Some(drums2);
    // Bass: A minor, then F.
    let mut bass = Clip::default();
    for (i, n) in [(0, 45), (3, 45), (6, 57), (8, 45), (10, 48), (12, 50), (14, 52)] {
        bass.steps[i] = Step { notes: vec![n], vel: 105, len: 2, ..Step::default() };
    }
    let mut bass2 = Clip::default();
    for (i, n) in [(0, 41), (3, 41), (6, 53), (8, 43), (10, 43), (12, 47), (14, 55)] {
        bass2.steps[i] = Step { notes: vec![n], vel: 105, len: 2, ..Step::default() };
    }
    p.tracks[1].clips[0] = Some(bass);
    p.tracks[1].clips[1] = Some(bass2);
    let mut chords = Clip::default();
    chords.steps[0] = Step { notes: vec![57, 60, 64], vel: 80, len: 14, ..Step::default() };
    let mut chords2 = Clip::default();
    chords2.steps[0] = Step { notes: vec![53, 57, 60], vel: 80, len: 7, ..Step::default() };
    chords2.steps[8] = Step { notes: vec![55, 59, 62], vel: 80, len: 7, ..Step::default() };
    p.tracks[2].clips[0] = Some(chords);
    p.tracks[2].clips[1] = Some(chords2);
    p.song = vec![Part { scene: 0, bars: 4 }, Part { scene: 1, bars: 4 }];
    p
}

/// What the audio thread shares with the screen.
pub struct Shared {
    pub project: Mutex<Project>,
    /// Scene playing on each track, or -1.
    pub playing: [AtomicI32; TRACKS],
    /// Scene queued for the next bar (-1 stop), or -2 for nothing queued.
    pub queued: [AtomicI32; TRACKS],
    /// Where each track's clip started, in steps.
    pub start: [AtomicU64; TRACKS],
    /// The step each track last played, for the screen.
    pub pos: [AtomicI32; TRACKS],
    pub song_mode: AtomicBool,
    pub song_part: AtomicUsize,
    pub song_bar: AtomicU64,
    /// Live notes to record: (track, step, notes, velocity).
    pub lock_handles: Mutex<[Option<Arc<AtomicF32>>; TRACKS]>,
    /// Samples processed, and the sample the latest step started on.
    pub samples: AtomicU64,
    pub step_at: AtomicU64,
    pub routes: Vec<Arc<AtomicUsize>>,
}

impl Shared {
    fn new(routes: Vec<Arc<AtomicUsize>>) -> Shared {
        Shared {
            project: Mutex::new(demo()),
            playing: std::array::from_fn(|_| AtomicI32::new(-1)),
            queued: std::array::from_fn(|_| AtomicI32::new(-2)),
            start: std::array::from_fn(|_| AtomicU64::new(0)),
            pos: std::array::from_fn(|_| AtomicI32::new(-1)),
            song_mode: AtomicBool::new(false),
            song_part: AtomicUsize::new(0),
            song_bar: AtomicU64::new(0),
            lock_handles: Mutex::new(std::array::from_fn(|_| None)),
            samples: AtomicU64::new(0),
            step_at: AtomicU64::new(0),
            routes,
        }
    }

    pub fn queue_scene(&self, scene: usize) {
        let p = self.project.lock().unwrap();
        for t in 0..TRACKS {
            let has = p.tracks[t].clips.get(scene).is_some_and(|c| c.is_some());
            self.queued[t].store(if has { scene as i32 } else { -1 }, Ordering::Relaxed);
        }
    }
}

/// The sequencer, run by the clock on the audio thread.
struct Sequencer {
    s: Arc<Shared>,
    rng: Rng,
}

impl Sequencer {
    /// Applies launches (and the song) at the start of a bar.
    fn bar(&mut self, step: u64, p: &Project) {
        if self.s.song_mode.load(Ordering::Relaxed) && !p.song.is_empty() {
            let mut part = self.s.song_part.load(Ordering::Relaxed);
            let bar = self.s.song_bar.load(Ordering::Relaxed);
            let first = step == 0;
            if !first && bar + 1 >= p.song[part.min(p.song.len() - 1)].bars as u64 {
                part = (part + 1) % p.song.len();
                self.s.song_bar.store(0, Ordering::Relaxed);
                self.s.song_part.store(part, Ordering::Relaxed);
                self.queue(p, p.song[part].scene);
            } else if !first {
                self.s.song_bar.store(bar + 1, Ordering::Relaxed);
            } else {
                self.queue(p, p.song[part.min(p.song.len() - 1)].scene);
            }
        }
        for t in 0..TRACKS {
            let q = self.s.queued[t].swap(-2, Ordering::Relaxed);
            if q >= -1 {
                self.s.playing[t].store(q, Ordering::Relaxed);
                self.s.start[t].store(step, Ordering::Relaxed);
            }
        }
    }

    fn queue(&self, p: &Project, scene: usize) {
        for t in 0..TRACKS {
            let has = p.tracks[t].clips.get(scene).is_some_and(|c| c.is_some());
            self.s.queued[t].store(if has { scene as i32 } else { -1 }, Ordering::Relaxed);
        }
    }
}

impl Song for Sequencer {
    fn step(&mut self, step: u64, out: &mut Vec<Ev>) {
        let shared = Arc::clone(&self.s);
        let Ok(p) = shared.project.try_lock() else { return };
        if step % 16 == 0 {
            self.bar(step, &p);
        }
        self.s.step_at.store(self.s.samples.load(Ordering::Relaxed), Ordering::Relaxed);
        let locks = self.s.lock_handles.try_lock().ok();
        for t in 0..TRACKS {
            let scene = self.s.playing[t].load(Ordering::Relaxed);
            let track = &p.tracks[t];
            let Some(clip) = (scene >= 0).then(|| track.clips.get(scene as usize).and_then(|c| c.as_ref())).flatten() else {
                self.s.pos[t].store(-1, Ordering::Relaxed);
                continue;
            };
            let len = clip.length.clamp(1, MAX_STEPS) as u64;
            let at = ((step - self.s.start[t].load(Ordering::Relaxed).min(step)) % len) as usize;
            self.s.pos[t].store(at as i32, Ordering::Relaxed);
            let st = &clip.steps[at];
            if let (Some(v), Some(l)) = (st.lock, locks.as_ref()) {
                if let Some(h) = &l[t] {
                    h.set(v);
                }
            }
            if st.notes.is_empty() || track.mute {
                continue;
            }
            if st.chance < 100 && self.rng.below(100) >= st.chance as u32 {
                continue;
            }
            let internal = self.s.routes[t].load(Ordering::Relaxed) == INTERNAL;
            let vel = st.vel as f32 / 127.0;
            for &n in &st.notes {
                if track.kind == Kind::Drums && internal {
                    if let Some(d) = DRUMS.get(n.saturating_sub(DRUM_BASE) as usize) {
                        out.push(Ev::Drum(*d, vel));
                    }
                } else {
                    let n = (n as i32 + if track.kind == Kind::Notes { track.octave as i32 * 12 } else { 0 }).clamp(0, 127) as u8;
                    if internal {
                        let secs = st.len as f32 * 60.0 / p.tempo.max(20.0) / 4.0 * 0.95;
                        out.push(Ev::Note(Note::new(TONES[t], n as f32).vel(vel).len(secs).pan(t as f32 / 3.5 - 1.0)));
                    } else {
                        // Track, note, then velocity and length packed.
                        out.push(Ev::Custom(t as u32, n as f32, st.vel as f32 + st.len as f32 * 1000.0));
                    }
                }
            }
        }
    }
}

/// The note outputs, on the audio thread: timed notes from the
/// sequencer, and live notes from the pads.
struct Outs {
    s: Arc<Shared>,
    outs: Vec<NoteOut>,
    sr: f32,
}

impl Extra for Outs {
    fn block(&mut self, frames: usize, sr: f32) {
        self.sr = sr;
        for o in self.outs.iter_mut() {
            o.advance(frames as u32);
        }
        self.s.samples.fetch_add(frames as u64, Ordering::Relaxed);
    }
    fn event(&mut self, a: u32, b: f32, c: f32) {
        let t = (a & 0xff) as usize;
        let kind = a >> 8;
        let Some(o) = self.outs.get_mut(t) else { return };
        let note = b as u8;
        match kind {
            0 => {
                let len = (c / 1000.0).floor().max(1.0);
                let vel = (c - len * 1000.0).clamp(1.0, 127.0) as u8;
                let tempo = self.s.project.try_lock().map(|p| p.tempo).unwrap_or(110.0);
                let gate = (len * 60.0 / tempo.max(20.0) / 4.0 * 0.95 * self.sr) as u32;
                o.trigger(note, vel, gate);
            }
            1 => o.note_on(note, c.clamp(1.0, 127.0) as u8),
            _ => o.note_off(note),
        }
    }
    fn frame(&mut self, _sr: f32) -> (f32, f32) {
        (0.0, 0.0)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum View {
    Launch,
    Step,
    Play,
    Song,
    Setup,
}
const VIEWS: [View; 5] = [View::Launch, View::Step, View::Play, View::Song, View::Setup];

impl View {
    fn label(self) -> &'static str {
        match self {
            View::Launch => "LAUNCH",
            View::Step => "STEP",
            View::Play => "PLAY",
            View::Song => "SONG",
            View::Setup => "SETUP",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Note,
    Velocity,
    Length,
    Chance,
    Lock,
}
const FIELDS: [Field; 5] = [Field::Note, Field::Velocity, Field::Length, Field::Chance, Field::Lock];

#[derive(Clone, Copy, PartialEq)]
enum SetupRow {
    Track,
    Kind,
    Plays,
    Mute,
    Octave,
    LockApp,
    LockInput,
    ClipLength,
    Tempo,
    Swing,
    Clock,
    Slot,
    Save,
    Load,
}
const SETUP: [SetupRow; 14] = [SetupRow::Track, SetupRow::Plays, SetupRow::Kind, SetupRow::Mute, SetupRow::Octave, SetupRow::LockApp, SetupRow::LockInput, SetupRow::ClipLength, SetupRow::Tempo, SetupRow::Swing, SetupRow::Clock, SetupRow::Slot, SetupRow::Save, SetupRow::Load];

pub struct Session {
    pub sound: Sound,
    pub s: Arc<Shared>,
    modbus: Arc<ModBus>,
    notes: Option<Arc<NoteBus>>,
    routes: Vec<NoteRoute>,
    outs: Option<Vec<NoteOut>>,
    pub view: usize,
    pub track: usize,
    pub scene: usize,
    page: usize,
    field: usize,
    held: Option<usize>,
    held_frames: u32,
    edited_while_held: bool,
    last_note: u8,
    pub recording: bool,
    play_octave: i32,
    prev: [bool; 16],
    prev_keys: [u8; 128],
    /// Live notes sounding from the pads: (pad, note).
    live: Vec<(usize, u8)>,
    part: usize,
    setup_row: usize,
    lock_route: [usize; TRACKS],
    slot: usize,
    message: (String, u32),
    dir: Option<PathBuf>,
    frame: u64,
}

impl Session {
    pub fn new(sound: Sound, modbus: Arc<ModBus>, notes: Option<Arc<NoteBus>>, dir: Option<PathBuf>) -> Session {
        let mut routes = Vec::new();
        let mut outs = Vec::new();
        for t in 0..TRACKS {
            let (r, o) = NoteRoute::new(notes.clone(), &format!("{NAME} {}", t + 1), "session", true);
            routes.push(r);
            outs.push(o);
        }
        let handles: Vec<Arc<AtomicUsize>> = outs.iter().map(|o| o.route_handle()).collect();
        let s = Arc::new(Shared::new(handles));
        sound.set_tempo(110.0);
        sound.set_reverb(0.15);
        Session { sound, s, modbus, notes, routes, outs: Some(outs), view: 0, track: 0, scene: 0, page: 0, field: 0, held: None, held_frames: 0, edited_while_held: false, last_note: 60, recording: false, play_octave: 0, prev: [false; 16], prev_keys: [0; 128], live: Vec::new(), part: 0, setup_row: 0, lock_route: [0; TRACKS], slot: 0, message: (String::new(), 0), dir, frame: 0 }
    }

    fn view(&self) -> View {
        VIEWS[self.view]
    }

    fn flash(&mut self, m: impl Into<String>) {
        self.message = (m.into(), 150);
    }

    fn kind(&self, t: usize) -> Kind {
        self.s.project.lock().unwrap().tracks[t].kind
    }

    fn start(&self) {
        // Anything selected but not yet playing comes in on the first bar.
        if !self.sound.playing() {
            // Starting from silence plays the whole selected scene.
            if !self.s.queued.iter().any(|q| q.load(Ordering::Relaxed) >= 0) || !self.s.song_mode.load(Ordering::Relaxed) {
                self.s.queue_scene(self.scene);
            }
            self.s.song_part.store(0, Ordering::Relaxed);
            self.s.song_bar.store(0, Ordering::Relaxed);
            self.sound.start();
        }
    }

    fn stop(&mut self) {
        self.sound.stop();
        for t in 0..TRACKS {
            // Keep what was playing queued, so play resumes the same scene.
            let p = self.s.playing[t].swap(-1, Ordering::Relaxed);
            if p >= 0 {
                self.s.queued[t].store(p, Ordering::Relaxed);
            }
        }
        self.recording = false;
    }

    /// The selected track's clip in the selected scene, made if missing.
    fn clip_mut<R>(&self, f: impl FnOnce(&mut Clip) -> R) -> R {
        let mut p = self.s.project.lock().unwrap();
        let c = p.tracks[self.track].clips[self.scene].get_or_insert_with(Clip::default);
        f(c)
    }

    fn has_clip(&self, t: usize, scene: usize) -> bool {
        self.s.project.lock().unwrap().tracks[t].clips[scene].is_some()
    }

    /// Plays or stops a note live on the selected track.
    fn live_note(&mut self, note: u8, vel: u8, on: bool) {
        let t = self.track;
        let internal = self.routes[t].route() == INTERNAL;
        if self.kind(t) == Kind::Drums && internal {
            if on {
                if let Some(d) = DRUMS.get(note.saturating_sub(DRUM_BASE) as usize) {
                    self.sound.drum(*d, vel as f32 / 127.0);
                }
            }
        } else if internal {
            if on {
                self.sound.play(Note::new(TONES[t], note as f32).vel(vel as f32 / 127.0).held(1000 + t as u32 * 128 + note as u32));
            } else {
                self.sound.off(1000 + t as u32 * 128 + note as u32);
            }
        } else {
            self.sound.custom(((if on { 1 } else { 2 }) << 8) | t as u32, note as f32, vel as f32);
        }
        if on && self.recording {
            self.record(note, vel);
        }
    }

    /// Records a note at the step it was played nearest to.
    fn record(&mut self, note: u8, vel: u8) {
        let scene = self.s.playing[self.track].load(Ordering::Relaxed);
        if scene < 0 || scene as usize != self.scene || !self.sound.playing() {
            // Recording plays the clip being recorded into.
            if self.sound.playing() {
                self.s.queued[self.track].store(self.scene as i32, Ordering::Relaxed);
            }
        }
        let pos = self.s.pos[self.track].load(Ordering::Relaxed);
        if pos < 0 {
            return;
        }
        let step_len = 60.0 / self.sound.tempo() / 4.0 * 48_000.0;
        let into = (self.s.samples.load(Ordering::Relaxed) - self.s.step_at.load(Ordering::Relaxed).min(self.s.samples.load(Ordering::Relaxed))) as f32 / step_len;
        let octave = if self.kind(self.track) == Kind::Notes { self.s.project.lock().unwrap().tracks[self.track].octave as i32 * 12 } else { 0 };
        let note = (note as i32 - octave).clamp(0, 127) as u8;
        self.clip_mut(|c| {
            let at = (pos as usize + if into > 0.5 { 1 } else { 0 }) % c.length.max(1);
            let st = &mut c.steps[at];
            if !st.notes.contains(&note) && st.notes.len() < 4 {
                st.notes.push(note);
            }
            st.vel = vel;
        });
    }

    fn pad_note(&self, p: usize) -> u8 {
        if self.kind(self.track) == Kind::Drums {
            return DRUM_BASE + ((3 - p / 4) * 4 + p % 4) as u8;
        }
        let rank = ((3 - p / 4) * 4 + p % 4) as i32;
        (kit::scale_note(48 + self.play_octave * 12, &kit::MAJOR, rank)).clamp(0, 127) as u8
    }

    fn edit_step(&mut self, step: usize, d: i32) {
        let field = FIELDS[self.field];
        let last = &mut self.last_note;
        let mut p = self.s.project.lock().unwrap();
        let c = p.tracks[self.track].clips[self.scene].get_or_insert_with(Clip::default);
        let st = &mut c.steps[step];
        match field {
            Field::Note => {
                if st.notes.is_empty() {
                    st.notes.push(*last);
                } else {
                    for n in st.notes.iter_mut() {
                        *n = (*n as i32 + d).clamp(0, 127) as u8;
                    }
                }
                *last = st.notes[0];
            }
            Field::Velocity => st.vel = (st.vel as i32 + d * 8).clamp(1, 127) as u8,
            Field::Length => st.len = (st.len as i32 + d).clamp(1, 64) as u8,
            Field::Chance => st.chance = (st.chance as i32 + d * 10).clamp(0, 100) as u8,
            Field::Lock => st.lock = Some((st.lock.unwrap_or(0.5) + d as f32 * 0.05).clamp(0.0, 1.0)),
        }
    }

    fn save(&mut self) {
        let Some(dir) = self.dir.clone() else {
            self.flash("No save folder");
            return;
        };
        self.sync_names();
        let p = self.s.project.lock().unwrap().clone();
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("project_{}.json", self.slot + 1));
        match serde_json::to_string_pretty(&p).map(|j| std::fs::write(&path, j)) {
            Ok(Ok(())) => self.flash(format!("Saved project {}", self.slot + 1)),
            _ => self.flash("Couldn't save"),
        }
    }

    fn load(&mut self) {
        let Some(dir) = self.dir.clone() else { return };
        let path = dir.join(format!("project_{}.json", self.slot + 1));
        match std::fs::read_to_string(&path).ok().and_then(|j| serde_json::from_str::<Project>(&j).ok()) {
            Some(mut p) => {
                p.tracks.resize_with(TRACKS, Track::default);
                for t in p.tracks.iter_mut() {
                    t.clips.resize(SCENES, None);
                    for c in t.clips.iter_mut().flatten() {
                        c.steps.resize(MAX_STEPS, Step::default());
                    }
                }
                self.stop();
                for t in 0..TRACKS {
                    self.s.queued[t].store(-2, Ordering::Relaxed);
                }
                self.sound.set_tempo(p.tempo);
                self.sound.set_swing(p.swing);
                *self.s.project.lock().unwrap() = p;
                self.apply_names();
                self.flash(format!("Loaded project {}", self.slot + 1));
            }
            None => self.flash(format!("Slot {} is empty", self.slot + 1)),
        }
    }

    /// Stores each track's routing by name, so a project finds its
    /// instruments again however the slots are numbered next time.
    fn sync_names(&mut self) {
        let mut p = self.s.project.lock().unwrap();
        p.tempo = self.sound.tempo();
        p.swing = self.sound.swing();
        for t in 0..TRACKS {
            p.tracks[t].plays = if self.routes[t].route() == INTERNAL { String::new() } else { self.routes[t].label() };
            let r = self.lock_route[t];
            p.tracks[t].lock_to = if r == 0 { String::new() } else { self.modbus.names().get(r - 1).cloned().unwrap_or_default() };
        }
    }

    fn apply_names(&mut self) {
        let p = self.s.project.lock().unwrap().clone();
        for t in 0..TRACKS {
            let route = if p.tracks[t].plays.is_empty() {
                INTERNAL
            } else {
                self.notes.as_ref().and_then(|b| b.instrument_index(&p.tracks[t].plays)).unwrap_or(INTERNAL)
            };
            self.s.routes[t].store(route, Ordering::Relaxed);
            self.lock_route[t] = if p.tracks[t].lock_to.is_empty() { 0 } else { self.modbus.index_of(&p.tracks[t].lock_to).map_or(0, |i| i + 1) };
        }
        self.refresh_locks();
    }

    fn refresh_locks(&self) {
        let mut h = self.s.lock_handles.lock().unwrap();
        for t in 0..TRACKS {
            let r = self.lock_route[t];
            h[t] = if r == 0 { None } else { self.modbus.get(r - 1) };
        }
    }

    fn setup_value(&self, r: SetupRow) -> String {
        let p = self.s.project.lock().unwrap();
        let t = &p.tracks[self.track];
        match r {
            SetupRow::Track => format!("{}", self.track + 1),
            SetupRow::Kind => if t.kind == Kind::Drums { "drums".into() } else { "notes".into() },
            SetupRow::Plays => self.routes[self.track].label(),
            SetupRow::Mute => if t.mute { "muted".into() } else { "on".into() },
            SetupRow::Octave => format!("{:+}", t.octave),
            SetupRow::LockApp => Patch::app_label(&self.modbus, self.lock_route[self.track]),
            SetupRow::LockInput => Patch::input_label(&self.modbus, self.lock_route[self.track]),
            SetupRow::ClipLength => t.clips[self.scene].as_ref().map_or("(no clip)".into(), |c| format!("{} steps", c.length)),
            SetupRow::Tempo => format!("{:.0} bpm", self.sound.tempo()),
            SetupRow::Swing => format!("{:.0}%", self.sound.swing() * 100.0),
            SetupRow::Clock => if self.sound.following() { "device".into() } else { "own".into() },
            SetupRow::Slot => format!("{}", self.slot + 1),
            SetupRow::Save => "< > to save".into(),
            SetupRow::Load => "< > to load".into(),
        }
    }

    fn setup_label(r: SetupRow) -> &'static str {
        match r {
            SetupRow::Track => "Track",
            SetupRow::Kind => "Kind",
            SetupRow::Plays => "Plays",
            SetupRow::Mute => "Mute",
            SetupRow::Octave => "Octave",
            SetupRow::LockApp => "Lock to app",
            SetupRow::LockInput => "Lock to input",
            SetupRow::ClipLength => "Clip length",
            SetupRow::Tempo => "Tempo",
            SetupRow::Swing => "Swing",
            SetupRow::Clock => "Clock",
            SetupRow::Slot => "Project slot",
            SetupRow::Save => "Save",
            SetupRow::Load => "Load",
        }
    }

    fn setup_edit(&mut self, d: i32) {
        let row = SETUP[self.setup_row];
        let t = self.track;
        match row {
            SetupRow::Track => self.track = (self.track as i32 + d).rem_euclid(TRACKS as i32) as usize,
            SetupRow::Kind => {
                let mut p = self.s.project.lock().unwrap();
                p.tracks[t].kind = if p.tracks[t].kind == Kind::Notes { Kind::Drums } else { Kind::Notes };
            }
            SetupRow::Plays => self.routes[t].step(d),
            SetupRow::Mute => {
                let mut p = self.s.project.lock().unwrap();
                p.tracks[t].mute = !p.tracks[t].mute;
            }
            SetupRow::Octave => {
                let mut p = self.s.project.lock().unwrap();
                p.tracks[t].octave = (p.tracks[t].octave as i32 + d).clamp(-3, 3) as i8;
            }
            SetupRow::LockApp => {
                self.lock_route[t] = Patch::step_app(&self.modbus, self.lock_route[t], d);
                self.refresh_locks();
            }
            SetupRow::LockInput => {
                self.lock_route[t] = Patch::step_input(&self.modbus, self.lock_route[t], d);
                self.refresh_locks();
            }
            SetupRow::ClipLength => self.clip_mut(|c| c.length = ((c.length as i32 + d * 16).clamp(16, MAX_STEPS as i32)) as usize),
            SetupRow::Tempo => self.sound.set_tempo(self.sound.tempo().round() + d as f32),
            SetupRow::Swing => self.sound.set_swing(((self.sound.swing() * 50.0).round() + d as f32) / 50.0),
            SetupRow::Clock => {
                self.stop();
                self.sound.set_follow(!self.sound.following());
            }
            SetupRow::Slot => self.slot = (self.slot as i32 + d).rem_euclid(8) as usize,
            SetupRow::Save => self.save(),
            SetupRow::Load => self.load(),
        }
    }

    fn pad_launch(&mut self, p: usize) {
        if p < TRACKS {
            let playing = self.s.playing[p].load(Ordering::Relaxed);
            let target = if playing == self.scene as i32 {
                -1
            } else if self.has_clip(p, self.scene) {
                self.scene as i32
            } else {
                -1
            };
            self.s.queued[p].store(target, Ordering::Relaxed);
            self.track = p;
            if target >= 0 && !self.sound.playing() {
                self.sound.start();
            }
        } else {
            self.scene = p - TRACKS;
            self.s.queue_scene(self.scene);
            if !self.sound.playing() {
                self.sound.start();
            }
        }
    }
}

impl App for Session {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn needs_background_audio(&self) -> bool {
        self.sound.playing()
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![("View".into(), self.view().label().into(), false), ("Track".into(), format!("{}", self.track + 1), false), ("Scene".into(), format!("{}", self.scene + 1), false)]
    }
    fn running(&self) -> Option<bool> {
        Some(self.sound.playing())
    }
    fn toggle_running(&mut self) {
        if self.sound.playing() {
            self.stop();
        } else {
            self.start();
        }
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.view().label())
    }
    fn toggle_grid_mode(&mut self) {
        self.view = (self.view + 1) % VIEWS.len();
        self.held = None;
    }
    fn on_exit(&mut self) {
        for (_, n) in std::mem::take(&mut self.live) {
            self.live_note(n, 0, false);
        }
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let p = self.s.project.lock().unwrap();
        match self.view() {
            View::Launch => std::array::from_fn(|i| {
                if i < TRACKS {
                    let playing = self.s.playing[i].load(Ordering::Relaxed) == self.scene as i32;
                    let queued = self.s.queued[i].load(Ordering::Relaxed) >= -1;
                    if queued {
                        PadColor::Yellow
                    } else if playing {
                        PadColor::Green
                    } else if p.tracks[i].clips[self.scene].is_some() {
                        PadColor::Blue
                    } else {
                        PadColor::Off
                    }
                } else if i - TRACKS == self.scene {
                    PadColor::Red
                } else {
                    PadColor::Blue
                }
            }),
            View::Step => {
                let clip = p.tracks[self.track].clips[self.scene].as_ref();
                let pos = self.s.pos[self.track].load(Ordering::Relaxed);
                std::array::from_fn(|i| {
                    let at = self.page * 16 + i;
                    if pos == at as i32 && self.sound.playing() {
                        PadColor::Red
                    } else if clip.is_some_and(|c| at < c.length && !c.steps[at].notes.is_empty()) {
                        PadColor::Green
                    } else if clip.is_some_and(|c| at >= c.length) {
                        PadColor::Off
                    } else {
                        PadColor::Blue
                    }
                })
            }
            View::Play => std::array::from_fn(|i| if self.recording && i == 0 { PadColor::Red } else if (self.pad_note(i) % 12) == 0 { PadColor::Blue } else { PadColor::Off }),
            View::Song => std::array::from_fn(|i| if i < SCENES { PadColor::Green } else if i == 15 { PadColor::Red } else { PadColor::Off }),
            View::Setup => [PadColor::Off; 16],
        }
    }

    fn tick(&mut self, input: &Input) {
        self.frame += 1;
        // Note lengths are worked out on the audio thread from the
        // project's tempo; keep it the tempo actually playing (the
        // device clock's, when following).
        if let Ok(mut p) = self.s.project.try_lock() {
            p.tempo = self.sound.tempo();
        }
        if self.message.1 > 0 {
            self.message.1 -= 1;
        }
        let pressed: Vec<usize> = (0..16).filter(|&p| input.grid[p] && !self.prev[p]).collect();
        let released: Vec<usize> = (0..16).filter(|&p| !input.grid[p] && self.prev[p]).collect();
        // SELECT: tap or hold.
        let select = input.knob1_press;
        let hold_select = input.knob2_press;
        match self.view() {
            View::Launch => {
                for p in pressed {
                    self.pad_launch(p);
                }
                if input.navigation_steps != 0 {
                    self.scene = (self.scene as i32 + input.navigation_steps).clamp(0, SCENES as i32 - 1) as usize;
                }
                if input.knob2 != 0 {
                    self.track = (self.track as i32 + input.knob2.signum()).rem_euclid(TRACKS as i32) as usize;
                }
                if hold_select {
                    for t in 0..TRACKS {
                        self.s.queued[t].store(-1, Ordering::Relaxed);
                    }
                } else if select {
                    self.toggle_running();
                }
            }
            View::Step => {
                for p in pressed {
                    if self.held.is_none() {
                        self.held = Some(p);
                        self.held_frames = 0;
                        self.edited_while_held = false;
                    }
                }
                if let Some(h) = self.held {
                    self.held_frames += 1;
                    let step = self.page * 16 + h;
                    if input.navigation_steps != 0 {
                        self.field = (self.field as i32 + input.navigation_steps).clamp(0, FIELDS.len() as i32 - 1) as usize;
                        self.edited_while_held = true;
                    }
                    if input.knob2 != 0 {
                        self.edit_step(step, input.knob2.signum());
                        self.edited_while_held = true;
                    }
                    if released.contains(&h) {
                        // A tap (no edit) toggles the step.
                        if !self.edited_while_held {
                            let last = self.last_note;
                            let kind = self.kind(self.track);
                            self.clip_mut(|c| {
                                if step >= c.length {
                                    return;
                                }
                                let st = &mut c.steps[step];
                                if st.notes.is_empty() {
                                    st.notes = vec![if kind == Kind::Drums { DRUM_BASE } else { last }];
                                    st.vel = 100;
                                } else {
                                    *st = Step::default();
                                }
                            });
                        }
                        self.held = None;
                    }
                } else {
                    if input.knob2 != 0 {
                        let len = self.s.project.lock().unwrap().tracks[self.track].clips[self.scene].as_ref().map_or(16, |c| c.length);
                        let pages = len.div_ceil(16) as i32;
                        self.page = (self.page as i32 + input.knob2.signum()).rem_euclid(pages.max(1)) as usize;
                    }
                    if input.navigation_steps != 0 {
                        self.track = (self.track as i32 + input.navigation_steps).rem_euclid(TRACKS as i32) as usize;
                        self.page = 0;
                    }
                    if select {
                        self.toggle_running();
                    }
                }
            }
            View::Play => {
                for p in pressed {
                    let n = self.pad_note(p);
                    self.live_note(n, 100, true);
                    self.live.push((p, n));
                }
                for p in released {
                    if let Some(i) = self.live.iter().position(|l| l.0 == p) {
                        let (_, n) = self.live.remove(i);
                        self.live_note(n, 0, false);
                    }
                }
                if input.knob2 != 0 {
                    self.play_octave = (self.play_octave + input.knob2.signum()).clamp(-2, 3);
                }
                if input.navigation_steps != 0 {
                    self.track = (self.track as i32 + input.navigation_steps).rem_euclid(TRACKS as i32) as usize;
                }
                if hold_select {
                    self.clip_mut(|c| *c = Clip { length: c.length, ..Clip::default() });
                    self.flash("Clip cleared");
                } else if select {
                    self.recording = !self.recording;
                    if self.recording {
                        self.clip_mut(|_| ());
                        self.s.queued[self.track].store(self.scene as i32, Ordering::Relaxed);
                        self.start();
                    }
                }
            }
            View::Song => {
                        for p in pressed {
                    let mut pr = self.s.project.lock().unwrap();
                    if p < SCENES && pr.song.len() < 64 {
                        pr.song.push(Part { scene: p, bars: 4 });
                        self.part = pr.song.len() - 1;
                    } else if p == 15 {
                        pr.song.pop();
                        self.part = self.part.min(pr.song.len().saturating_sub(1));
                    }
                }
                let n = self.s.project.lock().unwrap().song.len();
                if input.navigation_steps != 0 && n > 0 {
                    self.part = (self.part as i32 + input.navigation_steps).clamp(0, n as i32 - 1) as usize;
                }
                if input.knob2 != 0 && n > 0 {
                    let mut pr = self.s.project.lock().unwrap();
                    let b = &mut pr.song[self.part].bars;
                    *b = (*b as i32 + input.knob2.signum()).clamp(1, 64) as u32;
                }
                if select {
                    let on = !self.s.song_mode.load(Ordering::Relaxed);
                    self.s.song_mode.store(on, Ordering::Relaxed);
                    if on {
                        self.stop();
                        for t in 0..TRACKS {
                            self.s.queued[t].store(-2, Ordering::Relaxed);
                        }
                        self.s.song_part.store(0, Ordering::Relaxed);
                        self.s.song_bar.store(0, Ordering::Relaxed);
                        self.sound.start();
                    }
                }
            }
            View::Setup => {
                if input.navigation_steps != 0 {
                    self.setup_row = (self.setup_row as i32 + input.navigation_steps).clamp(0, SETUP.len() as i32 - 1) as usize;
                }
                if input.knob2 != 0 {
                    self.setup_edit(input.knob2.signum());
                }
                if select {
                    self.toggle_running();
                }
            }
        }
        // A MIDI keyboard always plays (and records into) the selected track.
        for n in 0..128 {
            let v = input.midi_keys.0[n];
            let was = self.prev_keys[n];
            if v > 0 && was == 0 {
                self.live_note(n as u8, v, true);
            } else if v == 0 && was > 0 {
                self.live_note(n as u8, 0, false);
            }
        }
        self.prev_keys = input.midi_keys.0;
        self.prev = input.grid;
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = Rgb565::new(3, 5, 6);
        let panel = Rgb565::new(6, 10, 12);
        let dim = Rgb565::new(14, 28, 24);
        let ink = kit::rgb(150, 230, 200);
        kit::clear(fb, bg);
        kit::rect(fb, 0, 0, 640, 30, panel);
        kit::text(fb, NAME, 12, 7, Size2::Medium, kit::WHITE, -1);
        // The views as tabs (F2 cycles).
        for (i, v) in VIEWS.iter().enumerate() {
            let x = 100 + i as i32 * 64;
            let sel = i == self.view;
            kit::round_rect(fb, x, 6, 60, 18, 6, if sel { ink } else { Rgb565::new(9, 16, 18) });
            kit::text(fb, v.label(), x + 30, 9, Size2::Small, if sel { kit::BLACK } else { dim }, 0);
        }
        let playing = self.sound.playing();
        let bar = self.s.pos[0..TRACKS].iter().map(|p| p.load(Ordering::Relaxed)).max().unwrap_or(-1);
        let status = if playing { format!("{:.0} bpm  {}", self.sound.tempo(), if bar >= 0 { format!("{}.{}", bar / 16 + 1, (bar % 16) / 4 + 1) } else { "-".into() }) } else { format!("{:.0} bpm  stopped", self.sound.tempo()) };
        kit::text(fb, &status, 630, 9, Size2::Small, if playing { ink } else { dim }, 1);
        let p = self.s.project.lock().unwrap().clone();
        match self.view() {
            View::Launch => {
                let (gx, gy, cw, ch) = (40, 56, 70, 30);
                for t in 0..TRACKS {
                    let (r, g, b) = TRACK_COLORS[t];
                    let col = kit::rgb(r, g, b);
                    let x = gx + t as i32 * cw;
                    kit::text(fb, &format!("{}", t + 1), x + cw / 2 - 2, 38, Size2::Small, if t == self.track { kit::WHITE } else { dim }, 0);
                    let playing_scene = self.s.playing[t].load(Ordering::Relaxed);
                    let queued = self.s.queued[t].load(Ordering::Relaxed);
                    for sc in 0..SCENES {
                        let y = gy + sc as i32 * ch;
                        let has = p.tracks[t].clips[sc].is_some();
                        let mut c = if has { kit::blend(col, bg, 0.55) } else { panel };
                        if playing_scene == sc as i32 && playing {
                            c = col;
                        }
                        if queued == sc as i32 && (self.frame / 10) % 2 == 0 {
                            c = kit::blend(col, kit::WHITE, 0.4);
                        }
                        kit::round_rect(fb, x + 3, y + 2, cw - 6, ch - 4, 5, c);
                        if playing_scene == sc as i32 && playing {
                            let pos = self.s.pos[t].load(Ordering::Relaxed).max(0) as f32;
                            let len = p.tracks[t].clips[sc].as_ref().map_or(16, |c| c.length) as f32;
                            kit::rect(fb, x + 6, y + ch - 7, ((cw - 12) as f32 * (pos + 1.0) / len) as i32, 2, kit::WHITE);
                        }
                    }
                }
                for sc in 0..SCENES {
                    let y = gy + sc as i32 * ch;
                    kit::text(fb, &format!("{}", sc + 1), 22, y + 9, Size2::Small, if sc == self.scene { kit::WHITE } else { dim }, 0);
                }
                kit::outline(fb, gx + self.track as i32 * cw + 1, gy + self.scene as i32 * ch, cw - 2, ch, 6, 2, kit::WHITE);
                kit::footer(fb, "Pads 1-8: this scene's clip on each track   9-16: launch a scene   SELECT: play/stop   F2: view", panel, dim);
            }
            View::Step => {
                let clip = p.tracks[self.track].clips[self.scene].clone().unwrap_or_default();
                let (r, g, b) = TRACK_COLORS[self.track];
                let col = kit::rgb(r, g, b);
                kit::text(fb, &format!("Track {}  scene {}  page {}/{}  ({} steps)", self.track + 1, self.scene + 1, self.page + 1, clip.length.div_ceil(16).max(1), clip.length), 12, 38, Size2::Small, kit::WHITE, -1);
                let pos = self.s.pos[self.track].load(Ordering::Relaxed);
                for i in 0..16 {
                    let at = self.page * 16 + i;
                    let x = 12 + (i as i32 % 4) * 92;
                    let y = 56 + (i as i32 / 4) * 66;
                    let st = &clip.steps[at];
                    let inside = at < clip.length;
                    let on = inside && !st.notes.is_empty();
                    let mut c = if on { kit::blend(col, bg, 1.0 - st.vel as f32 / 127.0 * 0.8) } else if inside { panel } else { bg };
                    if pos == at as i32 && playing {
                        c = kit::blend(c, kit::WHITE, 0.35);
                    }
                    kit::round_rect(fb, x, y, 86, 60, 8, c);
                    if self.held == Some(i) {
                        kit::outline(fb, x - 2, y - 2, 90, 64, 9, 2, kit::WHITE);
                    }
                    kit::text(fb, &format!("{}", at + 1), x + 6, y + 4, Size2::Small, dim, -1);
                    if on {
                        let names: Vec<String> = st.notes.iter().map(|&n| if p.tracks[self.track].kind == Kind::Drums { DRUMS.get(n.saturating_sub(DRUM_BASE) as usize).map_or("?".into(), |d| d.name().to_string()) } else { format!("{}{}", kit::note_name(n as i32), n as i32 / 12 - 1) }).collect();
                        let label: String = names.join(" ").chars().take(12).collect();
                        kit::text(fb, &label, x + 43, y + 22, Size2::Small, kit::BLACK, 0);
                        let extra = format!("{}{}{}", if st.len > 1 { format!("L{} ", st.len) } else { String::new() }, if st.chance < 100 { format!("{}% ", st.chance) } else { String::new() }, if st.lock.is_some() { "lock" } else { "" });
                        kit::text(fb, &extra, x + 43, y + 40, Size2::Small, kit::BLACK, 0);
                    }
                }
                // The held step's fields.
                let fx = 390;
                kit::round_rect(fb, fx, 56, 242, 258, 8, panel);
                if let Some(h) = self.held {
                    let st = &clip.steps[self.page * 16 + h];
                    kit::text(fb, &format!("Step {}", self.page * 16 + h + 1), fx + 10, 64, Size2::Medium, kit::WHITE, -1);
                    for (k, f) in FIELDS.iter().enumerate() {
                        let y = 92 + k as i32 * 28;
                        let sel = k == self.field;
                        let (l, v) = match f {
                            Field::Note => ("note", st.notes.iter().map(|&n| format!("{}{}", kit::note_name(n as i32), n as i32 / 12 - 1)).collect::<Vec<_>>().join(" ")),
                            Field::Velocity => ("velocity", format!("{}", st.vel)),
                            Field::Length => ("length", format!("{} steps", st.len)),
                            Field::Chance => ("chance", format!("{}%", st.chance)),
                            Field::Lock => ("lock", st.lock.map_or("-".into(), |v| format!("{:.2}", v))),
                        };
                        kit::round_rect(fb, fx + 6, y, 230, 24, 6, if sel { kit::blend(panel, kit::WHITE, 0.15) } else { panel });
                        kit::text(fb, l, fx + 14, y + 6, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
                        kit::text(fb, &v, fx + 228, y + 6, Size2::Small, ink, 1);
                    }
                    let lock = Patch::label(&self.modbus, self.lock_route[self.track]);
                    kit::paragraph(fb, &format!("Lock goes to: {lock} (set in SETUP)"), fx + 10, 240, 222, Size2::Small, dim);
                } else {
                    kit::paragraph(fb, "Tap a pad to set or clear a step. Hold one and use the D-pad to edit it: up/down picks note, velocity, length, chance or lock; left/right changes it.\n\nLeft/right: page. Up/down: track.", fx + 10, 66, 222, Size2::Small, dim);
                }
                kit::footer(fb, "STEP  SELECT: play/stop   F2: next view", panel, dim);
            }
            View::Play => {
                let kind = p.tracks[self.track].kind;
                kit::text(fb, &format!("Track {} plays {}{}", self.track + 1, self.routes[self.track].label(), if self.recording { "   RECORDING" } else { "" }), 12, 38, Size2::Small, if self.recording { kit::rgb(250, 90, 90) } else { kit::WHITE }, -1);
                let (r, g, b) = TRACK_COLORS[self.track];
                let col = kit::rgb(r, g, b);
                for i in 0..16 {
                    let x = 12 + (i as i32 % 4) * 92;
                    let y = 56 + (i as i32 / 4) * 66;
                    let n = self.pad_note(i);
                    let lit = self.live.iter().any(|l| l.0 == i);
                    kit::round_rect(fb, x, y, 86, 60, 8, if lit { col } else { panel });
                    let label = if kind == Kind::Drums { DRUMS.get((n - DRUM_BASE) as usize).map_or("-".to_string(), |d| d.name().to_string()) } else { format!("{}{}", kit::note_name(n as i32), n as i32 / 12 - 1) };
                    kit::text(fb, &label, x + 43, y + 22, Size2::Small, if lit { kit::BLACK } else { kit::WHITE }, 0);
                }
                kit::paragraph(fb, "The pads play the selected track (C major; a kit on drum tracks). A MIDI keyboard does too.\n\nSELECT: record into this scene's clip, to the nearest step. Hold SELECT: clear it.\n\nLeft/right: octave. Up/down: track.", 400, 60, 230, Size2::Small, dim);
                kit::footer(fb, "PLAY  F2: next view", panel, dim);
            }
            View::Song => {
                let on = self.s.song_mode.load(Ordering::Relaxed);
                let cur = self.s.song_part.load(Ordering::Relaxed);
                kit::text(fb, if on { "Song mode: on (SELECT to turn off)" } else { "Song mode: off (SELECT plays the song from the top)" }, 12, 38, Size2::Small, if on { ink } else { kit::WHITE }, -1);
                let total: u32 = p.song.iter().map(|s| s.bars).sum();
                for (i, part) in p.song.iter().enumerate().take(32) {
                    let x = 12 + (i as i32 % 8) * 76;
                    let y = 56 + (i as i32 / 8) * 54;
                    let (r, g, b) = TRACK_COLORS[part.scene % TRACKS];
                    let live = on && i == cur && playing;
                    kit::round_rect(fb, x, y, 70, 48, 8, if live { kit::rgb(r, g, b) } else { kit::blend(kit::rgb(r, g, b), bg, 0.6) });
                    if i == self.part {
                        kit::outline(fb, x - 2, y - 2, 74, 52, 9, 2, kit::WHITE);
                    }
                    kit::text(fb, &format!("scene {}", part.scene + 1), x + 35, y + 8, Size2::Small, kit::BLACK, 0);
                    kit::text(fb, &format!("{} bars", part.bars), x + 35, y + 26, Size2::Small, kit::BLACK, 0);
                }
                kit::text(fb, &format!("{} parts, {} bars", p.song.len(), total), 12, 280, Size2::Small, dim, -1);
                kit::footer(fb, "Pads 1-8: add scene   pad 16: remove last   up/down: part   left/right: bars", panel, dim);
            }
            View::Setup => {
                for (i, r) in SETUP.iter().enumerate() {
                    let y = 38 + i as i32 * 22;
                    let sel = i == self.setup_row;
                    kit::round_rect(fb, 8, y, 330, 20, 5, if sel { kit::blend(panel, kit::WHITE, 0.15) } else { panel });
                    kit::text(fb, Session::setup_label(*r), 16, y + 4, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
                    let v: String = self.setup_value(*r).chars().take(28).collect();
                    kit::text(fb, &v, 330, y + 4, Size2::Small, ink, 1);
                }
                // Tracks at a glance.
                for t in 0..TRACKS {
                    let y = 40 + t as i32 * 34;
                    let (r, g, b) = TRACK_COLORS[t];
                    kit::round_rect(fb, 350, y, 282, 30, 6, if t == self.track { kit::blend(panel, kit::rgb(r, g, b), 0.3) } else { panel });
                    kit::circle(fb, 364, y + 15, 6, if p.tracks[t].mute { dim } else { kit::rgb(r, g, b) });
                    let what = if p.tracks[t].kind == Kind::Drums { "drums" } else { "notes" };
                    kit::text(fb, &format!("{} {}  {}", t + 1, what, self.routes[t].label()).chars().take(38).collect::<String>(), 376, y + 9, Size2::Small, kit::WHITE, -1);
                }
                kit::footer(fb, "SETUP  up/down: row   left/right: change   SELECT: play/stop", panel, dim);
            }
        }
        if self.message.1 > 0 {
            kit::round_rect(fb, 180, 300, 280, 30, 8, kit::rgb(60, 120, 100));
            kit::text(fb, &self.message.0, 320, 308, Size2::Small, kit::WHITE, 0);
        }
    }

    fn audio_processor(&mut self) -> Option<Box<dyn crate::audio::AudioProcessor>> {
        let outs = self.outs.take().unwrap_or_default();
        let seq = Sequencer { s: Arc::clone(&self.s), rng: Rng::seeded_from_time() };
        Some(self.sound.processor(Some(Box::new(seq)), Some(Box::new(Outs { s: Arc::clone(&self.s), outs, sr: 48_000.0 }))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    let dir = (!cfg!(test)).then(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("saves/session"));
    let sound = Sound::new(NAME, &modbus, &mixer, &bus);
    // On the device Session drives the shared transport, so everything
    // following it (Skins, the Looper, MIDI clock out) plays in time.
    sound.set_follow(true);
    Box::new(Session::new(sound, Arc::clone(&modbus), ctx.try_get(), dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render};
    use crate::note_bus::NoteView;

    fn session() -> Session {
        Session::new(Sound::detached(), Arc::new(ModBus::new()), None, None)
    }

    #[test]
    fn play_starts_the_first_scene_on_the_first_bar() {
        let mut s = session();
        let mut p = s.audio_processor().unwrap();
        s.toggle_running();
        let out = render(&mut p, 40);
        assert!(energy(&out) > 1e-4, "the demo plays");
        assert_eq!(s.s.playing[0].load(Ordering::Relaxed), 0);
        assert_eq!(s.s.playing[1].load(Ordering::Relaxed), 0);
        assert_eq!(s.s.playing[3].load(Ordering::Relaxed), -1, "no clip, nothing playing");
    }

    #[test]
    fn launching_a_scene_waits_for_the_bar() {
        let mut s = session();
        s.sound.set_tempo(120.0);
        let mut p = s.audio_processor().unwrap();
        s.toggle_running();
        render(&mut p, 10); // a quarter bar in
        s.pad_launch(TRACKS + 1); // scene 2
        assert_eq!(s.s.playing[1].load(Ordering::Relaxed), 0, "still scene 1 mid-bar");
        // A bar at 120 bpm is 2 s = 96000 frames.
        render(&mut p, 190);
        assert_eq!(s.s.playing[1].load(Ordering::Relaxed), 1, "scene 2 from the next bar");
        assert_eq!(s.s.pos[1].load(Ordering::Relaxed) < 4, true, "and from its first step");
    }

    #[test]
    fn a_track_plays_another_app_through_the_note_bus() {
        let bus = Arc::new(NoteBus::new());
        let inbox = bus.register_instrument("plaits", "Plaits").unwrap();
        let mut s = Session::new(Sound::detached(), Arc::new(ModBus::new()), Some(Arc::clone(&bus)), None);
        // Track 2 (the bass) -> Plaits.
        s.track = 1;
        while s.routes[1].label() != "Plaits" {
            s.routes[1].step(1);
        }
        let mut p = s.audio_processor().unwrap();
        s.toggle_running();
        render(&mut p, 2);
        assert!(inbox.any_held(), "Plaits is playing the bass's first note");
        // Steps are two 16ths long (0.27 s at 110 bpm); the next note is at step 3.
        let mut view = NoteView::default();
        inbox.poll(&mut view);
        assert_eq!(view.keys[45] > 0, true, "A2");
    }

    #[test]
    fn steps_toggle_and_edit() {
        let mut s = session();
        s.view = 1; // STEP
        s.track = 3;
        let pad = |i: usize| Input { grid: std::array::from_fn(|g| g == i), ..Default::default() };
        s.tick(&pad(2));
        s.tick(&Input::default());
        let st = s.s.project.lock().unwrap().tracks[3].clips[0].as_ref().unwrap().steps[2].clone();
        assert_eq!(st.notes, vec![60]);
        // Hold and raise the note by two, then the velocity.
        s.tick(&pad(2));
        s.tick(&Input { knob2: 1, ..pad(2) });
        s.tick(&Input { knob2: 1, ..pad(2) });
        s.tick(&Input { navigation_steps: 1, ..pad(2) });
        s.tick(&Input { knob2: -1, ..pad(2) });
        s.tick(&Input::default());
        let st = s.s.project.lock().unwrap().tracks[3].clips[0].as_ref().unwrap().steps[2].clone();
        assert_eq!(st.notes, vec![62]);
        assert_eq!(st.vel, 92);
    }

    #[test]
    fn the_lock_lane_moves_any_apps_input() {
        let bus = Arc::new(ModBus::new());
        let target = bus.register("Synth: Cutoff");
        let mut s = Session::new(Sound::detached(), Arc::clone(&bus), None, None);
        s.lock_route[1] = bus.index_of("Synth: Cutoff").unwrap() + 1;
        s.refresh_locks();
        s.s.project.lock().unwrap().tracks[1].clips[0].as_mut().unwrap().steps[0].lock = Some(0.8);
        let mut p = s.audio_processor().unwrap();
        s.toggle_running();
        render(&mut p, 2);
        assert!((target.get() - 0.8).abs() < 1e-6);
    }

    #[test]
    fn two_sessions_on_the_device_clock_play_in_lockstep() {
        let clock = Arc::new(crate::clock::Clock::new());
        let mk = || {
            let snd = Sound::detached_with_clock(Arc::clone(&clock));
            snd.set_follow(true);
            Session::new(snd, Arc::new(ModBus::new()), None, None)
        };
        let mut a = mk();
        let mut b = mk();
        let mut pa = a.audio_processor().unwrap();
        let mut pb = b.audio_processor().unwrap();
        a.sound.set_tempo(128.0);
        assert_eq!(b.sound.tempo(), 128.0, "one tempo for the device");
        // b is started by nobody: it just has a scene waiting
        b.s.queue_scene(1);
        a.toggle_running();
        assert!(b.sound.playing(), "a's play button started the device");
        let mut buf = vec![0.0f32; 256 * 2];
        for _ in 0..900 {
            pa.process(&mut buf, 2, 48_000.0);
            pb.process(&mut buf, 2, 48_000.0);
            clock.end_block(256, 48_000.0);
        }
        let pos = |s: &Session| s.s.pos[1].load(Ordering::Relaxed);
        assert!(pos(&a) >= 0 && pos(&b) >= 0, "both playing: {} {}", pos(&a), pos(&b));
        assert_eq!(a.sound.step(), b.sound.step());
        // 900 * 256 samples at 128 bpm is 40 sixteenths
        assert_eq!(a.sound.step(), Some(40));
        a.toggle_running();
        pa.process(&mut buf, 2, 48_000.0);
        clock.end_block(256, 48_000.0);
        assert!(!b.sound.playing(), "and stopping stops everyone");
    }

    #[test]
    fn the_song_chains_scenes() {
        let mut s = session();
        s.sound.set_tempo(240.0); // a bar a second
        s.s.project.lock().unwrap().song = vec![Part { scene: 0, bars: 1 }, Part { scene: 1, bars: 1 }];
        let mut p = s.audio_processor().unwrap();
        s.view = 3;
        s.tick(&Input { knob1_press: true, ..Default::default() });
        render(&mut p, 50);
        assert_eq!(s.s.playing[1].load(Ordering::Relaxed), 0);
        render(&mut p, 94);
        assert_eq!(s.s.playing[1].load(Ordering::Relaxed), 1, "second part");
        render(&mut p, 94);
        assert_eq!(s.s.playing[1].load(Ordering::Relaxed), 0, "and round again");
    }

    #[test]
    fn recording_puts_notes_on_the_nearest_step() {
        let mut s = session();
        s.view = 2; // PLAY
        s.track = 4;
        s.scene = 0;
        let mut p = s.audio_processor().unwrap();
        s.tick(&Input { knob1_press: true, ..Default::default() });
        assert!(s.recording);
        render(&mut p, 3);
        s.tick(&Input { grid: std::array::from_fn(|g| g == 12), ..Default::default() });
        s.tick(&Input::default());
        let clip = s.s.project.lock().unwrap().tracks[4].clips[0].clone().unwrap();
        assert!(clip.steps.iter().take(3).any(|st| st.notes == vec![48]), "C3 recorded near the start");
    }

    #[test]
    fn projects_save_and_load() {
        let dir = std::env::temp_dir().join(format!("portamax-session-{}", std::process::id()));
        let mut s = Session::new(Sound::detached(), Arc::new(ModBus::new()), None, Some(dir.clone()));
        s.s.project.lock().unwrap().tracks[5].clips[3] = Some(Clip { length: 32, ..Clip::default() });
        s.sound.set_tempo(93.0);
        s.save();
        let mut t = Session::new(Sound::detached(), Arc::new(ModBus::new()), None, Some(dir.clone()));
        t.load();
        assert_eq!(t.s.project.lock().unwrap().tracks[5].clips[3].as_ref().unwrap().length, 32);
        assert_eq!(t.sound.tempo(), 93.0);
        let _ = std::fs::remove_dir_all(dir);
        let mut fb = FrameBuffer::new();
        for v in 0..5 {
            t.view = v;
            t.draw(&mut fb);
        }
    }
}
