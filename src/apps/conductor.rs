//! Conductor (AI): play the session with your hands, over the two depth
//! sensors.
//!
//! A 1-D convolutional network (`assets/npu/gesture.pmxn`, trained by
//! `tools/npu/train_gesture.py`) watches the last 0.8 s of both sensors,
//! 20 times a second, and recognises seven gestures: swipe right, swipe
//! left, push (both hands in), a wave over either sensor and a tap over
//! either sensor. Holding a hand still to play is not a gesture -- it was
//! trained on that too -- so hand heights stay free for continuous control.
//!
//! Six outputs, each patchable to any modulation input of any app:
//! - Left hand, Right hand: how close each hand is, continuously.
//! - Swipes: a value that steps up an eighth on a swipe right, down on a
//!   swipe left.
//! - Push: flips between 0 and 1 on each push.
//! - Waves: a new random value on each wave.
//! - Taps: a pulse that jumps to 1 on a tap and falls away.
//!
//! Its own voice (on by default) shows what the gestures do without any
//! patching: while your hands are over the sensors, a pad whose chord steps with swipes (I vi IV V ii V I IV),
//! whose filter follows your left hand and vibrato your right, that a
//! push mutes, a wave makes shimmer, and taps play a kick (left) and a
//! snare (right).

use crate::app::{App, Input, SlintExtra};
use crate::apps::hum::ai_header;
use crate::apps::kids_kit::{self as kit, Drum, Extra, Rng, Size2, Sound, Svf};
use crate::apps::neural::{self, Load, Model};
use crate::audio::AudioProcessor;
use crate::display::FrameBuffer;
use crate::modbus::{ModBus, Patch};
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::Arc;

const NAME: &str = "Conductor";
static MODEL: &[u8] = include_bytes!("../../assets/npu/gesture.pmxn");
const N: usize = 48;
const CLASSES: [&str; 8] = ["nothing", "swipe right", "swipe left", "push", "wave left", "wave right", "tap left", "tap right"];
/// Run the model every this many frames (20 a second at 60 fps).
const EVERY: u32 = 3;
const SURE: f32 = 0.8;
/// After a gesture, ignore others for this many frames.
const REFRACTORY: u32 = 24;
const OUTPUTS: [&str; 6] = ["Left hand", "Right hand", "Swipes", "Push", "Waves", "Taps"];
/// The voice's chords, as major-key degrees: I vi IV V ii V I IV.
const CHORDS: [i32; 8] = [0, 5, 3, 4, 1, 4, 0, 3];

/// The pad that demonstrates the gestures.
struct Pad {
    chord: Arc<AtomicI32>,
    cutoff: Arc<AtomicF32>,
    vibrato: Arc<AtomicF32>,
    muted: Arc<AtomicBool>,
    shimmer: Arc<AtomicF32>,
    on: Arc<AtomicBool>,
    /// How much hand is over the sensors: the pad sounds only while you play.
    presence: Arc<AtomicF32>,
    phase: [f32; 4],
    filt: Svf,
    amp: f32,
    lfo: f32,
}

impl Extra for Pad {
    fn frame(&mut self, sr: f32) -> (f32, f32) {
        let on = self.on.load(Ordering::Relaxed) && !self.muted.load(Ordering::Relaxed);
        let target = if on { 0.22 * (self.presence.get() * 3.0).min(1.0) } else { 0.0 };
        self.amp += (target - self.amp) * (1.0 - (-1.0 / (0.15 * sr)).exp());
        if self.amp < 1e-4 {
            return (0.0, 0.0);
        }
        let deg = CHORDS[(self.chord.load(Ordering::Relaxed).rem_euclid(8)) as usize];
        self.lfo = (self.lfo + 5.5 / sr).fract();
        let vib = 1.0 + self.vibrato.get() * 0.006 * (self.lfo * std::f32::consts::TAU).sin();
        let mut x = 0.0;
        let mut sh = 0.0;
        for (k, step) in [0, 2, 4, 7].iter().enumerate() {
            let n = kit::scale_note(48, &kit::MAJOR, deg + step) as f32;
            let hz = kit::midi_hz(n) * vib * (1.0 + 0.0015 * (k as f32 - 1.5));
            let dt = hz / sr;
            let p = self.phase[k];
            self.phase[k] = (p + dt).fract();
            x += kit::saw(p, dt) * 0.3;
            sh += (p * 4.0 * std::f32::consts::TAU).sin();
        }
        let cutoff = 200.0 * 2f32.powf(self.cutoff.get().clamp(0.0, 1.0) * 5.5);
        let y = self.filt.tick(x, cutoff, 1.2, sr).lp * self.amp + sh * 0.05 * self.shimmer.get() * self.amp * 4.0;
        (y, y)
    }
}

