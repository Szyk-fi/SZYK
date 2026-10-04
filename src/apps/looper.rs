//! Looper: four loops locked to the device clock.
//!
//! Each loop is anchored to the shared transport (clock.rs) -- it
//! remembers the beat it started on and how many beats long it is, and
//! its playback position is worked out from the clock's beat on every
//! sample. So loops can never drift from Session, Skins or each other,
//! loops of different lengths phase against each other exactly, and if
//! the tempo changes the loops follow it like varispeed tape (pitch
//! moves with speed -- the honest thing a sample-based looper without
//! time-stretching does).
//!
//! The first loop doesn't need a clock: with the transport stopped,
//! recording starts the moment you press, and closing it *sets the
//! tempo* -- it picks the number of bars that puts the tempo in a
//! musical range -- and starts the clock, so drums and sequences can
//! join in time with what you just played.
//!
//! With the clock running, recording waits for the next bar (or beat,
//! or not at all -- the Quantize setting) and closes on a bar line, so
//! loops are whole bars. A press up to a quarter-beat late closes on the
//! bar just passed instead of a whole bar later -- the usual human
//! late press.
//!
//! Overdubs are undoable (once, and redoable): the first pass of an
//! overdub copies what it's about to overwrite into the undo buffer,
//! and undo swaps the two, so nothing is allocated on the audio thread.
//!
//! Not modeled: input latency compensation. On the device the codec's
//! round trip is known and would shift the write head back by it; the
//! simulator's host audio latency isn't knowable from in here.

use crate::app::{App, Input, SlintExtra};
use crate::apps::kids_kit::{self as kit, Extra, Size2, Sound};
use crate::audio_bus::AudioBus;
use crate::clock::{Clock, Snap, Source};
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Looper";
pub const TRACKS: usize = 4;
const SR: f32 = 48_000.0;
/// Per loop. On the device this is 16-bit in PSRAM; 30 s is a 16-bar
/// loop down to 128 bpm.
pub const MAX_SECS: f32 = 30.0;
const MAX_LEN: usize = (MAX_SECS * SR) as usize;
/// Samples per overview peak.
const BIN: usize = 256;
const NO_SOURCE: usize = usize::MAX;

pub const EMPTY: u8 = 0;
/// Waiting for the next bar (or beat) to start recording.
pub const ARMED: u8 = 1;
pub const REC: u8 = 2;
pub const PLAY: u8 = 3;
pub const DUB: u8 = 4;
pub const MUTED: u8 = 5;
/// Recording the first loop with the clock stopped.
pub const FREE_REC: u8 = 6;
/// Fading out before clearing.
const CLEARING: u8 = 7;

const STATE_NAMES: [&str; 8] = ["empty", "armed", "REC", "play", "DUB", "muted", "REC", "clear"];

const QUANT_NAMES: [&str; 3] = ["bar", "beat", "off"];
const LENGTHS: [u32; 6] = [0, 1, 2, 4, 8, 16];

#[derive(Clone, Copy, Debug)]
enum Cmd {
    Rec(usize),
    Mute(usize),
    Undo(usize),
    Clear(usize),
    /// A loop was loaded into `data` by the UI: (track, samples, beats).
    Loaded(usize, usize, f64),
}

pub struct Data {
    pub buf: Vec<f32>,
    pub undo: Vec<f32>,
}

pub struct TrackShared {
    pub state: AtomicU8,
    pub len: AtomicUsize,
    len_beats: AtomicU64,
    /// Playhead, 0..1 of the loop.
    pub pos: AtomicF32,
    /// Samples recorded so far, while recording.
    pub rec_len: AtomicUsize,
    pub level: AtomicF32,
    pub pan: AtomicF32,
    pub reverse: AtomicBool,
    /// How much of the old layer survives each overdub pass.
    pub feedback: AtomicF32,
    /// 0 nothing to undo, 1 undo available, 2 redo available.
    pub undo: AtomicU8,
    /// Peak per BIN samples, for the screen.
    pub peaks: Vec<AtomicF32>,
    pub data: Mutex<Data>,
}

impl TrackShared {
    fn new() -> TrackShared {
        TrackShared {
            state: AtomicU8::new(EMPTY),
            len: AtomicUsize::new(0),
            len_beats: AtomicU64::new(0f64.to_bits()),
            pos: AtomicF32::new(0.0),
            rec_len: AtomicUsize::new(0),
            level: AtomicF32::new(0.8),
            pan: AtomicF32::new(0.0),
            reverse: AtomicBool::new(false),
            feedback: AtomicF32::new(1.0),
            undo: AtomicU8::new(0),
            peaks: (0..MAX_LEN / BIN + 1).map(|_| AtomicF32::new(0.0)).collect(),
            data: Mutex::new(Data { buf: Vec::new(), undo: Vec::new() }),
        }
    }
    pub fn beats(&self) -> f64 {
        f64::from_bits(self.len_beats.load(Ordering::Relaxed))
    }
    pub fn st(&self) -> u8 {
        self.state.load(Ordering::Relaxed)
    }
}

pub struct Shared {
    pub tracks: Vec<TrackShared>,
    cmds: Mutex<Vec<Cmd>>,
    source: Mutex<Option<Arc<Mutex<Vec<f32>>>>>,
    pub monitor: AtomicBool,
    pub quantize: AtomicU8,
    /// Index into LENGTHS (0 = until you press again).
    pub length: AtomicU8,
    pub level: AtomicF32,
}

impl Shared {
    fn send(&self, c: Cmd) {
        if let Ok(mut q) = self.cmds.lock() {
            if q.len() < 64 {
                q.push(c);
            }
        }
    }
}

/// Per-loop state only the audio thread touches.
#[derive(Clone, Copy, Default)]
struct Rt {
    anchor: f64,
    len_beats: f64,
    /// Samples per beat when it was recorded.
    spb: f64,
    len: usize,
    write: usize,
    arm_at: Option<f64>,
    stop_at: Option<f64>,
    dub_start: usize,
    dub_n: usize,
    gain: f32,
    /// A free recording closing: frames still to record this block.
    closing: bool,
}

struct Engine {
    s: Arc<Shared>,
    clock: Arc<Clock>,
    rt: [Rt; TRACKS],
    snap: Snap,
    fi: usize,
    input: Vec<f32>,
    sr: f32,
    peak: f32,
}

/// Bars (from 1, 2, 4, 8, 16) that put a free loop of `secs` nearest
/// to 110 bpm without leaving 70-160, and that tempo.
pub fn tempo_for(secs: f64, beats_per_bar: f64) -> (f64, f64) {
    let mut best = (1.0, 60.0 * beats_per_bar / secs.max(1e-3));
    let mut best_err = f64::MAX;
    for bars in [1.0, 2.0, 4.0, 8.0, 16.0] {
        let bpm = 60.0 * bars * beats_per_bar / secs.max(1e-3);
        let err = if (70.0..160.0).contains(&bpm) { (bpm / 110.0).ln().abs() } else { 10.0 + (bpm / 110.0).ln().abs() };
        if err < best_err {
            best_err = err;
            best = (bars, bpm);
        }
    }
    best
}

