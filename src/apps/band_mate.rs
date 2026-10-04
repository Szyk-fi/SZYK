//! Band Mate (AI): play chords on a guitar or keyboard into the input and
//! a drummer and bass player follow you.
//!
//! Ten times a second, the last quarter-second of input becomes the
//! energy in each semitone from C2 to B7, and a convolutional network
//! (`assets/npu/chords.pmxn`, trained by `tools/npu/train_chords.py` on
//! synthesized chords in guitar and piano voicings, inversions, sevenths
//! and strums) says which of 24 chords it is -- 12 major, 12 minor -- or
//! that it isn't a chord. A chord has to win twice running to count, so a
//! passing note doesn't move the band.
//!
//! The bass follows the chord's root (and its third and fifth, depending
//! on the style); the drums keep time. Five styles: Rock, Ballad, Funk,
//! Reggae (a one-drop) and Shuffle. Pads: 1 tap tempo, 2 drums on/off,
//! 3 bass on/off, 4 start/stop (or SELECT, or F3). Up/down picks a
//! setting, left/right changes it. With "Start" on "first chord", the
//! band comes in as soon as you play.

use crate::app::{App, Input, SlintExtra};
use crate::apps::ai_input::{Listen, Ring, Tap, Worker};
use crate::apps::hum::ai_header;
use crate::apps::kids_kit::{self as kit, Drum, Ev, Extra, Note, Size2, Song, Sound, Tone};
use crate::apps::neural::{self, Load, Model, Spectrum};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use embedded_graphics::pixelcolor::Rgb565;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Band Mate";
static MODEL: &[u8] = include_bytes!("../../assets/npu/chords.pmxn");
const FRAME: usize = 4096;
const LO: i32 = 36;
const BINS: usize = 72;
const FLOOR: f32 = -9.0;
const NO_CHORD: usize = 24;
/// 100 ms between analyses.
const HOP: u64 = 1600;
const NAMES: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];
const STYLES: [&str; 5] = ["Rock", "Ballad", "Funk", "Reggae", "Shuffle"];

pub fn chord_name(c: usize) -> String {
    match c {
        0..=11 => NAMES[c].to_string(),
        12..=23 => format!("{}m", NAMES[c - 12]),
        _ => "-".into(),
    }
}

/// Energy per semitone, C2 to B7, as `features.chord_features`: each
/// FFT bin's power, spread over the semitones within one of its pitch by
/// a triangle.
pub struct Features {
    spec: Spectrum,
    /// (bin, semitone, weight) for every non-zero weight.
    weights: Vec<(usize, usize, f64)>,
}

impl Features {
    pub fn new() -> Features {
        let mut weights = Vec::new();
        for b in 1..FRAME / 2 + 1 {
            let f = b as f64 * neural::MODEL_SR as f64 / FRAME as f64;
            let semis = 12.0 * (f / 440.0).log2() + 69.0;
            for k in 0..BINS {
                let w = 1.0 - (semis - (LO as usize + k) as f64).abs();
                if w > 0.0 {
                    weights.push((b, k, w));
                }
            }
        }
        Features { spec: Spectrum::new(FRAME, FRAME), weights }
    }

    pub fn compute(&mut self, frame: &[f32], out: &mut [f32]) {
        let mag = self.spec.compute(frame);
        let mut e = [0.0f64; BINS];
        for &(b, k, w) in &self.weights {
            let m = mag[b] as f64;
            e[k] += m * m * w;
        }
        for k in 0..BINS {
            out[k] = (e[k] as f32).sqrt();
        }
        neural::log_relative(&mut out[..BINS], FLOOR);
    }
}

pub struct Shared {
    chord: AtomicUsize,
    probs: Mutex<Vec<f32>>,
    history: Mutex<Vec<usize>>,
    load: Mutex<String>,
    style: AtomicUsize,
    drums: AtomicBool,
    bass: AtomicBool,
    auto_start: AtomicBool,
    heard_any: AtomicBool,
    gate: crate::util::AtomicF32,
    beat: AtomicI32,
}

/// The worker: features, the network, and deciding when the chord changed.
pub struct Listener {
    s: Arc<Shared>,
    model: Model,
    feats: Features,
    x: Vec<f32>,
    p: Vec<f32>,
    frame: Vec<f32>,
    next_end: u64,
    candidate: usize,
    count: u32,
    load: Load,
}

impl Listener {
    fn new(s: Arc<Shared>) -> Listener {
        let model = Model::from_bytes(MODEL).expect("chord model");
        let load = Load::new(model.macs());
        Listener { s, model, feats: Features::new(), x: vec![0.0; BINS], p: vec![0.0; 25], frame: vec![0.0; FRAME], next_end: 0, candidate: NO_CHORD, count: 0, load }
    }

