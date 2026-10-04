//! Monster Mic (Kids, ages 6-9): sing or talk into the microphone and
//! come out as someone else.
//!
//! The top two rows of pads pick a voice: Me, Robot, Chipmunk, Monster,
//! Alien, Cave, Ghost and Underwater. The bottom two rows are a parrot:
//! hold the first pad to record up to five seconds, then the other seven
//! play it back at seven pitches (the major pentatonic around your own
//! voice), through whichever voice is picked, so a "la" becomes a little
//! keyboard. Up/down sets how loud the live microphone is (all the way
//! down turns it off; headphones stop the speaker feeding back).
//!
//! The effects are the standard ones:
//! - pitch shifting: two read heads sweep through a short delay line at
//!   a different speed from the write head, crossfaded so each jump back
//!   is hidden while the other head is loudest (a Doppler shifter, as in
//!   the classic Eventide H910).
//! - Robot: ring modulation by a 110 Hz sine plus sample-rate reduction.
//! - Alien: ring modulation by a carrier that slowly sweeps.
//! - Cave: a long echo plus the room.
//! - Underwater: a low-pass filter whose cutoff wobbles.
//!
//! The input is whatever Source is chosen with left/right; it starts on
//! the device's hardware input, which only listens once an input has been
//! chosen in Settings.

use crate::app::{App, Input, SlintExtra};
use crate::audio::AudioProcessor;
use crate::audio_bus::{AudioBus, NO_SOURCE};
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::util::AtomicF32;
use crate::apps::kids_kit::{self as kit, Extra, Size2, Sound, Svf};
use embedded_graphics::pixelcolor::Rgb565;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NAME: &str = "Monster Mic";
const CLIP_SECONDS: f32 = 5.0;
const MAX_SR: f32 = 96_000.0;
/// Pitch shifter window, seconds: long enough for low voices, short
/// enough not to sound like an echo.
const WINDOW_S: f32 = 0.05;
const PARROT_PITCHES: [i32; 7] = [-5, -3, 0, 2, 4, 7, 9];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceFx {
    Me,
    Robot,
    Chipmunk,
    Monster,
    Alien,
    Cave,
    Ghost,
    Underwater,
}

const VOICES: [VoiceFx; 8] = [VoiceFx::Me, VoiceFx::Robot, VoiceFx::Chipmunk, VoiceFx::Monster, VoiceFx::Alien, VoiceFx::Cave, VoiceFx::Ghost, VoiceFx::Underwater];

impl VoiceFx {
    fn name(self) -> &'static str {
        match self {
            VoiceFx::Me => "Me",
            VoiceFx::Robot => "Robot",
            VoiceFx::Chipmunk => "Chipmunk",
            VoiceFx::Monster => "Monster",
            VoiceFx::Alien => "Alien",
            VoiceFx::Cave => "Cave",
            VoiceFx::Ghost => "Ghost",
            VoiceFx::Underwater => "Underwater",
        }
    }
    /// Semitones the voice is shifted by.
    fn shift(self) -> f32 {
        match self {
            VoiceFx::Chipmunk => 9.0,
            VoiceFx::Monster => -8.0,
            VoiceFx::Ghost => 3.0,
            _ => 0.0,
        }
    }
    fn reverb(self) -> f32 {
        match self {
            VoiceFx::Cave => 0.9,
            VoiceFx::Ghost => 0.8,
            VoiceFx::Alien => 0.4,
            _ => 0.15,
        }
    }
    fn color(self) -> Rgb565 {
        match self {
            VoiceFx::Me => kit::rgb(240, 180, 140),
            VoiceFx::Robot => kit::rgb(160, 170, 190),
            VoiceFx::Chipmunk => kit::rgb(200, 140, 80),
            VoiceFx::Monster => kit::rgb(120, 200, 90),
            VoiceFx::Alien => kit::rgb(120, 230, 200),
            VoiceFx::Cave => kit::rgb(150, 120, 100),
            VoiceFx::Ghost => kit::rgb(235, 235, 250),
            VoiceFx::Underwater => kit::rgb(70, 150, 240),
        }
    }
}

/// A pitch shifter: two read heads half a window apart, each faded in
/// and out with a sine-squared window, so their sum is always unity.
struct Shifter {
    buf: Vec<f32>,
    w: usize,
    /// 0..1 through the window.
    phase: f32,
}