impl Engine {
    fn set_state(&self, t: usize, st: u8) {
        self.s.tracks[t].state.store(st, Ordering::Relaxed);
    }

    fn quantum(&self) -> f64 {
        match self.s.quantize.load(Ordering::Relaxed) {
            0 => self.clock.bar_beats(),
            1 => 1.0,
            _ => 0.0,
        }
    }

    fn clear_peaks(&self, t: usize) {
        for p in &self.s.tracks[t].peaks {
            p.set(0.0);
        }
    }

    fn rescan_peaks(&self, t: usize, data: &Data, len: usize) {
        for (b, p) in self.s.tracks[t].peaks.iter().enumerate() {
            let a = b * BIN;
            if a >= len {
                p.set(0.0);
                continue;
            }
            let m = data.buf[a..(a + BIN).min(len)].iter().fold(0.0f32, |m, x| m.max(x.abs()));
            p.set(m);
        }
    }

    fn command(&mut self, c: Cmd, frames: usize) {
        let beat = self.snap.beat;
        match c {
            Cmd::Rec(t) => {
                let st = self.s.tracks[t].st();
                match st {
                    EMPTY => {
                        self.clear_peaks(t);
                        self.rt[t] = Rt { gain: self.rt[t].gain, ..Rt::default() };
                        self.s.tracks[t].undo.store(0, Ordering::Relaxed);
                        self.s.tracks[t].rec_len.store(0, Ordering::Relaxed);
                        let ext = self.clock.source() == Source::MidiIn;
                        // (`running()` counts a start that's been pressed
                        // but not reached the block boundary yet)
                        if self.clock.running() || ext {
                            self.set_state(t, ARMED);
                        } else {
                            self.set_state(t, FREE_REC);
                        }
                    }
                    ARMED => self.set_state(t, EMPTY),
                    REC => {
                        let rt = &mut self.rt[t];
                        let q = match self.s.quantize.load(Ordering::Relaxed) {
                            0 => self.clock.bar_beats(),
                            1 => 1.0,
                            _ => 0.0,
                        };
                        let into = beat - rt.anchor;
                        rt.stop_at = Some(if q <= 0.0 {
                            beat
                        } else {
                            let done = (into / q).floor();
                            // up to a quarter beat late closes the bar just passed
                            if done >= 1.0 && into - done * q < 0.25 {
                                rt.anchor + done * q
                            } else {
                                rt.anchor + (done + 1.0).max(1.0) * q
                            }
                        });
                    }
                    FREE_REC => {
                        if !self.rt[t].closing {
                            self.rt[t].closing = true;
                            let n = self.rt[t].write + frames;
                            let bpb = self.clock.bar_beats();
                            if self.snap.running {
                                // Someone started the clock meanwhile: fit to it.
                                let spb = self.sr as f64 * 60.0 / self.snap.bpm as f64;
                                let beats = ((n as f64 / spb / bpb).round().max(1.0)) * bpb;
                                let rt = &mut self.rt[t];
                                rt.len_beats = beats;
                                rt.spb = n as f64 / beats;
                                rt.anchor = beat + frames as f64 / spb - beats;
                            } else {
                                let (bars, bpm) = tempo_for(n as f64 / self.sr as f64, bpb);
                                self.clock.set_bpm(bpm as f32);
                                self.clock.start();
                                let rt = &mut self.rt[t];
                                rt.len_beats = bars * bpb;
                                rt.spb = n as f64 / rt.len_beats;
                                rt.anchor = 0.0;
                            }
                            self.rt[t].len = n;
                        }
                    }
                    PLAY | MUTED => {
                        let rt = &mut self.rt[t];
                        rt.dub_start = rt.write.min(rt.len.saturating_sub(1));
                        rt.dub_n = 0;
                        self.s.tracks[t].undo.store(0, Ordering::Relaxed);
                        self.set_state(t, DUB);
                    }
                    DUB => self.end_dub(t),
                    _ => {}
                }
            }
            Cmd::Mute(t) => match self.s.tracks[t].st() {
                PLAY => self.set_state(t, MUTED),
                MUTED => self.set_state(t, PLAY),
                DUB => {
                    self.end_dub(t);
                    self.set_state(t, MUTED);
                }
                ARMED | REC | FREE_REC => self.set_state(t, EMPTY),
                _ => {}
            },
            Cmd::Undo(t) => {
                if self.s.tracks[t].st() == DUB {
                    self.end_dub(t);
                }
                let u = self.s.tracks[t].undo.load(Ordering::Relaxed);
                if u != 0 {
                    let rt = self.rt[t];
                    if let Ok(mut d) = self.s.tracks[t].data.try_lock() {
                        let n = rt.dub_n.min(rt.len);
                        let Data { buf, undo } = &mut *d;
                        for k in 0..n {
                            let i = (rt.dub_start + k) % rt.len;
                            std::mem::swap(&mut buf[i], &mut undo[i]);
                        }
                        self.rescan_peaks(t, &d, rt.len);
                        self.s.tracks[t].undo.store(if u == 1 { 2 } else { 1 }, Ordering::Relaxed);
                    }
                }
            }
            Cmd::Clear(t) => {
                if self.s.tracks[t].st() != EMPTY {
                    self.set_state(t, CLEARING);
                }
            }
            Cmd::Loaded(t, len, beats) => {
                self.rt[t] = Rt { len, len_beats: beats, spb: len as f64 / beats.max(1e-6), anchor: 0.0, gain: 0.0, ..Rt::default() };
                if let Ok(d) = self.s.tracks[t].data.try_lock() {
                    self.rescan_peaks(t, &d, len);
                }
                self.s.tracks[t].undo.store(0, Ordering::Relaxed);
                self.publish_len(t);
                self.set_state(t, PLAY);
            }
        }
    }

    fn end_dub(&mut self, t: usize) {
        if self.rt[t].dub_n > 0 {
            self.s.tracks[t].undo.store(1, Ordering::Relaxed);
        }
        self.set_state(t, PLAY);
    }

    fn publish_len(&self, t: usize) {
        let tr = &self.s.tracks[t];
        tr.len.store(self.rt[t].len, Ordering::Relaxed);
        tr.len_beats.store(self.rt[t].len_beats.to_bits(), Ordering::Relaxed);
    }

    /// Recording finished: it's a loop now.
    fn close(&mut self, t: usize) {
        let rt = &mut self.rt[t];
        rt.len = rt.len.min(rt.write).max(1);
        self.publish_len(t);
        self.set_state(t, PLAY);
    }
}

