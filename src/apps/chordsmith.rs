//! Chordsmith: a whole chord under one finger, played on any instrument.
//!
//! The bottom two rows of pads are the chords of the key: I ii iii IV
//! V vi vii° and the borrowed bVII rock players love. Hold modifier pads
//! on the top two rows while pressing one to change it:
//!   row 1: 7th, sus4, add9, flip (major <-> minor)
//!   row 2: 6th, power (root and fifth), bass on the fifth, wide voicing
//!
//! Each new chord is voice-led from the last: of every inversion in the
//! range, it picks the one whose notes move least, the way a pianist
//! would, so a progression sounds connected rather than jumping. A bass
//! note an octave or two below follows the root (or the fifth).
//!
//! Five ways to play it (left/right on Style): Pad (held), Strum (notes
//! spread a few milliseconds apart, low to high), Arp up, Arp up-down,
//! and Pulse (the chord repeated in eighth notes). The strums and
//! arpeggios are timed on the audio thread to the sample.
//!
//! "Plays" sends the notes to any instrument app (Plaits, Voltage,
//! Atlas, Timbre Map...) or Chordsmith's own sounds.

use crate::app::{App, Input, SlintExtra};
use crate::apps::kids_kit::{self as kit, Ev, Extra, Note, Size2, Sound, Tone};
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::note_bus::{NoteBus, NoteOut, NoteRoute, INTERNAL};
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Chordsmith";
const NUMERALS: [&str; 8] = ["I", "ii", "iii", "IV", "V", "vi", "vii°", "bVII"];
const MOD_NAMES: [&str; 8] = ["7th", "sus4", "add9", "flip", "6th", "power", "5 bass", "wide"];
const STYLES: [&str; 5] = ["Pad", "Strum", "Arp up", "Arp up-down", "Pulse"];
const TONES: [Tone; 5] = [Tone::Pluck, Tone::Soft, Tone::Organ, Tone::Marimba, Tone::Glock];
/// Voicings sit in this range, around middle C.
const LOW: i32 = 52;
const HIGH: i32 = 79;

/// The chord a degree pad plays with the modifiers held: its root (MIDI
/// pitch class plus octave from C) and its intervals above the root.
pub fn build(key: i32, minor_key: bool, degree: usize, mods: [bool; 8]) -> (i32, Vec<i32>) {
    let scale: &[i32] = if minor_key { &kit::MINOR_SCALE } else { &kit::MAJOR };
    let (root, mut third, mut fifth) = if degree == 7 {
        // bVII: a major chord a whole tone below the tonic.
        (key + 10, 4, 7)
    } else {
        let r = kit::scale_note(0, scale, degree as i32);
        let t = kit::scale_note(0, scale, degree as i32 + 2) - r;
        let f = kit::scale_note(0, scale, degree as i32 + 4) - r;
        (key + r, t, f)
    };
    if mods[3] {
        third = if third == 4 { 3 } else { 4 };
        if fifth == 6 {
            fifth = 7;
        }
    }
    let mut iv = vec![0];
    if mods[5] {
        iv.push(7);
    } else {
        iv.push(if mods[1] { 5 } else { third });
        iv.push(fifth);
        if mods[0] {
            // The scale's own seventh (maj7 on I and IV, b7 on V).
            let seventh = if degree == 7 { 10 } else { kit::scale_note(0, scale, degree as i32 + 6) - kit::scale_note(0, scale, degree as i32) };
            iv.push(seventh.rem_euclid(12));
        }
        if mods[4] {
            iv.push(9);
        }
        if mods[2] {
            iv.push(14);
        }
    }
    (root.rem_euclid(12), iv)
}

