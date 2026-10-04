//! The norns cartridge's sound: a PolyPerc engine and a softcut-style
//! buffer engine, both written here from the documented norns APIs (no
//! norns or SuperCollider code is used).
//!
//! PolyPerc, as norns documents it: a polyphonic pulse wave through a
//! Moog-style low-pass with a percussive envelope. Each `hz` starts a new
//! note with the engine's current pw / cutoff / gain / release / amp / pan.
//! The filter here is a 4-pole transistor-ladder model; `gain` is its
//! resonance feedback (0..4, self-oscillating near 4), as MoogFF's is.
//! Unverified: the exact SuperCollider defaults; the ones below are the
//! values PolyPerc scripts commonly assume.
//!
//! Softcut: up to 6 voices reading and writing two mono buffers, each
//! with rate, loop points, crossfaded loop jumps, record level and
//! pre-level (overdub feedback), a state-variable filter with dry/lp/hp/
//! bp/br mix, level and pan. Buffers here hold `BUFFER_SECONDS` (norns
//! has ~350 s; Portamax's RAM budget is smaller).

use std::f32::consts::{PI, TAU};
use std::sync::{Arc, Mutex};

pub const POLY_VOICES: usize = 24;
pub const CUT_VOICES: usize = 6;
pub const BUFFER_SECONDS: f32 = 60.0;

/// A command from the Lua thread to the audio thread.
#[derive(Clone, Debug)]
pub enum Cmd {
    Engine(String, Vec<f32>),
    Softcut(String, Vec<f32>),
    Audio(String, Vec<f32>),
    /// A MIDI message from the script (`midi.connect():note_on(...)`):
    /// notes go out on the note bus to whatever Norns is routed to.
    Midi(Vec<u8>),
    /// Script stopped: silence everything and reset to power-on state.
    Reset,
}

/// Commands queued by the Lua thread, drained by the audio thread each
/// block (try_lock: if the UI side is mid-push, they wait one block).
#[derive(Default)]
pub struct Queue {
    pub cmds: Mutex<Vec<Cmd>>,
}

impl Queue {
    pub fn push(&self, c: Cmd) {
        if let Ok(mut q) = self.cmds.lock() {
            if q.len() < 4096 {
                q.push(c);
            }
        }
    }
}

// --------------------------------------------------------------- PolyPerc

#[derive(Clone, Copy)]
struct PercParams {
    pw: f32,
    cutoff: f32,
    gain: f32,
    release: f32,
    amp: f32,
    pan: f32,
}

impl Default for PercParams {
    fn default() -> Self {
        Self { pw: 0.5, cutoff: 1000.0, gain: 2.0, release: 0.5, amp: 0.3, pan: 0.0 }
    }
}

#[derive(Clone, Copy, Default)]
struct PercVoice {
    on: bool,
    phase: f32,
    inc: f32,
    p: PercParamsCopy,
    t: f32,
    /// Ladder filter stages.
    s: [f32; 4],
}

#[derive(Clone, Copy, Default)]
struct PercParamsCopy {
    pw: f32,
    g: f32,
    k: f32,
    release: f32,
    amp: f32,
    pan: f32,
}

/// PolyBLEP residual for a band-limited step at phase `t` (0..1).
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

const PERC_ATTACK: f32 = 0.01;

impl PercVoice {
    fn tick(&mut self) -> f32 {
        let dt = self.inc;
        let pw = self.p.pw.clamp(0.01, 0.99);
        let mut x = if self.phase < pw { 1.0 } else { -1.0 };
        x += blep(self.phase, dt);
        x -= blep((self.phase - pw).rem_euclid(1.0), dt);
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        // 4-pole ladder with tanh-saturated input and resonance feedback
        let g = self.p.g;
        let u = (x - self.p.k * self.s[3]).tanh();
        self.s[0] += g * (u - self.s[0]);
        self.s[1] += g * (self.s[0] - self.s[1]);
        self.s[2] += g * (self.s[1] - self.s[2]);
        self.s[3] += g * (self.s[2] - self.s[3]);
        // the ladder's passband drops as resonance rises; MoogFF makes
        // up some of it
        let y = self.s[3] * (1.0 + 0.5 * self.p.k);
        y
    }
}