impl Extra for Engine {
    fn block(&mut self, frames: usize, sr: f32) {
        self.sr = sr;
        self.fi = 0;
        // input first: commands closing a free loop count this block's frames
        self.input.clear();
        if let Ok(src) = self.s.source.try_lock() {
            if let Some(b) = src.as_ref() {
                if let Ok(b) = b.try_lock() {
                    self.input.extend(b.iter().take(frames));
                }
            }
        }
        self.input.resize(frames, 0.0);
        let p = self.input.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        self.peak = p.max(self.peak * 0.9);
        self.s.level.set(self.peak);

        // Free loops that closed last block become loops on this one,
        // the block the clock starts on.
        for t in 0..TRACKS {
            if self.s.tracks[t].st() == FREE_REC && self.rt[t].closing {
                self.rt[t].closing = false;
                self.rt[t].write = self.rt[t].len;
                self.close(t);
            }
        }
        self.snap = self.clock.snap();
        let cmds: Vec<Cmd> = self.s.cmds.try_lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        for c in cmds {
            self.command(c, frames);
        }
        // The transport stopping abandons a recording waiting on it.
        if !self.snap.running {
            for t in 0..TRACKS {
                match self.s.tracks[t].st() {
                    REC => self.set_state(t, EMPTY),
                    DUB => self.end_dub(t),
                    _ => {}
                }
            }
        }
        let rec = self.s.tracks.iter().any(|t| matches!(t.st(), ARMED | REC | FREE_REC | DUB));
        if self.clock.recording.load(Ordering::Relaxed) != rec {
            self.clock.recording.store(rec, Ordering::Relaxed);
        }
    }

    fn frame(&mut self, sr: f32) -> (f32, f32) {
        let s = Arc::clone(&self.s);
        let i = self.fi;
        self.fi += 1;
        let x = self.input.get(i).copied().unwrap_or(0.0);
        let beat = self.snap.beat_at(i, sr);
        let (mut l, mut r) = (0.0f32, 0.0f32);
        let q = self.quantum();
        for t in 0..TRACKS {
            let tr = &s.tracks[t];
            let mut st = tr.st();
            if st == ARMED && self.snap.running {
                let rt = &mut self.rt[t];
                let at = *rt.arm_at.get_or_insert_with(|| {
                    if q <= 0.0 {
                        beat
                    } else {
                        let k = (beat / q).floor() * q;
                        // already on the boundary (a start from the top): use it
                        if beat - k < 1e-6 {
                            k
                        } else {
                            k + q
                        }
                    }
                });
                if beat >= at - 1e-9 {
                    rt.anchor = at;
                    rt.spb = sr as f64 * 60.0 / self.snap.bpm as f64;
                    rt.write = 0;
                    rt.arm_at = None;
                    let bars = LENGTHS[self.s.length.load(Ordering::Relaxed) as usize % LENGTHS.len()];
                    rt.stop_at = (bars > 0).then(|| at + bars as f64 * self.clock.bar_beats());
                    tr.state.store(REC, Ordering::Relaxed);
                    st = REC;
                }
            }
            let Ok(mut d) = tr.data.try_lock() else { continue };
            if d.buf.len() < MAX_LEN {
                continue;
            }
            match st {
                REC | FREE_REC => {
                    let rt = &mut self.rt[t];
                    if st == REC {
                        if let Some(stop) = rt.stop_at {
                            if beat >= stop - 1e-9 {
                                rt.len_beats = stop - rt.anchor;
                                rt.len = ((rt.len_beats * rt.spb).round() as usize).max(1);
                                drop(d);
                                self.close(t);
                                st = PLAY;
                                // fall through to play this frame
                                let Ok(d2) = tr.data.try_lock() else { continue };
                                d = d2;
                            }
                        }
                    }
                    if st != PLAY {
                        let rt = &mut self.rt[t];
                        if rt.write < MAX_LEN {
                            d.buf[rt.write] = x;
                            let b = &tr.peaks[rt.write / BIN];
                            if x.abs() > b.get() {
                                b.set(x.abs());
                            }
                            rt.write += 1;
                            tr.rec_len.store(rt.write, Ordering::Relaxed);
                        } else if st == REC {
                            // Out of memory: close on the last whole beat.
                            rt.len_beats = (rt.write as f64 / rt.spb).floor().max(1.0);
                            rt.len = ((rt.len_beats * rt.spb) as usize).min(MAX_LEN);
                            drop(d);
                            self.close(t);
                            continue;
                        } else if !rt.closing {
                            // a free loop that hit the limit closes itself
                            let n = rt.write;
                            let (bars, bpm) = tempo_for(n as f64 / sr as f64, self.clock.bar_beats());
                            self.clock.set_bpm(bpm as f32);
                            self.clock.start();
                            let rt = &mut self.rt[t];
                            rt.closing = true;
                            rt.len_beats = bars * self.clock.bar_beats();
                            rt.spb = n as f64 / rt.len_beats;
                            rt.anchor = 0.0;
                            rt.len = n;
                        }
                        continue;
                    }
                }
                _ => {}
            }
            if !matches!(st, PLAY | DUB | MUTED | CLEARING) || !self.snap.running {
                continue;
            }
            let rt = &mut self.rt[t];
            if rt.len == 0 || rt.len_beats <= 0.0 {
                continue;
            }
            let ph = (beat - rt.anchor).rem_euclid(rt.len_beats) / rt.len_beats;
            let pos = ph * rt.len as f64;
            let idx = (pos as usize).min(rt.len - 1);
            rt.write = idx;
            tr.pos.set(ph as f32);
            let rev = tr.reverse.load(Ordering::Relaxed);
            let read = if rev { rt.len as f64 - 1.0 - pos } else { pos }.max(0.0);
            let a = (read as usize).min(rt.len - 1);
            let b = (a + 1) % rt.len;
            let f = (read - a as f64) as f32;
            let y = d.buf[a] + (d.buf[b] - d.buf[a]) * f;
            if st == DUB {
                if rt.dub_n < rt.len {
                    let v = d.buf[idx];
                    d.undo[idx] = v;
                }
                rt.dub_n += 1;
                let fb = tr.feedback.get();
                let v = d.buf[idx] * fb + x;
                d.buf[idx] = v;
                let pk = &tr.peaks[idx / BIN];
                if v.abs() > pk.get() {
                    pk.set(v.abs());
                }
            }
            let target = if st == PLAY || st == DUB { tr.level.get() } else { 0.0 };
            // ~5 ms to fade in or out: mutes and clears never click
            rt.gain += (target - rt.gain) * (1.0 - (-1.0 / (0.005 * sr)).exp());
            if st == CLEARING && rt.gain < 1e-3 {
                rt.gain = 0.0;
                drop(d);
                self.rt[t] = Rt::default();
                self.publish_len(t);
                self.clear_peaks(t);
                tr.undo.store(0, Ordering::Relaxed);
                tr.rec_len.store(0, Ordering::Relaxed);
                tr.pos.set(0.0);
                tr.state.store(EMPTY, Ordering::Relaxed);
                continue;
            }
            let (gl, gr) = kit::pan_gains(tr.pan.get());
            l += y * rt.gain * gl;
            r += y * rt.gain * gr;
        }
        if self.s.monitor.load(Ordering::Relaxed) {
            l += x * 0.7071;
            r += x * 0.7071;
        }
        // The kit's output stage halves and soft-clips, and a centred pan
        // is -3 dB a side; undo both so a loop at full level plays back
        // at the level it went in.
        (l * 2.0 * std::f32::consts::SQRT_2, r * 2.0 * std::f32::consts::SQRT_2)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Loops,
    Setup,
}
const VIEWS: [View; 2] = [View::Loops, View::Setup];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Source,
    Monitor,
    Quantize,
    Length,
    Track,
    Level,
    Pan,
    Reverse,
    Feedback,
    Slot,
    Save,
    Load,
}
const ROWS: [Row; 12] = [Row::Source, Row::Monitor, Row::Quantize, Row::Length, Row::Track, Row::Level, Row::Pan, Row::Reverse, Row::Feedback, Row::Slot, Row::Save, Row::Load];