impl Shifter {
    fn new() -> Shifter {
        Shifter { buf: vec![0.0; (MAX_SR * WINDOW_S * 2.0) as usize + 8], w: 0, phase: 0.0 }
    }
    fn read(&self, delay: f32) -> f32 {
        let n = self.buf.len();
        let pos = self.w as f32 - delay;
        let pos = if pos < 0.0 { pos + n as f32 } else { pos };
        let i = pos as usize % n;
        let f = pos.fract();
        self.buf[i] * (1.0 - f) + self.buf[(i + 1) % n] * f
    }
    #[inline]
    fn tick(&mut self, x: f32, semis: f32, sr: f32) -> f32 {
        let n = self.buf.len();
        self.buf[self.w] = x;
        self.w = (self.w + 1) % n;
        if semis == 0.0 {
            return x;
        }
        let ratio = 2f32.powf(semis / 12.0);
        let win = WINDOW_S * sr;
        // The delay changes by (1 - ratio) samples per sample: shrinking
        // (reading faster) raises the pitch.
        self.phase = (self.phase + (1.0 - ratio) / win).rem_euclid(1.0);
        let mut out = 0.0;
        for k in 0..2 {
            let p = (self.phase + k as f32 * 0.5).fract();
            let g = (p * std::f32::consts::PI).sin();
            out += self.read(1.0 + p * win) * g * g;
        }
        out
    }
}

struct Shared {
    voice: AtomicUsize,
    mic: AtomicF32,
    recording: AtomicBool,
    /// Samples recorded into the clip.
    clip_len: AtomicUsize,
    level: AtomicF32,
    /// The input buffer to read, chosen on the UI thread.
    source: Mutex<Option<Arc<Mutex<Vec<f32>>>>>,
    /// Where each parrot voice is in the clip, as a fraction, for the screen.
    playhead: AtomicF32,
}

struct Play {
    pos: f32,
    rate: f32,
}

struct Changer {
    s: Arc<Shared>,
    input: Vec<f32>,
    read: usize,
    clip: Vec<f32>,
    plays: Vec<Play>,
    shifter: Shifter,
    t: f32,
    hold: f32,
    hold_n: u32,
    echo: Vec<f32>,
    echo_w: usize,
    echo_lp: f32,
    filt: Svf,
    hp: Svf,
    gate: f32,
    level: f32,
    rec_w: usize,
}

impl Changer {
    fn new(s: Arc<Shared>) -> Changer {
        Changer {
            s,
            input: Vec::with_capacity(4096),
            read: 0,
            clip: vec![0.0; (MAX_SR * CLIP_SECONDS) as usize],
            plays: Vec::with_capacity(8),
            shifter: Shifter::new(),
            t: 0.0,
            hold: 0.0,
            hold_n: 0,
            echo: vec![0.0; MAX_SR as usize],
            echo_w: 0,
            echo_lp: 0.0,
            filt: Svf::default(),
            hp: Svf::default(),
            gate: 0.0,
            level: 0.0,
            rec_w: 0,
        }
    }

    fn effect(&mut self, x: f32, v: VoiceFx, sr: f32) -> f32 {
        self.t += 1.0 / sr;
        let y = self.shifter.tick(x, v.shift(), sr);
        match v {
            VoiceFx::Robot => {
                let ring = y * (self.t * TAU * 110.0).sin();
                // Sample-rate reduction to 8 kHz: hold every few samples.
                let every = (sr / 8000.0).round().max(1.0) as u32;
                if self.hold_n == 0 {
                    self.hold = (ring * 24.0).round() / 24.0;
                }
                self.hold_n = (self.hold_n + 1) % every;
                self.hold * 1.6
            }
            VoiceFx::Alien => {
                let carrier = 300.0 + 220.0 * (self.t * TAU * 0.4).sin();
                y * (self.t * TAU * carrier).sin() * 1.4 + y * 0.3
            }
            VoiceFx::Monster => {
                // Darker, with a little growl.
                let lp = self.filt.tick(y, 1800.0, 0.8, sr).lp;
                (lp * 2.2).tanh() * 0.9
            }
            VoiceFx::Cave => {
                let d = (0.38 * sr) as usize;
                let n = self.echo.len();
                let back = self.echo[(self.echo_w + n - d.min(n - 1)) % n];
                self.echo_lp += (back - self.echo_lp) * 0.25;
                self.echo[self.echo_w] = y + self.echo_lp * 0.55;
                self.echo_w = (self.echo_w + 1) % n;
                y + self.echo_lp * 0.8
            }
            VoiceFx::Ghost => {
                let trem = 0.7 + 0.3 * (self.t * TAU * 5.0).sin();
                let vib = self.filt.tick(y, 2500.0, 0.7, sr).lp;
                vib * trem
            }
            VoiceFx::Underwater => {
                let cutoff = 450.0 + 250.0 * (self.t * TAU * 1.3).sin();
                self.filt.tick(y, cutoff, 2.5, sr).lp * 1.6
            }
            VoiceFx::Me | VoiceFx::Chipmunk => y,
        }
    }
}

