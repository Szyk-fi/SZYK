//! Choir (AI): a vocal tuner and harmoniser.
//!
//! Sing into the input. The lead voice is pulled to the nearest note of
//! the key -- gently, or at "speed 0" all the way at once, the hard-tuned
//! sound -- and up to two harmony voices sing with it: a third above, a
//! fifth below, whatever the pads pick, always in the key. Or set Harmony
//! to "keys" and the harmony sings the notes you hold on the keyboard
//! (or that a sequencer sends Choir), a vocoder-style choir made from
//! your actual voice.
//!
//! Two pieces:
//!
//! * **Hearing the pitch** is Hum's neural pitch tracker
//!   (`assets/npu/hum.pmxn`): a 64 ms log-frequency spectrum 100 times a
//!   second, run on the NPU worker, never the audio thread. Its
//!   probabilities give the pitch to a few cents and say when there is
//!   no pitch at all (a consonant, a breath), which is exactly when a
//!   tuner must leave the voice alone.
//!
//! * **Moving the pitch** is TD-PSOLA (pitch-synchronous overlap-add):
//!   the input is cut into Hann-windowed grains two periods long, taken a
//!   period apart, and laid back down at the *new* period's spacing.
//!   Because each grain keeps the waveshape of one glottal pulse, the
//!   vocal tract's resonances -- the formants that make it sound like
//!   you -- stay where they were: a harmony a sixth up sounds like a
//!   second singer, not a sped-up tape. (Grain centres here are spaced by
//!   the detected period rather than aligned to the exact glottal
//!   closures the textbook version finds; on a steady voice the two are
//!   the same, on a rough one this is a little grainier.)
//!
//! Every voice -- the corrected lead, the harmonies, and the dry voice if
//! you mix it in -- comes out through the same 33 ms delay, so they stay
//! in time with each other. (That's PSOLA's price: it must have two of
//! the lowest voice's periods recorded before it can place a grain.)
//!
//! Large shifts (beyond about a fifth) get a slight buzz: PSOLA keeps the
//! formants by repeating or skipping whole periods, and far from the sung
//! pitch that repetition starts to be heard.

use crate::app::{App, Input, SlintExtra};
use crate::apps::ai_input::{Listen, Ring, Worker};
use crate::apps::hum;
use crate::apps::kids_kit::{self as kit, Extra, Size2, Sound};
use crate::apps::neural::{self, Load, Model, Spectrum};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::note_bus::{NoteBus, NoteInboxRef, NoteView};
use crate::util::AtomicF32;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicI32, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Choir";
/// The longest period PSOLA handles: 80 Hz (low male voice).
const MAX_PERIOD: f32 = 600.0;
/// Grain centres are taken this far behind the output. A grain is laid
/// down when its left edge reaches the output, so its centre is a period
/// ahead; its source grain may sit half a period later than ideal (grains
/// step whole periods); and that grain reaches a period further on. All
/// of it must already be recorded: 2.5 of the longest periods, about
/// 33 ms -- the latency of every voice, dry included.
const DELAY: f64 = 2.5 * MAX_PERIOD as f64 + 64.0;
const RING: usize = 8192;
const OLA: usize = 4096;
/// Frames of pitch history on screen (3 s).
const TRACE: usize = 300;

/// Harmony intervals: (label, steps along the scale; 7 = an octave).
pub const INTERVALS: [(&str, i32); 8] = [("off", 0), ("3rd up", 2), ("5th up", 4), ("6th up", 5), ("8ve up", 7), ("3rd down", -2), ("4th down", -3), ("8ve down", -7)];
const HARMONY_MODES: [&str; 3] = ["off", "in key", "keys"];
const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// The harmony note `steps` scale steps from `note` (which should be in
/// the scale), in `key`/`scale`. Steps count a 7-note scale; in other
/// scales the interval is the major scale's, landed on the nearest note
/// of the scale (a "third" over C in C pentatonic is E).
pub fn diatonic(note: i32, steps: i32, key: i32, scale: &[i32]) -> i32 {
    if steps == 0 {
        return note;
    }
    let n = scale.len() as i32;
    if n != 7 {
        // Not a seven-note scale: take the major-scale interval and land
        // on the nearest note of this scale.
        const MAJOR: [i32; 8] = [0, 2, 4, 5, 7, 9, 11, 12];
        let k = steps.unsigned_abs().min(7) as usize;
        let semis = steps.signum() * MAJOR[k];
        return hum::snap((note + semis) as f32, key, scale);
    }
    // every scale note from two octaves below to two above
    let mut notes = Vec::with_capacity(5 * n as usize);
    let base = note - note.rem_euclid(12) + key.rem_euclid(12) - 24;
    for o in 0..5 {
        for &d in scale {
            notes.push(base + o * 12 + d);
        }
    }
    notes.sort();
    let i = notes.iter().position(|&x| x >= note).unwrap_or(0) as i32;
    notes.get((i + steps).clamp(0, notes.len() as i32 - 1) as usize).copied().unwrap_or(note)
}

/// One pitch-shifted voice: streaming TD-PSOLA over a shared input ring.
#[derive(Clone)]
pub struct Psola {
    /// Absolute output time of the next grain's centre.
    next: f64,
    /// Absolute input time of the last analysis grain's centre.
    ana: f64,
    ola: Vec<f32>,
}

