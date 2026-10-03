//! Copy Cat (Kids, ages 6-9): a memory game with a tune.
//!
//! The pads make four big coloured mats, one per corner (any of its
//! four pads counts): red, blue, yellow and green, each its own note.
//! The cat plays a tune on the mats; you copy it. Every tune you get
//! right earns a star, and the next tune is one note longer and a little
//! faster. Get one wrong and the cat shows it again; three misses and the
//! game ends, keeping your best score.
//!
//! The four notes are C, E, G and the C above (a major chord), so every
//! tune the cat makes sounds like a tune. SELECT (or F3) starts a game.

use crate::app::{App, Input, SlintExtra};
use crate::audio::AudioProcessor;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::apps::kids_kit::{self as kit, Note, Rng, Size2, Sound, Tone};
use embedded_graphics::pixelcolor::Rgb565;
use std::sync::Arc;

const NAME: &str = "Copy Cat";
/// C E G C: a major chord, so any tune of them sounds good.
const NOTES: [i32; 4] = [60, 64, 67, 72];
const LIVES: u32 = 3;
const FIRST_LENGTH: usize = 2;
const LONGEST: usize = 24;

const MAT_COLORS: [(u8, u8, u8); 4] = [(235, 70, 70), (70, 140, 240), (245, 205, 50), (80, 200, 100)];

/// Which mat (0 top-left red, 1 top-right blue, 2 bottom-left yellow,
/// 3 bottom-right green) a pad belongs to.
fn pad_mat(pad: usize) -> usize {
    let (row, col) = (pad / 4, pad % 4);
    (row / 2) * 2 + col / 2
}

fn mat_color(m: usize) -> Rgb565 {
    let (r, g, b) = MAT_COLORS[m];
    kit::rgb(r, g, b)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Idle,
    /// The cat is playing note `i` of the tune; `t` frames into it.
    Showing { i: usize, t: u32 },
    /// Your turn: you've copied `i` notes so far.
    Copying { i: usize },
    Yay { t: u32 },
    Oops { t: u32 },
    Over { t: u32 },
}

pub struct CopyCat {
    sound: Sound,
    rng: Rng,
    tune: Vec<usize>,
    state: State,
    score: u32,
    best: u32,
    lives: u32,
    prev: [bool; 16],
    /// How lit each mat is, 0..1.
    glow: [f32; 4],
    frame: u64,
}

impl CopyCat {
    pub fn new(sound: Sound, rng: Rng) -> CopyCat {
        sound.set_reverb(0.2);
        CopyCat { sound, rng, tune: Vec::new(), state: State::Idle, score: 0, best: 0, lives: LIVES, prev: [false; 16], glow: [0.0; 4], frame: 0 }
    }

    /// Frames per note while the cat plays: quicker as the tune grows.
    fn note_frames(&self) -> u32 {
        (38 - (self.tune.len() as u32).saturating_sub(FIRST_LENGTH as u32) * 2).max(18)
    }

    fn start(&mut self) {
        self.score = 0;
        self.lives = LIVES;
        self.tune.clear();
        for _ in 0..FIRST_LENGTH {
            let m = self.rng.below(4) as usize;
            self.tune.push(m);
        }
        self.state = State::Showing { i: 0, t: 0 };
    }

    fn ring(&mut self, mat: usize) {
        self.sound.play(Note::new(Tone::Marimba, NOTES[mat] as f32).vel(0.9).pan([-0.5, 0.5, -0.5, 0.5][mat]));
        self.sound.play(Note::new(Tone::Glock, NOTES[mat] as f32 + 12.0).vel(0.25).pan([-0.5, 0.5, -0.5, 0.5][mat]));
        self.glow[mat] = 1.0;
    }

    fn fanfare(&self) {
        for (k, n) in [72, 76, 79, 84].iter().enumerate() {
            // Staggered with growing note lengths: they sound as a quick
            // rising arpeggio held together.
            self.sound.play(Note::new(Tone::Glock, *n as f32).vel(0.5 + k as f32 * 0.1).len(0.2 + k as f32 * 0.1));
        }
    }

