//! Tinkertone: a no-frills recreation of the feature set of Casio's 1981
//! MT-40 home keyboard, in an original look (not Casio's panel or name).
//!
//! What it reproduces, per the sources checked (Wikipedia's MT-40 article,
//! vintagetechnologyarchive.com, soundprogramming.net, a period MT-41
//! listing on Reverb):
//!
//! - 37-key melody keyboard, 8-note polyphony.
//! - 22 melody tones, held in 4 assignable tone presets.
//! - A separate 15-key bass keyboard with one fixed, monophonic bass
//!   timbre (so 8 + 1 = 9 voices total), played manually or as an
//!   auto-bass that follows the rhythm in Major / Minor / Minor 7th.
//! - 6 four-bar auto-rhythms on 5 analog-style drum sounds, tempo knob,
//!   synchro start (rhythm starts with the first bass key), and a Fill-in
//!   button that plays sixteenth-note pulses of snare or kick while held
//!   (the two kinds alternate press to press).
//! - Vibrato and Sustain switches, master volume and accompaniment volume.
//!
//! What is NOT verified, and is a documented best guess here:
//! - The 22 tone *names*: no source available listed them all. The ones
//!   sources do name (piano, organ, strings, electric piano, banjo) are
//!   included; the rest are period-typical Casiotone voice families.
//! - Exact key ranges (melody C3-C6, bass C2-D3), vibrato rate/depth,
//!   sustain length and the drum voices' exact identities.
//! - The rhythm names follow vintagetechnologyarchive.com (Rock, Samba,
//!   Swing, Slow Rock, Waltz, Pops); other sources don't list them.
//! - The tone generator itself: tones here are additive wavetables
//!   quantised to 8 bits to keep the early-digital grain. They are a
//!   model of the character, not a capture of the real chip.
//! - The auto-bass patterns are original. In particular the factory
//!   "Rock" bassline (the one behind Sleng Teng) is a composed work and is
//!   deliberately not transcribed here.
//!
//! Controls on Portamax: 37 + 15 keys don't fit on 16 pads, so F2 flips the
//! pads between KEYS (a 16-note window you slide across the 37 keys from
//! the menu) and BASS (the 15 bass keys, pad 16 = Fill-in). F3 starts and
//! stops the rhythm.

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16};
use crate::util::{accelerate, note_name, AtomicF32};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

const APP_NAME: &str = "Tinkertone";

// Own palette (house rule: every app picks its own colours).
const BG: Rgb565 = Rgb565::new(4, 7, 3);
const TITLE: Rgb565 = Rgb565::new(31, 52, 22);
const ACCENT: Rgb565 = Rgb565::new(31, 34, 6);
const DIM: Rgb565 = Rgb565::new(18, 32, 16);
const KEY_WHITE: Rgb565 = Rgb565::new(28, 56, 26);
const KEY_BLACK: Rgb565 = Rgb565::new(3, 6, 3);

/// Lowest melody key (MIDI note). 37 keys = three octaves, C3..C6.
pub const KEY_LOW: i32 = 48;
pub const KEYS: usize = 37;
/// Lowest bass key. 15 keys = C2..D3.
pub const BASS_LOW: i32 = 36;
pub const BASS_KEYS: usize = 15;
pub const VOICES: usize = 8;
const TABLE: usize = 2048;

// ------------------------------------------------------------------ tones

/// One melody tone: harmonic recipe + envelope.
#[derive(Clone, Copy)]
pub struct ToneSpec {
    pub name: &'static str,
    /// Relative amplitudes of harmonics 1..=12.
    harmonics: [f32; 12],
    /// Seconds.
    attack: f32,
    decay: f32,
    /// 0..1 level held while the key is down (0 = percussive).
    sustain: f32,
    release: f32,
    /// Octave shift from the key's written pitch.
    octave: i32,
}

const fn t(name: &'static str, harmonics: [f32; 12], attack: f32, decay: f32, sustain: f32, release: f32, octave: i32) -> ToneSpec {
    ToneSpec { name, harmonics, attack, decay, sustain, release, octave }
}

