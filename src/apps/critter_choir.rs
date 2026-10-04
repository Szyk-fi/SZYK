//! Critter Choir (Kids, ages 6-9): a choir of animals, four at a time.
//!
//! Each column of pads is one critter, its four pads four notes (bottom
//! low, top high), all from the major pentatonic so any mix sounds good.
//! Left/right swaps between the farm choir (cow, dog, duck, cat) and the
//! pond choir (frog, owl, bee, bird); each is arranged low voice to high
//! voice, left to right, like a real choir. F3 makes the choir sing a
//! little song by itself, which you can join in with.
//!
//! The voices are synthesized, not recorded: a buzzy source (the
//! "vocal cords", a band-limited sawtooth plus breath noise) through two
//! band-pass filters at the formants of a vowel, the way a voice is made.
//! Each animal has its own shape over time: a cow's "mmm-oo" opens from
//! a hum, a cat's "mee-ow" sweeps its formants from "ee" to "ah" to "oo"
//! while its pitch rises and falls, a frog's croak is chopped 25 times a
//! second, a bird's tweet is a fast upward chirp with a trill.

use crate::app::{App, Input, SlintExtra};
use crate::audio::AudioProcessor;
use crate::display::{FrameBuffer, WIDTH};
use crate::led_output::PadColor;
use crate::apps::kids_kit::{self as kit, Ev, Extra, Noise, Size2, Song, Sound, Svf};
use embedded_graphics::pixelcolor::Rgb565;
use std::sync::Arc;

const NAME: &str = "Critter Choir";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Critter {
    Cow,
    Dog,
    Cat,
    Duck,
    Frog,
    Owl,
    Bee,
    Bird,
}

const CHOIRS: [[Critter; 4]; 2] = [[Critter::Cow, Critter::Dog, Critter::Duck, Critter::Cat], [Critter::Frog, Critter::Owl, Critter::Bee, Critter::Bird]];
const CHOIR_NAMES: [&str; 2] = ["The Farm Choir", "The Pond Choir"];

impl Critter {
    fn from_u32(n: u32) -> Critter {
        [Critter::Cow, Critter::Dog, Critter::Cat, Critter::Duck, Critter::Frog, Critter::Owl, Critter::Bee, Critter::Bird][n as usize % 8]
    }
    fn name(self) -> &'static str {
        match self {
            Critter::Cow => "Cow",
            Critter::Dog => "Dog",
            Critter::Cat => "Cat",
            Critter::Duck => "Duck",
            Critter::Frog => "Frog",
            Critter::Owl => "Owl",
            Critter::Bee => "Bee",
            Critter::Bird => "Bird",
        }
    }
    /// The lowest of its four notes (MIDI), so the choir runs low to high.
    fn base(self) -> i32 {
        match self {
            Critter::Cow => 43,
            Critter::Dog => 52,
            Critter::Cat => 64,
            Critter::Duck => 60,
            Critter::Frog => 45,
            Critter::Owl => 57,
            Critter::Bee => 60,
            Critter::Bird => 79,
        }
    }
    /// Evens out the calls' loudness: the narrow formant filters make
    /// some voices far louder than others (measured peaks at velocity
    /// 0.9, scaled to about 0.3).
    fn gain(self) -> f32 {
        match self {
            Critter::Cow => 0.12,
            Critter::Dog => 0.12,
            Critter::Cat => 0.1,
            Critter::Duck => 0.42,
            Critter::Frog => 0.18,
            Critter::Owl => 0.13,
            Critter::Bee => 0.19,
            Critter::Bird => 1.1,
        }
    }
    /// How long one call lasts, seconds.
    fn length(self) -> f32 {
        match self {
            Critter::Cow => 0.9,
            Critter::Dog => 0.28,
            Critter::Cat => 0.7,
            Critter::Duck => 0.3,
            Critter::Frog => 0.45,
            Critter::Owl => 0.75,
            Critter::Bee => 0.6,
            Critter::Bird => 0.22,
        }
    }
}

