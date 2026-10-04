//! What the Kids apps share: a small sound engine (tuned percussion,
//! plucked strings, organ, chiptune and a synthesized drum kit), a
//! sample-accurate step clock, and drawing helpers for big, bright,
//! full-screen pictures.
//!
//! The apps that use it are in the launcher's KIDS section, as two
//! groups: five for ages 6-9 (Critter Choir, Rainbow Bells, Copy Cat,
//! Bug Beats, Monster Mic) and five for ages 10-12 (Beat Lab, Chord
//! Garden, Sound Detective, Ear Quest, Music Code).
//!
//! Every app draws its whole screen itself (`draw`) and hands the same
//! picture to the Slint GUI through `screen_extra`, so the device and the
//! desktop GUI show one design.
//!
//! The sounds are modelled on the real instruments:
//! - glockenspiel and marimba are sums of their bars' decaying modes. A
//!   free-free bar's overtones sit at 2.756, 5.404 and 8.933 times the
//!   fundamental. A marimba bar is undercut so its first overtones land
//!   near 3.9 and 9.2.
//! - the plucked string is Karplus-Strong.
//! - the hi-hats and cowbell use the TR-808's circuits: six detuned
//!   square waves through band-pass filters for the hats, two (540 Hz and
//!   800 Hz) for the cowbell.
//! - the kick is a sine whose pitch falls quickly. The clap is
//!   band-passed noise struck three times, then a tail.

use crate::app::SlintExtra;
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16};
use crate::util::AtomicF32;
use crate::apps::voltage_fx::{FxSettings, StereoFx};
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle, Rectangle, RoundedRectangle, Triangle};
use embedded_graphics::text::{Alignment, Baseline, Text, TextStyleBuilder};
use embedded_graphics::Pixel;
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------
// Sound
// ---------------------------------------------------------------------

/// The pitched instruments.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Glock,
    Marimba,
    Pluck,
    Soft,
    Organ,
    Chip,
    Bass,
    Flute,
}

impl Tone {
    #[cfg(test)]
    pub const ALL: [Tone; 8] = [Tone::Glock, Tone::Marimba, Tone::Pluck, Tone::Soft, Tone::Organ, Tone::Chip, Tone::Bass, Tone::Flute];
    pub fn name(self) -> &'static str {
        match self {
            Tone::Glock => "Bells",
            Tone::Marimba => "Marimba",
            Tone::Pluck => "Harp",
            Tone::Soft => "Soft synth",
            Tone::Organ => "Organ",
            Tone::Chip => "Game",
            Tone::Bass => "Bass",
            Tone::Flute => "Flute",
        }
    }
}

/// The drum kit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Drum {
    Kick,
    Snare,
    Hat,
    OpenHat,
    Clap,
    Shaker,
    Cowbell,
    HiTom,
    LowTom,
    Rim,
    Woodblock,
}

impl Drum {
    pub fn name(self) -> &'static str {
        match self {
            Drum::Kick => "Kick",
            Drum::Snare => "Snare",
            Drum::Hat => "Hi-hat",
            Drum::OpenHat => "Open hat",
            Drum::Clap => "Clap",
            Drum::Shaker => "Shaker",
            Drum::Cowbell => "Cowbell",
            Drum::HiTom => "High tom",
            Drum::LowTom => "Low tom",
            Drum::Rim => "Rim",
            Drum::Woodblock => "Woodblock",
        }
    }
}

/// One pitched note.
#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub tone: Tone,
    /// MIDI note number; fractions are fine.
    pub pitch: f32,
    /// 0..1.
    pub vel: f32,
    /// -1 (left) .. 1 (right).
    pub pan: f32,
    /// Seconds before the note is released. 0 = held until `Ev::Off(id)`.
    pub len: f32,
    pub id: u32,
}

impl Note {
    pub fn new(tone: Tone, pitch: f32) -> Note {
        Note { tone, pitch, vel: 0.8, pan: 0.0, len: 0.35, id: 0 }
    }
    pub fn vel(mut self, v: f32) -> Note {
        self.vel = v;
        self
    }
    pub fn pan(mut self, p: f32) -> Note {
        self.pan = p;
        self
    }
    pub fn len(mut self, s: f32) -> Note {
        self.len = s;
        self
    }
    /// Held until `Ev::Off(id)` (or `Sound::off(id)`).
    pub fn held(mut self, id: u32) -> Note {
        self.len = 0.0;
        self.id = id;
        self
    }
}

/// Everything the UI (or a `Song`) can ask the engine to do.
#[derive(Clone, Copy, Debug)]
pub enum Ev {
    Note(Note),
    /// Release every held note with this id.
    Off(u32),
    /// A drum hit at a velocity, 0..1.
    Drum(Drum, f32),
    /// Release every note.
    AllOff,
    /// For an app's own `Extra` voice: three numbers it defines.
    Custom(u32, f32, f32),
}

/// A pattern the clock plays: `step` is called on each step (a 16th
/// note by default), on the audio thread, at the exact sample the step
/// falls on. Read shared state with `try_lock` -- never block here.
pub trait Song: Send {
    fn step(&mut self, step: u64, out: &mut Vec<Ev>);
    /// Steps per quarter-note beat.
    fn steps_per_beat(&self) -> u32 {
        4
    }
}

/// An app's own sound, mixed in with the kit's voices (Critter Choir's
/// animals, Monster Mic's voice changer, Sound Detective's synth).
pub trait Extra: Send {
    /// Called at the start of each block, before any frame.
    fn block(&mut self, _frames: usize, _sample_rate: f32) {}
    fn event(&mut self, _a: u32, _b: f32, _c: f32) {}
    /// Called before every frame: events the extra wants the kit's own
    /// voices to play at exactly this sample (a strum, an arpeggio).
    fn poll(&mut self, _out: &mut Vec<Ev>) {}
    /// One stereo frame.
    fn frame(&mut self, sample_rate: f32) -> (f32, f32);
}

struct Shared {
    queue: Mutex<Vec<Ev>>,
    volume: AtomicF32,
    reverb: AtomicF32,
    tempo: AtomicF32,
    swing: AtomicF32,
    playing: AtomicBool,
    /// The number of steps fired since the last start (0 = none yet).
    steps: AtomicU64,
    /// The device transport, and whether this app's steps follow it
    /// (Session, the Looper and Skins all lock together) or run on the
    /// app's own tempo (the kids' apps, which stand alone).
    clock: Arc<crate::clock::Clock>,
    follow: AtomicBool,
    peak: AtomicF32,
    mix: Arc<AtomicF32>,
    ext_mix: Arc<AtomicF32>,
    bus: Option<Arc<Mutex<Vec<f32>>>>,
}

/// The UI side's handle on an app's sound. Cheap to clone.
#[derive(Clone)]
pub struct Sound {
    s: Arc<Shared>,
}