pub struct PolyPerc {
    p: PercParams,
    voices: Vec<PercVoice>,
    next: usize,
    sr: f32,
}

impl PolyPerc {
    pub fn new() -> Self {
        Self { p: PercParams::default(), voices: vec![PercVoice::default(); POLY_VOICES], next: 0, sr: 48_000.0 }
    }

    pub fn command(&mut self, name: &str, args: &[f32]) {
        let a = args.first().copied().unwrap_or(0.0);
        if !a.is_finite() {
            return;
        }
        match name {
            "hz" => self.note(a),
            "amp" => self.p.amp = a.clamp(0.0, 4.0),
            "pw" => self.p.pw = a.clamp(0.0, 1.0),
            "release" => self.p.release = a.clamp(0.001, 30.0),
            "cutoff" => self.p.cutoff = a.clamp(10.0, 20_000.0),
            "gain" => self.p.gain = a.clamp(0.0, 4.0),
            "pan" => self.p.pan = a.clamp(-1.0, 1.0),
            _ => {}
        }
    }

    fn note(&mut self, hz: f32) {
        if hz <= 0.0 {
            return;
        }
        // a free voice, else the oldest (round robin)
        let i = self.voices.iter().position(|v| !v.on).unwrap_or(self.next);
        self.next = (i + 1) % POLY_VOICES;
        let fc = self.p.cutoff.min(self.sr * 0.45);
        self.voices[i] = PercVoice {
            on: true,
            phase: 0.0,
            inc: (hz / self.sr).min(0.45),
            p: PercParamsCopy {
                pw: self.p.pw,
                g: 1.0 - (-TAU * fc / self.sr).exp(),
                k: self.p.gain,
                release: self.p.release,
                amp: self.p.amp,
                pan: self.p.pan,
            },
            t: 0.0,
            s: [0.0; 4],
        };
    }

    pub fn set_rate(&mut self, sr: f32) {
        self.sr = sr;
    }

    /// One stereo frame.
    pub fn tick(&mut self) -> (f32, f32) {
        let dt = 1.0 / self.sr;
        let (mut l, mut r) = (0.0, 0.0);
        for v in self.voices.iter_mut().filter(|v| v.on) {
            // Env.perc: linear-ish attack, exponential-curve (-4) release
            let env = if v.t < PERC_ATTACK {
                v.t / PERC_ATTACK
            } else {
                let x = ((v.t - PERC_ATTACK) / v.p.release).min(1.0);
                let curve = -4.0f32;
                1.0 - (1.0 - (curve * x).exp()) / (1.0 - curve.exp())
            };
            v.t += dt;
            if v.t > PERC_ATTACK + v.p.release {
                v.on = false;
            }
            let s = v.tick() * env * v.p.amp;
            // equal-power pan
            let a = (v.p.pan + 1.0) * PI / 4.0;
            l += s * a.cos();
            r += s * a.sin();
        }
        (l, r)
    }

    #[allow(dead_code)] // tests
    pub fn active(&self) -> usize {
        self.voices.iter().filter(|v| v.on).count()
    }
}

// ---------------------------------------------------------------- softcut

#[derive(Clone, Copy)]
struct CutVoice {
    enabled: bool,
    buffer: usize,
    play: bool,
    rec: bool,
    looping: bool,
    rate: f32,
    rate_target: f32,
    rate_slew: f32,
    level: f32,
    level_target: f32,
    level_slew: f32,
    pan: f32,
    loop_start: f32,
    loop_end: f32,
    fade: f32,
    rec_level: f32,
    pre_level: f32,
    /// Input mix from the two input channels (engine/ADC sum).
    input: [f32; 2],
    /// Read/write head position, seconds.
    pos: f64,
    /// A head fading out after a jump: position and remaining fade.
    old_pos: f64,
    old_fade: f32,
    filt: [f32; 2],
    f_dry: f32,
    f_lp: f32,
    f_hp: f32,
    f_bp: f32,
    f_br: f32,
    f_fc: f32,
    f_rq: f32,
}

