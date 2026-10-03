//! Mouth Drums (AI): teach Portamax your own sounds, then beatbox and it
//! plays drums.
//!
//! Hold one of the top eight pads and make a sound into the mic -- a "b",
//! a "pf", a "ts", a click, a clap -- a few times. Let go: that pad has
//! learned it. Do the same for others. From then on, every sound you make
//! is matched to the nearest one you taught and plays that pad's drum.
//!
//! How: an onset detector finds the start of each sound; its first 80 ms
//! become a 32-band mel spectrogram, and a small convolutional network
//! (`assets/npu/mouth.pmxn`, trained by `tools/npu/train_mouth.py` to tell
//! apart 32 families of mouth and body percussion) turns it into 64
//! numbers that describe the sound. A pad's examples are averaged into a
//! prototype; a new sound goes to the prototype it's most similar to
//! (cosine similarity), or nowhere if none is close enough ("Strictness").
//! Teaching is just averaging, so it's instant and happens on the device.
//!
//! The sound is heard about 80 ms after it starts, since the network
//! needs that much of it. The loop recorder places hits where you made
//! them, not where they were heard.
//!
//! Pads: 1-8 the sounds (hold to teach, tap to play), 9 record, 10 play,
//! 11 clear the loop, 12 click. Up/down picks a setting, left/right
//! changes it.

use crate::app::{App, Input, SlintExtra};
use crate::apps::ai_input::{Listen, Ring, Worker};
use crate::apps::hum::ai_header;
use crate::apps::kids_kit::{self as kit, Drum, Ev, Extra, Size2, Song, Sound};
use crate::apps::neural::{Load, Model, Spectrum};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Mouth Drums";
static MODEL: &[u8] = include_bytes!("../../assets/npu/mouth.pmxn");
const SR: f32 = 16_000.0;
const FRAME: usize = 256;
const HOP: usize = 96;
const FRAMES: usize = 12;
const MELS: usize = 32;
const PRE: usize = 2;
const FLOOR: f32 = -10.0;
const CLIP: usize = FRAME + HOP * (FRAMES - 1);
const EMBED: usize = 64;
const SOUNDS: usize = 8;
const STEPS: usize = 32;
const DRUMS: [Drum; 11] = [Drum::Kick, Drum::Snare, Drum::Hat, Drum::OpenHat, Drum::Clap, Drum::Rim, Drum::Cowbell, Drum::Woodblock, Drum::HiTom, Drum::LowTom, Drum::Shaker];
const DEFAULT_DRUMS: [usize; SOUNDS] = [0, 1, 2, 3, 4, 5, 6, 7];
const COLORS: [(u8, u8, u8); SOUNDS] = [(240, 90, 80), (250, 170, 60), (240, 220, 80), (130, 220, 110), (80, 210, 200), (90, 150, 240), (160, 120, 240), (230, 120, 200)];

/// Triangular mel filters, as `features.mel_filters`.
fn mel_filters() -> Vec<Vec<f32>> {
    let hz_to_mel = |f: f64| 2595.0 * (1.0 + f / 700.0).log10();
    let mel_to_hz = |m: f64| 700.0 * (10f64.powf(m / 2595.0) - 1.0);
    let (lo, hi) = (hz_to_mel(60.0), hz_to_mel(7600.0));
    let pts: Vec<f64> = (0..MELS + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (MELS + 1) as f64)).collect();
    (0..MELS)
        .map(|m| {
            let (a, c, b) = (pts[m], pts[m + 1], pts[m + 2]);
            (0..FRAME / 2 + 1)
                .map(|k| {
                    let f = k as f64 * SR as f64 / FRAME as f64;
                    ((f - a) / (c - a)).min((b - f) / (b - c)).max(0.0) as f32
                })
                .collect()
        })
        .collect()
}

/// The model's input: [32 mels, 12 frames], channel-major, as
/// `features.mouth_features`.
pub struct Features {
    spec: Spectrum,
    fb: Vec<Vec<f32>>,
}