impl Sound {
    /// Registers the app's mixer channel and its output on the audio bus
    /// (so Studio or an effect can take it), both under `name`.
    pub fn new(name: &str, modbus: &ModBus, mixer: &MixerBus, audio_bus: &AudioBus) -> Sound {
        let (mix, ext_mix) = mixer.register(name, modbus);
        Sound::build(mix, ext_mix, Some(audio_bus.register(name)), crate::clock::Clock::shared())
    }

    /// Not connected to anything: for tests.
    #[cfg(test)]
    pub fn detached() -> Sound {
        Sound::detached_with_clock(Arc::new(crate::clock::Clock::new()))
    }

    /// Not connected, but on a transport the test drives itself.
    #[cfg(test)]
    pub fn detached_with_clock(clock: Arc<crate::clock::Clock>) -> Sound {
        Sound::build(Arc::new(AtomicF32::new(1.0)), Arc::new(AtomicF32::new(0.0)), None, clock)
    }

    fn build(mix: Arc<AtomicF32>, ext_mix: Arc<AtomicF32>, bus: Option<Arc<Mutex<Vec<f32>>>>, clock: Arc<crate::clock::Clock>) -> Sound {
        Sound {
            s: Arc::new(Shared {
                queue: Mutex::new(Vec::with_capacity(64)),
                volume: AtomicF32::new(0.8),
                reverb: AtomicF32::new(0.25),
                tempo: AtomicF32::new(100.0),
                swing: AtomicF32::new(0.0),
                playing: AtomicBool::new(false),
                steps: AtomicU64::new(0),
                clock,
                follow: AtomicBool::new(false),
                peak: AtomicF32::new(0.0),
                mix,
                ext_mix,
                bus,
            }),
        }
    }

    pub fn send(&self, ev: Ev) {
        if let Ok(mut q) = self.s.queue.lock() {
            // A stalled audio thread mustn't grow this forever.
            if q.len() < 256 {
                q.push(ev);
            }
        }
    }
    pub fn play(&self, note: Note) {
        self.send(Ev::Note(note));
    }
    pub fn drum(&self, d: Drum, vel: f32) {
        self.send(Ev::Drum(d, vel));
    }
    pub fn off(&self, id: u32) {
        self.send(Ev::Off(id));
    }
    pub fn all_off(&self) {
        self.send(Ev::AllOff);
    }
    pub fn custom(&self, a: u32, b: f32, c: f32) {
        self.send(Ev::Custom(a, b, c));
    }

    pub fn set_tempo(&self, bpm: f32) {
        if self.following() {
            self.s.clock.set_bpm(bpm.clamp(40.0, 240.0));
        } else {
            self.s.tempo.set(bpm.clamp(40.0, 240.0));
        }
    }
    pub fn tempo(&self) -> f32 {
        if self.following() {
            self.s.clock.bpm()
        } else {
            self.s.tempo.get()
        }
    }
    /// Follow the device transport (true) or run on this app's own tempo.
    /// The device has one tempo, so following doesn't impose this app's
    /// on it -- an app opening mustn't change what's already playing.
    pub fn set_follow(&self, follow: bool) {
        if follow == self.following() {
            return;
        }
        if !follow && self.playing() {
            // leaving the device clock running for everyone else
            self.s.tempo.set(self.s.clock.bpm().clamp(40.0, 240.0));
        } else if self.playing() {
            self.stop();
        }
        self.s.follow.store(follow, Ordering::Relaxed);
    }
    pub fn following(&self) -> bool {
        self.s.follow.load(Ordering::Relaxed)
    }
    pub fn clock(&self) -> &Arc<crate::clock::Clock> {
        &self.s.clock
    }
    /// 0 = straight, 0.33 = a full triplet shuffle.
    pub fn set_swing(&self, s: f32) {
        self.s.swing.set(s.clamp(0.0, 0.4));
    }
    pub fn swing(&self) -> f32 {
        self.s.swing.get()
    }
    /// The app's own output level, before its mixer channel (0..1).
    pub fn set_volume(&self, v: f32) {
        self.s.volume.set(v.clamp(0.0, 1.0));
    }
    pub fn set_reverb(&self, r: f32) {
        self.s.reverb.set(r.clamp(0.0, 1.0));
    }

    /// Starts the clock from step 0.
    pub fn start(&self) {
        self.s.steps.store(0, Ordering::Relaxed);
        if self.following() {
            self.s.clock.start();
        } else {
            self.s.playing.store(true, Ordering::Relaxed);
        }
    }
    pub fn stop(&self) {
        if self.following() {
            self.s.clock.stop();
        } else {
            self.s.playing.store(false, Ordering::Relaxed);
        }
    }
    pub fn playing(&self) -> bool {
        if self.following() {
            self.s.clock.running()
        } else {
            self.s.playing.load(Ordering::Relaxed)
        }
    }
    /// The step most recently played, if the clock is running and has
    /// played one.
    pub fn step(&self) -> Option<u64> {
        let n = self.s.steps.load(Ordering::Relaxed);
        (self.playing() && n > 0).then(|| n - 1)
    }
    /// The output's recent peak, 0..1, for meters and dancing pictures.
    pub fn peak(&self) -> f32 {
        self.s.peak.get()
    }

    pub fn processor(&self, song: Option<Box<dyn Song>>, extra: Option<Box<dyn Extra>>) -> Box<dyn AudioProcessor> {
        Box::new(KidProcessor::new(Arc::clone(&self.s), song, extra))
    }
}

// --- DSP building blocks -------------------------------------------------

/// A small xorshift noise source, one per voice so voices don't share state.
#[derive(Clone, Copy)]
pub struct Noise(u32);
impl Noise {
    pub fn new(seed: u32) -> Noise {
        Noise(seed.max(1))
    }
    /// -1..1.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// The topology-preserving state-variable filter (Zavalishin): one
/// structure gives low-, band- and high-pass and stays stable when the
/// cutoff moves quickly.
#[derive(Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}
pub struct SvfOut {
    pub lp: f32,
    pub bp: f32,
    pub hp: f32,
}
impl Svf {
    #[inline]
    pub fn tick(&mut self, x: f32, cutoff: f32, q: f32, sr: f32) -> SvfOut {
        let g = (PI * (cutoff / sr).clamp(1e-5, 0.49)).tan();
        let k = 1.0 / q.max(0.05);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut { lp: v2, bp: v1, hp: x - k * v1 - v2 }
    }
}

/// PolyBLEP: subtracts the step's aliasing from a naive saw or square
/// near each discontinuity.
#[inline]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// A band-limited sawtooth, -1..1, at phase `p` (0..1) and increment `dt`.
#[inline]
pub fn saw(p: f32, dt: f32) -> f32 {
    2.0 * p - 1.0 - poly_blep(p, dt)
}

/// A band-limited square, -1..1.
#[inline]
pub fn square(p: f32, dt: f32) -> f32 {
    let naive = if p < 0.5 { 1.0 } else { -1.0 };
    naive + poly_blep(p, dt) - poly_blep((p + 0.5).fract(), dt)
}

#[inline]
pub fn midi_hz(n: f32) -> f32 {
    440.0 * 2f32.powf((n - 69.0) / 12.0)
}

/// Equal-power pan: (left gain, right gain).
#[inline]
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * PI;
    (a.cos(), a.sin())
}

