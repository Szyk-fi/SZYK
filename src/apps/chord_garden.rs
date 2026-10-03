//! Chord Garden (Kids, ages 10-12): grow a chord progression and play a
//! melody over it.
//!
//! Four flowers are the four chords of a progression, one bar each, round
//! and round. The pads:
//! - top row: pick which flower (chord) to change; you hear it.
//! - next two rows: the seven chords of the key, by Roman numeral (I ii
//!   iii IV / V vi vii°), plus a "7th" pad that adds the seventh to the
//!   picked chord. Picking a chord plants it and moves to the next flower.
//! - bottom row: four melody notes that always fit, because they are the
//!   notes of whichever chord is playing (root, third, fifth, octave).
//!
//! Up/down picks Key, Style or Tempo; left/right changes it. The styles
//! play the same chords four ways: Pads (held), Guitar (a strum
//! rhythm), Arpeggio (one note at a time) and Piano (bass and stabs).
//! SELECT plays and stops; hold SELECT plants a well-known progression
//! (I-V-vi-IV, I-vi-IV-V, vi-IV-I-V, ii-V-I, I-IV-V-I).
//!
//! The flowers are coloured by what each chord does in the key: green
//! for the home chords (I, iii, vi), blue for the ones that lead away
//! (ii, IV) and orange for the ones that pull back home (V, vii°).

use crate::app::{App, Input, SlintExtra};
use crate::audio::AudioProcessor;
use crate::display::{FrameBuffer, WIDTH};
use crate::led_output::PadColor;
use crate::apps::kids_kit::{self as kit, Ev, Note, Size2, Song, Sound, Tone};
use embedded_graphics::pixelcolor::Rgb565;
use std::sync::{Arc, Mutex};

const NAME: &str = "Chord Garden";
const NUMERALS: [&str; 7] = ["I", "ii", "iii", "IV", "V", "vi", "vii°"];
const STYLES: [&str; 4] = ["Pads", "Guitar", "Arpeggio", "Piano"];
/// Well-known progressions, as scale degrees (0 = I).
const FAMOUS: [([usize; 4], &str); 5] = [
    ([0, 4, 5, 3], "I-V-vi-IV: the most-used progression in pop music."),
    ([0, 5, 3, 4], "I-vi-IV-V: the 1950s doo-wop progression."),
    ([5, 3, 0, 4], "vi-IV-I-V: the pop progression starting on its sad chord."),
    ([1, 4, 0, 0], "ii-V-I: the move at the heart of jazz."),
    ([0, 3, 4, 0], "I-IV-V-I: the three-chord trick of folk, blues and rock."),
];

#[derive(Clone, Copy, PartialEq, Debug)]
struct Chord {
    degree: usize,
    seventh: bool,
}

#[derive(Clone)]
struct Garden {
    chords: [Chord; 4],
    key: i32,
    style: usize,
    /// Kept here too, so the accompanist can size its note lengths.
    tempo: f32,
}

/// MIDI notes of a chord built on `degree` of the major key starting at
/// `root`, stacked in thirds.
fn chord_notes(root: i32, c: Chord) -> Vec<i32> {
    let mut v: Vec<i32> = [0, 2, 4].iter().map(|k| kit::scale_note(root, &kit::MAJOR, c.degree as i32 + k)).collect();
    if c.seventh {
        v.push(kit::scale_note(root, &kit::MAJOR, c.degree as i32 + 6));
    }
    v
}

fn chord_name(key: i32, c: Chord) -> String {
    let notes = chord_notes(48 + key, c);
    let root = kit::note_name(notes[0]);
    let third = notes[1] - notes[0];
    let fifth = notes[2] - notes[0];
    let quality = match (third, fifth) {
        (4, 7) => "",
        (3, 7) => "m",
        _ => "dim",
    };
    if !c.seventh {
        return format!("{root}{quality}");
    }
    let seventh = notes[3] - notes[0];
    let s = match (quality, seventh) {
        ("", 11) => "maj7",
        ("", _) => "7",
        ("m", _) => "m7",
        _ => "m7b5",
    };
    format!("{root}{s}")
}

/// 0 home (tonic), 1 away (subdominant), 2 back home (dominant).
fn function(degree: usize) -> usize {
    match degree {
        0 | 2 | 5 => 0,
        1 | 3 => 1,
        _ => 2,
    }
}

