//! Bebot-inspired XY performance instrument, implemented independently.
//! This is not Normalware's DSP or artwork and does not claim sonic parity.
//! The existing fullscreen Slint pointer bridge supplies press/drag/release.
use crate::app::{App, Input, SlintExtra};
use crate::apps::kids_kit::{self as kit, Size2};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
#[path = "dsp.rs"]
mod dsp;
const NAME: &str = "Bebot";
const BG: Rgb565 = Rgb565::new(2, 5, 7);
const TEAL: Rgb565 = Rgb565::new(8, 57, 26);
const INK: Rgb565 = Rgb565::new(27, 58, 30);
const DIM: Rgb565 = Rgb565::new(16, 37, 22);
const MODES: [&str; 4] = ["Saw", "Pulse", "Sine", "PWM"];
const SCALES: [&str; 5] = ["Chromatic", "Major", "Minor", "Pentatonic", "Blues"];
const TUNES: [&str; 4] = ["Free", "Snap", "Slow", "Fast"];
const LABELS: [&str; 17] = [
    "Mode",
    "Scale",
    "Tune",
    "Root",
    "Low note",
    "Span",
    "Resonance",
    "Pulse width",
    "PWM cutoff",
    "Attack",
    "Release",
    "Overdrive",
    "Post drive",
    "Chorus",
    "Echo mix",
    "Echo time",
    "Echo repeat",
];
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Patch {
    pub v: [f32; 16],
    pub feedback: f32,
}
impl Default for Patch {
    fn default() -> Self {
        Self {
            v: [
                0., 3., 1., 0., 48., 24., 0.35, 0.5, 6000., 0.008, 0.12, 0.08, 0., 0.15, 0.15, 0.28,
            ],
            feedback: 0.3,
        }
    }
}
impl Patch {
    fn bounds(i: usize) -> (f32, f32, f32) {
        match i {
            0 => (0., 3., 1.),
            1 => (0., 4., 1.),
            2 => (0., 3., 1.),
            3 => (0., 11., 1.),
            4 => (24., 84., 1.),
            5 => (12., 48., 12.),
            6 | 7 | 11 | 13 | 14 => (0., 1., 0.025),
            8 => (100., 14000., 200.),
            9 => (0.002, 1., 0.01),
            10 => (0.01, 2., 0.025),
            12 => (0., 1., 1.),
            _ => (0.005, 2., 0.025),
        }
    }
    fn sanitize(&mut self) {
        for i in 0..16 {
            let (lo, hi, _) = Self::bounds(i);
            self.v[i] = if self.v[i].is_finite() {
                self.v[i].clamp(lo, hi)
            } else {
                lo
            };
            if i <= 5 || i == 12 {
                self.v[i] = self.v[i].round();
            }
        }
        self.feedback = if self.feedback.is_finite() {
            self.feedback.clamp(0., 0.9)
        } else {
            0.3
        };
    }
}
#[derive(Clone, Copy)]
pub struct Control {
    pub patch: Patch,
    pub notes: [f32; 8],
    pub gates: [f32; 8],
    pub y: [f32; 8],
    pub bend: f32,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            patch: Patch::default(),
            notes: [60.; 8],
            gates: [0.; 8],
            y: [0.7; 8],
            bend: 0.,
        }
    }
}
pub struct BebotApp {
    shared: Arc<Mutex<Control>>,
    state: Control,
    pointer: Option<(f32, f32)>,
    keys: [Option<usize>; 7],
    menu: bool,
    selected: usize,
    preset: usize,
    phase: f32,
    status: String,
    peak: Arc<AtomicF32>,
    level: Arc<AtomicF32>,
    ext: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
}
impl BebotApp {
    pub fn new(bus: Arc<AudioBus>, mixer: Arc<MixerBus>, mods: Arc<ModBus>) -> Self {
        let (level, ext) = mixer.register(NAME, &mods);
        Self {
            shared: Arc::new(Mutex::new(Control::default())),
            state: Control::default(),
            pointer: None,
            keys: [None; 7],
            menu: false,
            selected: 0,
            preset: 0,
            phase: 0.,
            status: String::new(),
            peak: Arc::new(AtomicF32::new(0.)),
            level,
            ext,
            output: bus.register(NAME),
        }
    }
    fn publish(&self) {
        *self.shared.lock().unwrap() = self.state;
    }
    fn load_factory(&mut self, i: usize) {
        self.preset = i % 8;
        self.state.patch = Patch::default();
        let v = &mut self.state.patch.v;
        match self.preset {
            1 => {
                v[0] = 1.;
                v[7] = 0.23;
                v[6] = 0.5;
            }
            2 => {
                v[0] = 2.;
                v[2] = 0.;
                v[13] = 0.;
                v[14] = 0.25;
            }
            3 => {
                v[0] = 3.;
                v[13] = 0.35;
                v[8] = 3500.;
            }
            4 => {
                v[6] = 0.7;
                v[11] = 0.4;
                v[14] = 0.3;
            }
            5 => {
                v[4] = 36.;
                v[5] = 12.;
                v[14] = 0.;
                v[13] = 0.;
                v[10] = 0.06;
            }
            6 => {
                v[9] = 0.3;
                v[10] = 0.8;
                v[13] = 0.5;
                v[14] = 0.35;
            }
            7 => {
                v[2] = 0.;
                v[5] = 48.;
                v[11] = 0.7;
                v[12] = 1.;
            }
            _ => {}
        }
        self.publish();
    }
    fn preset_name(&self) -> &str {
        [
            "Robot song",
            "Narrow pulse",
            "Whistle",
            "PWM choir",
            "Acid bot",
            "Bass bot",
            "Cloud robot",
            "Space chatter",
        ][self.preset]
    }
    fn value(&self, i: usize) -> String {
        if i == 16 {
            return format!("{:.0}%", self.state.patch.feedback * 100.);
        }
        let x = self.state.patch.v[i];
        match i {
            0 => MODES[x as usize].into(),
            1 => SCALES[x as usize].into(),
            2 => TUNES[x as usize].into(),
            3 => [
                "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
            ][x as usize]
                .into(),
            12 => if x > 0.5 { "On" } else { "Off" }.into(),
            4 | 5 | 8 => format!("{x:.0}"),
            9 | 10 | 15 => format!("{x:.3} s"),
            _ => format!("{:.0}%", x * 100.),
        }
    }
    fn edit(&mut self, i: usize, d: f32) {
        if i == 16 {
            self.state.patch.feedback = (self.state.patch.feedback + d * 0.025).clamp(0., 0.9);
            self.publish();
            return;
        }
        let (lo, hi, step) = Patch::bounds(i);
        self.state.patch.v[i] = (self.state.patch.v[i] + d * step).clamp(lo, hi);
        self.state.patch.sanitize();
        self.publish();
    }
    fn save(&mut self) {
        let path = std::env::var_os("PORTAMAX_SAVES_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "saves".into())
            .join("bebot");
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            std::fs::create_dir_all(&path)?;
            std::fs::write(
                path.join("user.json"),
                serde_json::to_vec_pretty(&self.state.patch)?,
            )?;
            Ok(())
        })();
        self.status = if result.is_ok() {
            "Saved user preset".into()
        } else {
            "Could not save preset".into()
        };
    }
    fn load(&mut self) {
        let path = std::env::var_os("PORTAMAX_SAVES_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "saves".into())
            .join("bebot/user.json");
        match std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Patch>(&b).ok())
        {
            Some(mut patch) => {
                patch.sanitize();
                self.state.patch = patch;
                self.publish();
                self.status = "Loaded user preset".into();
            }
            None => self.status = "No user preset saved".into(),
        }
    }
}
impl App for BebotApp {
    fn supports_pad_lock(&self) -> bool {
        true
    }
    fn play_surface(&self) -> bool {
        true
    }
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn hint(&self) -> String {
        "Hold left mouse + drag: pitch / tone. Release: stop. SELECT: settings".into()
    }
    fn on_exit(&mut self) {
        self.pointer = None;
        self.keys.fill(None);
        self.state.gates.fill(0.);
        self.publish();
    }
    fn needs_background_audio(&self) -> bool {
        self.state.gates.iter().any(|x| *x > 0.) || self.peak.get() > 0.0001
    }
    fn slint_pointer_pick(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() || x < 0. || y < 0. {
            self.pointer = None;
            self.state.gates[0] = 0.;
            self.publish();
            return;
        }
        // A drag remains a playing gesture even when it crosses toolbar controls.
        if self.pointer.is_some() {
            let nx = x.clamp(0., 639.) / 639.;
            let ny = (1. - (y - 42.) / 294.).clamp(0., 1.);
            self.pointer = Some((nx, ny));
            self.state.notes[0] = self.state.patch.v[4] + nx * self.state.patch.v[5];
            self.state.y[0] = ny;
            self.publish();
            return;
        }
        if x >= 640. || y >= 360. {
            return;
        }
        if y < 40. {
            if x < 55. {
                self.load_factory((self.preset + 7) % 8);
            } else if x < 290. {
                self.load_factory(self.preset + 1);
            } else if x < 395. {
                self.edit(0, if self.state.patch.v[0] >= 3. { -3. } else { 1. });
            } else if x < 505. {
                self.edit(2, if self.state.patch.v[2] >= 3. { -3. } else { 1. });
            } else {
                self.menu = !self.menu;
            }
            return;
        }
        if self.menu {
            if y >= 310. {
                if x < 200. {
                    self.save();
                } else if x < 400. {
                    self.load();
                } else {
                    self.menu = false;
                }
                return;
            }
            let col = (x / 320.) as usize;
            let row = ((y - 48.) / 28.).max(0.) as usize;
            if row < 9 && col * 9 + row < 17 {
                self.selected = col * 9 + row;
                self.edit(self.selected, if x % 320. < 160. { -1. } else { 1. });
            }
            return;
        }
        if y >= 338. {
            return;
        }
        let nx = x / 639.;
        let ny = (1. - (y - 42.) / 294.).clamp(0., 1.);
        self.pointer = Some((nx, ny));
        self.state.notes[0] = self.state.patch.v[4] + nx * self.state.patch.v[5];
        self.state.y[0] = ny;
        self.state.gates[0] = 1.;
        self.publish();
    }
    fn tick(&mut self, input: &Input) {
        self.phase += 0.045;
        if input.knob1_press || input.stick_click {
            self.menu = !self.menu;
        }
        if self.menu {
            self.selected = (self.selected as i32 + input.knob1 + input.navigation_steps)
                .rem_euclid(17) as usize;
            self.edit(self.selected, input.knob2 as f32);
        }
        self.state.bend = input.pitch_bend * 2.;
        let mut candidates = [None; 144];
        for i in 0..16 {
            if input.grid[i] {
                let degree = ((3 - i / 4) * 4 + i % 4) as i32;
                candidates[i] = Some((
                    dsp::degree_note(self.state.patch, degree),
                    0.8,
                    (input.pad_pressure[i] + input.stick[1] * 0.25).clamp(0., 1.),
                ));
            }
        }
        for n in 0..128 {
            if input.midi_keys.0[n] > 0 {
                candidates[16 + n] = Some((
                    n as f32,
                    input.midi_keys.0[n] as f32 / 127.,
                    (0.65 + input.mod_wheel * 0.35).clamp(0., 1.),
                ));
            }
        }
        // Keep a held key in its voice: releasing a lower key must not retune
        // all the higher keys into different envelope/filter states.
        for key in &mut self.keys {
            if key.is_some_and(|id| candidates[id].is_none()) {
                *key = None;
            }
        }
        for (id, value) in candidates.iter().enumerate() {
            if value.is_some() && !self.keys.contains(&Some(id)) {
                if let Some(slot) = self.keys.iter_mut().find(|k| k.is_none()) {
                    *slot = Some(id);
                }
            }
        }
        for (i, key) in self.keys.iter().enumerate() {
            let slot = i + 1;
            if let Some((note, gate, y)) = key.and_then(|id| candidates[id]) {
                self.state.notes[slot] = note;
                self.state.gates[slot] = gate;
                self.state.y[slot] = y;
            } else {
                self.state.gates[slot] = 0.;
            }
        }
        self.publish();
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        (0..17)
            .map(|i| (LABELS[i].into(), self.value(i), false))
            .collect()
    }
    fn instrument_settings(&self) -> Vec<crate::app::Setting> {
        (0..17)
            .map(|i| crate::app::Setting {
                label: LABELS[i].into(),
                value: self.value(i),
            })
            .collect()
    }
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        if index < 17 {
            self.edit(index, delta as f32);
        }
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(dsp::Processor::new(
            self.shared.clone(),
            self.level.clone(),
            self.ext.clone(),
            self.output.clone(),
            self.peak.clone(),
        )))
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn draw(&mut self, fb: &mut FrameBuffer) {
        kit::rect(fb, 0, 0, 640, 360, BG);
        kit::rect(fb, 0, 0, 640, 40, Rgb565::new(4, 12, 11));
        for (s, x) in [
            ("<", 20),
            (self.preset_name(), 65),
            (MODES[self.state.patch.v[0] as usize], 305),
            (TUNES[self.state.patch.v[2] as usize], 410),
            (if self.menu { "PLAY" } else { "SETTINGS" }, 530),
        ] {
            kit::text(fb, s, x, 14, Size2::Medium, INK, -1);
        }
        if self.menu {
            for i in 0..17 {
                let x = (i / 9) as i32 * 320 + 12;
                let y = (i % 9) as i32 * 28 + 49;
                if self.selected == i {
                    kit::rect(fb, x - 4, y, 310, 28, Rgb565::new(4, 19, 14));
                }
                kit::text(fb, LABELS[i], x, y + 6, Size2::Medium, DIM, -1);
                kit::text(fb, &self.value(i), x + 290, y + 6, Size2::Medium, TEAL, 1);
            }
            kit::text(fb, "SAVE USER", 30, 330, Size2::Small, TEAL, -1);
            kit::text(fb, "LOAD USER", 235, 330, Size2::Small, TEAL, -1);
            kit::text(fb, "BACK TO PLAY", 450, 330, Size2::Small, TEAL, -1);
            kit::text(fb, &self.status, 320, 344, Size2::Small, INK, 0);
            return;
        }
        for i in 0..=24 {
            let x = i * 640 / 24;
            let note = self.state.patch.v[4] + i as f32 / 24. * self.state.patch.v[5];
            let snapped = dsp::snap(note, self.state.patch);
            let c = if (note - snapped).abs() < 0.1 {
                Rgb565::new(4, 16, 12)
            } else {
                Rgb565::new(3, 9, 9)
            };
            kit::rect(fb, x, 42, 1, 294, c);
        }
        let active = self.state.gates.iter().any(|x| *x > 0.);
        let bob = if active {
            (self.phase.sin() * 7.) as i32
        } else {
            0
        };
        let cx = 320;
        let cy = 170 + bob;
        // Original squat radio robot: circular ears, glass visor and equalizer mouth.
        kit::circle(fb, cx - 103, cy, 23, DIM);
        kit::circle(fb, cx + 103, cy, 23, DIM);
        kit::rect(fb, cx - 92, cy - 68, 184, 143, DIM);
        kit::rect(fb, cx - 82, cy - 57, 164, 91, Rgb565::new(1, 8, 7));
        for dx in [-43, 43] {
            kit::circle(fb, cx + dx, cy - 13, 17, if active { TEAL } else { INK });
            kit::circle(fb, cx + dx + (self.state.y[0] * 6.) as i32, cy - 13, 6, BG);
        }
        for i in 0..7 {
            let h = if active {
                8 + ((self.phase + i as f32).sin().abs() * 19.) as i32
            } else {
                6
            };
            kit::rect(fb, cx - 48 + i * 15, cy + 51 - h / 2, 8, h, TEAL);
        }
        kit::rect(fb, cx - 3, cy - 95, 6, 28, DIM);
        kit::circle(fb, cx, cy - 98, 7, TEAL);
        kit::rect(fb, cx - 62, cy + 87, 124, 35, DIM);
        kit::text(fb, "SZYK", cx, cy + 98, Size2::Small, BG, 0);
        if let Some((x, y)) = self.pointer {
            let px = (x * 639.) as i32;
            let py = 42 + ((1. - y) * 294.) as i32;
            kit::circle(fb, px, py, 12, TEAL);
            kit::circle(fb, px, py, 7, BG);
        }
        kit::text(fb, "BEBOT / XY SYNTH", 16, 62, Size2::Medium, TEAL, -1);
        kit::text(
            fb,
            &format!(
                "{} / {}   {:.0}-{:.0}",
                SCALES[self.state.patch.v[1] as usize],
                TUNES[self.state.patch.v[2] as usize],
                self.state.patch.v[4],
                self.state.patch.v[4] + self.state.patch.v[5]
            ),
            16,
            86,
            Size2::Small,
            DIM,
            -1,
        );
        kit::text(
            fb,
            "HOLD + DRAG   X: PITCH   Y: TONE   RELEASE: STOP",
            320,
            344,
            Size2::Small,
            INK,
            0,
        );
    }
}
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    Box::new(BebotApp::new(ctx.get(), ctx.get(), ctx.get()))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
