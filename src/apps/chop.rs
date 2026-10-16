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
//!   gate, loop), reverse, and loading audio from the library (media/ and samples/).
//! - SAMPLE: the source, how to chop (whole, 4, 8, 16 or at transients),
//!   and SELECT to record; saving and loading the bank.
//! - SLICE: choose a WAV, auto-cut a chosen number of one-shots or add/drag manual markers.
//! - LOOP: record your playing (post-FX, up to 32 seconds); stop to repeat,
//!   pause/resume, clear, or save the performance loop with the bank.
//! - FX: the two effect slots and their two controls each, and the tempo
//!   the delay follows.
//!
//! Notes from MIDI or other apps play the pads (C2 = pad 1), so Session
//! can sequence it.

#[path = "sample_drum_stretch.rs"]
mod sync_stretch;
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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Chop";
pub const PADS: usize = 128;
const BANK_PADS: usize = 16;
const SR: f32 = 48_000.0;
/// The longest recording: 32 seconds.
const MAX_REC: usize = 48_000 * 32;
const FX_NAMES: [&str; 9] = [
    "Off", "Vinyl", "Lo-fi", "Filter", "Delay", "Reverb", "Compress", "Isolator", "Tape",
];
const FX_PARAMS: [[&str; 2]; 9] = [
    ["-", "-"],
    ["Age", "Noise"],
    ["Bits", "Rate"],
    ["Cutoff", "Res"],
    ["Time", "Feedback"],
    ["Size", "Mix"],
    ["Amount", "Punch"],
    ["Low", "High"],
    ["Drive", "Wow"],
];
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
    /// Beats in the entire source file; trims retain their proportional duration.
    pub beats: f32,
}

impl Default for PadSet {
    fn default() -> Self {
        PadSet {
            name: String::new(),
            start: 0.0,
            end: 1.0,
            pitch: 0.0,
            gain: 1.0,
            mode: Mode::OneShot,
            reverse: false,
            beats: 0.,
        }
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
    pub beat_playing: AtomicBool,
    pub beat_pattern: AtomicUsize,
    pub sync: AtomicBool,
    pub fit_samples: AtomicBool,
    pub grid: AtomicUsize,
    pub queued_pad: AtomicUsize,
    pub backing_beats: AtomicF32,
    pub beat_position: AtomicF32,
    pub stop_on_bar: AtomicBool,
    pub loop_ending: AtomicBool,
    pub loop_beats: AtomicF32,
    pub loop_origin: AtomicF32,
    pub backing_clip: Mutex<Option<Arc<Vec<f32>>>>,
    pub clear_voices: AtomicBool,
    /// Pad presses from the UI: (pad, velocity, on).
    pub live: Mutex<Vec<(usize, f32, bool)>>,
    /// 0 none, 1 recording the input, 2 resampling the output.
    pub recording: AtomicUsize,
    pub loop_armed: AtomicBool,
    pub rec_len: AtomicUsize,
    /// A finished recording, handed from the audio thread.
    pub finished: Mutex<Option<Vec<f32>>>,
    /// An empty buffer for the next recording, so the audio thread never
    /// allocates.
    pub spare: Mutex<Option<Vec<f32>>>,
    pub source: Mutex<Option<Arc<Mutex<Vec<f32>>>>>,
    pub level: AtomicF32,
    pub loop_clip: Mutex<Option<Arc<Vec<f32>>>>,
    pub loop_playing: AtomicBool,
    pub loop_head: AtomicF32,
    pub preview_clip: Mutex<Option<Arc<Vec<f32>>>>,
    pub preview_playing: AtomicBool,
    pub preview_head: AtomicF32,
    pub preview_generation: AtomicUsize,
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
    beats: f32,
    stretch: sync_stretch::Stretcher,
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
        Fx {
            lp: [Svf::default(); 2],
            noise: Noise::new(seed),
            hold: (0.0, 0.0),
            hold_n: 0,
            delay: vec![[0.0; 2]; 96_000],
            dw: 0,
            dlp: [0.0; 2],
            verb: StereoFx::new(),
            env: 0.0,
            wow: 0.0,
            tape: vec![[0.0; 2]; 2048],
            tw: 0,
            iso: [Svf::default(); 2],
            iso_hi: [Svf::default(); 2],
        }
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
                let w = self
                    .read_tape(40.0 + a * 60.0 * (1.0 + (self.wow * std::f32::consts::TAU).sin()));
                let cutoff = 16_000.0 * (1.0 - a * 0.8);
                let l = self.lp[0].tick(w[0], cutoff, 0.7, sr).lp;
                let r = self.lp[1].tick(w[1], cutoff, 0.7, sr).lp;
                let n = self.noise.next();
                let crackle = if self.noise.next() > 1.0 - a * 0.0015 {
                    n * 0.6
                } else {
                    0.0
                };
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
                    (
                        self.lp[0].tick(x.0, c, res, sr).lp,
                        self.lp[1].tick(x.1, c, res, sr).lp,
                    )
                } else if a > 0.52 {
                    let c = 20.0 * (8_000f32 / 20.0).powf((a - 0.52) / 0.48);
                    (
                        self.lp[0].tick(x.0, c, res, sr).hp,
                        self.lp[1].tick(x.1, c, res, sr).hp,
                    )
                } else {
                    x
                }
            }
            4 => {
                // Delay in tempo: 1/16 to 1/2 notes, ping-ponging.
                let notes = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0];
                let beats = notes[((a * 5.99) as usize).min(5)];
                let d =
                    ((beats * 60.0 / tempo.max(30.0) * sr) as usize).clamp(1, self.delay.len() - 1);
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
                let s = FxSettings {
                    reverb_size: a,
                    reverb_mix: b,
                    ..Default::default()
                };
                let (l, r) = self.verb.tick((x.0 + x.1) * 0.5, &s, sr);
                (l + (x.0 - x.1) * 0.5, r - (x.0 - x.1) * 0.5)
            }
            6 => {
                // A compressor with a fast attack: Amount lowers the threshold, Punch adds make-up.
                let lvl = x.0.abs().max(x.1.abs());
                let k = if lvl > self.env {
                    1.0 - (-1.0 / (0.003 * sr)).exp()
                } else {
                    1.0 - (-1.0 / (0.12 * sr)).exp()
                };
                self.env += (lvl - self.env) * k;
                let th = 10f32.powf((-6.0 - a * 30.0) / 20.0);
                let gain = if self.env > th {
                    (th / self.env).powf(0.75)
                } else {
                    1.0
                };
                let make = 1.0 + b * 3.0;
                ((x.0 * gain * make).tanh(), (x.1 * gain * make).tanh())
            }
            7 => {
                // Isolator: low and high bands each from killed (0) to +6 dB (1); 0.66 is flat.
                let gain = |v: f32| {
                    if v < 0.66 {
                        v / 0.66
                    } else {
                        1.0 + (v - 0.66) / 0.34
                    }
                };
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
    loop_clip: Option<Arc<Vec<f32>>>,
    loop_pos: f64,
    preview_clip: Option<Arc<Vec<f32>>>,
    preview_pos: f64,
    preview_generation: usize,
    beat_phase: f64,
    beat_was_playing: bool,
    beat_step: usize,
    beat_age: f32,
    beat_noise: Noise,
    backing_clip: Option<Arc<Vec<f32>>>,
    backing_pos: f64,
    master_beats: f64,
    pending: Option<(usize, Voice, f64)>,
    record_start: f64,
    record_end: Option<f64>,
    backing_stretch: sync_stretch::Stretcher,
    loop_stretch: sync_stretch::Stretcher,
}

impl Engine {
    fn start(&mut self, pad: usize, vel: f32) {
        let p = match self.s.pads.try_lock() {
            Ok(p) => p[pad].clone(),
            Err(_) => return,
        };
        let Some(data) = p.data else { return };
        let n = data.len() as f64;
        let (a, b) = (
            p.set.start.min(p.set.end) as f64 * n,
            p.set.end.max(p.set.start) as f64 * n,
        );
        if b - a < 2.0 {
            return;
        }
        let rate = 2f64.powf(p.set.pitch as f64 / 12.0);
        let voice = Voice {
            pos: if p.set.reverse { b - 1.0 } else { a },
            data,
            rate,
            start: a,
            end: b,
            gain: p.set.gain * vel,
            mode: p.set.mode,
            held: true,
            rev: p.set.reverse,
            env: 0.0,
            active: true,
            beats: p.set.beats,
            stretch: sync_stretch::Stretcher::default(),
        };
        let grid = self.s.grid.load(Ordering::Relaxed);
        if self.s.sync.load(Ordering::Relaxed)
            && grid > 0
            && self.s.beat_playing.load(Ordering::Relaxed)
            && self.beat_was_playing
        {
            let step = [0., 0.25, 0.5, 1.][grid.min(3)];
            let due = ((self.master_beats / step) - 1e-9).ceil() * step;
            if due > self.master_beats + 1e-9 {
                self.s.queued_pad.store(pad + 1, Ordering::Relaxed);
                self.pending = Some((pad, voice, due));
                return;
            }
        }
        self.pending = None;
        self.activate(pad, voice);
    }

    fn activate(&mut self, pad: usize, voice: Voice) {
        self.s.queued_pad.store(0, Ordering::Relaxed);
        if self.s.sync.load(Ordering::Relaxed) && !self.s.beat_playing.load(Ordering::Relaxed) {
            self.master_beats = 0.;
            self.beat_phase = 0.;
            self.beat_step = 0;
            self.beat_age = 0.;
            self.backing_pos = 0.;
            self.backing_stretch.reset();
            self.s.beat_playing.store(true, Ordering::Relaxed);
        }
        if self.s.loop_armed.swap(false, Ordering::AcqRel) {
            self.s.recording.store(3, Ordering::Relaxed);
            self.record_start = self.master_beats;
            self.record_end = None;
            self.s.loop_origin.set(self.master_beats as f32);
            self.s.loop_beats.set(0.);
        }
        for (index, slot) in self.voices.iter_mut().enumerate() {
            *slot = None;
            self.s.playhead[index].set(-1.);
        }
        self.s.preview_playing.store(false, Ordering::Release);
        self.voices[pad] = Some(voice);
    }

    fn stop(&mut self, pad: usize) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(p, v, _)| *p == pad && v.mode != Mode::OneShot)
        {
            self.pending = None;
            self.s.queued_pad.store(0, Ordering::Relaxed);
        }
        if let Some(v) = self.voices[pad].as_mut() {
            v.held = false;
        }
    }
}

impl Extra for Engine {
    fn block(&mut self, frames: usize, _sr: f32) {
        if self.s.clear_voices.swap(false, Ordering::AcqRel) {
            self.pending = None;
            self.s.queued_pad.store(0, Ordering::Relaxed);
            for (i, voice) in self.voices.iter_mut().enumerate() {
                *voice = None;
                self.s.playhead[i].set(-1.);
            }
        }

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
        let shared = self.s.clone();
        if let Ok(mut live) = shared.live.try_lock() {
            for (pad, vel, on) in live.drain(..) {
                if on {
                    self.start(pad, vel);
                } else {
                    self.stop(pad);
                }
            }
        }
        if let Some(inbox) = self.inbox.clone() {
            inbox.poll(&mut self.view);
            for k in 0..PADS.min(92) {
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
        if let Ok(clip) = self.s.loop_clip.try_lock() {
            let changed = match (&self.loop_clip, &*clip) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            };
            if changed {
                self.loop_clip = clip.clone();
                self.loop_pos = 0.;
                self.loop_stretch.reset();
            }
        }
        if let Ok(clip) = self.s.backing_clip.try_lock() {
            let changed = match (&self.backing_clip, &*clip) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            };
            if changed {
                self.backing_clip = clip.clone();
                self.backing_pos = 0.;
                self.backing_stretch.reset();
            }
        }
        let generation = self.s.preview_generation.load(Ordering::Acquire);
        if generation != self.preview_generation {
            if let Ok(clip) = self.s.preview_clip.try_lock() {
                self.preview_clip = clip.clone();
                self.preview_pos = 0.;
                self.preview_generation = generation;
            }
        }
        // Recording starts and stops.
        let want = self.s.recording.load(Ordering::Relaxed);
        let armed = self.s.loop_armed.load(Ordering::Acquire);
        if (want != 0 || armed) && self.rec.is_none() {
            if let Ok(mut sp) = self.s.spare.try_lock() {
                if let Some(mut b) = sp.take() {
                    b.clear();
                    self.rec = Some(b);
                }
            }
        } else if want == 0 && !armed && self.rec.is_some() {
            if let Ok(mut f) = self.s.finished.try_lock() {
                *f = self.rec.take();
            }
        }
    }

    fn frame(&mut self, sr: f32) -> (f32, f32) {
        let sync = self.s.sync.load(Ordering::Relaxed);
        let bpm = self.s.tempo.get().clamp(40., 240.);
        let beat_on = self.s.beat_playing.load(Ordering::Relaxed);
        if beat_on && !self.beat_was_playing {
            self.master_beats = 0.;
            self.beat_phase = 0.;
            self.beat_step = 0;
            self.beat_age = 0.;
            self.backing_pos = 0.;
            self.backing_stretch.reset();
        }
        if sync && !beat_on && self.pending.take().is_some() {
            self.s.queued_pad.store(0, Ordering::Relaxed);
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, _, due)| !sync || self.master_beats + 1e-9 >= *due)
        {
            let (pad, voice, _) = self.pending.take().unwrap();
            self.activate(pad, voice);
        }
        if self.s.stop_on_bar.swap(false, Ordering::AcqRel)
            && self.s.recording.load(Ordering::Relaxed) == 3
        {
            self.record_end = Some(
                self.record_start
                    + (((self.master_beats - self.record_start) / 4.)
                        .ceil()
                        .max(1.))
                        * 4.,
            );
        }
        if sync && self.s.recording.load(Ordering::Relaxed) == 3 {
            // End before the first frame of the next bar, including at the capacity limit.
            let max_bars = (MAX_REC as f64 / (SR as f64 * 60. / bpm as f64 * 4.))
                .floor()
                .max(1.);
            let end = self.record_end.unwrap_or(self.record_start + max_bars * 4.);
            if self.master_beats + 1e-9 >= end {
                self.s.loop_ending.store(false, Ordering::Relaxed);
                self.s.loop_beats.set((end - self.record_start) as f32);
                self.s.recording.store(0, Ordering::Relaxed);
                self.record_end = None;
            }
        }