impl Default for CutVoice {
    fn default() -> Self {
        Self {
            enabled: false,
            buffer: 0,
            play: false,
            rec: false,
            looping: true,
            rate: 1.0,
            rate_target: 1.0,
            rate_slew: 0.0,
            level: 1.0,
            level_target: 1.0,
            level_slew: 0.0,
            pan: 0.0,
            loop_start: 0.0,
            loop_end: 1.0,
            fade: 0.0,
            rec_level: 0.0,
            pre_level: 0.0,
            input: [0.0; 2],
            pos: 0.0,
            old_pos: 0.0,
            old_fade: 0.0,
            filt: [0.0; 2],
            f_dry: 1.0,
            f_lp: 0.0,
            f_hp: 0.0,
            f_bp: 0.0,
            f_br: 0.0,
            f_fc: 12_000.0,
            f_rq: 2.0,
        }
    }
}

pub struct Softcut {
    voices: [CutVoice; CUT_VOICES],
    pub buffers: [Vec<f32>; 2],
    sr: f32,
}

fn read(buf: &[f32], pos: f64, sr: f32) -> f32 {
    let x = pos * sr as f64;
    let i = x.floor();
    let f = (x - i) as f32;
    let n = buf.len() as i64;
    let i = (i as i64).rem_euclid(n.max(1)) as usize;
    let j = (i + 1) % buf.len().max(1);
    buf.get(i).copied().unwrap_or(0.0) * (1.0 - f) + buf.get(j).copied().unwrap_or(0.0) * f
}

impl Softcut {
    pub fn new() -> Self {
        Self { voices: [CutVoice::default(); CUT_VOICES], buffers: [Vec::new(), Vec::new()], sr: 48_000.0 }
    }

    /// Allocates the buffers (off the audio thread: call before handing
    /// the processor over).
    pub fn allocate(&mut self, sr: f32) {
        self.sr = sr;
        let n = (BUFFER_SECONDS * sr) as usize;
        for b in self.buffers.iter_mut() {
            b.clear();
            b.resize(n, 0.0);
        }
    }

    fn len_s(&self) -> f32 {
        self.buffers[0].len() as f32 / self.sr
    }

    pub fn command(&mut self, name: &str, a: &[f32]) {
        let v = a.first().map_or(0, |v| (*v as i32 - 1).clamp(0, CUT_VOICES as i32 - 1) as usize);
        let x = a.get(1).copied().unwrap_or(0.0);
        let x = if x.is_finite() { x } else { 0.0 };
        let len = self.len_s();
        match name {
            "buffer_clear" => self.buffers.iter_mut().for_each(|b| b.fill(0.0)),
            "buffer_clear_channel" => {
                if let Some(b) = self.buffers.get_mut(v) {
                    b.fill(0.0)
                }
            }
            "buffer_clear_region" | "buffer_clear_region_channel" => {
                let (ch, s, d) = if name == "buffer_clear_region" { (None, a.first().copied().unwrap_or(0.0), x) } else { (Some(v), x, a.get(2).copied().unwrap_or(0.0)) };
                let (i0, i1) = ((s.max(0.0) * self.sr) as usize, ((s + d).max(0.0) * self.sr) as usize);
                for (bi, b) in self.buffers.iter_mut().enumerate() {
                    if ch.is_none_or(|c| c == bi) {
                        let n = b.len();
                        b[i0.min(n)..i1.min(n)].fill(0.0);
                    }
                }
            }
            "reset" => *self = Self { voices: [CutVoice::default(); CUT_VOICES], buffers: std::mem::take(&mut self.buffers), sr: self.sr },
            _ => {
                let c = &mut self.voices[v];
                match name {
                    "enable" => c.enabled = x > 0.0,
                    "buffer" => c.buffer = (x as i32 - 1).clamp(0, 1) as usize,
                    "play" => c.play = x > 0.0,
                    "rec" => c.rec = x > 0.0,
                    "loop" => c.looping = x > 0.0,
                    "rate" => c.rate_target = x.clamp(-64.0, 64.0),
                    "rate_slew_time" => c.rate_slew = x.max(0.0),
                    "level" => c.level_target = x.max(0.0),
                    "level_slew_time" => c.level_slew = x.max(0.0),
                    "pan" => c.pan = x.clamp(-1.0, 1.0),
                    "loop_start" => c.loop_start = x.clamp(0.0, len),
                    "loop_end" => c.loop_end = x.clamp(0.0, len),
                    "fade_time" => c.fade = x.max(0.0),
                    "rec_level" => c.rec_level = x,
                    "pre_level" => c.pre_level = x,
                    "position" => {
                        c.old_pos = c.pos;
                        c.old_fade = if c.fade > 0.0 { 1.0 } else { 0.0 };
                        c.pos = x.clamp(0.0, len) as f64;
                    }
                    "level_input_cut" => {
                        // (ch, voice, amp): note the argument order
                        let ch = (a.first().copied().unwrap_or(1.0) as i32 - 1).clamp(0, 1) as usize;
                        let vi = (x as i32 - 1).clamp(0, CUT_VOICES as i32 - 1) as usize;
                        self.voices[vi].input[ch] = a.get(2).copied().unwrap_or(0.0);
                    }
                    "filter_dry" | "post_filter_dry" => c.f_dry = x,
                    "filter_lp" | "post_filter_lp" => c.f_lp = x,
                    "filter_hp" | "post_filter_hp" => c.f_hp = x,
                    "filter_bp" | "post_filter_bp" => c.f_bp = x,
                    "filter_br" | "post_filter_br" => c.f_br = x,
                    "filter_fc" | "post_filter_fc" => c.f_fc = x.clamp(10.0, 20_000.0),
                    "filter_rq" | "post_filter_rq" => c.f_rq = x.clamp(0.05, 8.0),
                    _ => {} // pre-filters, phase polls, slews we don't model
                }
            }
        }
    }