/// The pentatonic note a critter sings for row `row` (0 = top pad).
fn critter_note(c: Critter, row: usize) -> i32 {
    // Snap the base onto the C pentatonic, then step up the scale.
    let mut d = 0;
    while kit::scale_note(36, &kit::PENTATONIC, d) < c.base() {
        d += 1;
    }
    kit::scale_note(36, &kit::PENTATONIC, d + (3 - row as i32))
}

/// One animal call: where its pitch, vowel and loudness are at time `t`
/// (seconds) as a fraction `x` = t / length.
struct Shape {
    /// Pitch multiplier on the note.
    pitch: f32,
    f1: f32,
    f2: f32,
    amp: f32,
    /// 0 = all voice, 1 = all breath.
    noise: f32,
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn shape(c: Critter, t: f32) -> Shape {
    let x = (t / c.length()).clamp(0.0, 1.0);
    let fade = |a: f32, r: f32| (t / a).min(1.0) * (1.0 - smooth((x - (1.0 - r)) / r));
    match c {
        // "Mmm-ooo": closed lips (a hum, formants low) opening to "oo",
        // sagging in pitch at the end.
        Critter::Cow => Shape { pitch: 1.0 - 0.08 * smooth((x - 0.6) / 0.4), f1: 250.0 + 120.0 * smooth(x / 0.3), f2: 600.0 + 300.0 * smooth(x / 0.3), amp: fade(0.08, 0.35), noise: 0.05 },
        // "Woof": "w" (oo) to "o" fast, falling.
        Critter::Dog => Shape { pitch: 1.15 - 0.3 * x, f1: 350.0 + 400.0 * smooth(x / 0.4), f2: 800.0 + 400.0 * smooth(x / 0.4), amp: fade(0.01, 0.6), noise: 0.25 },
        // "Mee-ow": ee -> ah -> oo, pitch up then down.
        Critter::Cat => {
            let (f1, f2) = if x < 0.45 { (300.0 + 450.0 * smooth(x / 0.45), 2300.0 - 1100.0 * smooth(x / 0.45)) } else { (750.0 - 400.0 * smooth((x - 0.45) / 0.55), 1200.0 - 450.0 * smooth((x - 0.45) / 0.55)) };
            Shape { pitch: 1.0 + 0.12 * (x * std::f32::consts::PI).sin(), f1, f2, amp: fade(0.05, 0.3), noise: 0.08 }
        }
        // "Quack": nasal and buzzy, high formants, quick.
        Critter::Duck => Shape { pitch: 1.0 - 0.15 * x, f1: 1100.0, f2: 2500.0, amp: fade(0.01, 0.5), noise: 0.15 },
        // "Rib-bit": two croaks, the voice chopped 25 times a second.
        Critter::Frog => {
            let gap = (0.42..0.55).contains(&x);
            let chop = if ((t * 25.0).fract()) < 0.55 { 1.0 } else { 0.15 };
            Shape { pitch: if x < 0.5 { 1.0 } else { 1.06 }, f1: 500.0, f2: 1400.0, amp: if gap { 0.0 } else { fade(0.01, 0.15) * chop }, noise: 0.1 }
        }
        // "Hoo": round and breathy, gliding down a little.
        Critter::Owl => Shape { pitch: 1.0 - 0.04 * x, f1: 300.0, f2: 700.0, amp: fade(0.12, 0.45), noise: 0.35 },
        // "Bzzz": a flutter of wings, wobbling in pitch.
        Critter::Bee => Shape { pitch: 1.0 + 0.03 * (t * 52.0).sin(), f1: 600.0 + 150.0 * (t * 9.0).sin(), f2: 1800.0, amp: fade(0.05, 0.3) * (0.75 + 0.25 * (t * 140.0).sin()), noise: 0.1 },
        // "Tweet": a fast chirp up with a trill.
        Critter::Bird => Shape { pitch: 1.0 + 0.5 * smooth(x / 0.7) + 0.03 * (t * 190.0).sin(), f1: 2000.0, f2: 4000.0, amp: fade(0.005, 0.4), noise: 0.0 },
    }
}

struct Call {
    critter: Critter,
    hz: f32,
    vel: f32,
    pan: f32,
    t: f32,
    phase: f32,
    f1: Svf,
    f2: Svf,
    tone: Svf,
    noise: Noise,
}

/// The critters' voices, mixed into the kit's sound.
struct Voices {
    calls: Vec<Call>,
    seed: u32,
}

impl Extra for Voices {
    fn event(&mut self, a: u32, b: f32, c: f32) {
        let critter = Critter::from_u32(a);
        if self.calls.len() >= 12 {
            self.calls.remove(0);
        }
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let pan = [-0.6, -0.2, 0.2, 0.6][(a % 4) as usize];
        self.calls.push(Call { critter, hz: kit::midi_hz(b), vel: c, pan, t: 0.0, phase: 0.0, f1: Svf::default(), f2: Svf::default(), tone: Svf::default(), noise: Noise::new(self.seed) });
    }

