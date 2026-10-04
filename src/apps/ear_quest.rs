//! Ear Quest (Kids, ages 10-12): ear-training games that unlock one
//! after another.
//!
//! 1. Higher or lower: two notes; is the second higher or lower? The
//!    gap shrinks as your streak grows.
//! 2. Happy or sad: a major or a minor chord.
//! 3. Same or different: two short tunes; did a note change?
//! 4. Intervals: how far apart are two notes: a 2nd (a step), a 3rd, a
//!    5th or an octave?
//! 5. Which note: after the key's C, is this do, re, mi or so?
//!
//! Five right answers in a quest unlock the next. Answer on the pads:
//! two choices split the pads into left and right halves, four into
//! corners, each labelled on screen. SELECT plays the question again;
//! up/down picks another unlocked quest; hold SELECT skips a question.

use crate::app::{App, Input, SlintExtra};
use crate::audio::AudioProcessor;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::apps::kids_kit::{self as kit, Note, Rng, Size2, Sound, Tone};
use std::sync::Arc;

const NAME: &str = "Ear Quest";
const QUESTS: [&str; 5] = ["Higher or lower", "Happy or sad", "Same or different", "Intervals", "Which note"];
const UNLOCK_AFTER: u32 = 5;
/// Frames between notes of a question (60 a second).
const GAP: u32 = 30;

#[derive(Clone, Debug, PartialEq)]
struct Question {
    /// (frame offset, notes sounding together)
    plan: Vec<(u32, Vec<i32>)>,
    answer: usize,
    choices: Vec<&'static str>,
    /// Said after a wrong answer.
    explain: String,
}

fn make(quest: usize, rng: &mut Rng, streak: u32) -> Question {
    let base = 55 + rng.below(12) as i32;
    match quest {
        0 => {
            // Wide gaps first, down to a semitone as you get good.
            let max = (8 - streak.min(6) as i32).max(2);
            let gap = 1 + rng.below(max as u32) as i32;
            let up = rng.below(2) == 1;
            let second = if up { base + gap } else { base - gap };
            Question { plan: vec![(0, vec![base]), (GAP, vec![second])], answer: up as usize, choices: vec!["lower", "higher"], explain: format!("It went {} by {gap} semitone{}.", if up { "up" } else { "down" }, if gap == 1 { "" } else { "s" }) }
        }
        1 => {
            let minor = rng.below(2) == 1;
            let third = if minor { 3 } else { 4 };
            let c = vec![base, base + third, base + 7];
            Question {
                plan: vec![(0, vec![base]), (GAP / 2, vec![base + third]), (GAP, vec![base + 7]), (GAP * 2, c)],
                answer: minor as usize,
                choices: vec!["happy (major)", "sad (minor)"],
                explain: format!("It was {}: its middle note is {} semitones up.", if minor { "minor" } else { "major" }, third),
            }
        }
        2 => {
            let tune: Vec<i32> = (0..3).map(|_| kit::scale_note(base, &kit::MAJOR, rng.below(5) as i32)).collect();
            let changed = rng.below(2) == 1;
            let mut second = tune.clone();
            if changed {
                let k = rng.below(3) as usize;
                second[k] += if rng.below(2) == 1 { 2 } else { -2 };
            }
            let mut plan = Vec::new();
            for (k, n) in tune.iter().enumerate() {
                plan.push((k as u32 * GAP / 2, vec![*n]));
            }
            for (k, n) in second.iter().enumerate() {
                plan.push((GAP * 2 + k as u32 * GAP / 2, vec![*n]));
            }
            Question { plan, answer: changed as usize, choices: vec!["same", "different"], explain: if changed { "One note moved.".into() } else { "They were the same.".into() } }
        }
        3 => {
            const IV: [(i32, &str); 4] = [(2, "2nd (a step)"), (4, "3rd"), (7, "5th"), (12, "octave")];
            let k = rng.below(4) as usize;
            Question {
                plan: vec![(0, vec![base]), (GAP, vec![base + IV[k].0]), (GAP * 2, vec![base, base + IV[k].0])],
                answer: k,
                choices: IV.iter().map(|i| i.1).collect(),
                explain: format!("A {} is {} semitones.", IV[k].1, IV[k].0),
            }
        }
        _ => {
            const SOLFA: [(i32, &str); 4] = [(0, "do"), (2, "re"), (4, "mi"), (7, "so")];
            let k = rng.below(4) as usize;
            // The key's do, then a short pause, then the mystery note.
            let doh = 60;
            Question {
                plan: vec![(0, vec![doh, doh + 4, doh + 7]), (GAP * 2, vec![doh + SOLFA[k].0])],
                answer: k,
                choices: SOLFA.iter().map(|s| s.1).collect(),
                explain: format!("It was {}.", SOLFA[k].1),
            }
        }
    }
}

