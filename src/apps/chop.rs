//! Chop: a sampler built around resampling, in the spirit of the SP-404.
//!
//! Sixteen pads, each a sample. Record any source -- the microphone, a
//! line in, or any app's output -- into a pad, or record a longer take and
//! chop it across the pads, either into equal slices or at each hit
//! (transients). Everything plays through two effect slots (vinyl, lo-fi,
//! DJ filter, tempo delay, reverb, compressor, isolator, tape), and the
//! resample button records Chop's own output, effects and all, into the
//! next empty pad -- so a beat can be bounced, crushed, and chopped again.
//!
//! Views (F2):
//! - PLAY: the pads play (and pick) samples. SELECT starts/stops
//!   resampling into the next empty pad.
//! - EDIT: the picked pad's start, end, pitch, gain, mode (one-shot,
//!   gate, loop), reverse, and loading any WAV from the library (media/).
//! - SAMPLE: the source, how to chop (whole, 4, 8, 16 or at transients),
//!   and SELECT to record; saving and loading the bank.
//! - FX: the two effect slots and their two controls each, and the tempo
//!   the delay follows.
//!
//! Notes from MIDI or other apps play the pads (C2 = pad 1), so Session
//! can sequence it.

use crate::app::{App, Input, SlintExtra};
use crate::apps::kids_kit::{self as kit, Extra, Noise, Size2, Sound, Svf};
use crate::apps::voltage_fx::{FxSettings, StereoFx};
use crate::audio_bus::{AudioBus, NO_SOURCE};
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::note_bus::{NoteBus, NoteInboxRef, NoteView};
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Chop";
pub const PADS: usize = 16;
const SR: f32 = 48_000.0;
/// The longest recording: 32 seconds.
const MAX_REC: usize = 48_000 * 32;
const FX_NAMES: [&str; 9] = ["Off", "Vinyl", "Lo-fi", "Filter", "Delay", "Reverb", "Compress", "Isolator", "Tape"];
const FX_PARAMS: [[&str; 2]; 9] = [["-", "-"], ["Age", "Noise"], ["Bits", "Rate"], ["Cutoff", "Res"], ["Time", "Feedback"], ["Size", "Mix"], ["Amount", "Punch"], ["Low", "High"], ["Drive", "Wow"]];
const CHOPS: [&str; 5] = ["whole", "4 slices", "8 slices", "16 slices", "at hits"];

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum Mode {
    OneShot,
    Gate,
    Loop,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct PadSet {
    pub name: String,
    pub start: f32,
    pub end: f32,
    pub pitch: f32,
    pub gain: f32,
    pub mode: Mode,
    pub reverse: bool,
}

impl Default for PadSet {
    fn default() -> Self {
        PadSet { name: String::new(), start: 0.0, end: 1.0, pitch: 0.0, gain: 1.0, mode: Mode::OneShot, reverse: false }
    }
}

#[derive(Clone, Default)]
pub struct Pad {
    pub data: Option<Arc<Vec<f32>>>,
    pub set: PadSet,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub struct FxSlot {
    pub kind: usize,
    pub a: f32,
    pub b: f32,
}

pub struct Shared {
    pub pads: Mutex<Vec<Pad>>,
    pub fx: Mutex<[FxSlot; 2]>,
    pub tempo: AtomicF32,
    /// Pad presses from the UI: (pad, velocity, on).
    pub live: Mutex<Vec<(usize, f32, bool)>>,
    /// 0 none, 1 recording the input, 2 resampling the output.
    pub recording: AtomicUsize,
    pub rec_len: AtomicUsize,
    /// A finished recording, handed from the audio thread.
    pub finished: Mutex<Option<Vec<f32>>>,
    /// An empty buffer for the next recording, so the audio thread never
    /// allocates.
    pub spare: Mutex<Option<Vec<f32>>>,
    pub source: Mutex<Option<Arc<Mutex<Vec<f32>>>>>,
    pub level: AtomicF32,
    /// Where each pad's voice is, 0..1 (or -1), for the screen.
    pub playhead: [AtomicF32; PADS],
}

struct Voice {
    data: Arc<Vec<f32>>,
    pos: f64,
    rate: f64,
    start: f64,
    end: f64,
    gain: f32,
    mode: Mode,
    held: bool,
    rev: bool,
    env: f32,
    active: bool,
}

/// One effect slot's state.
struct Fx {
    lp: [Svf; 2],
    noise: Noise,
    hold: (f32, f32),
    hold_n: u32,
    delay: Vec<[f32; 2]>,
    dw: usize,
    dlp: [f32; 2],
    verb: StereoFx,
    env: f32,
    wow: f32,
    tape: Vec<[f32; 2]>,
    tw: usize,
    iso: [Svf; 2],
    iso_hi: [Svf; 2],
}

impl Fx {
    fn new(seed: u32) -> Fx {
        Fx { lp: [Svf::default(); 2], noise: Noise::new(seed), hold: (0.0, 0.0), hold_n: 0, delay: vec![[0.0; 2]; 96_000], dw: 0, dlp: [0.0; 2], verb: StereoFx::new(), env: 0.0, wow: 0.0, tape: vec![[0.0; 2]; 2048], tw: 0, iso: [Svf::default(); 2], iso_hi: [Svf::default(); 2] }
    }

    fn read_tape(&self, delay: f32) -> [f32; 2] {
        let n = self.tape.len();
        let pos = self.tw as f32 - delay;
        let pos = if pos < 0.0 { pos + n as f32 } else { pos };
        let i = pos as usize % n;
        let f = pos.fract();
        let a = self.tape[i];
        let b = self.tape[(i + 1) % n];
        [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
    }

    #[inline]
    fn tick(&mut self, slot: FxSlot, tempo: f32, x: (f32, f32), sr: f32) -> (f32, f32) {
        let (a, b) = (slot.a.clamp(0.0, 1.0), slot.b.clamp(0.0, 1.0));
        match slot.kind {
            1 => {
                // Vinyl: the top end goes as it ages, the speed wobbles, it crackles and hisses.
                self.wow = (self.wow + 0.55 / sr).fract();
                self.tape[self.tw] = [x.0, x.1];
                self.tw = (self.tw + 1) % self.tape.len();
                let w = self.read_tape(40.0 + a * 60.0 * (1.0 + (self.wow * std::f32::consts::TAU).sin()));
                let cutoff = 16_000.0 * (1.0 - a * 0.8);
                let l = self.lp[0].tick(w[0], cutoff, 0.7, sr).lp;
                let r = self.lp[1].tick(w[1], cutoff, 0.7, sr).lp;
                let n = self.noise.next();
                let crackle = if self.noise.next() > 1.0 - a * 0.0015 { n * 0.6 } else { 0.0 };
                let hiss = self.noise.next() * b * 0.02;
                (l + crackle + hiss, r + crackle + hiss)
            }
            2 => {
                // Lo-fi: fewer bits, a lower sample rate (held, not filtered: the aliasing is the point).
                let bits = 16.0 - a * 12.0;
                let q = 2f32.powf(bits - 1.0);
                let every = (1.0 + b * 11.0) as u32;
                if self.hold_n == 0 {
                    self.hold = ((x.0 * q).round() / q, (x.1 * q).round() / q);
                }
                self.hold_n = (self.hold_n + 1) % every;
                self.hold
            }
            3 => {
                // A DJ filter: left of centre low-pass, right of centre high-pass.
                let res = 0.6 + b * 6.0;
                if a < 0.48 {
                    let c = 60.0 * (20_000f32 / 60.0).powf(a / 0.48);
                    (self.lp[0].tick(x.0, c, res, sr).lp, self.lp[1].tick(x.1, c, res, sr).lp)
                } else if a > 0.52 {
                    let c = 20.0 * (8_000f32 / 20.0).powf((a - 0.52) / 0.48);
                    (self.lp[0].tick(x.0, c, res, sr).hp, self.lp[1].tick(x.1, c, res, sr).hp)
                } else {
                    x
                }
            }
            4 => {
                // Delay in tempo: 1/16 to 1/2 notes, ping-ponging.
                let notes = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0];
                let beats = notes[((a * 5.99) as usize).min(5)];
                let d = ((beats * 60.0 / tempo.max(30.0) * sr) as usize).clamp(1, self.delay.len() - 1);
                let n = self.delay.len();
                let back = self.delay[(self.dw + n - d) % n];
                let k = 1.0 - (-std::f32::consts::TAU * 3000.0 / sr).exp();
                self.dlp[0] += (back[0] - self.dlp[0]) * k;
                self.dlp[1] += (back[1] - self.dlp[1]) * k;
                let fb = b * 0.85;
                self.delay[self.dw] = [x.0 + self.dlp[1] * fb, self.dlp[0] * fb];
                self.dw = (self.dw + 1) % n;
                (x.0 + back[0] * 0.45, x.1 + back[1] * 0.45)
            }
            5 => {
                let s = FxSettings { reverb_size: a, reverb_mix: b, ..Default::default() };
                let (l, r) = self.verb.tick((x.0 + x.1) * 0.5, &s, sr);
                (l + (x.0 - x.1) * 0.5, r - (x.0 - x.1) * 0.5)
            }
            6 => {
                // A compressor with a fast attack: Amount lowers the threshold, Punch adds make-up.
                let lvl = x.0.abs().max(x.1.abs());
                let k = if lvl > self.env { 1.0 - (-1.0 / (0.003 * sr)).exp() } else { 1.0 - (-1.0 / (0.12 * sr)).exp() };
                self.env += (lvl - self.env) * k;
                let th = 10f32.powf((-6.0 - a * 30.0) / 20.0);
                let gain = if self.env > th { (th / self.env).powf(0.75) } else { 1.0 };
                let make = 1.0 + b * 3.0;
                ((x.0 * gain * make).tanh(), (x.1 * gain * make).tanh())
            }
            7 => {
                // Isolator: low and high bands each from killed (0) to +6 dB (1); 0.66 is flat.
                let gain = |v: f32| if v < 0.66 { v / 0.66 } else { 1.0 + (v - 0.66) / 0.34 };
                let (gl, gh) = (gain(a), gain(b));
                let mut out = [0.0; 2];
                for (c, v) in [x.0, x.1].iter().enumerate() {
                    let lo = self.iso[c].tick(*v, 250.0, 0.7, sr).lp;
                    let hi = self.iso_hi[c].tick(*v, 3000.0, 0.7, sr).hp;
                    let mid = v - lo - hi;
                    out[c] = lo * gl + mid + hi * gh;
                }
                (out[0], out[1])
            }
            8 => {
                // Tape: soft saturation and a slow wobble.
                self.wow = (self.wow + (0.4 + b * 5.0) / sr).fract();
                self.tape[self.tw] = [x.0, x.1];
                self.tw = (self.tw + 1) % self.tape.len();
                let w = self.read_tape(30.0 + b * 25.0 * (self.wow * std::f32::consts::TAU).sin());
                let g = 1.0 + a * 6.0;
                ((w[0] * g).tanh() / g.sqrt(), (w[1] * g).tanh() / g.sqrt())
            }
            _ => x,
        }
    }
}

/// The audio thread: the input tap, the voices, the effects, recording.
struct Engine {
    s: Arc<Shared>,
    voices: Vec<Option<Voice>>,
    fx: [Fx; 2],
    rec: Option<Vec<f32>>,
    input: Vec<f32>,
    read: usize,
    inbox: Option<Arc<NoteInboxRef>>,
    view: NoteView,
    strikes: [u8; 128],
    keys: [u8; 128],
    peak: f32,
}

impl Engine {
    fn start(&mut self, pad: usize, vel: f32) {
        let p = match self.s.pads.try_lock() {
            Ok(p) => p[pad].clone(),
            Err(_) => return,
        };
        let Some(data) = p.data else { return };
        let n = data.len() as f64;
        let (a, b) = (p.set.start.min(p.set.end) as f64 * n, p.set.end.max(p.set.start) as f64 * n);
        if b - a < 2.0 {
            return;
        }
        let rate = 2f64.powf(p.set.pitch as f64 / 12.0);
        self.voices[pad] = Some(Voice { pos: if p.set.reverse { b - 1.0 } else { a }, data, rate, start: a, end: b, gain: p.set.gain * vel, mode: p.set.mode, held: true, rev: p.set.reverse, env: 0.0, active: true });
    }

    fn stop(&mut self, pad: usize) {
        if let Some(v) = self.voices[pad].as_mut() {
            v.held = false;
        }
    }
}

impl Extra for Engine {
    fn block(&mut self, frames: usize, _sr: f32) {
        self.input.clear();
        if let Ok(src) = self.s.source.try_lock() {
            if let Some(b) = src.as_ref() {
                if let Ok(b) = b.try_lock() {
                    self.input.extend(b.iter().take(frames));
                }
            }
        }
        self.input.resize(frames, 0.0);
        self.read = 0;
        let p = self.input.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        self.peak = p.max(self.peak * 0.9);
        self.s.level.set(self.peak);
        let live: Vec<(usize, f32, bool)> = self.s.live.try_lock().map(|mut l| std::mem::take(&mut *l)).unwrap_or_default();
        for (pad, vel, on) in live {
            if on {
                self.start(pad, vel);
            } else {
                self.stop(pad);
            }
        }
        if let Some(inbox) = self.inbox.clone() {
            inbox.poll(&mut self.view);
            for k in 0..PADS {
                let n = 36 + k;
                if self.view.strikes[n] != self.strikes[n] {
                    self.strikes[n] = self.view.strikes[n];
                    self.start(k, (self.view.keys[n].max(1) as f32 / 127.0).max(0.3));
                } else if self.view.keys[n] == 0 && self.keys[n] > 0 {
                    self.stop(k);
                }
                self.keys[n] = self.view.keys[n];
            }
        }
        // Recording starts and stops.
        let want = self.s.recording.load(Ordering::Relaxed);
        if want != 0 && self.rec.is_none() {
            if let Ok(mut sp) = self.s.spare.try_lock() {
                if let Some(mut b) = sp.take() {
                    b.clear();
                    self.rec = Some(b);
                }
            }
        } else if want == 0 && self.rec.is_some() {
            if let Ok(mut f) = self.s.finished.try_lock() {
                *f = self.rec.take();
            }
        }
    }

    fn frame(&mut self, sr: f32) -> (f32, f32) {
        let inp = self.input.get(self.read).copied().unwrap_or(0.0);
        self.read += 1;
        let (mut l, mut r) = (0.0, 0.0);
        for (k, slot) in self.voices.iter_mut().enumerate() {
            let Some(v) = slot.as_mut() else { continue };
            let i = v.pos as usize;
            let f = (v.pos - i as f64) as f32;
            let a = v.data.get(i).copied().unwrap_or(0.0);
            let b = v.data.get(i + 1).copied().unwrap_or(a);
            let x = a + (b - a) * f;
            let target = if v.mode == Mode::Gate && !v.held { 0.0 } else { 1.0 };
            v.env += (target - v.env) * if target > v.env { 0.3 } else { 0.002 };
            let y = x * v.env * v.gain;
            l += y;
            r += y;
            if v.rev {
                v.pos -= v.rate;
            } else {
                v.pos += v.rate;
            }
            let out = if v.rev { v.pos < v.start } else { v.pos >= v.end - 1.0 };
            if out {
                if v.mode == Mode::Loop && v.held {
                    v.pos = if v.rev { v.end - 1.0 } else { v.start };
                } else {
                    v.active = false;
                }
            }
            if v.mode == Mode::Gate && !v.held && v.env < 1e-4 {
                v.active = false;
            }
            if v.mode == Mode::Loop && !v.held {
                v.active = v.env > 1e-4;
                v.env *= 0.999;
            }
            self.s.playhead[k].set(((v.pos - v.start) / (v.end - v.start)) as f32);
            if !v.active {
                self.s.playhead[k].set(-1.0);
                *slot = None;
            }
        }
        let slots = self.s.fx.try_lock().map(|f| *f).unwrap_or([FxSlot { kind: 0, a: 0.5, b: 0.5 }; 2]);
        let tempo = self.s.tempo.get();
        let mut y = (l, r);
        for k in 0..2 {
            y = self.fx[k].tick(slots[k], tempo, y, sr);
        }
        if let Some(rec) = self.rec.as_mut() {
            if rec.len() < MAX_REC {
                let v = if self.s.recording.load(Ordering::Relaxed) == 2 { (y.0 + y.1) * 0.5 } else { inp };
                rec.push(v);
                self.s.rec_len.store(rec.len(), Ordering::Relaxed);
            }
        }
        y
    }
}

/// Where a recording should be cut: equal slices, or at each hit.
pub fn slice_points(data: &[f32], how: usize, sensitivity: f32) -> Vec<(usize, usize)> {
    let n = data.len();
    if n == 0 {
        return Vec::new();
    }
    let count = match how {
        0 => 1,
        1 => 4,
        2 => 8,
        3 => 16,
        _ => 0,
    };
    if count > 0 {
        return (0..count).map(|k| (n * k / count, n * (k + 1) / count)).collect();
    }
    // Transients: a 5 ms frame much louder than the 50 ms before it.
    let hop = 240;
    let energies: Vec<f32> = data.chunks(hop).map(|c| c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).collect();
    let floor = energies.iter().cloned().fold(0.0f32, f32::max) * 0.001;
    let ratio = 2.0 + (1.0 - sensitivity) * 10.0;
    let mut starts = Vec::new();
    let mut last = 0usize;
    for i in 0..energies.len() {
        let before = energies[i.saturating_sub(10)..i].iter().sum::<f32>() / 10f32.max(i.min(10) as f32);
        if energies[i] > floor && energies[i] > before * ratio && (starts.is_empty() || i >= last + 16) {
            starts.push(i * hop);
            last = i;
        }
        if starts.len() >= PADS {
            break;
        }
    }
    if starts.is_empty() {
        return vec![(0, n)];
    }
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        let e = starts.get(k + 1).copied().unwrap_or(n);
        out.push((s, e));
    }
    out
}

/// Reads a WAV as mono at 48 kHz.
pub fn read_wav(path: &Path) -> Result<Vec<f32>, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = r.spec();
    let ch = spec.channels.max(1) as usize;
    let raw: Vec<f32> = if spec.sample_format == hound::SampleFormat::Float {
        r.samples::<f32>().filter_map(Result::ok).collect()
    } else {
        let div = 2f32.powi(spec.bits_per_sample as i32 - 1);
        r.samples::<i32>().filter_map(Result::ok).map(|x| x as f32 / div).collect()
    };
    let mono: Vec<f32> = raw.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).take(MAX_REC * 2).collect();
    let rate = spec.sample_rate as f64 / SR as f64;
    if (rate - 1.0).abs() < 1e-6 {
        return Ok(mono);
    }
    let n = (mono.len() as f64 / rate) as usize;
    Ok((0..n.min(MAX_REC))
        .map(|i| {
            let p = i as f64 * rate;
            let j = p as usize;
            let f = (p - j as f64) as f32;
            let a = mono.get(j).copied().unwrap_or(0.0);
            a + (mono.get(j + 1).copied().unwrap_or(a) - a) * f
        })
        .collect())
}