    fn oops_sound(&self) {
        self.sound.play(Note::new(Tone::Soft, 55.0).vel(0.6).len(0.25));
        self.sound.play(Note::new(Tone::Soft, 54.0).vel(0.5).len(0.5));
    }

    fn press(&mut self, mat: usize) {
        match self.state {
            State::Copying { i } => {
                self.ring(mat);
                if self.tune[i] == mat {
                    if i + 1 == self.tune.len() {
                        self.score += 1;
                        self.best = self.best.max(self.score);
                        self.state = State::Yay { t: 0 };
                    } else {
                        self.state = State::Copying { i: i + 1 };
                    }
                } else {
                    self.lives -= 1;
                    self.oops_sound();
                    self.state = if self.lives == 0 { State::Over { t: 0 } } else { State::Oops { t: 0 } };
                }
            }
            // Pads are free to play while nothing is being asked.
            State::Idle | State::Over { .. } => self.ring(mat),
            _ => {}
        }
    }

    fn message(&self) -> (&'static str, &'static str) {
        match self.state {
            State::Idle => ("Copy me!", "Press SELECT to play"),
            State::Showing { .. } => ("Watch me...", "listen to my tune"),
            State::Copying { .. } => ("Your turn!", "play it back"),
            State::Yay { .. } => ("Yay! A star!", "one more note now"),
            State::Oops { .. } => ("Oops!", "watch again"),
            State::Over { .. } => ("Good game!", "SELECT: play again"),
        }
    }
}