    /// The chord probabilities for one frame.
    pub fn analyse(&mut self, frame: &[f32]) -> &[f32] {
        let t = std::time::Instant::now();
        self.feats.compute(frame, &mut self.x);
        self.model.run(&self.x, &mut self.p);
        neural::softmax(&mut self.p);
        self.load.tick(t.elapsed());
        &self.p
    }

    fn step(&mut self, frame: &[f32]) {
        let rms = (frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32).sqrt();
        let c = if rms < self.s.gate.get() {
            NO_CHORD
        } else {
            let p = self.analyse(frame).to_vec();
            let c = neural::argmax(&p);
            *self.s.probs.lock().unwrap() = p.clone();
            if p[c] < 0.5 {
                NO_CHORD
            } else {
                c
            }
        };
        if c == NO_CHORD {
            self.count = 0;
            return;
        }
        if c == self.candidate {
            self.count += 1;
        } else {
            self.candidate = c;
            self.count = 1;
        }
        if self.count >= 2 && self.s.chord.load(Ordering::Relaxed) != c {
            self.s.chord.store(c, Ordering::Relaxed);
            self.s.heard_any.store(true, Ordering::Relaxed);
            let mut h = self.s.history.lock().unwrap();
            h.push(c);
            if h.len() > 8 {
                h.remove(0);
            }
        }
    }

    pub fn run(&mut self, ring: &Ring) {
        let written = ring.written();
        if self.next_end == 0 || written > self.next_end + 20 * HOP {
            self.next_end = written.max(FRAME as u64);
        }
        while self.next_end <= written {
            if !ring.read_ending_at(self.next_end, &mut self.frame) {
                self.next_end = written + HOP;
                break;
            }
            let f = std::mem::take(&mut self.frame);
            self.step(&f);
            self.frame = f;
            self.next_end += HOP;
        }
        if let Ok(mut l) = self.s.load.lock() {
            *l = self.load.summary();
        };
    }
}

/// The drummer and the bass player.
struct Band {
    s: Arc<Shared>,
}

impl Song for Band {
    fn step(&mut self, step: u64, out: &mut Vec<Ev>) {
        let s = &self.s;
        s.beat.store((step % 16) as i32, Ordering::Relaxed);
        let i = (step % 16) as usize;
        let style = s.style.load(Ordering::Relaxed) % STYLES.len();
        let drums = s.drums.load(Ordering::Relaxed);
        let c = s.chord.load(Ordering::Relaxed);
        let bass_on = s.bass.load(Ordering::Relaxed) && c != NO_CHORD;
        let root = if c < 24 { (c % 12) as i32 } else { 0 };
        let third = if c >= 12 { 3 } else { 4 };
        // E1 to D#2: the bass guitar's lowest octave.
        let bass = 28 + (root - 4).rem_euclid(12);
        let mut d = |dr: Drum, v: f32| {
            if drums {
                out.push(Ev::Drum(dr, v));
            }
        };
        let hit = |p: &str| p.as_bytes()[i] == b'x';
        let (kick, snare, hat, bassline): (&str, &str, &str, &[(usize, i32)]) = match style {
            0 => ("x.......x.x.....", "....x.......x...", "x.x.x.x.x.x.x.x.", &[(0, 0), (2, 0), (4, 0), (6, 0), (8, 0), (10, 0), (12, 0), (14, 0)]),
            1 => ("x.......x.......", "....x.......x...", "x...x...x...x...", &[(0, 0), (8, 7)]),
            2 => ("x..x......x..x..", "....x.......x...", "xxxxxxxxxxxxxxxx", &[(0, 0), (3, 12), (6, 7), (10, 0), (13, 12), (14, 10)]),
            3 => ("........x.......", "........x.......", "..x...x...x...x.", &[(0, 0), (6, 7), (8, 0), (11, 7), (14, 12)]),
            _ => ("x.....x.x.....x.", "....x.......x...", "x..xx..xx..xx..x", &[(0, 0), (4, 100), (8, 7), (12, 9)]),
        };
        if hit(kick) {
            d(Drum::Kick, 0.9);
        }
        if hit(snare) {
            d(if style == 3 { Drum::Rim } else { Drum::Snare }, 0.8);
        }
        if hit(hat) {
            d(Drum::Hat, if i % 4 == 0 { 0.55 } else { 0.35 });
        }
        if bass_on {
            for &(at, iv) in bassline {
                if at == i {
                    // 100 means "the chord's third".
                    let iv = if iv == 100 { third } else { iv };
                    out.push(Ev::Note(Note::new(Tone::Bass, (bass + iv) as f32).vel(0.75).len(0.22)));
                }
            }
        }
    }
}