    /// One stereo frame; `input` is the (L, R) signal routed to softcut.
    pub fn tick(&mut self, input: (f32, f32)) -> (f32, f32) {
        if self.buffers[0].is_empty() {
            return (0.0, 0.0);
        }
        let sr = self.sr;
        let dt = 1.0 / sr;
        let (mut l, mut r) = (0.0, 0.0);
        for vi in 0..CUT_VOICES {
            let c = &mut self.voices[vi];
            if !c.enabled {
                continue;
            }
            // slews
            let slew = |cur: &mut f32, target: f32, time: f32| {
                if time <= 0.0 {
                    *cur = target;
                } else {
                    *cur += (target - *cur) * (dt / time * 4.0).min(1.0);
                }
            };
            slew(&mut c.rate, c.rate_target, c.rate_slew);
            slew(&mut c.level, c.level_target, c.level_slew);
            let buf = &mut self.buffers[c.buffer];
            let n = buf.len();
            let (ls, le) = (c.loop_start.min(c.loop_end) as f64, c.loop_start.max(c.loop_end) as f64);
            let fade_step = if c.fade > 0.0 { dt / c.fade } else { 1.0 };
            // read
            let mut y = 0.0;
            if c.play {
                let w_new = 1.0 - c.old_fade;
                y = read(buf, c.pos, sr) * w_new.sqrt();
                if c.old_fade > 0.0 {
                    y += read(buf, c.old_pos, sr) * c.old_fade.sqrt();
                }
            }
            // write (overdub: keep pre_level of what was there)
            if c.rec {
                let inp = input.0 * c.input[0] + input.1 * c.input[1];
                let i = ((c.pos * sr as f64) as i64).rem_euclid(n as i64) as usize;
                let w = if c.old_fade > 0.0 { 1.0 - c.old_fade } else { 1.0 };
                buf[i] = buf[i] * (c.pre_level * w + (1.0 - w)) + inp * c.rec_level * w;
            }
            // advance, jumping at the loop ends with a crossfade
            if c.play || c.rec {
                c.pos += (c.rate * dt) as f64;
                if c.old_fade > 0.0 {
                    c.old_pos += (c.rate * dt) as f64;
                    c.old_fade = (c.old_fade - fade_step).max(0.0);
                }
                let over = if c.rate >= 0.0 { c.pos >= le } else { c.pos < ls };
                if over && le > ls {
                    if c.looping {
                        c.old_pos = c.pos;
                        c.old_fade = if c.fade > 0.0 { 1.0 } else { 0.0 };
                        c.pos = if c.rate >= 0.0 { ls + (c.pos - le) } else { le - (ls - c.pos) };
                    } else {
                        c.play = false;
                        c.rec = false;
                    }
                }
                let lim = n as f64 / sr as f64;
                c.pos = c.pos.rem_euclid(lim);
            }
            // filter (SVF), mixed
            let fc = c.f_fc.min(sr * 0.45);
            let g = (PI * fc / sr).tan();
            let k = c.f_rq;
            let a1 = 1.0 / (1.0 + g * (g + k));
            let v3 = y - c.filt[1];
            let v1 = a1 * c.filt[0] + g * a1 * v3;
            let v2 = c.filt[1] + g * v1;
            c.filt[0] = 2.0 * v1 - c.filt[0];
            c.filt[1] = 2.0 * v2 - c.filt[1];
            let lp = v2;
            let bp = v1;
            let hp = y - k * v1 - v2;
            let br = lp + hp;
            let out = (y * c.f_dry + lp * c.f_lp + hp * c.f_hp + bp * c.f_bp + br * c.f_br) * c.level;
            let a = (c.pan + 1.0) * PI / 4.0;
            l += out * a.cos();
            r += out * a.sin();
        }
        (l, r)
    }
}

