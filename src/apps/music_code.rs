//! Music Code (Kids, ages 10-12): write a tune as a program.
//!
//! A program is a row of blocks, run left to right, one musical step (an
//! eighth note) at a time, and then from the start again. There is one
//! variable, the note: a step of the C major scale, starting on C.
//!
//! - PLAY plays the note; REST is silence; CHORD plays a chord on it;
//!   DRUM plays a drum. These four take a step of time.
//! - UP and DOWN move the note one scale step, LEAP UP and LEAP DOWN four;
//!   DICE sets it at random; HOME puts it back on C. These take no time:
//!   the next block runs straight away, which is why "UP PLAY UP PLAY"
//!   climbs.
//! - REPEAT runs everything since the last REPEAT (or the start) once
//!   more: a loop.
//! - IF HIGH: if the note is high (A or above), go down five steps. An
//!   if-statement that keeps a melody from climbing forever.
//!
//! The pads are the blocks (top three rows) and the editor (bottom row:
//! cursor left, cursor right, delete, run/stop). A block goes in at the
//! cursor. Left/right also move the cursor; up/down changes the
//! instrument; SELECT runs and stops; hold SELECT clears the program.
//! Blocks that take no time but never reach one that does are cut off
//! after 64 in a row, so a program can't hang.

use crate::app::{App, Input, SlintExtra};
use crate::audio::AudioProcessor;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::apps::kids_kit::{self as kit, Drum, Ev, Note, Rng, Size2, Song, Sound, Tone};
use embedded_graphics::pixelcolor::Rgb565;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Music Code";
const MAX_BLOCKS: usize = 24;
const ROOT: i32 = 60;
/// Degrees at or above this are "high" for IF HIGH (A, the 6th of C major).
const HIGH: i32 = 5;
const MIN_DEG: i32 = -7;
const MAX_DEG: i32 = 14;
const TONES: [Tone; 5] = [Tone::Marimba, Tone::Pluck, Tone::Chip, Tone::Glock, Tone::Organ];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Block {
    Play,
    Rest,
    Up,
    Down,
    LeapUp,
    LeapDown,
    Chord,
    Drum,
    Repeat,
    Dice,
    Home,
    IfHigh,
}

/// The blocks on the top three rows of pads, in pad order.
const PALETTE: [Block; 12] = [Block::Play, Block::Rest, Block::Up, Block::Down, Block::LeapUp, Block::LeapDown, Block::Chord, Block::Drum, Block::Repeat, Block::Dice, Block::Home, Block::IfHigh];

impl Block {
    fn label(self) -> &'static str {
        match self {
            Block::Play => "PLAY",
            Block::Rest => "REST",
            Block::Up => "UP",
            Block::Down => "DOWN",
            Block::LeapUp => "LEAP UP",
            Block::LeapDown => "LEAP DN",
            Block::Chord => "CHORD",
            Block::Drum => "DRUM",
            Block::Repeat => "REPEAT",
            Block::Dice => "DICE",
            Block::Home => "HOME",
            Block::IfHigh => "IF HIGH",
        }
    }
    /// Does it take a step of time?
    fn timed(self) -> bool {
        matches!(self, Block::Play | Block::Rest | Block::Chord | Block::Drum)
    }
    /// Sound blocks green, note-movers blue, control flow orange.
    fn color(self) -> Rgb565 {
        match self {
            Block::Play | Block::Rest | Block::Chord | Block::Drum => kit::rgb(90, 200, 110),
            Block::Up | Block::Down | Block::LeapUp | Block::LeapDown | Block::Dice | Block::Home => kit::rgb(80, 150, 240),
            Block::Repeat | Block::IfHigh => kit::rgb(245, 160, 60),
        }
    }
}

/// The interpreter, on the audio thread.
struct Machine {
    program: Arc<Mutex<Vec<Block>>>,
    tone: Arc<AtomicUsize>,
    /// The block shown as running, and the note, for the screen.
    shown_pc: Arc<AtomicUsize>,
    shown_note: Arc<AtomicI32>,
    pc: usize,
    note: i32,
    repeated: Vec<bool>,
    rng: Rng,
}

