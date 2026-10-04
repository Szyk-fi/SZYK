//! Bug Beats (Kids, ages 6-9): a first drum machine.
//!
//! The pads are the pattern, laid out just as on screen: four rows of
//! sounds (high sounds at the top, the big drum at the bottom) and four
//! columns of beats, played left to right, round and round. Tap a pad to
//! put a bug there; the bug plays its sound each time the beat reaches
//! it, and jumps. Tap again and it flies away.
//!
//! Left/right picks the band: Drums (hi-hat, clap, snare, kick), Jungle
//! (shaker, woodblock, high and low tom) or Party (open hat, cowbell,
//! clap, kick). Up/down picks the speed: tortoise, dog or rabbit. F3 or
//! SELECT starts and stops; hold SELECT to clear every bug. It starts
//! with a simple beat in, so pressing play grooves straight away.

use crate::app::{App, Input, SlintExtra};
use crate::audio::AudioProcessor;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::apps::kids_kit::{self as kit, Drum, Ev, Size2, Song, Sound};
use embedded_graphics::pixelcolor::Rgb565;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Bug Beats";
type Pattern = [[bool; 4]; 4];

/// Rows top to bottom.
const BANDS: [(&str, [Drum; 4]); 3] = [
    ("Drums", [Drum::Hat, Drum::Clap, Drum::Snare, Drum::Kick]),
    ("Jungle", [Drum::Shaker, Drum::Woodblock, Drum::HiTom, Drum::LowTom]),
    ("Party", [Drum::OpenHat, Drum::Cowbell, Drum::Clap, Drum::Kick]),
];
const SPEEDS: [(&str, f32); 3] = [("tortoise", 80.0), ("dog", 110.0), ("rabbit", 140.0)];
/// Each row's bug colour, top to bottom: yellow, green, blue, and a red ladybird for the kick.
const BUG_COLORS: [(u8, u8, u8); 4] = [(240, 210, 60), (90, 200, 110), (80, 140, 240), (235, 70, 70)];

fn starter() -> Pattern {
    [[true, true, true, true], [false, false, false, false], [false, true, false, true], [true, false, true, false]]
}

struct Beat {
    pattern: Arc<Mutex<Pattern>>,
    band: Arc<AtomicUsize>,
}

impl Song for Beat {
    fn steps_per_beat(&self) -> u32 {
        1
    }
    fn step(&mut self, step: u64, out: &mut Vec<Ev>) {
        let Ok(p) = self.pattern.try_lock() else { return };
        let col = (step % 4) as usize;
        let band = BANDS[self.band.load(Ordering::Relaxed) % BANDS.len()].1;
        for row in 0..4 {
            if p[row][col] {
                // The first beat of the bar a little louder, like a drummer.
                out.push(Ev::Drum(band[row], if col == 0 { 1.0 } else { 0.8 }));
            }
        }
    }
}

pub struct BugBeats {
    sound: Sound,
    pattern: Arc<Mutex<Pattern>>,
    band: Arc<AtomicUsize>,
    speed: usize,
    prev: [bool; 16],
    /// Bugs jumping (row, col) -> 0..1
    jump: [[f32; 4]; 4],
    last_step: Option<u64>,
    frame: u64,
}

impl BugBeats {
    pub fn new(sound: Sound) -> BugBeats {
        sound.set_reverb(0.12);
        sound.set_tempo(SPEEDS[1].1);
        BugBeats { sound, pattern: Arc::new(Mutex::new(starter())), band: Arc::new(AtomicUsize::new(0)), speed: 1, prev: [false; 16], jump: [[0.0; 4]; 4], last_step: None, frame: 0 }
    }

    fn band(&self) -> usize {
        self.band.load(Ordering::Relaxed) % BANDS.len()
    }

    fn column(&self) -> Option<usize> {
        self.sound.step().map(|s| (s % 4) as usize)
    }
}

fn bug_color(row: usize) -> Rgb565 {
    let (r, g, b) = BUG_COLORS[row];
    kit::rgb(r, g, b)
}