impl Extra for Changer {
    fn block(&mut self, frames: usize, _sr: f32) {
        self.input.clear();
        self.read = 0;
        if let Ok(src) = self.s.source.try_lock() {
            if let Some(buf) = src.as_ref() {
                if let Ok(b) = buf.try_lock() {
                    self.input.extend(b.iter().take(frames));
                }
            }
        }
    }

    fn event(&mut self, a: u32, b: f32, _c: f32) {
        match a {
            // Play the clip back, `b` semitones up.
            1 => {
                if self.plays.len() >= 6 {
                    self.plays.remove(0);
                }
                self.plays.push(Play { pos: 0.0, rate: 2f32.powf(b / 12.0) });
            }
            // Start recording.
            2 => self.rec_w = 0,
            _ => {}
        }
    }

    fn frame(&mut self, sr: f32) -> (f32, f32) {
        let raw = self.input.get(self.read).copied().unwrap_or(0.0);
        self.read += 1;
        // Remove rumble, then a gentle gate so room hiss stays quiet.
        let x = self.hp.tick(raw, 90.0, 0.7, sr).hp;
        let a = x.abs();
        self.level = if a > self.level { a } else { self.level * 0.9995 };
        let open = if self.level > 0.006 { 1.0 } else { 0.0 };
        self.gate += (open - self.gate) * if open > self.gate { 0.01 } else { 0.0005 };
        let x = x * self.gate;

        if self.s.recording.load(Ordering::Relaxed) && self.rec_w < self.clip.len() {
            self.clip[self.rec_w] = x;
            self.rec_w += 1;
            self.s.clip_len.store(self.rec_w, Ordering::Relaxed);
        }
        let len = self.s.clip_len.load(Ordering::Relaxed);
        let mut parrot = 0.0;
        let mut head = -1.0;
        for p in self.plays.iter_mut() {
            let i = p.pos as usize;
            if i + 1 < len {
                let f = p.pos.fract();
                parrot += self.clip[i] * (1.0 - f) + self.clip[i + 1] * f;
                head = p.pos / len as f32;
            }
            p.pos += p.rate;
        }
        self.plays.retain(|p| (p.pos as usize) + 1 < len);
        self.s.playhead.set(head);

        let live = x * self.s.mic.get();
        let v = VOICES[self.s.voice.load(Ordering::Relaxed) % VOICES.len()];
        let y = self.effect(live + parrot, v, sr);
        (y, y)
    }
}

pub struct MonsterMic {
    sound: Sound,
    s: Arc<Shared>,
    bus: Arc<AudioBus>,
    source: usize,
    prev: [bool; 16],
    poll: u32,
    frame: u64,
    parrot_glow: [f32; 7],
}

impl MonsterMic {
    pub fn new(sound: Sound, bus: Arc<AudioBus>) -> MonsterMic {
        let s = Arc::new(Shared {
            voice: AtomicUsize::new(0),
            mic: AtomicF32::new(0.7),
            recording: AtomicBool::new(false),
            clip_len: AtomicUsize::new(0),
            level: AtomicF32::new(0.0),
            source: Mutex::new(None),
            playhead: AtomicF32::new(-1.0),
        });
        sound.set_reverb(VOICES[0].reverb());
        let mut m = MonsterMic { sound, s, bus, source: NO_SOURCE, prev: [false; 16], poll: 0, frame: 0, parrot_glow: [0.0; 7] };
        m.source = m.default_source();
        m.connect();
        m
    }