    fn frame(&mut self, sr: f32) -> (f32, f32) {
        let (mut l, mut r) = (0.0, 0.0);
        for c in self.calls.iter_mut() {
            let s = shape(c.critter, c.t);
            let hz = c.hz * s.pitch;
            let dt = hz / sr;
            let p = c.phase;
            c.phase = (p + dt).fract();
            let x = if c.critter == Critter::Bird {
                // A bird's syrinx makes an almost pure whistle.
                (p * std::f32::consts::TAU).sin()
            } else {
                let buzz = c.tone.tick(kit::saw(p, dt), (hz * 6.0).min(6000.0), 0.7, sr).lp;
                buzz * (1.0 - s.noise) + c.noise.next() * s.noise * 0.6
            };
            let y = if c.critter == Critter::Bird {
                x
            } else {
                let a = c.f1.tick(x, s.f1, 5.0, sr).bp;
                let b = c.f2.tick(x, s.f2, 6.0, sr).bp;
                (a + 0.6 * b) * 1.8
            };
            let v = y * s.amp * c.vel * c.critter.gain();
            let (gl, gr) = kit::pan_gains(c.pan);
            l += v * gl;
            r += v * gr;
            c.t += 1.0 / sr;
        }
        self.calls.retain(|c| c.t < c.critter.length() + 0.05);
        (l, r)
    }
}

/// The choir's own song: (eighth-note step, column, row). An original
/// call-and-response tune that passes the melody along the choir.
const SONG: [(u64, usize, usize); 20] = [
    (0, 2, 2), (2, 2, 1), (4, 2, 0), (6, 1, 1),
    (8, 3, 3), (9, 3, 2), (10, 3, 1), (12, 0, 2),
    (16, 2, 2), (18, 2, 1), (20, 1, 0), (22, 1, 1),
    (24, 3, 1), (25, 3, 2), (26, 3, 3), (28, 0, 3),
    (29, 1, 3), (30, 2, 3), (31, 0, 0), (31, 3, 3),
];
const SONG_STEPS: u64 = 32;

struct Choir {
    choir: Arc<std::sync::atomic::AtomicUsize>,
}

impl Song for Choir {
    fn steps_per_beat(&self) -> u32 {
        2
    }
    fn step(&mut self, step: u64, out: &mut Vec<Ev>) {
        let choir = self.choir.load(std::sync::atomic::Ordering::Relaxed) % 2;
        for &(s, col, row) in SONG.iter() {
            if s == step % SONG_STEPS {
                let c = CHOIRS[choir][col];
                out.push(Ev::Custom(c as u32, critter_note(c, row) as f32, 0.8));
            }
        }
    }
}

pub struct CritterChoir {
    sound: Sound,
    choir: Arc<std::sync::atomic::AtomicUsize>,
    prev: [bool; 16],
    /// How open each column's mouth is, 0..1, and which row sang last.
    mouth: [f32; 4],
    sang_row: [usize; 4],
    notes: Vec<(f32, f32, usize, f32)>,
    last_step: Option<u64>,
    frame: u64,
}

impl CritterChoir {
    pub fn new(sound: Sound) -> CritterChoir {
        sound.set_tempo(96.0);
        sound.set_reverb(0.22);
        CritterChoir { sound, choir: Arc::new(std::sync::atomic::AtomicUsize::new(0)), prev: [false; 16], mouth: [0.0; 4], sang_row: [0; 4], notes: Vec::new(), last_step: None, frame: 0 }
    }