impl Machine {
    /// Runs blocks until one takes time; returns what it played.
    fn run_step(&mut self, prog: &[Block], out: &mut Vec<Ev>) {
        if prog.is_empty() {
            return;
        }
        if self.repeated.len() != prog.len() {
            self.repeated = vec![false; prog.len()];
        }
        for _ in 0..64 {
            if self.pc >= prog.len() {
                self.pc = 0;
                self.repeated.iter_mut().for_each(|r| *r = false);
            }
            let b = prog[self.pc];
            let here = self.pc;
            self.pc += 1;
            let tone = TONES[self.tone.load(Ordering::Relaxed) % TONES.len()];
            let pitch = |d: i32| kit::scale_note(ROOT, &kit::MAJOR, d) as f32;
            match b {
                Block::Play => out.push(Ev::Note(Note::new(tone, pitch(self.note)).vel(0.8).len(0.25))),
                Block::Rest => {}
                Block::Chord => {
                    for k in [0, 2, 4] {
                        out.push(Ev::Note(Note::new(tone, pitch(self.note + k) - 12.0).vel(0.5).len(0.4)));
                    }
                }
                Block::Drum => out.push(Ev::Drum(Drum::Kick, 0.9)),
                Block::Up => self.note = (self.note + 1).min(MAX_DEG),
                Block::Down => self.note = (self.note - 1).max(MIN_DEG),
                Block::LeapUp => self.note = (self.note + 4).min(MAX_DEG),
                Block::LeapDown => self.note = (self.note - 4).max(MIN_DEG),
                Block::Dice => self.note = self.rng.below(8) as i32,
                Block::Home => self.note = 0,
                Block::IfHigh => {
                    if self.note >= HIGH {
                        self.note -= 5;
                    }
                }
                Block::Repeat => {
                    if !self.repeated[here] {
                        self.repeated[here] = true;
                        // Back to just after the previous REPEAT.
                        let start = prog[..here].iter().rposition(|b| *b == Block::Repeat).map_or(0, |i| i + 1);
                        for r in &mut self.repeated[start..here] {
                            *r = false;
                        }
                        self.pc = start;
                    } else {
                        self.repeated[here] = false;
                    }
                }
            }
            self.shown_pc.store(here, Ordering::Relaxed);
            self.shown_note.store(self.note, Ordering::Relaxed);
            if b.timed() {
                return;
            }
        }
    }
}

impl Song for Machine {
    fn steps_per_beat(&self) -> u32 {
        2
    }
    fn step(&mut self, step: u64, out: &mut Vec<Ev>) {
        if step == 0 {
            self.pc = 0;
            self.note = 0;
            self.repeated.clear();
        }
        let prog = match self.program.try_lock() {
            Ok(p) => p.clone(),
            Err(_) => return,
        };
        self.run_step(&prog, out);
    }
}

pub struct MusicCode {
    sound: Sound,
    program: Arc<Mutex<Vec<Block>>>,
    tone: Arc<AtomicUsize>,
    shown_pc: Arc<AtomicUsize>,
    shown_note: Arc<AtomicI32>,
    cursor: usize,
    prev: [bool; 16],
    flash: Option<(usize, u32)>,
}

fn demo() -> Vec<Block> {
    use Block::*;
    vec![Play, Up, Play, Up, Play, Rest, Repeat, LeapUp, Chord, Rest, Home, Drum]
}

impl MusicCode {
    pub fn new(sound: Sound) -> MusicCode {
        sound.set_tempo(108.0);
        sound.set_reverb(0.18);
        let program = demo();
        let cursor = program.len();
        MusicCode { sound, program: Arc::new(Mutex::new(program)), tone: Arc::new(AtomicUsize::new(0)), shown_pc: Arc::new(AtomicUsize::new(0)), shown_note: Arc::new(AtomicI32::new(0)), cursor, prev: [false; 16], flash: None }
    }