impl Psola {
    pub fn new() -> Psola {
        Psola { next: 0.0, ana: 0.0, ola: vec![0.0; OLA] }
    }

    /// One output sample at absolute time `t`. `input(i)` is the input
    /// at absolute time i (anything up to `t` is there); `period` the
    /// input's period in samples; `ratio` the pitch change (2 = an octave
    /// up). Output is the input `DELAY` samples ago, shifted.
    pub fn tick(&mut self, t: u64, period: f32, ratio: f32, input: &impl Fn(f64) -> f32) -> f32 {
        let p = period.clamp(32.0, MAX_PERIOD) as f64;
        let ratio = ratio.clamp(0.25, 4.0) as f64;
        let tf = t as f64;
        if self.next < tf - 1.0 {
            // first sample (or after a stall): start a grain now
            self.next = tf;
            self.ana = tf - DELAY;
        }
        // Lay down every grain whose left edge has reached the output
        // (centred a period ahead, so all of it is still to be output).
        while self.next <= tf {
            let centre_out = self.next + p;
            // the analysis grain nearest to this point in the input,
            // stepping whole periods so consecutive grains stay in phase
            let want = centre_out - DELAY;
            while self.ana + 0.5 * p < want {
                self.ana += p;
            }
            while self.ana - 0.5 * p > want {
                self.ana -= p;
            }
            // Hann grains two periods wide, `p / ratio` apart, overlap
            // to a sum of `ratio`; divide that back out.
            let g = (1.0 / ratio).min(2.0) as f32;
            let half = p as i64;
            let c = centre_out.round() as i64;
            for k in -half..half {
                let w = 0.5 - 0.5 * ((k + half) as f64 * std::f64::consts::PI / half as f64).cos();
                let x = input(self.ana + k as f64);
                let i = (c + k).rem_euclid(OLA as i64) as usize;
                self.ola[i] += x * w as f32 * g;
            }
            self.next += p / ratio;
        }
        let i = (t % OLA as u64) as usize;
        let y = self.ola[i];
        self.ola[i] = 0.0;
        y
    }
}

/// Settings, and what the detector found.
pub struct Shared {
    pub key: AtomicI32,
    pub scale: AtomicUsize,
    /// 0..1: how far toward the note the lead is pulled.
    pub amount: AtomicF32,
    /// Retune time, seconds (0 = at once).
    pub speed: AtomicF32,
    pub harmony: AtomicU8,
    pub voice: [AtomicUsize; 2],
    pub lead_level: AtomicF32,
    pub harm_level: AtomicF32,
    pub dry_level: AtomicF32,
    pub gate: AtomicF32,
    /// Detected pitch (MIDI), or -1.
    pub pitch: AtomicF32,
    /// The note the lead is being tuned to, or -1.
    pub target: AtomicI32,
    /// What each harmony voice is singing (MIDI), or -1.
    pub singing: [AtomicI32; 3],
    trace: Mutex<VecDeque<(f32, f32)>>,
    load: Mutex<String>,
}

/// The NPU side: Hum's model over the input ring.
pub struct Detector {
    model: Model,
    spec: Spectrum,
    frame: Vec<f32>,
    feats: Vec<f32>,
    probs: Vec<f32>,
    next_end: u64,
    smooth: VecDeque<f32>,
    load: Load,
    s: Arc<Shared>,
}

impl Detector {
    fn new(s: Arc<Shared>) -> Detector {
        let model = Model::from_bytes(hum::MODEL).expect("hum model");
        let load = Load::new(model.macs());
        let n = model.output_len();
        Detector { model, spec: Spectrum::new(hum::FRAME, hum::N_FFT), frame: vec![0.0; hum::FRAME], feats: vec![0.0; hum::BINS], probs: vec![0.0; n], next_end: 0, smooth: VecDeque::new(), load, s }
    }

    pub fn run(&mut self, ring: &Ring) {
        let written = ring.written();
        if self.next_end == 0 || written > self.next_end + 30 * hum::HOP {
            self.next_end = written.max(hum::FRAME as u64);
        }
        while self.next_end <= written {
            if !ring.read_ending_at(self.next_end, &mut self.frame) {
                self.next_end = written + hum::HOP;
                break;
            }
            let frame = std::mem::take(&mut self.frame);
            self.analyse(&frame);
            self.frame = frame;
            self.next_end += hum::HOP;
        }
    }

    fn analyse(&mut self, frame: &[f32]) {
        let rms = (frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32).sqrt();
        let pitch = if rms < self.s.gate.get() {
            None
        } else {
            let t = std::time::Instant::now();
            hum::features(&mut self.spec, frame, &mut self.feats);
            self.model.run(&self.feats, &mut self.probs);
            neural::softmax(&mut self.probs);
            self.load.tick(t.elapsed());
            hum::decode(&self.probs)
        };
        // Median of three removes one-frame octave slips.
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
        }
        self.s.pitch.set(pitch.unwrap_or(-1.0));
        let scale = hum::SCALES[self.s.scale.load(Ordering::Relaxed) % hum::SCALES.len()].1;
        let target = pitch.map(|p| hum::snap(p, self.s.key.load(Ordering::Relaxed), scale));
        self.s.target.store(target.unwrap_or(-1), Ordering::Relaxed);
        if let Ok(mut tr) = self.s.trace.lock() {
            tr.push_back((pitch.unwrap_or(-1.0), target.map_or(-1.0, |t| t as f32)));
            while tr.len() > TRACE {
                tr.pop_front();
            }
        }
        if let Ok(mut l) = self.s.load.lock() {
            *l = self.load.summary();
        }
    }
}