    fn choir_index(&self) -> usize {
        self.choir.load(std::sync::atomic::Ordering::Relaxed) % 2
    }

    fn sing(&mut self, col: usize, row: usize, live: bool) {
        let c = CHOIRS[self.choir_index()][col];
        if live {
            self.sound.custom(c as u32, critter_note(c, row) as f32, 0.9);
        }
        self.mouth[col] = 1.0;
        self.sang_row[col] = row;
        if self.notes.len() < 30 {
            self.notes.push((col_x(col) as f32 + 40.0, 120.0, row, 0.0));
        }
    }
}

fn col_x(col: usize) -> i32 {
    80 + col as i32 * 160
}

impl App for CritterChoir {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![("Choir".into(), CHOIR_NAMES[self.choir_index()].into(), false), ("Song".into(), if self.sound.playing() { "singing" } else { "F3" }.into(), false)]
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
    fn transport_action(&self) -> Option<&'static str> {
        Some(if self.sound.playing() { "STOP" } else { "SING" })
    }
    fn on_exit(&mut self) {
        self.sound.stop();
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        const COL: [PadColor; 4] = [PadColor::Red, PadColor::Yellow, PadColor::Green, PadColor::Blue];
        std::array::from_fn(|p| COL[p % 4])
    }

    fn tick(&mut self, input: &Input) {
        self.frame += 1;
        for p in 0..16 {
            if input.grid[p] && !self.prev[p] {
                self.sing(p % 4, p / 4, true);
            }
        }
        self.prev = input.grid;
        if input.knob2 != 0 || input.knob1_press {
            let next = (self.choir_index() + 1) % 2;
            self.choir.store(next, std::sync::atomic::Ordering::Relaxed);
        }
        // Animate the song's singers.
        if let Some(step) = self.sound.step() {
            let from = match self.last_step {
                Some(l) if l <= step => l + 1,
                _ => step,
            };
            for s in from.max(step.saturating_sub(8))..=step {
                for &(at, col, row) in SONG.iter() {
                    if at == s % SONG_STEPS {
                        self.sing(col, row, false);
                    }
                }
            }
            self.last_step = Some(step);
        } else {
            self.last_step = None;
        }
        for m in self.mouth.iter_mut() {
            *m = (*m - 0.035).max(0.0);
        }
        for n in self.notes.iter_mut() {
            n.1 -= 1.4;
            n.0 += (n.3 * 0.15).sin() * 0.8;
            n.3 += 1.0;
        }
        self.notes.retain(|n| n.3 < 60.0);
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let choir = self.choir_index();
        let (sky, ground) = if choir == 0 { (kit::rgb(207, 232, 236), kit::rgb(109, 157, 98)) } else { (kit::rgb(215, 231, 221), kit::rgb(73, 142, 126)) };
        kit::clear(fb, sky);
        kit::rect(fb, 0, 250, WIDTH as i32, 110, ground);
        if choir == 1 {
            // The pond.
            kit::round_rect(fb, 40, 268, 560, 50, 25, Rgb565::new(8, 30, 26));
        } else {
            // A fence.
            for k in 0..17 {
                kit::rect(fb, 10 + k * 38, 214, 8, 44, Rgb565::new(24, 44, 18));
            }
            kit::rect(fb, 0, 222, WIDTH as i32, 6, Rgb565::new(24, 44, 18));
            kit::rect(fb, 0, 242, WIDTH as i32, 6, Rgb565::new(24, 44, 18));
        }
        for (x, y) in [(82, 92), (290, 96)] {
            kit::round_rect(fb, x - 30, y, 72, 16, 8, kit::PAPER);
            kit::circle(fb, x, y, 16, kit::PAPER);
            kit::circle(fb, x + 20, y + 2, 12, kit::PAPER);
        }
        kit::circle(fb, 580, 70, 28, Rgb565::new(31, 56, 10));
        kit::kids_header(fb, NAME, "AGES 6-9");
        kit::text(fb, CHOIR_NAMES[choir], 320, 40, Size2::Large, Rgb565::new(4, 14, 10), 0);

        for col in 0..4 {
            let c = CHOIRS[choir][col];
            let x = col_x(col);
            let m = self.mouth[col];
            let hop = (m * m * 10.0) as i32;
            draw_critter(fb, c, x, 190 - hop, m);
            kit::text(fb, c.name(), x, 262, Size2::Medium, kit::WHITE, 0);
            // Its four pads, top to bottom, the lit one being sung.
            for row in 0..4 {
                let lit = m > 0.2 && self.sang_row[col] == row;
                let colr = if lit { kit::WHITE } else { kit::blend(kit::rainbow(col * 2), kit::BLACK, 0.2) };
                kit::round_rect(fb, x - 34 + row as i32 * 18, 286, 14, 14, 4, colr);
            }
        }
        for &(x, y, row, age) in &self.notes {
            let c = kit::blend(kit::rainbow(row * 2 + 1), kit::WHITE, age / 60.0);
            kit::circle(fb, x as i32, y as i32, 6, c);
            kit::rect(fb, x as i32 + 4, y as i32 - 18, 3, 18, c);
        }
        kit::kids_footer(fb, "Each column of pads is a critter (low notes at the bottom).   < >: other choir   F3: sing a song");
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(self.sound.processor(Some(Box::new(Choir { choir: Arc::clone(&self.choir) })), Some(Box::new(Voices { calls: Vec::with_capacity(12), seed: 12345 }))))
    }
}

