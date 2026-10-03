//! Rainbow Bells (Kids, ages 6-9): a rainbow glockenspiel with a loop
//! recorder.
//!
//! The 16 pads are 16 bars, lowest bottom-left, highest top-right, tuned
//! to the major pentatonic scale: no two notes clash, so whatever a child
//! plays sounds like music. Left/right picks the instrument (bells,
//! marimba, harp, flute); up/down moves everything an octave.
//!
//! SELECT starts recording: play a tune, press SELECT again and it loops
//! straight away. While it loops, SELECT records more on top (try it with
//! a different instrument). Hold SELECT to throw the loop away; F3 stops
//! and starts it.
//!
//! The loop is kept as clock steps (48 per beat at 100 bpm, 12.5 ms
//! each), and played back on the audio thread, so it comes back in time
//! rather than at the screen's frame rate.

use crate::app::{App, Input, SlintExtra};
use crate::audio::AudioProcessor;
use crate::display::{FrameBuffer, WIDTH};
use crate::led_output::PadColor;
use crate::apps::kids_kit::{self as kit, Ev, Note, Size2, Song, Sound, Tone};
use embedded_graphics::pixelcolor::Rgb565;
use std::sync::{Arc, Mutex};

const NAME: &str = "Rainbow Bells";
const STEPS_PER_BEAT: u32 = 48;
const TEMPO: f32 = 100.0;
/// The longest loop: 16 beats at 100 bpm, about 10 seconds.
const MAX_STEPS: u64 = STEPS_PER_BEAT as u64 * 16;
/// The shortest: half a second, so a single tap still makes a loop.
const MIN_STEPS: u64 = STEPS_PER_BEAT as u64;
const TONES: [Tone; 4] = [Tone::Glock, Tone::Marimba, Tone::Pluck, Tone::Flute];
const ROOT: i32 = 72; // C5: a glockenspiel sounds two octaves above its written range, high and bright

const SKY: Rgb565 = Rgb565::new(24, 52, 31);
const SKY2: Rgb565 = Rgb565::new(28, 58, 31);
const INK: Rgb565 = Rgb565::new(6, 12, 14);
const WOOD: Rgb565 = Rgb565::new(18, 26, 8);

/// Bar `i` (0 = lowest) of the 16.
fn bar_note(i: usize, octave: i32) -> i32 {
    kit::scale_note(ROOT - 12 + octave * 12, &kit::PENTATONIC, i as i32)
}

/// Pad index (row 0 = top) to bar: the bottom row holds the lowest notes.
fn pad_bar(pad: usize) -> usize {
    (3 - pad / 4) * 4 + pad % 4
}

/// A rainbow running red (low) to violet (high).
fn bar_color(i: usize) -> Rgb565 {
    let t = i as f32 / 15.0 * 7.0;
    let a = t.floor() as usize;
    kit::blend(kit::rainbow(a), kit::rainbow((a + 1).min(7)), t - a as f32)
}

#[derive(Clone, Default)]
struct Take {
    /// (step in the loop, bar, tone, octave)
    events: Vec<(u64, u8, Tone, i8)>,
    /// 0 while the first pass is still being recorded.
    len: u64,
}

struct Looper {
    take: Arc<Mutex<Take>>,
}