/// The audio thread: input tap, the voices, the mix.
struct Engine {
    s: Arc<Shared>,
    tap: crate::apps::ai_input::Tap,
    ring: Vec<f32>,
    t: u64,
    lead: Psola,
    harm: [Psola; 3],
    /// Smoothed: the lead's shift in semitones, the period, and each
    /// harmony voice's level.
    shift: f32,
    period: f32,
    hgain: [f32; 3],
    hshift: [f32; 3],
    inbox: Option<Arc<NoteInboxRef>>,
    view: NoteView,
    held: Vec<u8>,
    /// The frame within the current block.
    fi: usize,
}

impl Engine {
    fn input_at(ring: &[f32], t: u64, at: f64) -> f32 {
        if at < 0.0 || at > t as f64 {
            return 0.0;
        }
        let i = at.floor();
        let f = (at - i) as f32;
        let a = ring[(i as u64 % RING as u64) as usize];
        let b = ring[((i as u64 + 1) % RING as u64) as usize];
        a + (b - a) * f
    }
}

impl Extra for Engine {
    fn block(&mut self, frames: usize, sr: f32) {
        self.tap.process(frames, sr);
        self.fi = 0;
        // Notes held for "keys" harmony: the newest three.
        if let Some(inbox) = self.inbox.as_ref() {
            inbox.poll(&mut self.view);
            self.held.retain(|&n| self.view.keys[n as usize] > 0);
            for n in 0..128u8 {
                if self.view.keys[n as usize] > 0 && !self.held.contains(&n) {
                    self.held.push(n);
                }
            }
            while self.held.len() > 3 {
                self.held.remove(0);
            }
        }
    }

    fn frame(&mut self, sr: f32) -> (f32, f32) {
        // The tap kept this block's full-rate input; walk it as we go.
        let x = self.tap.input().get(self.fi).copied().unwrap_or(0.0);
        self.fi += 1;
        let t = self.t;
        self.ring[(t % RING as u64) as usize] = x;

        let pitch = self.s.pitch.get();
        let voiced = pitch > 0.0;
        let a = |secs: f32| 1.0 - (-1.0 / (secs.max(1e-4) * sr)).exp();
        if voiced {
            let hz = kit::midi_hz(pitch);
            let p = (sr / hz).clamp(32.0, MAX_PERIOD);
            // a 5 ms glide on the period keeps grains from jumping
            self.period += (p - self.period) * a(0.005);
        }
        let target = self.s.target.load(Ordering::Relaxed);
        let want = if voiced && target >= 0 { (target as f32 - pitch) * self.s.amount.get() } else { 0.0 };
        // Retune speed: how fast the lead slides to the note. Unvoiced
        // sounds let go at once so consonants are never shifted.
        let speed = self.s.speed.get();
        self.shift += (want - self.shift) * if voiced { a(speed) } else { a(0.003) };
        let lead_pitch = if voiced { pitch + self.shift } else { -1.0 };

        // Harmony targets.
        let mode = self.s.harmony.load(Ordering::Relaxed);
        let key = self.s.key.load(Ordering::Relaxed);
        let scale = hum::SCALES[self.s.scale.load(Ordering::Relaxed) % hum::SCALES.len()].1;
        let mut targets = [None::<f32>; 3];
        if voiced {
            match mode {
                1 => {
                    let base = if target >= 0 { target } else { hum::snap(lead_pitch, key, scale) };
                    for v in 0..2 {
                        let steps = INTERVALS[self.s.voice[v].load(Ordering::Relaxed) % INTERVALS.len()].1;
                        if steps != 0 {
                            // follow the lead's own wobble (vibrato) around its note
                            let wobble = lead_pitch - base as f32;
                            targets[v] = Some(diatonic(base, steps, key, scale) as f32 + wobble);
                        }
                    }
                }
                2 => {
                    for (v, &n) in self.held.iter().enumerate().take(3) {
                        targets[v] = Some(n as f32);
                    }
                }
                _ => {}
            }
        }
        let p = self.period;
        let ring = &self.ring;
        let input = |at: f64| Engine::input_at(ring, t, at);
        let lead_ratio = 2f32.powf(self.shift / 12.0);
        let lead = self.lead.tick(t, p, if voiced { lead_ratio } else { 1.0 }, &input);
        let (mut hl, mut hr) = (0.0, 0.0);
        for v in 0..3 {
            let on = targets[v].is_some();
            if let Some(tp) = targets[v] {
                // a 20 ms glide between harmony notes, like a singer's
                self.hshift[v] += ((tp - pitch) - self.hshift[v]) * a(0.02);
                self.s.singing[v].store(tp.round() as i32, Ordering::Relaxed);
            } else {
                self.s.singing[v].store(-1, Ordering::Relaxed);
            }
            self.hgain[v] += ((if on { 1.0 } else { 0.0 }) - self.hgain[v]) * a(0.015);
            if self.hgain[v] > 1e-4 {
                let r = 2f32.powf(self.hshift[v] / 12.0);
                let y = self.harm[v].tick(t, p, r, &input) * self.hgain[v];
                // voice 1 a little left, voice 2 a little right, 3 centre
                let (gl, gr) = kit::pan_gains([-0.35, 0.35, 0.0][v]);
                hl += y * gl * std::f32::consts::SQRT_2;
                hr += y * gr * std::f32::consts::SQRT_2;
            } else {
                // keep its clock with the others while silent
                self.harm[v] = Psola::new();
                self.hshift[v] = 0.0;
            }
        }
        let dry = Engine::input_at(ring, t, t as f64 - DELAY);
        self.t += 1;
        let lead = lead * self.s.lead_level.get();
        let hg = self.s.harm_level.get() * 0.75;
        let dry = dry * self.s.dry_level.get();
        let l = lead + dry + hl * hg;
        let r = lead + dry + hr * hg;
        // The kit halves and soft-clips its output; give back the half.
        (l * 2.0, r * 2.0)
    }
}