struct TapOnly(Tap);
impl Extra for TapOnly {
    fn block(&mut self, frames: usize, sr: f32) {
        self.0.process(frames, sr);
    }
    fn frame(&mut self, _sr: f32) -> (f32, f32) {
        (0.0, 0.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Input,
    Style,
    Tempo,
    Start,
    Sensitivity,
}
const ROWS: [Row; 5] = [Row::Input, Row::Style, Row::Tempo, Row::Start, Row::Sensitivity];

pub struct BandMate {
    sound: Sound,
    listen: Listen,
    s: Arc<Shared>,
    listener: Option<Listener>,
    worker: Option<Worker>,
    row: usize,
    prev: [bool; 16],
    taps: Vec<std::time::Instant>,
}

impl BandMate {
    pub fn new(sound: Sound, bus: Arc<AudioBus>) -> BandMate {
        let s = Arc::new(Shared {
            chord: AtomicUsize::new(NO_CHORD),
            probs: Mutex::new(vec![0.0; 25]),
            history: Mutex::new(Vec::new()),
            load: Mutex::new(String::new()),
            style: AtomicUsize::new(0),
            drums: AtomicBool::new(true),
            bass: AtomicBool::new(true),
            auto_start: AtomicBool::new(true),
            heard_any: AtomicBool::new(false),
            gate: crate::util::AtomicF32::new(0.004),
            beat: AtomicI32::new(-1),
        });
        sound.set_tempo(100.0);
        sound.set_reverb(0.12);
        BandMate { sound, listen: Listen::new(bus), listener: Some(Listener::new(Arc::clone(&s))), s, worker: None, row: 0, prev: [false; 16], taps: Vec::new() }
    }

    fn label(r: Row) -> &'static str {
        match r {
            Row::Input => "Input",
            Row::Style => "Style",
            Row::Tempo => "Tempo",
            Row::Start => "Start",
            Row::Sensitivity => "Sensitivity",
        }
    }
    fn value(&self, r: Row) -> String {
        match r {
            Row::Input => self.listen.name(),
            Row::Style => STYLES[self.s.style.load(Ordering::Relaxed) % STYLES.len()].into(),
            Row::Tempo => format!("{:.0} bpm", self.sound.tempo()),
            Row::Start => if self.s.auto_start.load(Ordering::Relaxed) { "on first chord".into() } else { "by hand".into() },
            Row::Sensitivity => format!("{:.0}%", (1.0 - self.s.gate.get() / 0.03).clamp(0.0, 1.0) * 100.0),
        }
    }
}

impl App for BandMate {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        ROWS.iter().map(|r| (BandMate::label(*r).to_string(), self.value(*r), false)).collect()
    }
    fn running(&self) -> Option<bool> {
        Some(self.sound.playing())
    }
    fn toggle_running(&mut self) {
        if self.sound.playing() {
            self.sound.stop();
            // Stopping by hand also stops it coming back in by itself.
            self.s.heard_any.store(false, Ordering::Relaxed);
        } else {
            self.sound.start();
        }
    }
    fn on_exit(&mut self) {
        self.sound.stop();
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        std::array::from_fn(|p| match p {
            0 => PadColor::Yellow,
            1 if self.s.drums.load(Ordering::Relaxed) => PadColor::Green,
            2 if self.s.bass.load(Ordering::Relaxed) => PadColor::Green,
            3 if self.sound.playing() => PadColor::Red,
            1..=3 => PadColor::Blue,
            _ => PadColor::Off,
        })
    }

