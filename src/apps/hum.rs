//! Hum (AI): sing, hum or play into the microphone and any instrument
//! plays your tune.
//!
//! A neural pitch tracker (`assets/npu/hum.pmxn`, trained by
//! `tools/npu/train_hum.py`) listens 100 times a second: each 64 ms frame
//! becomes a log-frequency spectrum, and the network says which of 145
//! pitches (a third of a semitone apart, C2 to C6) it hears, or that it
//! hears no pitch. Reading the probabilities around the peak gives the
//! pitch to a few cents, so the screen doubles as a tuner.
//!
//! Notes mode turns the pitch into notes: snapped to a scale if you like,
//! started when a pitch holds for 30 ms, changed when you move to another
//! note and hold it, ended when the pitch stops. They go wherever "Plays"
//! points: Hum's own sound, or any instrument (Plaits, Voltage, Atlas...).
//! Glide mode follows your voice continuously, vibrato and slides
//! included, with Hum's own glide synth.
//!
//! Controls: up/down picks a setting, left/right changes it. The model
//! runs on its own worker (the NPU's job on the device), never on the
//! audio thread; the bottom line shows how much NPU time it would use.

use crate::app::{App, Input, SlintExtra};
use crate::apps::ai_input::{Listen, Worker};
use crate::apps::kids_kit::{self as kit, Extra, Note, Size2, Sound, Svf, Tone};
use crate::apps::neural::{self, Load, Model, Spectrum};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::note_bus::{NoteBus, NoteOut, NoteRoute};
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Hum";
pub static MODEL: &[u8] = include_bytes!("../../assets/npu/hum.pmxn");
pub const FRAME: usize = 1024;
pub const N_FFT: usize = 2048;
pub const HOP: u64 = 160;
pub const BINS: usize = 216;
const FLOOR: f32 = -9.0;
const LO: f32 = 36.0;
const N_PITCH: usize = 145;
pub const UNVOICED: usize = 145;
/// Frames of history drawn on screen (4 s).
const TRACE: usize = 400;

pub const SCALES: [(&str, &[i32]); 6] = [("chromatic", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]), ("major", &[0, 2, 4, 5, 7, 9, 11]), ("minor", &[0, 2, 3, 5, 7, 8, 10]), ("pentatonic", &[0, 2, 4, 7, 9]), ("minor pent.", &[0, 3, 5, 7, 10]), ("blues", &[0, 3, 5, 6, 7, 10])];
const TONES: [Tone; 6] = [Tone::Flute, Tone::Soft, Tone::Organ, Tone::Chip, Tone::Bass, Tone::Marimba];

/// The log-frequency spectrum the model reads: 216 bins, three per
/// semitone from 40 Hz, relative to the loudest (as `features.hum_features`).
pub fn features(spec: &mut Spectrum, frame: &[f32], out: &mut [f32]) {
    let mag = spec.compute(frame);
    let bin_hz = neural::MODEL_SR / N_FFT as f32;
    for (k, o) in out.iter_mut().enumerate().take(BINS) {
        let hz = 40.0 * 2f32.powf(k as f32 / 36.0);
        *o = neural::mag_at(mag, hz, bin_hz);
    }
    neural::log_relative(&mut out[..BINS], FLOOR);
}

/// The pitch (MIDI, fractional) the probabilities point to, or None.
pub fn decode(p: &[f32]) -> Option<f32> {
    if p[UNVOICED] >= 0.5 {
        return None;
    }
    let best = neural::argmax(&p[..N_PITCH]);
    let (lo, hi) = (best.saturating_sub(2), (best + 3).min(N_PITCH));
    let (mut w, mut s) = (0.0, 0.0);
    for i in lo..hi {
        w += p[i];
        s += p[i] * i as f32;
    }
    Some(LO + s / w.max(1e-9) / 3.0)
}