impl Features {
    pub fn new() -> Features {
        Features { spec: Spectrum::new(FRAME, FRAME), fb: mel_filters() }
    }
    pub fn compute(&mut self, clip: &[f32], out: &mut [f32]) {
        for t in 0..FRAMES {
            let mag = self.spec.compute(&clip[t * HOP..t * HOP + FRAME]);
            for m in 0..MELS {
                out[m * FRAMES + t] = self.fb[m].iter().zip(mag.iter()).map(|(w, x)| w * x).sum();
            }
        }
        let peak = out[..MELS * FRAMES].iter().cloned().fold(0.0f32, f32::max).max(1e-9);
        for v in out[..MELS * FRAMES].iter_mut() {
            *v = ((*v / peak).max(1e-9)).ln().max(FLOOR);
        }
    }
}

#[derive(Clone)]
struct Proto {
    sum: Vec<f32>,
    count: u32,
}

impl Proto {
    fn unit(&self) -> Vec<f32> {
        let n = self.sum.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        self.sum.iter().map(|x| x / n).collect()
    }
}

pub struct Shared {
    protos: Mutex<Vec<Proto>>,
    drums: Mutex<[usize; SOUNDS]>,
    /// The pad being taught, or -1.
    teaching: AtomicI32,
    /// Bumped on each hit, per pad, for the screen.
    hits: [AtomicU32; SOUNDS],
    /// The last sound: which pad (-1 none) and how similar.
    last: Mutex<(i32, f32)>,
    strict: AtomicF32,
    gate: AtomicF32,
    recording: AtomicBool,
    pattern: Mutex<[[bool; STEPS]; SOUNDS]>,
    load: Mutex<String>,
    click: AtomicBool,
}

/// The worker: onsets, features, the embedding, and matching.
pub struct Listener {
    s: Arc<Shared>,
    sound: Sound,
    model: Model,
    feats: Features,
    x: Vec<f32>,
    emb: Vec<f32>,
    clip: Vec<f32>,
    hop: Vec<f32>,
    /// Next hop to examine (its first sample).
    next: u64,
    history: [f32; 8],
    last_onset: u64,
    pending: Option<u64>,
    load: Load,
}

impl Listener {
    fn new(s: Arc<Shared>, sound: Sound) -> Listener {
        let model = Model::from_bytes(MODEL).expect("mouth model");
        let load = Load::new(model.macs());
        Listener { s, sound, model, feats: Features::new(), x: vec![0.0; MELS * FRAMES], emb: vec![0.0; EMBED], clip: vec![0.0; CLIP], hop: vec![0.0; HOP], next: 0, history: [0.0; 8], last_onset: 0, pending: None, load }
    }

    /// The embedding of a clip that starts `PRE` hops before an onset.
    pub fn embed(&mut self, clip: &[f32]) -> Vec<f32> {
        let t = std::time::Instant::now();
        self.feats.compute(clip, &mut self.x);
        self.model.run(&self.x, &mut self.emb);
        self.load.tick(t.elapsed());
        let n = self.emb.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        self.emb.iter().map(|x| x / n).collect()
    }

    pub fn run(&mut self, ring: &Ring) {
        let written = ring.written();
        if self.next == 0 || written > self.next + 16_000 {
            self.next = written.saturating_sub(HOP as u64);
        }
        // Onsets: a hop much louder than the hops before it.
        while self.next + HOP as u64 <= written {
            if !ring.read_ending_at(self.next + HOP as u64, &mut self.hop) {
                self.next = written;
                break;
            }
            let e = self.hop.iter().map(|x| x * x).sum::<f32>() / HOP as f32;
            let avg = self.history.iter().sum::<f32>() / 8.0;
            let gate = self.s.gate.get();
            let apart = self.next >= self.last_onset + (0.07 * SR) as u64;
            if e > gate * gate && e > avg * 6.0 && apart && self.pending.is_none() {
                self.pending = Some(self.next);
                self.last_onset = self.next;
            }
            self.history.rotate_left(1);
            self.history[7] = e;
            self.next += HOP as u64;
        }
        // A pending sound is classified once its whole clip has arrived.
        if let Some(onset) = self.pending {
            let start = onset.saturating_sub((PRE * HOP) as u64);
            if written >= start + CLIP as u64 {
                self.pending = None;
                if ring.read_ending_at(start + CLIP as u64, &mut self.clip) {
                    let clip = std::mem::take(&mut self.clip);
                    let e = self.embed(&clip);
                    self.clip = clip;
                    self.heard(&e, (written - onset) as f32 / SR);
                }
            }
        }
        if let Ok(mut l) = self.s.load.lock() {
            *l = self.load.summary();
        };
    }