/// Attack / decay / sustain / release, linear attack and exponential
/// decay and release, in seconds.
#[derive(Clone, Copy)]
struct Adsr {
    a: f32,
    d: f32,
    s: f32,
    r: f32,
    level: f32,
    stage: u8, // 0 attack, 1 decay/sustain, 2 release, 3 done
}
impl Adsr {
    fn new(a: f32, d: f32, s: f32, r: f32) -> Adsr {
        Adsr { a, d, s, r, level: 0.0, stage: 0 }
    }
    fn release(&mut self) {
        if self.stage < 2 {
            self.stage = 2;
        }
    }
    #[inline]
    fn tick(&mut self, sr: f32) -> f32 {
        match self.stage {
            0 => {
                self.level += 1.0 / (self.a.max(0.0005) * sr);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = 1;
                }
            }
            1 => self.level += (self.s - self.level) * (1.0 - (-1.0 / (self.d.max(0.001) * sr)).exp()),
            2 => {
                self.level *= (-1.0 / (self.r.max(0.001) * sr / 4.6)).exp();
                if self.level < 1e-4 {
                    self.stage = 3;
                    self.level = 0.0;
                }
            }
            _ => {}
        }
        self.level
    }
}

/// The longest Karplus-Strong line: 48 kHz / 4096 is about 12 Hz, below
/// anything a kid's harp plays.
const KS_LEN: usize = 4096;

struct Voice {
    active: bool,
    note: Note,
    freq: f32,
    t: f32,
    released: bool,
    env: Adsr,
    phase: [f32; 4],
    filt: Svf,
    lp: f32,
    noise: Noise,
    ks: Vec<f32>,
    ks_w: usize,
    ks_prev: f32,
    /// Struck instruments: the summed mode amplitude, to know when it's over.
    ring: f32,
}

impl Voice {
    fn new(seed: u32) -> Voice {
        Voice {
            active: false,
            note: Note::new(Tone::Soft, 60.0),
            freq: 261.6,
            t: 0.0,
            released: false,
            env: Adsr::new(0.01, 0.1, 1.0, 0.1),
            phase: [0.0; 4],
            filt: Svf::default(),
            lp: 0.0,
            noise: Noise::new(seed),
            ks: vec![0.0; KS_LEN],
            ks_w: 0,
            ks_prev: 0.0,
            ring: 1.0,
        }
    }

    fn start(&mut self, note: Note, sr: f32) {
        self.active = true;
        self.note = note;
        self.freq = midi_hz(note.pitch);
        self.t = 0.0;
        self.released = false;
        self.phase = [0.0; 4];
        self.filt = Svf::default();
        self.lp = 0.0;
        self.ring = 1.0;
        self.env = match note.tone {
            Tone::Soft => Adsr::new(0.02, 0.25, 0.7, 0.3),
            Tone::Organ => Adsr::new(0.006, 0.05, 1.0, 0.08),
            Tone::Chip => Adsr::new(0.002, 0.12, 0.55, 0.05),
            Tone::Bass => Adsr::new(0.004, 0.3, 0.55, 0.09),
            Tone::Flute => Adsr::new(0.07, 0.2, 0.85, 0.15),
            // Struck and plucked: the gate only damps them early.
            _ => Adsr::new(0.0005, 1.0, 1.0, 0.25),
        };
        if note.tone == Tone::Pluck {
            // Fill the line with one period of noise; a softer pluck
            // starts duller (pre-filtered), like a finger versus a pick.
            let period = (sr / self.freq).clamp(2.0, (KS_LEN - 2) as f32) as usize;
            let bright = 0.25 + 0.7 * note.vel.clamp(0.0, 1.0);
            let mut lp = 0.0;
            for i in 0..KS_LEN {
                if i < period {
                    let n = self.noise.next();
                    lp += (n - lp) * bright;
                    self.ks[i] = lp;
                } else {
                    self.ks[i] = 0.0;
                }
            }
            self.ks_w = period;
            self.ks_prev = 0.0;
        }
    }

    fn release(&mut self) {
        self.released = true;
        self.env.release();
    }

    /// Decay times shrink as the note gets higher, as on a real bar.
    fn bar_scale(&self) -> f32 {
        (440.0 / self.freq).powf(0.35).clamp(0.3, 2.5)
    }

