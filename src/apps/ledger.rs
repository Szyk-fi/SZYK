//! Ledger: a tracker. Inspired by the idea of Polyend's Tracker (a
//! hardware tracker with a performance mode); the format, effects and
//! layout here are Portamax's own. The sound is eight real Plaits voices
//! (plaits_ffi.rs), one per track, each with its own engine -- the drum
//! engines make the kit, the rest the music.
//!
//! A pattern is up to 64 rows (sixteenths) by 8 tracks. Each cell has a
//! note (or OFF), a volume (00-99) and one effect with a value:
//! - T/M/H/D: Timbre, Morph, Harmonics or Decay for this note (00-99)
//! - R: retrigger the note n times within the row
//! - P: play the row with this % chance
//! - N: nudge the note late by n/8 of a row
//! 16 patterns; the next one queues and takes over when the current one
//! ends.
//!
//! Two faces:
//! - Play view (PERFORM pads): pads 1-8 mute tracks; pads 9-16 are held
//!   performance moves -- loop the last beat, half beat or row, reverse,
//!   octave up/down, half speed, dark. Knobs edit the focused track's
//!   sound, tempo and swing.
//! - The editor (R1): knob 1 moves down the rows, knob 1 press steps
//!   across the fields and tracks, knob 2 changes the field, knob 2 press
//!   clears it (a note field cycles empty -> OFF). Pads enter notes on
//!   the focused track at the cursor (chromatic from the edit octave),
//!   then the cursor moves down by Step Add.
//!
//! Patterns and track sounds are saved to `saves/ledger/project.json`.

use crate::{
    app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    display::FrameBuffer,
    led_output::PadColor,
    mixer_bus::MixerBus,
    modbus::ModBus,
    plaits_ffi::{PlaitsParams, PlaitsVoice},
    spleen_fonts::{SPLEEN_6X12, SPLEEN_8X16},
    util::AtomicF32,
};
use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "Ledger";
pub const TRACKS: usize = 8;
pub const ROWS: usize = 64;
pub const PATTERNS: usize = 16;

// Own palette: green-screen ledger paper.
const BG: Rgb565 = Rgb565::new(1, 5, 2);
const INK: Rgb565 = Rgb565::new(14, 56, 16);
const ACCENT: Rgb565 = Rgb565::new(31, 50, 10);
const DIM: Rgb565 = Rgb565::new(5, 26, 8);
const FAINT: Rgb565 = Rgb565::new(2, 10, 3);
const CURSOR: Rgb565 = Rgb565::new(8, 30, 10);

pub const FX: [(&str, &str); 8] = [("-", "none"), ("T", "Timbre"), ("M", "Morph"), ("H", "Harmonics"), ("D", "Decay"), ("R", "Retrigger"), ("P", "Chance"), ("N", "Nudge")];
/// T, M, H, D are 1..=4 in that order; a lock goes to `locks[fx - FX_T]`.
const FX_T: u8 = 1;
const FX_D: u8 = 4;
const FX_R: u8 = 5;
const FX_P: u8 = 6;
const FX_N: u8 = 7;

/// A note field: 0 = empty, 1..=128 = MIDI note + 1, 255 = note off.
const NOTE_EMPTY: u8 = 0;
const NOTE_OFF: u8 = 255;

/// One cell, packed: note | vol+1 (0 = empty) | fx | fx value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cell {
    pub note: u8,
    /// 0 = empty, 1..=100 = volume 0..99.
    pub vol: u8,
    pub fx: u8,
    pub val: u8,
}

impl Cell {
    fn pack(self) -> u32 {
        (self.note as u32) | (self.vol as u32) << 8 | (self.fx as u32) << 16 | (self.val as u32) << 24
    }
    fn unpack(x: u32) -> Self {
        Self { note: x as u8, vol: (x >> 8) as u8, fx: (x >> 16) as u8, val: (x >> 24) as u8 }
    }
    fn is_empty(self) -> bool {
        self == Cell::default()
    }
    fn note_text(self) -> String {
        match self.note {
            NOTE_EMPTY => "---".into(),
            NOTE_OFF => "OFF".into(),
            n => {
                let m = n - 1;
                let names = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];
                format!("{}{}", names[m as usize % 12], (m as i32 / 12 - 1).clamp(0, 9))
            }
        }
    }
    fn text(self) -> String {
        let vol = if self.vol == 0 { "--".to_string() } else { format!("{:02}", self.vol - 1) };
        let fx = if self.fx == 0 { "---".to_string() } else { format!("{}{:02}", FX[self.fx as usize % FX.len()].0, self.val) };
        format!("{} {} {}", self.note_text(), vol, fx)
    }
}

struct TrackShared {
    engine: AtomicU32,
    harmonics: AtomicF32,
    timbre: AtomicF32,
    morph: AtomicF32,
    decay: AtomicF32,
    level: AtomicF32,
    muted: AtomicBool,
    /// Telemetry: struck this row.
    lit: AtomicBool,
}

impl TrackShared {
    fn new(engine: u32, decay: f32) -> Self {
        Self {
            engine: AtomicU32::new(engine),
            harmonics: AtomicF32::new(0.5),
            timbre: AtomicF32::new(0.5),
            morph: AtomicF32::new(0.5),
            decay: AtomicF32::new(decay),
            level: AtomicF32::new(0.7),
            muted: AtomicBool::new(false),
            lit: AtomicBool::new(false),
        }
    }
}

/// Performance moves (pads 9-16 on PERFORM), held.
pub const MOVES: [&str; 8] = ["LOOP 4", "LOOP 2", "LOOP 1", "REVERSE", "OCT+", "OCT-", "HALF", "DARK"];

struct Shared {
    cells: Vec<AtomicU32>,
    tracks: [TrackShared; TRACKS],
    bpm: AtomicF32,
    swing: AtomicF32,
    length: [AtomicUsize; PATTERNS],
    pattern: AtomicUsize,
    queued: AtomicUsize,
    running: AtomicBool,
    moves: [AtomicBool; 8],
    // telemetry
    row: AtomicUsize,
    playing_pattern: AtomicUsize,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    dirty: AtomicU32,
    /// Audition requests from the editor: track << 8 | note, or u32::MAX.
    audition: AtomicU32,
}