    /// What to do with a sound: learn it, or match and play it.
    fn heard(&mut self, e: &[f32], latency_s: f32) {
        let s = Arc::clone(&self.s);
        let teaching = s.teaching.load(Ordering::Relaxed);
        if teaching >= 0 {
            let mut p = s.protos.lock().unwrap();
            let pr = &mut p[teaching as usize];
            for (a, b) in pr.sum.iter_mut().zip(e) {
                *a += b;
            }
            pr.count += 1;
            drop(p);
            s.hits[teaching as usize].fetch_add(1, Ordering::Relaxed);
            *s.last.lock().unwrap() = (teaching, 1.0);
            return;
        }
        let (best, sim) = classify(&s.protos.lock().unwrap(), e);
        *s.last.lock().unwrap() = (if sim >= s.strict.get() { best as i32 } else { -1 }, sim);
        if best == usize::MAX || sim < s.strict.get() {
            return;
        }
        let drum = DRUMS[s.drums.lock().unwrap()[best] % DRUMS.len()];
        self.sound.drum(drum, 0.95);
        s.hits[best].fetch_add(1, Ordering::Relaxed);
        if s.recording.load(Ordering::Relaxed) {
            if let Some(step) = self.sound.step() {
                let step_s = 60.0 / self.sound.tempo() / 4.0;
                let back = (latency_s / step_s).round() as i64;
                let at = (step as i64 - back).rem_euclid(STEPS as i64) as usize;
                s.pattern.lock().unwrap()[best][at] = true;
            }
        }
    }
}

/// The nearest taught sound and its cosine similarity.
fn classify(protos: &[Proto], e: &[f32]) -> (usize, f32) {
    let mut best = (usize::MAX, -1.0f32);
    for (i, p) in protos.iter().enumerate() {
        if p.count == 0 {
            continue;
        }
        let u = p.unit();
        let sim: f32 = u.iter().zip(e).map(|(a, b)| a * b).sum();
        if sim > best.1 {
            best = (i, sim);
        }
    }
    best
}

struct Loop {
    s: Arc<Shared>,
}

impl Song for Loop {
    fn step(&mut self, step: u64, out: &mut Vec<Ev>) {
        let at = (step % STEPS as u64) as usize;
        if self.s.click.load(Ordering::Relaxed) && at % 4 == 0 {
            out.push(Ev::Drum(Drum::Rim, if at % 16 == 0 { 0.5 } else { 0.25 }));
        }
        let (Ok(p), Ok(d)) = (self.s.pattern.try_lock(), self.s.drums.try_lock()) else { return };
        for k in 0..SOUNDS {
            if p[k][at] {
                out.push(Ev::Drum(DRUMS[d[k] % DRUMS.len()], 0.85));
            }
        }
    }
}

struct TapOnly(crate::apps::ai_input::Tap);
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
    Pad,
    Drum,
    Forget,
    Strict,
    Tempo,
}
const ROWS: [Row; 6] = [Row::Input, Row::Pad, Row::Drum, Row::Forget, Row::Strict, Row::Tempo];

pub struct MouthDrums {
    sound: Sound,
    listen: Listen,
    s: Arc<Shared>,
    listener: Option<Listener>,
    worker: Option<Worker>,
    row: usize,
    pad: usize,
    prev: [bool; 16],
    /// Frames each sound pad has been held (a long hold teaches).
    held: [u32; SOUNDS],
    seen_hits: [u32; SOUNDS],
    flash: [f32; SOUNDS],
}