/// A round bug seen from above: shell, head, spots and wiggly legs.
fn draw_bug(fb: &mut FrameBuffer, x: i32, y: i32, row: usize, wiggle: i32) {
    let c = bug_color(row);
    let dark = Rgb565::new(3, 5, 4);
    for s in [-1, 1] {
        for k in 0..3 {
            let ly = y - 8 + k * 9;
            kit::line(fb, x + s * 14, ly, x + s * 26, ly + (k - 1) * 4 + wiggle * s, 3, dark);
        }
    }
    kit::circle(fb, x, y - 20, 10, dark);
    kit::line(fb, x - 4, y - 28, x - 10, y - 38, 2, dark);
    kit::line(fb, x + 4, y - 28, x + 10, y - 38, 2, dark);
    kit::circle(fb, x, y, 22, c);
    kit::line(fb, x, y - 20, x, y + 20, 2, dark);
    for (dx, dy) in [(-10, -6), (9, -2), (-8, 9), (10, 10)] {
        kit::circle(fb, x + dx, y + dy, 4, dark);
    }
    kit::circle(fb, x - 4, y - 22, 3, kit::WHITE);
    kit::circle(fb, x + 4, y - 22, 3, kit::WHITE);
}

impl App for BugBeats {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![("Band".into(), BANDS[self.band()].0.into(), false), ("Speed".into(), SPEEDS[self.speed].0.into(), false)]
    }
    fn running(&self) -> Option<bool> {
        Some(self.sound.playing())
    }
    fn toggle_running(&mut self) {
        if self.sound.playing() {
            self.sound.stop();
        } else {
            self.sound.start();
        }
    }
    fn on_exit(&mut self) {
        self.sound.stop();
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let p = *self.pattern.lock().unwrap();
        let col = self.column();
        std::array::from_fn(|i| {
            let (row, c) = (i / 4, i % 4);
            if p[row][c] {
                [PadColor::Yellow, PadColor::Green, PadColor::Blue, PadColor::Red][row]
            } else if col == Some(c) {
                PadColor::Green
            } else {
                PadColor::Off
            }
        })
    }

    fn tick(&mut self, input: &Input) {
        self.frame += 1;
        for i in 0..16 {
            if input.grid[i] && !self.prev[i] {
                let (row, col) = (i / 4, i % 4);
                let mut p = self.pattern.lock().unwrap();
                p[row][col] = !p[row][col];
                if p[row][col] {
                    // Hear the bug you just placed.
                    self.sound.drum(BANDS[self.band()].1[row], 0.8);
                    self.jump[row][col] = 1.0;
                }
            }
        }
        self.prev = input.grid;
        if input.knob2 != 0 {
            let next = (self.band() as i32 + input.knob2.signum()).rem_euclid(BANDS.len() as i32) as usize;
            self.band.store(next, Ordering::Relaxed);
        }
        if input.navigation_steps != 0 {
            self.speed = (self.speed as i32 - input.navigation_steps.signum()).clamp(0, 2) as usize;
            self.sound.set_tempo(SPEEDS[self.speed].1);
        }
        if input.knob2_press {
            *self.pattern.lock().unwrap() = [[false; 4]; 4];
        } else if input.knob1_press {
            self.toggle_running();
        }
        if let Some(step) = self.sound.step() {
            if self.last_step != Some(step) {
                let col = (step % 4) as usize;
                let p = *self.pattern.lock().unwrap();
                for row in 0..4 {
                    if p[row][col] {
                        self.jump[row][col] = 1.0;
                    }
                }
            }
            self.last_step = Some(step);
        } else {
            self.last_step = None;
        }
        for r in self.jump.iter_mut() {
            for j in r.iter_mut() {
                *j = (*j - 0.07).max(0.0);
            }
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let grass = kit::rgb(216, 233, 186);
        kit::clear(fb, grass);
        // Blades of grass.
        for k in 0..40 {
            let x = (k * 53 % 640) as i32;
            let y = 40 + (k * 97 % 300) as i32;
            kit::line(fb, x, y, x + 3, y - 8, 2, kit::rgb(194, 217, 160));
        }
        kit::kids_header(fb, NAME, "AGES 6-9");
        let p = *self.pattern.lock().unwrap();
        let col_now = self.column();
        let band = self.band();
        let (gx, gy, cw, ch) = (150, 40, 104, 64);
        if let Some(c) = col_now {
            kit::card(fb, gx + c as i32 * cw - 4, gy - 4, cw, ch * 4 + 4, 14, Rgb565::new(24, 56, 14));
        }
        for row in 0..4 {
            let y = gy + row as i32 * ch;
            // The sound's name on the left.
            kit::card(fb, 10, y + 10, 128, ch - 20, 10, kit::blend(bug_color(row), kit::BLACK, 0.3));
            kit::text(fb, BANDS[band].1[row].name(), 74, y + ch / 2 - 8, Size2::Medium, kit::WHITE, 0);
            for col in 0..4 {
                let x = gx + col as i32 * cw;
                kit::card(fb, x + 4, y + 4, cw - 16, ch - 8, 12, kit::PAPER);
                kit::ring(fb, x + cw / 2 - 6, y + ch / 2, 5, 1, kit::rgb(180, 198, 152));
                if p[row][col] {
                    let j = self.jump[row][col];
                    let hop = ((j * std::f32::consts::PI).sin() * 14.0) as i32;
                    let wiggle = if j > 0.0 { ((self.frame / 3) % 2) as i32 * 4 - 2 } else { 0 };
                    draw_bug(fb, x + cw / 2 - 6, y + ch / 2 + 6 - hop, row, wiggle);
                }
            }
        }
        // Beat numbers.
        for col in 0..4 {
            let lit = col_now == Some(col);
            kit::text(fb, &(col + 1).to_string(), gx + col as i32 * cw + cw / 2 - 6, gy + 4 * ch + 2, Size2::Medium, if lit { kit::WHITE } else { Rgb565::new(8, 26, 6) }, 0);
        }

        // Band and speed.
        kit::card(fb, 578, 50, 52, 140, 12, Rgb565::new(6, 20, 6));
        kit::text(fb, "band", 604, 58, Size2::Small, Rgb565::new(20, 50, 18), 0);
        kit::text(fb, BANDS[band].0, 604, 74, Size2::Small, kit::WHITE, 0);
        kit::text(fb, "< >", 604, 90, Size2::Small, Rgb565::new(20, 50, 18), 0);
        kit::text(fb, "speed", 604, 122, Size2::Small, Rgb565::new(20, 50, 18), 0);
        kit::text(fb, SPEEDS[self.speed].0, 604, 138, Size2::Small, kit::WHITE, 0);
        // Speed shown as one, two or three paw prints.
        for k in 0..=self.speed as i32 {
            kit::circle(fb, 592 + k * 12, 164, 4, kit::WHITE);
        }
        kit::text(fb, "up/dn", 604, 174, Size2::Small, Rgb565::new(20, 50, 18), 0);

        let play = if self.sound.playing() { "SELECT: stop" } else { "SELECT: play" };
        kit::kids_footer(fb, &format!("Tap a pad to add a bug, tap again to shoo it.   {play}   hold SELECT: clear"));
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(self.sound.processor(Some(Box::new(Beat { pattern: Arc::clone(&self.pattern), band: Arc::clone(&self.band) })), None))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(BugBeats::new(Sound::new(NAME, &modbus, &mixer, &bus)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render};

    #[test]
    fn a_pad_adds_a_bug_and_taps_again_shoo_it() {
        let mut app = BugBeats::new(Sound::detached());
        assert!(!app.pattern.lock().unwrap()[1][2]);
        let pad = Input { grid: std::array::from_fn(|g| g == 6), ..Default::default() };
        app.tick(&pad);
        app.tick(&Input::default());
        assert!(app.pattern.lock().unwrap()[1][2]);
        app.tick(&pad);
        assert!(!app.pattern.lock().unwrap()[1][2]);
    }

    #[test]
    fn play_grooves_and_an_empty_pattern_is_silent() {
        let mut app = BugBeats::new(Sound::detached());
        app.sound.set_reverb(0.0);
        let mut p = app.audio_processor().unwrap();
        app.tick(&Input { knob1_press: true, ..Default::default() });
        assert_eq!(app.running(), Some(true));
        let out = render(&mut p, 100);
        assert!(energy(&out) > 1e-4);
        app.tick(&Input::default());
        assert!(app.column().is_some());
        app.tick(&Input { knob2_press: true, ..Default::default() });
        render(&mut p, 100);
        assert!(energy(&render(&mut p, 100)) < 1e-9, "no bugs, no sound");
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }

    #[test]
    fn every_band_plays_its_own_sounds() {
        for band in 0..BANDS.len() {
            let snd = Sound::detached();
            let mut beat = Beat { pattern: Arc::new(Mutex::new([[true; 4]; 4])), band: Arc::new(AtomicUsize::new(band)) };
            let mut out = Vec::new();
            beat.step(0, &mut out);
            assert_eq!(out.len(), 4);
            for (row, ev) in out.iter().enumerate() {
                assert!(matches!(ev, Ev::Drum(d, _) if *d == BANDS[band].1[row]));
            }
            drop(snd);
        }
    }
}