pub struct Looper {
    sound: Sound,
    pub s: Arc<Shared>,
    clock: Arc<Clock>,
    bus: Arc<AudioBus>,
    source: usize,
    view: usize,
    track: usize,
    row: usize,
    prev: [bool; 16],
    held: [u32; 16],
    slot: usize,
    dir: Option<PathBuf>,
    message: (String, u32),
    frame: u64,
    /// What the transport button will show once the engine catches up.
    pending: Option<bool>,
}

impl Looper {
    pub fn new(sound: Sound, bus: Arc<AudioBus>, clock: Arc<Clock>, dir: Option<PathBuf>) -> Looper {
        let s = Arc::new(Shared {
            tracks: (0..TRACKS).map(|_| TrackShared::new()).collect(),
            cmds: Mutex::new(Vec::with_capacity(64)),
            source: Mutex::new(None),
            monitor: AtomicBool::new(false),
            quantize: AtomicU8::new(0),
            length: AtomicU8::new(0),
            level: AtomicF32::new(0.0),
        });
        sound.set_reverb(0.0);
        sound.set_volume(1.0);
        let names = bus.names();
        let source = names.iter().position(|n| n == "Hardware input").or_else(|| names.iter().position(|n| n.contains("Audio In"))).unwrap_or(NO_SOURCE);
        let mut l = Looper { sound, s, clock, bus, source, view: 0, track: 0, row: 0, prev: [false; 16], held: [0; 16], slot: 0, dir, message: (String::new(), 0), frame: 0, pending: None };
        l.connect();
        l
    }

    fn connect(&mut self) {
        let b = if self.source == NO_SOURCE { None } else { self.bus.get(self.source) };
        *self.s.source.lock().unwrap() = b;
    }

    fn flash(&mut self, m: impl Into<String>) {
        self.message = (m.into(), 180);
    }

    fn st(&self, t: usize) -> u8 {
        self.s.tracks[t].st()
    }

    pub fn rec(&mut self, t: usize) {
        self.s.send(Cmd::Rec(t));
    }
    pub fn mute(&mut self, t: usize) {
        self.s.send(Cmd::Mute(t));
    }
    pub fn undo(&mut self, t: usize) {
        self.s.send(Cmd::Undo(t));
    }
    pub fn clear(&mut self, t: usize) {
        self.s.send(Cmd::Clear(t));
    }

    fn value(&self, r: Row) -> String {
        let tr = &self.s.tracks[self.track];
        match r {
            Row::Source => self.bus.source_name(self.source),
            Row::Monitor => if self.s.monitor.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Row::Quantize => QUANT_NAMES[self.s.quantize.load(Ordering::Relaxed) as usize % 3].into(),
            Row::Length => match LENGTHS[self.s.length.load(Ordering::Relaxed) as usize % LENGTHS.len()] {
                0 => "until pressed".into(),
                1 => "1 bar".into(),
                n => format!("{n} bars"),
            },
            Row::Track => format!("{}", self.track + 1),
            Row::Level => format!("{:.0}", tr.level.get() * 100.0),
            Row::Pan => format!("{:+.0}", tr.pan.get() * 100.0),
            Row::Reverse => if tr.reverse.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Row::Feedback => format!("{:.0}%", tr.feedback.get() * 100.0),
            Row::Slot => format!("{}", self.slot + 1),
            Row::Save => "< > to save".into(),
            Row::Load => "< > to load".into(),
        }
    }