impl MouthDrums {
    pub fn new(sound: Sound, bus: Arc<AudioBus>) -> MouthDrums {
        let s = Arc::new(Shared {
            protos: Mutex::new(vec![Proto { sum: vec![0.0; EMBED], count: 0 }; SOUNDS]),
            drums: Mutex::new(DEFAULT_DRUMS),
            teaching: AtomicI32::new(-1),
            hits: std::array::from_fn(|_| AtomicU32::new(0)),
            last: Mutex::new((-1, 0.0)),
            strict: AtomicF32::new(0.75),
            gate: AtomicF32::new(0.01),
            recording: AtomicBool::new(false),
            pattern: Mutex::new([[false; STEPS]; SOUNDS]),
            load: Mutex::new(String::new()),
            click: AtomicBool::new(true),
        });
        sound.set_tempo(96.0);
        sound.set_reverb(0.08);
        let listener = Listener::new(Arc::clone(&s), sound.clone());
        MouthDrums { sound, listen: Listen::new(bus), s, listener: Some(listener), worker: None, row: 0, pad: 0, prev: [false; 16], held: [0; SOUNDS], seen_hits: [0; SOUNDS], flash: [0.0; SOUNDS] }
    }

    fn drum_of(&self, pad: usize) -> Drum {
        DRUMS[self.s.drums.lock().unwrap()[pad] % DRUMS.len()]
    }

    fn label(r: Row) -> &'static str {
        match r {
            Row::Input => "Input",
            Row::Pad => "Pad",
            Row::Drum => "Plays",
            Row::Forget => "Forget pad",
            Row::Strict => "Strictness",
            Row::Tempo => "Tempo",
        }
    }

    fn value(&self, r: Row) -> String {
        match r {
            Row::Input => self.listen.name(),
            Row::Pad => format!("{}", self.pad + 1),
            Row::Drum => self.drum_of(self.pad).name().into(),
            Row::Forget => format!("< > ({} examples)", self.s.protos.lock().unwrap()[self.pad].count),
            Row::Strict => format!("{:.0}%", self.s.strict.get() * 100.0),
            Row::Tempo => format!("{:.0} bpm", self.sound.tempo()),
        }
    }
}