/// The chord's name, e.g. "Am7", "Gsus4", "F(add9)".
pub fn name(root: i32, iv: &[i32], mods: [bool; 8]) -> String {
    let r = kit::note_name(root);
    if mods[5] {
        return format!("{r}5");
    }
    let third = iv.get(1).copied().unwrap_or(4);
    let fifth = iv.get(2).copied().unwrap_or(7);
    let mut s = match (third, fifth) {
        (5, _) => format!("{r}sus4"),
        (3, 6) => format!("{r}dim"),
        (3, _) => format!("{r}m"),
        _ => r.to_string(),
    };
    if let Some(&sev) = iv.iter().find(|&&i| i == 10 || i == 11) {
        s.push_str(if sev == 11 { "maj7" } else { "7" });
        if third == 3 && sev == 11 {
            s = s.replace("mmaj7", "m(maj7)");
        }
    }
    if iv.contains(&9) {
        s.push('6');
    }
    if iv.contains(&14) {
        s.push_str("(add9)");
    }
    s
}

/// Voice-leads `pcs` (pitch classes, root first) from `prev`: of all the
/// ways to place the notes in [LOW, HIGH], the one closest to the last
/// chord. With no last chord, root position from C4 up.
pub fn voice(pcs: &[i32], prev: &[i32], wide: bool) -> Vec<i32> {
    let n = pcs.len();
    if prev.is_empty() {
        let base = 60 + (pcs[0] - 60).rem_euclid(12) - if (pcs[0] - 60).rem_euclid(12) > 5 { 12 } else { 0 };
        let mut v = Vec::new();
        let mut last = base - 1;
        for &pc in pcs {
            let mut x = last + 1 + (pc - (last + 1)).rem_euclid(12);
            if v.is_empty() {
                x = base;
            }
            v.push(x);
            last = x;
        }
        return spread(v, wide);
    }
    // Every inversion, each at every octave that fits.
    let mut best: Option<(i32, Vec<i32>)> = None;
    for rot in 0..n {
        let order: Vec<i32> = (0..n).map(|k| pcs[(rot + k) % n]).collect();
        for start in LOW..=HIGH {
            if start.rem_euclid(12) != order[0].rem_euclid(12) {
                continue;
            }
            let mut v = vec![start];
            for &pc in &order[1..] {
                let last = *v.last().unwrap();
                v.push(last + 1 + (pc - (last + 1)).rem_euclid(12));
            }
            if *v.last().unwrap() > HIGH + 4 {
                continue;
            }
            let cost = movement(&v, prev);
            if best.as_ref().is_none_or(|b| cost < b.0) {
                best = Some((cost, v));
            }
        }
    }
    spread(best.map(|b| b.1).unwrap_or_default(), wide)
}

/// How far a voicing moves from the last: each note's distance to the
/// nearest note of the previous chord.
fn movement(v: &[i32], prev: &[i32]) -> i32 {
    v.iter().map(|&a| prev.iter().map(|&b| (a - b).abs()).min().unwrap_or(0)).sum::<i32>() + (v.len() as i32 - prev.len() as i32).abs() * 2
}

/// A wide voicing: every other note up an octave (drop-2 inverted).
fn spread(mut v: Vec<i32>, wide: bool) -> Vec<i32> {
    if wide && v.len() >= 3 {
        v[1] += 12;
        v.sort();
    }
    v
}

/// The chord being held, shared with the audio thread.
#[derive(Clone, Default)]
pub struct Held {
    pub notes: Vec<u8>,
    pub bass: Option<u8>,
    pub vel: u8,
    /// Bumped on every new chord, so the player knows to strike it.
    pub gen: u64,
    pub held: bool,
}

struct Shared {
    held: Mutex<Held>,
    style: AtomicUsize,
    tone: AtomicUsize,
    strum_ms: AtomicF32,
    tempo: AtomicF32,
    route: Arc<AtomicUsize>,
    played: AtomicU64,
}

enum Out {
    On(u8, u8),
    Off(u8),
}

/// Plays the held chord, timed to the sample: strums, arpeggios, pulses.
struct Player {
    s: Arc<Shared>,
    notes: NoteOut,
    /// (samples from now, what to do)
    queue: Vec<(u32, Out)>,
    sounding: Vec<u8>,
    gen: u64,
    chord: Held,
    arp_i: usize,
    arp_dir: i32,
    until: u32,
    sr: f32,
}

impl Player {
    fn release_all(&mut self) {
        for n in std::mem::take(&mut self.sounding) {
            self.queue.push((0, Out::Off(n)));
        }
    }