    fn label(r: Row) -> &'static str {
        match r {
            Row::Source => "Input",
            Row::Monitor => "Hear input",
            Row::Quantize => "Start on",
            Row::Length => "Length",
            Row::Track => "Loop",
            Row::Level => "Level",
            Row::Pan => "Pan",
            Row::Reverse => "Reverse",
            Row::Feedback => "Overdub keeps",
            Row::Slot => "Slot",
            Row::Save => "Save",
            Row::Load => "Load",
        }
    }

    fn edit(&mut self, d: i32) {
        let tr = &self.s.tracks[self.track];
        match ROWS[self.row] {
            Row::Source => {
                self.source = crate::audio_bus::cycle_source(self.source, d, self.bus.len());
                self.connect();
            }
            Row::Monitor => {
                let v = !self.s.monitor.load(Ordering::Relaxed);
                self.s.monitor.store(v, Ordering::Relaxed);
            }
            Row::Quantize => {
                let v = (self.s.quantize.load(Ordering::Relaxed) as i32 + d).rem_euclid(3);
                self.s.quantize.store(v as u8, Ordering::Relaxed);
            }
            Row::Length => {
                let v = (self.s.length.load(Ordering::Relaxed) as i32 + d).rem_euclid(LENGTHS.len() as i32);
                self.s.length.store(v as u8, Ordering::Relaxed);
            }
            Row::Track => self.track = (self.track as i32 + d).rem_euclid(TRACKS as i32) as usize,
            Row::Level => tr.level.set((tr.level.get() + d as f32 * 0.05).clamp(0.0, 1.0)),
            Row::Pan => tr.pan.set((tr.pan.get() + d as f32 * 0.1).clamp(-1.0, 1.0)),
            Row::Reverse => {
                let v = !tr.reverse.load(Ordering::Relaxed);
                tr.reverse.store(v, Ordering::Relaxed);
            }
            Row::Feedback => tr.feedback.set((tr.feedback.get() + d as f32 * 0.05).clamp(0.0, 1.0)),
            Row::Slot => self.slot = (self.slot as i32 + d).rem_euclid(8) as usize,
            Row::Save => self.save(),
            Row::Load => self.load(),
        }
    }

    fn slot_dir(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("slot_{}", self.slot + 1)))
    }

    pub fn save(&mut self) {
        let Some(dir) = self.slot_dir() else {
            self.flash("no save folder");
            return;
        };
        if std::fs::create_dir_all(&dir).is_err() {
            self.flash("couldn't make the folder");
            return;
        }
        let mut tracks = Vec::new();
        let mut n = 0;
        for t in 0..TRACKS {
            let tr = &self.s.tracks[t];
            let len = tr.len.load(Ordering::Relaxed);
            let saved = matches!(tr.st(), PLAY | MUTED | DUB) && len > 0;
            if saved {
                // A brief lock: the audio thread skips this loop for at
                // most one block while it's copied.
                let copy: Vec<f32> = tr.data.lock().map(|d| d.buf[..len].to_vec()).unwrap_or_default();
                if crate::apps::chop::write_wav(&dir.join(format!("loop_{}.wav", t + 1)), &copy).is_ok() {
                    n += 1;
                }
            }
            tracks.push(serde_json::json!({
                "saved": saved,
                "beats": tr.beats(),
                "level": tr.level.get(),
                "pan": tr.pan.get(),
                "reverse": tr.reverse.load(Ordering::Relaxed),
                "feedback": tr.feedback.get(),
                "muted": tr.st() == MUTED,
            }));
        }
        let j = serde_json::json!({ "bpm": self.clock.bpm(), "beats_per_bar": self.clock.bar_beats(), "tracks": tracks });
        let _ = std::fs::write(dir.join("looper.json"), serde_json::to_string_pretty(&j).unwrap_or_default());
        self.flash(format!("saved {n} loop{} to slot {}", if n == 1 { "" } else { "s" }, self.slot + 1));
    }

    pub fn load(&mut self) {
        let Some(dir) = self.slot_dir() else { return };
        let Ok(j) = std::fs::read_to_string(dir.join("looper.json")) else {
            self.flash(format!("slot {} is empty", self.slot + 1));
            return;
        };
        let v: serde_json::Value = serde_json::from_str(&j).unwrap_or_default();
        if let Some(b) = v["bpm"].as_f64() {
            self.clock.set_bpm(b as f32);
        }
        if let Some(b) = v["beats_per_bar"].as_f64() {
            self.clock.beats_per_bar.store(b as u8, Ordering::Relaxed);
        }
        let mut n = 0;
        for t in 0..TRACKS {
            let tv = &v["tracks"][t];
            let tr = &self.s.tracks[t];
            tr.level.set(tv["level"].as_f64().unwrap_or(0.8) as f32);
            tr.pan.set(tv["pan"].as_f64().unwrap_or(0.0) as f32);
            tr.reverse.store(tv["reverse"].as_bool().unwrap_or(false), Ordering::Relaxed);
            tr.feedback.set(tv["feedback"].as_f64().unwrap_or(1.0) as f32);
            if !tv["saved"].as_bool().unwrap_or(false) {
                self.s.send(Cmd::Clear(t));
                continue;
            }
            let Ok(mut audio) = crate::apps::chop::read_wav(&dir.join(format!("loop_{}.wav", t + 1))) else { continue };
            audio.truncate(MAX_LEN);
            let beats = tv["beats"].as_f64().unwrap_or(4.0).max(0.25);
            let len = audio.len();
            if len == 0 {
                continue;
            }
            if let Ok(mut d) = tr.data.lock() {
                if d.buf.len() < MAX_LEN {
                    d.buf.resize(MAX_LEN, 0.0);
                    d.undo.resize(MAX_LEN, 0.0);
                }
                d.buf[..len].copy_from_slice(&audio);
            }
            self.s.send(Cmd::Loaded(t, len, beats));
            if tv["muted"].as_bool().unwrap_or(false) {
                self.s.send(Cmd::Mute(t));
            }
            n += 1;
        }
        self.flash(format!("loaded {n} loop{}", if n == 1 { "" } else { "s" }));
    }

    fn any(&self, f: impl Fn(u8) -> bool) -> bool {
        (0..TRACKS).any(|t| f(self.st(t)))
    }

    fn state_color(st: u8) -> Rgb565 {
        match st {
            REC | FREE_REC => kit::rgb(240, 70, 70),
            ARMED => kit::rgb(240, 150, 70),
            DUB => kit::rgb(250, 170, 60),
            PLAY => kit::rgb(110, 220, 140),
            MUTED => kit::rgb(110, 115, 130),
            _ => kit::rgb(70, 75, 90),
        }
    }
}