impl App for MouthDrums {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        ROWS.iter().map(|r| (MouthDrums::label(*r).to_string(), self.value(*r), false)).collect()
    }
    fn running(&self) -> Option<bool> {
        Some(self.sound.playing())
    }
    fn toggle_running(&mut self) {
        if self.sound.playing() {
            self.sound.stop();
            self.s.recording.store(false, Ordering::Relaxed);
        } else {
            self.sound.start();
        }
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let protos = self.s.protos.lock().unwrap();
        std::array::from_fn(|p| match p {
            0..=7 if protos[p].count > 0 => PadColor::Green,
            0..=7 => PadColor::Off,
            8 if self.s.recording.load(Ordering::Relaxed) => PadColor::Red,
            8 => PadColor::Yellow,
            9 if self.sound.playing() => PadColor::Green,
            9..=11 => PadColor::Blue,
            _ => PadColor::Off,
        })
    }

    fn tick(&mut self, input: &Input) {
        self.listen.tick();
        // Sound pads: a tap plays, a hold (over a quarter second) teaches.
        let mut teaching = -1;
        for p in 0..SOUNDS {
            if input.grid[p] {
                self.held[p] += 1;
                if self.held[p] == 1 {
                    self.pad = p;
                    self.sound.drum(self.drum_of(p), 0.8);
                }
                if self.held[p] > 15 {
                    teaching = p as i32;
                }
            } else {
                self.held[p] = 0;
            }
        }
        self.s.teaching.store(teaching, Ordering::Relaxed);
        let prev = self.prev;
        let down = |p: usize| input.grid[p] && !prev[p];
        if down(8) {
            let r = !self.s.recording.load(Ordering::Relaxed);
            self.s.recording.store(r, Ordering::Relaxed);
            if r && !self.sound.playing() {
                self.sound.start();
            }
        }
        if down(9) {
            self.toggle_running();
        }
        if down(10) {
            *self.s.pattern.lock().unwrap() = [[false; STEPS]; SOUNDS];
        }
        if down(11) {
            let c = !self.s.click.load(Ordering::Relaxed);
            self.s.click.store(c, Ordering::Relaxed);
        }
        self.prev = input.grid;
        if input.navigation_steps != 0 {
            self.row = (self.row as i32 + input.navigation_steps).clamp(0, ROWS.len() as i32 - 1) as usize;
        }
        if input.knob2 != 0 {
            let d = input.knob2.signum();
            match ROWS[self.row] {
                Row::Input => self.listen.step(d),
                Row::Pad => self.pad = (self.pad as i32 + d).rem_euclid(SOUNDS as i32) as usize,
                Row::Drum => {
                    let mut dr = self.s.drums.lock().unwrap();
                    dr[self.pad] = (dr[self.pad] as i32 + d).rem_euclid(DRUMS.len() as i32) as usize;
                    drop(dr);
                    self.sound.drum(self.drum_of(self.pad), 0.8);
                }
                Row::Forget => self.s.protos.lock().unwrap()[self.pad] = Proto { sum: vec![0.0; EMBED], count: 0 },
                Row::Strict => self.s.strict.set((self.s.strict.get() + d as f32 * 0.05).clamp(0.3, 0.98)),
                Row::Tempo => self.sound.set_tempo(self.sound.tempo() + d as f32 * 2.0),
            }
        }
        if input.knob1_press {
            self.toggle_running();
        }
        for k in 0..SOUNDS {
            let h = self.s.hits[k].load(Ordering::Relaxed);
            if h != self.seen_hits[k] {
                self.seen_hits[k] = h;
                self.flash[k] = 1.0;
            }
            self.flash[k] = (self.flash[k] - 0.06).max(0.0);
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = Rgb565::new(3, 4, 7);
        let panel = Rgb565::new(6, 9, 14);
        let dim = Rgb565::new(12, 24, 22);
        kit::clear(fb, bg);
        ai_header(fb, NAME, panel);
        let protos = self.s.protos.lock().unwrap().clone();
        let teaching = self.s.teaching.load(Ordering::Relaxed);
        let pattern = *self.s.pattern.lock().unwrap();
        let now = self.sound.step().map(|s| (s % STEPS as u64) as usize);
        // The eight sounds, as the pads.
        for p in 0..SOUNDS {
            let x = 236 + (p as i32 % 4) * 100;
            let y = 38 + (p as i32 / 4) * 70;
            let (r, g, b) = COLORS[p];
            let c = kit::rgb(r, g, b);
            let learned = protos[p].count > 0;
            let fill = if teaching == p as i32 { kit::blend(c, kit::WHITE, 0.5) } else if learned { kit::blend(c, kit::WHITE, self.flash[p] * 0.7) } else { kit::blend(c, bg, 0.75) };
            kit::round_rect(fb, x, y, 92, 62, 10, fill);
            if p == self.pad {
                kit::outline(fb, x - 3, y - 3, 98, 68, 12, 2, kit::WHITE);
            }
            let ink = if learned || teaching == p as i32 { kit::BLACK } else { dim };
            kit::text(fb, self.drum_of(p).name(), x + 46, y + 10, Size2::Small, ink, 0);
            let state = if teaching == p as i32 { format!("learning {}", protos[p].count) } else if learned { format!("{} heard", protos[p].count) } else { "hold to teach".into() };
            kit::text(fb, &state, x + 46, y + 38, Size2::Small, ink, 0);
        }
        // Settings.
        for (i, r) in ROWS.iter().enumerate() {
            let y = 38 + i as i32 * 30;
            let sel = i == self.row;
            kit::round_rect(fb, 8, y, 216, 27, 6, if sel { kit::blend(panel, kit::WHITE, 0.15) } else { panel });
            kit::text(fb, MouthDrums::label(*r), 16, y + 7, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
            let v: String = self.value(*r).chars().take(18).collect();
            kit::text(fb, &v, 216, y + 7, Size2::Small, kit::rgb(160, 200, 250), 1);
        }
        let (last, sim) = *self.s.last.lock().unwrap();
        let heard = match last {
            -1 if sim > 0.0 => format!("last sound: not close to any ({:.0}%)", sim * 100.0),
            -1 => "make a sound".to_string(),
            k => format!("last sound: pad {} ({:.0}% alike)", k + 1, sim * 100.0),
        };
        kit::text(fb, &heard, 12, 228, Size2::Small, kit::WHITE, -1);
        kit::paragraph(fb, "Hold a pad and make a sound 3-5 times to teach it. Then beatbox!", 12, 246, 210, Size2::Small, dim);
        // The loop: 8 rows of 32 steps.
        let (lx, ly) = (236, 186);
        let rec = self.s.recording.load(Ordering::Relaxed);
        kit::text(fb, if rec { "LOOP  recording (pad 9)" } else { "LOOP  pad 9 rec  10 play  11 clear  12 click" }, lx, ly - 4, Size2::Small, if rec { kit::rgb(250, 90, 90) } else { dim }, -1);
        for k in 0..SOUNDS {
            for st in 0..STEPS {
                let x = lx + st as i32 * 12 + (st as i32 / 4) * 1;
                let y = ly + 10 + k as i32 * 15;
                let (r, g, b) = COLORS[k];
                let on = pattern[k][st];
                let c = if on { kit::rgb(r, g, b) } else if now == Some(st) { Rgb565::new(10, 20, 20) } else { panel };
                kit::rect(fb, x, y, 10, 12, c);
            }
        }
        let load = self.s.load.lock().map(|l| l.clone()).unwrap_or_default();
        kit::footer(fb, &format!("NPU  {}", if load.is_empty() { "waiting for a sound".into() } else { load }), panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if let Some(mut l) = self.listener.take() {
            let ring = Arc::clone(&self.listen.ring);
            self.worker = Some(Worker::spawn("mouth-npu", move || l.run(&ring)));
        }
        Some(self.sound.processor(Some(Box::new(Loop { s: Arc::clone(&self.s) })), Some(Box::new(TapOnly(self.listen.tap())))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<AudioBus> = ctx.get();
    Box::new(MouthDrums::new(Sound::new(NAME, &modbus, &mixer, &bus), bus))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_matches_python_bit_for_bit() {
        crate::apps::neural::tests::check_vectors(MODEL, include_str!("../../assets/npu/mouth.test.json"));
    }

    #[test]
    fn features_match_python() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../assets/npu/mouth.features.json")).unwrap();
        let clip: Vec<f32> = v["clip"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let want: Vec<f32> = v["features"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let mut got = vec![0.0; MELS * FRAMES];
        Features::new().compute(&clip, &mut got);
        for (k, (a, b)) in got.iter().zip(&want).enumerate() {
            assert!((a - b).abs() < 2e-3, "{k}: {a} vs {b}");
        }
    }

    /// Mouth sounds, made up here: a "b" (a falling low thump), a "ts"
    /// (a short high hiss) and a "k" (a mid click) -- each a little
    /// different every time, like a person's.
    fn sound(kind: usize, rng: &mut kit::Rng) -> Vec<f32> {
        let mut out = vec![0.0f32; 4000];
        let mut ph = 0.0f32;
        let mut lp = 0.0f32;
        let mut hp_prev = 0.0f32;
        let vary = 0.8 + 0.4 * rng.next_u32() as f32 / u32::MAX as f32;
        for (i, o) in out.iter_mut().enumerate() {
            let t = i as f32 / SR;
            let n = rng.next_u32() as f32 / u32::MAX as f32 * 2.0 - 1.0;
            *o = match kind {
                0 => {
                    ph += (55.0 + 120.0 * (-t / (0.02 * vary)).exp()) / SR;
                    (ph * std::f32::consts::TAU).sin() * (-t / (0.12 * vary)).exp()
                }
                1 => {
                    let h = n - hp_prev;
                    hp_prev = n;
                    h * (-t / (0.03 * vary)).exp() * 0.5
                }
                _ => {
                    lp += (n - lp) * 0.5;
                    ph += 1800.0 * vary / SR;
                    ((ph * std::f32::consts::TAU).sin() * 0.5 + lp) * (-t / 0.012).exp()
                }
            };
        }
        out
    }

    #[test]
    fn three_taught_sounds_are_told_apart() {
        let s = MouthDrums::new(Sound::detached(), Arc::new(AudioBus::new()));
        let mut l = s.listener.unwrap();
        let mut rng = kit::Rng::new(7);
        let clip = |snd: Vec<f32>| {
            let mut c = vec![0.0f32; CLIP];
            for (i, v) in snd.iter().enumerate() {
                if PRE * HOP + i < CLIP {
                    c[PRE * HOP + i] = *v * 0.5;
                }
            }
            c
        };
        // Teach: three examples each.
        for kind in 0..3 {
            l.s.teaching.store(kind as i32, Ordering::Relaxed);
            for _ in 0..3 {
                let e = l.embed(&clip(sound(kind, &mut rng)));
                l.heard(&e, 0.08);
            }
        }
        l.s.teaching.store(-1, Ordering::Relaxed);
        // Then each new one goes to the right pad.
        let mut right = 0;
        for trial in 0..30 {
            let kind = trial % 3;
            let e = l.embed(&clip(sound(kind, &mut rng)));
            let (best, sim) = classify(&l.s.protos.lock().unwrap(), &e);
            if best == kind && sim > 0.75 {
                right += 1;
            }
        }
        assert!(right >= 28, "{right} of 30");
    }

    #[test]
    fn a_sound_in_the_mic_plays_its_drum_and_records_it() {
        let bus = Arc::new(AudioBus::new());
        let input = bus.register("Hardware input");
        let mut app = MouthDrums::new(Sound::detached(), Arc::clone(&bus));
        let mut l = app.listener.take().unwrap();
        // Teach pad 1 a "b" directly.
        let mut rng = kit::Rng::new(1);
        app.s.teaching.store(0, Ordering::Relaxed);
        for _ in 0..3 {
            let mut c = vec![0.0f32; CLIP];
            for (i, v) in sound(0, &mut rng).iter().enumerate().take(CLIP - PRE * HOP) {
                c[PRE * HOP + i] = *v * 0.5;
            }
            let e = l.embed(&c);
            l.heard(&e, 0.08);
        }
        app.s.teaching.store(-1, Ordering::Relaxed);
        app.listener = Some(l);
        let mut p = app.audio_processor().unwrap();
        app.s.recording.store(true, Ordering::Relaxed);
        app.sound.start();
        // Silence, then a "b" at 48 kHz.
        let b: Vec<f32> = sound(0, &mut rng).iter().flat_map(|&v| [v * 0.5, v * 0.5, v * 0.5]).collect();
        let mut fed = 0;
        let mut buf = vec![0.0f32; 1024];
        for k in 0..120 {
            let block: Vec<f32> = if (40..40 + b.len() / 512).contains(&k) {
                let s = &b[fed..fed + 512];
                fed += 512;
                s.to_vec()
            } else {
                vec![0.0; 512]
            };
            *input.lock().unwrap() = block;
            buf.fill(0.0);
            p.process(&mut buf, 2, 48_000.0);
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(app.s.last.lock().unwrap().0, 0, "heard as pad 1: {:?}", app.s.last.lock().unwrap());
        assert!(app.s.hits[0].load(Ordering::Relaxed) >= 4, "played");
        assert!(app.s.pattern.lock().unwrap()[0].iter().any(|&x| x), "recorded into the loop");
        app.tick(&Input::default());
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }
}