impl Shared {
    fn idx(p: usize, row: usize, t: usize) -> usize {
        (p % PATTERNS) * ROWS * TRACKS + (row % ROWS) * TRACKS + t % TRACKS
    }
    fn cell(&self, p: usize, row: usize, t: usize) -> Cell {
        Cell::unpack(self.cells[Self::idx(p, row, t)].load(Ordering::Relaxed))
    }
    fn set_cell(&self, p: usize, row: usize, t: usize, c: Cell) {
        self.cells[Self::idx(p, row, t)].store(c.pack(), Ordering::Relaxed);
        self.dirty.fetch_add(1, Ordering::Relaxed);
    }
    fn len(&self, p: usize) -> usize {
        self.length[p % PATTERNS].load(Ordering::Relaxed).clamp(1, ROWS)
    }
}

// ------------------------------------------------------------ controls

const CONTROLS: [&str; 16] = ["Timbre", "Morph", "Track", "Engine", "Tempo", "Swing", "Harmonics", "Decay", "Level", "Pattern", "Length", "Edit Octave", "Step Add", "Run", "Next Pattern", "Clear Pattern"];
const C_TIMBRE: usize = 0;
const C_MORPH: usize = 1;
const C_TRACK: usize = 2;
const C_ENGINE: usize = 3;
const C_TEMPO: usize = 4;
const C_SWING: usize = 5;
const C_HARM: usize = 6;
const C_DECAY: usize = 7;
const C_LEVEL: usize = 8;
const C_PATTERN: usize = 9;
const C_LENGTH: usize = 10;
const C_OCTAVE: usize = 11;
const C_STEP: usize = 12;
const C_RUN: usize = 13;
const C_NEXT: usize = 14;
const C_CLEAR: usize = 15;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "ledger",
        layers: vec![Layer::Native(0, "PERFORM"), Layer::Controls, Layer::Moments],
        hero: vec![[C_TIMBRE, C_MORPH], [C_TRACK, C_ENGINE], [C_TEMPO, C_SWING], [C_HARM, C_DECAY]],
        browse: Some(C_TRACK),
        routes: Routes { stick_x: Some(C_TIMBRE), stick_y: Some(C_MORPH), hand_l: Some(C_HARM), hand_r: Some(C_DECAY) },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: false,
    }
}

/// Editor fields per track: note, volume, effect type, effect value.
const FIELDS: usize = 4;

pub struct LedgerApp {
    p: Arc<Shared>,
    sensitivity: Arc<AtomicF32>,
    kit: PlayKit,
    // editor
    cur_row: usize,
    cur_track: usize,
    cur_field: usize,
    edit_octave: usize,
    step_add: usize,
    prev_grid: [bool; 16],
    save_path: Option<std::path::PathBuf>,
    saved: u32,
    seen: u32,
    quiet: u32,
}

impl LedgerApp {
    pub fn new(sensitivity: Arc<AtomicF32>, _nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        // Bass drum, snare, hi-hat, a VA bass, string chords, FM lead,
        // wavetable, modal (indices into plaits::ENGINE_NAMES).
        let engines = [21, 22, 23, 8, 6, 10, 13, 20];
        let decays = [0.5, 0.45, 0.25, 0.4, 0.75, 0.5, 0.6, 0.7];
        let p = Arc::new(Shared {
            cells: (0..PATTERNS * ROWS * TRACKS).map(|_| AtomicU32::new(0)).collect(),
            tracks: std::array::from_fn(|i| TrackShared::new(engines[i], decays[i])),
            bpm: AtomicF32::new(118.0),
            swing: AtomicF32::new(0.0),
            length: std::array::from_fn(|_| AtomicUsize::new(16)),
            pattern: AtomicUsize::new(0),
            queued: AtomicUsize::new(0),
            running: AtomicBool::new(false),
            moves: std::array::from_fn(|_| AtomicBool::new(false)),
            row: AtomicUsize::new(0),
            playing_pattern: AtomicUsize::new(0),
            mix_level,
            ext_mix_level,
            output,
            dirty: AtomicU32::new(0),
            audition: AtomicU32::new(u32::MAX),
        });
        let mut app = Self {
            p,
            sensitivity,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            cur_row: 0,
            cur_track: 0,
            cur_field: 0,
            edit_octave: 4,
            step_add: 1,
            prev_grid: [false; 16],
            save_path: (!cfg!(test)).then(|| std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves/ledger/project.json"))),
            saved: 0,
            seen: 0,
            quiet: 0,
        };
        if !app.load() {
            app.demo();
        }
        app.saved = app.p.dirty.load(Ordering::Relaxed);
        app
    }

    /// A first pattern, so it plays something the first time (original).
    fn demo(&self) {
        let p = &self.p;
        let n = |m: u8| Cell { note: m + 1, vol: 0, fx: 0, val: 0 };
        for r in [0, 4, 8, 10, 12] {
            p.set_cell(0, r, 0, n(36));
        }
        for r in [4, 12] {
            p.set_cell(0, r, 1, n(50));
        }
        p.set_cell(0, 15, 1, Cell { note: 51, vol: 41, fx: FX_P, val: 50 });
        for r in (0..16).step_by(2) {
            p.set_cell(0, r, 2, Cell { note: 85, vol: if r % 4 == 0 { 80 } else { 45 }, fx: 0, val: 0 });
        }
        p.set_cell(0, 14, 2, Cell { note: 85, vol: 60, fx: FX_R, val: 3 });
        let bass = [(0, 33), (3, 33), (6, 45), (8, 31), (11, 31), (14, 43)];
        for (r, m) in bass {
            p.set_cell(0, r, 3, n(m));
        }
        p.set_cell(0, 0, 4, Cell { note: 58, vol: 50, fx: FX_T, val: 30 });
        p.set_cell(0, 8, 4, Cell { note: 56, vol: 50, fx: FX_T, val: 60 });
    }

    fn pattern(&self) -> usize {
        self.p.pattern.load(Ordering::Relaxed) % PATTERNS
    }

    fn track(&self) -> &TrackShared {
        &self.p.tracks[self.cur_track % TRACKS]
    }

    fn engine_name(i: u32) -> &'static str {
        crate::apps::plaits::ENGINE_NAMES[i as usize % crate::apps::plaits::ENGINE_NAMES.len()]
    }

    fn value(&self, c: usize) -> String {
        let t = self.track();
        let pct = |a: &AtomicF32| format!("{:.0}%", a.get() * 100.0);
        match c {
            C_TIMBRE => pct(&t.timbre),
            C_MORPH => pct(&t.morph),
            C_HARM => pct(&t.harmonics),
            C_DECAY => pct(&t.decay),
            C_LEVEL => pct(&t.level),
            C_TRACK => format!("{} · {}", self.cur_track + 1, Self::engine_name(t.engine.load(Ordering::Relaxed))),
            C_ENGINE => Self::engine_name(t.engine.load(Ordering::Relaxed)).into(),
            C_TEMPO => format!("{:.0} BPM", self.p.bpm.get()),
            C_SWING => format!("{:.0}%", self.p.swing.get() * 100.0),
            C_PATTERN => format!("{:02}", self.pattern() + 1),
            C_LENGTH => format!("{} rows", self.p.len(self.pattern())),
            C_OCTAVE => format!("{}", self.edit_octave),
            C_STEP => format!("{}", self.step_add),
            C_RUN => (if self.p.running.load(Ordering::Relaxed) { "playing" } else { "stopped" }).into(),
            C_NEXT => format!("{:02}", self.p.queued.load(Ordering::Relaxed) % PATTERNS + 1),
            _ => "press to clear".into(),
        }
    }

