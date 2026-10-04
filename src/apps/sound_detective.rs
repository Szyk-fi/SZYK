//! Sound Detective (Kids, ages 10-12): a synthesizer puzzle.
//!
//! A mystery sound plays. Your synthesizer has the same five controls
//! the mystery one was made with: Wave, Octave, Brightness (a low-pass
//! filter), Attack and Length (the envelope). Turn yours until it sounds
//! the same, then press SELECT to check. A wrong check shows which
//! controls are already right. Level 1 only hides the wave; each level
//! hides one more control, until level 5 hides all five. Solve three
//! cases to go up a level.
//!
//! Pads: 1 hear the mystery again, 2 hear yours, 3 check, 4 new case;
//! the other twelve play your sound as a keyboard. Up/down picks a
//! control, left/right turns it (and plays yours, so you hear the
//! change). Hold SELECT for a new case.
//!
//! The synth is a real subtractive one: a band-limited oscillator, a
//! 12 dB/octave state-variable low-pass filter, and an attack/decay
//! envelope, which is how most synthesizers since the 1970s work.

use crate::app::{App, Input, SlintExtra};
use crate::audio::AudioProcessor;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::apps::kids_kit::{self as kit, Extra, Rng, Size2, Sound, Svf};
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const NAME: &str = "Sound Detective";
const CONTROLS: usize = 5;
const CONTROL_NAMES: [&str; CONTROLS] = ["Wave", "Octave", "Brightness", "Attack", "Length"];
const CHOICES: [&[&str]; CONTROLS] = [&["sine", "triangle", "saw", "square"], &["low", "middle", "high"], &["very dark", "dark", "medium", "bright", "very bright"], &["fast", "medium", "slow"], &["short", "medium", "long"]];
const CUTOFF: [f32; 5] = [300.0, 700.0, 1500.0, 3500.0, 9000.0];
const ATTACK: [f32; 3] = [0.004, 0.12, 0.45];
const LENGTH: [f32; 3] = [0.18, 0.55, 1.6];
const OCTAVE_ROOT: [i32; 3] = [45, 57, 69];
/// The motif each sound plays: root, fifth, octave.
const MOTIF: [i32; 3] = [0, 7, 12];
const MOTIF_GAP_S: f32 = 0.42;
const SOLVES_PER_LEVEL: u32 = 3;
const LEVELS: usize = 5;
/// Level n hides the first n controls in this order.
const HIDE_ORDER: [usize; CONTROLS] = [0, 1, 2, 3, 4];

type Patch = [usize; CONTROLS];

fn about(control: usize, choice: usize) -> &'static str {
    match (control, choice) {
        (0, 0) => "Sine: one pure frequency and nothing else. Soft, like a whistle.",
        (0, 1) => "Triangle: a sine with a few quiet odd harmonics. Gentle, like a flute.",
        (0, 2) => "Saw: every harmonic, loud. Buzzy and bright, like a brass or string section.",
        (0, 3) => "Square: only the odd harmonics. Hollow, like a clarinet or an old video game.",
        (1, _) => "Octave: each step up doubles the frequency, the same note but higher.",
        (2, _) => "Brightness is a low-pass filter: it removes the high harmonics above its cutoff. Darker = fewer harmonics.",
        (3, _) => "Attack: how fast the sound gets loud. Fast is a hit or pluck; slow fades in, like a bowed violin.",
        _ => "Length: how long the sound takes to die away after it starts.",
    }
}

struct Shared {
    mystery: [AtomicUsize; CONTROLS],
    mine: [AtomicUsize; CONTROLS],
}

fn load(a: &[AtomicUsize; CONTROLS]) -> Patch {
    std::array::from_fn(|i| a[i].load(Ordering::Relaxed))
}

struct Voice {
    patch: Patch,
    hz: f32,
    t: f32,
    phase: f32,
    filt: Svf,
}

/// The synth, plus the motif player.
struct Synth {
    s: Arc<Shared>,
    voices: Vec<Voice>,
    /// Pending motif notes: (seconds from now, which patch: 0 mystery / 1 mine, note).
    queue: Vec<(f32, u32, i32)>,
}