/// The nearest note of `scale` in `key` to `midi`.
pub fn snap(midi: f32, key: i32, scale: &[i32]) -> i32 {
    let r = midi.round() as i32;
    let mut best = r;
    let mut dist = f32::MAX;
    for n in r - 2..=r + 2 {
        if scale.contains(&(n - key).rem_euclid(12)) {
            let d = (n as f32 - midi).abs();
            if d < dist {
                dist = d;
                best = n;
            }
        }
    }
    best
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoteEv {
    On(i32, u8),
    Off(i32),
}

/// Turns a stream of pitches into notes.
#[derive(Default)]
pub struct Segmenter {
    pub current: Option<i32>,
    candidate: Option<i32>,
    count: u32,
    silent: u32,
}

impl Segmenter {
    /// One frame: `pitch` (already transposed) and the note it snaps to.
    pub fn step(&mut self, pitch: Option<(f32, i32)>, vel: u8, out: &mut Vec<NoteEv>) {
        match pitch {
            None => {
                self.silent += 1;
                self.candidate = None;
                self.count = 0;
                // 50 ms without a pitch ends the note.
                if self.silent >= 5 {
                    if let Some(n) = self.current.take() {
                        out.push(NoteEv::Off(n));
                    }
                }
            }
            Some((raw, note)) => {
                self.silent = 0;
                if self.current == Some(note) {
                    self.candidate = None;
                    self.count = 0;
                    return;
                }
                if self.candidate == Some(note) {
                    self.count += 1;
                } else {
                    self.candidate = Some(note);
                    self.count = 1;
                }
                // A new note needs 30 ms of holding; a change also needs
                // to have really left the old note (hysteresis), so
                // vibrato on one note doesn't flicker between two.
                let far = self.current.map_or(true, |c| (raw - c as f32).abs() > 0.65);
                if self.count >= 3 && far {
                    if let Some(c) = self.current.take() {
                        out.push(NoteEv::Off(c));
                    }
                    out.push(NoteEv::On(note, vel));
                    self.current = Some(note);
                    self.candidate = None;
                    self.count = 0;
                }
            }
        }
    }
}

/// Settings and what the worker found, shared between threads.
pub struct Shared {
    scale: AtomicUsize,
    key: AtomicI32,
    octave: AtomicI32,
    glide: std::sync::atomic::AtomicBool,
    tone: AtomicUsize,
    gate: AtomicF32,
    /// The latest pitch (MIDI), or -1.
    pitch: AtomicF32,
    confidence: AtomicF32,
    note: AtomicI32,
    trace: Mutex<VecDeque<(f32, i32)>>,
    load: Mutex<String>,
    /// For the glide synth: target pitch (MIDI) and loudness.
    glide_pitch: AtomicF32,
    glide_amp: AtomicF32,
}

/// The worker's side: the model and the note logic.
pub struct Tracker {
    model: Model,
    spec: Spectrum,
    frame: Vec<f32>,
    feats: Vec<f32>,
    probs: Vec<f32>,
    next_end: u64,
    seg: Segmenter,
    smooth: VecDeque<f32>,
    load: Load,
    s: Arc<Shared>,
    sound: Sound,
    notes: NoteOut,
    events: Vec<NoteEv>,
}

impl Tracker {
    fn new(s: Arc<Shared>, sound: Sound, notes: NoteOut) -> Tracker {
        let model = Model::from_bytes(MODEL).expect("hum model");
        let load = Load::new(model.macs());
        let n = model.output_len();
        Tracker { model, spec: Spectrum::new(FRAME, N_FFT), frame: vec![0.0; FRAME], feats: vec![0.0; BINS], probs: vec![0.0; n], next_end: 0, seg: Segmenter::default(), smooth: VecDeque::new(), load, s, sound, notes, events: Vec::new() }
    }

    /// Processes every new hop in the ring.
    pub fn run(&mut self, ring: &crate::apps::ai_input::Ring) {
        let written = ring.written();
        if self.next_end == 0 || written > self.next_end + 30 * HOP {
            // Starting, or fell behind: jump to now.
            self.next_end = written.max(FRAME as u64);
        }
        while self.next_end <= written {
            if !ring.read_ending_at(self.next_end, &mut self.frame) {
                self.next_end = written + HOP;
                break;
            }
            let frame = std::mem::take(&mut self.frame);
            self.analyse(&frame);
            self.frame = frame;
            self.next_end += HOP;
        }
    }

    pub fn analyse(&mut self, frame: &[f32]) {
        let s = Arc::clone(&self.s);
        let rms = (frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32).sqrt();
        // The gate saves the NPU (and false notes) when nobody's singing.
        let gate = s.gate.get();
        let pitch = if rms < gate {
            None
        } else {
            let t = std::time::Instant::now();
            features(&mut self.spec, frame, &mut self.feats);
            self.model.run(&self.feats, &mut self.probs);
            neural::softmax(&mut self.probs);
            self.load.tick(t.elapsed());
            s.confidence.set(1.0 - self.probs[UNVOICED]);
            decode(&self.probs)
        };
        // A median of three frames removes single-frame octave slips.
        let pitch = pitch.map(|p| {
            self.smooth.push_back(p);
            if self.smooth.len() > 3 {
                self.smooth.pop_front();
            }
            let mut v: Vec<f32> = self.smooth.iter().copied().collect();
            v.sort_by(|a, b| a.total_cmp(b));
            v[v.len() / 2]
        });
        if pitch.is_none() {
            self.smooth.clear();
            s.confidence.set(0.0);
        }
        s.pitch.set(pitch.unwrap_or(-1.0));
        let transpose = 12 * s.octave.load(Ordering::Relaxed);
        let scale = SCALES[s.scale.load(Ordering::Relaxed) % SCALES.len()].1;
        let key = s.key.load(Ordering::Relaxed);
        let snapped = pitch.map(|p| (p, snap(p, key, scale)));
        let vel = (40.0 + rms.sqrt() * 300.0).clamp(30.0, 127.0) as u8;
        let glide = s.glide.load(Ordering::Relaxed);
        self.events.clear();
        if glide {
            if let Some(c) = self.seg.current.take() {
                self.events.push(NoteEv::Off(c));
            }
        } else {
            self.seg.step(snapped, vel, &mut self.events);
        }
        let tone = TONES[s.tone.load(Ordering::Relaxed) % TONES.len()];
        for &e in &self.events {
            match e {
                NoteEv::On(n, v) => {
                    let n = (n + transpose).clamp(0, 127);
                    if self.notes.internal() {
                        self.sound.play(Note::new(tone, n as f32).vel(v as f32 / 127.0).held(n as u32));
                    } else {
                        self.notes.note_on(n as u8, v);
                    }
                }
                NoteEv::Off(n) => {
                    let n = (n + transpose).clamp(0, 127);
                    self.sound.off(n as u32);
                    self.notes.note_off(n as u8);
                }
            }
        }
        s.note.store(self.seg.current.map_or(-1, |n| n + transpose), Ordering::Relaxed);
        s.glide_pitch.set(pitch.map_or(-1.0, |p| p + transpose as f32));
        s.glide_amp.set(if glide && pitch.is_some() { (rms * 6.0).min(1.0) } else { 0.0 });
        if let Ok(mut t) = s.trace.lock() {
            t.push_back((pitch.unwrap_or(-1.0), self.seg.current.unwrap_or(-1)));
            while t.len() > TRACE {
                t.pop_front();
            }
        };
        if let Ok(mut l) = s.load.lock() {
            *l = self.load.summary();
        };
    }
}

/// Hum's own glide synth plus the input tap, on the audio thread.
struct Voice {
    tap: crate::apps::ai_input::Tap,
    s: Arc<Shared>,
    pitch: f32,
    amp: f32,
    phase: f32,
    filt: Svf,
}

impl Extra for Voice {
    fn block(&mut self, frames: usize, sr: f32) {
        self.tap.process(frames, sr);
    }
    fn frame(&mut self, sr: f32) -> (f32, f32) {
        let target = self.s.glide_pitch.get();
        let amp = self.s.glide_amp.get();
        if target > 0.0 {
            // A 30 ms portamento follows the voice without stepping.
            self.pitch += (target - self.pitch) * (1.0 - (-1.0 / (0.03 * sr)).exp());
        }
        self.amp += (amp - self.amp) * (1.0 - (-1.0 / (0.02 * sr)).exp());
        if self.amp < 1e-4 {
            return (0.0, 0.0);
        }
        let hz = kit::midi_hz(self.pitch);
        let dt = hz / sr;
        let p = self.phase;
        self.phase = (p + dt).fract();
        let raw = kit::saw(p, dt) * 0.5 + (p * std::f32::consts::TAU).sin() * 0.5;
        let y = self.filt.tick(raw, (hz * 5.0).min(8000.0), 0.8, sr).lp * self.amp * 0.5;
        (y, y)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Input,
    Plays,
    Sound,
    Mode,
    Scale,
    Key,
    Octave,
    Gate,
}
const ROWS: [Row; 8] = [Row::Input, Row::Plays, Row::Sound, Row::Mode, Row::Scale, Row::Key, Row::Octave, Row::Gate];

pub struct Hum {
    sound: Sound,
    listen: Listen,
    s: Arc<Shared>,
    route: NoteRoute,
    tracker: Option<Tracker>,
    worker: Option<Worker>,
    row: usize,
}

impl Hum {
    pub fn new(sound: Sound, bus: Arc<AudioBus>, notes: Option<Arc<NoteBus>>) -> Hum {
        let listen = Listen::new(bus);
        let s = Arc::new(Shared {
            scale: AtomicUsize::new(0),
            key: AtomicI32::new(0),
            octave: AtomicI32::new(0),
            glide: std::sync::atomic::AtomicBool::new(false),
            tone: AtomicUsize::new(0),
            gate: AtomicF32::new(0.004),
            pitch: AtomicF32::new(-1.0),
            confidence: AtomicF32::new(0.0),
            note: AtomicI32::new(-1),
            trace: Mutex::new(VecDeque::new()),
            load: Mutex::new(String::new()),
            glide_pitch: AtomicF32::new(-1.0),
            glide_amp: AtomicF32::new(0.0),
        });
        sound.set_reverb(0.15);
        let (route, out) = NoteRoute::new(notes, NAME, "hum", true);
        let tracker = Tracker::new(Arc::clone(&s), sound.clone(), out);
        Hum { sound, listen, s, route, tracker: Some(tracker), worker: None, row: 0 }
    }

    fn value(&self, r: Row) -> String {
        let s = &self.s;
        match r {
            Row::Input => self.listen.name(),
            Row::Plays => self.route.label(),
            Row::Sound => TONES[s.tone.load(Ordering::Relaxed) % TONES.len()].name().into(),
            Row::Mode => if s.glide.load(Ordering::Relaxed) { "glide (follows you)".into() } else { "notes".into() },
            Row::Scale => SCALES[s.scale.load(Ordering::Relaxed) % SCALES.len()].0.into(),
            Row::Key => kit::note_name(s.key.load(Ordering::Relaxed)).into(),
            Row::Octave => format!("{:+}", s.octave.load(Ordering::Relaxed)),
            Row::Gate => format!("{:.0}%", (1.0 - s.gate.get() / 0.03).clamp(0.0, 1.0) * 100.0),
        }
    }

    fn label(r: Row) -> &'static str {
        match r {
            Row::Input => "Input",
            Row::Plays => "Plays",
            Row::Sound => "Sound",
            Row::Mode => "Mode",
            Row::Scale => "Scale",
            Row::Key => "Key",
            Row::Octave => "Octave",
            Row::Gate => "Sensitivity",
        }
    }
}

impl App for Hum {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        ROWS.iter().map(|r| (Hum::label(*r).to_string(), self.value(*r), false)).collect()
    }
    fn needs_background_audio(&self) -> bool {
        // Keeps listening (and playing another app) when you leave.
        self.route.external()
    }

    fn tick(&mut self, input: &Input) {
        self.listen.tick();
        if input.navigation_steps != 0 {
            self.row = (self.row as i32 + input.navigation_steps).clamp(0, ROWS.len() as i32 - 1) as usize;
        }
        if input.knob2 != 0 {
            let d = input.knob2.signum();
            let s = &self.s;
            let bump = |a: &AtomicUsize, n: usize| a.store((a.load(Ordering::Relaxed) as i32 + d).rem_euclid(n as i32) as usize, Ordering::Relaxed);
            match ROWS[self.row] {
                Row::Input => self.listen.step(d),
                Row::Plays => self.route.step(d),
                Row::Sound => bump(&s.tone, TONES.len()),
                Row::Mode => {
                    let g = !s.glide.load(Ordering::Relaxed);
                    s.glide.store(g, Ordering::Relaxed);
                }
                Row::Scale => bump(&s.scale, SCALES.len()),
                Row::Key => s.key.store((s.key.load(Ordering::Relaxed) + d).rem_euclid(12), Ordering::Relaxed),
                Row::Octave => s.octave.store((s.octave.load(Ordering::Relaxed) + d).clamp(-2, 2), Ordering::Relaxed),
                Row::Gate => s.gate.set((s.gate.get() - d as f32 * 0.002).clamp(0.0005, 0.03)),
            }
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = Rgb565::new(2, 5, 6);
        let panel = Rgb565::new(4, 10, 12);
        let ink = kit::rgb(120, 230, 200);
        let dim = Rgb565::new(10, 26, 22);
        kit::clear(fb, bg);
        ai_header(fb, NAME, panel);
        for (i, r) in ROWS.iter().enumerate() {
            let y = 38 + i as i32 * 30;
            let sel = i == self.row;
            kit::round_rect(fb, 8, y, 210, 27, 6, if sel { kit::blend(panel, kit::WHITE, 0.15) } else { panel });
            kit::text(fb, Hum::label(*r), 16, y + 7, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
            let v = self.value(*r);
            let v: String = v.chars().take(18).collect();
            kit::text(fb, &v, 210, y + 7, Size2::Small, ink, 1);
        }

        // The pitch, as a tuner.
        let pitch = self.s.pitch.get();
        let top = 40;
        kit::round_rect(fb, 230, top, 400, 96, 10, panel);
        if pitch > 0.0 {
            let n = pitch.round() as i32;
            let cents = ((pitch - n as f32) * 100.0).round() as i32;
            kit::text(fb, &format!("{}{}", kit::note_name(n), n / 12 - 1), 300, top + 14, Size2::Huge, kit::WHITE, 0);
            kit::text(fb, &format!("{:.1} Hz  {:+} cents", kit::midi_hz(pitch), cents), 500, top + 12, Size2::Small, dim, 0);
            kit::rect(fb, 400, top + 52, 200, 2, dim);
            kit::rect(fb, 499, top + 40, 2, 26, dim);
            let x = 500 + cents.clamp(-50, 50) * 2;
            let c = if cents.abs() < 8 { ink } else { kit::rgb(250, 190, 80) };
            kit::triangle(fb, [(x - 7, top + 36), (x + 7, top + 36), (x, top + 50)], c);
            let conf = self.s.confidence.get();
            kit::text(fb, &format!("sure {:.0}%", conf * 100.0), 500, top + 72, Size2::Small, dim, 0);
        } else {
            kit::text(fb, "-", 300, top + 14, Size2::Huge, dim, 0);
            let msg = if self.listen.connected() { "sing, hum or play" } else { "no input: pick one in Settings" };
            kit::text(fb, msg, 500, top + 40, Size2::Small, dim, 0);
        }

        // The last four seconds: your pitch, and the notes it became.
        let (rx, ry, rw, rh) = (230, 144, 400, 182);
        kit::round_rect(fb, rx, ry, rw, rh, 10, panel);
        let trace: Vec<(f32, i32)> = self.s.trace.lock().map(|t| t.iter().copied().collect()).unwrap_or_default();
        let (lo, hi) = (36.0f32, 84.0f32);
        let y_of = |m: f32| ry + rh - 6 - ((m - lo) / (hi - lo) * (rh - 12) as f32) as i32;
        for oct in 0..5 {
            let m = 36.0 + oct as f32 * 12.0;
            kit::rect(fb, rx + 4, y_of(m), rw - 8, 1, Rgb565::new(6, 14, 14));
            kit::text(fb, &format!("C{}", oct + 2), rx + 6, y_of(m) - 13, Size2::Small, dim, -1);
        }
        let n = trace.len();
        for (i, &(p, note)) in trace.iter().enumerate() {
            let x = rx + rw - 6 - (n - i) as i32;
            if note >= 0 {
                let y = y_of(note as f32);
                kit::rect(fb, x, y - 3, 1, 6, kit::rgb(60, 120, 110));
            }
            if p > 0.0 {
                kit::rect(fb, x, y_of(p) - 1, 1, 3, ink);
            }
        }
        let level = self.listen.level.get();
        kit::round_rect(fb, 230, 330, 400 * level.min(1.0) as i32 + 4, 4, 2, ink);
        let load = self.s.load.lock().map(|l| l.clone()).unwrap_or_default();
        kit::footer(fb, &format!("NPU  {}", if load.is_empty() { "waiting for sound".into() } else { load }), panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        // Start the worker with the processor: the model only runs while
        // the app is listening.
        if let Some(mut t) = self.tracker.take() {
            let ring = Arc::clone(&self.listen.ring);
            self.worker = Some(Worker::spawn("hum-npu", move || t.run(&ring)));
        }
        let voice = Voice { tap: self.listen.tap(), s: Arc::clone(&self.s), pitch: 60.0, amp: 0.0, phase: 0.0, filt: Svf::default() };
        Some(self.sound.processor(None, Some(Box::new(voice))))
    }
}

/// The strip along the top of every AI app.
pub fn ai_header(fb: &mut FrameBuffer, title: &str, bg: Rgb565) {
    kit::rect(fb, 0, 0, 640, 30, bg);
    kit::text(fb, title, 12, 7, Size2::Medium, kit::WHITE, -1);
    let tw = kit::text_width(title, Size2::Medium);
    kit::round_rect(fb, 22 + tw, 7, 70, 16, 8, kit::rgb(70, 60, 140));
    kit::text(fb, "AI  NPU", 57 + tw, 9, Size2::Small, kit::WHITE, 0);
    kit::text(fb, "F1: home", 630, 9, Size2::Small, kit::blend(bg, kit::WHITE, 0.6), 1);
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<AudioBus> = ctx.get();
    Box::new(Hum::new(Sound::new(NAME, &modbus, &mixer, &bus), bus, ctx.try_get()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::render;

    #[test]
    fn the_model_matches_python_bit_for_bit() {
        crate::apps::neural::tests::check_vectors(MODEL, include_str!("../../assets/npu/hum.test.json"));
    }

    #[test]
    fn features_match_python() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../assets/npu/hum.features.json")).unwrap();
        let frame: Vec<f32> = v["frame"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let want: Vec<f32> = v["features"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let mut got = vec![0.0; BINS];
        features(&mut Spectrum::new(FRAME, N_FFT), &frame, &mut got);
        for (k, (a, b)) in got.iter().zip(&want).enumerate() {
            assert!((a - b).abs() < 2e-3, "bin {k}: {a} vs {b}");
        }
    }

    /// A sung vowel: harmonics through two formants, with vibrato.
    fn sing(hz: f32, n: usize, sr: f32, phase: &mut f32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *phase += hz / sr;
                let mut x = 0.0;
                for h in 1..12 {
                    let f = hz * h as f32;
                    let env = 1.0 / (1.0 + ((f - 700.0) / 150.0).powi(2)) + 0.6 / (1.0 + ((f - 1200.0) / 200.0).powi(2)) + 0.05;
                    x += env * (*phase * h as f32 * std::f32::consts::TAU).sin() / h as f32;
                }
                x * 0.2
            })
            .collect()
    }

    fn tracker() -> (Tracker, Arc<Shared>, Sound) {
        let h = Hum::new(Sound::detached(), Arc::new(AudioBus::new()), None);
        let s = Arc::clone(&h.s);
        let sound = h.sound.clone();
        let mut h = h;
        (h.tracker.take().unwrap(), s, sound)
    }

    #[test]
    fn it_hears_the_pitch_of_a_sung_note_to_a_few_cents() {
        let (mut t, s, _) = tracker();
        for &(hz, want) in &[(110.0, 45.0), (220.0, 57.0), (261.63, 60.0), (523.25, 72.0)] {
            let mut ph = 0.0;
            let x = sing(hz, 16_000, 16_000.0, &mut ph);
            // Three frames, so the three-frame median settles on this note.
            for k in 0..3 {
                t.analyse(&x[4000 + k * 160..4000 + k * 160 + FRAME]);
            }
            let p = s.pitch.get();
            assert!((p - want).abs() < 0.15, "{hz} Hz: heard {p}, want {want}");
        }
        // Silence and noise are not notes.
        t.analyse(&vec![0.0; FRAME]);
        assert!(s.pitch.get() < 0.0);
        let mut rng = kit::Rng::new(3);
        let noise: Vec<f32> = (0..FRAME).map(|_| rng.next_u32() as f32 / u32::MAX as f32 - 0.5).collect();
        t.analyse(&noise);
        assert!(s.pitch.get() < 0.0, "noise heard as {}", s.pitch.get());
    }

    #[test]
    fn a_sung_tune_becomes_notes() {
        let (mut t, s, _) = tracker();
        s.scale.store(1, Ordering::Relaxed); // C major
        let ring = crate::apps::ai_input::Ring::default();
        let mut ph = 0.0;
        let mut heard = Vec::new();
        for (hz, frames) in [(261.63, 40), (0.0, 10), (293.66, 40), (329.0, 40)] {
            for _ in 0..frames {
                let x = if hz > 0.0 { sing(hz, HOP as usize, 16_000.0, &mut ph) } else { vec![0.0; HOP as usize] };
                ring.write_blocking(&x);
                t.run(&ring);
                let n = s.note.load(Ordering::Relaxed);
                if heard.last() != Some(&n) {
                    heard.push(n);
                }
            }
        }
        // C, a gap, D, then E (329 Hz is a little flat of E; the scale snaps it).
        assert_eq!(heard.into_iter().filter(|&n| n >= 0).collect::<Vec<_>>(), vec![60, 62, 64]);
    }

    #[test]
    fn the_segmenter_ignores_vibrato_and_blips() {
        let mut seg = Segmenter::default();
        let mut ev = Vec::new();
        for i in 0..40 {
            let wobble = 0.4 * (i as f32 * 0.6).sin();
            seg.step(Some((60.0 + wobble, (60.0 + wobble).round() as i32)), 100, &mut ev);
        }
        assert_eq!(ev, vec![NoteEv::On(60, 100)], "vibrato inside the note doesn't retrigger it");
        seg.step(None, 0, &mut ev);
        seg.step(Some((60.0, 60)), 100, &mut ev);
        assert_eq!(ev.len(), 1, "a one-frame dropout doesn't end the note");
    }

    #[test]
    fn the_whole_app_plays_its_own_sound_from_the_mic() {
        let bus = Arc::new(AudioBus::new());
        let input = bus.register("Hardware input");
        let mut h = Hum::new(Sound::detached(), Arc::clone(&bus), None);
        let mut p = h.audio_processor().unwrap();
        let mut ph = 0.0;
        let mut out = Vec::new();
        for _ in 0..200 {
            *input.lock().unwrap() = sing(220.0, 512, 48_000.0, &mut ph);
            out.extend(render(&mut p, 1));
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        assert_eq!(h.s.note.load(Ordering::Relaxed), 57, "A3");
        let tail = &out[out.len() / 2..];
        assert!(tail.iter().map(|x| x * x).sum::<f32>() / tail.len() as f32 > 1e-5, "its own voice plays the note");
        let mut fb = FrameBuffer::new();
        h.draw(&mut fb);
    }
}