impl Song for Looper {
    fn steps_per_beat(&self) -> u32 {
        STEPS_PER_BEAT
    }
    fn step(&mut self, step: u64, out: &mut Vec<Ev>) {
        let Ok(t) = self.take.try_lock() else { return };
        if t.len == 0 {
            return;
        }
        let at = step % t.len;
        for &(s, bar, tone, oct) in &t.events {
            if s == at {
                out.push(Ev::Note(Note::new(tone, bar_note(bar as usize, oct as i32) as f32).vel(0.75).len(0.4).pan(bar as f32 / 7.5 - 1.0)));
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Rec {
    Empty,
    /// The first pass: the loop is as long as this takes.
    First,
    Looping,
    Overdub,
}

pub struct RainbowBells {
    sound: Sound,
    take: Arc<Mutex<Take>>,
    rec: Rec,
    tone: usize,
    octave: i32,
    prev: [bool; 16],
    /// Frames since each bar was struck, for the bounce.
    glow: [f32; 16],
    /// Notes floating up from struck bars: (x, y, colour index, age).
    sparks: Vec<(f32, f32, usize, f32)>,
    /// How many steps were in the take when last drawn, to light bars
    /// the loop plays.
    last_step: Option<u64>,
    frame: u64,
}

impl RainbowBells {
    pub fn new(sound: Sound) -> RainbowBells {
        sound.set_tempo(TEMPO);
        sound.set_reverb(0.3);
        RainbowBells { sound, take: Arc::new(Mutex::new(Take::default())), rec: Rec::Empty, tone: 0, octave: 0, prev: [false; 16], glow: [0.0; 16], sparks: Vec::new(), last_step: None, frame: 0 }
    }

    fn strike(&mut self, bar: usize, tone: Tone, oct: i32, from_loop: bool) {
        if !from_loop {
            self.sound.play(Note::new(tone, bar_note(bar, oct) as f32).vel(0.85).len(0.4).pan(bar as f32 / 7.5 - 1.0));
        }
        self.glow[bar] = 1.0;
        let (x, y, _, _) = bar_rect(bar);
        if self.sparks.len() < 40 {
            self.sparks.push((x as f32 + 17.0, y as f32 - 10.0, bar, 0.0));
        }
    }

    fn select(&mut self) {
        match self.rec {
            Rec::Empty => {
                *self.take.lock().unwrap() = Take::default();
                self.rec = Rec::First;
                self.sound.start();
            }
            Rec::First => {
                let steps = self.sound.step().map_or(MIN_STEPS, |s| s + 1).clamp(MIN_STEPS, MAX_STEPS);
                let mut t = self.take.lock().unwrap();
                if t.events.is_empty() {
                    drop(t);
                    self.erase();
                    return;
                }
                // Round up to a whole beat so the loop sits in time.
                t.len = steps.div_ceil(STEPS_PER_BEAT as u64) * STEPS_PER_BEAT as u64;
                drop(t);
                self.rec = Rec::Looping;
                self.sound.start();
            }
            Rec::Looping => {
                if !self.sound.playing() {
                    self.sound.start();
                }
                self.rec = Rec::Overdub;
            }
            Rec::Overdub => self.rec = Rec::Looping,
        }
    }

    fn erase(&mut self) {
        self.sound.stop();
        *self.take.lock().unwrap() = Take::default();
        self.rec = Rec::Empty;
    }

    fn record(&mut self, bar: usize) {
        let recording = matches!(self.rec, Rec::First | Rec::Overdub) && self.sound.playing();
        if !recording {
            return;
        }
        let Some(step) = self.sound.step() else { return };
        let mut t = self.take.lock().unwrap();
        if t.len == 0 && step >= MAX_STEPS {
            return;
        }
        let at = if t.len == 0 { step } else { step % t.len };
        // The note was already played live; the loop plays it from the
        // next time round.
        if t.events.len() < 512 {
            t.events.push((at, bar as u8, TONES[self.tone], self.octave as i8));
        }
    }
}

/// x, y, width, height of bar `i`: lower bars are longer.
fn bar_rect(i: usize) -> (i32, i32, i32, i32) {
    let w = 34;
    let x = 16 + i as i32 * 38;
    let h = 190 - i as i32 * 6;
    let y = 190 - h / 2 + 10;
    (x, y, w, h)
}

impl App for RainbowBells {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![
            ("Sound".into(), TONES[self.tone].name().into(), false),
            ("Octave".into(), format!("{:+}", self.octave), false),
            ("Loop".into(), format!("{:?}", self.rec), false),
        ]
    }
    /// Nothing to play until there's a loop.
    fn running(&self) -> Option<bool> {
        (self.rec != Rec::Empty).then(|| self.sound.playing())
    }
    fn toggle_running(&mut self) {
        if self.sound.playing() {
            self.sound.stop();
            if self.rec == Rec::First {
                self.erase();
            } else if self.rec == Rec::Overdub {
                self.rec = Rec::Looping;
            }
        } else if self.take.lock().unwrap().len > 0 {
            self.sound.start();
        }
    }
    fn on_exit(&mut self) {
        self.sound.all_off();
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        std::array::from_fn(|p| match pad_bar(p) / 4 {
            0 => PadColor::Red,
            1 => PadColor::Yellow,
            2 => PadColor::Green,
            _ => PadColor::Blue,
        })
    }

    fn tick(&mut self, input: &Input) {
        self.frame += 1;
        for p in 0..16 {
            if input.grid[p] && !self.prev[p] {
                let bar = pad_bar(p);
                self.strike(bar, TONES[self.tone], self.octave, false);
                self.record(bar);
            }
        }
        self.prev = input.grid;
        if input.knob2 != 0 {
            self.tone = (self.tone as i32 + input.knob2.signum()).rem_euclid(TONES.len() as i32) as usize;
            // Let them hear the new sound at once.
            self.sound.play(Note::new(TONES[self.tone], bar_note(4, self.octave) as f32).len(0.3));
        }
        if input.navigation_steps != 0 {
            self.octave = (self.octave - input.navigation_steps.signum()).clamp(-1, 1);
        }
        if input.knob2_press {
            self.erase();
        } else if input.knob1_press {
            self.select();
        }
        // The first pass stops by itself at the longest loop.
        if self.rec == Rec::First && self.sound.step().is_some_and(|s| s + 1 >= MAX_STEPS) {
            self.select();
        }
        // Light the bars the loop is playing, for every step since the
        // last frame.
        if let Some(step) = self.sound.step() {
            let from = match self.last_step {
                Some(l) if l <= step => l + 1,
                _ => step,
            };
            let t = self.take.lock().unwrap().clone();
            if t.len > 0 && from <= step {
                let mut hits = Vec::new();
                for s in from.max(step.saturating_sub(64))..=step {
                    hits.extend(t.events.iter().filter(|e| e.0 == s % t.len).map(|e| (e.1 as usize, e.2, e.3 as i32)));
                }
                for (bar, tone, oct) in hits {
                    self.strike(bar, tone, oct, true);
                }
            }
            self.last_step = Some(step);
        } else {
            self.last_step = None;
        }
        for g in self.glow.iter_mut() {
            *g = (*g - 0.06).max(0.0);
        }
        for s in self.sparks.iter_mut() {
            s.1 -= 1.6;
            s.0 += ((s.3 * 0.2) + s.2 as f32).sin() * 0.6;
            s.3 += 1.0;
        }
        self.sparks.retain(|s| s.3 < 70.0);
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        // Sky, in bands.
        for k in 0..12 {
            kit::rect(fb, 0, k * 30, WIDTH as i32, 30, kit::blend(SKY, SKY2, k as f32 / 11.0));
        }
        kit::header(fb, NAME, "AGES 6-9", Rgb565::new(10, 22, 26), kit::WHITE);
        // Two rails the bars rest on.
        kit::round_rect(fb, 8, 128, 624, 10, 4, WOOD);
        kit::round_rect(fb, 8, 262, 624, 10, 4, WOOD);
        for i in 0..16 {
            let (x, y, w, h) = bar_rect(i);
            let g = self.glow[i];
            let bounce = (g * g * 6.0) as i32;
            let c = kit::blend(bar_color(i), kit::WHITE, g * 0.6);
            kit::round_rect(fb, x + 2, y + 4 + bounce, w, h, 8, kit::blend(c, kit::BLACK, 0.45));
            kit::round_rect(fb, x, y + bounce, w, h, 8, c);
            // The two nail holes a real bar hangs on.
            kit::circle(fb, x + w / 2, 133 + bounce, 3, kit::blend(c, kit::BLACK, 0.5));
            kit::circle(fb, x + w / 2, 267 + bounce, 3, kit::blend(c, kit::BLACK, 0.5));
            let name = kit::note_name(bar_note(i, self.octave));
            kit::text(fb, name, x + w / 2, y + h / 2 - 6 + bounce, Size2::Medium, kit::blend(c, kit::BLACK, 0.65), 0);
        }
        for &(x, y, i, age) in &self.sparks {
            let c = kit::blend(bar_color(i), kit::WHITE, (age / 70.0).min(1.0) * 0.8);
            draw_note(fb, x as i32, y as i32, c);
        }

        // The sound and the loop.
        kit::round_rect(fb, 12, 290, 260, 44, 12, Rgb565::new(10, 22, 26));
        kit::text(fb, "<", 26, 296, Size2::Large, kit::WHITE, -1);
        kit::text(fb, TONES[self.tone].name(), 142, 302, Size2::Medium, kit::WHITE, 0);
        kit::text(fb, ">", 242, 296, Size2::Large, kit::WHITE, -1);
        let oct = match self.octave {
            -1 => "low",
            1 => "high",
            _ => "middle",
        };
        kit::text(fb, oct, 142, 320, Size2::Small, Rgb565::new(20, 44, 24), 0);

        let (label, c) = match self.rec {
            Rec::Empty => ("SELECT: record a tune", Rgb565::new(28, 12, 8)),
            Rec::First => ("Recording... SELECT to loop", Rgb565::new(31, 10, 8)),
            Rec::Looping => (if self.sound.playing() { "Looping! SELECT: add more" } else { "Loop stopped. F3 to play" }, Rgb565::new(8, 44, 14)),
            Rec::Overdub => ("Adding more... SELECT: done", Rgb565::new(31, 30, 4)),
        };
        kit::round_rect(fb, 284, 290, 344, 44, 12, Rgb565::new(10, 22, 26));
        let pulse = matches!(self.rec, Rec::First | Rec::Overdub) && (self.frame / 15) % 2 == 0;
        kit::circle(fb, 310, 312, if pulse { 13 } else { 11 }, c);
        kit::text(fb, label, 334, 298, Size2::Medium, kit::WHITE, -1);
        if self.rec != Rec::Empty {
            // How far round the loop we are.
            let t = self.take.lock().unwrap();
            let frac = match (self.sound.step(), t.len) {
                (Some(s), l) if l > 0 => (s % l) as f32 / l as f32,
                (Some(s), _) => s as f32 / MAX_STEPS as f32,
                _ => 0.0,
            };
            kit::round_rect(fb, 334, 318, 280, 8, 4, Rgb565::new(16, 32, 18));
            kit::round_rect(fb, 334, 318, (280.0 * frac) as i32 + 8, 8, 4, c);
        }
        kit::text(fb, "hold SELECT: start over   up/down: high/low", 320, 342, Size2::Small, INK, 0);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(self.sound.processor(Some(Box::new(Looper { take: Arc::clone(&self.take) })), None))
    }
}

/// A quaver: a filled head and a stem with a flag.
fn draw_note(fb: &mut FrameBuffer, x: i32, y: i32, c: Rgb565) {
    kit::circle(fb, x, y, 6, c);
    kit::rect(fb, x + 4, y - 20, 3, 20, c);
    kit::line(fb, x + 6, y - 20, x + 13, y - 12, 3, c);
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(RainbowBells::new(Sound::new(NAME, &modbus, &mixer, &bus)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render};

    fn pad(i: usize) -> Input {
        Input { grid: std::array::from_fn(|g| g == i), ..Default::default() }
    }

    #[test]
    fn the_bars_are_a_pentatonic_rainbow_low_to_high() {
        let notes: Vec<i32> = (0..16).map(|i| bar_note(i, 0)).collect();
        assert!(notes.windows(2).all(|w| w[1] > w[0]));
        assert!(notes.iter().all(|n| [0, 2, 4, 7, 9].contains(&(n % 12))), "only C D E G A");
        assert_eq!(pad_bar(12), 0, "bottom-left is lowest");
        assert_eq!(pad_bar(3), 15, "top-right is highest");
    }

    #[test]
    fn a_pad_rings_a_bar() {
        let mut app = RainbowBells::new(Sound::detached());
        let mut p = app.audio_processor().unwrap();
        app.tick(&pad(12));
        assert!(energy(&render(&mut p, 10)) > 1e-4);
        assert!(app.glow[0] > 0.5);
    }

    #[test]
    fn a_recorded_tune_comes_back_round() {
        let mut app = RainbowBells::new(Sound::detached());
        let mut p = app.audio_processor().unwrap();
        app.tick(&Input { knob1_press: true, ..Default::default() });
        assert_eq!(app.rec, Rec::First);
        render(&mut p, 2);
        app.tick(&pad(12));
        app.tick(&Input::default());
        render(&mut p, 30);
        app.tick(&pad(13));
        app.tick(&Input::default());
        render(&mut p, 10);
        app.tick(&Input { knob1_press: true, ..Default::default() });
        assert_eq!(app.rec, Rec::Looping);
        let t = app.take.lock().unwrap().clone();
        assert_eq!(t.events.len(), 2);
        assert!(t.len >= MIN_STEPS && t.len % STEPS_PER_BEAT as u64 == 0);
        // Let the live notes die away, then the loop should play them again.
        let out = render(&mut p, 600);
        let late = &out[out.len() / 2..];
        assert!(energy(late) > 1e-5, "the loop plays without anyone touching a pad");
        // Hold SELECT: gone.
        app.tick(&Input { knob2_press: true, ..Default::default() });
        assert_eq!(app.rec, Rec::Empty);
        assert!(!app.sound.playing());
    }

    #[test]
    fn it_draws() {
        let mut app = RainbowBells::new(Sound::detached());
        app.tick(&pad(5));
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
        assert!(matches!(app.slint_extra(), SlintExtra::Screen(_)));
    }
}