pub struct Conductor {
    sound: Sound,
    modbus: Arc<ModBus>,
    model: Model,
    load: Load,
    hist: [[f32; N]; 2],
    frame: u32,
    probs: [f32; 8],
    last_class: usize,
    refractory: u32,
    /// The latest gesture and frames since it.
    shown: Option<(usize, u32)>,
    outs: [f32; 6],
    targets: [AtomicUsize; 6],
    selected: usize,
    row: usize,
    rng: Rng,
    chord: Arc<AtomicI32>,
    cutoff: Arc<AtomicF32>,
    vibrato: Arc<AtomicF32>,
    muted: Arc<AtomicBool>,
    shimmer: Arc<AtomicF32>,
    voice_on: Arc<AtomicBool>,
    presence: Arc<AtomicF32>,
    count: [u32; 8],
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Voice,
    Output,
    App,
    Input,
}
const ROWS: [Row; 4] = [Row::Voice, Row::Output, Row::App, Row::Input];

impl Conductor {
    pub fn new(sound: Sound, modbus: Arc<ModBus>) -> Conductor {
        let model = Model::from_bytes(MODEL).expect("gesture model");
        let load = Load::new(model.macs());
        sound.set_reverb(0.35);
        Conductor {
            sound,
            modbus,
            model,
            load,
            hist: [[0.0; N]; 2],
            frame: 0,
            probs: [0.0; 8],
            last_class: 0,
            refractory: 0,
            shown: None,
            outs: [0.0, 0.0, 0.5, 0.0, 0.5, 0.0],
            targets: std::array::from_fn(|_| AtomicUsize::new(0)),
            selected: 0,
            row: 0,
            rng: Rng::seeded_from_time(),
            chord: Arc::new(AtomicI32::new(0)),
            cutoff: Arc::new(AtomicF32::new(0.5)),
            vibrato: Arc::new(AtomicF32::new(0.0)),
            muted: Arc::new(AtomicBool::new(false)),
            shimmer: Arc::new(AtomicF32::new(0.0)),
            voice_on: Arc::new(AtomicBool::new(true)),
            presence: Arc::new(AtomicF32::new(0.0)),
            count: [0; 8],
        }
    }

    /// Runs the model on the current window; returns a gesture when one
    /// is recognised.
    pub fn classify(&mut self) -> Option<usize> {
        let mut x = [0.0f32; 2 * N];
        x[..N].copy_from_slice(&self.hist[0]);
        x[N..].copy_from_slice(&self.hist[1]);
        let t = std::time::Instant::now();
        self.model.run(&x, &mut self.probs);
        neural::softmax(&mut self.probs);
        self.load.tick(t.elapsed());
        let c = neural::argmax(&self.probs);
        // Two windows in a row must agree, which rules out half-seen gestures.
        let fire = c != 0 && self.probs[c] >= SURE && c == self.last_class && self.refractory == 0;
        self.last_class = if self.probs[c] >= SURE { c } else { 0 };
        if fire {
            self.refractory = REFRACTORY;
            Some(c)
        } else {
            None
        }
    }