    fn insert(&mut self, b: Block) {
        let mut p = self.program.lock().unwrap();
        if p.len() >= MAX_BLOCKS {
            return;
        }
        let at = self.cursor.min(p.len());
        p.insert(at, b);
        drop(p);
        self.cursor = at + 1;
        self.flash = Some((at, 20));
        // A sound block is heard as you place it.
        let tone = TONES[self.tone.load(Ordering::Relaxed) % TONES.len()];
        match b {
            Block::Play => self.sound.play(Note::new(tone, ROOT as f32).len(0.2)),
            Block::Chord => {
                for k in [0, 4, 7] {
                    self.sound.play(Note::new(tone, (ROOT - 12 + k) as f32).vel(0.5).len(0.3));
                }
            }
            Block::Drum => self.sound.drum(Drum::Kick, 0.9),
            _ => {}
        }
    }

    fn delete(&mut self) {
        let mut p = self.program.lock().unwrap();
        if self.cursor > 0 && self.cursor <= p.len() {
            p.remove(self.cursor - 1);
            self.cursor -= 1;
        }
    }

    fn explain(&self) -> String {
        let p = self.program.lock().unwrap();
        let b = if self.sound.playing() { p.get(self.shown_pc.load(Ordering::Relaxed)).copied() } else { self.cursor.checked_sub(1).and_then(|i| p.get(i)).copied() };
        match b {
            Some(Block::Play) => "PLAY: sound the note, then wait one step.".into(),
            Some(Block::Rest) => "REST: wait one step in silence.".into(),
            Some(Block::Chord) => "CHORD: the note plus the notes two and four steps above it.".into(),
            Some(Block::Drum) => "DRUM: a kick drum, one step.".into(),
            Some(Block::Up) | Some(Block::Down) => "UP / DOWN: move the note one step of the scale. No time passes.".into(),
            Some(Block::LeapUp) | Some(Block::LeapDown) => "LEAP: move the note four steps at once.".into(),
            Some(Block::Dice) => "DICE: a random note from C to the C above. Each time round is different.".into(),
            Some(Block::Home) => "HOME: the note goes back to C.".into(),
            Some(Block::Repeat) => "REPEAT: go back and run everything since the last REPEAT once more. A loop!".into(),
            Some(Block::IfHigh) => "IF HIGH: IF the note is A or higher, THEN go down five steps. Otherwise do nothing.".into(),
            None => "Put blocks in with the pads. SELECT runs the program.".into(),
        }
    }
}