/// 22 tones. Recipes are period-typical early-digital voicings (see the
/// module doc for what is and isn't verified about the real list).
pub const TONES: [ToneSpec; 22] = [
    t("Piano", [1.0, 0.6, 0.35, 0.25, 0.15, 0.1, 0.06, 0.04, 0.0, 0.0, 0.0, 0.0], 0.003, 1.6, 0.0, 0.25, 0),
    t("Electric Piano", [1.0, 0.25, 0.05, 0.12, 0.0, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.004, 2.2, 0.0, 0.3, 0),
    t("Harpsichord", [0.6, 0.8, 0.7, 0.6, 0.5, 0.45, 0.4, 0.35, 0.3, 0.25, 0.2, 0.15], 0.002, 1.0, 0.0, 0.12, 0),
    t("Organ", [1.0, 0.8, 0.0, 0.6, 0.0, 0.3, 0.0, 0.4, 0.0, 0.0, 0.0, 0.0], 0.01, 0.0, 1.0, 0.04, 0),
    t("Pipe Organ", [1.0, 0.5, 0.4, 0.3, 0.2, 0.18, 0.1, 0.1, 0.05, 0.05, 0.0, 0.0], 0.06, 0.0, 1.0, 0.18, -1),
    t("Accordion", [0.7, 0.9, 0.6, 0.5, 0.45, 0.3, 0.25, 0.2, 0.1, 0.1, 0.05, 0.05], 0.04, 0.0, 1.0, 0.08, 0),
    t("Strings", [1.0, 0.5, 0.33, 0.25, 0.2, 0.16, 0.14, 0.12, 0.1, 0.08, 0.06, 0.05], 0.18, 0.0, 1.0, 0.4, 0),
    t("Violin", [1.0, 0.6, 0.45, 0.35, 0.3, 0.22, 0.2, 0.15, 0.12, 0.1, 0.08, 0.06], 0.09, 0.0, 1.0, 0.2, 0),
    t("Cello", [1.0, 0.7, 0.5, 0.4, 0.3, 0.2, 0.15, 0.1, 0.05, 0.0, 0.0, 0.0], 0.1, 0.0, 1.0, 0.25, -1),
    t("Flute", [1.0, 0.15, 0.05, 0.02, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.05, 0.0, 1.0, 0.08, 1),
    t("Piccolo", [1.0, 0.08, 0.03, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.03, 0.0, 1.0, 0.06, 2),
    t("Clarinet", [1.0, 0.0, 0.55, 0.0, 0.35, 0.0, 0.2, 0.0, 0.12, 0.0, 0.06, 0.0], 0.03, 0.0, 1.0, 0.07, 0),
    t("Oboe", [0.5, 0.9, 1.0, 0.6, 0.4, 0.3, 0.2, 0.12, 0.08, 0.05, 0.0, 0.0], 0.03, 0.0, 1.0, 0.07, 0),
    t("Saxophone", [1.0, 0.8, 0.7, 0.55, 0.45, 0.35, 0.28, 0.2, 0.14, 0.1, 0.06, 0.04], 0.04, 0.0, 1.0, 0.08, 0),
    t("Trumpet", [0.8, 1.0, 0.9, 0.75, 0.6, 0.48, 0.38, 0.3, 0.22, 0.16, 0.1, 0.06], 0.03, 0.0, 1.0, 0.06, 0),
    t("Horn", [1.0, 0.6, 0.35, 0.2, 0.1, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.07, 0.0, 1.0, 0.12, -1),
    t("Banjo", [0.7, 1.0, 0.8, 0.6, 0.55, 0.4, 0.35, 0.3, 0.2, 0.15, 0.1, 0.08], 0.002, 0.5, 0.0, 0.08, 0),
    t("Guitar", [1.0, 0.7, 0.45, 0.3, 0.2, 0.15, 0.1, 0.06, 0.03, 0.0, 0.0, 0.0], 0.003, 1.2, 0.0, 0.15, -1),
    t("Vibraphone", [1.0, 0.0, 0.0, 0.3, 0.0, 0.0, 0.0, 0.0, 0.0, 0.1, 0.0, 0.0], 0.003, 2.5, 0.0, 0.5, 0),
    t("Celesta", [1.0, 0.3, 0.0, 0.25, 0.0, 0.0, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0], 0.002, 1.2, 0.0, 0.3, 1),
    t("Glockenspiel", [1.0, 0.0, 0.4, 0.0, 0.0, 0.3, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2], 0.001, 1.4, 0.0, 0.4, 2),
    t("Synth Reed", [1.0, 0.5, 0.33, 0.25, 0.2, 0.17, 0.14, 0.12, 0.11, 0.1, 0.09, 0.08], 0.01, 0.3, 0.7, 0.1, 0),
];

/// Default contents of the 4 tone presets.
const DEFAULT_PRESETS: [usize; 4] = [0, 3, 6, 9];

/// Builds one wavetable, normalised and quantised to 8 bits.
fn build_table(h: &[f32; 12]) -> Vec<f32> {
    let mut tab = vec![0.0f32; TABLE];
    for (i, s) in tab.iter_mut().enumerate() {
        let ph = i as f32 / TABLE as f32 * TAU;
        *s = h.iter().enumerate().map(|(k, a)| a * ((k + 1) as f32 * ph).sin()).sum();
    }
    let peak = tab.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    for s in tab.iter_mut() {
        *s = (*s / peak * 127.0).round() / 127.0;
    }
    tab
}

/// The fixed bass timbre: a rounded, slightly hollow square.
const BASS_HARMONICS: [f32; 12] = [1.0, 0.25, 0.33, 0.1, 0.18, 0.05, 0.1, 0.0, 0.05, 0.0, 0.0, 0.0];

// ---------------------------------------------------------------- rhythms

pub const RHYTHM_NAMES: [&str; 6] = ["Rock", "Samba", "Swing", "Slow Rock", "Waltz", "Pops"];
pub const CHORD_NAMES: [&str; 3] = ["Major", "Minor", "Minor 7th"];
const BARS: usize = 4;

/// One rhythm: drum grids and an auto-bass line for one bar, repeated for
/// four bars with the 4th bar's own variation.
struct Rhythm {
    steps: usize,          // steps per bar
    steps_per_beat: usize, // 4 = sixteenths, 3 = triplets
    /// One string per drum, `x` = hit, `.` = rest; bar 1-3 and bar 4.
    drums: [&'static str; 5],
    drums_bar4: [&'static str; 5],
    /// Auto-bass: one char per step. `0` root, `3` third, `5` fifth,
    /// `8` top (octave or 7th), `L` fifth below, `.` rest/hold.
    bass: &'static str,
    bass_bar4: &'static str,
}

const RHYTHMS: [Rhythm; 6] = [
    // Rock
    Rhythm {
        steps: 16,
        steps_per_beat: 4,
        drums: ["x.......x.x.....", "....x.......x...", "x.x.x.x.x.x.x...", "..............x.", "................"],
        drums_bar4: ["x.......x.x.....", "....x.......x.xx", "x.x.x.x.x.x.....", "................", "................"],
        bass: "0...0.5.0...5.8.",
        bass_bar4: "0...0.5.8.5.3.L.",
    },
    // Samba
    Rhythm {
        steps: 16,
        steps_per_beat: 4,
        drums: ["x..xx..xx..xx..x", "..x..x....x..x..", "xxxxxxxxxxxxxxxx", "................", "................"],
        drums_bar4: ["x..xx..xx..xx..x", "..x..x....x.xxxx", "xxxxxxxxxxxx....", "................", "................"],
        bass: "0..5L..00..5L..0",
        bass_bar4: "0..5L..00..58.5.",
    },
    // Swing (triplet grid: 3 steps per beat, 12 per bar)
    Rhythm {
        steps: 12,
        steps_per_beat: 3,
        drums: ["x.....x.....", "...x.....x..", "x.xx.xx.xx.x", "............", "x..........."],
        drums_bar4: ["x.....x.....", "...x.....xxx", "x.xx.xx.....", "............", "x..........."],
        bass: "0..3..5..8..",
        bass_bar4: "0..3..5..L..",
    },
    // Slow Rock (12/8)
    Rhythm {
        steps: 12,
        steps_per_beat: 3,
        drums: ["x.....x.x...", "...x.....x..", "xxxxxxxxxxxx", "............", "............"],
        drums_bar4: ["x.....x.x...", "...x.....xxx", "xxxxxxxxx...", "............", "............"],
        bass: "0.35.80.35.8",
        bass_bar4: "0.35.85.3.L.",
    },
    // Waltz (3/4 in sixteenths)
    Rhythm {
        steps: 12,
        steps_per_beat: 4,
        drums: ["x...........", "....x...x...", "x...x...x...", "............", "............"],
        drums_bar4: ["x...........", "....x...x.xx", "x...x...x...", "............", "x..........."],
        bass: "0...........",
        bass_bar4: "0.......L...",
    },
    // Pops
    Rhythm {
        steps: 16,
        steps_per_beat: 4,
        drums: ["x.....x.x.......", "....x.......x...", "xxxxxxxxxxxxxxxx", "................", "................"],
        drums_bar4: ["x.....x.x.......", "....x.......xxxx", "xxxxxxxxxxxx....", "................", "x..............."],
        bass: "0.0.0.0.5.5.5.5.",
        bass_bar4: "0.0.0.0.5.5.8.8.",
    },
];

/// Semitone offset of a bass-pattern symbol for a chord type.
fn bass_offset(sym: u8, chord: usize) -> Option<i32> {
    let third = if chord == 0 { 4 } else { 3 };
    let top = if chord == 2 { 10 } else { 12 };
    match sym {
        b'0' => Some(0),
        b'3' => Some(third),
        b'5' => Some(7),
        b'8' => Some(top),
        b'L' => Some(-5),
        _ => None,
    }
}

// ------------------------------------------------------------- shared state

struct Shared {
    /// Melody keys currently held (by pads), indexed 0..37.
    keys: [AtomicBool; KEYS],
    /// Bass key held (0..15) or -1.
    bass_key: AtomicI32,
    preset: AtomicUsize,
    preset_tones: [AtomicUsize; 4],
    vibrato: AtomicBool,
    sustain: AtomicBool,
    rhythm: AtomicUsize,
    tempo: AtomicF32,
    playing: AtomicBool,
    synchro: AtomicBool,
    fill_held: AtomicBool,
    fill_kind: AtomicUsize,
    bass_auto: AtomicBool,
    chord: AtomicUsize,
    volume: AtomicF32,
    accomp: AtomicF32,
    ext_volume: Arc<AtomicF32>,
    ext_accomp: Arc<AtomicF32>,
    ext_tempo: Arc<AtomicF32>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    // telemetry for the screens
    step: AtomicUsize,
    bar: AtomicUsize,
    bass_sounding: AtomicI32,
    drum_flash: [AtomicF32; 5],
    active_voices: AtomicUsize,
    peak: AtomicF32,
    sample_rate: AtomicU32,
}

impl Shared {
    fn new(modbus: &ModBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register(APP_NAME, modbus);
        Self {
            keys: std::array::from_fn(|_| AtomicBool::new(false)),
            bass_key: AtomicI32::new(-1),
            preset: AtomicUsize::new(0),
            preset_tones: std::array::from_fn(|i| AtomicUsize::new(DEFAULT_PRESETS[i])),
            vibrato: AtomicBool::new(false),
            sustain: AtomicBool::new(false),
            rhythm: AtomicUsize::new(0),
            tempo: AtomicF32::new(120.0),
            playing: AtomicBool::new(false),
            synchro: AtomicBool::new(false),
            fill_held: AtomicBool::new(false),
            fill_kind: AtomicUsize::new(1),
            bass_auto: AtomicBool::new(false),
            chord: AtomicUsize::new(0),
            volume: AtomicF32::new(0.8),
            accomp: AtomicF32::new(0.7),
            ext_volume: modbus.register(format!("{APP_NAME}: Volume")),
            ext_accomp: modbus.register(format!("{APP_NAME}: Accomp Volume")),
            ext_tempo: modbus.register(format!("{APP_NAME}: Tempo")),
            mix_level,
            ext_mix_level,
            step: AtomicUsize::new(0),
            bar: AtomicUsize::new(0),
            bass_sounding: AtomicI32::new(-1),
            drum_flash: std::array::from_fn(|_| AtomicF32::new(0.0)),
            active_voices: AtomicUsize::new(0),
            peak: AtomicF32::new(0.0),
            sample_rate: AtomicU32::new(48_000),
        }
    }

    fn tone(&self) -> usize {
        self.preset_tones[self.preset.load(Ordering::Relaxed).min(3)].load(Ordering::Relaxed).min(TONES.len() - 1)
    }

    /// Bass key press: synchro start, then hold.
    fn press_bass(&self, k: usize) {
        if self.synchro.load(Ordering::Relaxed) && !self.playing.load(Ordering::Relaxed) {
            self.synchro.store(false, Ordering::Relaxed);
            self.step.store(0, Ordering::Relaxed);
            self.bar.store(0, Ordering::Relaxed);
            self.playing.store(true, Ordering::Relaxed);
        }
        self.bass_key.store(k as i32, Ordering::Relaxed);
    }
}

// --------------------------------------------------------------- the DSP

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Off,
    Attack,
    Decay,
    Release,
}

#[derive(Clone, Copy)]
struct Voice {
    key: i32,
    tone: usize,
    phase: f32,
    freq: f32,
    env: f32,
    stage: Stage,
    age: u64,
    held_s: f32,
}

impl Voice {
    const OFF: Voice = Voice { key: -1, tone: 0, phase: 0.0, freq: 0.0, env: 0.0, stage: Stage::Off, age: 0, held_s: 0.0 };
}

fn midi_hz(n: f32) -> f32 {
    440.0 * 2f32.powf((n - 69.0) / 12.0)
}

/// Analog-style drum voice (simple, deliberately lo-fi).
#[derive(Clone, Copy, Default)]
struct Drum {
    t: f32,
    on: bool,
    amp: f32,
}

struct Processor {
    shared: Arc<Shared>,
    tables: Arc<Vec<Vec<f32>>>,
    bass_table: Arc<Vec<f32>>,
    voices: [Voice; VOICES],
    prev_keys: [bool; KEYS],
    clock: u64,
    vib_phase: f32,
    // bass
    bass_phase: f32,
    bass_freq: f32,
    bass_env: f32,
    bass_gate: bool,
    prev_bass_key: i32,
    auto_root: i32,
    // sequencer
    step_pos: f64,
    seq_step: usize,
    seq_bar: usize,
    was_playing: bool,
    drums: [Drum; 5],
    noise: u32,
    hp: f32,
    hp_prev: f32,
}

impl Processor {
    fn noise(&mut self) -> f32 {
        // xorshift: cheap white noise
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        (self.noise as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn note_on(&mut self, key: i32, tone: usize) {
        let spec = &TONES[tone];
        let idx = self
            .voices
            .iter()
            .position(|v| v.stage == Stage::Off)
            .or_else(|| self.voices.iter().position(|v| v.stage == Stage::Release))
            .unwrap_or_else(|| (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap_or(0));
        self.clock += 1;
        self.voices[idx] = Voice {
            key,
            tone,
            phase: 0.0,
            freq: midi_hz((KEY_LOW + key + spec.octave * 12) as f32),
            env: self.voices[idx].env * 0.3,
            stage: Stage::Attack,
            age: self.clock,
            held_s: 0.0,
        };
    }

    fn trigger(&mut self, d: usize, amp: f32) {
        self.drums[d] = Drum { t: 0.0, on: true, amp };
        if d == 2 {
            self.drums[3].on = false; // closed hat chokes open hat
        }
        self.shared.drum_flash[d].set(1.0);
    }

    fn drum_sample(&mut self, sr: f32) -> f32 {
        let dt = 1.0 / sr;
        let mut out = 0.0;
        for d in 0..5 {
            if !self.drums[d].on {
                continue;
            }
            let t = self.drums[d].t;
            let a = self.drums[d].amp;
            let n = self.noise();
            let s = match d {
                0 => {
                    let f = 50.0 + 90.0 * (-t * 30.0).exp();
                    (TAU * f * t).sin() * (-t * 9.0).exp() * 1.1
                }
                1 => (n * 0.8 + (TAU * 190.0 * t).sin() * 0.4) * (-t * 18.0).exp() * 0.7,
                2 => n * (-t * 60.0).exp() * 0.35,
                3 => n * (-t * 7.0).exp() * 0.3,
                _ => n * (-t * 2.5).exp() * 0.25,
            };
            out += s * a;
            self.drums[d].t += dt;
            if t > 2.0 {
                self.drums[d].on = false;
            }
        }
        // the hats/cymbal want highs, the kick doesn't: a gentle one-pole
        // high-pass on everything but the kick would cost another split, so
        // keep one mild HP on the drum bus to thin the noise.
        let hp = out - self.hp_prev + 0.995 * self.hp;
        self.hp_prev = out;
        self.hp = hp;
        hp
    }

    fn sequencer_tick(&mut self) {
        let s = self.shared.clone();
        let r = &RHYTHMS[s.rhythm.load(Ordering::Relaxed).min(5)];
        let step = self.seq_step % r.steps;
        let bar4 = self.seq_bar % BARS == BARS - 1;
        let fill = s.fill_held.load(Ordering::Relaxed);
        if fill {
            // a pulse every step (sixteenths; triplets on the 12/8 grids)
            // of snare (kind 0) or kick (kind 1)
            let d = if s.fill_kind.load(Ordering::Relaxed) == 0 { 1 } else { 0 };
            self.trigger(d, 0.9);
        } else {
            let grid = if bar4 { &r.drums_bar4 } else { &r.drums };
            for d in 0..5 {
                if grid[d].as_bytes().get(step) == Some(&b'x') {
                    let accent = if step % r.steps_per_beat == 0 { 1.0 } else { 0.75 };
                    self.trigger(d, accent);
                }
            }
        }
        // auto bass (only while the rhythm runs, in Auto mode, with a root)
        if s.bass_auto.load(Ordering::Relaxed) && self.auto_root >= 0 {
            let line = if bar4 { r.bass_bar4 } else { r.bass };
            if let Some(off) = line.as_bytes().get(step).and_then(|c| bass_offset(*c, s.chord.load(Ordering::Relaxed))) {
                self.bass_note((BASS_LOW + self.auto_root + off) as f32, true);
            }
        }
        s.step.store(step, Ordering::Relaxed);
        s.bar.store(self.seq_bar % BARS, Ordering::Relaxed);
        self.seq_step += 1;
        if self.seq_step >= r.steps {
            self.seq_step = 0;
            self.seq_bar = (self.seq_bar + 1) % BARS;
        }
    }

    fn bass_note(&mut self, note: f32, retrigger: bool) {
        self.bass_freq = midi_hz(note);
        if retrigger {
            self.bass_env = 1.0;
        }
        self.bass_gate = true;
        self.shared.bass_sounding.store(note as i32 - BASS_LOW, Ordering::Relaxed);
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sr: f32) {
        if channels == 0 || sr <= 0.0 {
            return;
        }
        let s = Arc::clone(&self.shared);
        s.sample_rate.store(sr as u32, Ordering::Relaxed);
        let tone = s.tone();

        // ---- melody keys: diff against last block
        for k in 0..KEYS {
            let down = s.keys[k].load(Ordering::Relaxed);
            if down && !self.prev_keys[k] {
                self.note_on(k as i32, tone);
            } else if !down && self.prev_keys[k] {
                for v in self.voices.iter_mut().filter(|v| v.key == k as i32 && v.stage != Stage::Off && v.stage != Stage::Release) {
                    v.stage = Stage::Release;
                }
            }
            self.prev_keys[k] = down;
        }

        // ---- bass key (manual, or root for auto)
        let bk = s.bass_key.load(Ordering::Relaxed);
        let playing = s.playing.load(Ordering::Relaxed);
        let auto = s.bass_auto.load(Ordering::Relaxed) && playing;
        if bk != self.prev_bass_key {
            if bk >= 0 {
                self.auto_root = bk;
                if !auto {
                    self.bass_note((BASS_LOW + bk) as f32, true);
                }
            } else if !auto {
                self.bass_gate = false;
            }
            self.prev_bass_key = bk;
        }
        if !playing && self.was_playing {
            self.bass_gate = false;
            self.auto_root = if bk >= 0 { bk } else { -1 };
        }
        if playing && !self.was_playing {
            self.seq_step = s.step.load(Ordering::Relaxed);
            self.seq_bar = s.bar.load(Ordering::Relaxed);
            self.step_pos = 1.0; // fire the first step right away
        }
        self.was_playing = playing;

        let r = &RHYTHMS[s.rhythm.load(Ordering::Relaxed).min(5)];
        let bpm = (s.tempo.get() + s.ext_tempo.get() * 100.0).clamp(40.0, 240.0);
        let steps_per_sec = bpm / 60.0 * r.steps_per_beat as f32;
        let step_inc = steps_per_sec as f64 / sr as f64;

        let vol = (s.volume.get() + s.ext_volume.get()).clamp(0.0, 1.0);
        let acc = (s.accomp.get() + s.ext_accomp.get()).clamp(0.0, 1.0);
        let mix = (s.mix_level.get() + s.ext_mix_level.get()).clamp(0.0, 2.0);
        let vibrato = s.vibrato.load(Ordering::Relaxed);
        let sustain = s.sustain.load(Ordering::Relaxed);
        let dt = 1.0 / sr;
        let mut peak: f32 = 0.0;

        for frame in buffer.chunks_mut(channels) {
            if playing {
                self.step_pos += step_inc;
                if self.step_pos >= 1.0 {
                    self.step_pos -= 1.0;
                    self.sequencer_tick();
                }
            }
            // vibrato: ~6 Hz, ±25 cents, fading in over the first 0.3 s held
            self.vib_phase = (self.vib_phase + 6.0 * dt) % 1.0;
            let vib_lfo = (self.vib_phase * TAU).sin();

            let mut mel = 0.0;
            for v in self.voices.iter_mut() {
                if v.stage == Stage::Off {
                    continue;
                }
                let spec = &TONES[v.tone];
                let release = if sustain { (spec.release + 1.6).max(1.2) } else { spec.release };
                match v.stage {
                    Stage::Attack => {
                        v.env += dt / spec.attack.max(0.001);
                        if v.env >= 1.0 {
                            v.env = 1.0;
                            v.stage = Stage::Decay;
                        }
                    }
                    Stage::Decay => {
                        if spec.sustain < 1.0 {
                            let target = spec.sustain;
                            let rate = dt / spec.decay.max(0.01) * 4.0;
                            v.env += (target - v.env) * rate.min(1.0);
                            if spec.sustain == 0.0 && v.env < 0.0005 {
                                v.stage = Stage::Off;
                            }
                        }
                    }
                    Stage::Release => {
                        v.env -= v.env * (dt / release.max(0.005) * 4.0).min(1.0);
                        if v.env < 0.0005 {
                            v.stage = Stage::Off;
                        }
                    }
                    Stage::Off => {}
                }
                v.held_s += dt;
                let depth = if vibrato { 0.0145 * (v.held_s / 0.3).min(1.0) } else { 0.0 };
                let f = v.freq * (1.0 + depth * vib_lfo);
                v.phase = (v.phase + f * dt) % 1.0;
                let tab = &self.tables[v.tone];
                mel += tab[(v.phase * TABLE as f32) as usize % TABLE] * v.env;
            }

            // bass: gated with a little decay so auto lines stay punchy
            let bass = if self.bass_gate || self.bass_env > 0.0005 {
                let target = if self.bass_gate { 0.55 } else { 0.0 };
                let rate = if self.bass_gate { 3.0 } else { 25.0 };
                self.bass_env += (target - self.bass_env) * (rate * dt).min(1.0);
                self.bass_phase = (self.bass_phase + self.bass_freq * dt) % 1.0;
                self.bass_table[(self.bass_phase * TABLE as f32) as usize % TABLE] * self.bass_env
            } else {
                0.0
            };

            let drums = self.drum_sample(sr);
            // Gains leave headroom for all 8 voices on a sustained tone over
            // the full accompaniment; tanh rounds off anything left rather
            // than hard-clipping (a small speaker amp of the era would have
            // compressed similarly, not squared off).
            let out = ((mel * 0.12 * vol) + (bass * 0.42 + drums * 0.45) * acc * vol) * mix;
            let out = out.tanh();
            peak = peak.max(out.abs());
            for c in frame.iter_mut() {
                *c += out;
            }
        }
        s.active_voices.store(self.voices.iter().filter(|v| v.stage != Stage::Off).count(), Ordering::Relaxed);
        s.peak.set(s.peak.get() * 0.85 + peak * 0.15);
        for f in &s.drum_flash {
            f.set(f.get() * 0.8);
        }
        if !self.bass_gate && self.bass_env < 0.01 {
            s.bass_sounding.store(-1, Ordering::Relaxed);
        }
    }
}

// --------------------------------------------------------------- the menu

#[derive(Clone, Copy, PartialEq, Debug)]
enum Sel {
    Preset,
    PresetTone,
    Vibrato,
    Sustain,
    Rhythm,
    Tempo,
    StartStop,
    Synchro,
    FillIn,
    BassMode,
    Chord,
    Volume,
    Accomp,
    PadLayer,
    KeyWindow,
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Group(usize),
    Leaf(Sel),
}

const GROUPS: [&str; 5] = ["Tone", "Rhythm", "Bass", "Volume", "Pads"];

fn group_leaves(g: usize) -> &'static [Sel] {
    match g {
        0 => &[Sel::Preset, Sel::PresetTone, Sel::Vibrato, Sel::Sustain],
        1 => &[Sel::Rhythm, Sel::Tempo, Sel::StartStop, Sel::Synchro, Sel::FillIn],
        2 => &[Sel::BassMode, Sel::Chord],
        3 => &[Sel::Volume, Sel::Accomp],
        _ => &[Sel::PadLayer, Sel::KeyWindow],
    }
}

pub struct TinkertoneApp {
    shared: Arc<Shared>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; 5],
    bass_layer: bool,
    /// First melody key (0..=21) covered by the 16 pads in KEYS layer.
    window: usize,
    prev_grid: [bool; 16],
    /// Melody keys this app's pads are holding, to release on layer change.
    pad_keys: [Option<usize>; 16],
    /// Frames left on a menu-triggered fill.
    menu_fill_frames: u32,
}

impl TinkertoneApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, _audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self {
            shared: Arc::new(Shared::new(&modbus, &mixer_bus)),
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [true, true, false, false, false],
            bass_layer: false,
            window: 12,
            prev_grid: [false; 16],
            pad_keys: [None; 16],
            menu_fill_frames: 0,
        }
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for g in 0..GROUPS.len() {
            rows.push(Row::Group(g));
            if self.expanded[g] {
                rows.extend(group_leaves(g).iter().map(|s| Row::Leaf(*s)));
            }
        }
        rows
    }

    fn leaf_name(sel: Sel) -> &'static str {
        match sel {
            Sel::Preset => "Tone Preset",
            Sel::PresetTone => "Preset Holds",
            Sel::Vibrato => "Vibrato",
            Sel::Sustain => "Sustain",
            Sel::Rhythm => "Rhythm",
            Sel::Tempo => "Tempo",
            Sel::StartStop => ">> Start / Stop",
            Sel::Synchro => "Synchro Start",
            Sel::FillIn => ">> Fill-in",
            Sel::BassMode => "Bass",
            Sel::Chord => "Auto Chord",
            Sel::Volume => "Volume",
            Sel::Accomp => "Accomp Volume",
            Sel::PadLayer => "Pads",
            Sel::KeyWindow => "Key Window",
        }
    }

    fn leaf_value(&self, sel: Sel) -> String {
        let s = &self.shared;
        let on = |b: bool| if b { "ON" } else { "off" }.to_string();
        match sel {
            Sel::Preset => format!("{}", s.preset.load(Ordering::Relaxed) + 1),
            Sel::PresetTone => TONES[s.tone()].name.into(),
            Sel::Vibrato => on(s.vibrato.load(Ordering::Relaxed)),
            Sel::Sustain => on(s.sustain.load(Ordering::Relaxed)),
            Sel::Rhythm => RHYTHM_NAMES[s.rhythm.load(Ordering::Relaxed)].into(),
            Sel::Tempo => format!("{:.0} BPM", s.tempo.get()),
            Sel::StartStop => if s.playing.load(Ordering::Relaxed) { "running" } else { "stopped" }.into(),
            Sel::Synchro => if s.synchro.load(Ordering::Relaxed) { "ARMED" } else { "off" }.into(),
            Sel::FillIn => (if s.fill_kind.load(Ordering::Relaxed) == 0 { "next: kick" } else { "next: snare" }).into(),
            Sel::BassMode => if s.bass_auto.load(Ordering::Relaxed) { "Auto" } else { "Manual" }.into(),
            Sel::Chord => CHORD_NAMES[s.chord.load(Ordering::Relaxed)].into(),
            Sel::Volume => format!("{:.0}%", s.volume.get() * 100.0),
            Sel::Accomp => format!("{:.0}%", s.accomp.get() * 100.0),
            Sel::PadLayer => if self.bass_layer { "Bass keys" } else { "Melody keys" }.into(),
            Sel::KeyWindow => {
                format!("{}-{}", note_name(KEY_LOW + self.window as i32), note_name(KEY_LOW + self.window as i32 + 15))
            }
        }
    }

    fn edit(&mut self, sel: Sel, delta: i32) {
        if delta == 0 {
            return;
        }
        let s = &self.shared;
        let step = delta.signum();
        let cycle = |a: &AtomicUsize, n: usize| a.store((a.load(Ordering::Relaxed) as i32 + step).rem_euclid(n as i32) as usize, Ordering::Relaxed);
        let nudge = |a: &AtomicF32, amt: f32, lo: f32, hi: f32| a.set((a.get() + amt).clamp(lo, hi));
        let sens = self.sensitivity.get().max(0.01);
        match sel {
            Sel::Preset => cycle(&s.preset, 4),
            Sel::PresetTone => cycle(&s.preset_tones[s.preset.load(Ordering::Relaxed)], TONES.len()),
            Sel::Vibrato => s.vibrato.store(delta > 0, Ordering::Relaxed),
            Sel::Sustain => s.sustain.store(delta > 0, Ordering::Relaxed),
            Sel::Rhythm => cycle(&s.rhythm, RHYTHMS.len()),
            Sel::Tempo => nudge(&s.tempo, accelerate(delta) * 10.0 * sens, 40.0, 240.0),
            Sel::Synchro => s.synchro.store(delta > 0, Ordering::Relaxed),
            Sel::BassMode => s.bass_auto.store(delta > 0, Ordering::Relaxed),
            Sel::Chord => cycle(&s.chord, 3),
            Sel::Volume => nudge(&s.volume, accelerate(delta) * 0.1 * sens, 0.0, 1.0),
            Sel::Accomp => nudge(&s.accomp, accelerate(delta) * 0.1 * sens, 0.0, 1.0),
            Sel::PadLayer => self.set_layer(delta > 0),
            Sel::KeyWindow => {
                self.release_pads();
                self.window = (self.window as i32 + step).clamp(0, (KEYS - 16) as i32) as usize;
            }
            Sel::StartStop | Sel::FillIn => {}
        }
    }

    fn press(&mut self, sel: Sel) {
        match sel {
            Sel::StartStop => self.toggle_running(),
            Sel::FillIn => {
                // a short one-beat fill from the menu (pads hold it as long as wanted)
                let s = &self.shared;
                s.fill_kind.store(1 - s.fill_kind.load(Ordering::Relaxed), Ordering::Relaxed);
                s.fill_held.store(true, Ordering::Relaxed);
                self.menu_fill_frames = 15;
            }
            Sel::Vibrato => {
                self.shared.vibrato.fetch_xor(true, Ordering::Relaxed);
            }
            Sel::Sustain => {
                self.shared.sustain.fetch_xor(true, Ordering::Relaxed);
            }
            Sel::Synchro => {
                self.shared.synchro.fetch_xor(true, Ordering::Relaxed);
            }
            Sel::BassMode => {
                self.shared.bass_auto.fetch_xor(true, Ordering::Relaxed);
            }
            Sel::PadLayer => self.set_layer(!self.bass_layer),
            _ => {}
        }
    }

    fn release_pads(&mut self) {
        for k in self.pad_keys.iter_mut() {
            if let Some(key) = k.take() {
                self.shared.keys[key].store(false, Ordering::Relaxed);
            }
        }
        self.shared.bass_key.store(-1, Ordering::Relaxed);
        self.shared.fill_held.store(false, Ordering::Relaxed);
    }

    fn set_layer(&mut self, bass: bool) {
        if bass != self.bass_layer {
            self.release_pads();
            self.bass_layer = bass;
        }
    }

    fn handle_pads(&mut self, grid: &[bool; 16]) {
        let s = Arc::clone(&self.shared);
        for i in 0..16 {
            let (now, was) = (grid[i], self.prev_grid[i]);
            if now == was {
                continue;
            }
            if self.bass_layer {
                if i == 15 {
                    if now {
                        s.fill_kind.store(1 - s.fill_kind.load(Ordering::Relaxed), Ordering::Relaxed);
                    }
                    s.fill_held.store(now, Ordering::Relaxed);
                } else if now {
                    s.press_bass(i);
                } else if s.bass_key.load(Ordering::Relaxed) == i as i32 {
                    // fall back to another still-held bass pad, if any
                    let other = (0..15).rev().find(|&j| j != i && grid[j]);
                    s.bass_key.store(other.map_or(-1, |j| j as i32), Ordering::Relaxed);
                }
            } else {
                let key = self.window + i;
                if now {
                    self.pad_keys[i] = Some(key);
                    s.keys[key].store(true, Ordering::Relaxed);
                } else if let Some(k) = self.pad_keys[i].take() {
                    s.keys[k].store(false, Ordering::Relaxed);
                }
            }
        }
        self.prev_grid = *grid;
    }

    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .into_iter()
            .map(|r| match r {
                Row::Group(g) => (format!("{} {}", if self.expanded[g] { "v" } else { ">" }, GROUPS[g]), String::new(), true),
                Row::Leaf(s) => (Self::leaf_name(s).into(), self.leaf_value(s), false),
            })
            .collect()
    }

    /// Everything the Slint panel shows.
    #[allow(dead_code)] // Slint GUI only
    fn panel_extra(&self) -> crate::app::TinkertoneExtra {
        let s = &self.shared;
        let playing = s.playing.load(Ordering::Relaxed);
        let r = &RHYTHMS[s.rhythm.load(Ordering::Relaxed)];
        let preset = s.preset.load(Ordering::Relaxed);
        crate::app::TinkertoneExtra {
            keys_held: (0..KEYS).map(|k| s.keys[k].load(Ordering::Relaxed)).collect(),
            window: self.window,
            bass_layer: self.bass_layer,
            bass_held: s.bass_key.load(Ordering::Relaxed),
            bass_sounding: s.bass_sounding.load(Ordering::Relaxed),
            preset,
            preset_tones: (0..4).map(|i| TONES[s.preset_tones[i].load(Ordering::Relaxed)].name.to_string()).collect(),
            vibrato: s.vibrato.load(Ordering::Relaxed),
            sustain: s.sustain.load(Ordering::Relaxed),
            rhythm: s.rhythm.load(Ordering::Relaxed),
            tempo: s.tempo.get(),
            playing,
            synchro: s.synchro.load(Ordering::Relaxed),
            fill: s.fill_held.load(Ordering::Relaxed),
            bass_auto: s.bass_auto.load(Ordering::Relaxed),
            chord: CHORD_NAMES[s.chord.load(Ordering::Relaxed)].into(),
            step: s.step.load(Ordering::Relaxed),
            steps: r.steps,
            steps_per_beat: r.steps_per_beat,
            bar: s.bar.load(Ordering::Relaxed),
            drum_flash: (0..5).map(|d| s.drum_flash[d].get().clamp(0.0, 1.0)).collect(),
            volume: s.volume.get(),
            accomp: s.accomp.get(),
            peak: s.peak.get().clamp(0.0, 1.0),
        }
    }
}

impl App for TinkertoneApp {
    fn tick(&mut self, input: &Input) {
        if self.menu_fill_frames > 0 {
            self.menu_fill_frames -= 1;
            if self.menu_fill_frames == 0 && !(self.bass_layer && input.grid[15]) {
                self.shared.fill_held.store(false, Ordering::Relaxed);
            }
        }
        self.handle_pads(&input.grid);
        let rows = self.visible_rows();
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();
        if input.knob1_press {
            if let Some(Row::Group(g)) = current {
                self.expanded[g] = !self.expanded[g];
            }
        }
        if let Some(Row::Leaf(sel)) = current {
            self.edit(sel, input.knob2);
            if input.knob2_press {
                self.press(sel);
            }
        }
    }

    fn on_exit(&mut self) {
        self.release_pads();
        self.prev_grid = [false; 16];
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        fb.clear(BG).ok();
        Text::new(APP_NAME, Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, TITLE)).draw(fb).ok();
        let accent = MonoTextStyle::new(&SPLEEN_8X16, ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        let s = &self.shared;
        let head = format!("{}  {:.0} BPM", RHYTHM_NAMES[s.rhythm.load(Ordering::Relaxed)], s.tempo.get());
        Text::new(&head, Point::new(200, 27), accent).draw(fb).ok();

        let rows: Vec<(String, String)> = self
            .display_rows()
            .into_iter()
            .map(|(n, v, g)| (if g { n } else { format!("    {n}") }, v))
            .collect();
        self.list.draw(fb, 16, 68, 24, 10, &rows);

        // right: a 37-key strip (window outlined) and the 15 bass keys
        let (x0, y0) = (360, 60);
        let white_w = 12;
        let mut wx = x0;
        let mut white_pos = [0i32; KEYS];
        for k in 0..KEYS {
            let pc = (KEY_LOW + k as i32) % 12;
            if ![1, 3, 6, 8, 10].contains(&pc) {
                white_pos[k] = wx;
                let lit = s.keys[k].load(Ordering::Relaxed);
                Rectangle::new(Point::new(wx, y0), Size::new(white_w as u32 - 1, 60))
                    .into_styled(PrimitiveStyle::with_fill(if lit { ACCENT } else { KEY_WHITE }))
                    .draw(fb)
                    .ok();
                wx += white_w;
            } else {
                white_pos[k] = wx - 4;
            }
        }
        for k in 0..KEYS {
            let pc = (KEY_LOW + k as i32) % 12;
            if [1, 3, 6, 8, 10].contains(&pc) {
                let lit = s.keys[k].load(Ordering::Relaxed);
                Rectangle::new(Point::new(white_pos[k], y0), Size::new(8, 36))
                    .into_styled(PrimitiveStyle::with_fill(if lit { ACCENT } else { KEY_BLACK }))
                    .draw(fb)
                    .ok();
            }
        }
        if !self.bass_layer {
            let a = white_pos[self.window];
            let b = white_pos[self.window + 15] + white_w;
            Rectangle::new(Point::new(a, y0 + 63), Size::new((b - a).max(1) as u32, 3))
                .into_styled(PrimitiveStyle::with_fill(ACCENT))
                .draw(fb)
                .ok();
        }
        let by = y0 + 80;
        Text::new("BASS", Point::new(x0, by - 4), dim).draw(fb).ok();
        let held = s.bass_key.load(Ordering::Relaxed);
        let sounding = s.bass_sounding.load(Ordering::Relaxed);
        for k in 0..BASS_KEYS {
            let c = if held == k as i32 {
                ACCENT
            } else if sounding == k as i32 {
                DIM
            } else {
                Rgb565::new(8, 14, 7)
            };
            Rectangle::new(Point::new(x0 + k as i32 * 17, by + 2), Size::new(14, 14)).into_styled(PrimitiveStyle::with_fill(c)).draw(fb).ok();
        }
        // beat lamps
        let r = &RHYTHMS[s.rhythm.load(Ordering::Relaxed)];
        let beats = r.steps / r.steps_per_beat;
        let beat = s.step.load(Ordering::Relaxed) / r.steps_per_beat;
        let playing = s.playing.load(Ordering::Relaxed);
        for b in 0..beats {
            let on = playing && b == beat;
            Rectangle::new(Point::new(x0 + b as i32 * 22, by + 34), Size::new(16, 8))
                .into_styled(PrimitiveStyle::with_fill(if on { ACCENT } else { Rgb565::new(8, 14, 7) }))
                .draw(fb)
                .ok();
        }
        let info = format!(
            "{}  {}  {}{}",
            TONES[s.tone()].name,
            if s.bass_auto.load(Ordering::Relaxed) { CHORD_NAMES[s.chord.load(Ordering::Relaxed)] } else { "manual bass" },
            if s.vibrato.load(Ordering::Relaxed) { "VIB " } else { "" },
            if s.sustain.load(Ordering::Relaxed) { "SUS" } else { "" },
        );
        Text::new(&info, Point::new(x0, by + 62), dim).draw(fb).ok();
        let hint = if self.bass_layer { "pads 1-15: bass keys   pad 16: fill-in   F2: melody" } else { "pads: 16 melody keys   F2: bass keys   F3: rhythm" };
        Text::new(hint, Point::new(16, 337), dim).draw(fb).ok();
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let tables: Vec<Vec<f32>> = TONES.iter().map(|t| build_table(&t.harmonics)).collect();
        Some(Box::new(Processor {
            shared: Arc::clone(&self.shared),
            tables: Arc::new(tables),
            bass_table: Arc::new(build_table(&BASS_HARMONICS)),
            voices: [Voice::OFF; VOICES],
            prev_keys: [false; KEYS],
            clock: 0,
            vib_phase: 0.0,
            bass_phase: 0.0,
            bass_freq: 65.0,
            bass_env: 0.0,
            bass_gate: false,
            prev_bass_key: -1,
            auto_root: -1,
            step_pos: 0.0,
            seq_step: 0,
            seq_bar: 0,
            was_playing: false,
            drums: [Drum::default(); 5],
            noise: 0x1234_5678,
            hp: 0.0,
            hp_prev: 0.0,
        }))
    }

    fn running(&self) -> Option<bool> {
        Some(self.shared.playing.load(Ordering::Relaxed))
    }

    fn toggle_running(&mut self) {
        let s = &self.shared;
        let p = !s.playing.load(Ordering::Relaxed);
        if p {
            s.step.store(0, Ordering::Relaxed);
            s.bar.store(0, Ordering::Relaxed);
            s.synchro.store(false, Ordering::Relaxed);
        }
        s.playing.store(p, Ordering::Relaxed);
    }

    fn supports_pad_lock(&self) -> bool {
        true
    }

    fn needs_background_audio(&self) -> bool {
        self.shared.active_voices.load(Ordering::Relaxed) > 0 || self.shared.bass_sounding.load(Ordering::Relaxed) >= 0
    }

    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(if self.bass_layer { "BASS" } else { "KEYS" })
    }

    fn toggle_grid_mode(&mut self) {
        self.set_layer(!self.bass_layer);
    }

    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let s = &self.shared;
        std::array::from_fn(|i| {
            if self.bass_layer {
                if i == 15 {
                    if s.fill_held.load(Ordering::Relaxed) { PadColor::Yellow } else { PadColor::Blue }
                } else if s.bass_key.load(Ordering::Relaxed) == i as i32 {
                    PadColor::Green
                } else if s.bass_sounding.load(Ordering::Relaxed) == i as i32 {
                    PadColor::Yellow
                } else {
                    PadColor::Off
                }
            } else {
                let key = self.window + i;
                if s.keys[key].load(Ordering::Relaxed) {
                    PadColor::Green
                } else if (KEY_LOW + key as i32) % 12 == 0 {
                    PadColor::Blue // mark each C so the window is readable
                } else {
                    PadColor::Off
                }
            }
        })
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        self.list.selected = self.list.selected.min(rows.len() - 1);
        let (a, b) = self.list.centered_scroll_window(visible, rows.len());
        (rows[a..b].to_vec(), self.list.selected - a, a > 0, b < rows.len())
    }
    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        crate::app::SlintExtra::Tinkertone(self.panel_extra())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> TinkertoneApp {
        let modbus = Arc::new(ModBus::new());
        TinkertoneApp::new(
            Arc::new(AtomicF32::new(0.1)),
            Arc::new(AtomicF32::new(3.0)),
            Arc::clone(&modbus),
            Arc::new(AudioBus::new()),
            Arc::new(MixerBus::new()),
        )
    }

    fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for _ in 0..blocks {
            let mut buf = vec![0.0f32; 512 * 2];
            p.process(&mut buf, 2, 48_000.0);
            all.extend(buf.iter().step_by(2));
        }
        all
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn pads(on: &[usize]) -> Input {
        let mut i = Input::default();
        for &p in on {
            i.grid[p] = true;
        }
        i
    }

    #[test]
    fn a_held_key_sounds_and_release_goes_quiet() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.tick(&pads(&[0]));
        let held = render(&mut p, 20);
        assert!(rms(&held) > 0.01, "holding a melody pad must sound");
        a.tick(&Input::default());
        render(&mut p, 200);
        let after = render(&mut p, 10);
        assert!(rms(&after) < 1e-3, "released note must decay to silence");
    }

    #[test]
    fn polyphony_is_eight_and_the_ninth_steals() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.tick(&pads(&(0..9).collect::<Vec<_>>()));
        render(&mut p, 2);
        assert_eq!(a.shared.active_voices.load(Ordering::Relaxed), 8);
    }

    #[test]
    fn every_tone_produces_finite_audio() {
        for tone in 0..TONES.len() {
            let mut a = app();
            a.shared.preset_tones[0].store(tone, Ordering::Relaxed);
            a.shared.vibrato.store(true, Ordering::Relaxed);
            let mut p = a.audio_processor().unwrap();
            a.tick(&pads(&[3, 7, 10]));
            let out = render(&mut p, 10);
            assert!(out.iter().all(|v| v.is_finite()), "{}", TONES[tone].name);
            assert!(rms(&out) > 0.002, "{} is silent", TONES[tone].name);
        }
    }

    #[test]
    fn manual_bass_is_monophonic_and_follows_the_last_key() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.toggle_grid_mode();
        assert_eq!(a.grid_mode_label(), Some("BASS"));
        a.tick(&pads(&[0]));
        render(&mut p, 4);
        assert_eq!(a.shared.bass_sounding.load(Ordering::Relaxed), 0);
        a.tick(&pads(&[0, 7]));
        render(&mut p, 4);
        assert_eq!(a.shared.bass_sounding.load(Ordering::Relaxed), 7, "newest bass key wins");
        a.tick(&Input::default());
        render(&mut p, 100);
        assert_eq!(a.shared.bass_sounding.load(Ordering::Relaxed), -1);
    }

    #[test]
    fn synchro_start_begins_the_rhythm_on_the_first_bass_key() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.shared.synchro.store(true, Ordering::Relaxed);
        a.toggle_grid_mode();
        render(&mut p, 4);
        assert_eq!(a.running(), Some(false));
        a.tick(&pads(&[2]));
        assert_eq!(a.running(), Some(true), "first bass key starts the rhythm");
        let out = render(&mut p, 40);
        assert!(rms(&out) > 0.01);
    }

    #[test]
    fn auto_bass_plays_chord_tones_from_the_held_root() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.shared.bass_auto.store(true, Ordering::Relaxed);
        a.shared.rhythm.store(5, Ordering::Relaxed); // Pops: root x4 then fifth x4
        a.shared.tempo.set(200.0);
        a.toggle_grid_mode();
        a.toggle_running();
        a.tick(&pads(&[2])); // root D
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..120 {
            render(&mut p, 1);
            let b = a.shared.bass_sounding.load(Ordering::Relaxed);
            if b >= 0 {
                seen.insert(b);
            }
        }
        assert!(seen.contains(&2), "root heard");
        assert!(seen.contains(&9), "fifth (root + 7) heard: {seen:?}");
    }

    #[test]
    fn every_rhythm_runs_and_fill_in_overrides_it() {
        for r in 0..RHYTHMS.len() {
            assert!(RHYTHMS[r].drums.iter().chain(RHYTHMS[r].drums_bar4.iter()).all(|g| g.len() == RHYTHMS[r].steps), "{}", RHYTHM_NAMES[r]);
            assert_eq!(RHYTHMS[r].bass.len(), RHYTHMS[r].steps);
            assert_eq!(RHYTHMS[r].bass_bar4.len(), RHYTHMS[r].steps);
            let mut a = app();
            a.shared.rhythm.store(r, Ordering::Relaxed);
            let mut p = a.audio_processor().unwrap();
            a.toggle_running();
            let out = render(&mut p, 60);
            assert!(out.iter().all(|v| v.is_finite()));
            assert!(rms(&out) > 0.005, "{} is silent", RHYTHM_NAMES[r]);
        }
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.toggle_grid_mode();
        a.toggle_running();
        a.tick(&pads(&[15]));
        assert!(a.shared.fill_held.load(Ordering::Relaxed));
        let kind1 = a.shared.fill_kind.load(Ordering::Relaxed);
        render(&mut p, 20);
        a.tick(&Input::default());
        assert!(!a.shared.fill_held.load(Ordering::Relaxed));
        a.tick(&pads(&[15]));
        assert_ne!(a.shared.fill_kind.load(Ordering::Relaxed), kind1, "fill kind alternates press to press");
    }

    #[test]
    fn key_window_covers_all_37_keys_and_layer_change_releases_notes() {
        let mut a = app();
        a.window = 0;
        for _ in 0..40 {
            a.edit(Sel::KeyWindow, 1);
        }
        assert_eq!(a.window, KEYS - 16);
        a.tick(&pads(&[15]));
        assert!(a.shared.keys[KEYS - 1].load(Ordering::Relaxed), "top key reachable");
        a.toggle_grid_mode();
        assert!(!a.shared.keys[KEYS - 1].load(Ordering::Relaxed), "layer change releases held keys");
    }
}