impl Synth {
    fn start(&mut self, which: u32, note: i32) {
        let patch = if which == 0 { load(&self.s.mystery) } else { load(&self.s.mine) };
        if self.voices.len() >= 8 {
            self.voices.remove(0);
        }
        let n = OCTAVE_ROOT[patch[1] % 3] + note;
        self.voices.push(Voice { patch, hz: kit::midi_hz(n as f32), t: 0.0, phase: 0.0, filt: Svf::default() });
    }
}

/// One voice's output: oscillator, filter, envelope.
fn render(v: &mut Voice, sr: f32) -> f32 {
    let dt = v.hz / sr;
    let p = v.phase;
    v.phase = (p + dt).fract();
    let osc = match v.patch[0] {
        0 => (p * TAU).sin(),
        1 => 1.0 - 4.0 * (p - 0.5).abs(),
        2 => kit::saw(p, dt),
        _ => kit::square(p, dt) * 0.8,
    };
    let y = v.filt.tick(osc, CUTOFF[v.patch[2] % 5], 0.9, sr).lp;
    let a = ATTACK[v.patch[3] % 3];
    let len = LENGTH[v.patch[4] % 3];
    let env = if v.t < a { v.t / a } else { (-(v.t - a) / (len / 4.6)).exp() };
    v.t += 1.0 / sr;
    y * env * 0.6
}

impl Extra for Synth {
    fn event(&mut self, a: u32, b: f32, _c: f32) {
        match a {
            // Play the motif with patch b (0 mystery, 1 mine).
            0 => {
                self.queue.retain(|q| q.1 != b as u32);
                for (k, n) in MOTIF.iter().enumerate() {
                    self.queue.push((k as f32 * MOTIF_GAP_S, b as u32, *n));
                }
            }
            // One note of mine, b semitones up.
            _ => self.start(1, b as i32),
        }
    }
    fn frame(&mut self, sr: f32) -> (f32, f32) {
        let dt = 1.0 / sr;
        let mut k = 0;
        while k < self.queue.len() {
            self.queue[k].0 -= dt;
            if self.queue[k].0 <= 0.0 {
                let (_, which, note) = self.queue.remove(k);
                self.start(which, note);
            } else {
                k += 1;
            }
        }
        let mut out = 0.0;
        for v in self.voices.iter_mut() {
            out += render(v, sr);
        }
        self.voices.retain(|v| v.t < ATTACK[v.patch[3] % 3] + LENGTH[v.patch[4] % 3] * 1.3);
        (out, out)
    }
}

pub struct SoundDetective {
    sound: Sound,
    s: Arc<Shared>,
    rng: Rng,
    level: usize,
    solved: u32,
    case_no: u32,
    control: usize,
    /// Which controls a check found right, once there's been a check.
    right: Option<[bool; CONTROLS]>,
    checks: u32,
    message: (String, u32),
    prev: [bool; 16],
    score: u32,
}

impl SoundDetective {
    pub fn new(sound: Sound, rng: Rng) -> SoundDetective {
        sound.set_reverb(0.08);
        let s = Arc::new(Shared { mystery: std::array::from_fn(|_| AtomicUsize::new(0)), mine: std::array::from_fn(|_| AtomicUsize::new(0)) });
        let mut d = SoundDetective { sound, s, rng, level: 1, solved: 0, case_no: 0, control: 0, right: None, checks: 0, message: (String::new(), 0), prev: [false; 16], score: 0 };
        d.new_case();
        d
    }

    /// The controls hidden at this level.
    fn hidden(&self) -> Vec<usize> {
        HIDE_ORDER[..self.level.min(LEVELS)].to_vec()
    }