    fn gesture(&mut self, g: usize) {
        self.shown = Some((g, 0));
        self.count[g] += 1;
        match g {
            1 => {
                self.outs[2] = (self.outs[2] + 0.125).min(1.0);
                self.chord.fetch_add(1, Ordering::Relaxed);
            }
            2 => {
                self.outs[2] = (self.outs[2] - 0.125).max(0.0);
                self.chord.fetch_sub(1, Ordering::Relaxed);
            }
            3 => {
                self.outs[3] = 1.0 - self.outs[3];
                let m = !self.muted.load(Ordering::Relaxed);
                self.muted.store(m, Ordering::Relaxed);
            }
            4 | 5 => {
                self.outs[4] = self.rng.next_u32() as f32 / u32::MAX as f32;
                self.shimmer.set(1.0);
            }
            6 | 7 => {
                self.outs[5] = 1.0;
                if self.voice_on.load(Ordering::Relaxed) {
                    self.sound.drum(if g == 6 { Drum::Kick } else { Drum::Snare }, 0.9);
                }
            }
            _ => {}
        }
    }

    fn send(&self) {
        for (i, t) in self.targets.iter().enumerate() {
            let t = t.load(Ordering::Relaxed);
            if t > 0 {
                if let Some(h) = self.modbus.get(t - 1) {
                    h.set(self.outs[i]);
                }
            }
        }
    }
}

impl App for Conductor {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn play_surface(&self) -> bool {
        true
    }
    fn needs_background_audio(&self) -> bool {
        self.targets.iter().any(|t| t.load(Ordering::Relaxed) > 0)
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        OUTPUTS.iter().enumerate().map(|(i, n)| (n.to_string(), Patch::label(&self.modbus, self.targets[i].load(Ordering::Relaxed)), false)).collect()
    }