    fn label(&self, c: usize) -> String {
        let names = crate::apps::plaits::engine_param_names(self.track().engine.load(Ordering::Relaxed) as usize);
        match c {
            C_HARM => names[0].into(),
            C_TIMBRE => names[1].into(),
            C_MORPH => names[2].into(),
            _ => CONTROLS[c].into(),
        }
    }

    fn edit(&mut self, c: usize, d: i32) {
        if d == 0 {
            return;
        }
        let sens = self.sensitivity.get().max(0.01) * 10.0;
        let p = Arc::clone(&self.p);
        let t = &p.tracks[self.cur_track % TRACKS];
        let nudge = |a: &AtomicF32| a.set((a.get() + d as f32 * 0.01 * sens).clamp(0.0, 1.0));
        match c {
            C_TIMBRE => nudge(&t.timbre),
            C_MORPH => nudge(&t.morph),
            C_HARM => nudge(&t.harmonics),
            C_DECAY => nudge(&t.decay),
            C_LEVEL => nudge(&t.level),
            C_TRACK => self.cur_track = (self.cur_track as i32 + d.signum()).rem_euclid(TRACKS as i32) as usize,
            C_ENGINE => t.engine.store((t.engine.load(Ordering::Relaxed) as i32 + d.signum()).rem_euclid(crate::apps::plaits::ENGINE_NAMES.len() as i32) as u32, Ordering::Relaxed),
            C_TEMPO => p.bpm.set((p.bpm.get() + d as f32).clamp(40.0, 240.0)),
            C_SWING => p.swing.set((p.swing.get() + d as f32 * 0.01 * sens).clamp(0.0, 0.75)),
            C_PATTERN => {
                let n = (self.pattern() as i32 + d.signum()).rem_euclid(PATTERNS as i32) as usize;
                p.pattern.store(n, Ordering::Relaxed);
                p.queued.store(n, Ordering::Relaxed);
                self.cur_row = self.cur_row.min(p.len(n) - 1);
            }
            C_LENGTH => {
                let pat = self.pattern();
                let l = (p.len(pat) as i32 + d.signum() * if p.len(pat) >= 16 { 4 } else { 1 }).clamp(1, ROWS as i32) as usize;
                p.length[pat].store(l, Ordering::Relaxed);
                p.dirty.fetch_add(1, Ordering::Relaxed);
                self.cur_row = self.cur_row.min(l - 1);
            }
            C_OCTAVE => self.edit_octave = (self.edit_octave as i32 + d.signum()).clamp(0, 8) as usize,
            C_STEP => self.step_add = (self.step_add as i32 + d.signum()).clamp(0, 16) as usize,
            C_RUN => self.set_running(d > 0),
            C_NEXT => p.queued.store((p.queued.load(Ordering::Relaxed) as i32 + d.signum()).rem_euclid(PATTERNS as i32) as usize, Ordering::Relaxed),
            _ => {}
        }
    }

    fn set_running(&self, on: bool) {
        self.p.running.store(on, Ordering::Relaxed);
    }

    fn clear_pattern(&self) {
        let pat = self.pattern();
        for r in 0..ROWS {
            for t in 0..TRACKS {
                self.p.set_cell(pat, r, t, Cell::default());
            }
        }
    }