// ------------------------------------------------------------- processor

/// norns' mixer levels that matter here.
#[derive(Clone, Copy)]
struct Levels {
    /// softcut -> output
    cut: f32,
    /// engine -> softcut input
    eng_cut: f32,
    /// engine -> output
    eng: f32,
}

impl Default for Levels {
    fn default() -> Self {
        Self { cut: 1.0, eng_cut: 0.0, eng: 1.0 }
    }
}

pub struct Processor {
    pub queue: Arc<Queue>,
    pub poly: PolyPerc,
    pub cut: Softcut,
    levels: Levels,
    pending: Vec<Cmd>,
    pub mix_level: Arc<crate::util::AtomicF32>,
    pub ext_mix_level: Arc<crate::util::AtomicF32>,
    pub peak: Arc<crate::util::AtomicF32>,
    /// The script's MIDI notes, sent to another app (see note_bus.rs).
    pub notes: crate::note_bus::NoteOut,
}

impl Processor {
    pub fn new(queue: Arc<Queue>, mix_level: Arc<crate::util::AtomicF32>, ext_mix_level: Arc<crate::util::AtomicF32>, peak: Arc<crate::util::AtomicF32>) -> Self {
        let mut cut = Softcut::new();
        cut.allocate(48_000.0);
        Self { queue, poly: PolyPerc::new(), cut, levels: Levels::default(), pending: Vec::with_capacity(4096), mix_level, ext_mix_level, peak, notes: crate::note_bus::NoteOut::detached() }
    }

    fn apply(&mut self, c: Cmd) {
        match c {
            Cmd::Engine(n, a) => self.poly.command(&n, &a),
            Cmd::Softcut(n, a) => self.cut.command(&n, &a),
            Cmd::Audio(n, a) => {
                let x = a.first().copied().unwrap_or(0.0).max(0.0);
                match n.as_str() {
                    "level_cut" => self.levels.cut = x,
                    "level_eng_cut" => self.levels.eng_cut = x,
                    "level_eng" => self.levels.eng = x,
                    _ => {}
                }
            }
            Cmd::Midi(m) => match (m.first().map(|s| s & 0xF0), m.get(1), m.get(2)) {
                (Some(0x90), Some(&n), Some(&v)) if v > 0 => self.notes.note_on(n.min(127), v.min(127)),
                (Some(0x90 | 0x80), Some(&n), _) => self.notes.note_off(n.min(127)),
                _ => {}
            },
            Cmd::Reset => {
                self.notes.all_off();
                self.poly = PolyPerc::new();
                self.poly.set_rate(self.cut.sr);
                self.cut.command("reset", &[]);
                self.cut.command("buffer_clear", &[]);
                self.levels = Levels::default();
            }
        }
    }
}