    fn new_case(&mut self) {
        self.case_no += 1;
        self.checks = 0;
        self.right = None;
        let hidden = self.hidden();
        // A mystery that's different from where you start.
        loop {
            let mut differs = false;
            for c in 0..CONTROLS {
                let n = CHOICES[c].len() as u32;
                let v = self.rng.below(n) as usize;
                self.s.mystery[c].store(v, Ordering::Relaxed);
                let mine = if hidden.contains(&c) { (v + 1 + self.rng.below(n - 1) as usize) % n as usize } else { v };
                differs |= mine != v;
                self.s.mine[c].store(mine, Ordering::Relaxed);
            }
            if differs {
                break;
            }
        }
        self.control = hidden[0];
        // Play the new mystery, except for the first one: opening the app
        // mustn't make a sound by itself.
        if self.case_no > 1 {
            self.sound.custom(0, 0.0, 0.0);
        }
    }

    fn mine(&self) -> Patch {
        load(&self.s.mine)
    }
    fn mystery(&self) -> Patch {
        load(&self.s.mystery)
    }

    fn check(&mut self) {
        self.checks += 1;
        let (m, t) = (self.mine(), self.mystery());
        let right: [bool; CONTROLS] = std::array::from_fn(|c| m[c] == t[c]);
        if right.iter().all(|&r| r) {
            // Fewer checks, more points.
            self.score += (4u32.saturating_sub(self.checks)).max(1) * self.level as u32;
            self.solved += 1;
            if self.solved >= SOLVES_PER_LEVEL && self.level < LEVELS {
                self.level += 1;
                self.solved = 0;
                self.message = (format!("Case solved! Level {}: one more control is hidden.", self.level), 200);
            } else {
                self.message = ("Case solved! Here's the next one.".into(), 150);
            }
            for (k, n) in [72, 76, 79, 84].iter().enumerate() {
                self.sound.play(kit::Note::new(kit::Tone::Glock, *n as f32).vel(0.4).len(0.15 + k as f32 * 0.1));
            }
            self.new_case();
        } else {
            let hidden = self.hidden();
            let n = hidden.iter().filter(|&&c| right[c]).count();
            self.message = (format!("{n} of {} right. The green ones are correct.", hidden.len()), 150);
            self.right = Some(right);
        }
    }

    fn turn(&mut self, d: i32) {
        let c = self.control;
        let n = CHOICES[c].len() as i32;
        let v = (self.s.mine[c].load(Ordering::Relaxed) as i32 + d).rem_euclid(n) as usize;
        self.s.mine[c].store(v, Ordering::Relaxed);
        self.right = None;
        self.sound.custom(0, 1.0, 0.0);
    }
}

