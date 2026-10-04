//! Beat Lab (Kids, ages 10-12): a 16-step drum machine that explains
//! itself.
//!
//! Four tracks (kick, snare, hats and a percussion track), 16 steps each.
//! The pads are the 16 steps of the selected track, in reading order
//! (the top row is beat 1). Up/down picks a row of the menu on the left:
//! one of the tracks, the tempo, the swing or a style; left/right changes
//! it (a track's sound, the speed, how much swing, which style to load).
//! SELECT plays and stops; hold SELECT clears the selected track.
//!
//! The styles are starting points built from each genre's defining
//! rhythm, with a line on what makes it that style: rock's backbeat,
//! house's four-on-the-floor, boom bap's swung hats, reggaeton's dembow
//! (3+3+2), drum and bass's fast broken kick, funk's ghost notes and the
//! bossa nova clave.
//!
//! Swing delays every second 16th note; at 33% the pairs become a
//! triplet shuffle (see `Sound::set_swing`).

use crate::app::{App, Input, SlintExtra};
use crate::audio::AudioProcessor;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::apps::kids_kit::{self as kit, Drum, Ev, Size2, Song, Sound};
use std::sync::{Arc, Mutex};

const NAME: &str = "Beat Lab";
const TRACKS: usize = 4;

/// The sounds each track can use.
const TRACK_SOUNDS: [&[Drum]; TRACKS] = [
    &[Drum::Kick, Drum::LowTom],
    &[Drum::Snare, Drum::Clap, Drum::Rim],
    &[Drum::Hat, Drum::OpenHat, Drum::Shaker],
    &[Drum::Clap, Drum::Cowbell, Drum::Rim, Drum::Woodblock, Drum::HiTom, Drum::LowTom, Drum::Shaker, Drum::OpenHat],
];
const TRACK_NAMES: [&str; TRACKS] = ["Kick", "Snare", "Hats", "Perc"];

/// A step: 0 off, 1 soft (a ghost note), 2 normal, 3 accent.
type Pattern = [[u8; 16]; TRACKS];

#[derive(Clone, Copy)]
struct Style {
    name: &'static str,
    tempo: f32,
    swing: f32,
    /// Sound index per track.
    sounds: [usize; TRACKS],
    /// One string per track: '.' off, 'g' ghost, 'x' hit, 'X' accent.
    rows: [&'static str; TRACKS],
    about: &'static str,
}

const STYLES: [Style; 7] = [
    Style {
        name: "Rock",
        tempo: 112.0,
        swing: 0.0,
        sounds: [0, 0, 0, 1],
        rows: ["X.......x.x.....", "....X.......X...", "x.x.x.x.x.x.x.x.", "................"],
        about: "The backbeat: snare on beats 2 and 4, eighth-note hats, kick on 1 and 3.",
    },
    Style {
        name: "Boom bap",
        tempo: 90.0,
        swing: 0.18,
        sounds: [0, 0, 0, 0],
        rows: ["X.....x...x.....", "....X.......X...", "x.x.x.x.x.x.x.x.", "...........g...."],
        about: "Old-school hip hop: a heavy, lazy kick, a hard snare, and swung hats.",
    },
    Style {
        name: "House",
        tempo: 124.0,
        swing: 0.0,
        sounds: [0, 1, 1, 6],
        rows: ["X...X...X...X...", "....X.......X...", "..x...x...x...x.", "x.xxx.xxx.xxx.xx"],
        about: "Four on the floor: a kick on every beat, open hats between them.",
    },
    Style {
        name: "Reggaeton",
        tempo: 96.0,
        swing: 0.0,
        sounds: [0, 2, 2, 0],
        rows: ["X...X...X...X...", "...x..x....x..x.", "................", "................"],
        about: "The dembow: the snare plays 3+3+2 against a steady kick.",
    },
    Style {
        name: "Drum & bass",
        tempo: 170.0,
        swing: 0.0,
        sounds: [0, 0, 0, 2],
        rows: ["X.........x.....", "....X.......X...", "x.x.x.x.x.x.x.x.", ".......g......g."],
        about: "Fast and broken: the kick skips ahead of the beat, the snare stays on 2 and 4.",
    },
    Style {
        name: "Funk",
        tempo: 100.0,
        swing: 0.06,
        sounds: [0, 0, 0, 1],
        rows: ["X..x......x..x..", "....X..g.g..X..g", "xxxxxxxxxxxxxxxx", "................"],
        about: "Ghost notes: quiet snares between the loud ones keep it moving.",
    },
    Style {
        name: "Bossa nova",
        tempo: 128.0,
        swing: 0.0,
        sounds: [0, 2, 2, 2],
        rows: ["X..x....X..x....", "X..x..x...x..x..", "xxxxxxxxxxxxxxxx", "................"],
        about: "From Brazil: a soft rocking kick and the clave pattern on the rim.",
    },
];

fn parse(style: &Style) -> Pattern {
    std::array::from_fn(|t| {
        let row = style.rows[t].as_bytes();
        std::array::from_fn(|s| match row.get(s) {
            Some(b'X') => 3,
            Some(b'x') => 2,
            Some(b'g') => 1,
            _ => 0,
        })
    })
}

fn level_vel(l: u8) -> f32 {
    match l {
        1 => 0.35,
        2 => 0.75,
        _ => 1.0,
    }
}

#[derive(Clone)]
struct Kit {
    pattern: Pattern,
    sounds: [usize; TRACKS],
    mute: [bool; TRACKS],
}

struct Player {
    kit: Arc<Mutex<Kit>>,
}

impl Song for Player {
    fn step(&mut self, step: u64, out: &mut Vec<Ev>) {
        let Ok(k) = self.kit.try_lock() else { return };
        let s = (step % 16) as usize;
        for t in 0..TRACKS {
            let l = k.pattern[t][s];
            if l > 0 && !k.mute[t] {
                out.push(Ev::Drum(TRACK_SOUNDS[t][k.sounds[t] % TRACK_SOUNDS[t].len()], level_vel(l)));
            }
        }
    }
}

/// The menu rows on the left.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Track(usize),
    Tempo,
    Swing,
    Style,
}
const ROWS: [Row; 7] = [Row::Track(0), Row::Track(1), Row::Track(2), Row::Track(3), Row::Tempo, Row::Swing, Row::Style];