    fn knob(&self, c: usize) -> Knob<'_> {
        let t = self.track();
        match c {
            C_TIMBRE => Knob::F(&t.timbre, 0.0, 1.0),
            C_MORPH => Knob::F(&t.morph, 0.0, 1.0),
            C_HARM => Knob::F(&t.harmonics, 0.0, 1.0),
            C_DECAY => Knob::F(&t.decay, 0.0, 1.0),
            C_LEVEL => Knob::F(&t.level, 0.0, 1.0),
            C_ENGINE => Knob::U(&t.engine, crate::apps::plaits::ENGINE_NAMES.len() as u32),
            C_TEMPO => Knob::F(&self.p.bpm, 40.0, 240.0),
            C_SWING => Knob::F(&self.p.swing, 0.0, 0.75),
            C_RUN => Knob::B(&self.p.running),
            _ => Knob::None,
        }
    }

    // ------------------------------------------------------- editor

    fn cell_at_cursor(&self) -> Cell {
        self.p.cell(self.pattern(), self.cur_row, self.cur_track)
    }

    fn edit_field(&mut self, d: i32) {
        if d == 0 {
            return;
        }
        let mut c = self.cell_at_cursor();
        match self.cur_field {
            0 => {
                c.note = match c.note {
                    NOTE_EMPTY | NOTE_OFF => (self.edit_octave as i32 * 12 + 12 + 1).clamp(1, 128) as u8,
                    n => (n as i32 + d).clamp(1, 128) as u8,
                };
                self.audition(c.note);
            }
            1 => c.vol = if c.vol == 0 { 80 } else { (c.vol as i32 + d).clamp(1, 100) as u8 },
            2 => {
                c.fx = (c.fx as i32 + d.signum()).rem_euclid(FX.len() as i32) as u8;
                if c.fx == FX_R && c.val == 0 {
                    c.val = 2;
                }
            }
            _ => {
                if c.fx == 0 {
                    c.fx = FX_T;
                }
                c.val = (c.val as i32 + d).clamp(0, 99) as u8;
            }
        }
        self.p.set_cell(self.pattern(), self.cur_row, self.cur_track, c);
    }

    fn clear_field(&mut self) {
        let mut c = self.cell_at_cursor();
        match self.cur_field {
            0 => c.note = if c.note == NOTE_EMPTY { NOTE_OFF } else { NOTE_EMPTY },
            1 => c.vol = 0,
            _ => {
                c.fx = 0;
                c.val = 0;
            }
        }
        self.p.set_cell(self.pattern(), self.cur_row, self.cur_track, c);
    }

    fn audition(&self, note: u8) {
        if (1..=128).contains(&note) {
            self.p.audition.store(((self.cur_track as u32) << 8) | (note as u32 - 1), Ordering::Relaxed);
        }
    }

    /// A pad pressed in the editor: write that note here, step down.
    fn enter_note(&mut self, pad: usize) {
        // chromatic, bottom-left = C of the edit octave
        let pos = (3 - pad / 4) * 4 + pad % 4;
        let note = (self.edit_octave * 12 + 12 + pos).min(127) as u8;
        let mut c = self.cell_at_cursor();
        c.note = note + 1;
        self.p.set_cell(self.pattern(), self.cur_row, self.cur_track, c);
        self.audition(note + 1);
        let len = self.p.len(self.pattern());
        self.cur_row = (self.cur_row + self.step_add) % len;
    }

    fn editor_tick(&mut self, input: &Input) {
        let len = self.p.len(self.pattern());
        let rows = input.knob1 + input.navigation_steps + if input.nav_down { 1 } else { 0 } - if input.nav_up { 1 } else { 0 };
        if rows != 0 {
            self.cur_row = (self.cur_row as i32 + rows).rem_euclid(len as i32) as usize;
        }
        if input.knob1_press {
            self.cur_field += 1;
            if self.cur_field >= FIELDS {
                self.cur_field = 0;
                self.cur_track = (self.cur_track + 1) % TRACKS;
            }
        }
        self.edit_field(input.knob2);
        if input.knob2_press {
            self.clear_field();
        }
        for i in 0..16 {
            if input.grid[i] && !self.prev_grid[i] {
                self.enter_note(i);
            }
        }
        self.prev_grid = input.grid;
    }

    fn perform_pads(&mut self, grid: &[bool; 16]) {
        for i in 0..16 {
            let (now, was) = (grid[i], self.prev_grid[i]);
            if i < TRACKS {
                if now && !was {
                    self.p.tracks[i].muted.fetch_xor(true, Ordering::Relaxed);
                }
            } else {
                self.p.moves[i - TRACKS].store(now, Ordering::Relaxed);
            }
        }
        self.prev_grid = *grid;
    }

    /// The visible window of pattern rows, as text: (row number, cells).
    pub fn pattern_lines(&self, first_track: usize, tracks: usize, rows: usize) -> Vec<(usize, String)> {
        let pat = self.pattern();
        let len = self.p.len(pat);
        let top = self.cur_row.saturating_sub(rows / 2).min(len.saturating_sub(rows));
        (top..(top + rows).min(len))
            .map(|r| {
                let cells: Vec<String> = (first_track..(first_track + tracks).min(TRACKS)).map(|t| self.p.cell(pat, r, t).text()).collect();
                (r, cells.join(" | "))
            })
            .collect()
    }

    fn first_visible_track(&self) -> usize {
        (self.cur_track / 4) * 4
    }

    // --------------------------------------------------------- save

    fn autosave(&mut self) {
        let Some(path) = self.save_path.clone() else { return };
        let dirty = self.p.dirty.load(Ordering::Relaxed);
        if dirty == self.saved {
            return;
        }
        if dirty != self.seen {
            self.seen = dirty;
            self.quiet = 0;
            return;
        }
        self.quiet += 1;
        if self.quiet < 30 {
            return;
        }
        self.saved = dirty;
        let mut patterns = Vec::new();
        for pat in 0..PATTERNS {
            let mut cells = Vec::new();
            for r in 0..ROWS {
                for t in 0..TRACKS {
                    let c = self.p.cell(pat, r, t);
                    if !c.is_empty() {
                        cells.push(serde_json::json!([r, t, c.note, c.vol, c.fx, c.val]));
                    }
                }
            }
            patterns.push(serde_json::json!({ "length": self.p.len(pat), "cells": cells }));
        }
        let tracks: Vec<serde_json::Value> = self
            .p
            .tracks
            .iter()
            .map(|t| {
                serde_json::json!({
                    "engine": Self::engine_name(t.engine.load(Ordering::Relaxed)),
                    "harmonics": t.harmonics.get(), "timbre": t.timbre.get(), "morph": t.morph.get(),
                    "decay": t.decay.get(), "level": t.level.get(),
                })
            })
            .collect();
        let doc = serde_json::json!({ "tempo": self.p.bpm.get(), "swing": self.p.swing.get(), "tracks": tracks, "patterns": patterns });
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, serde_json::to_string(&doc).unwrap_or_default()) {
            eprintln!("Ledger: couldn't save: {e}");
        }
    }

    /// Loads the saved project; false if there isn't one.
    fn load(&mut self) -> bool {
        let Some(path) = &self.save_path else { return false };
        let Ok(text) = std::fs::read_to_string(path) else { return false };
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
        let p = &self.p;
        if let Some(b) = doc["tempo"].as_f64() {
            p.bpm.set((b as f32).clamp(40.0, 240.0));
        }
        if let Some(s) = doc["swing"].as_f64() {
            p.swing.set((s as f32).clamp(0.0, 0.75));
        }
        for (t, v) in p.tracks.iter().zip(doc["tracks"].as_array().into_iter().flatten()) {
            if let Some(i) = crate::apps::plaits::ENGINE_NAMES.iter().position(|e| Some(*e) == v["engine"].as_str()) {
                t.engine.store(i as u32, Ordering::Relaxed);
            }
            for (a, k) in [(&t.harmonics, "harmonics"), (&t.timbre, "timbre"), (&t.morph, "morph"), (&t.decay, "decay"), (&t.level, "level")] {
                if let Some(x) = v[k].as_f64() {
                    a.set((x as f32).clamp(0.0, 1.0));
                }
            }
        }
        for (pi, pv) in doc["patterns"].as_array().into_iter().flatten().enumerate().take(PATTERNS) {
            if let Some(l) = pv["length"].as_u64() {
                p.length[pi].store((l as usize).clamp(1, ROWS), Ordering::Relaxed);
            }
            for c in pv["cells"].as_array().into_iter().flatten() {
                let f = |i: usize| c[i].as_u64().unwrap_or(0) as usize;
                let cell = Cell { note: f(2) as u8, vol: (f(3) as u8).min(100), fx: (f(4) as u8) % FX.len() as u8, val: (f(5) as u8).min(99) };
                p.set_cell(pi, f(0), f(1), cell);
            }
        }
        true
    }
}