    #[inline]
    fn render(&mut self, sr: f32) -> f32 {
        let dt_s = 1.0 / sr;
        if !self.released && self.note.len > 0.0 && self.t >= self.note.len {
            self.release();
        }
        let f = self.freq;
        let inc = f / sr;
        let out = match self.note.tone {
            Tone::Glock => {
                const RATIO: [f32; 4] = [1.0, 2.756, 5.404, 8.933];
                const AMP: [f32; 4] = [1.0, 0.28, 0.1, 0.04];
                const TAU_S: [f32; 4] = [1.4, 0.45, 0.16, 0.07];
                self.struck(&RATIO, &AMP, &TAU_S, sr, 0.004)
            }
            Tone::Marimba => {
                const RATIO: [f32; 4] = [1.0, 3.93, 9.2, 0.0];
                const AMP: [f32; 4] = [1.0, 0.32, 0.08, 0.0];
                const TAU_S: [f32; 4] = [0.6, 0.12, 0.04, 1.0];
                self.struck(&RATIO, &AMP, &TAU_S, sr, 0.006)
            }
            Tone::Pluck => {
                let d = (sr / f - 0.5).clamp(1.0, (KS_LEN - 2) as f32);
                let read = self.ks_w as f32 - d;
                let read = if read < 0.0 { read + KS_LEN as f32 } else { read };
                let i0 = read as usize % KS_LEN;
                let i1 = (i0 + 1) % KS_LEN;
                let fr = read.fract();
                let y = self.ks[i0] * (1.0 - fr) + self.ks[i1] * fr;
                // The two-point average is the string's loss; low notes
                // ring longer, as on a harp.
                // A released string is damped: about 50 ms to die whatever its
                // pitch (the loss is per trip round the loop, so it's set
                // from the period).
                let loss = if self.released { (-1.0 / (f * 0.05)).exp() } else { 0.996 + 0.003 * (1.0 - (f / 1000.0).min(1.0)) };
                let next = 0.5 * (y + self.ks_prev) * loss;
                self.ks_prev = y;
                self.ks[self.ks_w] = next;
                self.ks_w = (self.ks_w + 1) % KS_LEN;
                self.ring = self.ring * 0.9995 + y.abs() * 0.0005;
                if self.t > 0.05 && self.ring < 2e-4 {
                    self.active = false;
                }
                y * 1.4
            }
            Tone::Soft => {
                let p = self.phase[0];
                self.phase[0] = (p + inc).fract();
                let tri = 1.0 - 4.0 * (p - 0.5).abs();
                let k = 1.0 - (-TAU * 2600.0 / sr).exp();
                self.lp += (tri - self.lp) * k;
                self.lp * self.env.tick(sr) * 0.8
            }
            Tone::Organ => {
                // 8', 4', 2 2/3' and 2' drawbars.
                const H: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
                const A: [f32; 4] = [0.55, 0.3, 0.18, 0.12];
                let mut s = 0.0;
                for i in 0..4 {
                    s += (self.phase[i] * TAU).sin() * A[i];
                    self.phase[i] = (self.phase[i] + inc * H[i]).fract();
                }
                s * self.env.tick(sr)
            }
            Tone::Chip => {
                let p = self.phase[0];
                self.phase[0] = (p + inc).fract();
                square(p, inc) * 0.6 * self.env.tick(sr)
            }
            Tone::Bass => {
                let p = self.phase[0];
                self.phase[0] = (p + inc).fract();
                let e = self.env.tick(sr);
                let cutoff = f * (1.5 + 7.0 * e * self.note.vel);
                self.filt.tick(saw(p, inc), cutoff.min(9000.0), 1.1, sr).lp * e * 0.75
            }
            Tone::Flute => {
                // A gentle vibrato that arrives after the note starts.
                let vib = 1.0 + 0.004 * (self.t * TAU * 5.2).sin() * (self.t / 0.4).min(1.0);
                let p = self.phase[0];
                self.phase[0] = (p + inc * vib).fract();
                let tone = (p * TAU).sin() + 0.18 * (p * 2.0 * TAU).sin();
                let breath = self.filt.tick(self.noise.next(), f * 2.0, 2.0, sr).bp * 0.25;
                (tone * 0.6 + breath) * self.env.tick(sr)
            }
        };
        if self.env.stage == 3 {
            self.active = false;
        }
        self.t += dt_s;
        out * self.note.vel
    }

    /// A struck bar: decaying modes plus a few ms of mallet noise.
    #[inline]
    fn struck(&mut self, ratio: &[f32; 4], amp: &[f32; 4], tau: &[f32; 4], sr: f32, click: f32) -> f32 {
        let scale = self.bar_scale();
        let damp = if self.released { self.env.tick(sr) } else { 1.0 };
        let mut s = 0.0;
        let mut total = 0.0;
        for i in 0..4 {
            if amp[i] == 0.0 {
                continue;
            }
            let fi = self.freq * ratio[i];
            if fi > sr * 0.45 {
                continue;
            }
            let a = amp[i] * (-self.t / (tau[i] * scale)).exp();
            total += a;
            s += (self.phase[i] * TAU).sin() * a;
            self.phase[i] = (self.phase[i] + fi / sr).fract();
        }
        if self.t < click {
            s += self.noise.next() * 0.25 * (1.0 - self.t / click);
        }
        self.ring = total * damp;
        if self.ring < 1e-4 || (self.released && self.env.stage == 3) {
            self.active = false;
        }
        s * damp * 0.6
    }
}

struct DrumVoice {
    active: bool,
    kind: Drum,
    vel: f32,
    t: f32,
    phase: [f32; 6],
    f1: Svf,
    f2: Svf,
    noise: Noise,
}

/// The TR-808's six hi-hat/cymbal oscillator frequencies.
const HAT_HZ: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

impl DrumVoice {
    fn new(seed: u32) -> DrumVoice {
        DrumVoice { active: false, kind: Drum::Kick, vel: 0.0, t: 0.0, phase: [0.0; 6], f1: Svf::default(), f2: Svf::default(), noise: Noise::new(seed) }
    }

    fn start(&mut self, kind: Drum, vel: f32) {
        self.active = true;
        self.kind = kind;
        self.vel = vel.clamp(0.0, 1.0);
        self.t = 0.0;
        self.phase = [0.0; 6];
        self.f1 = Svf::default();
        self.f2 = Svf::default();
    }

    fn length(&self) -> f32 {
        match self.kind {
            Drum::Kick => 0.9,
            Drum::Snare => 0.5,
            Drum::Hat => 0.25,
            Drum::OpenHat => 1.2,
            Drum::Clap => 0.6,
            Drum::Shaker => 0.3,
            Drum::Cowbell => 0.9,
            Drum::HiTom | Drum::LowTom => 0.9,
            Drum::Rim => 0.1,
            Drum::Woodblock => 0.3,
        }
    }

    #[inline]
    fn sine(&mut self, i: usize, f: f32, sr: f32) -> f32 {
        let s = (self.phase[i] * TAU).sin();
        self.phase[i] = (self.phase[i] + f / sr).fract();
        s
    }