/// Eyes with a shine, looking slightly up.
fn eyes(fb: &mut FrameBuffer, x: i32, y: i32, gap: i32, r: i32) {
    for s in [-1, 1] {
        kit::circle(fb, x + s * gap, y, r, kit::WHITE);
        kit::circle(fb, x + s * gap, y - 1, r * 3 / 5, kit::BLACK);
        kit::circle(fb, x + s * gap + r / 4, y - r / 3, (r / 4).max(1), kit::WHITE);
    }
}

/// An open mouth: wider and taller the louder the critter sings.
fn mouth(fb: &mut FrameBuffer, x: i32, y: i32, w: i32, open: f32, c: Rgb565) {
    let h = 3 + (open * 16.0) as i32;
    kit::round_rect(fb, x - w / 2, y, w, h, (h / 2).max(1) as u32, c);
}

fn draw_critter(fb: &mut FrameBuffer, c: Critter, x: i32, y: i32, open: f32) {
    let dark = Rgb565::new(6, 6, 6);
    match c {
        Critter::Cow => {
            let body = Rgb565::new(30, 60, 29);
            kit::triangle(fb, [(x - 40, y - 46), (x - 52, y - 72), (x - 30, y - 50)], Rgb565::new(28, 54, 18));
            kit::triangle(fb, [(x + 40, y - 46), (x + 52, y - 72), (x + 30, y - 50)], Rgb565::new(28, 54, 18));
            kit::circle(fb, x, y - 20, 46, body);
            kit::circle(fb, x - 22, y - 42, 12, dark);
            kit::circle(fb, x + 28, y - 8, 10, dark);
            kit::round_rect(fb, x - 30, y, 60, 34, 16, Rgb565::new(31, 44, 24));
            kit::circle(fb, x - 10, y + 10, 3, dark);
            kit::circle(fb, x + 10, y + 10, 3, dark);
            eyes(fb, x, y - 30, 16, 8);
            mouth(fb, x, y + 18, 20, open, Rgb565::new(18, 8, 8));
        }
        Critter::Dog => {
            let body = Rgb565::new(24, 40, 14);
            kit::round_rect(fb, x - 54, y - 56, 24, 60, 12, Rgb565::new(16, 26, 8));
            kit::round_rect(fb, x + 30, y - 56, 24, 60, 12, Rgb565::new(16, 26, 8));
            kit::circle(fb, x, y - 22, 40, body);
            kit::round_rect(fb, x - 22, y - 8, 44, 30, 14, Rgb565::new(29, 52, 22));
            kit::circle(fb, x, y - 6, 7, dark);
            eyes(fb, x, y - 32, 15, 8);
            mouth(fb, x, y + 6, 18, open, Rgb565::new(20, 8, 10));
            if open > 0.3 {
                kit::round_rect(fb, x - 5, y + 12, 10, 12, 5, Rgb565::new(31, 30, 20));
            }
        }
        Critter::Cat => {
            let body = Rgb565::new(30, 40, 10);
            kit::triangle(fb, [(x - 38, y - 30), (x - 30, y - 76), (x - 6, y - 52)], body);
            kit::triangle(fb, [(x + 38, y - 30), (x + 30, y - 76), (x + 6, y - 52)], body);
            kit::circle(fb, x, y - 22, 40, body);
            for s in [-1, 1] {
                kit::line(fb, x + s * 14, y - 8, x + s * 48, y - 14, 2, dark);
                kit::line(fb, x + s * 14, y - 4, x + s * 48, y, 2, dark);
            }
            kit::triangle(fb, [(x - 6, y - 14), (x + 6, y - 14), (x, y - 8)], Rgb565::new(31, 30, 24));
            eyes(fb, x, y - 32, 15, 9);
            mouth(fb, x, y - 2, 14, open, Rgb565::new(20, 8, 10));
        }
        Critter::Duck => {
            let body = Rgb565::new(31, 58, 8);
            kit::circle(fb, x, y - 24, 38, body);
            kit::round_rect(fb, x - 26, y - 16, 52, 14, 7, Rgb565::new(31, 36, 4));
            // The bill opens: the lower half drops.
            let drop = (open * 14.0) as i32;
            kit::round_rect(fb, x - 22, y - 2 + drop, 44, 12, 6, Rgb565::new(29, 30, 2));
            if drop > 2 {
                kit::rect(fb, x - 18, y - 4, 36, drop + 2, Rgb565::new(16, 8, 6));
            }
            eyes(fb, x, y - 38, 14, 7);
        }
        Critter::Frog => {
            let body = Rgb565::new(10, 48, 10);
            kit::circle(fb, x - 24, y - 46, 18, body);
            kit::circle(fb, x + 24, y - 46, 18, body);
            kit::round_rect(fb, x - 50, y - 46, 100, 70, 34, body);
            eyes(fb, x, y - 48, 24, 11);
            // The throat sac puffs out on each croak.
            kit::circle(fb, x, y + 12, 8 + (open * 18.0) as i32, Rgb565::new(24, 56, 18));
            kit::rect(fb, x - 34, y - 6, 68, 4, Rgb565::new(4, 24, 4));
        }
        Critter::Owl => {
            let body = Rgb565::new(18, 28, 10);
            kit::triangle(fb, [(x - 38, y - 50), (x - 30, y - 80), (x - 14, y - 58)], body);
            kit::triangle(fb, [(x + 38, y - 50), (x + 30, y - 80), (x + 14, y - 58)], body);
            kit::round_rect(fb, x - 44, y - 66, 88, 100, 40, body);
            kit::round_rect(fb, x - 28, y - 10, 56, 42, 20, Rgb565::new(26, 44, 20));
            kit::circle(fb, x - 18, y - 36, 17, Rgb565::new(29, 52, 22));
            kit::circle(fb, x + 18, y - 36, 17, Rgb565::new(29, 52, 22));
            eyes(fb, x, y - 36, 18, 11);
            let o = (open * 10.0) as i32;
            kit::triangle(fb, [(x - 6, y - 20), (x + 6, y - 20), (x, y - 8 + o)], Rgb565::new(30, 42, 4));
        }
        Critter::Bee => {
            let wing = kit::blend(kit::WHITE, Rgb565::new(20, 50, 31), 0.4);
            let flap = if open > 0.1 { ((open * 40.0) as i32 % 2) * 8 } else { 0 };
            kit::circle(fb, x - 22, y - 58 - flap, 20, wing);
            kit::circle(fb, x + 22, y - 58 - flap, 20, wing);
            kit::round_rect(fb, x - 46, y - 46, 92, 64, 32, Rgb565::new(31, 52, 4));
            for k in 0..3 {
                kit::rect(fb, x - 22 + k * 18, y - 45, 9, 62, dark);
            }
            kit::circle(fb, x - 44, y - 16, 22, Rgb565::new(31, 52, 4));
            kit::circle(fb, x - 50, y - 22, 6, kit::WHITE);
            kit::circle(fb, x - 50, y - 22, 3, dark);
            kit::line(fb, x - 50, y - 36, x - 58, y - 52, 2, dark);
            mouth(fb, x - 48, y - 8, 10, open, Rgb565::new(18, 8, 8));
        }
        Critter::Bird => {
            let body = Rgb565::new(10, 36, 30);
            kit::circle(fb, x, y - 20, 34, body);
            kit::triangle(fb, [(x + 20, y - 40), (x + 50, y - 56), (x + 34, y - 30)], Rgb565::new(6, 24, 24));
            kit::circle(fb, x - 6, y - 4, 18, Rgb565::new(28, 50, 26));
            eyes(fb, x - 10, y - 28, 8, 7);
            let o = (open * 10.0) as i32;
            kit::triangle(fb, [(x - 34, y - 26), (x - 56, y - 20 - o / 2), (x - 32, y - 16)], Rgb565::new(31, 40, 4));
            if o > 2 {
                kit::triangle(fb, [(x - 34, y - 16), (x - 52, y - 10 + o / 2), (x - 32, y - 12)], Rgb565::new(29, 34, 2));
            }
        }
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(CritterChoir::new(Sound::new(NAME, &modbus, &mixer, &bus)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, pitch_hz, render};

    #[test]
    fn every_critter_sings_its_note_and_stops() {
        for choir in 0..2 {
            for col in 0..4 {
                let mut app = CritterChoir::new(Sound::detached());
                app.sound.set_reverb(0.0);
                app.choir.store(choir, std::sync::atomic::Ordering::Relaxed);
                let mut p = app.audio_processor().unwrap();
                app.tick(&Input { grid: std::array::from_fn(|g| g == 12 + col), ..Default::default() });
                let out = render(&mut p, 12);
                let c = CHOIRS[choir][col];
                assert!(energy(&out) > 1e-4, "{c:?} is silent: {}", energy(&out));
                assert!(out.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
                let tail = render(&mut p, 120);
                assert!(energy(&tail[tail.len() - 4096..]) < 1e-9, "{c:?} keeps singing");
                assert!(app.mouth[col] > 0.5);
            }
        }
    }

    #[test]
    fn the_critters_are_about_as_loud_as_each_other() {
        for c in 0..8u32 {
            let mut v = Voices { calls: Vec::new(), seed: 1 };
            let cr = Critter::from_u32(c);
            v.event(c, critter_note(cr, 2) as f32, 0.9);
            let peak = (0..48_000).map(|_| v.frame(48_000.0).0.abs()).fold(0.0f32, f32::max);
            assert!((0.15..0.6).contains(&peak), "{cr:?}: {peak}");
        }
    }

    #[test]
    fn the_owl_sings_in_tune() {
        // The owl's "hoo" holds its pitch best of the critters.
        let mut v = Voices { calls: Vec::new(), seed: 1 };
        v.event(Critter::Owl as u32, 69.0, 1.0);
        let left: Vec<f32> = (0..30_000).map(|_| v.frame(48_000.0).0).collect();
        let hz = pitch_hz(&left[4800..], 48_000.0);
        // Its formants can make the 2nd harmonic loudest: accept either.
        let r = hz / 440.0;
        assert!((r - 1.0).abs() < 0.05 || (r - 2.0).abs() < 0.1, "{hz}");
    }

    #[test]
    fn columns_run_low_to_high_and_rows_bottom_to_top() {
        for choir in CHOIRS {
            assert!(choir.windows(2).all(|w| critter_note(w[0], 3) <= critter_note(w[1], 3)), "{choir:?}: low voice to high");
            for c in choir {
                assert!(critter_note(c, 3) < critter_note(c, 0), "{c:?}: bottom pad lowest");
                for row in 0..4 {
                    assert!(kit::PENTATONIC.contains(&(critter_note(c, row) % 12)));
                }
            }
        }
    }

    #[test]
    fn f3_sings_a_song_without_any_pads() {
        let mut app = CritterChoir::new(Sound::detached());
        let mut p = app.audio_processor().unwrap();
        app.toggle_running();
        let out = render(&mut p, 200);
        assert!(energy(&out) > 1e-4);
        app.tick(&Input::default());
        assert!(app.mouth.iter().any(|&m| m > 0.0), "the singers move");
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }
}