impl PlayHost for LedgerApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS.len()
    }
    fn kit_label(&self, i: usize) -> String {
        self.label(i % CONTROLS.len())
    }
    fn kit_value(&self, i: usize) -> String {
        self.value(i % CONTROLS.len())
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        let u = |v: usize, n: usize| v.min(n - 1) as f32 / (n - 1).max(1) as f32;
        Some(match i {
            C_TRACK => u(self.cur_track, TRACKS),
            C_PATTERN => u(self.pattern(), PATTERNS),
            C_LENGTH => u(self.p.len(self.pattern()) - 1, ROWS),
            C_OCTAVE => u(self.edit_octave, 9),
            C_STEP => u(self.step_add, 17),
            C_NEXT => u(self.p.queued.load(Ordering::Relaxed), PATTERNS),
            C_CLEAR => 0.0,
            _ => return self.knob(i).norm(),
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        matches!(i, C_TRACK | C_ENGINE | C_PATTERN | C_LENGTH | C_OCTAVE | C_STEP | C_RUN | C_NEXT | C_CLEAR)
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit(i, delta);
    }
    /// Knob 2 press resets; on Clear Pattern it clears.
    fn kit_reset(&mut self, i: usize) {
        let t = self.track();
        match i {
            C_TIMBRE | C_MORPH | C_HARM => self.knob(i).set(0.5),
            C_DECAY => t.decay.set(0.5),
            C_LEVEL => t.level.set(0.7),
            C_TEMPO => self.p.bpm.set(118.0),
            C_SWING => self.p.swing.set(0.0),
            C_CLEAR => self.clear_pattern(),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let pick = |n: usize| (v.clamp(0.0, 1.0) * (n - 1) as f32).round() as usize;
        match i {
            C_TRACK => self.cur_track = pick(TRACKS),
            C_PATTERN => {
                let n = pick(PATTERNS);
                self.p.pattern.store(n, Ordering::Relaxed);
                self.p.queued.store(n, Ordering::Relaxed);
            }
            C_LENGTH => self.p.length[self.pattern()].store(pick(ROWS) + 1, Ordering::Relaxed),
            C_OCTAVE => self.edit_octave = pick(9),
            C_STEP => self.step_add = pick(17),
            C_NEXT => self.p.queued.store(pick(PATTERNS), Ordering::Relaxed),
            C_CLEAR => {}
            _ => self.knob(i).set(v),
        }
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        if pad < TRACKS {
            format!("{} {}", pad + 1, &Self::engine_name(self.p.tracks[pad].engine.load(Ordering::Relaxed)).chars().take(6).collect::<String>())
        } else {
            MOVES[pad - TRACKS].into()
        }
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, _held: bool) -> PadColor {
        if pad < TRACKS {
            let t = &self.p.tracks[pad];
            if t.muted.load(Ordering::Relaxed) {
                PadColor::Red
            } else if t.lit.load(Ordering::Relaxed) {
                PadColor::Yellow
            } else {
                PadColor::Green
            }
        } else if self.p.moves[pad - TRACKS].load(Ordering::Relaxed) {
            PadColor::Yellow
        } else {
            PadColor::Blue
        }
    }
    fn kit_line(&self) -> String {
        let moves: Vec<&str> = MOVES.iter().enumerate().filter(|(i, _)| self.p.moves[*i].load(Ordering::Relaxed)).map(|(_, m)| *m).collect();
        format!(
            "P{:02} row {:02}/{}  {}",
            self.p.playing_pattern.load(Ordering::Relaxed) + 1,
            self.p.row.load(Ordering::Relaxed),
            self.p.len(self.p.playing_pattern.load(Ordering::Relaxed)),
            if moves.is_empty() { String::new() } else { moves.join(" ") }
        )
    }
}

impl App for LedgerApp {
    fn play_surface(&self) -> bool {
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
        let was_menu = self.kit.menu;
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        if self.kit.menu != was_menu {
            // leaving or entering the editor lets go of held moves
            for m in &self.p.moves {
                m.store(false, Ordering::Relaxed);
            }
            self.prev_grid = input.grid;
        } else if self.kit.menu {
            self.editor_tick(&step.input);
        } else if step.native.is_some() {
            self.perform_pads(&step.input.grid);
        } else {
            self.prev_grid = [false; 16];
            for m in &self.p.moves {
                m.store(false, Ordering::Relaxed);
            }
        }
        self.autosave();
    }
    fn on_exit(&mut self) {
        for m in &self.p.moves {
            m.store(false, Ordering::Relaxed);
        }
        self.prev_grid = [false; 16];
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        let small = MonoTextStyle::new(&SPLEEN_6X12, INK);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        if !self.kit.menu {
            if let Some(col) = self.play_column() {
                let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
                kit::draw::column(f, &col, 16, 40, 350, 285, pal);
            }
            // a mini view of the playing rows
            let first = 0;
            let lines = self.pattern_lines(first, 2, 16);
            let playing = self.p.row.load(Ordering::Relaxed);
            for (i, (r, text)) in lines.iter().enumerate() {
                let style = if *r == playing && self.p.running.load(Ordering::Relaxed) { MonoTextStyle::new(&SPLEEN_6X12, ACCENT) } else { dim };
                Text::new(&format!("{r:02} {text}"), Point::new(380, 52 + i as i32 * 16), style).draw(f).ok();
            }
            Text::new("pads 1-8 mute, 9-16 moves   F3: play   R1: edit", Point::new(16, 340), dim).draw(f).ok();
            return;
        }
        // the editor: 4 tracks across, rows down
        let first = self.first_visible_track();
        let title = MonoTextStyle::new(&SPLEEN_8X16, ACCENT);
        Text::new(&format!("{APP_NAME}  P{:02}  oct {}  step {}", self.pattern() + 1, self.edit_octave, self.step_add), Point::new(16, 30), title).draw(f).ok();
        for t in first..first + 4 {
            let x = 52 + (t - first) as i32 * 144;
            let tr = &self.p.tracks[t];
            let style = if t == self.cur_track { small } else { dim };
            let name = format!("{} {}{}", t + 1, Self::engine_name(tr.engine.load(Ordering::Relaxed)), if tr.muted.load(Ordering::Relaxed) { " (m)" } else { "" });
            Text::new(&name, Point::new(x, 50), style).draw(f).ok();
        }
        let lines = self.pattern_lines(first, 4, 17);
        let playing = self.p.row.load(Ordering::Relaxed);
        let running = self.p.running.load(Ordering::Relaxed) && self.p.playing_pattern.load(Ordering::Relaxed) == self.pattern();
        for (i, (r, text)) in lines.iter().enumerate() {
            let y = 68 + i as i32 * 15;
            if *r == self.cur_row {
                Rectangle::new(Point::new(14, y - 11), Size::new(612, 14)).into_styled(PrimitiveStyle::with_fill(FAINT)).draw(f).ok();
                // the field under the cursor
                let fx = 52 + (self.cur_track - first) as i32 * 144 + [0, 4, 7, 7][self.cur_field] * 6;
                let w = [3, 2, 1, 3][self.cur_field] as u32 * 6;
                let fx = if self.cur_field == 3 { fx + 6 } else { fx };
                let w = if self.cur_field == 3 { 12 } else { w };
                Rectangle::new(Point::new(fx - 1, y - 11), Size::new(w + 2, 14)).into_styled(PrimitiveStyle::with_fill(CURSOR)).draw(f).ok();
            }
            let beat = r % 4 == 0;
            let style = if running && *r == playing { MonoTextStyle::new(&SPLEEN_6X12, ACCENT) } else if beat { small } else { dim };
            Text::new(&format!("{r:02}"), Point::new(16, y), style).draw(f).ok();
            for (k, cell) in text.split(" | ").enumerate() {
                Text::new(cell, Point::new(52 + k as i32 * 144, y), style).draw(f).ok();
            }
        }
        let fxhelp = match self.cur_field {
            2 | 3 => format!("FX {}: {}", FX[self.cell_at_cursor().fx as usize % FX.len()].0, FX[self.cell_at_cursor().fx as usize % FX.len()].1),
            _ => "pads: notes  knob1: rows (press: field)  knob2: value (press: clear)".into(),
        };
        Text::new(&fxhelp, Point::new(16, 340), dim).draw(f).ok();
    }
    /// The editor's list: every row of the pattern for the focused track
    /// (the grid panel shows four tracks across); the cursor is the
    /// selection.
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        if !self.kit.menu {
            return CONTROLS.iter().enumerate().map(|(i, _)| (self.label(i), self.value(i), false)).collect();
        }
        let pat = self.pattern();
        (0..self.p.len(pat)).map(|r| (format!("{r:02}{}", if r % 4 == 0 { " ·" } else { "" }), self.p.cell(pat, r, self.cur_track).text(), false)).collect()
    }
    fn slint_selected(&self) -> usize {
        if self.kit.menu { self.cur_row } else { 0 }
    }
    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let first = self.first_visible_track();
        let pat = self.pattern();
        let playing = self.p.row.load(Ordering::Relaxed);
        let running = self.p.running.load(Ordering::Relaxed) && self.p.playing_pattern.load(Ordering::Relaxed) == pat;
        // columns: row number, then note / vol / fx for four tracks
        let mut cells: Vec<String> = vec![String::new()];
        for t in first..first + 4 {
            let tr = &self.p.tracks[t];
            let name: String = Self::engine_name(tr.engine.load(Ordering::Relaxed)).chars().take(9).collect();
            cells.push(format!("{}{} {}", t + 1, if tr.muted.load(Ordering::Relaxed) { "m" } else { "" }, name));
            cells.push(String::new());
            cells.push(String::new());
        }
        let len = self.p.len(pat);
        let window = 21;
        let focus_row = if self.kit.menu { self.cur_row } else { playing };
        let top = focus_row.saturating_sub(window / 2).min(len.saturating_sub(window));
        let mut highlight = -1;
        for (i, r) in (top..(top + window).min(len)).enumerate() {
            let mark = if running && r == playing { "▶" } else if r % 4 == 0 { "·" } else { "" };
            cells.push(format!("{r:02}{mark}"));
            for t in first..first + 4 {
                let c = self.p.cell(pat, r, t);
                let text = c.text();
                let mut parts = text.split(' ');
                for _ in 0..3 {
                    cells.push(parts.next().unwrap_or("").to_string());
                }
            }
            if r == focus_row {
                highlight = (i + 1) as i32;
            }
        }
        let mut col_x = vec![0.0f32];
        for t in 0..4 {
            let x = 22.0 + t as f32 * 70.0;
            col_x.extend([x, x + 22.0, x + 38.0]);
        }
        crate::app::SlintExtra::Grid(crate::app::GridExtra {
            caption: format!("TRACKER / PATTERN {:02} / {:.0} BPM", self.pattern() + 1, self.p.bpm.get()),
            title: if self.kit.menu { format!("Track {} · {}", self.cur_track + 1, ["note", "volume", "effect", "effect value"][self.cur_field]) } else { self.kit_line() },
            cells,
            col_x,
            highlight,
            footer: if self.kit.menu {
                "knob1 rows (press: next field)  knob2 value (press: clear)\npads enter notes  T M H D R P N effects".into()
            } else {
                "pads 1-8 mute  9-16: LOOP4 LOOP2 LOOP1 REV OCT+ OCT- HALF DARK".into()
            },
            meter: -1.0,
        })
    }
    fn running(&self) -> Option<bool> {
        Some(self.p.running.load(Ordering::Relaxed))
    }
    fn toggle_running(&mut self) {
        let on = !self.p.running.load(Ordering::Relaxed);
        self.set_running(on);
    }
    fn needs_background_audio(&self) -> bool {
        self.p.running.load(Ordering::Relaxed)
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(Processor::new(Arc::clone(&self.p))))
    }
}