impl App for SoundDetective {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        let m = self.mine();
        (0..CONTROLS).map(|c| (CONTROL_NAMES[c].to_string(), CHOICES[c][m[c]].to_string(), false)).collect()
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        std::array::from_fn(|p| match p {
            0 => PadColor::Yellow,
            1 => PadColor::Blue,
            2 => PadColor::Green,
            3 => PadColor::Red,
            _ => PadColor::Off,
        })
    }

    fn tick(&mut self, input: &Input) {
        for p in 0..16 {
            if input.grid[p] && !self.prev[p] {
                match p {
                    0 => self.sound.custom(0, 0.0, 0.0),
                    1 => self.sound.custom(0, 1.0, 0.0),
                    2 => self.check(),
                    3 => self.new_case(),
                    _ => self.sound.custom(1, (p - 4) as f32, 0.0),
                }
            }
        }
        self.prev = input.grid;
        if input.navigation_steps != 0 {
            let hidden = self.hidden();
            let i = hidden.iter().position(|&c| c == self.control).unwrap_or(0) as i32;
            let j = (i + input.navigation_steps).clamp(0, hidden.len() as i32 - 1) as usize;
            self.control = hidden[j];
        }
        if input.knob2 != 0 {
            self.turn(input.knob2.signum());
        }
        if input.knob2_press {
            self.new_case();
        } else if input.knob1_press {
            self.check();
        }
        if self.message.1 > 0 {
            self.message.1 -= 1;
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = kit::PAPER;
        let panel = kit::rgb(223, 231, 242);
        let ink = kit::INK;
        let dim = kit::MUTED;
        let good = kit::TEAL;
        kit::clear(fb, bg);
        kit::kids_header(fb, NAME, "AGES 10-12");
        kit::text(fb, &format!("Case {}   level {} of {}   score {}", self.case_no, self.level, LEVELS, self.score), 12, 38, Size2::Medium, ink, -1);
        // Solved-at-this-level dots.
        for k in 0..SOLVES_PER_LEVEL {
            let c = if k < self.solved { good } else { panel };
            kit::circle(fb, 600 + k as i32 * 12 - 24, 46, 5, c);
        }
        let hidden = self.hidden();
        let (m, t) = (self.mine(), self.mystery());
        for c in 0..CONTROLS {
            let y = 64 + c as i32 * 40;
            let is_hidden = hidden.contains(&c);
            let sel = c == self.control;
            let bgc = if sel { kit::blend(panel, kit::INK, 0.15) } else { panel };
            kit::card(fb, 10, y, 300, 36, 8, bgc);
            if sel { kit::outline(fb, 10, y, 300, 36, 8, 2, kit::TEAL); }
            let right = self.right.map(|r| r[c]).unwrap_or(false);
            let name_c = if !is_hidden { dim } else if right { good } else { kit::INK };
            kit::text(fb, CONTROL_NAMES[c], 20, y + 10, Size2::Medium, name_c, -1);
            if is_hidden {
                kit::text(fb, &format!("< {} >", CHOICES[c][m[c]]), 300, y + 10, Size2::Medium, if sel { kit::INK } else { ink }, 1);
                if right {
                    kit::text(fb, "ok", 150, y + 12, Size2::Small, good, -1);
                }
            } else {
                kit::text(fb, &format!("{} (given)", CHOICES[c][t[c]]), 300, y + 12, Size2::Small, dim, 1);
            }
        }

        // Your sound, drawn: one cycle of the wave and its envelope.
        let (px, py, pw, ph) = (326, 64, 300, 92);
        kit::card(fb, px, py, pw, ph, 8, panel);
        kit::text(fb, "your wave", px + 8, py + 4, Size2::Small, dim, -1);
        let mut last = None;
        // A one-pole filter over the drawn cycle shows roughly what the
        // brightness control does to the shape.
        let k = (CUTOFF[m[2]] / 9000.0).clamp(0.05, 1.0);
        let mut lp = 0.0;
        for pass in 0..2 {
            for i in 0..=200 {
                let p = (i as f32 / 100.0).fract();
                let w = match m[0] {
                    0 => (p * TAU).sin(),
                    1 => 1.0 - 4.0 * (p - 0.5).abs(),
                    2 => 2.0 * p - 1.0,
                    _ => if p < 0.5 { 1.0 } else { -1.0 },
                };
                lp += (w - lp) * k;
                if pass == 1 {
                    let x = px + 10 + i * (pw - 20) / 200;
                    let y = py + ph / 2 + 6 - (lp * (ph as f32 / 2.0 - 16.0)) as i32;
                    if let Some((lx, ly)) = last {
                        kit::line(fb, lx, ly, x, y, 2, ink);
                    }
                    last = Some((x, y));
                }
            }
        }
        let (ex, ey, eh) = (px, py + ph + 8, 60);
        kit::card(fb, ex, ey, pw, eh, 8, panel);
        kit::text(fb, "your envelope", ex + 8, ey + 4, Size2::Small, dim, -1);
        let total = 2.2;
        let mut last = None;
        for i in 0..=100 {
            let tt = i as f32 / 100.0 * total;
            let a = ATTACK[m[3]];
            let env = if tt < a { tt / a } else { (-(tt - a) / (LENGTH[m[4]] / 4.6)).exp() };
            let x = ex + 10 + i * (pw - 20) / 100;
            let y = ey + eh - 8 - (env * (eh as f32 - 24.0)) as i32;
            if let Some((lx, ly)) = last {
                kit::line(fb, lx, ly, x, y, 2, kit::rgb(250, 190, 80));
            }
            last = Some((x, y));
        }

        // What the selected control does.
        kit::card(fb, 326, 232, 300, 92, 8, panel);
        kit::paragraph(fb, about(self.control, m[self.control]), 336, 240, 284, Size2::Small, kit::INK);
        if self.message.1 > 0 {
            kit::card(fb, 10, 270, 300, 54, 8, kit::blend(panel, good, 0.25));
            kit::paragraph(fb, &self.message.0, 20, 278, 284, Size2::Small, kit::INK);
        } else {
            kit::paragraph(fb, "Pad 1: hear the mystery\nPad 2: hear yours\nPad 3 or SELECT: check   Pad 4: new case", 14, 274, 296, Size2::Small, dim);
        }
        kit::kids_footer(fb, "up/down: control   left/right: change   pads 5-16: play   hold SELECT: new case");
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(self.sound.processor(None, Some(Box::new(Synth { s: Arc::clone(&self.s), voices: Vec::with_capacity(8), queue: Vec::with_capacity(8) }))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(SoundDetective::new(Sound::new(NAME, &modbus, &mixer, &bus), Rng::seeded_from_time()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render as run};

    #[test]
    fn a_case_hides_only_its_levels_controls() {
        let d = SoundDetective::new(Sound::detached(), Rng::new(5));
        let (m, t) = (d.mine(), d.mystery());
        assert_ne!(m[0], t[0], "level 1: the wave is the mystery");
        assert_eq!(&m[1..], &t[1..], "the rest are given");
    }

    #[test]
    fn matching_the_mystery_solves_it_and_three_solves_level_up() {
        let mut d = SoundDetective::new(Sound::detached(), Rng::new(9));
        for _ in 0..SOLVES_PER_LEVEL {
            let t = d.mystery();
            // Turn the wave until it matches.
            while d.mine()[0] != t[0] {
                d.tick(&Input { knob2: 1, ..Default::default() });
                d.tick(&Input::default());
            }
            d.tick(&Input { knob1_press: true, ..Default::default() });
        }
        assert_eq!(d.level, 2);
        assert!(d.score > 0);
        let (m, t) = (d.mine(), d.mystery());
        assert!(m[0] != t[0] || m[1] != t[1]);
        assert_eq!(&m[2..], &t[2..]);
    }

    #[test]
    fn a_wrong_check_shows_whats_right() {
        let mut d = SoundDetective::new(Sound::detached(), Rng::new(2));
        d.level = 3;
        d.new_case();
        d.check();
        let r = d.right.expect("feedback after a wrong check");
        assert!(r[3] && r[4], "the given controls count as right");
    }

    #[test]
    fn brightness_really_filters() {
        // A saw through the darkest and brightest settings: the dark one
        // must have much less energy in its upper harmonics.
        let hf = |b: usize| {
            let mut v = Voice { patch: [2, 1, b, 0, 2], hz: 220.0, t: 0.0, phase: 0.0, filt: Svf::default() };
            let x: Vec<f32> = (0..9600).map(|_| render(&mut v, 48_000.0)).collect();
            // First difference boosts highs: a crude high-frequency measure.
            x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>() / x.iter().map(|s| s * s).sum::<f32>()
        };
        assert!(hf(0) * 20.0 < hf(4), "{} vs {}", hf(0), hf(4));
    }

    #[test]
    fn the_mystery_and_yours_both_play() {
        let mut d = SoundDetective::new(Sound::detached(), Rng::new(1));
        let mut p = d.audio_processor().unwrap();
        d.tick(&Input { grid: std::array::from_fn(|g| g == 1), ..Default::default() });
        assert!(energy(&run(&mut p, 120)) > 1e-5);
        d.tick(&Input { grid: std::array::from_fn(|g| g == 8), ..Default::default() });
        assert!(energy(&run(&mut p, 20)) > 1e-5);
        let mut fb = FrameBuffer::new();
        d.draw(&mut fb);
    }
}