    #[inline]
    fn render(&mut self, sr: f32) -> f32 {
        let t = self.t;
        let e = |tau: f32| (-t / tau).exp();
        let out = match self.kind {
            Drum::Kick => {
                let f = 48.0 + 120.0 * e(0.03);
                let body = self.sine(0, f, sr) * e(0.32);
                let click = if t < 0.002 { self.noise.next() * 0.4 } else { 0.0 };
                (body + click) * 1.1
            }
            Drum::Snare => {
                let tone = self.sine(0, 185.0, sr) * 0.45 * e(0.06) + self.sine(1, 330.0, sr) * 0.25 * e(0.04);
                let n = self.noise.next();
                let rattle = self.f1.tick(n, 1800.0, 0.7, sr).hp * 0.75 * e(0.15);
                tone + rattle
            }
            Drum::Hat | Drum::OpenHat => {
                let mut metal = 0.0;
                for i in 0..6 {
                    metal += if self.phase[i] < 0.5 { 1.0 } else { -1.0 };
                    self.phase[i] = (self.phase[i] + HAT_HZ[i] * 1.0 / sr).fract();
                }
                let b = self.f1.tick(metal / 6.0, 10_000.0, 1.2, sr).bp;
                let h = self.f2.tick(b, 7_000.0, 0.7, sr).hp;
                let tau = if self.kind == Drum::Hat { 0.035 } else { 0.28 };
                h * 2.6 * e(tau)
            }
            Drum::Clap => {
                let n = self.noise.next();
                let b = self.f1.tick(n, 1100.0, 1.6, sr).bp;
                // Three quick hands, then the room.
                let mut env = 0.0;
                for k in 0..3 {
                    let dt = t - k as f32 * 0.011;
                    if dt >= 0.0 {
                        env += (-dt / 0.005).exp();
                    }
                }
                if t >= 0.03 {
                    env += 0.7 * (-(t - 0.03) / 0.13).exp();
                }
                b * env * 1.5
            }
            Drum::Shaker => {
                let n = self.noise.next();
                let h = self.f1.tick(n, 5500.0, 0.8, sr).hp;
                let env = (t / 0.012).min(1.0) * e(0.06);
                h * env * 0.7
            }
            Drum::Cowbell => {
                let mut sq = 0.0;
                for (i, f) in [540.0f32, 800.0].iter().enumerate() {
                    sq += if self.phase[i] < 0.5 { 1.0 } else { -1.0 };
                    self.phase[i] = (self.phase[i] + f / sr).fract();
                }
                let b = self.f1.tick(sq * 0.5, 2640.0, 1.0, sr).bp + self.f2.tick(sq * 0.5, 800.0, 1.5, sr).bp * 0.6;
                b * (0.6 * e(0.025) + 0.4 * e(0.25)) * 1.2
            }
            Drum::HiTom | Drum::LowTom => {
                let (base, swing) = if self.kind == Drum::HiTom { (170.0, 80.0) } else { (95.0, 55.0) };
                let f = base + swing * e(0.07);
                self.sine(0, f, sr) * e(0.28) * 0.95
            }
            Drum::Rim => {
                let n = self.noise.next();
                self.sine(0, 1700.0, sr) * 0.5 * e(0.01) + self.f1.tick(n, 3500.0, 2.0, sr).bp * e(0.004)
            }
            Drum::Woodblock => {
                let n = if t < 0.0015 { self.noise.next() * 0.3 } else { 0.0 };
                self.sine(0, 980.0, sr) * 0.7 * e(0.045) + self.sine(1, 980.0 * 2.32, sr) * 0.2 * e(0.015) + n
            }
        };
        self.t += 1.0 / sr;
        if self.t > self.length() {
            self.active = false;
        }
        out * self.vel
    }
}

const VOICES: usize = 20;
const DRUMS: usize = 12;

struct KidProcessor {
    s: Arc<Shared>,
    song: Option<Box<dyn Song>>,
    extra: Option<Box<dyn Extra>>,
    voices: Vec<Voice>,
    drums: Vec<DrumVoice>,
    fx: StereoFx,
    pending: Vec<Ev>,
    /// Scratch for events made during a block, kept to avoid allocating.
    scratch: Vec<Ev>,
    /// Samples until the next step.
    until_step: f64,
    was_playing: bool,
    follower: crate::clock::StepFollower,
    mono: Vec<f32>,
    peak: f32,
}

impl KidProcessor {
    fn new(s: Arc<Shared>, song: Option<Box<dyn Song>>, extra: Option<Box<dyn Extra>>) -> KidProcessor {
        KidProcessor {
            s,
            song,
            extra,
            voices: (0..VOICES).map(|i| Voice::new(0x9E37_79B9 ^ (i as u32 * 7919 + 1))).collect(),
            drums: (0..DRUMS).map(|i| DrumVoice::new(0x85EB_CA6B ^ (i as u32 * 104_729 + 3))).collect(),
            fx: StereoFx::new(),
            pending: Vec::with_capacity(64),
            scratch: Vec::with_capacity(64),
            until_step: 0.0,
            was_playing: false,
            follower: Default::default(),
            mono: Vec::with_capacity(2048),
            peak: 0.0,
        }
    }

    fn handle(&mut self, ev: Ev, sr: f32) {
        match ev {
            Ev::Note(n) => {
                // Free voice, else the quietest (oldest-sounding) one.
                let i = self.voices.iter().position(|v| !v.active).unwrap_or_else(|| {
                    let mut best = 0;
                    let mut best_t = -1.0;
                    for (i, v) in self.voices.iter().enumerate() {
                        let score = v.t + if v.released { 100.0 } else { 0.0 };
                        if score > best_t {
                            best_t = score;
                            best = i;
                        }
                    }
                    best
                });
                self.voices[i].start(n, sr);
            }
            Ev::Off(id) => {
                for v in self.voices.iter_mut().filter(|v| v.active && v.note.id == id && v.note.len <= 0.0) {
                    v.release();
                }
            }
            Ev::AllOff => {
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    v.release();
                }
            }
            Ev::Drum(d, vel) => {
                // A hat cuts the open hat off, as on a real kit.
                if d == Drum::Hat {
                    for dv in self.drums.iter_mut().filter(|dv| dv.active && dv.kind == Drum::OpenHat) {
                        dv.active = false;
                    }
                }
                let i = self.drums.iter().position(|v| !v.active).unwrap_or_else(|| {
                    let mut best = 0;
                    for (i, v) in self.drums.iter().enumerate() {
                        if v.t > self.drums[best].t {
                            best = i;
                        }
                    }
                    best
                });
                self.drums[i].start(d, vel);
            }
            Ev::Custom(a, b, c) => {
                if let Some(x) = self.extra.as_mut() {
                    x.event(a, b, c);
                }
            }
        }
    }

    fn step_samples(&self, step: u64, sr: f32, per_beat: u32) -> f64 {
        let base = 60.0 / self.s.tempo.get().max(20.0) as f64 / per_beat.max(1) as f64 * sr as f64;
        // Swing lengthens the on-beat step of each pair and shortens the
        // off-beat one by the same amount, so the bar stays the same length.
        let sw = self.s.swing.get().clamp(0.0, 0.4) as f64;
        if step % 2 == 0 {
            base * (1.0 + sw)
        } else {
            base * (1.0 - sw)
        }
    }
}

impl AudioProcessor for KidProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sr: f32) {
        let channels = channels.max(1);
        let frames = buffer.len() / channels;
        if let Ok(mut q) = self.s.queue.try_lock() {
            self.pending.extend(q.drain(..));
        }
        let evs: Vec<Ev> = std::mem::take(&mut self.pending);
        for ev in &evs {
            self.handle(*ev, sr);
        }
        self.pending = evs;
        self.pending.clear();

        if let Some(x) = self.extra.as_mut() {
            x.block(frames, sr);
        }
        let follow = self.s.follow.load(Ordering::Relaxed);
        let snap = self.s.clock.snap();
        let playing = if follow { snap.running } else { self.s.playing.load(Ordering::Relaxed) };
        if playing && !self.was_playing {
            self.until_step = 0.0;
        }
        self.was_playing = playing;
        let per_beat = self.song.as_ref().map_or(4, |s| s.steps_per_beat());
        let swing = self.s.swing.get();