    /// The device's microphone input, if there is one.
    fn default_source(&self) -> usize {
        let names = self.bus.names();
        names.iter().position(|n| n == "Hardware input").or_else(|| names.iter().position(|n| n.contains("Audio In"))).unwrap_or(NO_SOURCE)
    }

    fn connect(&mut self) {
        let buf = if self.source == NO_SOURCE { None } else { self.bus.get(self.source) };
        *self.s.source.lock().unwrap() = buf;
    }

    fn voice(&self) -> VoiceFx {
        VOICES[self.s.voice.load(Ordering::Relaxed) % VOICES.len()]
    }

    fn clip_seconds(&self) -> f32 {
        self.s.clip_len.load(Ordering::Relaxed) as f32 / 48_000.0
    }
}

impl App for MonsterMic {
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn needs_background_audio(&self) -> bool {
        false
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![
            ("Voice".into(), self.voice().name().into(), false),
            ("Source".into(), self.bus.source_name(self.source), false),
            ("Mic".into(), format!("{:.0}%", self.s.mic.get() * 100.0), false),
        ]
    }
    fn on_exit(&mut self) {
        self.s.recording.store(false, Ordering::Relaxed);
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let v = self.s.voice.load(Ordering::Relaxed);
        let rec = self.s.recording.load(Ordering::Relaxed);
        let has_clip = self.s.clip_len.load(Ordering::Relaxed) > 0;
        std::array::from_fn(|p| match p {
            0..=7 if p == v => PadColor::Yellow,
            0..=7 => PadColor::Blue,
            8 => PadColor::Red,
            _ if rec => PadColor::Off,
            _ if has_clip => PadColor::Green,
            _ => PadColor::Off,
        })
    }