pub struct BeatLab {
    sound: Sound,
    kit: Arc<Mutex<Kit>>,
    row: usize,
    track: usize,
    style: usize,
    style_loaded: Option<usize>,
    prev: [bool; 16],
    flash: [f32; TRACKS],
    last_step: Option<u64>,
}

impl BeatLab {
    pub fn new(sound: Sound) -> BeatLab {
        let st = STYLES[0];
        sound.set_tempo(st.tempo);
        sound.set_swing(st.swing);
        sound.set_reverb(0.1);
        BeatLab { sound, kit: Arc::new(Mutex::new(Kit { pattern: parse(&st), sounds: st.sounds, mute: [false; TRACKS] })), row: 0, track: 0, style: 0, style_loaded: Some(0), prev: [false; 16], flash: [0.0; TRACKS], last_step: None }
    }

    fn load_style(&mut self, i: usize) {
        let st = STYLES[i];
        let mut k = self.kit.lock().unwrap();
        k.pattern = parse(&st);
        k.sounds = st.sounds;
        drop(k);
        self.sound.set_tempo(st.tempo);
        self.sound.set_swing(st.swing);
        self.style_loaded = Some(i);
    }

    /// What the selected row teaches.
    fn tip(&self) -> String {
        match ROWS[self.row] {
            Row::Track(0) => "The kick is the biggest drum: it's where you'd stamp your foot.".into(),
            Row::Track(1) => "The snare (or clap) answers the kick. Tap a step again to make it softer, then loud.".into(),
            Row::Track(2) => "Hats keep time between the big drums: try every step for a busier feel.".into(),
            Row::Track(_) => "Percussion adds colour. Left/right changes its sound.".into(),
            Row::Tempo => format!("Tempo is speed, in beats per minute. {:.0} bpm is {:.1} beats a second.", self.sound.tempo(), self.sound.tempo() / 60.0),
            Row::Swing => "Swing makes every second 16th note late. A little sounds human; lots sounds like a shuffle.".into(),
            Row::Style => format!("{}: {}", STYLES[self.style].name, STYLES[self.style].about),
        }
    }
}