        let gain = self.s.volume.get() * (self.s.mix.get() + self.s.ext_mix.get()).clamp(0.0, 2.0);
        let rv = self.s.reverb.get();
        let fxs = FxSettings { reverb_size: 0.45, reverb_mix: 1.0, ..Default::default() };
        self.mono.clear();
        let mut song_evs: Vec<Ev> = std::mem::take(&mut self.scratch);
        let mut peak = 0.0f32;
        for (fi, frame) in buffer.chunks_mut(channels).enumerate() {
            if follow {
                // Sample-locked to the device transport: the same beat
                // every other follower sees on this very sample.
                if let Some(step) = self.follower.poll(&snap, snap.beat_at(fi, sr), per_beat, swing) {
                    if let Some(song) = self.song.as_mut() {
                        song.step(step, &mut song_evs);
                    }
                    for ev in song_evs.drain(..) {
                        self.handle(ev, sr);
                    }
                    self.s.steps.store(step + 1, Ordering::Relaxed);
                }
            } else if playing && self.s.playing.load(Ordering::Relaxed) {
                if self.until_step <= 0.0 {
                    let step = self.s.steps.load(Ordering::Relaxed);
                    if let Some(song) = self.song.as_mut() {
                        song.step(step, &mut song_evs);
                    }
                    for ev in song_evs.drain(..) {
                        self.handle(ev, sr);
                    }
                    self.until_step += self.step_samples(step, sr, per_beat);
                    self.s.steps.store(step + 1, Ordering::Relaxed);
                }
                self.until_step -= 1.0;
            }
            if let Some(x) = self.extra.as_mut() {
                x.poll(&mut song_evs);
            }
            for ev in song_evs.drain(..) {
                self.handle(ev, sr);
            }
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for v in self.voices.iter_mut().filter(|v| v.active) {
                let s = v.render(sr);
                let (gl, gr) = pan_gains(v.note.pan);
                l += s * gl;
                r += s * gr;
            }
            for d in self.drums.iter_mut().filter(|d| d.active) {
                let s = d.render(sr) * 0.7071;
                l += s;
                r += s;
            }
            if let Some(x) = self.extra.as_mut() {
                let (xl, xr) = x.frame(sr);
                l += xl;
                r += xr;
            }
            if rv > 0.0 {
                let send = (l + r) * 0.5;
                let (wl, wr) = self.fx.tick(send, &fxs, sr);
                // `tick` at full mix returns 0.7 dry + the wet; keep only the wet.
                l += (wl - 0.7 * send) * rv * 1.5;
                r += (wr - 0.7 * send) * rv * 1.5;
            }
            // A soft limiter, so a room full of pads can't clip harshly.
            let l = (l * gain * 0.5).tanh();
            let r = (r * gain * 0.5).tanh();
            peak = peak.max(l.abs()).max(r.abs());
            self.mono.push((l + r) * 0.5);
            match frame.len() {
                1 => frame[0] += (l + r) * 0.5,
                _ => {
                    frame[0] += l;
                    frame[1] += r;
                }
            }
        }
        // Jumps up at once, falls back over a few blocks.
        self.scratch = song_evs;
        self.peak = peak.max(self.peak * 0.85);
        self.s.peak.set(self.peak);
        if let Some(bus) = &self.s.bus {
            if let Ok(mut b) = bus.try_lock() {
                b.clear();
                b.extend_from_slice(&self.mono);
            }
        }
    }
}

// ---------------------------------------------------------------------
// Music helpers
// ---------------------------------------------------------------------

/// Major pentatonic: any of these notes sound good together, which is
/// why the youngest kids' instruments use it.
pub const PENTATONIC: [i32; 5] = [0, 2, 4, 7, 9];
pub const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
/// Natural minor.
pub const MINOR_SCALE: [i32; 7] = [0, 2, 3, 5, 7, 8, 10];

/// The `degree`th note (0-based, may run past one octave) of `scale`
/// from `root`.
pub fn scale_note(root: i32, scale: &[i32], degree: i32) -> i32 {
    let n = scale.len() as i32;
    root + 12 * degree.div_euclid(n) + scale[degree.rem_euclid(n) as usize]
}

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

pub fn note_name(n: i32) -> &'static str {
    NOTE_NAMES[n.rem_euclid(12) as usize]
}

/// A small deterministic random-number generator for games (no global
/// state, so a test can seed it).
#[derive(Clone)]
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x2545_F491_4F6C_DD1D | 1)
    }
    pub fn seeded_from_time() -> Rng {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        Rng::new(t)
    }
    pub fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 32) as u32
    }
    /// 0..n
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            self.next_u32() % n
        }
    }
}

// ---------------------------------------------------------------------
// Pictures
// ---------------------------------------------------------------------

pub fn rgb(r: u8, g: u8, b: u8) -> Rgb565 {
    Rgb565::new(r >> 3, g >> 2, b >> 3)
}