impl App for MusicCode {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        let p = self.program.lock().unwrap();
        vec![("Program".into(), p.iter().map(|b| b.label()).collect::<Vec<_>>().join(" "), false), ("Sound".into(), TONES[self.tone.load(Ordering::Relaxed) % TONES.len()].name().into(), false)]
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
        std::array::from_fn(|p| match p {
            0 | 1 | 6 | 7 => PadColor::Green,
            8 | 11 => PadColor::Yellow,
            12..=14 => PadColor::Off,
            15 => if self.sound.playing() { PadColor::Red } else { PadColor::Green },
            _ => PadColor::Blue,
        })
    }

    fn tick(&mut self, input: &Input) {
        for p in 0..16 {
            if input.grid[p] && !self.prev[p] {
                match p {
                    0..=11 => self.insert(PALETTE[p]),
                    12 => self.cursor = self.cursor.saturating_sub(1),
                    13 => self.cursor = (self.cursor + 1).min(self.program.lock().unwrap().len()),
                    14 => self.delete(),
                    _ => self.toggle_running(),
                }
            }
        }
        self.prev = input.grid;
        if input.knob2 != 0 {
            let len = self.program.lock().unwrap().len() as i32;
            self.cursor = (self.cursor as i32 + input.knob2.signum()).clamp(0, len) as usize;
        }
        if input.navigation_steps != 0 {
            let t = (self.tone.load(Ordering::Relaxed) as i32 - input.navigation_steps.signum()).rem_euclid(TONES.len() as i32) as usize;
            self.tone.store(t, Ordering::Relaxed);
            self.sound.play(Note::new(TONES[t], ROOT as f32).len(0.25));
        }
        if input.knob2_press {
            self.program.lock().unwrap().clear();
            self.cursor = 0;
        } else if input.knob1_press {
            self.toggle_running();
        }
        if let Some((i, t)) = self.flash {
            self.flash = if t > 0 { Some((i, t - 1)) } else { None };
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = kit::PAPER;
        let panel = kit::rgb(222, 235, 225);
        let dim = kit::MUTED;
        kit::clear(fb, bg);
        kit::kids_header(fb, NAME, "AGES 10-12");
        let prog = self.program.lock().unwrap().clone();
        let running = self.sound.playing();
        let pc = self.shown_pc.load(Ordering::Relaxed);

        // The program, eight blocks to a line.
        let (bx, by, bw, bh) = (12, 40, 66, 40);
        let at = |i: usize| (bx + (i as i32 % 8) * (bw + 4), by + (i as i32 / 8) * (bh + 12));
        for (i, b) in prog.iter().enumerate() {
            let (x, y) = at(i);
            let live = running && i == pc;
            let flash = matches!(self.flash, Some((j, _)) if j == i);
            let c = if live || flash { kit::blend(b.color(), kit::WHITE, 0.5) } else { kit::blend(b.color(), bg, 0.25) };
            kit::card(fb, x, y, bw, bh, 8, c);
            kit::text(fb, b.label(), x + bw / 2, y + bh / 2 - 6, Size2::Small, kit::BLACK, 0);
            if live {
                kit::outline(fb, x, y, bw, bh, 8, 2, kit::TEAL);
                kit::triangle(fb, [(x + bw / 2 - 6, y + bh + 2), (x + bw / 2 + 6, y + bh + 2), (x + bw / 2, y + bh + 9)], kit::INK);
            }
        }
        // The cursor: a caret where the next block goes.
        if prog.len() < MAX_BLOCKS || self.cursor < prog.len() {
            let (x, y) = if self.cursor < 8 * 3 { at(self.cursor) } else { at(MAX_BLOCKS - 1) };
            kit::rect(fb, x - 3, y - 2, 3, bh + 4, kit::rgb(250, 220, 80));
        }
        if prog.is_empty() {
            kit::text(fb, "(empty program)", bx + 10, by + 12, Size2::Medium, dim, -1);
        }

        // The note variable, on a ladder of the scale.
        let note = self.shown_note.load(Ordering::Relaxed);
        let (lx, ly) = (590, 200);
        kit::text(fb, "note", lx, 196 - 0, Size2::Small, dim, 0);
        for d in 0..8 {
            let y = ly + 120 - d * 14;
            let here = running && note == d;
            kit::rect(fb, lx - 30, y, 60, 2, if d == 0 || d == 7 { dim } else { panel });
            if here {
                kit::circle(fb, lx, y, 7, kit::rgb(250, 220, 80));
            }
            kit::text(fb, kit::note_name(kit::scale_note(ROOT, &kit::MAJOR, d)), lx - 34, y - 6, Size2::Small, dim, 1);
        }
        if running && !(0..8).contains(&note) {
            kit::text(fb, &format!("{note:+}"), lx, ly + 132, Size2::Small, kit::INK, 0);
        }

        // What the block does, and the pad map.
        kit::card(fb, 12, 200, 520, 40, 8, panel);
        kit::paragraph(fb, &self.explain(), 20, 206, 504, Size2::Small, kit::INK);
        let (gx, gy, gw, gh) = (12, 248, 128, 18);
        for p in 0..16 {
            let x = gx + (p as i32 % 4) * (gw + 4);
            let y = gy + (p as i32 / 4) * (gh + 3);
            let (label, c) = match p {
                0..=11 => (PALETTE[p].label(), kit::blend(PALETTE[p].color(), bg, 0.45)),
                12 => ("< cursor", panel),
                13 => ("cursor >", panel),
                14 => ("delete", panel),
                _ => (if running { "STOP" } else { "RUN" }, kit::rgb(60, 120, 70)),
            };
            kit::card(fb, x, y, gw, gh, 5, c);
            kit::text(fb, label, x + gw / 2, y + 3, Size2::Small, kit::INK, 0);
        }
        let tone = TONES[self.tone.load(Ordering::Relaxed) % TONES.len()].name();
        kit::kids_footer(fb, &format!("{}/{MAX_BLOCKS} blocks   up/down: {tone}   left/right: cursor   hold SELECT: clear", prog.len()));
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let m = Machine { program: Arc::clone(&self.program), tone: Arc::clone(&self.tone), shown_pc: Arc::clone(&self.shown_pc), shown_note: Arc::clone(&self.shown_note), pc: 0, note: 0, repeated: Vec::new(), rng: Rng::seeded_from_time() };
        Some(self.sound.processor(Some(Box::new(m)), None))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(MusicCode::new(Sound::new(NAME, &modbus, &mixer, &bus)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render};
    use Block::*;

    /// The notes (scale degrees, or None for silence) a program plays
    /// over `steps` steps.
    fn trace(prog: Vec<Block>, steps: u64) -> Vec<Option<i32>> {
        let mut m = Machine { program: Arc::new(Mutex::new(prog)), tone: Arc::new(AtomicUsize::new(0)), shown_pc: Arc::new(AtomicUsize::new(0)), shown_note: Arc::new(AtomicI32::new(0)), pc: 0, note: 0, repeated: Vec::new(), rng: Rng::new(1) };
        (0..steps)
            .map(|s| {
                let mut out = Vec::new();
                m.step(s, &mut out);
                out.iter().find_map(|e| if let Ev::Note(n) = e { Some(n.pitch as i32) } else { None })
            })
            .map(|p| p.map(|p| kit::MAJOR.iter().position(|&x| x == (p - ROOT).rem_euclid(12)).unwrap() as i32 + 7 * (p - ROOT).div_euclid(12)))
            .collect()
    }

    #[test]
    fn up_play_climbs_and_the_program_loops() {
        assert_eq!(trace(vec![Play, Up], 4), vec![Some(0), Some(1), Some(2), Some(3)], "the note carries on round the loop");
        assert_eq!(trace(vec![Home, Play, Up, Play], 4), vec![Some(0), Some(1), Some(0), Some(1)]);
    }

    #[test]
    fn repeat_runs_the_section_once_more() {
        // (PLAY UP) twice, then REST, then HOME and round again.
        let t = trace(vec![Play, Up, Repeat, Rest, Home], 6);
        assert_eq!(t, vec![Some(0), Some(1), None, Some(0), Some(1), None]);
        // Two sections: each repeats only its own blocks.
        let t = trace(vec![Home, Play, Repeat, Up, Play, Repeat], 6);
        assert_eq!(t, vec![Some(0), Some(0), Some(1), Some(2), Some(0), Some(0)]);
    }

    #[test]
    fn if_high_keeps_it_in_range() {
        let t = trace(vec![Play, LeapUp, IfHigh], 6);
        assert_eq!(t, vec![Some(0), Some(4), Some(3), Some(2), Some(1), Some(0)]);
    }

    #[test]
    fn a_program_with_no_time_cant_hang() {
        let t = trace(vec![Up, Down], 3);
        assert_eq!(t, vec![None, None, None]);
    }

    #[test]
    fn editing_with_pads() {
        let mut app = MusicCode::new(Sound::detached());
        app.tick(&Input { knob2_press: true, ..Default::default() });
        let pad = |i: usize| Input { grid: std::array::from_fn(|g| g == i), ..Default::default() };
        for p in [0, 2, 0] {
            app.tick(&pad(p));
            app.tick(&Input::default());
        }
        assert_eq!(*app.program.lock().unwrap(), vec![Play, Up, Play]);
        app.tick(&pad(12));
        app.tick(&Input::default());
        app.tick(&pad(14));
        app.tick(&Input::default());
        assert_eq!(*app.program.lock().unwrap(), vec![Play, Play], "delete removes the block before the cursor");
        let mut p = app.audio_processor().unwrap();
        app.tick(&pad(15));
        assert!(app.sound.playing());
        assert!(energy(&render(&mut p, 60)) > 1e-4);
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }
}