impl App for BeatLab {
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
            ("Track".into(), TRACK_NAMES[self.track].into(), false),
            ("Tempo".into(), format!("{:.0} bpm", self.sound.tempo()), false),
            ("Swing".into(), format!("{:.0}%", self.sound.swing() * 100.0), false),
            ("Style".into(), STYLES[self.style].name.into(), false),
        ]
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
        let k = self.kit.lock().unwrap();
        let now = self.sound.step().map(|s| (s % 16) as usize);
        std::array::from_fn(|s| {
            if now == Some(s) {
                PadColor::Green
            } else {
                match k.pattern[self.track][s] {
                    0 => PadColor::Off,
                    1 => PadColor::Blue,
                    2 => PadColor::Yellow,
                    _ => PadColor::Red,
                }
            }
        })
    }

    fn tick(&mut self, input: &Input) {
        for s in 0..16 {
            if input.grid[s] && !self.prev[s] {
                let mut k = self.kit.lock().unwrap();
                // Off -> normal -> accent -> ghost -> off.
                let l = &mut k.pattern[self.track][s];
                *l = match *l {
                    0 => 2,
                    2 => 3,
                    3 => 1,
                    _ => 0,
                };
                let (l, snd) = (*l, k.sounds[self.track]);
                drop(k);
                if l > 0 && !self.sound.playing() {
                    self.sound.drum(TRACK_SOUNDS[self.track][snd % TRACK_SOUNDS[self.track].len()], level_vel(l));
                }
                self.style_loaded = None;
            }
        }
        self.prev = input.grid;
        if input.navigation_steps != 0 {
            self.row = (self.row as i32 + input.navigation_steps).clamp(0, ROWS.len() as i32 - 1) as usize;
            if let Row::Track(t) = ROWS[self.row] {
                self.track = t;
            }
        }
        if input.knob2 != 0 {
            let d = input.knob2.signum();
            match ROWS[self.row] {
                Row::Track(t) => {
                    let mut k = self.kit.lock().unwrap();
                    let n = TRACK_SOUNDS[t].len() as i32;
                    k.sounds[t] = (k.sounds[t] as i32 + d).rem_euclid(n) as usize;
                    let snd = TRACK_SOUNDS[t][k.sounds[t]];
                    drop(k);
                    self.sound.drum(snd, 0.8);
                }
                Row::Tempo => self.sound.set_tempo(((self.sound.tempo() / 2.0).round() * 2.0 + d as f32 * 2.0).clamp(60.0, 180.0)),
                Row::Swing => self.sound.set_swing(((self.sound.swing() * 50.0).round() + d as f32 * 3.0) / 50.0),
                Row::Style => {
                    self.style = (self.style as i32 + d).rem_euclid(STYLES.len() as i32) as usize;
                    self.load_style(self.style);
                }
            }
        }
        if input.knob2_press {
            self.kit.lock().unwrap().pattern[self.track] = [0; 16];
            self.style_loaded = None;
        } else if input.knob1_press {
            self.toggle_running();
        }
        if let Some(step) = self.sound.step() {
            if self.last_step != Some(step) {
                let k = self.kit.lock().unwrap();
                for t in 0..TRACKS {
                    if k.pattern[t][(step % 16) as usize] > 0 {
                        self.flash[t] = 1.0;
                    }
                }
            }
            self.last_step = Some(step);
        } else {
            self.last_step = None;
        }
        for f in self.flash.iter_mut() {
            *f = (*f - 0.08).max(0.0);
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let panel = kit::rgb(232, 240, 232);
        let colors = [kit::rgb(239, 116, 106), kit::rgb(239, 190, 80), kit::rgb(85, 190, 184), kit::rgb(166, 135, 218)];
        kit::kids_scene(fb, kit::PlayWorld::Sky);
        kit::kids_header(fb, NAME, "AGES 10-12");
        let k = self.kit.lock().unwrap().clone();

        // The list stays in its existing navigation order. Only the selected
        // instrument's 16 pads are expanded, matching the physical 4x4 grid.
        for (i, row) in ROWS.iter().enumerate() {
            let y = 42 + i as i32 * 41;
            let selected = i == self.row;
            let (label, value) = match row {
                Row::Track(t) => (TRACK_NAMES[*t].to_string(), TRACK_SOUNDS[*t][k.sounds[*t] % TRACK_SOUNDS[*t].len()].name().to_string()),
                Row::Tempo => ("Tempo".into(), format!("{:.0} bpm", self.sound.tempo())),
                Row::Swing => ("Swing".into(), format!("{:.0}%", self.sound.swing() * 100.0)),
                Row::Style => ("Style".into(), STYLES[self.style].name.to_string()),
            };
            kit::card(fb, 10, y, 226, 36, 12, if selected { kit::rgb(202, 230, 220) } else { panel });
            if let Row::Track(t) = row {
                kit::round_rect(fb, 16, y + 7, 5, 22, 3, colors[*t]);
                // Small activity marks keep the other instruments visible.
                for step in 0..16 {
                    if k.pattern[*t][step] > 0 { kit::circle(fb, 120 + step as i32 * 6, y + 28, 2, colors[*t]); }
                }
            }
            kit::kids_label(fb, &label, 27, y + 2, kit::INK, -1);
            kit::text(fb, &value, 28, y + 23, Size2::Small, kit::INK, -1);
            if selected {
                kit::outline(fb, 10, y, 226, 36, 12, 2, kit::TEAL);
                kit::text(fb, "< >", 224, y + 9, Size2::Medium, kit::INK, 1);
            }
        }
        kit::card(fb, 250, 40, 380, 228, 18, kit::PAPER);
        kit::kids_title(fb, TRACK_NAMES[self.track], 264, 46, kit::INK, -1);
        kit::round_rect(fb, 526, 46, 90, 26, 13, kit::TEAL);
        kit::text(fb, if self.sound.playing() { "PLAYING" } else { "READY" }, 571, 52, Size2::Medium, kit::WHITE, 0);
        kit::text(fb, "Tap: on > loud > soft > off", 264, 79, Size2::Medium, kit::INK, -1);
        let now = self.sound.step().map(|s| (s % 16) as usize);
        for step in 0..16 {
            let x = 264 + (step as i32 % 4) * 88;
            let y = 102 + (step as i32 / 4) * 41;
            let level = k.pattern[self.track][step];
            let c = if level == 0 { panel } else { kit::blend(colors[self.track], kit::WHITE, 0.35) };
            kit::card(fb, x, y, 80, 35, 12, c);
            kit::kids_label(fb, &(step + 1).to_string(), x + 10, y + 7, kit::INK, -1);
            if level == 0 {
                kit::ring(fb, x + 59, y + 18, 6, 2, kit::MUTED);
            } else {
                // One/two/three bars mean soft/on/loud without relying on color.
                for bar in 0..level {
                    let h = 6 + bar as i32 * 4;
                    kit::round_rect(fb, x + 47 + bar as i32 * 8, y + 25 - h, 5, h, 2, kit::INK);
                }
            }
            if now == Some(step) {
                kit::outline(fb, x - 2, y - 2, 84, 39, 12, 3, kit::INK);
            }
        }
        kit::card(fb, 250, 274, 380, 60, 14, kit::PAPER);
        kit::paragraph(fb, &self.tip(), 260, 279, 360, Size2::Medium, kit::INK);
        let play = if self.sound.playing() { "SELECT: stop" } else { "SELECT: play" };
        kit::kids_footer(fb, &format!("Pads: steps 1-16   up/down: menu   left/right: change   {play}   hold SELECT: clear"));
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(self.sound.processor(Some(Box::new(Player { kit: Arc::clone(&self.kit) })), None))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(BeatLab::new(Sound::new(NAME, &modbus, &mixer, &bus)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render};

    #[test]
    fn every_style_is_sixteen_steps_on_every_track() {
        for st in STYLES {
            for r in st.rows {
                assert_eq!(r.len(), 16, "{}", st.name);
            }
            assert!((60.0..=180.0).contains(&st.tempo));
        }
        // The dembow really is 3+3+2 against the beat: hits 3 steps, 3+2, ...
        let d = parse(&STYLES[3]);
        let hits: Vec<usize> = (0..16).filter(|&s| d[1][s] > 0).collect();
        assert_eq!(hits, vec![3, 6, 11, 14]);
    }

    #[test]
    fn a_pad_cycles_on_loud_soft_off() {
        let mut app = BeatLab::new(Sound::detached());
        app.kit.lock().unwrap().pattern[0] = [0; 16];
        let pad = |i: usize| Input { grid: std::array::from_fn(|g| g == i), ..Default::default() };
        let mut seen = Vec::new();
        for _ in 0..4 {
            app.tick(&pad(5));
            app.tick(&Input::default());
            seen.push(app.kit.lock().unwrap().pattern[0][5]);
        }
        assert_eq!(seen, vec![2, 3, 1, 0]);
    }

    #[test]
    fn styles_load_and_play() {
        let mut app = BeatLab::new(Sound::detached());
        let mut p = app.audio_processor().unwrap();
        for _ in 0..6 {
            app.tick(&Input { navigation_steps: 1, ..Default::default() });
        }
        assert_eq!(ROWS[app.row], Row::Style);
        app.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(app.style, 1);
        assert!((app.sound.swing() - 0.18).abs() < 1e-6);
        app.tick(&Input { knob1_press: true, ..Default::default() });
        assert!(energy(&render(&mut p, 100)) > 1e-4);
        let mut fb = FrameBuffer::new();
        app.tick(&Input::default());
        app.draw(&mut fb);
    }

    #[test]
    fn swing_delays_the_offbeat() {
        let snd = Sound::detached();
        snd.set_reverb(0.0);
        snd.set_tempo(120.0);
        snd.set_swing(0.3);
        let k = Kit { pattern: [[0; 16], [2; 16], [0; 16], [0; 16]], sounds: [0, 2, 0, 0], mute: [false; TRACKS] };
        let mut p = snd.processor(Some(Box::new(Player { kit: Arc::new(Mutex::new(k)) })), None);
        snd.start();
        let out = render(&mut p, 20);
        // 16ths at 120 bpm are 6000 frames; with 30% swing the second one
        // lands at 7800, not 6000.
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let onset = |from: usize| (from..left.len()).find(|&i| left[i].abs() > 0.02).unwrap();
        let second = onset(5000);
        assert!((second as i32 - 7800).abs() < 40, "{second}");
    }
}