/// Mix two colours, `t` = 0 gives `a`, 1 gives `b`.
pub fn blend(a: Rgb565, b: Rgb565, t: f32) -> Rgb565 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb565::new(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Red, orange, yellow, green, teal, blue, indigo, violet.
pub fn rainbow(i: usize) -> Rgb565 {
    const C: [(u8, u8, u8); 8] = [(240, 70, 70), (250, 150, 50), (250, 215, 60), (100, 210, 90), (60, 200, 190), (70, 140, 240), (120, 100, 230), (200, 100, 220)];
    let (r, g, b) = C[i % 8];
    rgb(r, g, b)
}

pub const WHITE: Rgb565 = Rgb565::new(31, 63, 31);
pub const BLACK: Rgb565 = Rgb565::new(0, 0, 0);

pub fn clear(fb: &mut FrameBuffer, c: Rgb565) {
    Rectangle::new(Point::zero(), Size::new(WIDTH as u32, HEIGHT as u32)).into_styled(PrimitiveStyle::with_fill(c)).draw(fb).ok();
}

pub fn rect<D: DrawTarget<Color = Rgb565>>(fb: &mut D, x: i32, y: i32, w: i32, h: i32, c: Rgb565) {
    if w > 0 && h > 0 {
        Rectangle::new(Point::new(x, y), Size::new(w as u32, h as u32)).into_styled(PrimitiveStyle::with_fill(c)).draw(fb).ok();
    }
}

pub fn round_rect<D: DrawTarget<Color = Rgb565>>(fb: &mut D, x: i32, y: i32, w: i32, h: i32, r: u32, c: Rgb565) {
    if w > 0 && h > 0 {
        RoundedRectangle::with_equal_corners(Rectangle::new(Point::new(x, y), Size::new(w as u32, h as u32)), Size::new(r, r)).into_styled(PrimitiveStyle::with_fill(c)).draw(fb).ok();
    }
}

pub fn outline<D: DrawTarget<Color = Rgb565>>(fb: &mut D, x: i32, y: i32, w: i32, h: i32, r: u32, width: u32, c: Rgb565) {
    if w > 0 && h > 0 {
        RoundedRectangle::with_equal_corners(Rectangle::new(Point::new(x, y), Size::new(w as u32, h as u32)), Size::new(r, r)).into_styled(PrimitiveStyle::with_stroke(c, width)).draw(fb).ok();
    }
}

pub fn circle<D: DrawTarget<Color = Rgb565>>(fb: &mut D, cx: i32, cy: i32, r: i32, c: Rgb565) {
    if r > 0 {
        Circle::with_center(Point::new(cx, cy), (r * 2) as u32).into_styled(PrimitiveStyle::with_fill(c)).draw(fb).ok();
    }
}

pub fn ring<D: DrawTarget<Color = Rgb565>>(fb: &mut D, cx: i32, cy: i32, r: i32, width: u32, c: Rgb565) {
    if r > 0 {
        Circle::with_center(Point::new(cx, cy), (r * 2) as u32).into_styled(PrimitiveStyle::with_stroke(c, width)).draw(fb).ok();
    }
}

pub fn line<D: DrawTarget<Color = Rgb565>>(fb: &mut D, x0: i32, y0: i32, x1: i32, y1: i32, width: u32, c: Rgb565) {
    Line::new(Point::new(x0, y0), Point::new(x1, y1)).into_styled(PrimitiveStyle::with_stroke(c, width)).draw(fb).ok();
}

pub fn triangle<D: DrawTarget<Color = Rgb565>>(fb: &mut D, p: [(i32, i32); 3], c: Rgb565) {
    Triangle::new(Point::new(p[0].0, p[0].1), Point::new(p[1].0, p[1].1), Point::new(p[2].0, p[2].1)).into_styled(PrimitiveStyle::with_fill(c)).draw(fb).ok();
}

/// A five-pointed star.
pub fn star<D: DrawTarget<Color = Rgb565>>(fb: &mut D, cx: i32, cy: i32, r: i32, c: Rgb565) {
    let pt = |k: i32, rad: f32| {
        let a = -PI / 2.0 + k as f32 * PI / 5.0;
        (cx + (a.cos() * rad) as i32, cy + (a.sin() * rad) as i32)
    };
    let inner = r as f32 * 0.42;
    for k in 0..5 {
        let tip = pt(2 * k, r as f32);
        let a = pt(2 * k - 1, inner);
        let b = pt(2 * k + 1, inner);
        triangle(fb, [tip, a, b], c);
        triangle(fb, [(cx, cy), a, b], c);
    }
}

/// Text sizes: 6x12, 8x16, 16x32, and the 16x32 font doubled (32x64)
/// for the few words a young child needs to see from arm's length.
#[derive(Clone, Copy, PartialEq)]
pub enum Size2 {
    Small,
    Medium,
    Large,
    Huge,
}

fn font(s: Size2) -> &'static MonoFont<'static> {
    match s {
        Size2::Small => &SPLEEN_6X12,
        Size2::Medium => &SPLEEN_8X16,
        Size2::Large | Size2::Huge => &SPLEEN_16X32,
    }
}

/// Draws `s` with its top edge at `y`. `align`: -1 left of x, 0 centred
/// on x, 1 right of x.
pub fn text(fb: &mut FrameBuffer, s: &str, x: i32, y: i32, size: Size2, c: Rgb565, align: i8) {
    let alignment = match align {
        0 => Alignment::Center,
        1 => Alignment::Right,
        _ => Alignment::Left,
    };
    let ts = TextStyleBuilder::new().baseline(Baseline::Top).alignment(alignment).build();
    let style = MonoTextStyle::new(font(size), c);
    if size == Size2::Huge {
        let mut big = Scaled { fb, k: 2 };
        Text::with_text_style(s, Point::new(x.div_euclid(2), y.div_euclid(2)), style, ts).draw(&mut big).ok();
    } else {
        Text::with_text_style(s, Point::new(x, y), style, ts).draw(fb).ok();
    }
}

/// The width a string takes at a size, in pixels.
pub fn text_width(s: &str, size: Size2) -> i32 {
    let w = font(size).character_size.width as i32 * s.chars().count() as i32;
    if size == Size2::Huge {
        w * 2
    } else {
        w
    }
}

/// Wraps `s` to lines of at most `max_px` at a size, then draws them.
/// Returns the y just below the last line.
pub fn paragraph(fb: &mut FrameBuffer, s: &str, x: i32, y: i32, max_px: i32, size: Size2, c: Rgb565) -> i32 {
    let cw = font(size).character_size.width as i32;
    let lh = font(size).character_size.height as i32 + 3;
    let max_chars = (max_px / cw).max(1) as usize;
    let mut yy = y;
    for para in s.split('\n') {
        let mut line_s = String::new();
        for word in para.split_whitespace() {
            if !line_s.is_empty() && line_s.chars().count() + 1 + word.chars().count() > max_chars {
                text(fb, &line_s, x, yy, size, c, -1);
                yy += lh;
                line_s.clear();
            }
            if !line_s.is_empty() {
                line_s.push(' ');
            }
            line_s.push_str(word);
        }
        text(fb, &line_s, x, yy, size, c, -1);
        yy += lh;
    }
    yy
}

/// Draws onto the frame `k` times bigger: one source pixel becomes a
/// k x k block. Lets the 16x32 font (and any other shape) be drawn huge.
pub struct Scaled<'a> {
    pub fb: &'a mut FrameBuffer,
    pub k: i32,
}
impl OriginDimensions for Scaled<'_> {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32 / self.k as u32, HEIGHT as u32 / self.k as u32)
    }
}
impl DrawTarget for Scaled<'_> {
    type Color = Rgb565;
    type Error = core::convert::Infallible;
    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(&mut self, pixels: I) -> Result<(), Self::Error> {
        let k = self.k;
        for Pixel(p, c) in pixels {
            rect(self.fb, p.x * k, p.y * k, k, k, c);
        }
        Ok(())
    }
}

/// The strip along the top of every Kids screen: the app's name, which
/// age group it's for, and the way home.
pub fn header(fb: &mut FrameBuffer, title: &str, ages: &str, bg: Rgb565, ink: Rgb565) {
    rect(fb, 0, 0, WIDTH as i32, 30, bg);
    text(fb, title, 12, 7, Size2::Medium, ink, -1);
    let tw = text_width(title, Size2::Medium);
    round_rect(fb, 22 + tw, 7, text_width(ages, Size2::Small) + 12, 16, 8, blend(bg, ink, 0.25));
    text(fb, ages, 28 + tw, 9, Size2::Small, ink, -1);
    text(fb, "F1: home", WIDTH as i32 - 10, 9, Size2::Small, blend(bg, ink, 0.6), 1);
}

/// A one-line help strip along the bottom.
pub fn footer(fb: &mut FrameBuffer, s: &str, bg: Rgb565, ink: Rgb565) {
    rect(fb, 0, HEIGHT as i32 - 22, WIDTH as i32, 22, bg);
    text(fb, s, 12, HEIGHT as i32 - 17, Size2::Small, ink, -1);
}