    fn strike(&mut self, delay: u32) {
        let style = self.s.style.load(Ordering::Relaxed) % STYLES.len();
        let gap = (self.s.strum_ms.get() * 0.001 * self.sr) as u32;
        let mut all: Vec<u8> = self.chord.bass.into_iter().collect();
        all.extend(self.chord.notes.iter().copied());
        match style {
            0 | 4 => {
                for &n in &all {
                    self.queue.push((delay, Out::On(n, self.chord.vel)));
                }
            }
            1 => {
                for (k, &n) in all.iter().enumerate() {
                    self.queue.push((delay + k as u32 * gap, Out::On(n, self.chord.vel.saturating_sub(k as u8 * 4).max(20))));
                }
            }
            _ => {
                if let Some(b) = self.chord.bass {
                    self.queue.push((delay, Out::On(b, self.chord.vel)));
                }
            }
        }
    }

    fn eighth(&self) -> u32 {
        (60.0 / self.s.tempo.get().max(30.0) / 2.0 * self.sr) as u32
    }
}

impl Extra for Player {
    fn block(&mut self, frames: usize, sr: f32) {
        self.sr = sr;
        self.notes.advance(frames as u32);
        let Ok(h) = self.s.held.try_lock() else { return };
        if h.gen != self.gen {
            self.gen = h.gen;
            self.chord = h.clone();
            drop(h);
            self.release_all();
            self.queue.retain(|q| matches!(q.1, Out::Off(_)));
            self.arp_i = 0;
            self.arp_dir = 1;
            self.until = 0;
            if self.chord.held {
                self.strike(0);
                self.until = self.eighth();
            }
        } else if !h.held && self.chord.held {
            self.chord.held = false;
            drop(h);
            self.queue.retain(|q| matches!(q.1, Out::Off(_)));
            self.release_all();
        }
    }

    fn poll(&mut self, out: &mut Vec<Ev>) {
        // Arpeggios and pulses: one event an eighth note.
        let style = self.s.style.load(Ordering::Relaxed) % STYLES.len();
        if self.chord.held && style >= 2 && !self.chord.notes.is_empty() {
            if self.until == 0 {
                let notes = &self.chord.notes;
                if style == 4 {
                    let prev: Vec<u8> = self.sounding.iter().copied().filter(|n| notes.contains(n)).collect();
                    for n in prev {
                        self.queue.push((0, Out::Off(n)));
                    }
                    self.sounding.retain(|n| !notes.contains(n));
                    for &n in notes.clone().iter() {
                        self.queue.push((1, Out::On(n, self.chord.vel.saturating_sub(15))));
                    }
                } else {
                    let n = notes[self.arp_i.min(notes.len() - 1)];
                    let prev: Vec<u8> = self.sounding.iter().copied().filter(|x| notes.contains(x)).collect();
                    for p in prev {
                        self.queue.push((0, Out::Off(p)));
                    }
                    self.sounding.retain(|x| !notes.contains(x));
                    self.queue.push((1, Out::On(n, self.chord.vel)));
                    let len = notes.len() as i32;
                    if style == 2 {
                        self.arp_i = ((self.arp_i as i32 + 1) % len) as usize;
                    } else {
                        let next = self.arp_i as i32 + self.arp_dir;
                        if next < 0 || next >= len {
                            self.arp_dir = -self.arp_dir;
                        }
                        self.arp_i = (self.arp_i as i32 + self.arp_dir).clamp(0, len - 1) as usize;
                    }
                }
                self.until = self.eighth();
            } else {
                self.until -= 1;
            }
        }
        let internal = self.s.route.load(Ordering::Relaxed) == INTERNAL;
        let tone = TONES[self.s.tone.load(Ordering::Relaxed) % TONES.len()];
        let mut k = 0;
        while k < self.queue.len() {
            if self.queue[k].0 == 0 {
                let (_, o) = self.queue.remove(k);
                match o {
                    Out::On(n, v) => {
                        if internal {
                            out.push(Ev::Note(Note::new(tone, n as f32).vel(v as f32 / 127.0).held(n as u32).pan((n as f32 - 64.0) / 30.0)));
                        } else {
                            self.notes.note_on(n, v);
                        }
                        self.sounding.push(n);
                        self.s.played.fetch_add(1, Ordering::Relaxed);
                    }
                    Out::Off(n) => {
                        out.push(Ev::Off(n as u32));
                        self.notes.note_off(n);
                    }
                }
            } else {
                self.queue[k].0 -= 1;
                k += 1;
            }
        }
    }