// ------------------------------------------------------------------ DSP

/// What a track does at a sample offset within the block.
#[derive(Clone, Copy)]
enum Ev {
    GateOff,
    On { note: u8 },
    Off,
}

struct TrackDsp {
    voice: PlaitsVoice,
    note: u8,
    gate: bool,
    vol: f32,
    /// This note's parameter locks (NaN = track setting).
    locks: [f32; 4],
    /// Pending events: (samples from now, event).
    pending: [(i64, Option<Ev>); 12],
    /// Seconds since the note ended (render stops after the tail).
    idle: f32,
}

impl TrackDsp {
    fn new() -> Self {
        Self { voice: PlaitsVoice::new(), note: 60, gate: false, vol: 1.0, locks: [f32::NAN; 4], pending: [(0, None); 12], idle: 10.0 }
    }
    fn schedule(&mut self, at: i64, e: Ev) {
        if let Some(slot) = self.pending.iter_mut().find(|p| p.1.is_none()) {
            *slot = (at, Some(e));
        }
    }
}

const RETRIG_GAP: i64 = 48;
const TAIL_S: f32 = 5.0;

struct Processor {
    p: Arc<Shared>,
    tracks: Vec<TrackDsp>,
    /// Samples until the next row.
    until_row: f64,
    /// Row index the next row tick will play (pattern position).
    next_row: usize,
    /// Rows played in total (for swing parity and loop moves).
    ticks: u64,
    was_running: bool,
    rng: u32,
    buf: Vec<f32>,
    mix: Vec<f32>,
    loop_anchor: Option<usize>,
}

impl Processor {
    fn new(p: Arc<Shared>) -> Self {
        Self { p, tracks: (0..TRACKS).map(|_| TrackDsp::new()).collect(), until_row: 0.0, next_row: 0, ticks: 0, was_running: false, rng: 0x1234_abcd, buf: vec![0.0; 4096], mix: vec![0.0; 4096], loop_anchor: None }
    }