fn function_color(f: usize) -> Rgb565 {
    [kit::rgb(110, 210, 110), kit::rgb(90, 150, 240), kit::rgb(250, 160, 60)][f]
}

struct Accompanist {
    garden: Arc<Mutex<Garden>>,
}

impl Song for Accompanist {
    fn step(&mut self, step: u64, out: &mut Vec<Ev>) {
        let Ok(g) = self.garden.try_lock() else { return };
        let bar = ((step / 16) % 4) as usize;
        let s = (step % 16) as usize;
        let root = 48 + g.key;
        let c = g.chords[bar];
        let notes = chord_notes(root, c);
        let bass = (notes[0] - 12) as f32;
        let beat = 60.0 / g.tempo.max(30.0);
        match g.style {
            // Held chords and a held bass note.
            0 => {
                if s == 0 {
                    for (i, n) in notes.iter().enumerate() {
                        out.push(Ev::Note(Note::new(Tone::Soft, (*n + 12) as f32).vel(0.42).len(beat * 3.6).pan(-0.3 + 0.2 * i as f32)));
                    }
                    out.push(Ev::Note(Note::new(Tone::Bass, bass).vel(0.6).len(beat * 3.5)));
                }
            }
            // A down, down-up, up-down strum rhythm.
            1 => {
                const STRUMS: [usize; 6] = [0, 4, 6, 10, 12, 14];
                if STRUMS.contains(&s) {
                    let loud = if s % 4 == 0 { 0.6 } else { 0.4 };
                    for (i, n) in notes.iter().enumerate() {
                        out.push(Ev::Note(Note::new(Tone::Pluck, (*n + 12) as f32).vel(loud - i as f32 * 0.04).pan(-0.2 + 0.15 * i as f32)));
                    }
                }
                if s == 0 || s == 8 {
                    out.push(Ev::Note(Note::new(Tone::Bass, bass).vel(0.6).len(beat * 1.6)));
                }
            }
            // Up and down the chord in 8ths.
            2 => {
                if s % 2 == 0 {
                    let mut up = notes.clone();
                    up.push(notes[0] + 12);
                    let order: Vec<i32> = up.iter().chain(up.iter().rev().skip(1).take(up.len().saturating_sub(2))).copied().collect();
                    let n = order[(s / 2) % order.len()];
                    out.push(Ev::Note(Note::new(Tone::Marimba, (n + 12) as f32).vel(0.7).pan(((s / 2) as f32 / 4.0 - 1.0) * 0.5)));
                }
                if s == 0 {
                    out.push(Ev::Note(Note::new(Tone::Bass, bass).vel(0.55).len(beat * 3.5)));
                }
            }
            // Bass on 1 and 3, chord stabs on the off-beats.
            _ => {
                if s == 0 || s == 8 {
                    out.push(Ev::Note(Note::new(Tone::Bass, bass + if s == 8 { 7.0 } else { 0.0 }).vel(0.65).len(beat * 0.9)));
                }
                if s == 4 || s == 12 || s == 14 {
                    for n in notes.iter() {
                        out.push(Ev::Note(Note::new(Tone::Organ, (*n + 12) as f32).vel(0.32).len(beat * 0.35)));
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Setting {
    Key,
    Style,
    Tempo,
}
const SETTINGS: [Setting; 3] = [Setting::Key, Setting::Style, Setting::Tempo];

pub struct ChordGarden {
    sound: Sound,
    garden: Arc<Mutex<Garden>>,
    slot: usize,
    setting: usize,
    famous: usize,
    prev: [bool; 16],
    bloom: [f32; 4],
    sparkle: Vec<(f32, f32, f32)>,
    last_bar: Option<u64>,
}

impl ChordGarden {
    pub fn new(sound: Sound) -> ChordGarden {
        sound.set_tempo(96.0);
        sound.set_reverb(0.3);
        let chords = FAMOUS[0].0.map(|d| Chord { degree: d, seventh: false });
        ChordGarden { sound, garden: Arc::new(Mutex::new(Garden { chords, key: 0, style: 0, tempo: 96.0 })), slot: 0, setting: 0, famous: 0, prev: [false; 16], bloom: [0.0; 4], sparkle: Vec::new(), last_bar: None }
    }

    fn playing_slot(&self) -> Option<usize> {
        self.sound.step().map(|s| ((s / 16) % 4) as usize)
    }

    /// Plays chord `c` once, so picking one is heard.
    fn audition(&self, c: Chord) {
        let key = self.garden.lock().unwrap().key;
        for (i, n) in chord_notes(48 + key, c).iter().enumerate() {
            self.sound.play(Note::new(Tone::Pluck, (*n + 12) as f32).vel(0.55).pan(-0.2 + 0.15 * i as f32));
        }
    }

    fn explain(&self) -> String {
        let g = self.garden.lock().unwrap();
        let degrees = g.chords.map(|c| c.degree);
        if let Some((_, about)) = FAMOUS.iter().find(|(d, _)| *d == degrees) {
            return about.to_string();
        }
        let c = g.chords[self.slot];
        let what = match function(c.degree) {
            0 => "a home chord: it feels settled.",
            1 => "an away chord: it moves away from home.",
            _ => "a pull-home chord: it wants to go back to I.",
        };
        format!("{} ({}) is {}", NUMERALS[c.degree], chord_name(g.key, c), what)
    }
}

impl App for ChordGarden {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        let g = self.garden.lock().unwrap();
        vec![
            ("Key".into(), format!("{} major", kit::note_name(g.key)), false),
            ("Style".into(), STYLES[g.style].into(), false),
            ("Chords".into(), g.chords.iter().map(|c| chord_name(g.key, *c)).collect::<Vec<_>>().join(" "), false),
        ]
    }
    fn running(&self) -> Option<bool> {
        Some(self.sound.playing())
    }
    fn toggle_running(&mut self) {
        if self.sound.playing() {
            self.sound.stop();
            self.sound.all_off();
        } else {
            self.sound.start();
        }
    }
    fn on_exit(&mut self) {
        self.sound.stop();
        self.sound.all_off();
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let g = self.garden.lock().unwrap();
        let fc = |d: usize| [PadColor::Green, PadColor::Blue, PadColor::Yellow][function(d)];
        std::array::from_fn(|p| match p {
            0..=3 if p == self.slot => PadColor::Red,
            0..=3 => fc(g.chords[p].degree),
            4..=10 => fc(p - 4),
            11 => if g.chords[self.slot].seventh { PadColor::Red } else { PadColor::Off },
            _ => PadColor::Green,
        })
    }

    fn tick(&mut self, input: &Input) {
        for p in 0..16 {
            if !(input.grid[p] && !self.prev[p]) {
                continue;
            }
            match p {
                0..=3 => {
                    self.slot = p;
                    let c = self.garden.lock().unwrap().chords[p];
                    self.audition(c);
                }
                4..=10 => {
                    let c = Chord { degree: p - 4, seventh: false };
                    self.garden.lock().unwrap().chords[self.slot] = c;
                    self.audition(c);
                    self.bloom[self.slot] = 1.0;
                    self.slot = (self.slot + 1) % 4;
                }
                11 => {
                    let mut g = self.garden.lock().unwrap();
                    let c = &mut g.chords[self.slot];
                    c.seventh = !c.seventh;
                    let c = *c;
                    drop(g);
                    self.audition(c);
                }
                _ => {
                    // A melody note from the chord that's playing (or picked).
                    let g = self.garden.lock().unwrap();
                    let c = g.chords[self.playing_slot().unwrap_or(self.slot)];
                    let notes = chord_notes(60 + g.key, c);
                    drop(g);
                    let k = p - 12;
                    let n = if k < 3 { notes[k] } else { notes[0] + 12 };
                    self.sound.play(Note::new(Tone::Flute, (n + 12) as f32).vel(0.75).len(0.45));
                    if self.sparkle.len() < 30 {
                        self.sparkle.push((120.0 + k as f32 * 130.0, 150.0, 0.0));
                    }
                }
            }
        }
        self.prev = input.grid;
        if input.navigation_steps != 0 {
            self.setting = (self.setting as i32 + input.navigation_steps).clamp(0, SETTINGS.len() as i32 - 1) as usize;
        }
        if input.knob2 != 0 {
            let d = input.knob2.signum();
            match SETTINGS[self.setting] {
                Setting::Key => {
                    let mut g = self.garden.lock().unwrap();
                    // Round the circle of fifths: each step changes one note.
                    g.key = (g.key + 7 * d).rem_euclid(12);
                }
                Setting::Style => {
                    let mut g = self.garden.lock().unwrap();
                    g.style = (g.style as i32 + d).rem_euclid(STYLES.len() as i32) as usize;
                }
                Setting::Tempo => {
                    self.sound.set_tempo((self.sound.tempo() + d as f32 * 4.0).clamp(60.0, 160.0));
                    self.garden.lock().unwrap().tempo = self.sound.tempo();
                }
            }
        }
        if input.knob2_press {
            self.famous = (self.famous + 1) % FAMOUS.len();
            let chords = FAMOUS[self.famous].0.map(|d| Chord { degree: d, seventh: false });
            self.garden.lock().unwrap().chords = chords;
            self.bloom = [1.0; 4];
            self.slot = 0;
        } else if input.knob1_press {
            self.toggle_running();
        }
        if let Some(step) = self.sound.step() {
            let bar = step / 16;
            if self.last_bar != Some(bar) {
                self.bloom[(bar % 4) as usize] = 1.0;
            }
            self.last_bar = Some(bar);
        } else {
            self.last_bar = None;
        }
        for b in self.bloom.iter_mut() {
            *b = (*b - 0.02).max(0.0);
        }
        for s in self.sparkle.iter_mut() {
            s.1 -= 1.5;
            s.2 += 1.0;
        }
        self.sparkle.retain(|s| s.2 < 50.0);
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let sky = Rgb565::new(5, 12, 12);
        let soil = Rgb565::new(10, 14, 6);
        kit::clear(fb, sky);
        kit::rect(fb, 0, 214, WIDTH as i32, 50, soil);
        kit::header(fb, NAME, "AGES 10-12", Rgb565::new(3, 8, 8), kit::WHITE);
        let g = self.garden.lock().unwrap().clone();
        let playing = self.playing_slot();
        for i in 0..4 {
            let c = g.chords[i];
            let x = 90 + i as i32 * 130;
            let col = function_color(function(c.degree));
            let b = self.bloom[i];
            let r = 26 + (b * 8.0) as i32 + if playing == Some(i) { 6 } else { 0 };
            let top = 120;
            kit::rect(fb, x - 3, top, 6, 214 - top, Rgb565::new(8, 34, 8));
            kit::round_rect(fb, x + 2, 170, 26, 12, 6, Rgb565::new(8, 40, 8));
            // Petals: one per chord note.
            let n = if c.seventh { 8 } else { 6 };
            for k in 0..n {
                let a = k as f32 / n as f32 * std::f32::consts::TAU + b * 0.6;
                kit::circle(fb, x + (a.cos() * r as f32) as i32, top + (a.sin() * r as f32) as i32, r / 2 + 3, kit::blend(col, kit::WHITE, b * 0.4));
            }
            kit::circle(fb, x, top, r / 2 + 4, Rgb565::new(31, 52, 10));
            kit::text(fb, NUMERALS[c.degree], x, top - 8, Size2::Medium, kit::BLACK, 0);
            if c.seventh {
                kit::text(fb, "7", x + r / 2 + 6, top - 4, Size2::Small, kit::WHITE, 0);
            }
            kit::text(fb, &chord_name(g.key, c), x, 222, Size2::Large, kit::WHITE, 0);
            if i == self.slot {
                kit::round_rect(fb, x - 40, 256, 80, 5, 2, kit::WHITE);
            }
        }
        for &(x, y, age) in &self.sparkle {
            kit::star(fb, x as i32, y as i32, 8, kit::blend(Rgb565::new(31, 56, 10), sky, age / 50.0));
        }
        // Settings on the right.
        let sx = 556;
        for (k, s) in SETTINGS.iter().enumerate() {
            let y = 44 + k as i32 * 52;
            let sel = k == self.setting;
            kit::round_rect(fb, sx, y, 76, 46, 8, if sel { Rgb565::new(8, 22, 20) } else { Rgb565::new(4, 12, 12) });
            let (label, value) = match s {
                Setting::Key => ("key", format!("{} major", kit::note_name(g.key))),
                Setting::Style => ("style", STYLES[g.style].to_string()),
                Setting::Tempo => ("tempo", format!("{:.0} bpm", self.sound.tempo())),
            };
            kit::text(fb, label, sx + 38, y + 6, Size2::Small, Rgb565::new(16, 36, 24), 0);
            kit::text(fb, &value, sx + 38, y + 24, Size2::Small, kit::WHITE, 0);
        }
        // Explanation and key.
        kit::round_rect(fb, 10, 270, 620, 60, 10, Rgb565::new(3, 8, 8));
        kit::paragraph(fb, &self.explain(), 20, 278, 440, Size2::Small, kit::WHITE);
        for (k, (label, f)) in [("home", 0), ("away", 1), ("pull home", 2)].iter().enumerate() {
            let x = 470 + (k as i32 % 2) * 76;
            let y = 280 + (k as i32 / 2) * 20;
            kit::circle(fb, x, y + 6, 6, function_color(*f));
            kit::text(fb, label, x + 10, y, Size2::Small, kit::WHITE, -1);
        }
        let play = if self.sound.playing() { "SELECT stop" } else { "SELECT play" };
        kit::footer(fb, &format!("Row 1: pick a flower  Rows 2-3: plant a chord / 7th  Row 4: melody   {play}   hold: famous"), Rgb565::new(3, 8, 8), Rgb565::new(18, 40, 26));
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(self.sound.processor(Some(Box::new(Accompanist { garden: Arc::clone(&self.garden) })), None))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(ChordGarden::new(Sound::new(NAME, &modbus, &mixer, &bus)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render};

    fn pad(i: usize) -> Input {
        Input { grid: std::array::from_fn(|g| g == i), ..Default::default() }
    }

    #[test]
    fn the_chords_of_c_major_are_named_right() {
        let names: Vec<String> = (0..7).map(|d| chord_name(0, Chord { degree: d, seventh: false })).collect();
        assert_eq!(names, ["C", "Dm", "Em", "F", "G", "Am", "Bdim"]);
        let sevenths: Vec<String> = (0..7).map(|d| chord_name(0, Chord { degree: d, seventh: true })).collect();
        assert_eq!(sevenths, ["Cmaj7", "Dm7", "Em7", "Fmaj7", "G7", "Am7", "Bm7b5"]);
        assert_eq!(chord_name(7, Chord { degree: 4, seventh: false }), "D", "V of G");
    }

    #[test]
    fn planting_chords_fills_the_garden_in_order() {
        let mut app = ChordGarden::new(Sound::detached());
        for p in [9, 7, 4, 8] {
            // vi IV I V
            app.tick(&pad(p));
            app.tick(&Input::default());
        }
        assert_eq!(app.garden.lock().unwrap().chords.map(|c| c.degree), [5, 3, 0, 4]);
        assert!(app.explain().starts_with("vi-IV-I-V"));
        app.tick(&pad(1));
        app.tick(&Input::default());
        app.tick(&pad(11));
        assert!(app.garden.lock().unwrap().chords[1].seventh);
    }

    #[test]
    fn melody_pads_are_the_playing_chords_notes() {
        let mut app = ChordGarden::new(Sound::detached());
        let g = app.garden.lock().unwrap().clone();
        let notes = chord_notes(60, g.chords[0]);
        assert_eq!(notes, vec![60, 64, 67]);
        let mut p = app.audio_processor().unwrap();
        app.tick(&pad(13));
        assert!(energy(&render(&mut p, 10)) > 1e-4);
    }

    #[test]
    fn every_style_plays_all_four_bars() {
        for style in 0..STYLES.len() {
            let mut app = ChordGarden::new(Sound::detached());
            app.garden.lock().unwrap().style = style;
            let mut a = Accompanist { garden: Arc::clone(&app.garden) };
            for bar in 0..4 {
                let mut out = Vec::new();
                for s in 0..16 {
                    a.step(bar * 16 + s, &mut out);
                }
                assert!(out.len() >= 2, "style {style} bar {bar}");
            }
            let mut p = app.audio_processor().unwrap();
            app.toggle_running();
            let o = render(&mut p, 60);
            assert!(energy(&o) > 1e-4 && o.iter().all(|x| x.abs() <= 1.0), "style {style}");
            let mut fb = FrameBuffer::new();
            app.tick(&Input::default());
            app.draw(&mut fb);
        }
    }
}