impl App for CopyCat {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![("Stars".into(), self.score.to_string(), false), ("Best".into(), self.best.to_string(), false), ("Tries left".into(), self.lives.to_string(), false)]
    }
    fn running(&self) -> Option<bool> {
        Some(!matches!(self.state, State::Idle | State::Over { .. }))
    }
    fn toggle_running(&mut self) {
        if matches!(self.state, State::Idle | State::Over { .. }) {
            self.start();
        } else {
            self.state = State::Idle;
        }
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        const C: [PadColor; 4] = [PadColor::Red, PadColor::Blue, PadColor::Yellow, PadColor::Green];
        std::array::from_fn(|p| C[pad_mat(p)])
    }

    fn tick(&mut self, input: &Input) {
        self.frame += 1;
        let mut pressed = [false; 4];
        for p in 0..16 {
            if input.grid[p] && !self.prev[p] {
                pressed[pad_mat(p)] = true;
            }
        }
        self.prev = input.grid;
        for (m, &on) in pressed.iter().enumerate() {
            if on {
                self.press(m);
            }
        }
        if input.knob1_press && matches!(self.state, State::Idle | State::Over { .. }) {
            self.start();
        }
        let nf = self.note_frames();
        self.state = match self.state {
            State::Showing { i, t } => {
                if t == 10 {
                    let m = self.tune[i];
                    self.ring(m);
                }
                if t + 1 >= nf + 10 {
                    if i + 1 < self.tune.len() {
                        State::Showing { i: i + 1, t: 10 }
                    } else {
                        State::Copying { i: 0 }
                    }
                } else {
                    State::Showing { i, t: t + 1 }
                }
            }
            State::Yay { t } => {
                if t == 12 {
                    self.fanfare();
                }
                if t > 75 {
                    if self.tune.len() < LONGEST {
                        let m = self.rng.below(4) as usize;
                        self.tune.push(m);
                    }
                    State::Showing { i: 0, t: 0 }
                } else {
                    State::Yay { t: t + 1 }
                }
            }
            State::Oops { t } => {
                if t > 70 {
                    State::Showing { i: 0, t: 0 }
                } else {
                    State::Oops { t: t + 1 }
                }
            }
            State::Over { t } => State::Over { t: t.saturating_add(1) },
            s => s,
        };
        for g in self.glow.iter_mut() {
            *g = (*g - 0.05).max(0.0);
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = Rgb565::new(25, 50, 26);
        kit::clear(fb, bg);
        kit::header(fb, NAME, "AGES 6-9", Rgb565::new(12, 20, 16), kit::WHITE);

        // The four mats, laid out like the pads.
        for m in 0..4 {
            let x = 24 + (m as i32 % 2) * 152;
            let y = 44 + (m as i32 / 2) * 150;
            let g = self.glow[m];
            let c = kit::blend(kit::blend(mat_color(m), kit::BLACK, 0.25), kit::WHITE, g * 0.7);
            let grow = (g * 6.0) as i32;
            kit::round_rect(fb, x + 4, y + 6, 140, 138, 22, kit::blend(bg, kit::BLACK, 0.25));
            kit::round_rect(fb, x - grow, y - grow, 140 + 2 * grow, 138 + 2 * grow, 22, c);
            if g > 0.3 {
                kit::star(fb, x + 70, y + 69, 28 + (g * 10.0) as i32, kit::WHITE);
            }
        }

        // The cat, and what it says.
        let (big, small) = self.message();
        let cx = 476;
        let cy = 210;
        let showing = matches!(self.state, State::Showing { .. });
        let happy = matches!(self.state, State::Yay { .. });
        let sad = matches!(self.state, State::Oops { .. });
        let fur = Rgb565::new(30, 42, 12);
        let bob = if happy { ((self.frame as f32 * 0.4).sin() * 6.0) as i32 } else { 0 };
        // Tail.
        kit::round_rect(fb, cx + 64, cy + 24 + bob, 16, 70, 8, fur);
        kit::round_rect(fb, cx - 60, cy + 30 + bob, 120, 90, 40, fur);
        kit::triangle(fb, [(cx - 64, cy - 30 + bob), (cx - 54, cy - 94 + bob), (cx - 14, cy - 66 + bob)], fur);
        kit::triangle(fb, [(cx + 64, cy - 30 + bob), (cx + 54, cy - 94 + bob), (cx + 14, cy - 66 + bob)], fur);
        kit::triangle(fb, [(cx - 54, cy - 46 + bob), (cx - 50, cy - 80 + bob), (cx - 26, cy - 62 + bob)], Rgb565::new(31, 40, 24));
        kit::triangle(fb, [(cx + 54, cy - 46 + bob), (cx + 50, cy - 80 + bob), (cx + 26, cy - 62 + bob)], Rgb565::new(31, 40, 24));
        kit::circle(fb, cx, cy - 22 + bob, 64, fur);
        // Eyes: closed happy arcs, or looking at the mat being played.
        let look = match self.state {
            State::Showing { i, .. } => [(-4, -4), (4, -4), (-4, 4), (4, 4)][self.tune[i]],
            _ => (0, 0),
        };
        for s in [-1, 1] {
            let ex = cx + s * 24;
            let ey = cy - 34 + bob;
            if happy {
                kit::ring(fb, ex, ey + 6, 10, 4, kit::BLACK);
                kit::rect(fb, ex - 12, ey + 6, 24, 12, fur);
            } else {
                kit::circle(fb, ex, ey, 14, kit::WHITE);
                kit::circle(fb, ex + look.0, ey + look.1, 8, Rgb565::new(4, 20, 6));
                kit::circle(fb, ex + look.0 + 3, ey + look.1 - 3, 3, kit::WHITE);
            }
        }
        kit::triangle(fb, [(cx - 8, cy - 12 + bob), (cx + 8, cy - 12 + bob), (cx, cy - 4 + bob)], Rgb565::new(31, 30, 24));
        for s in [-1, 1] {
            kit::line(fb, cx + s * 18, cy - 6 + bob, cx + s * 64, cy - 14 + bob, 2, kit::BLACK);
            kit::line(fb, cx + s * 18, cy - 1 + bob, cx + s * 64, cy + 2 + bob, 2, kit::BLACK);
        }
        if sad {
            kit::ring(fb, cx, cy + 14 + bob, 10, 3, kit::BLACK);
            kit::rect(fb, cx - 12, cy + 14 + bob, 24, 12, fur);
        } else {
            let open = if showing { 4 + (self.glow.iter().cloned().fold(0.0, f32::max) * 10.0) as i32 } else { 4 };
            kit::round_rect(fb, cx - 8, cy + 2 + bob, 16, open, 3, Rgb565::new(18, 6, 8));
        }

        // Speech bubble.
        kit::round_rect(fb, 330, 44, 290, 66, 18, kit::WHITE);
        kit::triangle(fb, [(440, 108), (470, 108), (452, 126)], kit::WHITE);
        kit::text(fb, big, 475, 52, Size2::Large, Rgb565::new(6, 10, 10), 0);
        kit::text(fb, small, 475, 88, Size2::Small, Rgb565::new(10, 20, 16), 0);

        // Stars and tries.
        let stars = self.score.min(10) as i32;
        for k in 0..stars {
            kit::star(fb, 344 + k * 26, 318, 11, Rgb565::new(31, 52, 4));
        }
        if self.score > 10 {
            kit::text(fb, &format!("x{}", self.score), 344 + 10 * 26, 310, Size2::Medium, kit::BLACK, -1);
        }
        for k in 0..LIVES as i32 {
            let c = if (k as u32) < self.lives { Rgb565::new(30, 12, 12) } else { Rgb565::new(18, 34, 18) };
            kit::circle(fb, 586 - k * 26 - 5, 336, 7, c);
            kit::circle(fb, 586 - k * 26 + 5, 336, 7, c);
            kit::triangle(fb, [(586 - k * 26 - 12, 338), (586 - k * 26 + 12, 338), (586 - k * 26, 352)], c);
        }
        kit::text(fb, &format!("best {}", self.best), 344, 336, Size2::Small, Rgb565::new(8, 18, 12), -1);
        if let State::Copying { i } = self.state {
            kit::text(fb, &format!("{} of {}", i, self.tune.len()), 476, 140, Size2::Medium, Rgb565::new(6, 14, 10), 0);
        }
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(self.sound.processor(None, None))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(CopyCat::new(Sound::new(NAME, &modbus, &mixer, &bus), Rng::seeded_from_time()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render};

    fn mat_pad(m: usize) -> Input {
        // The top-left pad of mat m.
        let pad = (m / 2) * 8 + (m % 2) * 2;
        Input { grid: std::array::from_fn(|g| g == pad), ..Default::default() }
    }

    fn wait_for_turn(app: &mut CopyCat) {
        for _ in 0..2000 {
            if matches!(app.state, State::Copying { .. }) {
                return;
            }
            app.tick(&Input::default());
        }
        panic!("never my turn: {:?}", app.state);
    }

    #[test]
    fn every_pad_belongs_to_its_corner() {
        assert_eq!(pad_mat(0), 0);
        assert_eq!(pad_mat(3), 1);
        assert_eq!(pad_mat(12), 2);
        assert_eq!(pad_mat(15), 3);
        assert_eq!((0..16).filter(|&p| pad_mat(p) == 3).count(), 4);
    }

    #[test]
    fn copying_right_earns_a_star_and_a_longer_tune() {
        let mut app = CopyCat::new(Sound::detached(), Rng::new(7));
        let mut p = app.audio_processor().unwrap();
        app.tick(&Input { knob1_press: true, ..Default::default() });
        wait_for_turn(&mut app);
        assert!(energy(&render(&mut p, 5)) > 1e-6, "the cat played");
        let tune = app.tune.clone();
        assert_eq!(tune.len(), FIRST_LENGTH);
        for &m in &tune {
            app.tick(&mat_pad(m));
            app.tick(&Input::default());
        }
        assert_eq!(app.score, 1);
        wait_for_turn(&mut app);
        assert_eq!(app.tune.len(), FIRST_LENGTH + 1);
        assert_eq!(&app.tune[..FIRST_LENGTH], &tune[..], "the same tune plus one note");
    }

    #[test]
    fn three_misses_end_the_game() {
        let mut app = CopyCat::new(Sound::detached(), Rng::new(3));
        app.toggle_running();
        for k in 0..3 {
            wait_for_turn(&mut app);
            let wrong = (app.tune[0] + 1) % 4;
            app.tick(&mat_pad(wrong));
            app.tick(&Input::default());
            assert_eq!(app.lives, 2 - k);
        }
        assert!(matches!(app.state, State::Over { .. }));
        assert_eq!(app.running(), Some(false));
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }
}