    fn rand100(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng % 100
    }

    /// Plays row `row` of pattern `pat` into the tracks' event queues.
    fn play_row(&mut self, pat: usize, row: usize, row_samples: f64) {
        let transpose = if self.p.moves[4].load(Ordering::Relaxed) { 12 } else { 0 } - if self.p.moves[5].load(Ordering::Relaxed) { 12 } else { 0 };
        self.p.row.store(row, Ordering::Relaxed);
        self.p.playing_pattern.store(pat, Ordering::Relaxed);
        for t in 0..TRACKS {
            let c = self.p.cell(pat, row, t);
            self.p.tracks[t].lit.store(false, Ordering::Relaxed);
            if c.note == NOTE_EMPTY {
                continue;
            }
            if c.fx == FX_P && self.rand100() >= c.val as u32 {
                continue;
            }
            let delay = if c.fx == FX_N { (row_samples * c.val.min(7) as f64 / 8.0) as i64 } else { 0 };
            let tr = &mut self.tracks[t];
            if c.note == NOTE_OFF {
                tr.schedule(delay, Ev::Off);
                continue;
            }
            if self.p.tracks[t].muted.load(Ordering::Relaxed) {
                continue;
            }
            self.p.tracks[t].lit.store(true, Ordering::Relaxed);
            let note = ((c.note - 1) as i32 + transpose).clamp(0, 127) as u8;
            tr.vol = if c.vol == 0 { 1.0 } else { (c.vol - 1) as f32 / 99.0 };
            tr.locks = [f32::NAN; 4];
            if (FX_T..=FX_D).contains(&c.fx) {
                tr.locks[(c.fx - FX_T) as usize] = c.val as f32 / 99.0;
            }
            let hits = if c.fx == FX_R { (c.val as i64).clamp(1, 8) } else { 1 };
            let spacing = row_samples as i64 / hits;
            for h in 0..hits {
                let at = delay + h * spacing;
                tr.schedule(at, Ev::GateOff);
                tr.schedule(at + RETRIG_GAP, Ev::On { note });
            }
        }
    }

    /// The row after `row`, given the held performance moves.
    fn advance(&mut self, row: usize, len: usize) -> usize {
        let m = |i: usize| self.p.moves[i].load(Ordering::Relaxed);
        let span = if m(0) { Some(4) } else if m(1) { Some(2) } else if m(2) { Some(1) } else { None };
        if let Some(span) = span {
            let anchor = *self.loop_anchor.get_or_insert(row - row % span);
            let next = row + 1;
            return if next >= anchor + span || next >= len { anchor } else { next };
        }
        self.loop_anchor = None;
        if m(3) {
            (row + len - 1) % len
        } else {
            row + 1
        }
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        if channels == 0 || rate < 1000.0 {
            return;
        }
        let frames = out.len() / channels;
        if frames > self.mix.len() {
            return;
        }
        let running = self.p.running.load(Ordering::Relaxed);
        if running && !self.was_running {
            self.next_row = 0;
            self.until_row = 0.0;
            self.ticks = 0;
            self.loop_anchor = None;
            let q = self.p.pattern.load(Ordering::Relaxed);
            self.p.playing_pattern.store(q, Ordering::Relaxed);
        }
        if !running && self.was_running {
            for t in self.tracks.iter_mut() {
                t.schedule(0, Ev::Off);
            }
            for t in &self.p.tracks {
                t.lit.store(false, Ordering::Relaxed);
            }
        }
        self.was_running = running;
        // editor audition
        let a = self.p.audition.swap(u32::MAX, Ordering::Relaxed);
        if a != u32::MAX {
            let tr = &mut self.tracks[(a >> 8) as usize % TRACKS];
            tr.vol = 0.9;
            tr.locks = [f32::NAN; 4];
            tr.schedule(0, Ev::GateOff);
            tr.schedule(RETRIG_GAP, Ev::On { note: (a & 0x7f) as u8 });
        }
        let half = self.p.moves[6].load(Ordering::Relaxed);
        let bpm = self.p.bpm.get().clamp(40.0, 240.0) as f64 * if half { 0.5 } else { 1.0 };
        let row_samples = rate as f64 * 60.0 / bpm / 4.0;
        let swing = self.p.swing.get().clamp(0.0, 0.75) as f64;
        let dark = self.p.moves[7].load(Ordering::Relaxed);
        let level = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0);
        self.mix[..frames].fill(0.0);