    fn tick(&mut self, input: &Input) {
        self.frame += 1;
        for c in 0..2 {
            self.hist[c].rotate_left(1);
            self.hist[c][N - 1] = input.hands[c].clamp(0.0, 1.0);
        }
        self.refractory = self.refractory.saturating_sub(1);
        if self.frame % EVERY == 0 {
            if let Some(g) = self.classify() {
                self.gesture(g);
            }
        }
        self.outs[0] = input.hands[0].clamp(0.0, 1.0);
        self.outs[1] = input.hands[1].clamp(0.0, 1.0);
        self.outs[5] *= 0.93;
        if input.hands[0] > 0.02 {
            self.cutoff.set(input.hands[0]);
        }
        self.vibrato.set(input.hands[1]);
        let near = input.hands[0].max(input.hands[1]).clamp(0.0, 1.0);
        let p = self.presence.get();
        // Comes in quickly, lingers for a couple of seconds after the hands go.
        self.presence.set(if near > p { near } else { p * 0.985 });
        self.shimmer.set(self.shimmer.get() * 0.97);
        if let Some((g, t)) = self.shown {
            self.shown = if t > 90 { None } else { Some((g, t + 1)) };
        }
        self.send();

        if input.navigation_steps != 0 {
            self.row = (self.row as i32 + input.navigation_steps).clamp(0, ROWS.len() as i32 - 1) as usize;
        }
        if input.knob2 != 0 {
            let d = input.knob2.signum();
            let t = &self.targets[self.selected];
            match ROWS[self.row] {
                Row::Voice => {
                    let v = !self.voice_on.load(Ordering::Relaxed);
                    self.voice_on.store(v, Ordering::Relaxed);
                }
                Row::Output => self.selected = (self.selected as i32 + d).rem_euclid(OUTPUTS.len() as i32) as usize,
                Row::App => t.store(Patch::step_app(&self.modbus, t.load(Ordering::Relaxed), d), Ordering::Relaxed),
                Row::Input => t.store(Patch::step_input(&self.modbus, t.load(Ordering::Relaxed), d), Ordering::Relaxed),
            }
        }
        if input.knob2_press {
            self.targets[self.selected].store(0, Ordering::Relaxed);
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = Rgb565::new(4, 3, 8);
        let panel = Rgb565::new(8, 6, 15);
        let dim = Rgb565::new(16, 24, 26);
        let ink = kit::rgb(200, 170, 250);
        kit::clear(fb, bg);
        ai_header(fb, NAME, panel);
        // The two sensors, last 0.8 s.
        let (sx, sy, sw, sh) = (8, 38, 300, 120);
        kit::round_rect(fb, sx, sy, sw, sh, 8, panel);
        let colors = [kit::rgb(250, 150, 90), kit::rgb(90, 190, 250)];
        for c in 0..2 {
            for t in 1..N {
                let x0 = sx + 8 + (t as i32 - 1) * (sw - 16) / (N as i32 - 1);
                let x1 = sx + 8 + t as i32 * (sw - 16) / (N as i32 - 1);
                let y0 = sy + sh - 10 - (self.hist[c][t - 1] * (sh - 20) as f32) as i32;
                let y1 = sy + sh - 10 - (self.hist[c][t] * (sh - 20) as f32) as i32;
                kit::line(fb, x0, y0, x1, y1, 2, colors[c]);
            }
        }
        kit::text(fb, "left", sx + 10, sy + 6, Size2::Small, colors[0], -1);
        kit::text(fb, "right", sx + 50, sy + 6, Size2::Small, colors[1], -1);
        // What the network thinks, now.
        let (px, py) = (318, 38);
        kit::round_rect(fb, px, py, 314, 120, 8, panel);
        for (i, name) in CLASSES.iter().enumerate() {
            let y = py + 6 + i as i32 * 14;
            kit::text(fb, name, px + 8, y, Size2::Small, if self.probs[i] > 0.5 { kit::WHITE } else { dim }, -1);
            kit::rect(fb, px + 92, y + 2, (self.probs[i] * 170.0) as i32, 9, if i == 0 { dim } else { ink });
            kit::text(fb, &self.count[i].to_string(), px + 306, y, Size2::Small, dim, 1);
        }
        // The latest gesture, big.
        if let Some((g, t)) = self.shown {
            let fade = 1.0 - t as f32 / 90.0;
            let c = kit::blend(bg, kit::WHITE, fade);
            kit::text(fb, CLASSES[g], 158, 172, Size2::Large, c, 0);
        } else {
            kit::text(fb, "swipe, push, wave or tap", 158, 180, Size2::Small, dim, 0);
        }
        // The outputs and where they go.
        for (i, name) in OUTPUTS.iter().enumerate() {
            let y = 212 + i as i32 * 19;
            let sel = i == self.selected;
            kit::text(fb, name, 12, y, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
            kit::round_rect(fb, 100, y + 2, 100, 9, 4, panel);
            kit::round_rect(fb, 100, y + 2, (self.outs[i].clamp(0.0, 1.0) * 100.0) as i32 + 2, 9, 4, ink);
            kit::text(fb, &Patch::label(&self.modbus, self.targets[i].load(Ordering::Relaxed)), 210, y, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
        }
        // Settings.
        for (i, r) in ROWS.iter().enumerate() {
            let y = 208 + i as i32 * 28;
            let sel = i == self.row;
            kit::round_rect(fb, 430, y, 202, 25, 6, if sel { kit::blend(panel, kit::WHITE, 0.15) } else { panel });
            let t = self.targets[self.selected].load(Ordering::Relaxed);
            let (l, v) = match r {
                Row::Voice => ("Own voice", if self.voice_on.load(Ordering::Relaxed) { "on".to_string() } else { "off".into() }),
                Row::Output => ("Output", OUTPUTS[self.selected].to_string()),
                Row::App => ("to app", Patch::app_label(&self.modbus, t)),
                Row::Input => ("to input", Patch::input_label(&self.modbus, t)),
            };
            kit::text(fb, l, 438, y + 6, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
            let v: String = v.chars().take(16).collect();
            kit::text(fb, &v, 626, y + 6, Size2::Small, ink, 1);
        }
        kit::footer(fb, &format!("NPU  {}", self.load.summary()), panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let pad = Pad { chord: Arc::clone(&self.chord), cutoff: Arc::clone(&self.cutoff), vibrato: Arc::clone(&self.vibrato), muted: Arc::clone(&self.muted), shimmer: Arc::clone(&self.shimmer), on: Arc::clone(&self.voice_on), presence: Arc::clone(&self.presence), phase: [0.0, 0.25, 0.5, 0.75], filt: Svf::default(), amp: 0.0, lfo: 0.0 };
        Some(self.sound.processor(None, Some(Box::new(pad))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(Conductor::new(Sound::new(NAME, &modbus, &mixer, &bus), modbus))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_matches_python_bit_for_bit() {
        crate::apps::neural::tests::check_vectors(MODEL, include_str!("../../assets/npu/gesture.test.json"));
    }

    fn bump(t: f32, center: f32, width: f32, h: f32) -> f32 {
        h * (-0.5 * ((t - center) / width).powi(2)).exp()
    }

    /// Plays `frames` of sensor values into the app; returns the gestures
    /// it recognised.
    fn perform(app: &mut Conductor, f: impl Fn(f32) -> [f32; 2], frames: usize) -> Vec<usize> {
        let before = app.count;
        for i in 0..frames {
            app.tick(&Input { hands: f(i as f32), ..Default::default() });
        }
        (0..8).flat_map(|g| std::iter::repeat_n(g, (app.count[g] - before[g]) as usize)).collect()
    }

    #[test]
    fn it_recognises_each_gesture_once() {
        let mut app = Conductor::new(Sound::detached(), Arc::new(ModBus::new()));
        // Swipe right: left sensor, then right.
        assert_eq!(perform(&mut app, |t| [bump(t, 20.0, 4.0, 0.8), bump(t, 28.0, 4.0, 0.8)], 90), vec![1]);
        assert_eq!(perform(&mut app, |t| [bump(t, 28.0, 4.0, 0.7), bump(t, 20.0, 4.0, 0.7)], 90), vec![2]);
        // Tap right.
        assert_eq!(perform(&mut app, |t| [0.0, bump(t, 20.0, 2.5, 0.8)], 90), vec![7]);
        // A wave over the left sensor: three bumps.
        assert_eq!(perform(&mut app, |t| [(0..3).map(|k| bump(t, 15.0 + k as f32 * 10.0, 2.5, 0.8)).sum::<f32>(), 0.0], 100), vec![4]);
    }

    #[test]
    fn holding_still_to_play_is_not_a_gesture() {
        let mut app = Conductor::new(Sound::detached(), Arc::new(ModBus::new()));
        // Hands come in slowly, then hold still while playing.
        let g = perform(&mut app, |t| [(t / 120.0).min(1.0) * 0.6 + 0.02 * (t * 0.2).sin(), (t / 150.0).min(1.0) * 0.3], 420);
        assert!(g.is_empty(), "{g:?}");
        let g = perform(&mut app, |t| [(1.0 - t / 300.0).max(0.0), 0.0], 300);
        assert!(g.is_empty(), "slowly leaving: {g:?}");
    }

    #[test]
    fn a_swipe_moves_its_patched_output() {
        let bus = Arc::new(ModBus::new());
        let target = bus.register("Synth: Cutoff");
        let mut app = Conductor::new(Sound::detached(), Arc::clone(&bus));
        app.targets[2].store(bus.index_of("Synth: Cutoff").unwrap() + 1, Ordering::Relaxed);
        perform(&mut app, |t| [bump(t, 20.0, 4.0, 0.8), bump(t, 28.0, 4.0, 0.8)], 90);
        assert!((target.get() - 0.625).abs() < 1e-6, "{}", target.get());
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }
}