/// Which choice a pad gives, for 2 or 4 choices.
fn pad_choice(pad: usize, n: usize) -> usize {
    let (row, col) = (pad / 4, pad % 4);
    if n == 2 {
        col / 2
    } else {
        (row / 2) * 2 + col / 2
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Asking,
    Right(u32),
    Wrong(u32),
}

pub struct EarQuest {
    sound: Sound,
    rng: Rng,
    quest: usize,
    unlocked: usize,
    right: [u32; 5],
    streak: u32,
    best_streak: u32,
    q: Question,
    phase: Phase,
    /// Frames since the question started playing, or None when idle.
    playing: Option<u32>,
    prev: [bool; 16],
    picked: Option<usize>,
    started: bool,
}

impl EarQuest {
    pub fn new(sound: Sound, mut rng: Rng) -> EarQuest {
        sound.set_reverb(0.15);
        let q = make(0, &mut rng, 0);
        EarQuest { sound, rng, quest: 0, unlocked: 0, right: [0; 5], streak: 0, best_streak: 0, q, phase: Phase::Asking, playing: None, prev: [false; 16], picked: None, started: false }
    }

    fn next(&mut self) {
        self.q = make(self.quest, &mut self.rng, self.streak);
        self.phase = Phase::Asking;
        self.picked = None;
        self.playing = Some(0);
    }

    fn answer(&mut self, choice: usize) {
        if self.phase != Phase::Asking {
            return;
        }
        self.picked = Some(choice);
        if choice == self.q.answer {
            self.right[self.quest] += 1;
            self.streak += 1;
            self.best_streak = self.best_streak.max(self.streak);
            if self.right[self.quest] >= UNLOCK_AFTER && self.unlocked == self.quest && self.unlocked + 1 < QUESTS.len() {
                self.unlocked += 1;
            }
            self.sound.play(Note::new(Tone::Glock, 84.0).vel(0.4).len(0.2));
            self.sound.play(Note::new(Tone::Glock, 91.0).vel(0.3).len(0.3));
            self.phase = Phase::Right(0);
        } else {
            self.streak = 0;
            self.sound.play(Note::new(Tone::Soft, 50.0).vel(0.4).len(0.3));
            self.phase = Phase::Wrong(0);
        }
    }
}

impl App for EarQuest {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![("Quest".into(), QUESTS[self.quest].into(), false), ("Streak".into(), self.streak.to_string(), false)]
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        const C: [PadColor; 4] = [PadColor::Blue, PadColor::Yellow, PadColor::Green, PadColor::Red];
        let n = self.q.choices.len();
        std::array::from_fn(|p| C[pad_choice(p, n)])
    }

    fn tick(&mut self, input: &Input) {
        // Nothing plays until the first press.
        let n = self.q.choices.len();
        for p in 0..16 {
            if input.grid[p] && !self.prev[p] {
                if !self.started {
                    self.started = true;
                    self.next();
                } else {
                    self.answer(pad_choice(p, n));
                }
            }
        }
        self.prev = input.grid;
        if input.knob1_press {
            if !self.started {
                self.started = true;
                self.next();
            } else {
                self.playing = Some(0);
            }
        }
        if input.knob2_press && self.started {
            self.streak = 0;
            self.next();
        }
        if input.navigation_steps != 0 {
            let q = (self.quest as i32 + input.navigation_steps).clamp(0, self.unlocked as i32) as usize;
            if q != self.quest {
                self.quest = q;
                self.streak = 0;
                if self.started {
                    self.next();
                } else {
                    self.q = make(self.quest, &mut self.rng, 0);
                }
            }
        }
        if let Some(f) = self.playing {
            for (at, notes) in &self.q.plan {
                if *at == f {
                    for (i, n) in notes.iter().enumerate() {
                        self.sound.play(Note::new(Tone::Marimba, *n as f32).vel(0.75 - i as f32 * 0.08).len(0.6));
                    }
                }
            }
            let end = self.q.plan.iter().map(|p| p.0).max().unwrap_or(0);
            self.playing = if f > end { None } else { Some(f + 1) };
        }
        self.phase = match self.phase {
            Phase::Right(t) if t > 70 => {
                self.next();
                Phase::Asking
            }
            Phase::Right(t) => Phase::Right(t + 1),
            Phase::Wrong(t) if t == 20 => {
                self.playing = Some(0);
                Phase::Wrong(t + 1)
            }
            Phase::Wrong(t) if t > 200 => {
                self.next();
                Phase::Asking
            }
            Phase::Wrong(t) => Phase::Wrong(t + 1),
            p => p,
        };
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = kit::PAPER;
        let panel = kit::rgb(225, 216, 235);
        let dim = kit::MUTED;
        kit::clear(fb, bg);
        kit::kids_header(fb, NAME, "AGES 10-12");

        // The quest map down the left: a path of five stops.
        for (i, name) in QUESTS.iter().enumerate() {
            let y = 46 + i as i32 * 54;
            let open = i <= self.unlocked;
            let here = i == self.quest;
            if i > 0 {
                kit::rect(fb, 27, y - 34, 4, 34, if open { dim } else { panel });
            }
            let c = if here { kit::rgb(250, 200, 60) } else if open { kit::rgb(110, 200, 150) } else { panel };
            kit::circle(fb, 29, y + 10, 14, c);
            kit::text(fb, &(i + 1).to_string(), 29, y + 2, Size2::Medium, if open { kit::BLACK } else { dim }, 0);
            kit::text(fb, name, 50, y + 2, Size2::Small, if open { kit::INK } else { dim }, -1);
            let progress = if open { format!("{} right", self.right[i]) } else { "locked".into() };
            kit::text(fb, &progress, 50, y + 14, Size2::Small, dim, -1);
        }

        // The question.
        let (qx, qw) = (180, 450);
        let title = if !self.started { "Press any pad to start".to_string() } else { QUESTS[self.quest].to_string() };
        kit::text(fb, &title, qx + qw / 2, 40, if self.started { Size2::Large } else { Size2::Medium }, kit::INK, 0);
        let listening = self.playing.is_some();
        let say = match self.phase {
            _ if !self.started => "Listen, then answer on the pads.".to_string(),
            Phase::Asking if listening => "Listen...".to_string(),
            Phase::Asking => "Which was it?".to_string(),
            Phase::Right(_) => "Right!".to_string(),
            Phase::Wrong(_) => format!("Not quite. {} Listen again.", self.q.explain),
        };
        kit::paragraph(fb, &say, qx + 8, 72, qw - 16, Size2::Small, if matches!(self.phase, Phase::Right(_)) { kit::TEAL } else { dim });

        // The answer mats, laid out like the pads.
        let n = self.q.choices.len();
        let colors = [kit::rgb(80, 140, 240), kit::rgb(240, 200, 60), kit::rgb(90, 200, 110), kit::rgb(235, 80, 80)];
        for (k, label) in self.q.choices.iter().enumerate() {
            let (x, y, w, h) = if n == 2 { (qx + k as i32 * (qw / 2), 108, qw / 2 - 10, 188) } else { (qx + (k as i32 % 2) * (qw / 2), 108 + (k as i32 / 2) * 96, qw / 2 - 10, 90) };
            let mut c = kit::blend(colors[k], bg, 0.35);
            if let Some(pick) = self.picked {
                if k == self.q.answer && self.phase != Phase::Asking {
                    c = colors[k];
                } else if k == pick {
                    c = kit::blend(colors[k], bg, 0.7);
                }
            }
            kit::card(fb, x, y, w, h, 14, c);
            kit::text(fb, label, x + w / 2, y + h / 2 - 8, Size2::Medium, kit::INK, 0);
            let pads = if n == 2 { ["LEFT PADS", "RIGHT PADS"][k] } else { ["TOP LEFT", "TOP RIGHT", "BOTTOM LEFT", "BOTTOM RIGHT"][k] };
            kit::text(fb, pads, x + w / 2, y + h - 22, Size2::Small, kit::INK, 0);
            if self.phase != Phase::Asking && k == self.q.answer {
                kit::star(fb, x + w - 24, y + 22, 12, kit::INK);
            }
        }
        kit::text(fb, &format!("streak {}   best {}", self.streak, self.best_streak), qx + qw / 2, 304, Size2::Medium, kit::INK, 0);
        if self.unlocked + 1 < QUESTS.len() {
            let need = UNLOCK_AFTER.saturating_sub(self.right[self.unlocked]);
            kit::text(fb, &format!("{need} more right in quest {} unlocks quest {}", self.unlocked + 1, self.unlocked + 2), qx + qw / 2, 322, Size2::Small, dim, 0);
        }
        kit::kids_footer(fb, "pads: answer   SELECT: hear it again   up/down: quest   hold SELECT: skip");
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(self.sound.processor(None, None))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(EarQuest::new(Sound::new(NAME, &modbus, &mixer, &bus), Rng::seeded_from_time()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render};

    fn press_choice(app: &mut EarQuest, c: usize) {
        let n = app.q.choices.len();
        let pad = (0..16).find(|&p| pad_choice(p, n) == c).unwrap();
        app.tick(&Input { grid: std::array::from_fn(|g| g == pad), ..Default::default() });
        app.tick(&Input::default());
    }

    #[test]
    fn every_quests_answer_matches_what_it_plays() {
        let mut rng = Rng::new(11);
        for _ in 0..200 {
            let q = make(0, &mut rng, 0);
            assert_eq!(q.answer == 1, q.plan[1].1[0] > q.plan[0].1[0]);
            let q = make(1, &mut rng, 0);
            assert_eq!(q.answer == 1, q.plan[3].1[1] - q.plan[3].1[0] == 3, "sad = minor third");
            let q = make(2, &mut rng, 0);
            let a: Vec<i32> = q.plan[..3].iter().map(|p| p.1[0]).collect();
            let b: Vec<i32> = q.plan[3..].iter().map(|p| p.1[0]).collect();
            assert_eq!(q.answer == 1, a != b);
            let q = make(3, &mut rng, 0);
            assert_eq!(q.plan[1].1[0] - q.plan[0].1[0], [2, 4, 7, 12][q.answer]);
            let q = make(4, &mut rng, 0);
            assert_eq!(q.plan[1].1[0] - 60, [0, 2, 4, 7][q.answer]);
        }
    }

    #[test]
    fn right_answers_unlock_the_next_quest() {
        let mut app = EarQuest::new(Sound::detached(), Rng::new(4));
        let mut p = app.audio_processor().unwrap();
        assert!(energy(&render(&mut p, 4)) < 1e-12, "silent until you start");
        app.tick(&Input { knob1_press: true, ..Default::default() });
        for _ in 0..UNLOCK_AFTER {
            let a = app.q.answer;
            press_choice(&mut app, a);
            assert!(matches!(app.phase, Phase::Right(_)));
            for _ in 0..80 {
                app.tick(&Input::default());
            }
        }
        assert_eq!(app.unlocked, 1);
        assert!(energy(&render(&mut p, 10)) > 1e-5);
        app.tick(&Input { navigation_steps: 1, ..Default::default() });
        assert_eq!(app.quest, 1);
        assert_eq!(app.q.choices.len(), 2);
        let wrong = 1 - app.q.answer;
        press_choice(&mut app, wrong);
        assert_eq!(app.streak, 0);
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }

    #[test]
    fn four_choices_use_the_corners() {
        assert_eq!(pad_choice(0, 4), 0);
        assert_eq!(pad_choice(15, 4), 3);
        assert_eq!(pad_choice(12, 2), 0);
        assert_eq!(pad_choice(3, 2), 1);
    }
}