pub fn write_wav(path: &Path, data: &[f32]) -> Result<(), String> {
    let spec = hound::WavSpec { channels: 1, sample_rate: SR as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for &x in data {
        w.write_sample((x.clamp(-1.0, 1.0) * 32767.0) as i16).map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

fn scan_wavs(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 3 || out.len() > 500 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            scan_wavs(&p, out, depth + 1);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")) {
            out.push(p);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum View {
    Play,
    Edit,
    Sample,
    Fx,
}
const VIEWS: [View; 4] = [View::Play, View::Edit, View::Sample, View::Fx];

pub struct Chop {
    sound: Sound,
    pub s: Arc<Shared>,
    bus: Arc<AudioBus>,
    source: usize,
    inbox: Option<Arc<NoteInboxRef>>,
    pub view: usize,
    pub pad: usize,
    row: usize,
    chop: usize,
    sensitivity: f32,
    prev: [bool; 16],
    library: Vec<PathBuf>,
    lib_index: usize,
    /// Where a recording goes: the pad it fills (or starts filling).
    rec_target: usize,
    message: (String, u32),
    dir: Option<PathBuf>,
    media: PathBuf,
    poll: u32,
}

impl Chop {
    pub fn new(sound: Sound, bus: Arc<AudioBus>, notes: Option<Arc<NoteBus>>, dir: Option<PathBuf>, media: PathBuf) -> Chop {
        let s = Arc::new(Shared {
            pads: Mutex::new(vec![Pad::default(); PADS]),
            fx: Mutex::new([FxSlot { kind: 0, a: 0.5, b: 0.5 }, FxSlot { kind: 0, a: 0.5, b: 0.5 }]),
            tempo: AtomicF32::new(90.0),
            live: Mutex::new(Vec::new()),
            recording: AtomicUsize::new(0),
            rec_len: AtomicUsize::new(0),
            finished: Mutex::new(None),
            spare: Mutex::new(Some(Vec::with_capacity(MAX_REC))),
            source: Mutex::new(None),
            level: AtomicF32::new(0.0),
            playhead: std::array::from_fn(|_| AtomicF32::new(-1.0)),
        });
        sound.set_reverb(0.0);
        let names = bus.names();
        let source = names.iter().position(|n| n == "Hardware input").or_else(|| names.iter().position(|n| n.contains("Audio In"))).unwrap_or(NO_SOURCE);
        let inbox = notes.and_then(|b| b.register_instrument("chop", NAME));
        let mut c = Chop { sound, s, bus, source, inbox, view: 0, pad: 0, row: 0, chop: 0, sensitivity: 0.5, prev: [false; 16], library: Vec::new(), lib_index: 0, rec_target: 0, message: (String::new(), 0), dir, media, poll: 0 };
        c.connect();
        c.load_bank(true);
        c
    }

    fn connect(&mut self) {
        let b = if self.source == NO_SOURCE { None } else { self.bus.get(self.source) };
        *self.s.source.lock().unwrap() = b;
    }

    fn flash(&mut self, m: impl Into<String>) {
        self.message = (m.into(), 180);
    }

    fn next_empty(&self, from: usize) -> usize {
        let p = self.s.pads.lock().unwrap();
        (0..PADS).map(|k| (from + k) % PADS).find(|&k| p[k].data.is_none()).unwrap_or(from)
    }

    /// Starts or stops recording: 1 = the source, 2 = Chop's own output.
    pub fn record(&mut self, what: usize) {
        if self.s.recording.load(Ordering::Relaxed) != 0 {
            self.s.recording.store(0, Ordering::Relaxed);
            return;
        }
        if self.s.spare.lock().unwrap().is_none() {
            *self.s.spare.lock().unwrap() = Some(Vec::with_capacity(MAX_REC));
        }
        self.rec_target = if what == 2 { self.next_empty(self.pad) } else { self.pad };
        self.s.rec_len.store(0, Ordering::Relaxed);
        self.s.recording.store(what, Ordering::Relaxed);
    }

    /// Takes a finished recording and puts it on the pads.
    fn collect(&mut self) {
        let Some(data) = self.s.finished.lock().unwrap().take() else { return };
        // The audio thread gets a new spare for next time.
        *self.s.spare.lock().unwrap() = Some(Vec::with_capacity(MAX_REC));
        if data.len() < 480 {
            self.flash("Too short to keep");
            return;
        }
        let resampled = self.rec_target != self.pad || self.s.recording.load(Ordering::Relaxed) == 2;
        let how = if resampled { 0 } else { self.chop };
        self.place(&data, how);
    }

    /// Puts `data` on the pads from the recording target, sliced `how`.
    pub fn place(&mut self, data: &[f32], how: usize) {
        let data = trim(data);
        let slices = slice_points(&data, how, self.sensitivity);
        let mut pads = self.s.pads.lock().unwrap();
        for (k, (a, b)) in slices.iter().enumerate() {
            let pad = (self.rec_target + k) % PADS;
            pads[pad] = Pad { data: Some(Arc::new(data[*a..*b].to_vec())), set: PadSet { name: format!("take {}", pad + 1), ..PadSet::default() } };
        }
        drop(pads);
        self.flash(format!("{} on pad{} {}", if slices.len() == 1 { "Recorded".to_string() } else { format!("{} slices", slices.len()) }, if slices.len() == 1 { "" } else { "s from" }, self.rec_target + 1));
    }

    fn save_bank(&mut self) {
        let Some(dir) = self.dir.clone() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let pads = self.s.pads.lock().unwrap().clone();
        let mut sets = Vec::new();
        for (k, p) in pads.iter().enumerate() {
            let path = dir.join(format!("pad_{:02}.wav", k + 1));
            if let Some(d) = &p.data {
                let _ = write_wav(&path, d);
            } else {
                let _ = std::fs::remove_file(&path);
            }
            sets.push(p.set.clone());
        }
        let fx = *self.s.fx.lock().unwrap();
        let j = serde_json::json!({ "pads": sets, "fx": fx, "tempo": self.s.tempo.get() });
        let _ = std::fs::write(dir.join("bank.json"), serde_json::to_string_pretty(&j).unwrap_or_default());
        self.flash("Bank saved");
    }

    fn load_bank(&mut self, quiet: bool) {
        let Some(dir) = self.dir.clone() else { return };
        let Ok(j) = std::fs::read_to_string(dir.join("bank.json")) else {
            if !quiet {
                self.flash("No saved bank");
            }
            return;
        };
        let v: serde_json::Value = serde_json::from_str(&j).unwrap_or_default();
        let sets: Vec<PadSet> = serde_json::from_value(v["pads"].clone()).unwrap_or_default();
        let mut pads = self.s.pads.lock().unwrap();
        for k in 0..PADS {
            let data = read_wav(&dir.join(format!("pad_{:02}.wav", k + 1))).ok().map(Arc::new);
            pads[k] = Pad { data, set: sets.get(k).cloned().unwrap_or_default() };
        }
        drop(pads);
        if let Ok(fx) = serde_json::from_value::<[FxSlot; 2]>(v["fx"].clone()) {
            *self.s.fx.lock().unwrap() = fx;
        }
        if let Some(t) = v["tempo"].as_f64() {
            self.s.tempo.set(t as f32);
        }
        if !quiet {
            self.flash("Bank loaded");
        }
    }

    fn rows(&self) -> Vec<(&'static str, String)> {
        let pads = self.s.pads.lock().unwrap();
        let p = &pads[self.pad];
        let fx = *self.s.fx.lock().unwrap();
        match VIEWS[self.view] {
            View::Play => vec![],
            View::Edit => vec![
                ("Start", format!("{:.1}%", p.set.start * 100.0)),
                ("End", format!("{:.1}%", p.set.end * 100.0)),
                ("Pitch", format!("{:+.0} st", p.set.pitch)),
                ("Gain", format!("{:.0}%", p.set.gain * 100.0)),
                ("Mode", format!("{:?}", p.set.mode)),
                ("Reverse", if p.set.reverse { "on".into() } else { "off".into() }),
                ("Library", self.library.get(self.lib_index).map_or("(media/ has no WAVs)".into(), |f| f.file_name().map_or(String::new(), |n| n.to_string_lossy().to_string()))),
                ("Clear pad", "< >".into()),
            ],
            View::Sample => vec![
                ("Source", self.bus.source_name(self.source)),
                ("Chop", CHOPS[self.chop].into()),
                ("Hit sensitivity", format!("{:.0}%", self.sensitivity * 100.0)),
                ("Save bank", "< >".into()),
                ("Load bank", "< >".into()),
            ],
            View::Fx => vec![
                ("FX 1", FX_NAMES[fx[0].kind].into()),
                (FX_PARAMS[fx[0].kind][0], format!("{:.0}", fx[0].a * 100.0)),
                (FX_PARAMS[fx[0].kind][1], format!("{:.0}", fx[0].b * 100.0)),
                ("FX 2", FX_NAMES[fx[1].kind].into()),
                (FX_PARAMS[fx[1].kind][0], format!("{:.0}", fx[1].a * 100.0)),
                (FX_PARAMS[fx[1].kind][1], format!("{:.0}", fx[1].b * 100.0)),
                ("Tempo", format!("{:.0} bpm", self.s.tempo.get())),
            ],
        }
    }

    fn edit(&mut self, d: i32) {
        let df = d as f32;
        match VIEWS[self.view] {
            View::Play => {}
            View::Edit => {
                if self.row == 6 {
                    if self.library.is_empty() {
                        let mut v = Vec::new();
                        scan_wavs(&self.media, &mut v, 0);
                        self.library = v;
                    }
                    if !self.library.is_empty() {
                        self.lib_index = (self.lib_index as i32 + d).rem_euclid(self.library.len() as i32) as usize;
                    }
                    return;
                }
                let mut pads = self.s.pads.lock().unwrap();
                let p = &mut pads[self.pad];
                match self.row {
                    0 => p.set.start = (p.set.start + df * 0.005).clamp(0.0, p.set.end - 0.001),
                    1 => p.set.end = (p.set.end + df * 0.005).clamp(p.set.start + 0.001, 1.0),
                    2 => p.set.pitch = (p.set.pitch + df).clamp(-24.0, 24.0),
                    3 => p.set.gain = (p.set.gain + df * 0.05).clamp(0.0, 2.0),
                    4 => p.set.mode = match (p.set.mode, d > 0) {
                        (Mode::OneShot, true) => Mode::Gate,
                        (Mode::Gate, true) => Mode::Loop,
                        (Mode::Loop, true) => Mode::OneShot,
                        (Mode::OneShot, false) => Mode::Loop,
                        (Mode::Gate, false) => Mode::OneShot,
                        (Mode::Loop, false) => Mode::Gate,
                    },
                    5 => p.set.reverse = !p.set.reverse,
                    _ => *p = Pad::default(),
                }
            }
            View::Sample => match self.row {
                0 => {
                    self.source = crate::audio_bus::cycle_source(self.source, d, self.bus.len());
                    self.connect();
                }
                1 => self.chop = (self.chop as i32 + d).rem_euclid(CHOPS.len() as i32) as usize,
                2 => self.sensitivity = (self.sensitivity + df * 0.05).clamp(0.0, 1.0),
                3 => self.save_bank(),
                _ => self.load_bank(false),
            },
            View::Fx => {
                if self.row == 6 {
                    self.s.tempo.set((self.s.tempo.get().round() + df).clamp(40.0, 240.0));
                    return;
                }
                let mut fx = self.s.fx.lock().unwrap();
                let slot = &mut fx[self.row / 3];
                match self.row % 3 {
                    0 => slot.kind = (slot.kind as i32 + d).rem_euclid(FX_NAMES.len() as i32) as usize,
                    1 => slot.a = (slot.a + df * 0.02).clamp(0.0, 1.0),
                    _ => slot.b = (slot.b + df * 0.02).clamp(0.0, 1.0),
                }
            }
        }
    }

    fn select(&mut self) {
        match VIEWS[self.view] {
            View::Play | View::Fx => self.record(2),
            View::Sample => self.record(1),
            View::Edit => {
                if self.row == 6 {
                    if let Some(f) = self.library.get(self.lib_index).cloned() {
                        match read_wav(&f) {
                            Ok(d) => {
                                let mut pads = self.s.pads.lock().unwrap();
                                pads[self.pad] = Pad { data: Some(Arc::new(d)), set: PadSet { name: f.file_stem().map_or(String::new(), |n| n.to_string_lossy().to_string()), ..PadSet::default() } };
                                drop(pads);
                                self.flash("Loaded");
                            }
                            Err(e) => self.flash(e),
                        }
                    }
                } else {
                    self.s.live.lock().unwrap().push((self.pad, 1.0, true));
                }
            }
        }
    }
}

/// Removes silence from the start of a recording (the time before the
/// first sound), keeping 5 ms of it.
fn trim(data: &[f32]) -> Vec<f32> {
    let peak = data.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    let start = data.iter().position(|x| x.abs() > peak * 0.05).unwrap_or(0).saturating_sub(240);
    data[start..].to_vec()
}

impl App for Chop {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![("View".into(), format!("{:?}", VIEWS[self.view]), false), ("Pad".into(), format!("{}", self.pad + 1), false)]
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(match VIEWS[self.view] {
            View::Play => "PLAY",
            View::Edit => "EDIT",
            View::Sample => "SAMPLE",
            View::Fx => "FX",
        })
    }
    fn toggle_grid_mode(&mut self) {
        self.view = (self.view + 1) % VIEWS.len();
        self.row = 0;
    }
    fn running(&self) -> Option<bool> {
        Some(self.s.recording.load(Ordering::Relaxed) != 0)
    }
    fn toggle_running(&mut self) {
        self.select();
    }
    fn transport_action(&self) -> Option<&'static str> {
        Some(if self.s.recording.load(Ordering::Relaxed) != 0 { "STOP REC" } else if VIEWS[self.view] == View::Sample { "RECORD" } else { "RESAMPLE" })
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let pads = self.s.pads.lock().unwrap();
        std::array::from_fn(|i| {
            if i == self.pad {
                PadColor::Red
            } else if self.s.playhead[i].get() >= 0.0 {
                PadColor::Yellow
            } else if pads[i].data.is_some() {
                PadColor::Green
            } else {
                PadColor::Off
            }
        })
    }

    fn tick(&mut self, input: &Input) {
        self.poll += 1;
        if self.poll % 15 == 0 && self.source != NO_SOURCE {
            let _ = self.bus.get(self.source);
        }
        for p in 0..16 {
            if input.grid[p] && !self.prev[p] {
                self.pad = p;
                self.s.live.lock().unwrap().push((p, 1.0, true));
            } else if !input.grid[p] && self.prev[p] {
                self.s.live.lock().unwrap().push((p, 0.0, false));
            }
        }
        self.prev = input.grid;
        let n = self.rows().len();
        if input.navigation_steps != 0 && n > 0 {
            self.row = (self.row as i32 + input.navigation_steps).clamp(0, n as i32 - 1) as usize;
        }
        if input.knob2 != 0 {
            if VIEWS[self.view] == View::Play {
                // Left/right picks the pad.
                self.pad = (self.pad as i32 + input.knob2.signum()).rem_euclid(PADS as i32) as usize;
            } else {
                self.edit(input.knob2.signum());
            }
        }
        if input.knob1_press {
            self.select();
        }
        if self.s.recording.load(Ordering::Relaxed) == 0 {
            self.collect();
        } else if self.s.rec_len.load(Ordering::Relaxed) >= MAX_REC {
            self.s.recording.store(0, Ordering::Relaxed);
        }
        if self.message.1 > 0 {
            self.message.1 -= 1;
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = Rgb565::new(5, 4, 3);
        let panel = Rgb565::new(10, 9, 6);
        let dim = Rgb565::new(22, 26, 20);
        let ink = kit::rgb(250, 200, 90);
        kit::clear(fb, bg);
        kit::rect(fb, 0, 0, 640, 30, panel);
        kit::text(fb, NAME, 12, 7, Size2::Medium, kit::WHITE, -1);
        for (i, v) in ["PLAY", "EDIT", "SAMPLE", "FX"].iter().enumerate() {
            let x = 70 + i as i32 * 64;
            let sel = i == self.view;
            kit::round_rect(fb, x, 6, 60, 18, 6, if sel { ink } else { Rgb565::new(13, 12, 9) });
            kit::text(fb, v, x + 30, 9, Size2::Small, if sel { kit::BLACK } else { dim }, 0);
        }
        let rec = self.s.recording.load(Ordering::Relaxed);
        let status = match rec {
            1 => format!("RECORDING {:.1}s", self.s.rec_len.load(Ordering::Relaxed) as f32 / SR),
            2 => format!("RESAMPLING {:.1}s", self.s.rec_len.load(Ordering::Relaxed) as f32 / SR),
            _ => {
                let fx = *self.s.fx.lock().unwrap();
                format!("{} > {}", FX_NAMES[fx[0].kind], FX_NAMES[fx[1].kind])
            }
        };
        kit::text(fb, &status, 630, 9, Size2::Small, if rec > 0 { kit::rgb(250, 90, 90) } else { dim }, 1);
        let pads = self.s.pads.lock().unwrap().clone();
        // The pads.
        for p in 0..PADS {
            let x = 8 + (p as i32 % 4) * 78;
            let y = 38 + (p as i32 / 4) * 50;
            let has = pads[p].data.is_some();
            let head = self.s.playhead[p].get();
            let c = if head >= 0.0 { ink } else if has { kit::blend(panel, ink, 0.35) } else { panel };
            kit::round_rect(fb, x, y, 74, 46, 6, c);
            if p == self.pad {
                kit::outline(fb, x - 2, y - 2, 78, 50, 7, 2, kit::WHITE);
            }
            kit::text(fb, &format!("{}", p + 1), x + 5, y + 3, Size2::Small, if head >= 0.0 { kit::BLACK } else { dim }, -1);
            if has {
                let n: String = pads[p].set.name.chars().take(11).collect();
                kit::text(fb, &n, x + 37, y + 26, Size2::Small, if head >= 0.0 { kit::BLACK } else { kit::WHITE }, 0);
            }
        }
        // The picked pad's waveform, with its start, end and playhead.
        let (wx, wy, ww, wh) = (330, 38, 302, 90);
        kit::round_rect(fb, wx, wy, ww, wh, 8, panel);
        if let Some(d) = &pads[self.pad].data {
            let n = d.len();
            let mid = wy + wh / 2;
            for i in 0..ww - 8 {
                let a = n * i as usize / (ww - 8) as usize;
                let b = (n * (i as usize + 1) / (ww - 8) as usize).max(a + 1).min(n);
                let peak = d[a..b].iter().fold(0.0f32, |m, x| m.max(x.abs()));
                let h = (peak * (wh / 2 - 6) as f32) as i32;
                let t = i as f32 / (ww - 8) as f32;
                let inside = t >= pads[self.pad].set.start && t <= pads[self.pad].set.end;
                kit::rect(fb, wx + 4 + i, mid - h, 1, h * 2 + 1, if inside { ink } else { dim });
            }
            let head = self.s.playhead[self.pad].get();
            if head >= 0.0 {
                let st = &pads[self.pad].set;
                let x = wx + 4 + ((st.start + (st.end - st.start) * head) * (ww - 8) as f32) as i32;
                kit::rect(fb, x, wy + 4, 2, wh - 8, kit::WHITE);
            }
            kit::text(fb, &format!("{:.2} s", n as f32 / SR), wx + ww - 8, wy + 4, Size2::Small, dim, 1);
        } else {
            kit::text(fb, "empty pad", wx + ww / 2, wy + wh / 2 - 6, Size2::Small, dim, 0);
        }
        // The view's rows.
        let rows = self.rows();
        for (i, (l, v)) in rows.iter().enumerate() {
            let y = 136 + i as i32 * 24;
            let sel = i == self.row;
            kit::round_rect(fb, 330, y, 302, 21, 5, if sel { kit::blend(panel, kit::WHITE, 0.15) } else { panel });
            kit::text(fb, l, 338, y + 5, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
            let v: String = v.chars().take(26).collect();
            kit::text(fb, &v, 624, y + 5, Size2::Small, ink, 1);
        }
        let help = match VIEWS[self.view] {
            View::Play if self.s.pads.lock().unwrap().iter().all(|p| p.data.is_none()) => "No sounds yet. F2 to SAMPLE to record the input or load a file from your library, then chop it across the pads.",
            View::Play => "Pads play. Left/right picks a pad. SELECT or F3: resample Chop's output (with FX) into the next empty pad.",
            View::Edit => "Up/down: setting. Left/right: change. SELECT plays the pad (or loads the library file on the Library row).",
            View::Sample => "SELECT or F3: record the source into the picked pad, chopped as set. 'at hits' cuts at every hit.",
            View::Fx => "Two effects in series on everything Chop plays. SELECT or F3: resample.",
        };
        kit::paragraph(fb, help, 8, 246, 316, Size2::Small, dim);
        let lvl = self.s.level.get();
        kit::round_rect(fb, 8, 230, (316.0 * lvl.min(1.0)) as i32 + 3, 5, 2, if rec == 1 { kit::rgb(250, 90, 90) } else { ink });
        if self.message.1 > 0 {
            kit::round_rect(fb, 330, 306, 302, 24, 6, kit::rgb(120, 100, 40));
            kit::text(fb, &self.message.0, 481, 312, Size2::Small, kit::WHITE, 0);
        }
        kit::footer(fb, "F2: view   SELECT/F3: record or resample", panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn crate::audio::AudioProcessor>> {
        let engine = Engine { s: Arc::clone(&self.s), voices: (0..PADS).map(|_| None).collect(), fx: [Fx::new(11), Fx::new(23)], rec: None, input: Vec::with_capacity(4096), read: 0, inbox: self.inbox.take(), view: NoteView::default(), strikes: [0; 128], keys: [0; 128], peak: 0.0 };
        Some(self.sound.processor(None, Some(Box::new(engine))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<AudioBus> = ctx.get();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let media = std::env::var_os("PORTAMAX_MEDIA_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("media"));
    let dir = (!cfg!(test)).then(|| root.join("saves/chop"));
    Box::new(Chop::new(Sound::new(NAME, &modbus, &mixer, &bus), Arc::clone(&bus), ctx.try_get(), dir, media))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, pitch_hz, render};

    fn tone(hz: f32, secs: f32) -> Vec<f32> {
        (0..(secs * SR) as usize).map(|i| (i as f32 * std::f32::consts::TAU * hz / SR).sin() * 0.5).collect()
    }

    fn chop_with_input() -> (Chop, Arc<Mutex<Vec<f32>>>) {
        let bus = Arc::new(AudioBus::new());
        let input = bus.register("Hardware input");
        (Chop::new(Sound::detached(), bus, None, None, PathBuf::from("/nonexistent")), input)
    }

    #[test]
    fn recording_the_input_fills_the_pad_and_it_plays_back_in_tune() {
        let (mut c, input) = chop_with_input();
        let mut p = c.audio_processor().unwrap();
        c.pad = 5;
        c.record(1);
        let t = tone(440.0, 1.0);
        for k in 0..60 {
            *input.lock().unwrap() = t[k * 512..(k + 1) * 512].to_vec();
            render(&mut p, 1);
        }
        c.record(1);
        render(&mut p, 1);
        c.tick(&Input::default());
        let len = c.s.pads.lock().unwrap()[5].data.as_ref().map_or(0, |d| d.len());
        assert!((len as i32 - 30_720).abs() < 1200, "{len}");
        // Up an octave plays at 880 Hz.
        c.s.pads.lock().unwrap()[5].set.pitch = 12.0;
        input.lock().unwrap().fill(0.0);
        c.s.live.lock().unwrap().push((5, 1.0, true));
        let out = render(&mut p, 20);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let hz = pitch_hz(&left[2000..], SR);
        assert!((hz / 880.0 - 1.0).abs() < 0.01, "{hz}");
    }

    #[test]
    fn slicing_equal_and_at_hits() {
        let data = vec![0.1f32; 48_000];
        let s = slice_points(&data, 2, 0.5);
        assert_eq!(s.len(), 8);
        assert_eq!(s[1], (6000, 12000));
        // Four clicks 0.25 s apart with decays: four slices, each at a hit.
        let mut beat = vec![0.0f32; 48_000];
        for k in 0..4 {
            for i in 0..3000 {
                beat[k * 12_000 + 2000 + i] = (i as f32 * 0.3).sin() * (-(i as f32) / 600.0).exp();
            }
        }
        let s = slice_points(&beat, 4, 0.5);
        assert_eq!(s.len(), 4, "{s:?}");
        for (k, (a, _)) in s.iter().enumerate() {
            assert!((*a as i32 - (k as i32 * 12_000 + 2000)).abs() <= 240, "{k}: {a}");
        }
    }

    #[test]
    fn resampling_captures_the_effects() {
        let (mut c, _input) = chop_with_input();
        c.s.pads.lock().unwrap()[0] = Pad { data: Some(Arc::new(tone(220.0, 0.5))), set: PadSet::default() };
        // Lo-fi at 4 bits: the resampled pad has few distinct levels.
        *c.s.fx.lock().unwrap() = [FxSlot { kind: 2, a: 1.0, b: 0.0 }, FxSlot { kind: 0, a: 0.5, b: 0.5 }];
        let mut p = c.audio_processor().unwrap();
        c.pad = 0;
        c.record(2);
        render(&mut p, 1);
        c.s.live.lock().unwrap().push((0, 1.0, true));
        render(&mut p, 30);
        c.record(2);
        render(&mut p, 1);
        c.tick(&Input::default());
        let pads = c.s.pads.lock().unwrap();
        let d = pads[1].data.as_ref().expect("resampled into the next empty pad, 2");
        let mut levels: Vec<i32> = d.iter().map(|x| (x * 1000.0).round() as i32).collect();
        levels.sort();
        levels.dedup();
        assert!(levels.len() < 40, "crushed to a few levels: {}", levels.len());
    }

    #[test]
    fn every_effect_is_bounded_and_passes_sound() {
        for kind in 0..FX_NAMES.len() {
            let mut fx = Fx::new(3);
            let slot = FxSlot { kind, a: 0.7, b: 0.6 };
            let t = tone(330.0, 0.5);
            let out: Vec<(f32, f32)> = t.iter().map(|&x| fx.tick(slot, 120.0, (x, x), SR)).collect();
            let e: f32 = out.iter().map(|(l, r)| l * l + r * r).sum::<f32>() / out.len() as f32;
            assert!(e > 1e-4, "{}: silent", FX_NAMES[kind]);
            assert!(out.iter().all(|(l, r)| l.is_finite() && l.abs() < 4.0 && r.abs() < 4.0), "{}", FX_NAMES[kind]);
        }
    }

    #[test]
    fn notes_play_pads_and_a_bank_saves() {
        let bus = Arc::new(NoteBus::new());
        let dir = std::env::temp_dir().join(format!("portamax-chop-{}", std::process::id()));
        let mut c = Chop::new(Sound::detached(), Arc::new(AudioBus::new()), Some(Arc::clone(&bus)), Some(dir.clone()), PathBuf::from("/nonexistent"));
        c.s.pads.lock().unwrap()[2] = Pad { data: Some(Arc::new(tone(500.0, 0.3))), set: PadSet { pitch: 3.0, ..PadSet::default() } };
        let mut p = c.audio_processor().unwrap();
        let inbox = bus.inbox_ref(bus.instrument_index(NAME).unwrap());
        inbox.note_on(38, 100);
        render(&mut p, 2);
        assert!(energy(&render(&mut p, 5)) > 1e-4);
        c.save_bank();
        let mut d = Chop::new(Sound::detached(), Arc::new(AudioBus::new()), None, Some(dir.clone()), PathBuf::from("/nonexistent"));
        d.load_bank(false);
        let pads = d.s.pads.lock().unwrap();
        assert_eq!(pads[2].set.pitch, 3.0);
        assert!((pads[2].data.as_ref().unwrap().len() as i32 - 14_400).abs() < 2);
        drop(pads);
        let _ = std::fs::remove_dir_all(dir);
        let mut fb = FrameBuffer::new();
        for v in 0..4 {
            d.view = v;
            d.draw(&mut fb);
        }
    }
}