pub struct Choir {
    sound: Sound,
    listen: Listen,
    pub s: Arc<Shared>,
    detector: Arc<Mutex<Detector>>,
    worker: Option<Worker>,
    inbox: Option<Arc<NoteInboxRef>>,
    row: usize,
    prev: [bool; 16],
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Input,
    Key,
    Scale,
    Amount,
    Speed,
    Harmony,
    Voice1,
    Voice2,
    Lead,
    Harm,
    Dry,
    Gate,
}
const ROWS: [Row; 12] = [Row::Input, Row::Key, Row::Scale, Row::Amount, Row::Speed, Row::Harmony, Row::Voice1, Row::Voice2, Row::Lead, Row::Harm, Row::Dry, Row::Gate];

impl Choir {
    pub fn new(sound: Sound, bus: Arc<AudioBus>, notes: Option<Arc<NoteBus>>) -> Choir {
        let s = Arc::new(Shared {
            key: AtomicI32::new(0),
            scale: AtomicUsize::new(1),
            amount: AtomicF32::new(1.0),
            speed: AtomicF32::new(0.04),
            harmony: AtomicU8::new(1),
            voice: [AtomicUsize::new(1), AtomicUsize::new(0)],
            lead_level: AtomicF32::new(0.9),
            harm_level: AtomicF32::new(0.7),
            dry_level: AtomicF32::new(0.0),
            gate: AtomicF32::new(0.01),
            pitch: AtomicF32::new(-1.0),
            target: AtomicI32::new(-1),
            singing: std::array::from_fn(|_| AtomicI32::new(-1)),
            trace: Mutex::new(VecDeque::with_capacity(TRACE + 1)),
            load: Mutex::new(String::new()),
        });
        sound.set_reverb(0.12);
        sound.set_volume(1.0);
        let detector = Arc::new(Mutex::new(Detector::new(Arc::clone(&s))));
        let inbox = notes.and_then(|b| b.register_instrument("choir", NAME));
        Choir { sound, listen: Listen::new(bus), s, detector, worker: None, inbox, row: 0, prev: [false; 16] }
    }

    /// For tests: runs the detector on whatever's arrived.
    #[cfg(test)]
    fn detect_now(&self) {
        self.detector.lock().unwrap().run(&self.listen.ring);
    }