impl App for Looper {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn needs_background_audio(&self) -> bool {
        self.any(|s| s != EMPTY)
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        ROWS.iter().map(|r| (Looper::label(*r).to_string(), self.value(*r), false)).collect()
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(match VIEWS[self.view] {
            View::Loops => "LOOPS",
            View::Setup => "SETUP",
        })
    }
    fn toggle_grid_mode(&mut self) {
        self.view = (self.view + 1) % VIEWS.len();
    }
    /// The transport button records on the selected loop, the way a
    /// looper's footswitch does.
    fn running(&self) -> Option<bool> {
        Some(self.pending.unwrap_or_else(|| matches!(self.st(self.track), ARMED | REC | FREE_REC | DUB)))
    }
    fn toggle_running(&mut self) {
        let was = self.running().unwrap_or(false);
        // Shown at once, before the audio thread has taken the command.
        self.pending = Some(!was);
        self.rec(self.track);
    }
    fn transport_action(&self) -> Option<&'static str> {
        Some(match self.st(self.track) {
            EMPTY => "REC",
            ARMED => "CANCEL",
            REC | FREE_REC => "CLOSE",
            DUB => "PLAY",
            _ => "DUB",
        })
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let blink = self.frame / 15 % 2 == 0;
        std::array::from_fn(|p| {
            let t = p % 4;
            let st = self.st(t);
            match p / 4 {
                0 => match st {
                    REC | FREE_REC | DUB => PadColor::Red,
                    ARMED => if blink { PadColor::Red } else { PadColor::Off },
                    PLAY | MUTED => PadColor::Yellow,
                    _ => PadColor::Off,
                },
                1 => match st {
                    PLAY | DUB => PadColor::Green,
                    MUTED => PadColor::Blue,
                    _ => PadColor::Off,
                },
                2 => if self.s.tracks[t].undo.load(Ordering::Relaxed) != 0 { PadColor::Yellow } else { PadColor::Off },
                _ => if st != EMPTY { PadColor::Red } else { PadColor::Off },
            }
        })
    }

    fn tick(&mut self, input: &Input) {
        self.frame += 1;
        if self.s.cmds.lock().map(|q| q.is_empty()).unwrap_or(false) {
            self.pending = None;
        }
        if self.message.1 > 0 {
            self.message.1 -= 1;
        }
        // keep the input source publishing while we're around
        if self.frame % 15 == 0 && self.source != NO_SOURCE {
            let _ = self.bus.get(self.source);
        }
        for p in 0..16 {
            let t = p % 4;
            let down = input.grid[p] && !self.prev[p];
            if input.grid[p] {
                self.held[p] += 1;
            } else {
                self.held[p] = 0;
            }
            match p / 4 {
                0 if down => {
                    self.track = t;
                    self.rec(t);
                }
                1 if down => {
                    self.track = t;
                    self.mute(t);
                }
                2 if down => {
                    self.track = t;
                    self.undo(t);
                }
                // clear needs a short hold: it can't be undone
                3 if self.held[p] == 24 => {
                    self.track = t;
                    self.clear(t);
                    self.flash(format!("loop {} cleared", t + 1));
                }
                3 if down && self.st(t) != EMPTY => self.flash("hold to clear"),
                _ => {}
            }
        }
        self.prev = input.grid;
        match VIEWS[self.view] {
            View::Loops => {
                if input.navigation_steps != 0 {
                    self.track = (self.track as i32 + input.navigation_steps).clamp(0, TRACKS as i32 - 1) as usize;
                }
                if input.knob2 != 0 {
                    let tr = &self.s.tracks[self.track];
                    tr.level.set((tr.level.get() + input.knob2.signum() as f32 * 0.05).clamp(0.0, 1.0));
                }
                if input.knob1_press {
                    self.toggle_running();
                }
            }
            View::Setup => {
                if input.navigation_steps != 0 {
                    self.row = (self.row as i32 + input.navigation_steps).clamp(0, ROWS.len() as i32 - 1) as usize;
                }
                if input.knob2 != 0 {
                    self.edit(input.knob2.signum());
                }
                if input.knob1_press && matches!(ROWS[self.row], Row::Save | Row::Load) {
                    self.edit(1);
                }
            }
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let bg = kit::rgb(16, 18, 26);
        let panel = kit::rgb(30, 33, 46);
        let dim = kit::rgb(125, 130, 150);
        let ink = kit::rgb(235, 200, 120);
        kit::round_rect(fb, 0, 0, 640, 360, 0, bg);
        kit::text(fb, NAME, 12, 8, Size2::Medium, kit::WHITE, -1);
        let snap = self.clock.snap();
        let bpb = self.clock.bar_beats();
        let status = if snap.running {
            let b = self.clock.beat_now();
            format!("{:.1} bpm   {}.{}", snap.bpm, (b / bpb).floor() as i64 + 1, (b.rem_euclid(bpb)).floor() as i64 + 1)
        } else {
            format!("{:.1} bpm   stopped", self.clock.bpm())
        };
        kit::text(fb, &status, 628, 12, Size2::Small, dim, 1);
        // input meter
        let lvl = self.s.level.get().min(1.0);
        kit::round_rect(fb, 120, 16, 160, 8, 3, panel);
        kit::round_rect(fb, 120, 16, (160.0 * lvl) as i32 + 2, 8, 3, if lvl > 0.9 { kit::rgb(240, 70, 70) } else { kit::rgb(110, 220, 140) });

        match VIEWS[self.view] {
            View::Loops => {
                for t in 0..TRACKS {
                    let tr = &self.s.tracks[t];
                    let st = tr.st();
                    let y = 40 + t as i32 * 66;
                    let sel = t == self.track;
                    let col = Looper::state_color(st);
                    kit::round_rect(fb, 8, y, 624, 60, 8, panel);
                    if sel {
                        kit::outline(fb, 8, y, 624, 60, 8, 2, kit::blend(panel, kit::WHITE, 0.5));
                    }
                    kit::round_rect(fb, 14, y + 6, 64, 48, 6, kit::blend(panel, col, 0.35));
                    kit::text(fb, &format!("{}", t + 1), 46, y + 8, Size2::Medium, kit::WHITE, 0);
                    let name = if st == ARMED && self.frame / 15 % 2 == 1 { "" } else { STATE_NAMES[st as usize] };
                    kit::text(fb, name, 46, y + 38, Size2::Small, col, 0);
                    // the waveform
                    let (x0, w) = (86, 470);
                    let len = tr.len.load(Ordering::Relaxed);
                    let recording = matches!(st, REC | FREE_REC);
                    let span = if recording { tr.rec_len.load(Ordering::Relaxed).max((SR * 4.0) as usize) } else { len };
                    if span > 0 && st != EMPTY {
                        let shown = if recording { tr.rec_len.load(Ordering::Relaxed) } else { len };
                        for px in 0..w {
                            let a = px as usize * span / w as usize;
                            let b = ((px as usize + 1) * span / w as usize).max(a + 1);
                            if a >= shown {
                                break;
                            }
                            let mut m = 0.0f32;
                            for bin in a / BIN..=(b.min(shown) - 1) / BIN {
                                m = m.max(tr.peaks.get(bin).map_or(0.0, |p| p.get()));
                            }
                            let h = ((m.sqrt() * 24.0) as i32).clamp(1, 24);
                            let c = if st == MUTED { kit::blend(panel, col, 0.5) } else { kit::blend(panel, col, 0.85) };
                            kit::round_rect(fb, x0 + px as i32, y + 30 - h, 1, h * 2, 0, c);
                        }
                        // bar lines
                        let beats = tr.beats();
                        if beats > 0.0 && !recording {
                            let bars = (beats / bpb).round().max(1.0) as i32;
                            if bars <= 32 {
                                for k in 1..bars {
                                    let x = x0 + k * w / bars;
                                    kit::round_rect(fb, x, y + 4, 1, 52, 0, kit::blend(panel, kit::WHITE, 0.15));
                                }
                            }
                            if snap.running && matches!(st, PLAY | DUB | MUTED) {
                                let x = x0 + (tr.pos.get() * w as f32) as i32;
                                kit::round_rect(fb, x, y + 2, 2, 56, 0, kit::WHITE);
                            }
                        }
                    } else if st == EMPTY {
                        kit::text(fb, if sel { "SELECT or the top pad records" } else { "" }, x0 + w / 2, y + 22, Size2::Small, dim, 0);
                    }
                    // right: length and undo
                    let info = if len > 0 && !recording {
                        let beats = tr.beats();
                        if (beats / bpb).fract().abs() < 1e-6 {
                            format!("{} bar{}", (beats / bpb) as u32, if (beats / bpb) as u32 == 1 { "" } else { "s" })
                        } else {
                            format!("{beats:.2} beats")
                        }
                    } else if recording {
                        format!("{:.1}s", tr.rec_len.load(Ordering::Relaxed) as f32 / SR)
                    } else {
                        String::new()
                    };
                    kit::text(fb, &info, 624, y + 10, Size2::Small, kit::WHITE, 1);
                    let u = tr.undo.load(Ordering::Relaxed);
                    if u != 0 {
                        kit::text(fb, if u == 1 { "undo" } else { "redo" }, 624, y + 38, Size2::Small, ink, 1);
                    }
                }
                kit::text(fb, "pads: row 1 rec/dub  row 2 play/mute  row 3 undo  row 4 hold to clear", 320, 312, Size2::Small, dim, 0);
            }
            View::Setup => {
                for (i, r) in ROWS.iter().enumerate() {
                    let col = i / 6;
                    let y = 44 + (i % 6) as i32 * 40;
                    let x = 8 + col as i32 * 316;
                    let sel = i == self.row;
                    kit::round_rect(fb, x, y, 308, 34, 6, if sel { kit::blend(panel, kit::WHITE, 0.15) } else { panel });
                    kit::text(fb, Looper::label(*r), x + 10, y + 11, Size2::Small, if sel { kit::WHITE } else { dim }, -1);
                    let v: String = self.value(*r).chars().take(24).collect();
                    kit::text(fb, &v, x + 298, y + 11, Size2::Small, ink, 1);
                }
                kit::text(fb, &format!("Loop settings apply to loop {}.", self.track + 1), 320, 296, Size2::Small, dim, 0);
            }
        }
        if self.message.1 > 0 {
            kit::round_rect(fb, 170, 290, 300, 26, 6, kit::rgb(120, 90, 40));
            kit::text(fb, &self.message.0, 320, 297, Size2::Small, kit::WHITE, 0);
        }
        kit::footer(fb, "F2: view   SELECT/F3: rec / close / dub   up/down: loop", panel, dim);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn crate::audio::AudioProcessor>> {
        // The loop memory, made here (on the UI thread) the first time
        // the Looper is needed, never on the audio thread.
        for tr in &self.s.tracks {
            if let Ok(mut d) = tr.data.lock() {
                if d.buf.len() < MAX_LEN {
                    d.buf.resize(MAX_LEN, 0.0);
                    d.undo.resize(MAX_LEN, 0.0);
                }
            }
        }
        let e = Engine { s: Arc::clone(&self.s), clock: Arc::clone(&self.clock), rt: [Rt::default(); TRACKS], snap: self.clock.snap(), fi: 0, input: Vec::with_capacity(4096), sr: SR, peak: 0.0 };
        Some(self.sound.processor(None, Some(Box::new(e))))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<AudioBus> = ctx.get();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = (!cfg!(test)).then(|| root.join("saves/looper"));
    Box::new(Looper::new(Sound::new(NAME, &modbus, &mixer, &bus), bus, Clock::shared(), dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::pitch_hz;

    struct Rig {
        l: Looper,
        p: Box<dyn crate::audio::AudioProcessor>,
        clock: Arc<Clock>,
        input: Arc<Mutex<Vec<f32>>>,
        t: usize,
        out: Vec<f32>,
    }

    impl Rig {
        fn new() -> Rig {
            let bus = Arc::new(AudioBus::new());
            let input = bus.register("Hardware input");
            let clock = Arc::new(Clock::new());
            let mut l = Looper::new(Sound::detached(), bus, Arc::clone(&clock), None);
            let p = l.audio_processor().unwrap();
            Rig { l, p, clock, input, t: 0, out: Vec::new() }
        }
        /// Runs `blocks` of 480 frames with `f(sample index)` as input.
        fn run(&mut self, blocks: usize, f: impl Fn(usize) -> f32) {
            let mut buf = vec![0.0f32; 480 * 2];
            for _ in 0..blocks {
                *self.input.lock().unwrap() = (0..480).map(|i| f(self.t + i)).collect();
                buf.iter_mut().for_each(|x| *x = 0.0);
                self.p.process(&mut buf, 2, SR);
                self.out.extend(buf.chunks(2).map(|c| c[0]));
                self.clock.end_block(480, SR);
                self.t += 480;
            }
        }
        fn st(&self, t: usize) -> u8 {
            self.l.s.tracks[t].st()
        }
    }

    fn sine(hz: f32) -> impl Fn(usize) -> f32 {
        move |i| (i as f32 * std::f32::consts::TAU * hz / SR).sin() * 0.4
    }

    #[test]
    fn tempo_for_picks_bars_in_a_musical_range() {
        // 2 s of 4/4: one bar is 120 bpm
        let (bars, bpm) = tempo_for(2.0, 4.0);
        assert_eq!(bars, 1.0);
        assert!((bpm - 120.0).abs() < 1e-9);
        // 8 s: 4 bars at 120 rather than one bar at 30
        let (bars, bpm) = tempo_for(8.0, 4.0);
        assert_eq!(bars, 4.0);
        assert!((bpm - 120.0).abs() < 1e-9);
        // 1.2 s: one bar at 200 is too fast -- but 1 bar is the least, so
        // it stays at 1 bar (200 bpm) as the nearest it can get
        let (bars, _) = tempo_for(1.2, 4.0);
        assert_eq!(bars, 1.0);
    }

    #[test]
    fn the_first_loop_sets_the_tempo_and_starts_the_clock() {
        let mut r = Rig::new();
        r.l.rec(0);
        r.run(1, sine(330.0));
        assert_eq!(r.st(0), FREE_REC);
        // 200 blocks of 480 = 2 s -> one bar at 120 bpm (the block the
        // close is pressed in still records)
        r.run(198, sine(330.0));
        r.l.rec(0);
        r.run(1, sine(330.0));
        r.run(1, |_| 0.0);
        assert_eq!(r.st(0), PLAY);
        assert!(r.clock.snap().running, "closing the loop started the clock");
        assert!((r.clock.bpm() - 120.0).abs() < 0.1, "{}", r.clock.bpm());
        assert_eq!(r.l.s.tracks[0].beats(), 4.0);
        assert_eq!(r.l.s.tracks[0].len.load(Ordering::Relaxed), 96_000);
        // it plays back what went in, at the same pitch and level
        r.out.clear();
        r.run(100, |_| 0.0);
        let tail = &r.out[2000..];
        let hz = pitch_hz(tail, SR);
        assert!((hz - 330.0).abs() < 3.0, "{hz}");
        let pk = tail.iter().fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(pk > 0.25 && pk < 0.5, "level {pk}");
    }

    #[test]
    fn with_the_clock_running_recording_waits_for_the_bar_and_closes_on_one() {
        let mut r = Rig::new();
        r.clock.set_bpm(120.0); // a bar = 2 s = 200 blocks
        r.clock.start();
        r.run(50, |_| 0.0); // half a bar in (the clock starts after block 0)
        r.l.rec(1);
        r.run(1, |_| 0.0);
        assert_eq!(r.st(1), ARMED);
        r.run(149, |_| 0.0);
        assert_eq!(r.st(1), ARMED, "not yet");
        r.run(2, |_| 0.0);
        assert_eq!(r.st(1), REC, "started on the bar");
        // a bar and a half later: close -> rounds up to two bars
        r.run(300, sine(440.0));
        r.l.rec(1);
        r.run(1, sine(440.0));
        assert_eq!(r.st(1), REC, "still recording to the bar line");
        r.run(100, sine(440.0));
        assert_eq!(r.st(1), PLAY);
        assert_eq!(r.l.s.tracks[1].beats(), 8.0);
        assert_eq!(r.l.s.tracks[1].len.load(Ordering::Relaxed), 192_000);
    }

    #[test]
    fn a_slightly_late_press_closes_the_bar_just_passed() {
        let mut r = Rig::new();
        r.clock.set_bpm(120.0);
        r.clock.start();
        r.l.rec(0);
        r.run(2, |_| 0.0);
        assert_eq!(r.st(0), REC, "the clock was at the top: starts with it");
        r.run(203, sine(220.0)); // 4 blocks (40 ms) past the bar line
        r.l.rec(0);
        r.run(1, |_| 0.0);
        assert_eq!(r.st(0), PLAY);
        assert_eq!(r.l.s.tracks[0].beats(), 4.0, "one bar, not two");
    }

    #[test]
    fn overdub_layers_and_undo_redo_swap_it_out_and_back() {
        let mut r = Rig::new();
        r.clock.set_bpm(120.0);
        r.l.s.length.store(1, Ordering::Relaxed); // fixed 1 bar
        r.clock.start();
        r.l.rec(0);
        r.run(202, sine(300.0));
        assert_eq!(r.st(0), PLAY);
        let energy_of = |r: &mut Rig| {
            r.out.clear();
            r.run(200, |_| 0.0);
            r.out.iter().map(|x| x * x).sum::<f32>()
        };
        let one = energy_of(&mut r);
        // overdub the same tone in phase for one full pass: twice the amplitude
        r.l.rec(0);
        r.run(1, |_| 0.0);
        assert_eq!(r.st(0), DUB);
        r.run(200, sine(300.0));
        r.l.rec(0);
        r.run(1, |_| 0.0);
        assert_eq!(r.st(0), PLAY);
        assert_eq!(r.l.s.tracks[0].undo.load(Ordering::Relaxed), 1);
        let two = energy_of(&mut r);
        assert!(two > one * 2.0, "layered {two} vs {one}");
        r.l.undo(0);
        r.run(1, |_| 0.0);
        assert_eq!(r.l.s.tracks[0].undo.load(Ordering::Relaxed), 2);
        let undone = energy_of(&mut r);
        assert!((undone / one - 1.0).abs() < 0.05, "undo: {undone} vs {one}");
        r.l.undo(0);
        r.run(1, |_| 0.0);
        let redone = energy_of(&mut r);
        assert!((redone / two - 1.0).abs() < 0.05, "redo: {redone} vs {two}");
    }

    #[test]
    fn loops_stay_locked_to_the_clock_and_each_other() {
        let mut r = Rig::new();
        r.clock.set_bpm(120.0);
        r.l.s.length.store(1, Ordering::Relaxed);
        r.clock.start();
        // loop 1: a click on the downbeat of a one-bar loop
        r.l.rec(0);
        r.run(202, |i| if (i + 96_000 - 480) % 96_000 < 48 { 0.8 } else { 0.0 });
        // loop 2: two bars, silent; mute loop 1 later and unmute: it
        // must come back exactly on the bar
        r.l.s.length.store(2, Ordering::Relaxed);
        r.l.rec(1);
        r.run(500, |_| 0.0);
        r.l.mute(0);
        r.run(50, |_| 0.0);
        r.l.mute(0);
        r.out.clear();
        let start = r.t;
        r.run(800, |_| 0.0);
        let clicks: Vec<usize> = r.out.windows(2).enumerate().filter(|(_, w)| w[0].abs() < 0.2 && w[1].abs() >= 0.2).map(|(i, _)| start + i + 1).collect();
        assert!(clicks.len() >= 3, "{clicks:?}");
        for c in &clicks {
            // on bar lines: 96 000 samples apart, from the clock's start
            // (one block in -- the clock started at the end of block 0)
            let off = (*c as i64 - 480).rem_euclid(96_000);
            assert!(off < 4 || off > 96_000 - 4, "click at {c} is {off} off the bar");
        }
    }

    #[test]
    fn mute_and_clear_fade_and_clear_empties() {
        let mut r = Rig::new();
        r.clock.start();
        r.l.s.length.store(1, Ordering::Relaxed);
        r.l.rec(2);
        r.run(202, sine(500.0));
        assert_eq!(r.st(2), PLAY);
        r.l.mute(2);
        r.out.clear();
        r.run(20, |_| 0.0);
        let after = &r.out[2400..];
        assert!(after.iter().all(|x| x.abs() < 1e-3), "muted");
        // no click on the way down: the biggest sample-to-sample jump is small
        let jump = r.out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(jump < 0.1, "{jump}");
        r.l.mute(2);
        r.l.clear(2);
        r.run(20, |_| 0.0);
        assert_eq!(r.st(2), EMPTY);
        assert_eq!(r.l.s.tracks[2].len.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn stopping_the_clock_abandons_a_clocked_recording_and_silences_loops() {
        let mut r = Rig::new();
        r.clock.start();
        r.l.s.length.store(1, Ordering::Relaxed);
        r.l.rec(0);
        r.run(202, sine(400.0));
        r.l.s.quantize.store(2, Ordering::Relaxed); // start at once
        r.l.rec(1);
        r.run(20, sine(400.0));
        assert_eq!(r.st(1), REC);
        r.clock.stop();
        r.run(2, |_| 0.0);
        assert_eq!(r.st(1), EMPTY);
        assert_eq!(r.st(0), PLAY, "the loop is kept, just not playing");
        r.out.clear();
        r.run(10, |_| 0.0);
        assert!(r.out.iter().all(|x| x.abs() < 1e-3));
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("pmx_looper_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut r = Rig::new();
        r.l.dir = Some(dir.clone());
        r.clock.set_bpm(100.0);
        r.clock.start();
        r.l.s.length.store(1, Ordering::Relaxed);
        r.l.rec(3);
        r.run(250, sine(262.0));
        assert_eq!(r.st(3), PLAY);
        r.l.s.tracks[3].pan.set(-0.5);
        r.l.save();
        let mut r2 = Rig::new();
        r2.l.dir = Some(dir.clone());
        r2.l.load();
        r2.run(1, |_| 0.0);
        assert_eq!(r2.st(3), PLAY);
        assert_eq!(r2.l.s.tracks[3].beats(), 4.0);
        assert_eq!(r2.l.s.tracks[3].len.load(Ordering::Relaxed), r.l.s.tracks[3].len.load(Ordering::Relaxed));
        assert_eq!(r2.l.s.tracks[3].pan.get(), -0.5);
        assert_eq!(r2.clock.bpm(), 100.0);
        assert_eq!(r2.st(0), EMPTY);
        r2.clock.start();
        r2.out.clear();
        r2.run(60, |_| 0.0);
        let hz = pitch_hz(&r2.out[1000..], SR);
        assert!((hz - 262.0).abs() < 3.0, "{hz}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn draws_both_views_in_every_state() {
        let mut r = Rig::new();
        r.l.rec(0);
        r.run(10, sine(200.0));
        r.clock.start();
        let mut fb = FrameBuffer::new();
        for v in 0..2 {
            r.l.view = v;
            r.l.draw(&mut fb);
        }
        let _ = r.l.grid_led_overlay();
    }
}