    fn tick(&mut self, input: &Input) {
        self.frame += 1;
        for p in 0..16 {
            let down = input.grid[p] && !self.prev[p];
            if down && p < 8 {
                self.s.voice.store(p, Ordering::Relaxed);
                self.sound.set_reverb(VOICES[p].reverb());
            }
            if down && p > 8 {
                let k = p - 9;
                self.sound.custom(1, PARROT_PITCHES[k] as f32, 0.0);
                self.parrot_glow[k] = 1.0;
            }
        }
        // Pad 9 records while it's held.
        let rec = input.grid[8];
        if rec && !self.prev[8] {
            self.s.clip_len.store(0, Ordering::Relaxed);
            self.sound.custom(2, 0.0, 0.0);
        }
        self.s.recording.store(rec, Ordering::Relaxed);
        self.prev = input.grid;
        if input.navigation_steps != 0 {
            let m = (self.s.mic.get() - input.navigation_steps as f32 * 0.1).clamp(0.0, 1.0);
            self.s.mic.set((m * 10.0).round() / 10.0);
        }
        if input.knob2 != 0 {
            self.source = crate::audio_bus::cycle_source(self.source, input.knob2.signum(), self.bus.len());
            self.connect();
        }
        // Keep telling the bus we're listening, so the input stays awake.
        self.poll += 1;
        if self.poll % 15 == 0 && self.source != NO_SOURCE {
            let _ = self.bus.get(self.source);
        }
        let lvl = self.sound.peak();
        self.s.level.set(lvl);
        for g in self.parrot_glow.iter_mut() {
            *g = (*g - 0.05).max(0.0);
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let v = self.voice();
        let bg = kit::blend(kit::PAPER, v.color(), 0.12);
        kit::clear(fb, bg);
        kit::kids_header(fb, NAME, "AGES 6-9");

        // The face of the current voice; its mouth follows your voice.
        let level = self.sound.peak();
        draw_face(fb, v, 170, 170, level, self.frame);
        kit::text(fb, v.name(), 170, 282, Size2::Large, kit::INK, 0);

        // The eight voices as the top two rows of pads.
        let (px, py) = (340, 44);
        for (i, vv) in VOICES.iter().enumerate() {
            let x = px + (i as i32 % 4) * 72;
            let y = py + (i as i32 / 4) * 60;
            let sel = *vv == v;
            kit::card(fb, x, y, 66, 54, 10, if sel { vv.color() } else { kit::blend(kit::PAPER, vv.color(), 0.2) });
            if sel { kit::outline(fb, x - 2, y - 2, 70, 58, 12, 2, kit::TEAL); }
            kit::text(fb, vv.name(), x + 33, y + 21, Size2::Small, kit::INK, 0);
        }
        // The parrot: record pad and seven pitch pads.
        let y2 = py + 130;
        let rec = self.s.recording.load(Ordering::Relaxed);
        kit::card(fb, px, y2, 66, 54, 10, if rec { kit::rgb(250, 60, 60) } else { kit::rgb(120, 40, 40) });
        kit::circle(fb, px + 33, y2 + 20, 9, kit::WHITE);
        kit::text(fb, "hold", px + 33, y2 + 34, Size2::Small, kit::WHITE, 0);
        let has_clip = self.clip_seconds() > 0.05;
        for k in 0..7 {
            let i = k + 1;
            let x = px + (i as i32 % 4) * 72;
            let y = y2 + (i as i32 / 4) * 60;
            let g = self.parrot_glow[k];
            let c = if has_clip { kit::blend(kit::rainbow(k), kit::WHITE, g * 0.6) } else { kit::blend(kit::PAPER, kit::rainbow(k), 0.15) };
            kit::card(fb, x, y, 66, 54, 10, c);
            let label = match PARROT_PITCHES[k] {
                0 => "parrot".to_string(),
                n => format!("{n:+}"),
            };
            if !has_clip { kit::text(fb, "no clip", x + 33, y + 36, Size2::Small, kit::MUTED, 0); }
            kit::text(fb, &label, x + 33, y + 21, Size2::Small, kit::BLACK, 0);
        }
        // The clip and the mic.
        let ly = 304;
        kit::card(fb, px, ly, 282, 14, 7, kit::blend(bg, kit::TEAL, 0.15));
        let frac = (self.clip_seconds() / CLIP_SECONDS).min(1.0);
        kit::card(fb, px, ly, (282.0 * frac) as i32 + 6, 14, 7, if rec { kit::rgb(250, 60, 60) } else { kit::rgb(120, 220, 140) });
        let head = self.s.playhead.get();
        if head >= 0.0 {
            kit::rect(fb, px + (282.0 * frac * head) as i32, ly - 3, 3, 20, kit::WHITE);
        }
        let source_name: String = self.bus.source_name(self.source).chars().take(35).collect();
        let status = if self.source == NO_SOURCE {
            "No microphone: ask a grown-up to pick one in Settings > Input".to_string()
        } else if self.s.mic.get() <= 0.0 {
            format!("Live mic off (up/down)   input: {}", source_name)
        } else {
            format!("Live mic {:.0}% (up/down)   input: {} (< >)", self.s.mic.get() * 100.0, source_name)
        };
        kit::kids_footer(fb, &status);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(self.sound.processor(None, Some(Box::new(Changer::new(Arc::clone(&self.s))))))
    }
}

fn draw_face(fb: &mut FrameBuffer, v: VoiceFx, x: i32, y: i32, level: f32, frame: u64) {
    let c = v.color();
    let dark = kit::blend(c, kit::BLACK, 0.7);
    let open = (level * 3.0).min(1.0);
    match v {
        VoiceFx::Robot => {
            kit::rect(fb, x - 4, y - 112, 8, 24, dark);
            kit::circle(fb, x, y - 114, 8, kit::rgb(250, 80, 80));
            kit::round_rect(fb, x - 90, y - 90, 180, 170, 18, c);
            kit::rect(fb, x - 104, y - 30, 14, 50, dark);
            kit::rect(fb, x + 90, y - 30, 14, 50, dark);
            for s in [-1, 1] {
                kit::rect(fb, x + s * 38 - 18, y - 50, 36, 30, kit::rgb(30, 40, 50));
                kit::rect(fb, x + s * 38 - 8, y - 42, 16, 14, kit::rgb(90, 240, 255));
            }
            let h = 10 + (open * 36.0) as i32;
            kit::rect(fb, x - 50, y + 20, 100, h, kit::rgb(30, 40, 50));
            for k in 0..5 {
                kit::rect(fb, x - 46 + k * 20, y + 22, 12, (h - 4).max(2), kit::rgb(90, 240, 255));
            }
        }
        VoiceFx::Ghost => {
            let sway = ((frame as f32 * 0.05).sin() * 6.0) as i32;
            kit::circle(fb, x + sway, y - 20, 86, c);
            kit::rect(fb, x - 86 + sway, y - 20, 172, 100, c);
            for k in 0..4 {
                kit::circle(fb, x - 64 + k * 43 + sway, y + 80, 22, c);
            }
            for s in [-1, 1] {
                kit::circle(fb, x + s * 30 + sway, y - 30, 14, kit::BLACK);
            }
            kit::round_rect(fb, x - 14 + sway, y + 10, 28, 12 + (open * 34.0) as i32, 14, kit::BLACK);
        }
        _ => {
            kit::circle(fb, x, y, 92, c);
            match v {
                VoiceFx::Chipmunk => {
                    kit::circle(fb, x - 70, y - 80, 26, c);
                    kit::circle(fb, x + 70, y - 80, 26, c);
                    kit::circle(fb, x - 70, y - 80, 14, kit::rgb(240, 170, 150));
                    kit::circle(fb, x + 70, y - 80, 14, kit::rgb(240, 170, 150));
                    kit::circle(fb, x - 54, y + 26, 26, kit::rgb(240, 200, 160));
                    kit::circle(fb, x + 54, y + 26, 26, kit::rgb(240, 200, 160));
                }
                VoiceFx::Monster => {
                    kit::triangle(fb, [(x - 60, y - 70), (x - 76, y - 120), (x - 36, y - 84)], kit::WHITE);
                    kit::triangle(fb, [(x + 60, y - 70), (x + 76, y - 120), (x + 36, y - 84)], kit::WHITE);
                }
                VoiceFx::Alien => {
                    for s in [-1, 1] {
                        kit::line(fb, x + s * 30, y - 84, x + s * 54, y - 126, 4, dark);
                        kit::circle(fb, x + s * 54, y - 126, 10, kit::rgb(250, 120, 220));
                    }
                }
                VoiceFx::Underwater => {
                    // A diving mask.
                    kit::round_rect(fb, x - 76, y - 64, 152, 54, 22, kit::rgb(40, 60, 90));
                    kit::round_rect(fb, x - 68, y - 58, 136, 42, 18, kit::rgb(170, 220, 250));
                    let b = (frame / 4 % 40) as i32;
                    kit::ring(fb, x + 100, y - b * 3, 8, 2, kit::WHITE);
                    kit::ring(fb, x + 120, y + 30 - b * 2, 5, 2, kit::WHITE);
                }
                _ => {}
            }
            if v == VoiceFx::Alien {
                for s in [-1, 1] {
                    kit::circle(fb, x + s * 36, y - 30, 26, kit::BLACK);
                    kit::circle(fb, x + s * 36 + 8, y - 38, 7, kit::WHITE);
                }
            } else if v == VoiceFx::Monster {
                kit::circle(fb, x, y - 36, 30, kit::WHITE);
                kit::circle(fb, x, y - 32, 14, kit::BLACK);
            } else {
                for s in [-1, 1] {
                    kit::circle(fb, x + s * 34, y - 34, 16, kit::WHITE);
                    kit::circle(fb, x + s * 34, y - 32, 9, kit::BLACK);
                    kit::circle(fb, x + s * 34 + 3, y - 36, 3, kit::WHITE);
                }
            }
            let h = 10 + (open * 50.0) as i32;
            let w = if v == VoiceFx::Chipmunk { 40 } else { 84 };
            kit::round_rect(fb, x - w / 2, y + 22, w, h, (h / 2).max(1) as u32, kit::rgb(70, 20, 30));
            if v == VoiceFx::Monster {
                for k in 0..4 {
                    kit::triangle(fb, [(x - 40 + k * 22, y + 22), (x - 26 + k * 22, y + 22), (x - 33 + k * 22, y + 34)], kit::WHITE);
                }
            }
            if v == VoiceFx::Chipmunk {
                kit::rect(fb, x - 10, y + 22, 9, 14, kit::WHITE);
                kit::rect(fb, x + 1, y + 22, 9, 14, kit::WHITE);
            }
            if v == VoiceFx::Cave {
                // A miner's lamp on a helmet.
                kit::round_rect(fb, x - 80, y - 100, 160, 40, 20, kit::rgb(240, 200, 40));
                kit::circle(fb, x, y - 90, 14, kit::rgb(255, 250, 200));
            }
        }
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    let modbus: Arc<crate::modbus::ModBus> = ctx.get();
    let mixer: Arc<crate::mixer_bus::MixerBus> = ctx.get();
    let bus: Arc<AudioBus> = ctx.get();
    Box::new(MonsterMic::new(Sound::new(NAME, &modbus, &mixer, &bus), bus))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::kids_kit::tests::{energy, pitch_hz, render};

    /// A bus with a "Hardware input" carrying a 220 Hz tone.
    fn with_tone() -> (MonsterMic, Arc<Mutex<Vec<f32>>>) {
        let bus = Arc::new(AudioBus::new());
        let input = bus.register("Hardware input");
        *input.lock().unwrap() = (0..512).map(|i| (i as f32 * TAU * 220.0 / 48_000.0).sin() * 0.3).collect();
        let app = MonsterMic::new(Sound::detached(), bus);
        (app, input)
    }

    fn run(app: &mut MonsterMic, p: &mut Box<dyn AudioProcessor>, input: &Arc<Mutex<Vec<f32>>>, blocks: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let mut phase = 0usize;
        for _ in 0..blocks {
            *input.lock().unwrap() = (0..512).map(|i| ((phase + i) as f32 * TAU * 220.0 / 48_000.0).sin() * 0.3).collect();
            phase += 512;
            out.extend(render(p, 1));
            app.tick(&Input::default());
        }
        out
    }

    #[test]
    fn it_listens_to_the_hardware_input_and_the_chipmunk_is_higher() {
        let (mut app, input) = with_tone();
        assert_eq!(app.bus.source_name(app.source), "Hardware input");
        app.sound.set_reverb(0.0);
        let mut p = app.audio_processor().unwrap();
        let me = run(&mut app, &mut p, &input, 60);
        let left: Vec<f32> = me.iter().step_by(2).copied().collect();
        let hz = pitch_hz(&left[left.len() / 2..], 48_000.0);
        assert!((hz / 220.0 - 1.0).abs() < 0.03, "Me: {hz}");

        app.tick(&Input { grid: std::array::from_fn(|g| g == 2), ..Default::default() });
        app.tick(&Input::default());
        app.sound.set_reverb(0.0);
        let chip = run(&mut app, &mut p, &input, 80);
        let left: Vec<f32> = chip.iter().step_by(2).copied().collect();
        let hz = pitch_hz(&left[left.len() / 2..], 48_000.0);
        let want = 220.0 * 2f32.powf(9.0 / 12.0);
        assert!((hz / want - 1.0).abs() < 0.04, "Chipmunk: {hz}, want {want}");
    }

    #[test]
    fn every_voice_makes_sound_from_a_voice() {
        for v in 0..8 {
            let (mut app, input) = with_tone();
            let mut p = app.audio_processor().unwrap();
            app.tick(&Input { grid: std::array::from_fn(|g| g == v), ..Default::default() });
            let out = run(&mut app, &mut p, &input, 40);
            assert!(energy(&out) > 1e-4, "{:?}", VOICES[v]);
            assert!(out.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
        }
    }

    #[test]
    fn the_parrot_records_and_plays_back_with_the_mic_off() {
        let (mut app, input) = with_tone();
        app.sound.set_reverb(0.0);
        let mut p = app.audio_processor().unwrap();
        let hold = Input { grid: std::array::from_fn(|g| g == 8), ..Default::default() };
        for _ in 0..40 {
            app.tick(&hold);
            *input.lock().unwrap() = (0..512).map(|i| (i as f32 * TAU * 220.0 / 48_000.0).sin() * 0.3).collect();
            render(&mut p, 1);
        }
        app.tick(&Input::default());
        assert!(app.clip_seconds() > 0.3, "{}", app.clip_seconds());
        // Mic off, silence in: only the parrot can make sound.
        app.s.mic.set(0.0);
        input.lock().unwrap().fill(0.0);
        render(&mut p, 20);
        assert!(energy(&render(&mut p, 10)) < 1e-8);
        app.tick(&Input { grid: std::array::from_fn(|g| g == 11), ..Default::default() });
        assert!(energy(&render(&mut p, 10)) > 1e-4, "the parrot repeats it");
    }

    #[test]
    fn with_no_input_it_says_so_and_stays_quiet() {
        let mut app = MonsterMic::new(Sound::detached(), Arc::new(AudioBus::new()));
        assert_eq!(app.source, NO_SOURCE);
        let mut p = app.audio_processor().unwrap();
        assert!(energy(&render(&mut p, 10)) < 1e-9);
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
    }
}