impl crate::audio::AudioProcessor for Processor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sr: f32) {
        if channels == 0 || sr <= 0.0 {
            return;
        }
        if (self.poly.sr - sr).abs() > 0.5 {
            // Buffers are sized at construction for 48 kHz; at another
            // rate they just hold proportionally more or less time.
            self.poly.set_rate(sr);
            self.cut.sr = sr;
        }
        if let Ok(mut q) = self.queue.cmds.try_lock() {
            std::mem::swap(&mut *q, &mut self.pending);
        }
        for c in std::mem::take(&mut self.pending) {
            self.apply(c);
        }
        let mix = (self.mix_level.get() + self.ext_mix_level.get()).clamp(0.0, 2.0);
        let mut peak = 0.0f32;
        for frame in buffer.chunks_mut(channels) {
            let (el, er) = self.poly.tick();
            let send = (el * self.levels.eng_cut, er * self.levels.eng_cut);
            let (cl, cr) = self.cut.tick(send);
            let l = ((el * self.levels.eng + cl * self.levels.cut) * mix).tanh();
            let r = ((er * self.levels.eng + cr * self.levels.cut) * mix).tanh();
            peak = peak.max(l.abs()).max(r.abs());
            if channels == 1 {
                frame[0] += (l + r) * 0.5;
            } else {
                frame[0] += l;
                frame[1] += r;
                for c in frame.iter_mut().skip(2) {
                    *c += (l + r) * 0.5;
                }
            }
        }
        self.peak.set(self.peak.get() * 0.8 + peak * 0.2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_polyperc_note_rings_then_dies_at_its_release() {
        let mut p = PolyPerc::new();
        p.command("release", &[0.2]);
        p.command("hz", &[220.0]);
        let first: Vec<f32> = (0..4800).map(|_| p.tick().0).collect();
        assert!(first.iter().map(|x| x * x).sum::<f32>() > 1.0, "audible");
        for _ in 0..9600 {
            p.tick();
        }
        assert_eq!(p.active(), 0, "voice freed after attack + release");
    }

    #[test]
    fn cutoff_darkens_the_tone() {
        let bright = |fc: f32| {
            let mut p = PolyPerc::new();
            p.command("cutoff", &[fc]);
            p.command("gain", &[0.0]);
            p.command("release", &[2.0]);
            p.command("hz", &[110.0]);
            let x: Vec<f32> = (0..4800).map(|_| p.tick().0).collect();
            // energy of the first difference ~ high-frequency content
            x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>() / x.iter().map(|v| v * v).sum::<f32>().max(1e-9)
        };
        assert!(bright(5000.0) > bright(300.0) * 4.0);
    }

    #[test]
    fn softcut_records_then_plays_back_a_delay_with_feedback() {
        let mut s = Softcut::new();
        s.allocate(48_000.0);
        for (n, a) in [
            ("enable", vec![1.0, 1.0]),
            ("play", vec![1.0, 1.0]),
            ("rec", vec![1.0, 1.0]),
            ("rec_level", vec![1.0, 1.0]),
            ("pre_level", vec![1.0, 0.5]),
            ("loop_start", vec![1.0, 1.0]),
            ("loop_end", vec![1.0, 1.1]),
            ("position", vec![1.0, 1.0]),
            ("level", vec![1.0, 1.0]),
        ] {
            s.command(n, &a);
        }
        s.command("level_input_cut", &[1.0, 1.0, 1.0]);
        // an impulse in, then silence: it should come back every 0.1 s, halving
        let mut out = Vec::new();
        for i in 0..48_000 / 2 {
            let x = if i == 10 { 1.0 } else { 0.0 };
            out.push(s.tick((x, 0.0)).0);
        }
        let echo = |t: f32| {
            let c = (t * 48_000.0) as usize + 10;
            out[c - 3..c + 3].iter().fold(0.0f32, |m, v| m.max(v.abs()))
        };
        assert!(echo(0.1) > 0.3, "first repeat {}", echo(0.1));
        assert!(echo(0.2) < echo(0.1) * 0.8 && echo(0.2) > 0.1, "decays by pre_level: {} {}", echo(0.1), echo(0.2));
    }
}