/// The frame as RGBA bytes for the Slint GUI.
#[allow(dead_code)] // Slint GUI only
pub fn frame_rgba(fb: &FrameBuffer) -> Vec<u8> {
    let mut out = Vec::with_capacity(WIDTH * HEIGHT * 4);
    for &p in fb.buffer() {
        out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, 255]);
    }
    out
}

/// What a Kids app hands the Slint GUI: its own full screen.
#[allow(dead_code)] // Slint GUI only
pub fn screen_extra(fb: &FrameBuffer) -> SlintExtra {
    SlintExtra::Screen(crate::app::ScreenExtra { frame_rgba: frame_rgba(fb), width: WIDTH as u32, height: HEIGHT as u32 })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Runs `blocks` blocks of 512 stereo frames and returns them.
    pub fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        let mut buf = vec![0.0f32; 1024];
        for _ in 0..blocks {
            buf.fill(0.0);
            p.process(&mut buf, 2, 48_000.0);
            all.extend_from_slice(&buf);
        }
        all
    }

    pub fn energy(x: &[f32]) -> f32 {
        x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32
    }

    /// The loudest frequency in a signal: an FFT peak, refined with a
    /// parabola through the bins either side. Fine for these tones, whose
    /// fundamental is their strongest partial.
    pub fn pitch_hz(x: &[f32], sr: f32) -> f32 {
        use rustfft::num_complex::Complex;
        let n = 32768;
        let mut buf: Vec<Complex<f32>> = (0..n)
            .map(|i| {
                let w = 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos();
                Complex::new(x.get(i).copied().unwrap_or(0.0) * w, 0.0)
            })
            .collect();
        rustfft::FftPlanner::new().plan_fft_forward(n).process(&mut buf);
        let mag: Vec<f32> = buf[..n / 2].iter().map(|c| c.norm()).collect();
        let k = (2..n / 2 - 1).max_by(|&a, &b| mag[a].total_cmp(&mag[b])).unwrap();
        let (a, b, c) = (mag[k - 1].ln(), mag[k].ln(), mag[k + 1].ln());
        (k as f32 + 0.5 * (a - c) / (a - 2.0 * b + c)) * sr / n as f32
    }

    #[test]
    fn every_tone_and_drum_sounds_then_ends() {
        for tone in Tone::ALL {
            let snd = Sound::detached();
            snd.set_reverb(0.0);
            let mut p = snd.processor(None, None);
            snd.play(Note::new(tone, 60.0).len(0.2));
            let out = render(&mut p, 20);
            assert!(energy(&out) > 1e-4, "{tone:?} is silent");
            assert!(out.iter().all(|x| x.is_finite() && x.abs() <= 1.0), "{tone:?} bounded");
            let tail = render(&mut p, 400);
            assert!(energy(&tail[tail.len() - 4096..]) < 1e-8, "{tone:?} rings forever");
        }
        for d in [Drum::Kick, Drum::Snare, Drum::Hat, Drum::OpenHat, Drum::Clap, Drum::Shaker, Drum::Cowbell, Drum::HiTom, Drum::LowTom, Drum::Rim, Drum::Woodblock] {
            let snd = Sound::detached();
            snd.set_reverb(0.0);
            let mut p = snd.processor(None, None);
            snd.drum(d, 1.0);
            let out = render(&mut p, 10);
            assert!(energy(&out) > 1e-5, "{d:?} is silent");
            let tail = render(&mut p, 200);
            assert!(energy(&tail[tail.len() - 4096..]) < 1e-9, "{d:?} rings forever");
        }
    }

    #[test]
    fn notes_are_in_tune() {
        for (tone, pitch) in [(Tone::Glock, 69.0), (Tone::Pluck, 57.0), (Tone::Soft, 69.0), (Tone::Flute, 69.0)] {
            let snd = Sound::detached();
            snd.set_reverb(0.0);
            let mut p = snd.processor(None, None);
            snd.play(Note::new(tone, pitch).len(1.0));
            let out = render(&mut p, 70);
            let left: Vec<f32> = out.iter().step_by(2).copied().skip(2400).collect();
            let hz = pitch_hz(&left, 48_000.0);
            let want = midi_hz(pitch);
            // A plucked string's loudest partial depends on the random
            // pluck, so it only has to be one of the note's harmonics.
            let ratio = if tone == Tone::Pluck { hz / want / (hz / want).round().max(1.0) } else { hz / want };
            assert!((ratio - 1.0).abs() < 0.01, "{tone:?}: {hz} Hz, want {want}");
        }
    }

    #[test]
    fn a_held_note_lasts_until_released() {
        let snd = Sound::detached();
        snd.set_reverb(0.0);
        let mut p = snd.processor(None, None);
        snd.play(Note::new(Tone::Organ, 60.0).held(7));
        render(&mut p, 200);
        assert!(energy(&render(&mut p, 4)) > 1e-3, "still sounding");
        snd.off(7);
        render(&mut p, 20);
        assert!(energy(&render(&mut p, 4)) < 1e-8, "released");
    }

    struct Every4;
    impl Song for Every4 {
        fn step(&mut self, step: u64, out: &mut Vec<Ev>) {
            if step % 4 == 0 {
                out.push(Ev::Drum(Drum::Rim, 1.0));
            }
        }
    }

    #[test]
    fn the_clock_plays_steps_on_time() {
        let snd = Sound::detached();
        snd.set_reverb(0.0);
        snd.set_tempo(120.0);
        let mut p = snd.processor(Some(Box::new(Every4)), None);
        snd.start();
        let out = render(&mut p, 94); // ~1 s
        // 120 bpm: a beat every 0.5 s = 24000 frames. Find the onsets.
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let mut onsets = Vec::new();
        let mut quiet = 0;
        for (i, x) in left.iter().enumerate() {
            if x.abs() > 0.05 && quiet > 2000 {
                onsets.push(i);
            }
            quiet = if x.abs() > 0.05 { 0 } else { quiet + 1 };
        }
        assert!(onsets.len() >= 1 && left[..200].iter().any(|x| x.abs() > 0.05), "first beat at once");
        assert!(onsets.iter().any(|&o| (o as i32 - 24_000).abs() < 12), "second beat at 0.5 s: {onsets:?}");
        assert_eq!(snd.step().map(|s| s >= 8), Some(true));
    }

    #[test]
    fn huge_text_draws_twice_the_size() {
        let mut fb = FrameBuffer::new();
        text(&mut fb, "A", 100, 100, Size2::Huge, WHITE, -1);
        let lit: Vec<usize> = fb.buffer().iter().enumerate().filter(|(_, &p)| p != 0).map(|(i, _)| i).collect();
        let ys: Vec<usize> = lit.iter().map(|i| i / WIDTH).collect();
        let h = ys.iter().max().unwrap() - ys.iter().min().unwrap();
        assert!(h > 32, "{h}");
    }
}