        let inp = self.input.get(self.read).copied().unwrap_or(0.0);
        self.read += 1;
        let (mut l, mut r) = (0.0, 0.0);
        for (k, slot) in self.voices.iter_mut().enumerate() {
            let Some(v) = slot.as_mut() else { continue };
            let i = v.pos as usize;
            let f = (v.pos - i as f64) as f32;
            let a = v.data.get(i).copied().unwrap_or(0.0);
            let b = v.data.get(i + 1).copied().unwrap_or(a);
            let fitted = sync && self.s.fit_samples.load(Ordering::Relaxed) && v.beats > 0.;
            let step = if fitted {
                v.data.len() as f64 * bpm as f64 / (v.beats as f64 * 60. * sr as f64)
            } else {
                v.rate * SR as f64 / sr as f64
            };
            let x = if fitted {
                v.stretch.render(
                    &v.data,
                    v.pos as f32,
                    (v.rate * SR as f64 / sr as f64) as f32,
                    if v.rev { -1. } else { 1. },
                    v.start as f32,
                    (v.end - 1.) as f32,
                )
            } else {
                a + (b - a) * f
            };
            let target = if v.mode == Mode::Gate && !v.held {
                0.0
            } else {
                1.0
            };
            v.env += (target - v.env) * if target > v.env { 0.3 } else { 0.002 };
            let y = x * v.env * v.gain;
            l += y;
            r += y;
            if v.rev {
                v.pos -= step;
            } else {
                v.pos += step;
            }
            let out = if v.rev {
                v.pos < v.start
            } else {
                v.pos >= if fitted { v.end } else { v.end - 1.0 }
            };
            if out {
                if v.mode == Mode::Loop && v.held {
                    v.pos = v.start + (v.pos - v.start).rem_euclid(v.end - v.start);
                    v.stretch.reset();
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
        let slots = self.s.fx.try_lock().map(|f| *f).unwrap_or(
            [FxSlot {
                kind: 0,
                a: 0.5,
                b: 0.5,
            }; 2],
        );
        let tempo = self.s.tempo.get();
        let mut y = (l, r);
        for k in 0..2 {
            y = self.fx[k].tick(slots[k], tempo, y, sr);
        }
        if let Some(rec) = self.rec.as_mut() {
            if self.s.recording.load(Ordering::Relaxed) != 0 && rec.len() < MAX_REC {
                let v = if self.s.recording.load(Ordering::Relaxed) >= 2 {
                    (y.0 + y.1) * 0.5
                } else {
                    inp
                };
                rec.push(v);
                self.s.rec_len.store(rec.len(), Ordering::Relaxed);
            }
        }
        // The performance loop is post-FX and excluded from a new recording.
        if self.s.loop_playing.load(Ordering::Relaxed) {
            if let Some(clip) = self.loop_clip.as_ref().filter(|c| !c.is_empty()) {
                let beats = self.s.loop_beats.get();
                if sync && beats > 0. {
                    let pos = (self.master_beats - self.s.loop_origin.get() as f64)
                        .rem_euclid(beats as f64)
                        / beats as f64
                        * clip.len() as f64;
                    if pos < self.loop_pos {
                        self.loop_stretch.reset();
                    }
                    self.loop_pos = pos;
                }
                let i = self.loop_pos as usize % clip.len();
                let f = self.loop_pos.fract() as f32;
                let sample = if sync && beats > 0. {
                    self.loop_stretch.render(
                        clip,
                        self.loop_pos as f32,
                        SR / sr,
                        1.,
                        0.,
                        (clip.len() - 1) as f32,
                    )
                } else {
                    clip[i] + (clip[(i + 1) % clip.len()] - clip[i]) * f
                };
                y.0 += sample;
                y.1 += sample;
                if !sync || beats <= 0. {
                    self.loop_pos = (self.loop_pos + SR as f64 / sr as f64) % clip.len() as f64;
                }
                self.s
                    .loop_head
                    .set(self.loop_pos as f32 / clip.len() as f32);
            }
        } else {
            self.s.loop_head.set(-1.);
        }
        // Independent backing track: never choked by pads or baked into their loop.
        let beat_on = self.s.beat_playing.load(Ordering::Acquire);
        if beat_on && self.backing_clip.is_none() {
            let pattern = self.s.beat_pattern.load(Ordering::Relaxed) % 3;
            let step = self.beat_step;
            let t = self.beat_age;
            let kick = match pattern {
                2 => matches!(step, 0 | 3 | 8 | 10),
                _ => step == 0 || step == 8,
            };
            let snare = if pattern == 1 {
                step == 8
            } else {
                step == 4 || step == 12
            };
            let hat = if pattern == 2 { true } else { step % 2 == 0 };
            let noise = self.beat_noise.next();
            let k = if kick {
                (std::f32::consts::TAU * (48. * t + 1.8 * (1. - (-t * 55.).exp()))).sin()
                    * (-t * 22.).exp()
                    * 0.32
            } else {
                0.
            };
            let n = if snare {
                (noise * 0.7 + (t * 180. * std::f32::consts::TAU).sin() * 0.3)
                    * (-t * 32.).exp()
                    * 0.23
            } else {
                0.
            };
            let h = if hat {
                noise * (-t * 160.).exp() * 0.085
            } else {
                0.
            };
            y.0 += k + n + h;
            y.1 += k + n + h;
            self.beat_age += 1. / sr;
            self.beat_phase += self.s.tempo.get() as f64 * 4. / (60. * sr as f64);
            if self.beat_phase >= 1. {
                self.beat_phase -= 1.;
                self.beat_step = (self.beat_step + 1) % 16;
                self.beat_age = 0.;
            }
        }
        if beat_on {
            if let Some(clip) = self.backing_clip.as_ref().filter(|c| !c.is_empty()) {
                let i = self.backing_pos as usize % clip.len();
                let f = self.backing_pos.fract() as f32;
                let v = if sync {
                    self.backing_stretch.render(
                        clip,
                        self.backing_pos as f32,
                        SR / sr,
                        1.,
                        0.,
                        (clip.len() - 1) as f32,
                    )
                } else {
                    clip[i] + (clip[(i + 1) % clip.len()] - clip[i]) * f
                } * 0.65;
                y.0 += v;
                y.1 += v;
                let step = if sync {
                    clip.len() as f64 * bpm as f64
                        / (self.s.backing_beats.get() as f64 * 60. * sr as f64)
                } else {
                    SR as f64 / sr as f64
                };
                self.backing_pos += step;
                if self.backing_pos >= clip.len() as f64 {
                    self.backing_pos %= clip.len() as f64;
                    self.backing_stretch.reset();
                }
            }
        }
        self.beat_was_playing = beat_on;
        if beat_on
            || self.s.loop_playing.load(Ordering::Relaxed)
            || self.s.recording.load(Ordering::Relaxed) == 3
        {
            self.master_beats += bpm as f64 / (60. * sr as f64);
        }
        self.s.beat_position.set(self.master_beats as f32);
        // Audition is independent of pads, effects, recording and loop arming.
        if self.s.preview_playing.load(Ordering::Acquire) {
            if let Some(clip) = self.preview_clip.as_ref() {
                let i = self.preview_pos as usize;
                if i < clip.len() {
                    let f = self.preview_pos.fract() as f32;
                    let sample =
                        clip[i] + (clip.get(i + 1).copied().unwrap_or(clip[i]) - clip[i]) * f;
                    y.0 += sample * 0.8;
                    y.1 += sample * 0.8;
                    self.preview_pos += SR as f64 / sr as f64;
                    self.s
                        .preview_head
                        .set((self.preview_pos as f32 / clip.len() as f32).min(1.));
                } else {
                    self.s.preview_playing.store(false, Ordering::Release);
                    self.s.preview_head.set(-1.);
                }
            }
        } else {
            self.s.preview_head.set(-1.);
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
        return (0..count)
            .map(|k| (n * k / count, n * (k + 1) / count))
            .collect();
    }
    // Transients: a 5 ms frame much louder than the 50 ms before it.
    let hop = 240;
    let energies: Vec<f32> = data
        .chunks(hop)
        .map(|c| c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32)
        .collect();
    let floor = energies.iter().cloned().fold(0.0f32, f32::max) * 0.001;
    let ratio = 2.0 + (1.0 - sensitivity) * 10.0;
    let mut starts = Vec::new();
    let mut last = 0usize;
    for i in 0..energies.len() {
        let before =
            energies[i.saturating_sub(10)..i].iter().sum::<f32>() / 10f32.max(i.min(10) as f32);
        if energies[i] > floor
            && energies[i] > before * ratio
            && (starts.is_empty() || i >= last + 16)
        {
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
    if spec.channels == 0 || spec.sample_rate == 0 {
        return Err("Invalid WAV channel count or sample rate".into());
    }
    let ch = spec.channels as usize;
    let raw: Vec<f32> = if spec.sample_format == hound::SampleFormat::Float {
        r.samples::<f32>().filter_map(Result::ok).collect()
    } else {
        let div = 2f32.powi(spec.bits_per_sample as i32 - 1);
        r.samples::<i32>()
            .filter_map(Result::ok)
            .map(|x| x as f32 / div)
            .collect()
    };
    if raw.is_empty() || raw.iter().any(|x| !x.is_finite()) {
        return Err("WAV contains no usable finite samples".into());
    }
    let mono: Vec<f32> = raw
        .chunks(ch)
        .map(|f| f.iter().sum::<f32>() / ch as f32)
        .take(MAX_REC * 2)
        .collect();
    let rate = spec.sample_rate as f64 / SR as f64;
    if (rate - 1.0).abs() < 1e-6 {
        return Ok(mono.into_iter().take(MAX_REC).collect());
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

fn read_audio(path: &Path) -> Result<Vec<f32>, String> {
    let clip = crate::apps::collection::decode::load(path)?;
    let n = (clip.samples.len() as f64 * SR as f64 / clip.rate as f64) as usize;
    if n > MAX_REC {
        return Err("Choose a sample or backing loop no longer than 32 seconds".into());
    }
    Ok((0..n)
        .map(|i| {
            let pos = i as f64 * clip.rate as f64 / SR as f64;
            let j = pos as usize;
            let f = pos.fract() as f32;
            let a = clip.samples.at(j.min(clip.samples.len() - 1));
            let b = clip.samples.at((j + 1).min(clip.samples.len() - 1));
            (a[0] + a[1] + (b[0] + b[1] - a[0] - a[1]) * f) * 0.5
        })
        .collect())
}

pub fn write_wav(path: &Path, data: &[f32]) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for &x in data {
        w.write_sample((x.clamp(-1.0, 1.0) * 32767.0) as i16)
            .map_err(|e| e.to_string())?;
    }
    w.finalize().map_err(|e| e.to_string())
}

fn guess_source_beats(frames: usize, bpm: f32) -> f32 {
    // Preserve the attack/tail of short one-shots unless the player assigns beats.
    if frames < SR as usize / 2 {
        0.
    } else {
        guess_beats(frames, bpm)
    }
}

fn guess_beats(frames: usize, bpm: f32) -> f32 {
    let beats = frames as f32 / SR * bpm / 60.;
    2f32.powf(beats.max(0.25).log2().round()).clamp(0.25, 128.)
}

fn scan_wavs(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 3 || out.len() > 500 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            scan_wavs(&p, out, depth + 1);
        } else if crate::apps::collection::decode::supported(&p) {
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
    Slice,
    Loop,
}
const VIEWS: [View; 6] = [
    View::Play,
    View::Edit,
    View::Sample,
    View::Fx,
    View::Slice,
    View::Loop,
];
// Keep stable view IDs for touch/controller actions; navigate in screen-tab order.
const VIEW_CYCLE: [usize; 6] = [0, 4, 1, 2, 3, 5];

#[derive(Clone, Copy)]
enum Touch {
    Pad(usize),
    Marker(usize),
    Row { index: usize, last_y: f32 },
    Button,
}

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
    samples: PathBuf,
    poll: u32,
    rec_kind: usize,
    source_sample: Option<Arc<Vec<f32>>>,
    markers: Vec<usize>,
    preview_file: Option<PathBuf>,
    slice_count: usize,
    bank: usize,
    added_banks: usize,
    library_query: String,
    backing_name: String,
    pending_erase: Option<usize>,
    held_targets: [Option<usize>; BANK_PADS],
    touch: Option<Touch>,
    // Retire clips on the UI thread after the engine drops its cached Arc.
    retired_loops: Vec<Arc<Vec<f32>>>,
}

impl Chop {
    pub fn new(
        sound: Sound,
        bus: Arc<AudioBus>,
        notes: Option<Arc<NoteBus>>,
        dir: Option<PathBuf>,
        media: PathBuf,
    ) -> Chop {
        let s = Arc::new(Shared {
            pads: Mutex::new(vec![Pad::default(); PADS]),
            fx: Mutex::new([
                FxSlot {
                    kind: 0,
                    a: 0.5,
                    b: 0.5,
                },
                FxSlot {
                    kind: 0,
                    a: 0.5,
                    b: 0.5,
                },
            ]),
            tempo: AtomicF32::new(90.0),
            beat_playing: AtomicBool::new(false),
            beat_pattern: AtomicUsize::new(0),
            sync: AtomicBool::new(false),
            fit_samples: AtomicBool::new(true),
            grid: AtomicUsize::new(1),
            queued_pad: AtomicUsize::new(0),
            backing_beats: AtomicF32::new(16.),
            beat_position: AtomicF32::new(0.),
            stop_on_bar: AtomicBool::new(false),
            loop_ending: AtomicBool::new(false),
            loop_beats: AtomicF32::new(0.),
            loop_origin: AtomicF32::new(0.),
            backing_clip: Mutex::new(None),
            clear_voices: AtomicBool::new(false),
            live: Mutex::new(Vec::new()),
            recording: AtomicUsize::new(0),
            loop_armed: AtomicBool::new(false),
            rec_len: AtomicUsize::new(0),
            finished: Mutex::new(None),
            spare: Mutex::new(Some(Vec::with_capacity(MAX_REC))),
            source: Mutex::new(None),
            level: AtomicF32::new(0.0),
            loop_clip: Mutex::new(None),
            loop_playing: AtomicBool::new(false),
            loop_head: AtomicF32::new(-1.),
            preview_clip: Mutex::new(None),
            preview_playing: AtomicBool::new(false),
            preview_head: AtomicF32::new(-1.),
            preview_generation: AtomicUsize::new(0),
            playhead: std::array::from_fn(|_| AtomicF32::new(-1.0)),
        });
        sound.set_reverb(0.0);
        let names = bus.names();
        let source = names
            .iter()
            .position(|n| n == "Hardware input")
            .or_else(|| names.iter().position(|n| n.contains("Audio In")))
            .unwrap_or(NO_SOURCE);
        let inbox = notes.and_then(|b| b.register_instrument("chop", NAME));
        let mut c = Chop {
            sound,
            s,
            bus,
            source,
            inbox,
            view: 0,
            pad: 0,
            row: 0,
            chop: 0,
            sensitivity: 0.5,
            prev: [false; 16],
            library: Vec::new(),
            lib_index: 0,
            rec_target: 0,
            message: (String::new(), 0),
            dir,
            samples: media.parent().unwrap_or(Path::new(".")).join("samples"),
            media,
            poll: 0,
            rec_kind: 0,
            source_sample: None,
            markers: Vec::new(),
            preview_file: None,
            slice_count: 16,
            bank: 0,
            added_banks: 1,
            library_query: String::new(),
            backing_name: String::new(),
            pending_erase: None,
            held_targets: [None; BANK_PADS],
            touch: None,
            retired_loops: Vec::new(),
        };
        c.connect();
        c.load_bank(true);
        c
    }

    fn connect(&mut self) {
        let b = if self.source == NO_SOURCE {
            None
        } else {
            self.bus.get(self.source)
        };
        *self.s.source.lock().unwrap() = b;
    }

    fn flash(&mut self, m: impl Into<String>) {
        self.message = (m.into(), 180);
    }

    fn next_empty(&self, from: usize) -> usize {
        let p = self.s.pads.lock().unwrap();
        (0..PADS)
            .map(|k| (from + k) % PADS)
            .find(|&k| p[k].data.is_none())
            .unwrap_or(from)
    }

    /// Starts/stops source (1) or resample (2) recording; performance (3)
    /// arms until the next playable pad hit. A second tap cancels the arm.
    pub fn record(&mut self, what: usize) {
        if self.s.loop_armed.swap(false, Ordering::AcqRel) {
            self.flash("Loop arm cancelled");
            return;
        }
        if self.s.recording.load(Ordering::Relaxed) != 0 {
            if self.s.sync.load(Ordering::Relaxed) && self.s.recording.load(Ordering::Relaxed) == 3
            {
                if self.s.loop_ending.swap(true, Ordering::AcqRel) {
                    return;
                }
                self.s.stop_on_bar.store(true, Ordering::Release);
                self.flash("Ending at the next whole bar");
                return;
            }
            self.s.recording.store(0, Ordering::Relaxed);
            return;
        }
        if self.s.spare.lock().unwrap().is_none() {
            *self.s.spare.lock().unwrap() = Some(Vec::with_capacity(MAX_REC));
        }
        self.rec_target = if what == 2 {
            self.next_empty(self.pad)
        } else {
            self.pad
        };
        self.rec_kind = what;
        self.s.loop_ending.store(false, Ordering::Relaxed);
        self.s.stop_on_bar.store(false, Ordering::Relaxed);
        if what == 3 {
            self.stop_preview();
            self.s.loop_playing.store(false, Ordering::Relaxed);
        }
        self.s.rec_len.store(0, Ordering::Relaxed);
        if what == 3 {
            self.s.loop_armed.store(true, Ordering::Release);
            self.flash("ARMED - hit a pad to start");
        } else {
            self.s.recording.store(what, Ordering::Relaxed);
        }
    }

    /// Takes a finished recording and puts it on the pads.
    fn collect(&mut self) {
        let Some(data) = self.s.finished.lock().unwrap().take() else {
            return;
        };
        // The audio thread gets a new spare for next time.
        *self.s.spare.lock().unwrap() = Some(Vec::with_capacity(MAX_REC));
        if data.is_empty() && self.rec_kind == 3 {
            return;
        }
        if data.len() < 480 {
            self.flash("Too short to keep");
            return;
        }
        if self.rec_kind == 3 {
            let clip = Arc::new(data);
            if let Some(old) = self.s.loop_clip.lock().unwrap().replace(clip) {
                self.retired_loops.push(old);
            }
            self.s.loop_playing.store(true, Ordering::Relaxed);
            self.flash("Performance loop playing");
        } else {
            self.place(&data, if self.rec_kind == 2 { 0 } else { self.chop });
        }
    }

    fn refresh_library(&mut self) {
        self.library.clear();
        scan_wavs(&self.media, &mut self.library, 0);
        scan_wavs(&self.samples, &mut self.library, 0);
        self.library.sort();
        self.library.dedup();
        self.lib_index = self.lib_index.min(self.library.len().saturating_sub(1));
    }

    fn stop_preview(&self) {
        self.s.preview_playing.store(false, Ordering::Release);
    }

    fn audition(&mut self) {
        if self.library.is_empty() {
            self.refresh_library();
        }
        let Some(path) = self.library.get(self.lib_index).cloned() else {
            self.flash("Add WAV files to media/ to preview");
            return;
        };
        if self.preview_file.as_ref() == Some(&path)
            && self.s.preview_playing.load(Ordering::Acquire)
        {
            self.stop_preview();
            return;
        }
        if self.preview_file.as_ref() != Some(&path) {
            match read_audio(&path) {
                Ok(data) => {
                    let clip = Arc::new(data);
                    if let Some(old) = self.s.preview_clip.lock().unwrap().replace(clip) {
                        self.retired_loops.push(old);
                    }
                    self.preview_file = Some(path);
                }
                Err(error) => {
                    self.flash(error);
                    return;
                }
            }
        }
        self.s.preview_generation.fetch_add(1, Ordering::AcqRel);
        self.s.preview_playing.store(true, Ordering::Release);
    }

    fn browse_file(&mut self, delta: i32) {
        if self.library.is_empty() {
            self.refresh_library();
        }
        if !self.library.is_empty() {
            self.stop_preview();
            self.lib_index =
                (self.lib_index as i32 + delta).rem_euclid(self.library.len() as i32) as usize;
        }
    }

    fn native_action(&mut self, command: i32, value: f32) {
        if !matches!(command, 2050..=2053) {
            self.pending_erase = None;
        }
        match command {
            2060 => {
                let on = !self.s.sync.load(Ordering::Relaxed);
                self.s.sync.store(on, Ordering::Relaxed);
                if on {
                    if let Some(c) = self.s.backing_clip.lock().unwrap().as_ref() {
                        self.s.tempo.set(
                            (self.s.backing_beats.get() * 60. * SR / c.len() as f32)
                                .clamp(40., 240.),
                        );
                    }
                }
                self.flash(if on {
                    "Sync on · first pad starts drums"
                } else {
                    "Free timing · original speed"
                });
            }
            2061 => {
                let grid = (self.s.grid.load(Ordering::Relaxed) + 1) % 4;
                self.s.grid.store(grid, Ordering::Relaxed);
            }
            2062 => {
                let mut pads = self.s.pads.lock().unwrap();
                let Some(data) = pads[self.pad].data.clone() else {
                    return;
                };
                let current = pads[self.pad].set.beats;
                let step = if current < 1. || (current == 1. && value < 0.) {
                    0.25
                } else {
                    1.
                };
                let next = if current <= 0. && value > 0. {
                    guess_beats(data.len(), self.s.tempo.get())
                } else {
                    (current + value.signum() * step).clamp(0., 128.)
                };
                for p in pads.iter_mut() {
                    if p.data.as_ref().is_some_and(|d| Arc::ptr_eq(d, &data)) {
                        p.set.beats = next;
                    }
                }
            }
            2063 => {
                self.s
                    .backing_beats
                    .set((self.s.backing_beats.get() + value.signum()).clamp(1., 128.));
            }
            2064 => {
                if let Some(clip) = self.s.backing_clip.lock().unwrap().as_ref() {
                    self.s.tempo.set(
                        (self.s.backing_beats.get() * 60. * SR / clip.len() as f32)
                            .clamp(40., 240.),
                    );
                }
            }
            2065 => {
                let on = !self.s.fit_samples.load(Ordering::Relaxed);
                self.s.fit_samples.store(on, Ordering::Relaxed);
            }
            2047 => {
                self.stop_preview();
                if let Some(path) = self.library.get(self.lib_index).cloned() {
                    match read_audio(&path) {
                        Ok(data) if data.len() >= 2 => {
                            if let Some(old) =
                                self.s.backing_clip.lock().unwrap().replace(Arc::new(data))
                            {
                                self.retired_loops.push(old);
                            }
                            if let Some(c) = self.s.backing_clip.lock().unwrap().as_ref() {
                                self.s
                                    .backing_beats
                                    .set(guess_beats(c.len(), self.s.tempo.get()));
                            }
                            self.backing_name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            self.s.beat_playing.store(false, Ordering::Release);
                            self.flash("Backing loop loaded · tap Drum beat to play");
                        }
                        Ok(_) => self.flash("Backing loop is too short"),
                        Err(e) => self.flash(e),
                    }
                } else {
                    self.flash("Choose a backing loop from Library");
                }
            }
            2048 => {
                self.stop_preview();
                self.refresh_library();
            }
            2049 => {
                let i = value as usize;
                if i < self.library.len() {
                    self.stop_preview();
                    self.lib_index = i;
                }
            }
            2050 => self.pending_erase = Some(self.bank),
            2051 => self.pending_erase = Some(PADS),
            2052 => self.confirm_erase(),
            2053 => self.pending_erase = None,
            2000 => {
                self.view = (value as usize).min(VIEWS.len() - 1);
                self.row = 0;
            }
            2001 => {
                let pad = (value as usize).min(PADS - 1);
                self.pad = pad;
                self.bank = pad / BANK_PADS;
                self.s.live.lock().unwrap().push((pad, 1., true));
            }
            2002 => {
                self.s
                    .live
                    .lock()
                    .unwrap()
                    .push(((value as usize).min(PADS - 1), 0., false));
            }
            2003 => self.change_bank(value as i32),
            2004 => self.load_source(),
            2005 => {
                self.auto_slice(3);
                self.view = 4;
            }
            2006 => {
                if self.ensure_source() {
                    self.view = 4;
                    self.markers = vec![0];
                    self.flash("Tap the waveform to place slice starts");
                }
            }
            2007 => self.audition(),
            2008 => self.browse_file(value as i32),
            2009 => {
                self.slice_count =
                    (self.slice_count as i32 + value as i32).clamp(1, PADS as i32) as usize
            }
            2010 => {
                self.stop_preview();
                self.record(3);
            }
            2011 => self.toggle_loop(),
            2012 => self.clear_loop(),
            2013 => {
                if self.view == 4 {
                    self.touch_at(334. + value.clamp(0., 1.) * 294., 70.);
                }
            }
            2014 => self.touch_at(-1., -1.),
            2040 => {
                let banks = self.bank_count();
                if banks < PADS / BANK_PADS {
                    self.added_banks = banks + 1;
                    self.bank = banks;
                    self.pad = banks * BANK_PADS;
                    self.flash(format!(
                        "Bank {} added · select an empty pad, then load a sound",
                        banks + 1
                    ));
                }
            }
            2045 => {
                self.stop_preview();
                self.record(1);
            }
            2046 => {
                self.stop_preview();
                self.record(2);
            }
            2044 => {
                self.stop_preview();
                if let Some(path) = self.library.get(self.lib_index).cloned() {
                    match read_audio(&path) {
                        Ok(data) if data.len() >= 2 => {
                            let beats = guess_source_beats(data.len(), self.s.tempo.get());
                            self.s.pads.lock().unwrap()[self.pad] = Pad {
                                data: Some(Arc::new(data)),
                                set: PadSet {
                                    beats,
                                    name: path
                                        .file_stem()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into_owned(),
                                    ..PadSet::default()
                                },
                            };
                            self.flash(format!("Sample loaded on pad {}", self.pad + 1));
                        }
                        Ok(_) => self.flash("Sample is too short"),
                        Err(e) => self.flash(e),
                    }
                } else {
                    self.flash("Choose an audio file first");
                }
            }
            2041 => {
                let playing = self.s.beat_playing.load(Ordering::Relaxed);
                self.s.beat_playing.store(!playing, Ordering::Release);
            }
            2042 => self
                .s
                .tempo
                .set((self.s.tempo.get() + value).clamp(40., 240.)),
            2043 => {
                if let Some(old) = self.s.backing_clip.lock().unwrap().take() {
                    self.retired_loops.push(old);
                }
                self.backing_name.clear();
                let next = (self.s.beat_pattern.load(Ordering::Relaxed) + 1) % 3;
                self.s.beat_pattern.store(next, Ordering::Relaxed);
            }
            2016 => {
                self.row = (value as usize).min(self.rows().len().saturating_sub(1));
                match VIEWS[self.view] {
                    View::Slice if self.row > 1 => self.select(),
                    View::Loop => self.select(),
                    View::Edit if self.row == 6 => self.select(),
                    View::Sample if self.row == 3 => self.save_bank(),
                    View::Sample if self.row == 4 => self.load_bank(false),
                    _ => {}
                }
            }
            2017 => {
                self.view = 4;
                self.auto_slice(4);
            }
            2018 => self.apply_markers(),
            2019 => {
                if self.pad > 0 && self.pad < self.markers.len() {
                    self.markers.remove(self.pad);
                    self.pad = self.pad.min(self.markers.len() - 1);
                }
            }
            2020..=2031 => {
                self.row = (command - 2020) as usize;
                if self.row < self.rows().len() {
                    self.edit(value as i32);
                }
            }
            _ => {}
        }
    }

    fn confirm_erase(&mut self) {
        let Some(bank) = self.pending_erase.take() else {
            return;
        };
        let old_count = self.bank_count();
        let mut pads = self.s.pads.lock().unwrap();
        if bank == PADS {
            for p in pads.iter_mut() {
                if let Some(old) = p.data.take() {
                    self.retired_loops.push(old);
                }
                *p = Pad::default();
            }
            self.added_banks = 1;
            self.slice_count = 16;
            self.bank = 0;
            self.pad = 0;
        } else {
            for p in pads.drain(bank * BANK_PADS..(bank + 1) * BANK_PADS) {
                if let Some(old) = p.data {
                    self.retired_loops.push(old);
                }
            }
            pads.extend((0..BANK_PADS).map(|_| Pad::default()));
            self.added_banks = old_count.saturating_sub(1).max(1);
            self.slice_count = self.slice_count.min(self.added_banks * BANK_PADS);
            self.bank = bank.min(self.added_banks - 1);
            self.pad = self.bank * BANK_PADS;
        }
        drop(pads);
        self.s.live.lock().unwrap().clear();
        self.s.clear_voices.store(true, Ordering::Release);
        self.s.loop_armed.store(false, Ordering::Release);
        self.held_targets.fill(None);
        self.touch = None;
        self.source_sample = None;
        self.markers.clear();
        self.flash(if bank == PADS {
            "All pads cleared"
        } else {
            "Bank erased · later banks moved left"
        });
    }

    fn load_source(&mut self) {
        self.stop_preview();
        self.refresh_library();
        let Some(path) = self.library.get(self.lib_index).cloned() else {
            self.flash("Put audio files in samples/ or media/");
            return;
        };
        match read_audio(&path) {
            Ok(data) if data.len() >= 2 => {
                self.source_sample = Some(Arc::new(data));
                self.markers = vec![0];
                self.auto_slice(3);
                self.view = 4;
                self.row = 0;
                self.flash(format!(
                    "{}: {} one-shots",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    self.markers.len()
                ));
            }
            Ok(_) => self.flash("Sample is too short to chop"),
            Err(e) => self.flash(e),
        }
    }

    fn ensure_source(&mut self) -> bool {
        if self.source_sample.is_none() {
            self.source_sample = self.s.pads.lock().unwrap()[self.pad].data.clone();
        }
        if self.source_sample.is_none() {
            self.flash("Load a WAV or record a sample first");
            return false;
        }
        true
    }

    fn auto_slice(&mut self, how: usize) {
        if !self.ensure_source() {
            return;
        }
        let data = self.source_sample.as_ref().unwrap();
        self.markers = if how == 3 {
            let count = self.slice_count.min(data.len() / 2).max(1);
            (0..count).map(|i| data.len() * i / count).collect()
        } else {
            slice_points(data, how, self.sensitivity)
                .iter()
                .map(|(a, _)| *a)
                .collect()
        };
        self.apply_markers();
    }

    fn apply_markers(&mut self) {
        let Some(data) = self.source_sample.clone() else {
            self.flash("Load a sample first");
            return;
        };
        if self.markers.is_empty() {
            self.markers.push(0);
        }
        self.markers.sort_unstable();
        self.markers.dedup();
        self.markers.truncate(PADS);
        let mut pads = self.s.pads.lock().unwrap();
        for i in 0..PADS {
            pads[i] = if let Some(&start) = self.markers.get(i) {
                let end = self.markers.get(i + 1).copied().unwrap_or(data.len());
                Pad {
                    data: Some(data.clone()),
                    set: PadSet {
                        beats: guess_source_beats(data.len(), self.s.tempo.get()),
                        name: format!("Slice {}", i + 1),
                        start: start as f32 / data.len() as f32,
                        end: end as f32 / data.len() as f32,
                        ..PadSet::default()
                    },
                }
            } else {
                Pad::default()
            };
        }
        drop(pads);
        self.pad = self.pad.min(self.markers.len() - 1);
        self.bank = self.pad / BANK_PADS;
        self.flash(format!("{} one-shots assigned", self.markers.len()));
    }

    fn toggle_loop(&mut self) {
        if self.s.loop_clip.lock().unwrap().is_none() {
            self.flash("Record a performance loop first");
            return;
        }
        let playing = self.s.loop_playing.load(Ordering::Relaxed);
        self.s.loop_playing.store(!playing, Ordering::Relaxed);
    }

    fn clear_loop(&mut self) {
        self.s.loop_armed.store(false, Ordering::Release);
        self.s.stop_on_bar.store(false, Ordering::Relaxed);
        self.s.loop_ending.store(false, Ordering::Relaxed);
        self.s.loop_beats.set(0.);
        self.s.loop_playing.store(false, Ordering::Relaxed);
        if let Some(old) = self.s.loop_clip.lock().unwrap().take() {
            self.retired_loops.push(old);
        }
        self.flash("Performance loop cleared");
    }

    /// Assign shared sample ranges without copying sixteen full clips.
    pub fn place(&mut self, data: &[f32], how: usize) {
        let data = Arc::new(trim(data));
        if data.len() < 2 {
            self.flash("Sample is empty");
            return;
        }
        let slices = slice_points(&data, how, self.sensitivity);
        self.source_sample = Some(data.clone());
        self.markers = slices.iter().map(|(a, _)| *a).collect();
        let mut pads = self.s.pads.lock().unwrap();
        for (k, (a, b)) in slices.iter().enumerate() {
            let pad = (self.rec_target + k) % PADS;
            pads[pad] = Pad {
                data: Some(data.clone()),
                set: PadSet {
                    beats: guess_source_beats(data.len(), self.s.tempo.get()),
                    name: format!("Take {}", pad + 1),
                    start: *a as f32 / data.len() as f32,
                    end: *b as f32 / data.len() as f32,
                    ..PadSet::default()
                },
            };
        }
        drop(pads);
        self.flash(format!(
            "{} one-shots from pad {}",
            slices.len(),
            self.rec_target + 1
        ));
    }

    fn bank_count(&self) -> usize {
        let occupied = self
            .s
            .pads
            .lock()
            .unwrap()
            .iter()
            .rposition(|p| p.data.is_some())
            .map_or(1, |i| i + 1);
        occupied
            .max(self.added_banks * BANK_PADS)
            .max(self.slice_count)
            .max(self.pad + 1)
            .div_ceil(BANK_PADS)
    }

    fn cycle_view(&mut self, delta: i32) {
        let position = VIEW_CYCLE.iter().position(|v| *v == self.view).unwrap_or(0);
        let next = (position as i32 + delta.rem_euclid(VIEW_CYCLE.len() as i32))
            .rem_euclid(VIEW_CYCLE.len() as i32) as usize;
        self.view = VIEW_CYCLE[next];
        self.row = 0;
    }

    fn change_bank(&mut self, delta: i32) {
        self.bank = (self.bank as i32 + delta).clamp(0, self.bank_count() as i32 - 1) as usize;
        self.pad = self.bank * BANK_PADS;
    }

    fn touch_at(&mut self, x: f32, y: f32) {
        if x < 0. || y < 0. {
            if let Some(Touch::Pad(p)) = self.touch.take() {
                self.s.live.lock().unwrap().push((p, 0., false));
            }
            return;
        }
        if let Some(touch) = self.touch {
            match touch {
                Touch::Marker(i) => {
                    if let Some(data) = self.source_sample.as_ref() {
                        if i > 0 && i < self.markers.len() {
                            let lo = self.markers[i - 1] + 2;
                            let hi = self
                                .markers
                                .get(i + 1)
                                .copied()
                                .unwrap_or(data.len())
                                .saturating_sub(2);
                            if lo <= hi {
                                self.markers[i] =
                                    (((x - 334.) / 294.).clamp(0., 1.) * data.len() as f32).round()
                                        as usize;
                                self.markers[i] = self.markers[i].clamp(lo, hi);
                            }
                        }
                    }
                }
                Touch::Row { index, last_y } => {
                    let steps = ((last_y - y) / 4.).trunc() as i32;
                    if steps != 0 {
                        self.row = index;
                        self.edit(steps);
                        self.touch = Some(Touch::Row { index, last_y: y });
                    }
                }
                _ => {}
            }
            return;
        }
        self.touch = Some(Touch::Button);
        if y < 30. && (70. ..394.).contains(&x) {
            self.view = VIEW_CYCLE[((x - 70.) / 54.) as usize];
            self.row = 0;
            if self.view == 4 {
                self.refresh_library();
            }
            return;
        }
        if x < 324. && (220. ..244.).contains(&y) {
            if x < 86. {
                self.change_bank(-1);
            } else if x >= 238. {
                self.change_bank(1);
            }
            return;
        }
        if (8. ..320.).contains(&x) && (38. ..220.).contains(&y) {
            let p =
                self.bank * BANK_PADS + ((y - 38.) / 46.) as usize * 4 + ((x - 8.) / 78.) as usize;
            if p < PADS {
                self.pad = p;
                self.s.live.lock().unwrap().push((p, 1., true));
                self.touch = Some(Touch::Pad(p));
            }
            return;
        }
        if x < 324. && (244. ..278.).contains(&y) {
            match ((x - 8.) / 104.).floor() as i32 {
                0 => self.load_source(),
                1 => {
                    self.auto_slice(3);
                    self.view = 4;
                }
                2 => {
                    if self.ensure_source() {
                        self.view = 4;
                        self.markers = vec![0];
                        self.flash("Tap waveform markers, then APPLY");
                    }
                }
                _ => {}
            }
            return;
        }
        if x < 324. && (284. ..318.).contains(&y) {
            match ((x - 8.) / 104.).floor() as i32 {
                0 => self.record(3),
                1 => self.toggle_loop(),
                2 => self.clear_loop(),
                _ => {}
            }
            return;
        }
        if self.view == 4
            && (330. ..632.).contains(&x)
            && (38. ..128.).contains(&y)
            && self.ensure_source()
        {
            let n = self.source_sample.as_ref().unwrap().len();
            let frame = (((x - 334.) / 294.).clamp(0., 1.) * n as f32).round() as usize;
            if let Some(i) = self
                .markers
                .iter()
                .enumerate()
                .min_by_key(|(_, m)| m.abs_diff(frame))
                .filter(|(_, m)| {
                    m.abs_diff(frame) as f32 / (n as f32)
                        < 0.025f32.min(0.35 / self.markers.len().max(1) as f32)
                })
                .map(|(i, _)| i)
            {
                self.pad = i;
                self.touch = Some(Touch::Marker(i));
            } else if self.markers.len() < PADS && frame > 1 && frame < n.saturating_sub(2) {
                self.markers.push(frame);
                self.markers.sort_unstable();
                self.pad = self.markers.iter().position(|&m| m == frame).unwrap();
                self.touch = Some(Touch::Marker(self.pad));
            }
            return;
        }
        if x >= 330. && y >= 136. {
            let row = ((y - 136.) / 24.) as usize;
            if row >= self.rows().len() {
                return;
            }
            self.row = row;
            if (self.view == 4 && row > 1) || self.view == 5 {
                self.select();
            } else if x >= 588. {
                self.edit(if x < 610. { -1 } else { 1 });
            } else if matches!(VIEWS[self.view], View::Slice | View::Loop) {
                self.select();
            } else if self.view == 1 && row == 6 {
                self.select();
            } else {
                self.touch = Some(Touch::Row {
                    index: row,
                    last_y: y,
                });
            }
        }
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
        let clip = self.s.loop_clip.lock().unwrap().clone();
        if let Some(data) = clip.as_ref() {
            if let Err(e) = write_wav(&dir.join("performance.wav"), data) {
                self.flash(e);
                return;
            }
        }
        let backing = self.s.backing_clip.lock().unwrap().clone();
        if let Some(clip) = backing.as_ref() {
            if let Err(e) = write_wav(&dir.join("backing.wav"), clip) {
                self.flash(e);
                return;
            }
        }
        let j = serde_json::json!({ "sync":self.s.sync.load(Ordering::Relaxed),"fit_samples":self.s.fit_samples.load(Ordering::Relaxed),"grid":self.s.grid.load(Ordering::Relaxed),"backing_beats":self.s.backing_beats.get(),"loop_beats":self.s.loop_beats.get(), "backing_name":self.backing_name,"backing":backing.is_some(), "added_banks":self.added_banks, "beat_pattern":self.s.beat_pattern.load(Ordering::Relaxed), "slice_count":self.slice_count, "pads": sets, "fx": fx, "tempo": self.s.tempo.get(), "performance_loop":clip.is_some() });
        let _ = std::fs::write(
            dir.join("bank.json"),
            serde_json::to_string_pretty(&j).unwrap_or_default(),
        );
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
        self.added_banks = v["added_banks"].as_u64().unwrap_or(1).clamp(1, 8) as usize;
        self.s.beat_pattern.store(
            v["beat_pattern"].as_u64().unwrap_or(0).min(2) as usize,
            Ordering::Relaxed,
        );
        self.s
            .sync
            .store(v["sync"].as_bool().unwrap_or(false), Ordering::Relaxed);
        self.s.fit_samples.store(
            v["fit_samples"].as_bool().unwrap_or(true),
            Ordering::Relaxed,
        );
        self.s.grid.store(
            v["grid"].as_u64().unwrap_or(1).min(3) as usize,
            Ordering::Relaxed,
        );
        self.s
            .backing_beats
            .set(v["backing_beats"].as_f64().unwrap_or(16.).clamp(1., 128.) as f32);
        self.s
            .loop_beats
            .set(v["loop_beats"].as_f64().unwrap_or(0.).max(0.) as f32);
        self.s.loop_origin.set(0.);
        let sets: Vec<PadSet> = serde_json::from_value(v["pads"].clone()).unwrap_or_default();
        let mut pads = self.s.pads.lock().unwrap();
        for k in 0..PADS {
            let data = read_wav(&dir.join(format!("pad_{:02}.wav", k + 1)))
                .ok()
                .map(Arc::new);
            let mut set = sets.get(k).cloned().unwrap_or_default();
            if set.beats <= 0. {
                if let Some(clip) = data.as_ref() {
                    set.beats =
                        guess_source_beats(clip.len(), v["tempo"].as_f64().unwrap_or(90.) as f32);
                }
            }
            pads[k] = Pad { data, set };
        }
        drop(pads);
        if let Ok(fx) = serde_json::from_value::<[FxSlot; 2]>(v["fx"].clone()) {
            *self.s.fx.lock().unwrap() = fx;
        }
        if let Some(t) = v["tempo"].as_f64() {
            self.s.tempo.set(t as f32);
        }
        self.s.beat_playing.store(false, Ordering::Release);
        self.backing_name = v["backing_name"]
            .as_str()
            .unwrap_or("Loaded backing")
            .to_string();
        let backing = if v["backing"].as_bool() == Some(true) {
            read_wav(&dir.join("backing.wav")).ok().map(Arc::new)
        } else {
            None
        };
        if backing.is_none() {
            self.backing_name.clear();
        }
        if let Some(old) = std::mem::replace(&mut *self.s.backing_clip.lock().unwrap(), backing) {
            self.retired_loops.push(old);
        }
        self.s.loop_armed.store(false, Ordering::Release);
        self.s.loop_playing.store(false, Ordering::Relaxed);
        let clip = if v["performance_loop"].as_bool() == Some(true) {
            read_wav(&dir.join("performance.wav")).ok().map(Arc::new)
        } else {
            None
        };
        if let Some(old) = std::mem::replace(&mut *self.s.loop_clip.lock().unwrap(), clip) {
            self.retired_loops.push(old);
        }
        self.slice_count = v["slice_count"]
            .as_u64()
            .unwrap_or(16)
            .clamp(1, PADS as u64) as usize;
        self.bank = 0;
        self.pad = 0;
        self.source_sample = None;
        self.markers.clear();
        if !quiet {
            self.flash("Bank loaded");
        }
    }

    fn rows(&self) -> Vec<(&'static str, String)> {
        self.rows_for(VIEWS[self.view])
    }

    fn rows_for(&self, view: View) -> Vec<(&'static str, String)> {
        let pads = self.s.pads.lock().unwrap();
        let p = &pads[self.pad];
        let fx = *self.s.fx.lock().unwrap();
        match view {
            View::Play => vec![],
            View::Slice => vec![
                (
                    "File",
                    self.library
                        .get(self.lib_index)
                        .and_then(|p| p.file_name())
                        .map_or("No WAVs in media/".into(), |n| {
                            n.to_string_lossy().into_owned()
                        }),
                ),
                ("Slices", self.slice_count.to_string()),
                ("Load + cut", "TAP".into()),
                ("Auto cut", "TAP".into()),
                ("At hits", "TAP".into()),
                ("Clear markers", "TAP".into()),
                ("Apply markers", format!("{} pads", self.markers.len())),
                ("Remove marker", format!("Pad {}", self.pad + 1)),
            ],
            View::Loop => vec![
                (
                    "Record playing",
                    if self.s.loop_armed.load(Ordering::Acquire) {
                        "CANCEL ARM"
                    } else if self.s.recording.load(Ordering::Relaxed) == 3 {
                        "STOP + LOOP"
                    } else {
                        "RECORD"
                    }
                    .into(),
                ),
                (
                    "Performance",
                    if self.s.loop_playing.load(Ordering::Relaxed) {
                        "PAUSE"
                    } else {
                        "PLAY"
                    }
                    .into(),
                ),
                ("Clear loop", "TAP".into()),
                (
                    "Length",
                    format!(
                        "{:.2} s",
                        self.s
                            .loop_clip
                            .lock()
                            .unwrap()
                            .as_ref()
                            .map_or(0., |c| c.len() as f32 / SR)
                    ),
                ),
            ],
            View::Edit => vec![
                ("Start", format!("{:.1}%", p.set.start * 100.0)),
                ("End", format!("{:.1}%", p.set.end * 100.0)),
                ("Pitch", format!("{:+.0} st", p.set.pitch)),
                ("Gain", format!("{:.0}%", p.set.gain * 100.0)),
                ("Mode", format!("{:?}", p.set.mode)),
                (
                    "Reverse",
                    if p.set.reverse {
                        "on".into()
                    } else {
                        "off".into()
                    },
                ),
                (
                    "Library",
                    self.library
                        .get(self.lib_index)
                        .map_or("(media/ has no WAVs)".into(), |f| {
                            f.file_name()
                                .map_or(String::new(), |n| n.to_string_lossy().to_string())
                        }),
                ),
                ("Clear pad", "< >".into()),
            ],
            View::Sample => vec![
                ("Source", self.bus.source_name(self.source)),
                ("Chop", CHOPS[self.chop].into()),
                (
                    "Hit sensitivity",
                    format!("{:.0}%", self.sensitivity * 100.0),
                ),
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

    fn controller_entries(&self) -> Vec<(usize, usize, crate::app::Setting)> {
        VIEWS
            .iter()
            .enumerate()
            .flat_map(|(view, &v)| {
                self.rows_for(v)
                    .into_iter()
                    .enumerate()
                    .map(move |(row, (label, value))| {
                        let group = if v == View::Edit {
                            format!("Pad {}", self.pad + 1)
                        } else {
                            format!("{v:?}")
                        };
                        (
                            view,
                            row,
                            crate::app::Setting {
                                label: format!("{group} / {label}"),
                                value,
                            },
                        )
                    })
            })
            .collect()
    }

    fn edit(&mut self, d: i32) {
        let df = d as f32;
        match VIEWS[self.view] {
            View::Play => {}
            View::Slice => {
                if self.row == 1 {
                    self.slice_count = (self.slice_count as i32 + d).clamp(1, PADS as i32) as usize;
                }
                if self.row == 0 {
                    if self.library.is_empty() {
                        self.refresh_library();
                    }
                    if !self.library.is_empty() {
                        self.lib_index = (self.lib_index as i32 + d)
                            .rem_euclid(self.library.len() as i32)
                            as usize;
                    }
                }
            }
            View::Loop => {}
            View::Edit => {
                if self.row == 6 {
                    if self.library.is_empty() {
                        self.refresh_library();
                    }
                    if !self.library.is_empty() {
                        self.lib_index = (self.lib_index as i32 + d)
                            .rem_euclid(self.library.len() as i32)
                            as usize;
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
                    4 => {
                        p.set.mode = match (p.set.mode, d > 0) {
                            (Mode::OneShot, true) => Mode::Gate,
                            (Mode::Gate, true) => Mode::Loop,
                            (Mode::Loop, true) => Mode::OneShot,
                            (Mode::OneShot, false) => Mode::Loop,
                            (Mode::Gate, false) => Mode::OneShot,
                            (Mode::Loop, false) => Mode::Gate,
                        }
                    }
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
                    self.s
                        .tempo
                        .set((self.s.tempo.get().round() + df).clamp(40.0, 240.0));
                    return;
                }
                let mut fx = self.s.fx.lock().unwrap();
                let slot = &mut fx[self.row / 3];
                match self.row % 3 {
                    0 => {
                        slot.kind =
                            (slot.kind as i32 + d).rem_euclid(FX_NAMES.len() as i32) as usize
                    }
                    1 => slot.a = (slot.a + df * 0.02).clamp(0.0, 1.0),
                    _ => slot.b = (slot.b + df * 0.02).clamp(0.0, 1.0),
                }
            }
        }
    }

    fn select(&mut self) {
        match VIEWS[self.view] {
            View::Play | View::Fx => self.record(2),
            View::Slice => match self.row {
                0 | 2 => self.load_source(),
                1 | 3 => self.auto_slice(3),
                4 => self.auto_slice(4),
                5 => {
                    if self.ensure_source() {
                        self.markers = vec![0];
                        self.flash("Tap waveform to add markers, then Apply");
                    }
                }
                6 => self.apply_markers(),
                _ => {
                    if self.pad > 0 && self.pad < self.markers.len() {
                        self.markers.remove(self.pad);
                        self.pad = self.pad.min(self.markers.len() - 1);
                    }
                }
            },
            View::Loop => match self.row {
                0 => self.record(3),
                1 => self.toggle_loop(),
                2 => self.clear_loop(),
                _ => {}
            },
            View::Sample => self.record(1),
            View::Edit => {
                if self.row == 6 {
                    if let Some(f) = self.library.get(self.lib_index).cloned() {
                        match read_audio(&f) {
                            Ok(d) => {
                                let mut pads = self.s.pads.lock().unwrap();
                                pads[self.pad] = Pad {
                                    data: Some(Arc::new(d)),
                                    set: PadSet {
                                        name: f.file_stem().map_or(String::new(), |n| {
                                            n.to_string_lossy().to_string()
                                        }),
                                        ..PadSet::default()
                                    },
                                };
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
    let start = data
        .iter()
        .position(|x| x.abs() > peak * 0.05)
        .unwrap_or(0)
        .saturating_sub(240);
    data[start..].to_vec()
}

impl App for Chop {
    fn on_enter(&mut self) {
        self.refresh_library();
    }
    fn on_exit(&mut self) {
        self.stop_preview();
        self.touch_at(-1., -1.);
    }
    fn background_tick(&mut self) {
        self.retired_loops.retain(|c| Arc::strong_count(c) > 1);
        if self.s.rec_len.load(Ordering::Relaxed) >= MAX_REC {
            self.s.recording.store(0, Ordering::Relaxed);
        }
        if self.s.recording.load(Ordering::Relaxed) == 0 {
            self.collect();
        }
    }
    // A source lists the pad's Edit rows whatever view Chop is showing.
    fn instrument_settings(&self) -> Vec<crate::app::Setting> {
        self.rows_for(View::Edit)
            .into_iter()
            .map(|(label, value)| crate::app::Setting {
                label: label.into(),
                value,
            })
            .collect()
    }
    fn controller_settings(&self) -> Option<Vec<crate::app::Setting>> {
        let mut settings = vec![
            crate::app::Setting {
                label: "View".into(),
                value: format!("{:?}", VIEWS[self.view]),
            },
            crate::app::Setting {
                label: "Pad".into(),
                value: (self.pad + 1).to_string(),
            },
        ];
        settings.extend(
            self.controller_entries()
                .into_iter()
                .map(|(_, _, setting)| setting),
        );
        Some(settings)
    }
    fn controller_select(&mut self, index: usize) {
        if let Some(index) = index.checked_sub(2) {
            if let Some((view, row, _)) = self.controller_entries().into_iter().nth(index) {
                self.view = view;
                self.row = row;
            }
        }
    }
    fn controller_adjust(&mut self, index: usize, delta: i32) {
        match index {
            0 => {
                self.cycle_view(delta);
            }
            1 => self.pad = (self.pad as i32 + delta).rem_euclid(PADS as i32) as usize,
            _ if index < self.controller_entries().len() + 2 => {
                self.controller_select(index);
                self.edit(delta);
            }
            _ => {}
        }
    }
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        let Some(edit) = VIEWS.iter().position(|v| *v == View::Edit) else {
            return;
        };
        let (view, row) = (
            std::mem::replace(&mut self.view, edit),
            std::mem::replace(&mut self.row, index),
        );
        self.edit(delta);
        self.view = view;
        self.row = row;
    }
    fn slint_library_search(&mut self, query: &str) {
        self.stop_preview();
        self.library_query = query.to_lowercase();
        if let Some(i) = self.library.iter().position(|p| {
            p.to_string_lossy()
                .to_lowercase()
                .contains(&self.library_query)
        }) {
            self.lib_index = i;
        }
    }
    fn slint_pointer_pick(&mut self, x: f32, y: f32) {
        if x >= 2000. {
            self.native_action(x as i32, y);
        } else {
            self.touch_at(x, y);
        }
    }
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let pads = self.s.pads.lock().unwrap();
        let selected = self.library.get(self.lib_index);
        let audition = self.s.preview_playing.load(Ordering::Acquire)
            && self.preview_file.as_ref() == selected
            && selected.is_some();
        let preview = self.s.preview_clip.lock().unwrap();
        let data = if audition {
            preview.as_ref()
        } else if self.view == 4 {
            self.source_sample.as_ref()
        } else {
            pads[self.pad].data.as_ref()
        };
        let wave = data.map_or_else(Vec::new, |data| {
            (0..168)
                .map(|i| {
                    let a = data.len() * i / 168;
                    let b = (data.len() * (i + 1) / 168).max(a + 1).min(data.len());
                    data.get(a..b)
                        .unwrap_or(&[])
                        .iter()
                        .fold(0f32, |m, x| m.max(x.abs()))
                        .min(1.)
                })
                .collect()
        });
        let names = (0..BANK_PADS)
            .map(|i| pads[self.bank * BANK_PADS + i].set.name.clone())
            .collect();
        let loaded = (0..BANK_PADS)
            .map(|i| pads[self.bank * BANK_PADS + i].data.is_some())
            .collect();
        let seconds = data.map_or(0., |data| data.len() as f32 / SR);
        let markers = if self.view == 4 && !audition {
            self.source_sample.as_ref().map_or_else(Vec::new, |data| {
                self.markers
                    .iter()
                    .map(|m| *m as f32 / data.len() as f32)
                    .collect()
            })
        } else {
            Vec::new()
        };
        drop(preview);
        drop(pads);
        let recording = self.s.recording.load(Ordering::Relaxed) != 0;
        let armed = self.s.loop_armed.load(Ordering::Acquire);
        let status = if armed {
            "Armed · play a pad to start".into()
        } else if recording {
            format!(
                "Recording · {:.1}s",
                self.s.rec_len.load(Ordering::Relaxed) as f32 / SR
            )
        } else if self.message.1 > 0 {
            self.message.0.clone()
        } else {
            "Ready to play".into()
        };
        let rows = self.rows();
        let sample_beats = self.s.pads.lock().unwrap()[self.pad].set.beats;
        SlintExtra::Chop(crate::app::ChopExtra {
            ending: self.s.loop_ending.load(Ordering::Relaxed),
            queued_pad: self.s.queued_pad.load(Ordering::Relaxed) as i32 - 1,
            sync: self.s.sync.load(Ordering::Relaxed),
            fit_samples: self.s.fit_samples.load(Ordering::Relaxed),
            grid: self.s.grid.load(Ordering::Relaxed) as i32,
            sample_beats,
            backing_beats: self.s.backing_beats.get(),
            beat_position: self.s.beat_position.get(),
            library_files: self
                .library
                .iter()
                .filter(|p| {
                    p.to_string_lossy()
                        .to_lowercase()
                        .contains(&self.library_query)
                })
                .map(|p| {
                    p.strip_prefix(&self.media)
                        .unwrap_or(p)
                        .to_string_lossy()
                        .into_owned()
                })
                .collect(),
            library_ids: self
                .library
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    p.to_string_lossy()
                        .to_lowercase()
                        .contains(&self.library_query)
                })
                .map(|(i, _)| i as i32)
                .collect(),
            library_selected: self.lib_index as i32,
            erase: self.pending_erase.map_or(String::new(), |b| {
                if b == PADS {
                    "Clear ALL pads?".into()
                } else {
                    format!("Erase bank {}?", b + 1)
                }
            }),
            loaded_backing: !self.backing_name.is_empty(),
            backing_name: self.backing_name.clone(),
            view: self.view,
            pad: self.pad,
            bank: self.bank,
            banks: self.bank_count(),
            count: self.slice_count,
            filename: selected
                .and_then(|p| p.file_name())
                .map_or("Choose audio from media/".into(), |n| {
                    n.to_string_lossy().into_owned()
                }),
            status,
            names,
            loaded,
            heads: (0..BANK_PADS)
                .map(|i| self.s.playhead[self.bank * BANK_PADS + i].get())
                .collect(),
            wave,
            markers,
            seconds,
            previewing: self.s.preview_playing.load(Ordering::Acquire),
            preview_head: self.s.preview_head.get(),
            armed,
            recording,
            looping: self.s.loop_playing.load(Ordering::Relaxed),
            loop_head: self.s.loop_head.get(),
            beat: self.s.beat_playing.load(Ordering::Relaxed),
            bpm: self.s.tempo.get(),
            beat_pattern: ["Straight", "Half-time", "Breakbeat"]
                [self.s.beat_pattern.load(Ordering::Relaxed) % 3]
                .into(),
            rows: rows.iter().map(|(name, _)| name.to_string()).collect(),
            values: rows.iter().map(|(_, value)| value.clone()).collect(),
            selected: self.row,
        })
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![
            ("View".into(), format!("{:?}", VIEWS[self.view]), false),
            ("Pad".into(), format!("{}", self.pad + 1), false),
        ]
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(match VIEWS[self.view] {
            View::Play => "PLAY",
            View::Edit => "EDIT",
            View::Sample => "INPUT",
            View::Fx => "FX",
            View::Slice => "SLICES",
            View::Loop => "LOOP",
        })
    }
    fn toggle_grid_mode(&mut self) {
        self.cycle_view(1);
    }
    fn running(&self) -> Option<bool> {
        Some(
            self.s.loop_armed.load(Ordering::Acquire)
                || self.s.recording.load(Ordering::Relaxed) != 0
                || self.s.loop_playing.load(Ordering::Relaxed),
        )
    }
    fn toggle_running(&mut self) {
        if self.s.loop_armed.load(Ordering::Acquire) {
            self.record(3);
            return;
        }
        if self.s.recording.load(Ordering::Relaxed) != 0 {
            self.record(self.rec_kind);
            return;
        }
        if VIEWS[self.view] == View::Loop {
            if self.s.loop_armed.load(Ordering::Acquire)
                || self.s.recording.load(Ordering::Relaxed) == 3
                || self.s.loop_clip.lock().unwrap().is_none()
            {
                self.record(3);
            } else {
                self.toggle_loop();
            }
        } else {
            self.select();
        }
    }
    fn transport_action(&self) -> Option<&'static str> {
        if self.s.loop_armed.load(Ordering::Acquire) {
            return Some("CANCEL ARM");
        }
        if self.s.recording.load(Ordering::Relaxed) == 3 {
            return Some("END LOOP");
        }
        if VIEWS[self.view] == View::Loop {
            return Some(if self.s.loop_armed.load(Ordering::Acquire) {
                "CANCEL ARM"
            } else if self.s.recording.load(Ordering::Relaxed) == 3 {
                "END LOOP"
            } else if self.s.loop_clip.lock().unwrap().is_none() {
                "REC LOOP"
            } else if self.s.loop_playing.load(Ordering::Relaxed) {
                "PAUSE LOOP"
            } else {
                "PLAY LOOP"
            });
        }
        Some(if self.s.recording.load(Ordering::Relaxed) != 0 {
            "STOP REC"
        } else if VIEWS[self.view] == View::Sample {
            "RECORD"
        } else {
            "RESAMPLE"
        })
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let pads = self.s.pads.lock().unwrap();
        std::array::from_fn(|i| {
            let i = self.bank * BANK_PADS + i;
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
        self.retired_loops.retain(|c| Arc::strong_count(c) > 1);
        self.poll += 1;
        if self.poll % 15 == 0 && self.source != NO_SOURCE {
            let _ = self.bus.get(self.source);
        }
        for p in 0..16 {
            if input.grid[p] && !self.prev[p] {
                let target = self.bank * BANK_PADS + p;
                self.pad = target;
                self.held_targets[p] = Some(target);
                self.s.live.lock().unwrap().push((target, 1.0, true));
            } else if !input.grid[p] && self.prev[p] {
                if let Some(target) = self.held_targets[p].take() {
                    self.s.live.lock().unwrap().push((target, 0.0, false));
                }
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
                self.pad =
                    (self.pad as i32 + input.knob2.signum()).rem_euclid(PADS as i32) as usize;
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
        self.bank = self.pad / BANK_PADS;
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
        for (i, v) in ["PLAY", "SLICES", "EDIT", "INPUT", "FX", "LOOP"]
            .iter()
            .enumerate()
        {
            let x = 70 + i as i32 * 54;
            let sel = VIEW_CYCLE[i] == self.view;
            kit::round_rect(
                fb,
                x,
                4,
                50,
                22,
                6,
                if sel { ink } else { Rgb565::new(13, 12, 9) },
            );
            kit::text(
                fb,
                v,
                x + 25,
                10,
                Size2::Small,
                if sel { kit::BLACK } else { dim },
                0,
            );
        }
        let rec = self.s.recording.load(Ordering::Relaxed);
        let status = match rec {
            0 if self.s.loop_armed.load(Ordering::Acquire) => "ARMED / HIT A PAD TO START".into(),
            1 => format!(
                "RECORDING {:.1}s",
                self.s.rec_len.load(Ordering::Relaxed) as f32 / SR
            ),
            3 => format!(
                "LOOP REC {:.1}s",
                self.s.rec_len.load(Ordering::Relaxed) as f32 / SR
            ),
            2 => format!(
                "RESAMPLING {:.1}s",
                self.s.rec_len.load(Ordering::Relaxed) as f32 / SR
            ),
            _ if self.message.1 > 0 => self.message.0.chars().take(36).collect(),
            _ => {
                let fx = *self.s.fx.lock().unwrap();
                format!("{} > {}", FX_NAMES[fx[0].kind], FX_NAMES[fx[1].kind])
            }
        };
        kit::text(
            fb,
            &status,
            630,
            9,
            Size2::Small,
            if rec > 0 { kit::rgb(250, 90, 90) } else { dim },
            1,
        );
        let pads = self.s.pads.lock().unwrap().clone();
        // The pads.
        for local in 0..BANK_PADS {
            let p = self.bank * BANK_PADS + local;
            let x = 8 + (local as i32 % 4) * 78;
            let y = 38 + (local as i32 / 4) * 46;
            let has = pads[p].data.is_some();
            let head = self.s.playhead[p].get();
            let c = if head >= 0.0 {
                ink
            } else if has {
                kit::blend(panel, ink, 0.35)
            } else {
                panel
            };
            kit::round_rect(fb, x, y, 74, 42, 6, c);
            if p == self.pad {
                kit::outline(fb, x - 2, y - 2, 78, 46, 7, 2, kit::WHITE);
            }
            kit::text(
                fb,
                &format!("{}", p + 1),
                x + 5,
                y + 3,
                Size2::Small,
                if head >= 0.0 { kit::BLACK } else { dim },
                -1,
            );
            if has {
                let n: String = pads[p].set.name.chars().take(11).collect();
                kit::text(
                    fb,
                    &n,
                    x + 37,
                    y + 26,
                    Size2::Small,
                    if head >= 0.0 { kit::BLACK } else { kit::WHITE },
                    0,
                );
            }
        }
        // The picked pad's waveform, with its start, end and playhead.
        let (wx, wy, ww, wh) = (330, 38, 302, 90);
        kit::round_rect(fb, wx, wy, ww, wh, 8, panel);
        let display_data = if self.view == 4 {
            self.source_sample.as_ref()
        } else {
            pads[self.pad].data.as_ref()
        };
        if let Some(d) = display_data {
            let n = d.len();
            let mid = wy + wh / 2;
            for i in 0..ww - 8 {
                let a = n * i as usize / (ww - 8) as usize;
                let b = (n * (i as usize + 1) / (ww - 8) as usize).max(a + 1).min(n);
                let peak = d[a..b].iter().fold(0.0f32, |m, x| m.max(x.abs()));
                let h = (peak * (wh / 2 - 6) as f32) as i32;
                let t = i as f32 / (ww - 8) as f32;
                let inside = t >= pads[self.pad].set.start && t <= pads[self.pad].set.end;
                kit::rect(
                    fb,
                    wx + 4 + i,
                    mid - h,
                    1,
                    h * 2 + 1,
                    if inside { ink } else { dim },
                );
            }
            let head = self.s.playhead[self.pad].get();
            if head >= 0.0 {
                let st = &pads[self.pad].set;
                let x = wx + 4 + ((st.start + (st.end - st.start) * head) * (ww - 8) as f32) as i32;
                kit::rect(fb, x, wy + 4, 2, wh - 8, kit::WHITE);
            }
            if self.view == 4 {
                for (i, &marker) in self.markers.iter().enumerate() {
                    let mx = wx + 4 + (marker as f32 / n as f32 * 294.) as i32;
                    kit::rect(
                        fb,
                        mx,
                        wy + 3,
                        1,
                        wh - 6,
                        if i == self.pad { kit::WHITE } else { ink },
                    );
                    if i / BANK_PADS == self.bank {
                        kit::text(
                            fb,
                            &format!("{}", i + 1),
                            mx + 2,
                            wy + 10,
                            Size2::Small,
                            kit::WHITE,
                            -1,
                        );
                    }
                }
            }
            kit::text(
                fb,
                &format!("{:.2} s", n as f32 / SR),
                wx + ww - 8,
                wy + 4,
                Size2::Small,
                dim,
                1,
            );
        } else {
            kit::text(
                fb,
                "empty pad",
                wx + ww / 2,
                wy + wh / 2 - 6,
                Size2::Small,
                dim,
                0,
            );
        }
        // The view's rows.
        let rows = self.rows();
        for (i, (l, v)) in rows.iter().enumerate() {
            let y = 136 + i as i32 * 24;
            let sel = i == self.row;
            kit::round_rect(
                fb,
                330,
                y,
                302,
                21,
                5,
                if sel {
                    kit::blend(panel, kit::WHITE, 0.15)
                } else {
                    panel
                },
            );
            kit::text(
                fb,
                l,
                338,
                y + 5,
                Size2::Small,
                if sel { kit::WHITE } else { dim },
                -1,
            );
            let v: String = v.chars().take(20).collect();
            if !((self.view == 4 && i > 1) || self.view == 5) {
                kit::text(fb, "-  +", 626, y + 5, Size2::Small, kit::WHITE, 1);
            }
            kit::text(fb, &v, 580, y + 5, Size2::Small, ink, 1);
        }
        let help = match VIEWS[self.view] {
            View::Play if self.s.pads.lock().unwrap().iter().all(|p| p.data.is_none()) => "No sounds yet. F2 to SAMPLE to record the input or load a file from your library, then chop it across the pads.",
            View::Play => "Pads play. Left/right picks a pad. SELECT or F3: resample Chop's output (with FX) into the next empty pad.",
            View::Edit => "Up/down: setting. Left/right: change. SELECT plays the pad (or loads the library file on the Library row).",
            View::Sample => "SELECT or F3: record the source into the picked pad, chopped as set. 'at hits' cuts at every hit.",
            View::Slice => "Load a loop, Auto 16, or Clear markers then tap/drag the waveform. Apply assigns one-shots in order.",
            View::Loop => "REC LOOP arms; the first playable pad starts recording. Tap END LOOP to repeat. CANCEL ARM disarms before playing.",
            View::Fx => "Two effects in series on everything Chop plays. SELECT or F3: resample.",
        };
        kit::round_rect(fb, 8, 222, 74, 20, 4, panel);
        kit::text(fb, "< BANK", 45, 227, Size2::Small, ink, 0);
        kit::text(
            fb,
            &format!("{} / {}", self.bank + 1, self.bank_count()),
            162,
            227,
            Size2::Small,
            ink,
            0,
        );
        kit::round_rect(fb, 242, 222, 74, 20, 4, panel);
        kit::text(fb, "BANK >", 279, 227, Size2::Small, ink, 0);
        let auto = format!("CUT {}", self.slice_count);
        for (i, label) in ["LOAD WAV", auto.as_str(), "MARKERS"].iter().enumerate() {
            let x = 8 + i as i32 * 104;
            kit::round_rect(fb, x, 244, 98, 34, 5, panel);
            kit::text(fb, label, x + 49, 256, Size2::Small, ink, 0);
        }
        for (i, label) in [
            if self.s.loop_armed.load(Ordering::Acquire) {
                "CANCEL ARM"
            } else if rec == 3 {
                "END LOOP"
            } else {
                "REC LOOP"
            },
            if self.s.loop_playing.load(Ordering::Relaxed) {
                "PAUSE"
            } else {
                "PLAY LOOP"
            },
            "CLEAR LOOP",
        ]
        .iter()
        .enumerate()
        {
            let x = 8 + i as i32 * 104;
            kit::round_rect(fb, x, 284, 98, 34, 5, panel);
            kit::text(fb, label, x + 49, 296, Size2::Small, ink, 0);
        }
        kit::text(
            fb,
            if self.view == 4 {
                "Tap / drag markers on waveform, then APPLY MARKERS"
            } else {
                "Set SLICES, LOAD WAV, then play. BANK changes the 16 pads."
            },
            8,
            337,
            Size2::Small,
            dim,
            -1,
        );
        let _ = help;
        let loop_head = self.s.loop_head.get();
        if loop_head >= 0. {
            kit::rect(fb, 8, 320, (310. * loop_head) as i32, 3, ink);
        }
        let lvl = self.s.level.get();
        kit::round_rect(
            fb,
            8,
            239,
            (316.0 * lvl.min(1.0)) as i32 + 3,
            5,
            2,
            if rec == 1 { kit::rgb(250, 90, 90) } else { ink },
        );
    }

    fn audio_processor(&mut self) -> Option<Box<dyn crate::audio::AudioProcessor>> {
        let engine = Engine {
            s: Arc::clone(&self.s),
            voices: (0..PADS).map(|_| None).collect(),
            fx: [Fx::new(11), Fx::new(23)],
            rec: None,
            input: Vec::with_capacity(4096),
            read: 0,
            inbox: self.inbox.take(),
            view: NoteView::default(),
            strikes: [0; 128],
            keys: [0; 128],
            peak: 0.0,
            loop_clip: None,
            loop_pos: 0.,
            preview_clip: None,
            preview_pos: 0.,
            preview_generation: 0,
            beat_phase: 0.,
            beat_was_playing: false,
            beat_step: 0,
            beat_age: 1.,
            beat_noise: Noise::new(91),
            backing_clip: None,
            backing_pos: 0.,
            master_beats: 0.,
            pending: None,
            record_start: 0.,
            record_end: None,
            backing_stretch: sync_stretch::Stretcher::default(),
            loop_stretch: sync_stretch::Stretcher::default(),
        };
        Some(self.sound.processor(None, Some(Box::new(engine))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<AudioBus> = ctx.get();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let media = std::env::var_os("PORTAMAX_MEDIA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("media"));
    let dir = (!cfg!(test)).then(|| root.join("saves/chop"));
    let mut chop = Chop::new(
        Sound::new(NAME, &modbus, &mixer, &bus),
        Arc::clone(&bus),
        ctx.try_get(),
        dir,
        media,
    );
    chop.samples = root.join("samples");
    Box::new(chop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, pitch_hz, render};

    fn tone(hz: f32, secs: f32) -> Vec<f32> {
        (0..(secs * SR) as usize)
            .map(|i| (i as f32 * std::f32::consts::TAU * hz / SR).sin() * 0.5)
            .collect()
    }

    fn chop_with_input() -> (Chop, Arc<Mutex<Vec<f32>>>) {
        let bus = Arc::new(AudioBus::new());
        let input = bus.register("Hardware input");
        (
            Chop::new(
                Sound::detached(),
                bus,
                None,
                None,
                PathBuf::from("/nonexistent"),
            ),
            input,
        )
    }

    #[test]
    fn pads_button_and_view_setting_follow_the_visible_tabs() {
        let (mut c, _) = chop_with_input();
        let tabs = [
            (0, "PLAY"),
            (4, "SLICES"),
            (1, "EDIT"),
            (2, "INPUT"),
            (3, "FX"),
            (5, "LOOP"),
        ];
        for _ in 0..2 {
            for (id, label) in tabs {
                assert_eq!(c.view, id);
                assert_eq!(c.grid_mode_label(), Some(label));
                c.row = 7;
                c.toggle_grid_mode();
                assert_eq!(c.row, 0);
            }
        }
        for (i, (id, _)) in tabs.iter().enumerate() {
            c.native_action(2000, *id as f32);
            c.toggle_grid_mode();
            assert_eq!(c.view, tabs[(i + 1) % 6].0);
            c.controller_adjust(0, -1);
            assert_eq!(c.view, *id);
            c.slint_pointer_pick(95. + i as f32 * 54., 12.);
            c.slint_pointer_pick(-1., -1.);
            assert_eq!(c.view, *id);
        }
    }

    #[test]
    fn short_hits_stay_natural_and_source_beats_can_be_assigned_explicitly() {
        assert_eq!(guess_source_beats(4800, 120.), 0.);
        let (mut c, _) = chop_with_input();
        c.s.pads.lock().unwrap()[0] = Pad {
            data: Some(Arc::new(tone(220., 0.1))),
            set: PadSet::default(),
        };
        c.native_action(2062, 1.);
        assert_eq!(c.s.pads.lock().unwrap()[0].set.beats, 0.25);
        c.native_action(2062, -1.);
        assert_eq!(c.s.pads.lock().unwrap()[0].set.beats, 0.);
        c.s.pads.lock().unwrap()[0].set.beats = 4.;
        c.native_action(2062, -1.);
        assert_eq!(c.s.pads.lock().unwrap()[0].set.beats, 3.);
    }

    #[test]
    fn sync_launches_on_the_audio_grid_and_a_release_keeps_one_shots_queued() {
        let (mut c, _) = chop_with_input();
        c.s.sync.store(true, Ordering::Relaxed);
        c.s.tempo.set(240.);
        for i in 0..2 {
            c.s.pads.lock().unwrap()[i] = Pad {
                data: Some(Arc::new(tone(220., 1.))),
                set: PadSet::default(),
            };
        }
        let mut p = c.audio_processor().unwrap();
        c.native_action(2001, 0.);
        render(&mut p, 1);
        assert!(c.s.beat_playing.load(Ordering::Relaxed));
        c.native_action(2001, 1.);
        c.native_action(2002, 1.);
        render(&mut p, 4);
        assert!(c.s.playhead[0].get() >= 0.);
        assert_eq!(c.s.playhead[1].get(), -1.);
        render(&mut p, 1);
        assert_eq!(c.s.playhead[0].get(), -1.);
        assert!(c.s.playhead[1].get() > 0.);
        let played = c.s.playhead[1].get() * 48000.;
        assert!(
            (played - 72.).abs() < 3.,
            "launch at frame 3000, got {played}"
        );
        c.native_action(2001, 0.);
        render(&mut p, 1);
        assert!(c.s.queued_pad.load(Ordering::Relaxed) > 0);
        c.native_action(2041, 0.);
        render(&mut p, 1);
        assert!(!c.s.beat_playing.load(Ordering::Relaxed));
        assert_eq!(c.s.queued_pad.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn fitting_a_sample_changes_duration_without_changing_pitch() {
        let (mut c, _) = chop_with_input();
        c.s.sync.store(true, Ordering::Relaxed);
        c.s.tempo.set(120.);
        *c.s.backing_clip.lock().unwrap() = Some(Arc::new(vec![0.; 48000]));
        c.s.pads.lock().unwrap()[0] = Pad {
            data: Some(Arc::new(tone(440., 1.))),
            set: PadSet {
                beats: 4.,
                ..PadSet::default()
            },
        };
        let mut p = c.audio_processor().unwrap();
        c.native_action(2001, 0.);
        let out = render(&mut p, 187);
        assert!(c.s.playhead[0].get() > 0.99);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let hz = pitch_hz(&left[8000..80000], SR);
        assert!((hz - 440.).abs() < 12., "pitch was {hz}");
        render(&mut p, 2);
        assert_eq!(c.s.playhead[0].get(), -1.);
    }

    #[test]
    fn backing_follows_tempo_without_a_pitch_shift() {
        let (mut c, _) = chop_with_input();
        c.s.sync.store(true, Ordering::Relaxed);
        c.s.tempo.set(240.);
        c.s.backing_beats.set(4.);
        *c.s.backing_clip.lock().unwrap() = Some(Arc::new(tone(330., 2.)));
        c.s.beat_playing.store(true, Ordering::Relaxed);
        let mut p = c.audio_processor().unwrap();
        let out = render(&mut p, 250);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let hz = pitch_hz(&left[8000..40000], SR);
        assert!((hz - 330.).abs() < 12., "pitch was {hz}");
        assert!((c.s.beat_position.get() - 250. * 512. * 4. / 48000.).abs() < 0.001);
    }

    #[test]
    fn synced_take_captures_the_quantized_attack_and_closes_on_a_whole_bar() {
        let (mut c, _) = chop_with_input();
        c.s.sync.store(true, Ordering::Relaxed);
        c.s.tempo.set(240.);
        *c.s.backing_clip.lock().unwrap() = Some(Arc::new(vec![0.; 48000]));
        c.s.beat_playing.store(true, Ordering::Relaxed);
        c.s.pads.lock().unwrap()[0] = Pad {
            data: Some(Arc::new(
                (0..96000)
                    .map(|i| (i as f32 * 440. * std::f32::consts::TAU / SR).cos() * 0.5)
                    .collect(),
            )),
            set: PadSet::default(),
        };
        let mut p = c.audio_processor().unwrap();
        render(&mut p, 1);
        c.record(3);
        c.native_action(2001, 0.);
        render(&mut p, 20);
        c.record(3);
        assert_eq!(c.s.recording.load(Ordering::Relaxed), 3);
        render(&mut p, 100);
        assert_eq!(c.s.recording.load(Ordering::Relaxed), 0);
        c.tick(&Input::default());
        let clip = c.s.loop_clip.lock().unwrap().clone().unwrap();
        assert!(
            (clip.len() as i32 - 48000).abs() <= 1,
            "{} frames",
            clip.len()
        );
        assert!(clip[0] > 0.01, "first attack must be recorded");
        assert_eq!(c.s.loop_beats.get(), 4.);
        render(&mut p, 500);
        let phase = (c.s.beat_position.get() - c.s.loop_origin.get()).rem_euclid(4.) / 4.;
        assert!((c.s.loop_head.get() - phase).abs() < 0.0001);
        c.s.tempo.set(120.);
        let slower = render(&mut p, 500);
        let phase = (c.s.beat_position.get() - c.s.loop_origin.get()).rem_euclid(4.) / 4.;
        assert!((c.s.loop_head.get() - phase).abs() < 0.0001);
        let left: Vec<f32> = slower.iter().step_by(2).copied().collect();
        let hz = pitch_hz(&left[10000..40000], SR);
        assert!(
            (hz - 440.).abs() < 12.,
            "recorded loop pitch after tempo change: {hz}"
        );
    }

    #[test]
    fn loaded_backing_loops_independently_and_survives_kit_save() {
        let (mut c, _) = chop_with_input();
        let dir = std::env::temp_dir().join(format!("chop-backing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Drums.wav");
        write_wav(&path, &tone(440., 0.1)).unwrap();
        c.media = dir.clone();
        c.dir = Some(dir.join("kit"));
        c.refresh_library();
        c.native_action(2047, 0.);
        assert!(!c.backing_name.is_empty());
        assert!(c.s.pads.lock().unwrap().iter().all(|p| p.data.is_none()));
        let mut p = c.audio_processor().unwrap();
        c.record(3);
        c.native_action(2041, 0.);
        assert!(energy(&render(&mut p, 40)) > 0.001);
        assert!(c.s.loop_armed.load(Ordering::Acquire));
        assert_eq!(c.s.rec_len.load(Ordering::Relaxed), 0);
        c.s.sync.store(true, Ordering::Relaxed);
        c.s.grid.store(3, Ordering::Relaxed);
        c.s.backing_beats.set(7.);
        c.save_bank();
        c.s.backing_clip.lock().unwrap().take();
        c.backing_name.clear();
        c.load_bank(false);
        assert!(c.s.backing_clip.lock().unwrap().is_some());
        assert_eq!(c.backing_name, "Drums.wav");
        assert!(!c.s.beat_playing.load(Ordering::Relaxed));
        assert!(c.s.sync.load(Ordering::Relaxed));
        assert_eq!(c.s.grid.load(Ordering::Relaxed), 3);
        assert_eq!(c.s.backing_beats.get(), 7.);
        assert!(path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn erasing_requires_confirmation_and_removes_only_the_requested_bank() {
        let (mut c, _) = chop_with_input();
        for i in [0, 16, 32] {
            c.s.pads.lock().unwrap()[i] = Pad {
                data: Some(Arc::new(tone(220., 0.3))),
                set: PadSet {
                    name: format!("Pad {i}"),
                    ..PadSet::default()
                },
            };
        }
        c.added_banks = 3;
        c.bank = 1;
        c.pad = 16;
        let mut p = c.audio_processor().unwrap();
        c.native_action(2001, 16.);
        render(&mut p, 1);
        c.native_action(2052, 0.);
        assert!(c.s.pads.lock().unwrap()[16].data.is_some());
        c.native_action(2050, 0.);
        c.native_action(2053, 0.);
        c.native_action(2052, 0.);
        assert_eq!(c.bank_count(), 3);
        c.native_action(2050, 0.);
        c.native_action(2052, 0.);
        assert_eq!(c.bank_count(), 2);
        assert_eq!(c.s.pads.lock().unwrap()[16].set.name, "Pad 32");
        assert_eq!(c.s.pads.lock().unwrap()[0].set.name, "Pad 0");
        render(&mut p, 1);
        assert!(c.s.playhead.iter().all(|p| p.get() < 0.));
        c.native_action(2051, 0.);
        assert!(c.s.pads.lock().unwrap()[0].data.is_some());
        c.native_action(2052, 0.);
        assert!(c.s.pads.lock().unwrap().iter().all(|p| p.data.is_none()));
        assert_eq!(c.bank_count(), 1);
        c.native_action(2052, 0.);
        assert_eq!(c.bank_count(), 1);
    }

    #[test]
    fn library_scans_nested_samples_and_loads_real_audio() {
        let (mut c, _) = chop_with_input();
        let root = std::env::temp_dir().join(format!("chop-library-roots-{}", std::process::id()));
        c.media = root.join("media");
        c.samples = root.join("samples");
        std::fs::create_dir_all(&c.media).unwrap();
        std::fs::create_dir_all(c.samples.join("Drum loops/Nested")).unwrap();
        write_wav(&c.media.join("Media.wav"), &tone(220., 0.1)).unwrap();
        let sample = c.samples.join("Drum loops/Nested/Loop.wav");
        write_wav(&sample, &tone(440., 0.2)).unwrap();
        c.refresh_library();
        assert_eq!(c.library.len(), 2);
        c.lib_index = c.library.iter().position(|p| p == &sample).unwrap();
        c.native_action(2004, 0.);
        assert!(c.s.pads.lock().unwrap()[0].data.is_some());
        let mut processor = c.audio_processor().unwrap();
        c.native_action(2001, 0.);
        assert!(energy(&render(&mut processor, 5)) > 0.001);
        // Rescanning overlapping roots must not duplicate library entries.
        c.samples = c.media.clone();
        c.refresh_library();
        assert_eq!(c.library.len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn library_search_keeps_real_file_indices_and_decodes_other_formats() {
        let (mut c, _) = chop_with_input();
        c.library = vec![
            PathBuf::from("drums/kick.wav"),
            PathBuf::from("vocals/Hello.wav"),
        ];
        c.slint_library_search("HELLO");
        let SlintExtra::Chop(extra) = c.slint_extra() else {
            panic!()
        };
        assert_eq!(extra.library_ids, vec![1]);
        assert_eq!(extra.library_selected, 1);
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/stereo.flac");
        let data = read_audio(&file).unwrap();
        assert!((data.len() as i32 - 4800).abs() <= 1);
        assert!(data.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn one_shots_choke_across_banks_and_ignore_release() {
        let (mut c, _) = chop_with_input();
        for i in [0, 16] {
            c.s.pads.lock().unwrap()[i] = Pad {
                data: Some(Arc::new(tone(220., 1.))),
                set: PadSet::default(),
            };
        }
        let mut p = c.audio_processor().unwrap();
        c.native_action(2001, 0.);
        render(&mut p, 1);
        assert!(c.s.playhead[0].get() >= 0.);
        c.native_action(2001, 16.);
        render(&mut p, 1);
        assert_eq!(c.s.playhead[0].get(), -1.);
        assert!(c.s.playhead[16].get() >= 0.);
        c.native_action(2002, 0.);
        c.native_action(2002, 16.);
        render(&mut p, 2);
        assert!(c.s.playhead[16].get() >= 0.);
    }

    #[test]
    fn backing_beat_is_audible_without_starting_armed_recording() {
        let (mut c, _) = chop_with_input();
        let mut p = c.audio_processor().unwrap();
        c.record(3);
        c.native_action(2041, 0.);
        let audio = render(&mut p, 50);
        assert!(audio.iter().map(|v| v * v).sum::<f32>() > 1.);
        assert!(c.s.loop_armed.load(Ordering::Acquire));
        assert_eq!(c.s.rec_len.load(Ordering::Relaxed), 0);
        c.native_action(2041, 0.);
        assert!(render(&mut p, 5).iter().all(|v| v.abs() < 1e-6));
        c.native_action(2042, 1000.);
        assert_eq!(c.s.tempo.get(), 240.);
        c.native_action(2042, -1000.);
        assert_eq!(c.s.tempo.get(), 40.);
    }

    #[test]
    fn empty_banks_are_explicit_and_stay_available() {
        let (mut c, _) = chop_with_input();
        c.native_action(2040, 0.);
        assert_eq!(c.bank, 1);
        assert_eq!(c.bank_count(), 2);
        c.change_bank(-1);
        assert_eq!(c.bank_count(), 2);
        for _ in 0..20 {
            c.native_action(2040, 0.);
        }
        assert_eq!(c.bank_count(), 8);
        assert_eq!(c.bank, 7);
    }

    #[test]
    fn preview_never_assigns_pads_or_triggers_armed_recording() {
        let (mut c, _) = chop_with_input();
        let dir = std::env::temp_dir().join(format!("chop-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write_wav(&dir.join("preview.wav"), &tone(330., 0.1)).unwrap();
        c.media = dir.clone();
        c.refresh_library();
        let mut p = c.audio_processor().unwrap();
        c.record(3);
        c.audition();
        let audio = render(&mut p, 5);
        assert!(audio.iter().any(|v| v.abs() > 1e-5));
        assert!(c.s.loop_armed.load(Ordering::Acquire));
        assert_eq!(c.s.rec_len.load(Ordering::Relaxed), 0);
        assert!(c.s.pads.lock().unwrap().iter().all(|p| p.data.is_none()));
        render(&mut p, 20);
        assert!(!c.s.preview_playing.load(Ordering::Acquire));
        std::fs::remove_dir_all(dir).unwrap();
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
        let len = c.s.pads.lock().unwrap()[5]
            .data
            .as_ref()
            .map_or(0, |d| d.len());
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
            assert!(
                (*a as i32 - (k as i32 * 12_000 + 2000)).abs() <= 240,
                "{k}: {a}"
            );
        }
    }

    #[test]
    fn resampling_captures_the_effects() {
        let (mut c, _input) = chop_with_input();
        c.s.pads.lock().unwrap()[0] = Pad {
            data: Some(Arc::new(tone(220.0, 0.5))),
            set: PadSet::default(),
        };
        // Lo-fi at 4 bits: the resampled pad has few distinct levels.
        *c.s.fx.lock().unwrap() = [
            FxSlot {
                kind: 2,
                a: 1.0,
                b: 0.0,
            },
            FxSlot {
                kind: 0,
                a: 0.5,
                b: 0.5,
            },
        ];
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
        let d = pads[1]
            .data
            .as_ref()
            .expect("resampled into the next empty pad, 2");
        let mut levels: Vec<i32> = d.iter().map(|x| (x * 1000.0).round() as i32).collect();
        levels.sort();
        levels.dedup();
        assert!(
            levels.len() < 40,
            "crushed to a few levels: {}",
            levels.len()
        );
    }

    #[test]
    fn every_effect_is_bounded_and_passes_sound() {
        for kind in 0..FX_NAMES.len() {
            let mut fx = Fx::new(3);
            let slot = FxSlot {
                kind,
                a: 0.7,
                b: 0.6,
            };
            let t = tone(330.0, 0.5);
            let out: Vec<(f32, f32)> = t
                .iter()
                .map(|&x| fx.tick(slot, 120.0, (x, x), SR))
                .collect();
            let e: f32 = out.iter().map(|(l, r)| l * l + r * r).sum::<f32>() / out.len() as f32;
            assert!(e > 1e-4, "{}: silent", FX_NAMES[kind]);
            assert!(
                out.iter()
                    .all(|(l, r)| l.is_finite() && l.abs() < 4.0 && r.abs() < 4.0),
                "{}",
                FX_NAMES[kind]
            );
        }
    }

    #[test]
    fn notes_play_pads_and_a_bank_saves() {
        let bus = Arc::new(NoteBus::new());
        let dir = std::env::temp_dir().join(format!("portamax-chop-{}", std::process::id()));
        let mut c = Chop::new(
            Sound::detached(),
            Arc::new(AudioBus::new()),
            Some(Arc::clone(&bus)),
            Some(dir.clone()),
            PathBuf::from("/nonexistent"),
        );
        c.s.pads.lock().unwrap()[2] = Pad {
            data: Some(Arc::new(tone(500.0, 0.3))),
            set: PadSet {
                pitch: 3.0,
                ..PadSet::default()
            },
        };
        let mut p = c.audio_processor().unwrap();
        let inbox = bus.inbox_ref(bus.instrument_index(NAME).unwrap());
        inbox.note_on(38, 100);
        render(&mut p, 2);
        assert!(energy(&render(&mut p, 5)) > 1e-4);
        c.save_bank();
        let mut d = Chop::new(
            Sound::detached(),
            Arc::new(AudioBus::new()),
            None,
            Some(dir.clone()),
            PathBuf::from("/nonexistent"),
        );
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
    #[test]
    fn file_touch_load_maps_sixteen_one_shots_and_keeps_time_alignment() {
        let dir = std::env::temp_dir().join(format!("rove-chop-loop-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut data = tone(220., 1.);
        data[..1000].fill(0.);
        write_wav(&dir.join("drums.wav"), &data).unwrap();
        let (mut c, _) = chop_with_input();
        c.media = dir.clone();
        c.on_enter();
        c.slint_pointer_pick(40., 255.);
        c.slint_pointer_pick(-1., -1.);
        assert_eq!(c.view, 4);
        let pads = c.s.pads.lock().unwrap();
        for (i, pad) in pads.iter().take(16).enumerate() {
            assert_eq!(pad.set.mode, Mode::OneShot);
            assert!((pad.set.start - i as f32 / 16.).abs() < 1e-6);
            assert!((pad.set.end - (i + 1) as f32 / 16.).abs() < 1e-6);
            assert_eq!(pad.data.as_ref().unwrap().len(), data.len());
            assert!(Arc::ptr_eq(
                pad.data.as_ref().unwrap(),
                pads[0].data.as_ref().unwrap()
            ));
        }
        assert_eq!(
            pads[0].data.as_ref().unwrap()[500],
            0.,
            "import keeps leading silence/timing"
        );
        drop(pads);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn manual_markers_move_and_assign_ranges_and_touch_pads_release() {
        let (mut c, _) = chop_with_input();
        c.source_sample = Some(Arc::new(tone(330., 1.)));
        let tap = |c: &mut Chop, x, y| {
            c.slint_pointer_pick(x, y);
            c.slint_pointer_pick(-1., -1.);
        };
        tap(&mut c, 240., 255.); // Manual, with one fixed start marker.
        for fraction in [0.25, 0.5, 0.75] {
            tap(&mut c, 334. + 294. * fraction, 70.);
        }
        assert_eq!(c.markers.len(), 4);
        c.slint_pointer_pick(334. + 294. * 0.5, 70.);
        c.slint_pointer_pick(334. + 294. * 0.6, 70.);
        c.slint_pointer_pick(-1., -1.);
        assert!((c.markers[2] as f32 / SR - 0.6).abs() < 0.001);
        tap(&mut c, 400., 285.); // Apply markers.
        let pads = c.s.pads.lock().unwrap();
        assert_eq!(pads.iter().filter(|p| p.data.is_some()).count(), 4);
        assert!((pads[1].set.end - 0.6).abs() < 0.001);
        assert!((pads[2].set.start - 0.6).abs() < 0.001);
        drop(pads);
        tap(&mut c, 20., 50.);
        assert_eq!(
            *c.s.live.lock().unwrap(),
            vec![(0, 1., true), (0, 0., false)]
        );
        let mut processor = c.audio_processor().unwrap();
        assert!(
            energy(&render(&mut processor, 10)) > 1e-4,
            "one shot continues after release"
        );
        assert!(
            energy(&render(&mut processor, 30)[20 * 1024..]) < 1e-6,
            "slice finishes rather than looping"
        );
    }

    #[test]
    fn performance_recording_repeats_after_pad_release_and_can_pause_and_save() {
        let (mut c, _) = chop_with_input();
        c.s.pads.lock().unwrap()[0] = Pad {
            data: Some(Arc::new(tone(220., 0.1))),
            set: PadSet::default(),
        };
        let mut processor = c.audio_processor().unwrap();
        c.record(3);
        render(&mut processor, 40);
        assert!(c.s.loop_armed.load(Ordering::Acquire));
        assert_eq!(
            c.s.rec_len.load(Ordering::Relaxed),
            0,
            "armed waiting time must not be captured"
        );
        c.s.live.lock().unwrap().push((1, 1., true)); // Empty pad does not start.
        render(&mut processor, 1);
        assert!(c.s.loop_armed.load(Ordering::Acquire));
        c.s.live.lock().unwrap().push((0, 1., true));
        render(&mut processor, 2);
        c.s.live.lock().unwrap().push((0, 0., false));
        render(&mut processor, 10);
        c.record(3);
        render(&mut processor, 1);
        c.tick(&Input::default());
        assert!(c.s.loop_playing.load(Ordering::Relaxed));
        let length = c.s.loop_clip.lock().unwrap().as_ref().unwrap().len();
        assert_eq!(length, 12 * 512);
        assert!(
            energy(&render(&mut processor, 60)) > 1e-4,
            "loop remains audible well after original one-shot ends"
        );
        c.toggle_loop();
        assert!(
            energy(&render(&mut processor, 5)) < 1e-6,
            "pause silences loop"
        );
        c.toggle_loop();
        assert!(energy(&render(&mut processor, 20)) > 1e-4);
        let dir =
            std::env::temp_dir().join(format!("rove-chop-performance-{}", std::process::id()));
        c.dir = Some(dir.clone());
        c.save_bank();
        let (mut restored, _) = chop_with_input();
        restored.dir = Some(dir.clone());
        restored.load_bank(false);
        assert_eq!(
            restored.s.loop_clip.lock().unwrap().as_ref().unwrap().len(),
            length
        );
        assert!(
            !restored.s.loop_playing.load(Ordering::Relaxed),
            "restored loop waits for Play"
        );
        c.clear_loop();
        render(&mut processor, 1);
        c.tick(&Input::default());
        assert!(c.s.loop_clip.lock().unwrap().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn arbitrary_slice_counts_and_banked_pads_keep_gate_ownership() {
        let (mut c, _) = chop_with_input();
        c.source_sample = Some(Arc::new(tone(330., 1.)));
        for count in [1, 7, 23, 128] {
            c.slice_count = count;
            c.auto_slice(3);
            let pads = c.s.pads.lock().unwrap();
            assert_eq!(pads.iter().filter(|p| p.data.is_some()).count(), count);
            assert!((pads[count - 1].set.end - 1.).abs() < 1e-6);
            assert_eq!(pads[count - 1].set.mode, Mode::OneShot);
        }
        c.slice_count = 23;
        c.auto_slice(3);
        c.change_bank(1);
        assert_eq!(c.bank, 1);
        c.tick(&Input {
            grid: std::array::from_fn(|i| i == 0),
            ..Default::default()
        });
        assert_eq!(c.pad, 16);
        c.change_bank(-1);
        c.tick(&Input::default());
        assert_eq!(
            *c.s.live.lock().unwrap(),
            vec![(16, 1., true), (16, 0., false)]
        );
        c.s.live.lock().unwrap().clear();
        c.change_bank(1);
        c.slint_pointer_pick(20., 50.);
        c.slint_pointer_pick(-1., -1.);
        assert_eq!(
            *c.s.live.lock().unwrap(),
            vec![(16, 1., true), (16, 0., false)]
        );
        // More than sixteen voices/slots must not index past MIDI's 128 keys.
        let mut processor = c.audio_processor().unwrap();
        assert!(energy(&render(&mut processor, 3)) > 1e-4);
    }
    #[test]
    fn loop_arm_can_cancel_and_midi_starts_on_the_first_audio_block() {
        let bus = Arc::new(NoteBus::new());
        let mut c = Chop::new(
            Sound::detached(),
            Arc::new(AudioBus::new()),
            Some(bus.clone()),
            None,
            PathBuf::from("/nonexistent"),
        );
        c.s.pads.lock().unwrap()[0] = Pad {
            data: Some(Arc::new(vec![0.5; 48000])),
            set: PadSet::default(),
        };
        let mut processor = c.audio_processor().unwrap();
        c.view = 4; // Global transport must cancel/stop even on the Slice view.
        c.record(3);
        assert_eq!(c.transport_action(), Some("CANCEL ARM"));
        c.toggle_running();
        render(&mut processor, 2);
        assert!(!c.s.loop_armed.load(Ordering::Acquire));
        assert_eq!(c.s.recording.load(Ordering::Relaxed), 0);
        assert!(c.s.finished.lock().unwrap().is_none());
        c.record(3);
        render(&mut processor, 20);
        bus.inbox_ref(bus.instrument_index(NAME).unwrap())
            .note_on(36, 100);
        render(&mut processor, 1);
        assert!(!c.s.loop_armed.load(Ordering::Acquire));
        assert_eq!(c.s.rec_len.load(Ordering::Relaxed), 512);
        assert_eq!(c.transport_action(), Some("END LOOP"));
        c.toggle_running();
        render(&mut processor, 1);
        c.tick(&Input::default());
        let clip = c.s.loop_clip.lock().unwrap().clone().unwrap();
        assert_eq!(clip.len(), 512, "no pre-trigger silence");
        assert!(clip[0] > 0., "first attack sample must be captured");
    }
}