    fn tick(&mut self, input: &Input) {
        self.listen.tick();
        let prev = self.prev;
        let down = |p: usize| input.grid[p] && !prev[p];
        if down(0) {
            let now = std::time::Instant::now();
            self.taps.retain(|t| now.duration_since(*t).as_secs_f32() < 2.5);
            self.taps.push(now);
            if self.taps.len() >= 3 {
                let span = now.duration_since(self.taps[0]).as_secs_f32() / (self.taps.len() - 1) as f32;
                self.sound.set_tempo(60.0 / span);
            }
            self.sound.drum(Drum::Rim, 0.5);
        }
        if down(1) {
            let v = !self.s.drums.load(Ordering::Relaxed);
            self.s.drums.store(v, Ordering::Relaxed);
        }
        if down(2) {
            let v = !self.s.bass.load(Ordering::Relaxed);
            self.s.bass.store(v, Ordering::Relaxed);
        }
        if down(3) || input.knob1_press {
            self.toggle_running();
        }
        self.prev = input.grid;
        // The band comes in on the first chord.
        if self.s.auto_start.load(Ordering::Relaxed) && self.s.heard_any.load(Ordering::Relaxed) && !self.sound.playing() {
            self.sound.start();
        }
        if input.navigation_steps != 0 {
            self.row = (self.row as i32 + input.navigation_steps).clamp(0, ROWS.len() as i32 - 1) as usize;
        }
        if input.knob2 != 0 {
            let d = input.knob2.signum();
            match ROWS[self.row] {
                Row::Input => self.listen.step(d),
                Row::Style => self.s.style.store((self.s.style.load(Ordering::Relaxed) as i32 + d).rem_euclid(STYLES.len() as i32) as usize, Ordering::Relaxed),
                Row::Tempo => self.sound.set_tempo(self.sound.tempo().round() + d as f32 * 2.0),
                Row::Start => {
                    let v = !self.s.auto_start.load(Ordering::Relaxed);
                    self.s.auto_start.store(v, Ordering::Relaxed);
                }
                Row::Sensitivity => self.s.gate.set((self.s.gate.get() - d as f32 * 0.002).clamp(0.0005, 0.03)),
            }
        }
        // Swing for the shuffle.
        self.sound.set_swing(if self.s.style.load(Ordering::Relaxed) % STYLES.len() == 4 { 0.3 } else { 0.0 });
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = Rgb565::new(5, 3, 3);
        let panel = Rgb565::new(10, 7, 7);
        let dim = Rgb565::new(22, 26, 22);
        let ink = kit::rgb(250, 180, 110);
        kit::clear(fb, bg);
        ai_header(fb, NAME, panel);
        for (i, r) in ROWS.iter().enumerate() {
            let y = 38 + i as i32 * 30;
            let sel = i == self.row;
            kit::round_rect(fb, 8, y, 210, 27, 6, if sel { kit::blend(panel, kit::WHITE, 0.15) } else { panel });
            kit::text(fb, BandMate::label(*r), 16, y + 7, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
            let v: String = self.value(*r).chars().take(17).collect();
            kit::text(fb, &v, 210, y + 7, Size2::Small, ink, 1);
        }
        // Pads.
        let pads = [("tap tempo", true), ("drums", self.s.drums.load(Ordering::Relaxed)), ("bass", self.s.bass.load(Ordering::Relaxed)), (if self.sound.playing() { "stop" } else { "start" }, true)];
        for (k, (l, on)) in pads.iter().enumerate() {
            let y = 196 + k as i32 * 26;
            kit::round_rect(fb, 8, y, 210, 22, 6, if *on { kit::blend(panel, ink, 0.3) } else { panel });
            kit::text(fb, &format!("pad {}: {l}", k + 1), 16, y + 5, Size2::Small, kit::WHITE, -1);
        }
        // The chord, big.
        let c = self.s.chord.load(Ordering::Relaxed);
        kit::round_rect(fb, 228, 38, 404, 140, 12, panel);
        kit::text(fb, &chord_name(c), 330, 60, Size2::Huge, if c == NO_CHORD { dim } else { kit::WHITE }, 0);
        let quality = if c < 12 { "major" } else if c < 24 { "minor" } else if self.listen.connected() { "play a chord" } else { "no input" };
        kit::text(fb, quality, 330, 140, Size2::Small, dim, 0);
        // The network's top guesses.
        let probs = self.s.probs.lock().unwrap().clone();
        let mut order: Vec<usize> = (0..25).collect();
        order.sort_by(|a, b| probs[*b].total_cmp(&probs[*a]));
        let any = probs.iter().any(|&p| p > 0.0);
        for (k, &i) in order.iter().take(if any { 5 } else { 0 }).enumerate() {
            let y = 52 + k as i32 * 22;
            kit::text(fb, &chord_name(i), 450, y, Size2::Small, kit::WHITE, -1);
            kit::rect(fb, 490, y + 2, (probs[i] * 130.0) as i32, 10, ink);
        }
        // What you've played.
        let h = self.s.history.lock().unwrap().clone();
        kit::text(fb, "you played", 236, 190, Size2::Small, dim, -1);
        for (k, &c) in h.iter().enumerate() {
            kit::round_rect(fb, 236 + k as i32 * 49, 206, 45, 30, 6, kit::blend(panel, ink, 0.25));
            kit::text(fb, &chord_name(c), 258 + k as i32 * 49, 213, Size2::Small, kit::WHITE, 0);
        }
        // The beat.
        let beat = self.s.beat.load(Ordering::Relaxed);
        for b in 0..4 {
            let lit = self.sound.playing() && beat >= 0 && beat / 4 == b;
            kit::circle(fb, 260 + b * 30, 266, 9, if lit { ink } else { panel });
        }
        kit::text(fb, if self.sound.playing() { STYLES[self.s.style.load(Ordering::Relaxed) % STYLES.len()] } else { "band waiting" }, 390, 260, Size2::Small, dim, -1);
        let level = self.listen.level.get();
        kit::round_rect(fb, 228, 300, (400.0 * level.min(1.0)) as i32 + 4, 4, 2, ink);
        let load = self.s.load.lock().map(|l| l.clone()).unwrap_or_default();
        kit::footer(fb, &format!("NPU  {}", if load.is_empty() { "waiting for sound".into() } else { load }), panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if let Some(mut l) = self.listener.take() {
            let ring = Arc::clone(&self.listen.ring);
            self.worker = Some(Worker::spawn("chords-npu", move || l.run(&ring)));
        }
        Some(self.sound.processor(Some(Box::new(Band { s: Arc::clone(&self.s) })), Some(Box::new(TapOnly(self.listen.tap())))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<AudioBus> = ctx.get();
    Box::new(BandMate::new(Sound::new(NAME, &modbus, &mixer, &bus), bus))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, render};

    #[test]
    fn the_model_matches_python_bit_for_bit() {
        crate::apps::neural::tests::check_vectors(MODEL, include_str!("../../assets/npu/chords.test.json"));
    }

    #[test]
    fn features_match_python() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../assets/npu/chords.features.json")).unwrap();
        let frame: Vec<f32> = v["frame"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let want: Vec<f32> = v["features"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let mut got = vec![0.0; BINS];
        Features::new().compute(&frame, &mut got);
        for (k, (a, b)) in got.iter().zip(&want).enumerate() {
            assert!((a - b).abs() < 2e-3, "{k}: {a} vs {b}");
        }
    }

    /// A strummed chord: each note a plucked-string-like tone with
    /// harmonics, at 16 kHz.
    fn strum(notes: &[i32], n: usize, sr: f32) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / sr;
                notes
                    .iter()
                    .map(|&m| {
                        let f = 440.0 * 2f32.powf((m as f32 - 69.0) / 12.0);
                        (1..8).map(|h| (t * f * h as f32 * std::f32::consts::TAU).sin() / (h * h) as f32).sum::<f32>() * (-t / 1.5).exp()
                    })
                    .sum::<f32>()
                    * 0.15
            })
            .collect()
    }

    #[test]
    fn it_names_open_guitar_chords() {
        let mut l = Listener::new(BandMate::new(Sound::detached(), Arc::new(AudioBus::new())).s);
        // Open-position guitar shapes: E, Am, C, G, D, Em.
        for (notes, want) in [(&[40, 47, 52, 56, 59, 64][..], 4), (&[45, 52, 57, 60, 64][..], 21), (&[48, 52, 55, 60, 64][..], 0), (&[43, 47, 50, 55, 59, 67][..], 7), (&[50, 57, 62, 66][..], 2), (&[40, 47, 52, 55, 59, 64][..], 16)] {
            let x = strum(notes, 6000, 16_000.0);
            let p = l.analyse(&x[1000..1000 + FRAME]).to_vec();
            assert_eq!(neural::argmax(&p), want, "{notes:?}: heard {} ({:?})", chord_name(neural::argmax(&p)), p[neural::argmax(&p)]);
        }
    }

    #[test]
    fn the_band_follows_the_chords_you_play() {
        let bus = Arc::new(AudioBus::new());
        let input = bus.register("Hardware input");
        let mut app = BandMate::new(Sound::detached(), Arc::clone(&bus));
        let mut p = app.audio_processor().unwrap();
        let am = strum(&[45, 52, 57, 60, 64], 48_000 * 2, 48_000.0);
        let mut out = Vec::new();
        for k in 0..170 {
            let i = (k * 512) % (am.len() - 512);
            *input.lock().unwrap() = am[i..i + 512].to_vec();
            out.extend(render(&mut p, 1));
            app.tick(&Input::default());
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        assert_eq!(chord_name(app.s.chord.load(Ordering::Relaxed)), "Am");
        assert!(app.sound.playing(), "the band came in on its own");
        assert!(energy(&out[out.len() / 2..]) > 1e-5);
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }
}