        let mut done = 0usize;
        while done < frames {
            // fire rows that are due now
            if running && self.until_row <= 0.0 {
                let mut pat = self.p.playing_pattern.load(Ordering::Relaxed) % PATTERNS;
                let mut len = self.p.len(pat);
                if self.next_row >= len {
                    // pattern end: the queued pattern takes over
                    pat = self.p.queued.load(Ordering::Relaxed) % PATTERNS;
                    self.p.pattern.store(pat, Ordering::Relaxed);
                    len = self.p.len(pat);
                    self.next_row = 0;
                }
                let row = self.next_row;
                // swing: odd rows late, even rows early by the same amount
                let this_len = row_samples * if self.ticks % 2 == 0 { 1.0 + swing * 0.5 } else { 1.0 - swing * 0.5 };
                self.play_row(pat, row, this_len);
                self.ticks += 1;
                self.until_row += this_len;
                self.next_row = self.advance(row, len);
            }
            // render up to the next event
            let mut chunk = frames - done;
            if running {
                chunk = chunk.min(self.until_row.ceil().max(1.0) as usize);
            }
            for tr in &self.tracks {
                for (at, e) in tr.pending.iter() {
                    if e.is_some() && *at > 0 {
                        chunk = chunk.min(*at as usize);
                    }
                }
            }
            let chunk = chunk.max(1);
            // apply events due now
            for tr in self.tracks.iter_mut() {
                for slot in tr.pending.iter_mut() {
                    if let (at, Some(e)) = *slot {
                        if at <= 0 {
                            match e {
                                Ev::GateOff => tr.gate = false,
                                Ev::On { note } => {
                                    tr.note = note;
                                    tr.gate = true;
                                    tr.idle = 0.0;
                                }
                                Ev::Off => tr.gate = false,
                            }
                            *slot = (0, None);
                        }
                    }
                }
            }
            let dt = chunk as f32 / rate;
            for (ti, tr) in self.tracks.iter_mut().enumerate() {
                if !tr.gate {
                    tr.idle += dt;
                }
                if !tr.gate && tr.idle > TAIL_S {
                    continue;
                }
                let s = &self.p.tracks[ti];
                let lock = |i: usize, v: f32| if tr.locks[i].is_nan() { v } else { tr.locks[i] };
                let timbre = lock(0, s.timbre.get()) * if dark { 0.3 } else { 1.0 };
                let params = PlaitsParams {
                    engine: s.engine.load(Ordering::Relaxed) as i32,
                    note: tr.note as f32,
                    harmonics: lock(2, s.harmonics.get()),
                    timbre,
                    morph: lock(1, s.morph.get()) * if dark { 0.5 } else { 1.0 },
                    decay: lock(3, s.decay.get()),
                    lpg_colour: if dark { 0.1 } else { 0.5 },
                    trigger: tr.gate,
                };
                tr.voice.render(&mut self.buf[..chunk], rate, &params);
                let g = s.level.get() * tr.vol * 0.4;
                for (m, x) in self.mix[done..done + chunk].iter_mut().zip(&self.buf[..chunk]) {
                    *m += x * g;
                }
            }
            for tr in self.tracks.iter_mut() {
                for slot in tr.pending.iter_mut() {
                    if slot.1.is_some() {
                        slot.0 -= chunk as i64;
                    }
                }
            }
            if running {
                self.until_row -= chunk as f64;
            }
            done += chunk;
        }
        for (frame, m) in out.chunks_mut(channels).zip(&self.mix[..frames]) {
            let y = (m * level).tanh();
            for c in frame.iter_mut() {
                *c += y;
            }
        }
        if let Ok(mut b) = self.p.output.try_lock() {
            b.clear();
            b.extend(self.mix[..frames].iter().map(|m| (m * level).tanh()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> LedgerApp {
        LedgerApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
    }

    fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for _ in 0..blocks {
            let mut out = vec![0.0f32; 512];
            p.process(&mut out, 2, 48_000.0);
            assert!(out.iter().all(|v| v.is_finite()));
            all.extend(out.iter().step_by(2));
        }
        all
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn cells_pack_and_print_like_a_tracker() {
        let c = Cell { note: 61, vol: 65, fx: FX_R, val: 3 };
        assert_eq!(Cell::unpack(c.pack()), c);
        assert_eq!(c.text(), "C-4 64 R03");
        assert_eq!(Cell { note: NOTE_OFF, ..Default::default() }.text(), "OFF -- ---");
    }

    #[test]
    fn the_demo_pattern_plays_in_time_and_stops_quietly() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        assert!(rms(&render(&mut p, 20)) < 1e-4, "stopped: silent");
        a.toggle_running();
        let mut rows = std::collections::BTreeSet::new();
        let mut x = Vec::new();
        for _ in 0..400 {
            x.extend(render(&mut p, 1));
            rows.insert(a.p.row.load(Ordering::Relaxed));
        }
        assert!(rms(&x) > 0.01, "the demo pattern sounds ({})", rms(&x));
        assert_eq!(rows.len(), 16, "all 16 rows played in ~2 s at 118 BPM: {rows:?}");
        a.toggle_running();
        render(&mut p, 5);
        let tail = render(&mut p, 1200);
        assert!(rms(&tail[tail.len() - 4800..]) < 0.005, "and decays after stop");
    }

    #[test]
    fn rows_land_on_the_beat_grid() {
        let mut a = app();
        a.clear_pattern();
        a.p.bpm.set(120.0); // a row = 125 ms = 6000 samples
        a.p.set_cell(0, 0, 0, Cell { note: 37, ..Default::default() });
        a.p.set_cell(0, 8, 0, Cell { note: 37, ..Default::default() });
        let mut p = a.audio_processor().unwrap();
        a.toggle_running();
        let x = render(&mut p, 200); // 1.07 s
        let onset = |from: usize| (from..x.len()).find(|&i| x[i].abs() > 0.02).unwrap_or(usize::MAX);
        let first = onset(0);
        let second = onset(first + 24_000);
        assert!(first < 600, "first hit at the start ({first})");
        assert!((second as i64 - first as i64 - 48_000).abs() < 900, "row 8 lands 1 s later ({})", second - first);
    }

    #[test]
    fn mute_and_the_loop_move() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.toggle_running();
        let pad = |i: usize| Input { grid: std::array::from_fn(|k| k == i), ..Default::default() };
        a.tick(&pad(0));
        a.tick(&Input::default());
        assert!(a.p.tracks[0].muted.load(Ordering::Relaxed), "pad 1 mutes track 1");
        a.tick(&pad(10)); // LOOP 1
        render(&mut p, 100);
        let r1 = a.p.row.load(Ordering::Relaxed);
        render(&mut p, 100);
        assert_eq!(a.p.row.load(Ordering::Relaxed), r1, "LOOP 1 holds the row");
        a.tick(&Input::default());
        render(&mut p, 100);
        assert_ne!(a.p.row.load(Ordering::Relaxed), r1, "letting go moves on");
    }

    #[test]
    fn the_editor_enters_notes_and_effects() {
        let mut a = app();
        a.clear_pattern();
        a.kit.menu = true;
        // pad bottom-left = C of edit octave 4 -> C-4 (MIDI 60)
        a.tick(&Input { grid: std::array::from_fn(|k| k == 12), ..Default::default() });
        assert_eq!(a.p.cell(0, 0, 0).note, 61);
        assert_eq!(a.cur_row, 1, "cursor steps down");
        a.tick(&Input::default());
        a.tick(&Input { knob1: -1, ..Default::default() });
        a.tick(&Input { knob1_press: true, ..Default::default() }); // -> vol
        a.tick(&Input { knob1_press: true, ..Default::default() }); // -> fx
        a.tick(&Input { knob2: 5, ..Default::default() });
        assert_eq!(a.p.cell(0, 0, 0).fx, FX_T, "fx type steps one at a time, however fast the knob");
        for _ in 0..4 {
            a.tick(&Input { knob2: 1, ..Default::default() });
        }
        assert_eq!(a.p.cell(0, 0, 0).fx, FX_R);
        a.tick(&Input { knob1_press: true, ..Default::default() }); // -> value
        a.tick(&Input { knob2: 2, ..Default::default() });
        assert_eq!(a.p.cell(0, 0, 0).val, 4);
        assert_eq!(a.p.cell(0, 0, 0).text(), "C-4 -- R04");
        a.tick(&Input { knob1_press: true, ..Default::default() }); // -> next track's note
        assert_eq!((a.cur_track, a.cur_field), (1, 0));
        a.tick(&Input { knob2_press: true, ..Default::default() });
        assert_eq!(a.p.cell(0, 0, 1).note, NOTE_OFF, "clearing an empty note writes OFF");
    }
}