    fn label(r: Row) -> &'static str {
        match r {
            Row::Input => "Input",
            Row::Key => "Key",
            Row::Scale => "Scale",
            Row::Amount => "Tuning",
            Row::Speed => "Retune speed",
            Row::Harmony => "Harmony",
            Row::Voice1 => "Voice 1",
            Row::Voice2 => "Voice 2",
            Row::Lead => "Lead level",
            Row::Harm => "Harmony level",
            Row::Dry => "Dry voice",
            Row::Gate => "Gate",
        }
    }

    fn value(&self, r: Row) -> String {
        let s = &self.s;
        match r {
            Row::Input => self.listen.name(),
            Row::Key => NOTE_NAMES[s.key.load(Ordering::Relaxed).rem_euclid(12) as usize].into(),
            Row::Scale => hum::SCALES[s.scale.load(Ordering::Relaxed) % hum::SCALES.len()].0.into(),
            Row::Amount => format!("{:.0}%", s.amount.get() * 100.0),
            Row::Speed => {
                let ms = s.speed.get() * 1000.0;
                if ms < 1.0 {
                    "instant".into()
                } else {
                    format!("{ms:.0} ms")
                }
            }
            Row::Harmony => HARMONY_MODES[s.harmony.load(Ordering::Relaxed) as usize % 3].into(),
            Row::Voice1 => INTERVALS[s.voice[0].load(Ordering::Relaxed) % INTERVALS.len()].0.into(),
            Row::Voice2 => INTERVALS[s.voice[1].load(Ordering::Relaxed) % INTERVALS.len()].0.into(),
            Row::Lead => format!("{:.0}", s.lead_level.get() * 100.0),
            Row::Harm => format!("{:.0}", s.harm_level.get() * 100.0),
            Row::Dry => format!("{:.0}", s.dry_level.get() * 100.0),
            Row::Gate => format!("{:.3}", s.gate.get()),
        }
    }

    fn edit(&mut self, d: i32) {
        let s = &self.s;
        let bump = |a: &AtomicF32, step: f32, lo: f32, hi: f32| a.set((a.get() + d as f32 * step).clamp(lo, hi));
        match ROWS[self.row] {
            Row::Input => self.listen.step(d),
            Row::Key => s.key.store((s.key.load(Ordering::Relaxed) + d).rem_euclid(12), Ordering::Relaxed),
            Row::Scale => s.scale.store((s.scale.load(Ordering::Relaxed) as i32 + d).rem_euclid(hum::SCALES.len() as i32) as usize, Ordering::Relaxed),
            Row::Amount => bump(&s.amount, 0.05, 0.0, 1.0),
            Row::Speed => {
                // in steps that feel even: 0, 5, 10, 20, 40, 80, 160, 320 ms
                let steps = [0.0, 0.005, 0.01, 0.02, 0.04, 0.08, 0.16, 0.32];
                let cur = steps.iter().position(|&x| (x - s.speed.get()).abs() < 1e-4).unwrap_or(4) as i32;
                s.speed.set(steps[(cur + d).clamp(0, steps.len() as i32 - 1) as usize]);
            }
            Row::Harmony => s.harmony.store((s.harmony.load(Ordering::Relaxed) as i32 + d).rem_euclid(3) as u8, Ordering::Relaxed),
            Row::Voice1 | Row::Voice2 => {
                let v = if ROWS[self.row] == Row::Voice1 { 0 } else { 1 };
                s.voice[v].store((s.voice[v].load(Ordering::Relaxed) as i32 + d).rem_euclid(INTERVALS.len() as i32) as usize, Ordering::Relaxed);
            }
            Row::Lead => bump(&s.lead_level, 0.05, 0.0, 1.0),
            Row::Harm => bump(&s.harm_level, 0.05, 0.0, 1.0),
            Row::Dry => bump(&s.dry_level, 0.05, 0.0, 1.0),
            Row::Gate => bump(&s.gate, 0.002, 0.0, 0.1),
        }
    }
}