    fn frame(&mut self, _sr: f32) -> (f32, f32) {
        (0.0, 0.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Plays,
    Sound,
    Key,
    Mode,
    Style,
    Strum,
    Tempo,
    Bass,
}
const ROWS: [Row; 8] = [Row::Plays, Row::Sound, Row::Key, Row::Mode, Row::Style, Row::Strum, Row::Tempo, Row::Bass];

pub struct Chordsmith {
    sound: Sound,
    s: Arc<Shared>,
    route: NoteRoute,
    notes: Option<NoteOut>,
    key: i32,
    minor: bool,
    bass: usize,
    row: usize,
    prev_pads: [bool; 16],
    voicing: Vec<i32>,
    pub current: Option<(usize, [bool; 8], String)>,
    history: Vec<String>,
}

impl Chordsmith {
    pub fn new(sound: Sound, notes: Option<Arc<NoteBus>>) -> Chordsmith {
        let (route, out) = NoteRoute::new(notes, NAME, "chordsmith", true);
        let s = Arc::new(Shared { held: Mutex::new(Held::default()), style: AtomicUsize::new(1), tone: AtomicUsize::new(0), strum_ms: AtomicF32::new(22.0), tempo: AtomicF32::new(100.0), route: out.route_handle(), played: AtomicU64::new(0) });
        sound.set_reverb(0.25);
        Chordsmith { sound, s, route, notes: Some(out), key: 0, minor: false, bass: 1, row: 0, prev_pads: [false; 16], voicing: Vec::new(), current: None, history: Vec::new() }
    }

    /// Plays degree `d` with `mods`.
    pub fn press(&mut self, d: usize, mods: [bool; 8]) {
        let (root, iv) = build(self.key, self.minor, d, mods);
        let pcs: Vec<i32> = iv.iter().map(|i| root + i).collect();
        let v = voice(&pcs, &self.voicing, mods[7]);
        self.voicing = v.clone();
        let label = name(root, &iv, mods);
        let bass_pc = if mods[6] { root + 7 } else { root };
        let bass = match self.bass {
            0 => None,
            b => Some((36 + (bass_pc - 36).rem_euclid(12) + if b == 2 { -12 } else { 0 }).clamp(24, 60) as u8),
        };
        let mut h = self.s.held.lock().unwrap();
        h.notes = v.iter().map(|&n| n.clamp(0, 127) as u8).collect();
        h.bass = bass;
        h.vel = 100;
        h.gen += 1;
        h.held = true;
        drop(h);
        if self.history.last() != Some(&label) {
            self.history.push(label.clone());
            if self.history.len() > 8 {
                self.history.remove(0);
            }
        }
        self.current = Some((d, mods, label));
    }

    pub fn release(&mut self) {
        self.s.held.lock().unwrap().held = false;
        self.current = None;
    }

    fn label(r: Row) -> &'static str {
        match r {
            Row::Plays => "Plays",
            Row::Sound => "Sound",
            Row::Key => "Key",
            Row::Mode => "Mode",
            Row::Style => "Style",
            Row::Strum => "Strum",
            Row::Tempo => "Tempo",
            Row::Bass => "Bass",
        }
    }

    fn value(&self, r: Row) -> String {
        match r {
            Row::Plays => self.route.label(),
            Row::Sound => TONES[self.s.tone.load(Ordering::Relaxed) % TONES.len()].name().into(),
            Row::Key => kit::note_name(self.key).into(),
            Row::Mode => if self.minor { "minor".into() } else { "major".into() },
            Row::Style => STYLES[self.s.style.load(Ordering::Relaxed) % STYLES.len()].into(),
            Row::Strum => format!("{:.0} ms", self.s.strum_ms.get()),
            Row::Tempo => format!("{:.0} bpm", self.s.tempo.get()),
            Row::Bass => ["off", "low", "lower"][self.bass].into(),
        }
    }

    fn numeral(&self, d: usize) -> String {
        if !self.minor {
            return NUMERALS[d].into();
        }
        ["i", "ii°", "III", "iv", "v", "VI", "VII", "bVII"][d].into()
    }
}

/// Pad index (row 0 top) -> degree, for the bottom two rows.
fn pad_degree(p: usize) -> Option<usize> {
    match p {
        8..=15 => Some(if p >= 12 { p - 12 } else { p - 4 }),
        _ => None,
    }
}

impl App for Chordsmith {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn needs_background_audio(&self) -> bool {
        self.route.external() && self.current.is_some()
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        ROWS.iter().map(|r| (Chordsmith::label(*r).to_string(), self.value(*r), false)).collect()
    }
    fn on_exit(&mut self) {
        self.release();
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        std::array::from_fn(|p| match pad_degree(p) {
            Some(d) if self.current.as_ref().is_some_and(|c| c.0 == d) => PadColor::Red,
            Some(0) | Some(2) | Some(5) => PadColor::Green,
            Some(1) | Some(3) => PadColor::Blue,
            Some(_) => PadColor::Yellow,
            None => if self.prev_pads[p] { PadColor::Red } else { PadColor::Off },
        })
    }

    fn tick(&mut self, input: &Input) {
        let mods: [bool; 8] = std::array::from_fn(|i| input.grid[i]);
        // The bottom row is degrees 1-4 (I..IV), the row above 5-8.
        let mut newest = None;
        for p in 8..16 {
            if input.grid[p] && !self.prev_pads[p] {
                newest = pad_degree(p);
            }
        }
        if let Some(d) = newest {
            self.press(d, mods);
        } else if let Some((d, m, _)) = self.current.clone() {
            // Changing modifiers while a chord is held changes the chord.
            if m != mods && (8..16).any(|p| input.grid[p] && pad_degree(p) == Some(d)) {
                self.press(d, mods);
            }
        }
        if self.current.is_some() && !(8..16).any(|p| input.grid[p]) && !(0..128).any(|n| input.midi_keys.0[n] > 0) {
            self.release();
        }
        self.prev_pads = input.grid;
        if input.navigation_steps != 0 {
            self.row = (self.row as i32 + input.navigation_steps).clamp(0, ROWS.len() as i32 - 1) as usize;
        }
        if input.knob2 != 0 {
            let d = input.knob2.signum();
            let bump = |a: &AtomicUsize, n: usize| a.store((a.load(Ordering::Relaxed) as i32 + d).rem_euclid(n as i32) as usize, Ordering::Relaxed);
            match ROWS[self.row] {
                Row::Plays => self.route.step(d),
                Row::Sound => bump(&self.s.tone, TONES.len()),
                Row::Key => {
                    self.key = (self.key + d).rem_euclid(12);
                    self.voicing.clear();
                }
                Row::Mode => self.minor = !self.minor,
                Row::Style => bump(&self.s.style, STYLES.len()),
                Row::Strum => self.s.strum_ms.set((self.s.strum_ms.get() + d as f32 * 4.0).clamp(4.0, 120.0)),
                Row::Tempo => self.s.tempo.set((self.s.tempo.get() + d as f32 * 2.0).clamp(40.0, 220.0)),
                Row::Bass => self.bass = (self.bass as i32 + d).rem_euclid(3) as usize,
            }
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = Rgb565::new(4, 4, 7);
        let panel = Rgb565::new(8, 8, 14);
        let dim = Rgb565::new(16, 26, 24);
        let ink = kit::rgb(240, 200, 120);
        kit::clear(fb, bg);
        kit::rect(fb, 0, 0, 640, 30, panel);
        kit::text(fb, NAME, 12, 7, Size2::Medium, kit::WHITE, -1);
        kit::text(fb, &format!("{} {}", kit::note_name(self.key), if self.minor { "minor" } else { "major" }), 630, 9, Size2::Small, dim, 1);
        // Settings.
        for (i, r) in ROWS.iter().enumerate() {
            let y = 38 + i as i32 * 28;
            let sel = i == self.row;
            kit::round_rect(fb, 8, y, 200, 25, 6, if sel { kit::blend(panel, kit::WHITE, 0.15) } else { panel });
            kit::text(fb, Chordsmith::label(*r), 16, y + 6, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
            let v: String = self.value(*r).chars().take(16).collect();
            kit::text(fb, &v, 200, y + 6, Size2::Small, ink, 1);
        }
        // The chord.
        kit::round_rect(fb, 218, 38, 414, 110, 12, panel);
        match &self.current {
            Some((d, _, label)) => {
                kit::text(fb, label, 330, 54, Size2::Huge, kit::WHITE, 0);
                kit::text(fb, &self.numeral(*d), 540, 64, Size2::Large, ink, 0);
            }
            None => {
                kit::text(fb, "-", 330, 54, Size2::Huge, dim, 0);
            }
        }
        // A keyboard with the voicing on it, C3 to C6.
        let (kx, ky) = (226, 156);
        let held = self.s.held.lock().unwrap().clone();
        let on = |n: i32| held.held && (held.notes.contains(&(n as u8)) || held.bass == Some(n as u8));
        let whites: Vec<i32> = (36..=84).filter(|n| ![1, 3, 6, 8, 10].contains(&(n % 12))).collect();
        let w = 398 / whites.len() as i32;
        for (k, &n) in whites.iter().enumerate() {
            kit::rect(fb, kx + k as i32 * w, ky, w - 1, 52, if on(n) { ink } else { kit::rgb(220, 220, 225) });
        }
        for (k, &n) in whites.iter().enumerate() {
            if [0, 2, 5, 7, 9].contains(&(n % 12)) && n < 84 {
                let b = n + 1;
                kit::rect(fb, kx + k as i32 * w + w * 2 / 3, ky, w * 2 / 3, 32, if on(b) { kit::rgb(200, 140, 60) } else { kit::rgb(30, 30, 36) });
            }
        }
        // The pads: modifiers above, chords below.
        for p in 0..16 {
            let x = 226 + (p as i32 % 4) * 100;
            let y = 218 + (p as i32 / 4) * 24;
            let (label, c) = match pad_degree(p) {
                Some(d) => {
                    let (root, iv) = build(self.key, self.minor, d, [false; 8]);
                    (format!("{} {}", self.numeral(d), name(root, &iv, [false; 8])), if self.current.as_ref().is_some_and(|c| c.0 == d) { ink } else { kit::blend(panel, kit::WHITE, 0.12) })
                }
                None => (MOD_NAMES[p].to_string(), if self.prev_pads[p] { kit::rgb(120, 170, 240) } else { panel }),
            };
            kit::round_rect(fb, x, y, 96, 21, 5, c);
            kit::text(fb, &label, x + 48, y + 4, Size2::Small, kit::WHITE, 0);
        }
        kit::text(fb, &self.history.join("  "), 12, 286, Size2::Small, dim, -1);
        kit::footer(fb, "Bottom rows: chords   top rows: hold to change them   up/down: setting   left/right: change", panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn crate::audio::AudioProcessor>> {
        let notes = self.notes.take().unwrap_or_else(NoteOut::detached);
        let player = Player { s: Arc::clone(&self.s), notes, queue: Vec::with_capacity(32), sounding: Vec::with_capacity(16), gen: 0, chord: Held::default(), arp_i: 0, arp_dir: 1, until: 0, sr: 48_000.0 };
        Some(self.sound.processor(None, Some(Box::new(player))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(Chordsmith::new(Sound::new(NAME, &modbus, &mixer, &bus), ctx.try_get()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render};

    fn named(d: usize, mods: [bool; 8]) -> String {
        let (r, iv) = build(0, false, d, mods);
        name(r, &iv, mods)
    }

    #[test]
    fn the_degree_pads_name_the_chords_of_c_major() {
        let n: Vec<String> = (0..8).map(|d| named(d, [false; 8])).collect();
        assert_eq!(n, ["C", "Dm", "Em", "F", "G", "Am", "Bdim", "Bb"]);
        let mut m = [false; 8];
        m[0] = true;
        assert_eq!(named(4, m), "G7");
        assert_eq!(named(0, m), "Cmaj7");
        assert_eq!(named(5, m), "Am7");
        let mut sus = [false; 8];
        sus[1] = true;
        assert_eq!(named(4, sus), "Gsus4");
        let mut flip = [false; 8];
        flip[3] = true;
        assert_eq!(named(3, flip), "Fm", "the borrowed iv");
        let mut power = [false; 8];
        power[5] = true;
        assert_eq!(named(0, power), "C5");
        let (r, iv) = build(9, true, 0, [false; 8]);
        assert_eq!(name(r, &iv, [false; 8]), "Am", "A minor's i");
    }

    #[test]
    fn progressions_are_voice_led() {
        // C F G C: no voice should leap; a pianist moves at most a few
        // semitones per finger.
        let mut prev: Vec<i32> = Vec::new();
        for d in [0, 3, 4, 0, 5, 1, 4] {
            let (root, iv) = build(0, false, d, [false; 8]);
            let pcs: Vec<i32> = iv.iter().map(|i| root + i).collect();
            let v = voice(&pcs, &prev, false);
            assert!(v.iter().all(|&n| (LOW..=HIGH + 4).contains(&n)), "{v:?}");
            if !prev.is_empty() {
                assert!(movement(&v, &prev) <= 6, "{prev:?} -> {v:?}");
            }
            let mut pc: Vec<i32> = v.iter().map(|n| n % 12).collect();
            pc.sort();
            let mut want: Vec<i32> = pcs.iter().map(|n| n.rem_euclid(12)).collect();
            want.sort();
            assert_eq!(pc, want, "the right notes");
            prev = v;
        }
    }

    #[test]
    fn a_strum_starts_each_string_a_little_later() {
        let mut c = Chordsmith::new(Sound::detached(), None);
        c.sound.set_reverb(0.0);
        c.s.style.store(1, Ordering::Relaxed);
        c.s.strum_ms.set(40.0);
        let mut p = c.audio_processor().unwrap();
        c.press(0, [false; 8]);
        // 40 ms per string: after one block (10.7 ms) only the bass has
        // started.
        render(&mut p, 1);
        assert_eq!(c.s.played.load(Ordering::Relaxed), 1);
        render(&mut p, 20);
        assert_eq!(c.s.played.load(Ordering::Relaxed), 4, "bass + three notes");
        assert!(energy(&render(&mut p, 10)) > 1e-4);
        c.release();
        render(&mut p, 60);
        assert!(energy(&render(&mut p, 10)) < 1e-7, "released");
    }

    #[test]
    fn an_arpeggio_plays_one_note_an_eighth() {
        let mut c = Chordsmith::new(Sound::detached(), None);
        c.s.style.store(2, Ordering::Relaxed);
        c.s.tempo.set(120.0); // an eighth = 0.25 s = 12000 samples
        let mut p = c.audio_processor().unwrap();
        c.press(5, [false; 8]);
        render(&mut p, 1);
        let first = c.s.played.load(Ordering::Relaxed);
        render(&mut p, 94); // ~1 s: four more eighths
        let after = c.s.played.load(Ordering::Relaxed);
        assert!((4..=5).contains(&(after - first)), "{first} -> {after}");
    }

    #[test]
    fn it_plays_another_app() {
        let bus = Arc::new(NoteBus::new());
        let inbox = bus.register_instrument("plaits", "Plaits").unwrap();
        let mut c = Chordsmith::new(Sound::detached(), Some(Arc::clone(&bus)));
        while c.route.label() != "Plaits" {
            c.route.step(1);
        }
        c.s.style.store(0, Ordering::Relaxed);
        let mut p = c.audio_processor().unwrap();
        let pad = |i: usize| Input { grid: std::array::from_fn(|g| g == i), ..Default::default() };
        c.tick(&pad(12)); // I
        render(&mut p, 2);
        assert!(inbox.any_held());
        c.tick(&Input::default());
        render(&mut p, 2);
        assert!(!inbox.any_held(), "let go, notes off");
        let mut fb = FrameBuffer::new();
        c.draw(&mut fb);
    }
}