impl App for Choir {
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        if index < ROWS.len() {
            let keep = std::mem::replace(&mut self.row, index);
            self.edit(delta);
            self.row = keep;
        }
    }
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn needs_background_audio(&self) -> bool {
        // a vocal chain keeps working while you look at another screen
        self.worker.is_some()
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        ROWS.iter().map(|r| (Choir::label(*r).to_string(), self.value(*r), false)).collect()
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let v1 = self.s.voice[0].load(Ordering::Relaxed);
        let v2 = self.s.voice[1].load(Ordering::Relaxed);
        std::array::from_fn(|p| {
            let (v, i) = if p < 8 { (v1, p) } else { (v2, p - 8) };
            if v == i {
                if i == 0 {
                    PadColor::Yellow
                } else if p < 8 {
                    PadColor::Green
                } else {
                    PadColor::Blue
                }
            } else {
                PadColor::Off
            }
        })
    }

    fn tick(&mut self, input: &Input) {
        self.listen.tick();
        // Top two rows pick voice 1's interval, bottom two voice 2's.
        for p in 0..16 {
            if input.grid[p] && !self.prev[p] {
                let (v, i) = if p < 8 { (0, p) } else { (1, p - 8) };
                self.s.voice[v].store(i, Ordering::Relaxed);
                if self.s.harmony.load(Ordering::Relaxed) == 0 && i != 0 {
                    self.s.harmony.store(1, Ordering::Relaxed);
                }
            }
        }
        self.prev = input.grid;
        if input.navigation_steps != 0 {
            self.row = (self.row as i32 + input.navigation_steps).clamp(0, ROWS.len() as i32 - 1) as usize;
        }
        if input.knob2 != 0 {
            self.edit(input.knob2.signum());
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = kit::rgb(20, 16, 30);
        let panel = kit::rgb(38, 30, 54);
        let dim = kit::rgb(140, 125, 165);
        let ink = kit::rgb(240, 170, 220);
        let lead_c = kit::rgb(255, 214, 120);
        let raw_c = kit::rgb(120, 110, 150);
        kit::round_rect(fb, 0, 0, 640, 360, 0, bg);
        kit::text(fb, NAME, 12, 8, Size2::Medium, kit::WHITE, -1);
        kit::text(fb, &self.listen.name(), 628, 12, Size2::Small, dim, 1);
        let lvl = self.listen.level.get().min(1.0);
        kit::round_rect(fb, 100, 16, 140, 8, 3, panel);
        kit::round_rect(fb, 100, 16, (140.0 * lvl) as i32 + 2, 8, 3, kit::rgb(120, 220, 160));

        // Tuner: the note, and how far off it you're singing.
        kit::round_rect(fb, 8, 40, 300, 120, 10, panel);
        let pitch = self.s.pitch.get();
        let target = self.s.target.load(Ordering::Relaxed);
        if pitch > 0.0 && target >= 0 {
            let name = format!("{}{}", NOTE_NAMES[target.rem_euclid(12) as usize], target / 12 - 1);
            kit::text(fb, &name, 158, 52, Size2::Huge, kit::WHITE, 0);
            let cents = ((pitch - target as f32) * 100.0).clamp(-50.0, 50.0);
            kit::round_rect(fb, 28, 130, 260, 6, 3, kit::blend(panel, kit::WHITE, 0.15));
            kit::round_rect(fb, 157, 124, 2, 18, 0, kit::blend(panel, kit::WHITE, 0.4));
            let x = 158 + (cents / 50.0 * 130.0) as i32;
            let c = if cents.abs() < 8.0 { kit::rgb(120, 230, 150) } else { lead_c };
            kit::round_rect(fb, x - 4, 122, 9, 22, 3, c);
            kit::text(fb, &format!("{:+.0} cents", cents), 158, 146, Size2::Small, dim, 0);
        } else {
            kit::text(fb, "sing", 158, 80, Size2::Large, dim, 0);
        }

        // Pitch trace: what you sang (dim) and what comes out (bright).
        kit::round_rect(fb, 8, 168, 300, 140, 10, panel);
        let tr: Vec<(f32, f32)> = self.s.trace.lock().map(|t| t.iter().copied().collect()).unwrap_or_default();
        let voiced: Vec<f32> = tr.iter().filter(|(p, _)| *p > 0.0).map(|(p, _)| *p).collect();
        let centre = if voiced.is_empty() { 60.0 } else { voiced.iter().sum::<f32>() / voiced.len() as f32 };
        let lo = centre - 6.0;
        let y_of = |m: f32| 300 - ((m - lo) / 12.0 * 124.0) as i32;
        let key = self.s.key.load(Ordering::Relaxed);
        let scale = hum::SCALES[self.s.scale.load(Ordering::Relaxed) % hum::SCALES.len()].1;
        for n in lo.ceil() as i32..=(lo + 12.0) as i32 {
            if scale.contains(&(n - key).rem_euclid(12)) {
                let y = y_of(n as f32);
                kit::round_rect(fb, 14, y, 288, 1, 0, kit::blend(panel, kit::WHITE, if (n - key).rem_euclid(12) == 0 { 0.25 } else { 0.1 }));
            }
        }
        let amount = self.s.amount.get();
        for (i, (p, t)) in tr.iter().enumerate() {
            if *p <= 0.0 {
                continue;
            }
            let x = 14 + (i as i32 * 288 / TRACE as i32);
            let y = y_of(*p).clamp(172, 304);
            kit::round_rect(fb, x, y, 1, 2, 0, raw_c);
            if *t >= 0.0 {
                let out = p + (t - p) * amount;
                let y2 = y_of(out).clamp(172, 304);
                kit::round_rect(fb, x, y2 - 1, 1, 3, 0, lead_c);
            }
        }

        // Settings.
        for (i, r) in ROWS.iter().enumerate() {
            let y = 40 + i as i32 * 22;
            let sel = i == self.row;
            kit::round_rect(fb, 316, y, 316, 20, 5, if sel { kit::blend(panel, kit::WHITE, 0.15) } else { panel });
            kit::text(fb, Choir::label(*r), 324, y + 4, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
            let v: String = self.value(*r).chars().take(22).collect();
            kit::text(fb, &v, 624, y + 4, Size2::Small, ink, 1);
        }
        // Who's singing what.
        let mut parts = Vec::new();
        for v in 0..3 {
            let n = self.s.singing[v].load(Ordering::Relaxed);
            if n >= 0 {
                parts.push(format!("{}{}", NOTE_NAMES[n.rem_euclid(12) as usize], n / 12 - 1));
            }
        }
        let sing = if parts.is_empty() { "harmony: -".to_string() } else { format!("harmony: {}", parts.join("  ")) };
        kit::text(fb, &sing, 324, 314, Size2::Small, ink, -1);
        let load = self.s.load.lock().map(|l| l.clone()).unwrap_or_default();
        kit::text(fb, &load, 12, 314, Size2::Small, dim, -1);
        kit::footer(fb, "pads: top rows voice 1, bottom rows voice 2   up/down + left/right: settings", panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if !cfg!(test) {
            let d = Arc::clone(&self.detector);
            let ring = Arc::clone(&self.listen.ring);
            self.worker = Some(Worker::spawn("choir-npu", move || {
                if let Ok(mut d) = d.lock() {
                    d.run(&ring);
                }
            }));
        }
        let e = Engine {
            s: Arc::clone(&self.s),
            tap: self.listen.tap(),
            ring: vec![0.0; RING],
            t: 0,
            lead: Psola::new(),
            harm: [Psola::new(), Psola::new(), Psola::new()],
            shift: 0.0,
            period: 200.0,
            hgain: [0.0; 3],
            hshift: [0.0; 3],
            inbox: self.inbox.take(),
            view: NoteView::default(),
            held: Vec::with_capacity(8),
            fi: 0,
        };
        Some(self.sound.processor(None, Some(Box::new(e))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<AudioBus> = ctx.get();
    Box::new(Choir::new(Sound::new(NAME, &modbus, &mixer, &bus), bus, ctx.try_get()))
}

#[cfg(test)]
mod tests {
    use super::*;
    const SR: f32 = 48_000.0;

    /// The fundamental: the shortest lag whose normalised autocorrelation
    /// is within 10% of the best (so a formant-loud harmonic, or an
    /// octave below, can't fool it), refined with a parabola.
    fn pitch_hz(x: &[f32], sr: f32) -> f32 {
        let x = &x[..x.len().min(16_384)];
        let n = x.len() / 2;
        let ac = |l: usize| {
            let (mut c, mut e1, mut e2) = (0.0f64, 0.0f64, 0.0f64);
            for i in 0..n {
                c += (x[i] * x[i + l]) as f64;
                e1 += (x[i] * x[i]) as f64;
                e2 += (x[i + l] * x[i + l]) as f64;
            }
            c / (e1 * e2).sqrt().max(1e-12)
        };
        let (lo, hi) = ((sr / 1000.0) as usize, (sr / 60.0) as usize);
        let r: Vec<f64> = (0..=hi + 1).map(|l| if l < lo { 0.0 } else { ac(l) }).collect();
        let best = r[lo..=hi].iter().cloned().fold(f64::MIN, f64::max);
        let l = (lo + 1..hi).find(|&l| r[l] >= 0.9 * best && r[l] >= r[l - 1] && r[l] >= r[l + 1]).unwrap_or(lo);
        let (a, b, c) = (r[l - 1], r[l], r[l + 1]);
        let d = 0.5 * (a - c) / (a - 2.0 * b + c);
        sr / (l as f64 + if d.is_finite() { d } else { 0.0 }) as f32
    }

    /// A sung vowel with one formant at `formant` Hz.
    fn vowel(hz: f32, formant: f32, n: usize, phase: &mut f32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *phase += hz / SR;
                let mut x = 0.0;
                let mut h = 1;
                while hz * (h as f32) < 5000.0 {
                    let f = hz * h as f32;
                    let env = 1.0 / (1.0 + ((f - formant) / 120.0).powi(2)) + 0.03;
                    x += env * (*phase * h as f32 * std::f32::consts::TAU).sin();
                    h += 1;
                }
                x * 0.25
            })
            .collect()
    }

    /// Magnitude of the `hz` component (a single DFT bin).
    fn mag(x: &[f32], hz: f32) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / x.len() as f64).cos();
            let a = std::f64::consts::TAU * hz as f64 * i as f64 / SR as f64;
            re += v as f64 * w * a.cos();
            im += v as f64 * w * a.sin();
        }
        ((re * re + im * im).sqrt() / x.len() as f64) as f32
    }

    fn shift(input: &[f32], period: f32, ratio: f32) -> Vec<f32> {
        let mut p = Psola::new();
        let mut out = Vec::with_capacity(input.len());
        for t in 0..input.len() {
            let f = |at: f64| if at < 0.0 || at > t as f64 { 0.0 } else { input[at as usize] };
            out.push(p.tick(t as u64, period, ratio, &f));
        }
        out
    }

    #[test]
    fn diatonic_harmony_stays_in_the_key() {
        let major = hum::SCALES[1].1;
        assert_eq!(diatonic(64, 2, 0, major), 67, "E + a third in C = G");
        assert_eq!(diatonic(69, -2, 0, major), 65, "A - a third = F");
        assert_eq!(diatonic(71, 4, 0, major), 77, "B + a fifth in C = F");
        assert_eq!(diatonic(60, 7, 0, major), 72);
        assert_eq!(diatonic(62, 2, 2, major), 66, "D + a third in D major = F#");
        let pent = hum::SCALES[3].1;
        assert_eq!(diatonic(60, 2, 0, pent), 64, "a third in C pentatonic is still E");
    }

    #[test]
    fn psola_moves_the_pitch_and_keeps_the_formant() {
        let mut ph = 0.0;
        let x = vowel(150.0, 700.0, 48_000, &mut ph);
        let y = shift(&x, SR / 150.0, 1.5);
        let tail = &y[12_000..];
        let hz = pitch_hz(tail, SR);
        assert!((hz - 225.0).abs() < 2.0, "{hz}");
        // The 3rd harmonic (675 Hz) sits on the formant; the 5th (1125)
        // is where a tape-style shift would have moved the formant to.
        let on = mag(tail, 675.0);
        let off = mag(tail, 1125.0);
        assert!(on > 3.0 * off, "formant kept: 675 Hz {on} vs 1125 Hz {off}");
        // and a fifth down
        let y = shift(&x, SR / 150.0, 2.0 / 3.0);
        let hz = pitch_hz(&y[12_000..], SR);
        assert!((hz - 100.0).abs() < 1.5, "{hz}");
    }

    #[test]
    fn unshifted_it_passes_the_input_through_delayed() {
        let mut rng = kit::Rng::new(7);
        let x: Vec<f32> = (0..24_000).map(|_| rng.next_u32() as f32 / u32::MAX as f32 - 0.5).collect();
        let y = shift(&x, 200.0, 1.0);
        // find the lag, then check it's the input
        let best = (1300..1900usize)
            .max_by(|&a, &b| {
                let c = |l: usize| (6000..20_000).map(|i| x[i - l] * y[i]).sum::<f32>();
                c(a).total_cmp(&c(b))
            })
            .unwrap();
        let err: f32 = (6000..20_000).map(|i| (x[i - best] - y[i]).powi(2)).sum::<f32>();
        let sig: f32 = (6000..20_000).map(|i| y[i].powi(2)).sum::<f32>();
        assert!(err / sig < 1e-3, "lag {best}: error {}", err / sig);
    }

    struct Rig {
        c: Choir,
        p: Box<dyn AudioProcessor>,
        input: Arc<Mutex<Vec<f32>>>,
        ph: f32,
    }

    impl Rig {
        fn new(notes: Option<Arc<NoteBus>>) -> Rig {
            let bus = Arc::new(AudioBus::new());
            let input = bus.register("Hardware input");
            let mut c = Choir::new(Sound::detached(), bus, notes);
            c.sound.set_reverb(0.0);
            let p = c.audio_processor().unwrap();
            Rig { c, p, input, ph: 0.0 }
        }
        /// Sings `hz` for `blocks` of 480, returns the left channel.
        fn sing(&mut self, hz: f32, blocks: usize) -> Vec<f32> {
            let mut out = Vec::new();
            let mut buf = vec![0.0f32; 960];
            for _ in 0..blocks {
                *self.input.lock().unwrap() = vowel(hz, 700.0, 480, &mut self.ph);
                buf.iter_mut().for_each(|x| *x = 0.0);
                self.p.process(&mut buf, 2, SR);
                out.extend(buf.chunks(2).map(|c| c[0]));
                self.c.detect_now();
            }
            out
        }
    }

    #[test]
    fn a_flat_note_comes_out_in_tune() {
        let mut r = Rig::new(None);
        r.c.s.voice[0].store(0, Ordering::Relaxed); // lead only
        r.c.s.speed.set(0.0);
        // A3 is 220 Hz; sing 40 cents flat
        let out = r.sing(215.0, 150);
        assert_eq!(r.c.s.target.load(Ordering::Relaxed), 57);
        let hz = pitch_hz(&out[out.len() - 24_000..], SR);
        assert!((hz - 220.0).abs() < 1.5, "tuned to {hz}");
        // with tuning off it stays where it was sung
        r.c.s.amount.set(0.0);
        let out = r.sing(215.0, 100);
        let hz = pitch_hz(&out[out.len() - 24_000..], SR);
        assert!((hz - 215.0).abs() < 1.5, "left alone at {hz}");
    }

    #[test]
    fn the_harmony_sings_a_third_above_in_the_key() {
        let mut r = Rig::new(None);
        r.c.s.lead_level.set(0.0);
        r.c.s.voice[0].store(1, Ordering::Relaxed); // 3rd up
        let out = r.sing(220.0, 150);
        // A in C major: a third up is C (261.6 Hz), not C# (277)
        let hz = pitch_hz(&out[out.len() - 24_000..], SR);
        assert!((hz - 261.6).abs() < 3.0, "{hz}");
        assert_eq!(r.c.s.singing[0].load(Ordering::Relaxed), 60);
        // in A major the same third is C#
        r.c.s.key.store(9, Ordering::Relaxed);
        let out = r.sing(220.0, 100);
        let hz = pitch_hz(&out[out.len() - 24_000..], SR);
        assert!((hz - 277.2).abs() < 3.0, "{hz}");
    }

    #[test]
    fn in_keys_mode_the_harmony_sings_the_notes_you_hold() {
        let bus = Arc::new(NoteBus::new());
        let mut r = Rig::new(Some(Arc::clone(&bus)));
        r.c.s.lead_level.set(0.0);
        r.c.s.harmony.store(2, Ordering::Relaxed);
        let inbox = bus.inbox_ref(bus.instrument_index(NAME).unwrap());
        inbox.note_on(64, 100); // E4
        let out = r.sing(196.0, 150);
        let hz = pitch_hz(&out[out.len() - 24_000..], SR);
        assert!((hz - 329.6).abs() < 3.5, "{hz}");
        inbox.note_off(64);
        let out = r.sing(196.0, 30);
        let tail = &out[out.len() - 4800..];
        assert!(tail.iter().all(|x| x.abs() < 1e-3), "silent when nothing's held");
        let mut fb = FrameBuffer::new();
        r.c.draw(&mut fb);
    }

    #[test]
    fn silence_and_breath_leave_it_quiet() {
        let mut r = Rig::new(None);
        let mut out = Vec::new();
        let mut buf = vec![0.0f32; 960];
        for _ in 0..60 {
            *r.input.lock().unwrap() = vec![0.0; 480];
            buf.iter_mut().for_each(|x| *x = 0.0);
            r.p.process(&mut buf, 2, SR);
            out.extend(buf.chunks(2).map(|c| c[0]));
            r.c.detect_now();
        }
        assert!(out.iter().all(|x| x.abs() < 1e-6));
        assert_eq!(r.c.s.pitch.get(), -1.0);
    }
}
