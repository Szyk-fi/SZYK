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
//! - The melody tone generator: tones here are additive wavetables
//!   quantised to 8 bits, summed and quantised again at one 8-bit "DAC".
//!   A model of the character, not a capture of the real chip.
//! - Exact drum and bass circuit values (no MT-40 service data was found):
//!   kick 64 Hz, snare body 185 Hz, bass low-pass 260 Hz, output low-pass
//!   9 kHz are chosen in the range of period analog rhythm units.
//!
//! How the sound is built (researched October 2026):
//! - Casio's own staff describe the MT-40's bass and drums as analog,
//!   "characterized by a thick sound" (note-pr.casio.co.jp, "The 'Song
//!   Setting' Hidden in the Miniature Mascot of ... the MT-40").
//!   soundprogramming.net likewise lists "analog percussion sounds" and an
//!   NEC D775G CPU. So here: the bass is a square wave from a divider
//!   (what a CPU makes) through a fixed analog-style low-pass and VCA; the
//!   kick and snare are two-pole resonators pinged by the rhythm trigger
//!   (bridged-T style), and the snare rattle and metals are one shared
//!   noise source through their own filters and decaying VCAs.
//! - Early Casiotones are described as digital oscillators into an analog
//!   output low-pass, with audible quantisation noise (MetaFilter thread
//!   "MT-40 Riddim"; secondary). Hence the 8-bit sum and the output LPF.
//! - The Rock rhythm only came alive slowed to 80-110 BPM (Wikipedia,
//!   "Sleng Teng"), so the power-on tempo is 84.
//! - The auto-bass patterns are original. In particular the factory
//!   "Rock" bassline (the one behind Sleng Teng) is a composed work and is
//!   deliberately not transcribed here.
//!
//! Your own bass line (not on the original): Bass mode My Line loops a
//! line you record on the bass keys, quantised to the rhythm's steps,
//! transposed by the held bass key and bent to the Auto Chord. Saved to
//! `saves/tinkertone/bassline.json`.
//!
//! Controls on Portamax: 37 + 15 keys don't fit on 16 pads, so F2 flips the
//! pads between KEYS (a 16-note window you slide across the 37 keys from
//! the menu) and BASS (the 15 bass keys, pad 16 = Fill-in) -- the first two
//! of the shared play kit's pad layers, followed by its Controls and
//! Moments. F3 starts and stops the rhythm.

use crate::app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes};
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

// ------------------------------------------------------------ bass modes

pub const BASS_MANUAL: usize = 0;
pub const BASS_AUTO: usize = 1;
/// Your own recorded line, looping with the rhythm.
pub const BASS_LINE: usize = 2;
pub const BASS_MODE_NAMES: [&str; 3] = ["Manual", "Auto", "My Line"];

/// Longest own line: four bars of sixteenths.
pub const LINE_MAX: usize = 64;
/// Line lengths on offer, in bars.
pub const LINE_BARS: [usize; 3] = [1, 2, 4];
/// Line step values besides a note (semitones from the line's first
/// recorded key): keep sounding the previous note, or go silent.
pub const LINE_HOLD: i32 = 1000;
pub const LINE_REST: i32 = 1001;

/// A recorded interval, bent to the chord the auto-bass is set to: lines
/// are heard as major, so Minor and Minor 7th flatten the third and the
/// major seventh.
fn line_interval(off: i32, chord: usize) -> i32 {
    if chord == 0 {
        return off;
    }
    let (oct, pc) = (off.div_euclid(12), off.rem_euclid(12));
    let pc = match pc {
        4 => 3,
        11 => 10,
        x => x,
    };
    oct * 12 + pc
}

/// Tempo knob range, BPM. Unverified for the real knob; wide enough for
/// the 80-110 BPM range the Rock rhythm is known to be used at.
pub const TEMPO_MIN: f32 = 40.0;
pub const TEMPO_MAX: f32 = 240.0;
/// Power-on tempo: in the reggae/dancehall range (see the module doc).
pub const TEMPO_DEFAULT: f32 = 84.0;

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
    bass_mode: AtomicUsize,
    /// Your own bass line: a semitone offset from `line_ref`, or
    /// LINE_HOLD / LINE_REST, per step.
    line: [AtomicI32; LINE_MAX],
    /// The bass key (0..15) the line was first recorded from, -1 = empty.
    line_ref: AtomicI32,
    /// Index into LINE_BARS.
    line_bars: AtomicUsize,
    line_rec: AtomicBool,
    /// Bumped on every change to the line, so the UI knows to save it.
    line_dirty: AtomicU32,
    line_pos: AtomicUsize,
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
            tempo: AtomicF32::new(TEMPO_DEFAULT),
            playing: AtomicBool::new(false),
            synchro: AtomicBool::new(false),
            fill_held: AtomicBool::new(false),
            fill_kind: AtomicUsize::new(1),
            bass_mode: AtomicUsize::new(BASS_MANUAL),
            line: std::array::from_fn(|_| AtomicI32::new(LINE_HOLD)),
            line_ref: AtomicI32::new(-1),
            line_bars: AtomicUsize::new(1),
            line_rec: AtomicBool::new(false),
            line_dirty: AtomicU32::new(0),
            line_pos: AtomicUsize::new(0),
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
        let take = self.line_rec.load(Ordering::Relaxed) && self.bass_mode.load(Ordering::Relaxed) == BASS_LINE;
        if (self.synchro.load(Ordering::Relaxed) || take) && !self.playing.load(Ordering::Relaxed) {
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

/// A two-pole resonator: what an analog drum's bridged-T network does
/// when the rhythm generator kicks it with a trigger pulse -- rings at
/// one pitch and dies away exponentially, no oscillator involved.
#[derive(Clone, Copy, Default)]
struct Reso {
    a1: f32,
    a2: f32,
    gain: f32,
    y1: f32,
    y2: f32,
}

impl Reso {
    /// `t60`: seconds to fall 60 dB.
    fn tune(&mut self, hz: f32, t60: f32, sr: f32) {
        let w = TAU * hz / sr;
        let r = 10f32.powf(-3.0 / (t60.max(0.005) * sr));
        self.a1 = 2.0 * r * w.cos();
        self.a2 = r * r;
        self.gain = w.sin(); // unit-amplitude ring for a unit impulse
    }
    fn tick(&mut self, x: f32) -> f32 {
        let y = x * self.gain + self.a1 * self.y1 - self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Zavalishin's TPT state-variable filter, low-pass out. Stands in for
/// the analog RC/op-amp low-pass stages after the digital oscillators.
#[derive(Clone, Copy, Default)]
struct Svf {
    g: f32,
    k: f32,
    ic1: f32,
    ic2: f32,
}

impl Svf {
    fn tune(&mut self, hz: f32, q: f32, sr: f32) {
        self.g = (std::f32::consts::PI * (hz / sr).min(0.49)).tan();
        self.k = 1.0 / q.max(0.1);
    }
    fn lp(&mut self, x: f32) -> f32 {
        let a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + self.g * a1 * v3;
        let v2 = self.ic2 + self.g * v1;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v2
    }
    fn bp(&mut self, x: f32) -> f32 {
        let a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + self.g * a1 * v3;
        let v2 = self.ic2 + self.g * v1;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v1
    }
}

/// One-pole high-pass (a coupling capacitor).
#[derive(Clone, Copy, Default)]
struct Hp1 {
    c: f32,
    x1: f32,
    y1: f32,
}

impl Hp1 {
    fn tune(&mut self, hz: f32, sr: f32) {
        self.c = (-TAU * hz / sr).exp();
    }
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.c * (self.y1 + x - self.x1);
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// The five analog drum voices. Kick and snare are resonators pinged by
/// the trigger; the snare's rattle and the metals are the shared noise
/// source through their own filters and decaying VCAs.
#[derive(Clone, Copy, Default)]
struct Drums {
    kick: Reso,
    snare: Reso,
    snare_bp: Svf,
    hat_hp: [Hp1; 2],
    cym_bp: Svf,
    /// Noise VCA levels: snare rattle, closed hat, open hat, cymbal.
    env: [f32; 4],
    /// Per-sample decay multipliers for those four.
    dec: [f32; 4],
    tuned_sr: f32,
}

impl Drums {
    fn retune(&mut self, sr: f32) {
        if self.tuned_sr == sr {
            return;
        }
        self.tuned_sr = sr;
        self.kick.tune(KICK_HZ, 0.42, sr);
        self.snare.tune(SNARE_HZ, 0.16, sr);
        self.snare_bp.tune(2400.0, 0.9, sr);
        self.hat_hp[0].tune(6500.0, sr);
        self.hat_hp[1].tune(6500.0, sr);
        self.cym_bp.tune(5200.0, 0.7, sr);
        // time constants (s) of the noise VCAs
        for (d, tau) in self.dec.iter_mut().zip([0.07f32, 0.022, 0.16, 0.55]) {
            *d = (-1.0 / (tau * sr)).exp();
        }
    }
}

/// Kick and snare body pitches. Unverified (no MT-40 service data found);
/// chosen in the range of period analog rhythm units.
const KICK_HZ: f32 = 64.0;
const SNARE_HZ: f32 = 185.0;

/// Fixed corner of the bass's analog low-pass. The bass keys span
/// C2-D3 (65-147 Hz), so it passes the fundamental and a soft third
/// harmonic: round and thick, the way the bass is described.
const BASS_LPF_HZ: f32 = 260.0;
/// Corner of the output stage's low-pass on melody + bass + drums.
const OUT_LPF_HZ: f32 = 9000.0;

struct Processor {
    shared: Arc<Shared>,
    tables: Arc<Vec<Vec<f32>>>,
    voices: [Voice; VOICES],
    prev_keys: [bool; KEYS],
    clock: u64,
    vib_phase: f32,
    // bass
    bass_phase: f32,
    bass_freq: f32,
    bass_env: f32,
    bass_gate: bool,
    bass_attack: bool,
    bass_lpf: Svf,
    bass_hp: Hp1,
    prev_bass_key: i32,
    auto_root: i32,
    // own-line recording
    rec_key: i32,
    rec_step: usize,
    last_idx: usize,
    // sequencer
    step_pos: f64,
    seq_step: usize,
    seq_bar: usize,
    was_playing: bool,
    drums: Drums,
    noise: u32,
    out_lpf: Svf,
    out_hp: Hp1,
    tuned_sr: f32,
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
        let dr = &mut self.drums;
        match d {
            0 => {
                dr.kick.tick(amp * 1.2);
            }
            1 => {
                dr.snare.tick(amp * 0.8);
                dr.env[0] = amp;
            }
            2 => {
                dr.env[1] = amp;
                dr.env[2] = 0.0; // closed hat chokes the open hat
            }
            3 => dr.env[2] = amp,
            _ => dr.env[3] = amp,
        }
        self.shared.drum_flash[d].set(1.0);
    }

    fn drum_sample(&mut self) -> f32 {
        let n = self.noise();
        let dr = &mut self.drums;
        // The kick's ring is soft-clipped a touch, as the transistor
        // buffer after a bridged-T would; that's most of its weight.
        let kick = (dr.kick.tick(0.0) * 1.6).tanh();
        let body = dr.snare.tick(0.0) * 0.55;
        let rattle = dr.snare_bp.bp(n) * dr.env[0] * 0.9;
        let thin = dr.hat_hp[0].tick(n);
        let hiss = dr.hat_hp[1].tick(thin);
        let hats = hiss * (dr.env[1] * 0.55 + dr.env[2] * 0.4);
        let cym = dr.cym_bp.bp(n) * dr.env[3] * 0.35;
        for (e, d) in dr.env.iter_mut().zip(dr.dec) {
            *e *= d;
        }
        kick + body + rattle + hats + cym
    }

    /// The line step that's playing right now, and the next one to play.
    fn line_slots(&self, steps: usize) -> (usize, usize) {
        let bars = LINE_BARS[self.shared.line_bars.load(Ordering::Relaxed).min(2)];
        let len = (steps * bars).clamp(1, LINE_MAX);
        let next = ((self.seq_bar % bars) * steps + self.seq_step) % len;
        (self.last_idx % len, next)
    }

    fn sequencer_tick(&mut self) {
        let s = self.shared.clone();
        let r = &RHYTHMS[s.rhythm.load(Ordering::Relaxed).min(5)];
        let step = self.seq_step % r.steps;
        let bar4 = self.seq_bar % BARS == BARS - 1;
        let (_, idx) = self.line_slots(r.steps);
        self.last_idx = idx;
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
        match s.bass_mode.load(Ordering::Relaxed) {
            // auto bass (only while the rhythm runs, with a root)
            BASS_AUTO if self.auto_root >= 0 => {
                let line = if bar4 { r.bass_bar4 } else { r.bass };
                if let Some(off) = line.as_bytes().get(step).and_then(|c| bass_offset(*c, s.chord.load(Ordering::Relaxed))) {
                    self.bass_note((BASS_LOW + self.auto_root + off) as f32, true);
                }
            }
            BASS_LINE => {
                if self.rec_key >= 0 {
                    // Holding a key while recording: it owns every step it
                    // spans, wiping whatever an earlier take put there.
                    if idx != self.rec_step {
                        s.line[idx].store(LINE_HOLD, Ordering::Relaxed);
                        s.line_dirty.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    let reference = s.line_ref.load(Ordering::Relaxed);
                    let root = if self.auto_root >= 0 { self.auto_root } else { reference };
                    match s.line[idx].load(Ordering::Relaxed) {
                        LINE_HOLD => {}
                        LINE_REST => self.bass_gate = false,
                        off if reference >= 0 => {
                            let n = BASS_LOW + root + line_interval(off, s.chord.load(Ordering::Relaxed));
                            self.bass_note(n as f32, true);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        s.step.store(step, Ordering::Relaxed);
        s.bar.store(self.seq_bar % BARS, Ordering::Relaxed);
        s.line_pos.store(idx, Ordering::Relaxed);
        self.seq_step += 1;
        if self.seq_step >= r.steps {
            self.seq_step = 0;
            self.seq_bar = (self.seq_bar + 1) % BARS;
        }
    }

    fn bass_note(&mut self, note: f32, retrigger: bool) {
        self.bass_freq = midi_hz(note);
        if retrigger {
            self.bass_attack = true;
        }
        self.bass_gate = true;
        self.shared.bass_sounding.store(note as i32 - BASS_LOW, Ordering::Relaxed);
    }

    /// A bass key went down or up while recording your own line. Notes
    /// land on the nearest step; a key let go inside its own step rests on
    /// the next one.
    fn record_key(&mut self, bk: i32, steps: usize) {
        let s = Arc::clone(&self.shared);
        let bars = LINE_BARS[s.line_bars.load(Ordering::Relaxed).min(2)];
        let len = (steps * bars).clamp(1, LINE_MAX);
        let (now, next) = self.line_slots(steps);
        let fresh = self.step_pos >= 1.0; // rhythm just started: nothing has played yet
        let slot = if fresh || self.step_pos >= 0.5 { next } else { now };
        if bk >= 0 {
            let mut reference = s.line_ref.load(Ordering::Relaxed);
            if reference < 0 {
                reference = bk;
                s.line_ref.store(bk, Ordering::Relaxed);
            }
            s.line[slot].store((bk - reference).clamp(-24, 24), Ordering::Relaxed);
            self.rec_key = bk;
            self.rec_step = slot;
            self.bass_note((BASS_LOW + bk) as f32, true);
        } else if self.rec_key >= 0 {
            let at = if slot == self.rec_step { (slot + 1) % len } else { slot };
            if s.line[at].load(Ordering::Relaxed) == LINE_HOLD {
                s.line[at].store(LINE_REST, Ordering::Relaxed);
            }
            self.rec_key = -1;
            self.bass_gate = false;
        }
        s.line_dirty.fetch_add(1, Ordering::Relaxed);
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sr: f32) {
        if channels == 0 || sr <= 0.0 {
            return;
        }
        let s = Arc::clone(&self.shared);
        s.sample_rate.store(sr as u32, Ordering::Relaxed);
        if self.tuned_sr != sr {
            self.tuned_sr = sr;
            self.bass_lpf.tune(BASS_LPF_HZ, 0.75, sr);
            self.bass_hp.tune(28.0, sr);
            self.out_lpf.tune(OUT_LPF_HZ, 0.6, sr);
            self.out_hp.tune(40.0, sr);
        }
        self.drums.retune(sr);
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

        let r = &RHYTHMS[s.rhythm.load(Ordering::Relaxed).min(5)];
        let playing = s.playing.load(Ordering::Relaxed);
        if playing && !self.was_playing {
            self.seq_step = s.step.load(Ordering::Relaxed);
            self.seq_bar = s.bar.load(Ordering::Relaxed);
            self.step_pos = 1.0; // fire the first step right away
        }

        // ---- bass key (manual, or root for auto / your line, or a take)
        let bk = s.bass_key.load(Ordering::Relaxed);
        let mode = s.bass_mode.load(Ordering::Relaxed);
        let follow = mode != BASS_MANUAL && playing;
        let recording = mode == BASS_LINE && playing && s.line_rec.load(Ordering::Relaxed);
        if !recording {
            self.rec_key = -1;
        }
        if bk != self.prev_bass_key {
            if recording {
                self.record_key(bk, r.steps);
            } else if bk >= 0 {
                self.auto_root = bk;
                if !follow {
                    self.bass_note((BASS_LOW + bk) as f32, true);
                }
            } else if !follow {
                self.bass_gate = false;
            }
            self.prev_bass_key = bk;
        }
        if !playing && self.was_playing {
            self.bass_gate = false;
            self.auto_root = if bk >= 0 { bk } else { -1 };
        }
        self.was_playing = playing;

        let bpm = (s.tempo.get() + s.ext_tempo.get() * 100.0).clamp(TEMPO_MIN, TEMPO_MAX);
        let steps_per_sec = bpm / 60.0 * r.steps_per_beat as f32;
        let step_inc = steps_per_sec as f64 / sr as f64;

        let vol = (s.volume.get() + s.ext_volume.get()).clamp(0.0, 1.0);
        let acc = (s.accomp.get() + s.ext_accomp.get()).clamp(0.0, 1.0);
        let mix = (s.mix_level.get() + s.ext_mix_level.get()).clamp(0.0, 2.0);
        let vibrato = s.vibrato.load(Ordering::Relaxed);
        let sustain = s.sustain.load(Ordering::Relaxed);
        let dt = 1.0 / sr;
        // bass VCA: ~2 ms attack, a slow sag while held, ~45 ms release
        let bass_att = 1.0 - (-dt / 0.002).exp();
        let bass_sag = 1.0 - (-dt / 0.45).exp();
        let bass_rel = 1.0 - (-dt / 0.045).exp();
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
            // The voices are summed digitally and leave through one 8-bit
            // DAC: quantise the sum, grit and all.
            let mel = (mel / VOICES as f32 * 127.0).round() / 127.0 * VOICES as f32;

            // Bass: a square from the CPU's divider into the analog
            // low-pass and VCA.
            let bass = if self.bass_gate || self.bass_env > 0.0005 {
                if self.bass_attack {
                    self.bass_env += (1.0 - self.bass_env) * bass_att;
                    if self.bass_env > 0.98 {
                        self.bass_attack = false;
                    }
                } else if self.bass_gate {
                    self.bass_env += (0.7 - self.bass_env) * bass_sag;
                } else {
                    self.bass_env -= self.bass_env * bass_rel;
                }
                self.bass_phase = (self.bass_phase + self.bass_freq * dt) % 1.0;
                let sq = if self.bass_phase < 0.5 { 1.0 } else { -1.0 };
                self.bass_hp.tick(self.bass_lpf.lp(sq)) * self.bass_env
            } else {
                self.bass_lpf.lp(0.0);
                0.0
            };

            let drums = self.drum_sample();
            // Gains leave headroom for all 8 voices on a sustained tone over
            // the full accompaniment; tanh rounds off anything left rather
            // than hard-clipping (a small speaker amp of the era would have
            // compressed similarly, not squared off).
            let pre = (mel * 0.12 * vol) + (bass * 0.36 + drums * 0.42) * acc * vol;
            let out = self.out_hp.tick(self.out_lpf.lp(pre)) * mix;
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
    LineRec,
    LineBars,
    LineClear,
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
        2 => &[Sel::BassMode, Sel::Chord, Sel::LineRec, Sel::LineBars, Sel::LineClear],
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
    /// Where your own bass line is saved (None in tests).
    line_path: Option<std::path::PathBuf>,
    line_saved: u32,
    line_seen: u32,
    line_quiet: u32,
    /// The shared play view (play_kit.rs). Its two native layers *are*
    /// `bass_layer`: KEYS = 0, BASS = 1, kept in step by `set_layer`.
    kit: PlayKit,
}

const LAYER_KEYS: u8 = 0;
const LAYER_BASS: u8 = 1;

/// The play view's controls, most important first. The MT-40's only
/// continuous controls are the two volumes and the tempo (vibrato and
/// sustain are switches on the real thing, not depths), so those take the
/// first knob pair and the expression routes; the panel's buttons follow.
const CONTROLS: [(Sel, &str); 13] = [
    (Sel::Volume, "Volume"),
    (Sel::Accomp, "Accomp"),
    (Sel::Tempo, "Tempo"),
    (Sel::Rhythm, "Rhythm"),
    (Sel::Preset, "Tone Preset"),
    (Sel::PresetTone, "Tone"),
    (Sel::Vibrato, "Vibrato"),
    (Sel::Sustain, "Sustain"),
    (Sel::BassMode, "Bass Mode"),
    (Sel::Chord, "Chord"),
    (Sel::Synchro, "Synchro"),
    (Sel::KeyWindow, "Key Window"),
    (Sel::LineRec, "Rec Line"),
];
const C_VOLUME: usize = 0;
const C_ACCOMP: usize = 1;
const C_TEMPO: usize = 2;
const C_PRESET: usize = 4;
/// Key Window positions: the 16-pad window's first key, 0..=KEYS-16.
const WINDOWS: usize = KEYS - 16 + 1;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "tinkertone",
        layers: vec![Layer::Native(LAYER_KEYS, "KEYS"), Layer::Native(LAYER_BASS, "BASS"), Layer::Controls, Layer::Moments],
        hero: vec![[C_VOLUME, C_ACCOMP], [C_TEMPO, 3], [C_PRESET, 5], [6, 7]],
        // The four tone-preset buttons are what an MT-40 player reaches
        // for most; D-pad up/down steps them.
        browse: Some(C_PRESET),
        // Stick Y swells the master volume like an expression pedal, X
        // pushes the tempo (rushing/dragging the rhythm); the left hand
        // rides the accompaniment level, the right the master volume.
        routes: Routes { stick_x: Some(C_TEMPO), stick_y: Some(C_VOLUME), hand_l: Some(C_ACCOMP), hand_r: Some(C_VOLUME) },
        throws: Vec::new(),
        midi_to_pads: true,
        own_expression: false,
    }
}

fn usize_norm(v: usize, n: usize) -> f32 {
    if n < 2 { 0.0 } else { v.min(n - 1) as f32 / (n - 1) as f32 }
}

fn usize_pick(v: f32, n: usize) -> usize {
    (v.clamp(0.0, 1.0) * n.saturating_sub(1) as f32).round() as usize
}

impl TinkertoneApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, _audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let mut app = Self {
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
            line_path: (!cfg!(test)).then(|| std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves/tinkertone/bassline.json"))),
            line_saved: 0,
            line_seen: 0,
            line_quiet: 0,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
        };
        app.load_line();
        app
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
            Sel::LineRec => "Record Line",
            Sel::LineBars => "Line Length",
            Sel::LineClear => ">> Clear Line",
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
            Sel::BassMode => BASS_MODE_NAMES[s.bass_mode.load(Ordering::Relaxed).min(2)].into(),
            Sel::LineRec => if s.line_rec.load(Ordering::Relaxed) { "REC" } else { "off" }.into(),
            Sel::LineBars => {
                let b = LINE_BARS[s.line_bars.load(Ordering::Relaxed).min(2)];
                format!("{b} bar{}", if b == 1 { "" } else { "s" })
            }
            Sel::LineClear => {
                if s.line_ref.load(Ordering::Relaxed) < 0 { "empty".into() } else { format!("{} notes", self.line_notes()) }
            }
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
            Sel::Tempo => nudge(&s.tempo, accelerate(delta) * 10.0 * sens, TEMPO_MIN, TEMPO_MAX),
            Sel::Synchro => s.synchro.store(delta > 0, Ordering::Relaxed),
            Sel::BassMode => cycle(&s.bass_mode, BASS_MODE_NAMES.len()),
            Sel::LineRec => self.set_line_rec(delta > 0),
            Sel::LineBars => {
                cycle(&s.line_bars, LINE_BARS.len());
                s.line_dirty.fetch_add(1, Ordering::Relaxed);
            }
            Sel::Chord => cycle(&s.chord, 3),
            Sel::Volume => nudge(&s.volume, accelerate(delta) * 0.1 * sens, 0.0, 1.0),
            Sel::Accomp => nudge(&s.accomp, accelerate(delta) * 0.1 * sens, 0.0, 1.0),
            Sel::PadLayer => self.set_layer(delta > 0),
            Sel::KeyWindow => {
                self.release_pads();
                self.window = (self.window as i32 + step).clamp(0, (KEYS - 16) as i32) as usize;
            }
            Sel::StartStop | Sel::FillIn | Sel::LineClear => {}
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
                let m = &self.shared.bass_mode;
                m.store((m.load(Ordering::Relaxed) + 1) % BASS_MODE_NAMES.len(), Ordering::Relaxed);
            }
            Sel::LineRec => self.set_line_rec(!self.shared.line_rec.load(Ordering::Relaxed)),
            Sel::LineClear => self.clear_line(),
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
        // The menu's Pads row and F2 are two ways to the same thing.
        self.kit.set_native(if bass { LAYER_BASS } else { LAYER_KEYS });
    }

    /// After F2 moves the kit: if it landed on KEYS or BASS, the app's own
    /// mode follows (releasing anything the other layer held).
    fn sync_layer(&mut self) {
        if let Layer::Native(id, _) = self.kit.layer() {
            self.set_layer(id == LAYER_BASS);
        }
    }

    fn knob(&self, i: usize) -> Knob<'_> {
        let s = &self.shared;
        // Ranges are exactly what `edit` clamps each one to; the
        // AtomicUsize choices have no Knob variant and are handled by hand
        // in kit_norm/kit_set_norm.
        match CONTROLS[i % CONTROLS.len()].0 {
            Sel::Volume => Knob::F(&s.volume, 0.0, 1.0),
            Sel::Accomp => Knob::F(&s.accomp, 0.0, 1.0),
            Sel::Tempo => Knob::F(&s.tempo, TEMPO_MIN, TEMPO_MAX),
            Sel::Vibrato => Knob::B(&s.vibrato),
            Sel::Sustain => Knob::B(&s.sustain),
            Sel::LineRec => Knob::B(&s.line_rec),
            Sel::Synchro => Knob::B(&s.synchro),
            _ => Knob::None,
        }
    }

    /// Arm or disarm recording your own line. Arming switches the bass to
    /// My Line; with the rhythm stopped, the first bass key starts it.
    fn set_line_rec(&mut self, on: bool) {
        let s = &self.shared;
        if on {
            s.bass_mode.store(BASS_LINE, Ordering::Relaxed);
        }
        s.line_rec.store(on, Ordering::Relaxed);
    }

    fn clear_line(&mut self) {
        let s = &self.shared;
        for c in &s.line {
            c.store(LINE_HOLD, Ordering::Relaxed);
        }
        s.line_ref.store(-1, Ordering::Relaxed);
        s.line_dirty.fetch_add(1, Ordering::Relaxed);
    }

    /// Steps in the line at its current rhythm and length.
    fn line_len(&self) -> usize {
        let s = &self.shared;
        let r = &RHYTHMS[s.rhythm.load(Ordering::Relaxed).min(5)];
        (r.steps * LINE_BARS[s.line_bars.load(Ordering::Relaxed).min(2)]).min(LINE_MAX)
    }

    fn line_notes(&self) -> usize {
        self.shared.line[..self.line_len()].iter().filter(|c| c.load(Ordering::Relaxed) < LINE_HOLD).count()
    }

    /// The line for the panel: per step, the bass key it plays (0..15,
    /// may run past either end), -1 hold, -2 rest.
    #[allow(dead_code)] // Slint GUI only
    fn line_view(&self) -> Vec<i32> {
        let s = &self.shared;
        let reference = s.line_ref.load(Ordering::Relaxed);
        s.line[..self.line_len()]
            .iter()
            .map(|c| match c.load(Ordering::Relaxed) {
                LINE_HOLD => -1,
                LINE_REST => -2,
                off => (reference + off).max(0),
            })
            .collect()
    }

    /// Writes the line to the SD card once it has been still for half a
    /// second (so a take isn't saved note by note).
    fn autosave_line(&mut self) {
        let Some(path) = self.line_path.clone() else { return };
        let dirty = self.shared.line_dirty.load(Ordering::Relaxed);
        if dirty == self.line_saved {
            self.line_quiet = 0;
            return;
        }
        if dirty != self.line_seen {
            self.line_seen = dirty;
            self.line_quiet = 0;
            return;
        }
        self.line_quiet += 1;
        if self.line_quiet < 30 {
            return;
        }
        self.line_saved = dirty;
        let s = &self.shared;
        let steps: Vec<serde_json::Value> = s
            .line
            .iter()
            .map(|c| match c.load(Ordering::Relaxed) {
                LINE_HOLD => serde_json::Value::from("-"),
                LINE_REST => serde_json::Value::from("x"),
                off => serde_json::Value::from(off),
            })
            .collect();
        let doc = serde_json::json!({
            "reference": s.line_ref.load(Ordering::Relaxed),
            "bars": LINE_BARS[s.line_bars.load(Ordering::Relaxed).min(2)],
            "steps": steps,
        });
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap_or_default()) {
            eprintln!("Tinkertone: couldn't save the bass line: {e}");
        }
    }

    fn load_line(&mut self) {
        let Some(path) = &self.line_path else { return };
        let Ok(text) = std::fs::read_to_string(path) else { return };
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
            eprintln!("Tinkertone: {} isn't a bass line; ignoring it", path.display());
            return;
        };
        let s = &self.shared;
        let reference = doc["reference"].as_i64().unwrap_or(-1).clamp(-1, BASS_KEYS as i64 - 1) as i32;
        s.line_ref.store(reference, Ordering::Relaxed);
        if let Some(i) = LINE_BARS.iter().position(|&b| Some(b as u64) == doc["bars"].as_u64()) {
            s.line_bars.store(i, Ordering::Relaxed);
        }
        if let Some(steps) = doc["steps"].as_array() {
            for (c, v) in s.line.iter().zip(steps) {
                let val = match v {
                    serde_json::Value::String(x) if x == "x" => LINE_REST,
                    serde_json::Value::Number(n) => n.as_i64().map_or(LINE_HOLD, |n| n.clamp(-24, 24) as i32),
                    _ => LINE_HOLD,
                };
                c.store(val, Ordering::Relaxed);
            }
        }
        if reference >= 0 {
            s.bass_mode.store(BASS_LINE, Ordering::Relaxed);
        }
    }

    fn preset_index(&self) -> usize {
        self.shared.preset.load(Ordering::Relaxed).min(3)
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
            bass_mode: BASS_MODE_NAMES[s.bass_mode.load(Ordering::Relaxed).min(2)].into(),
            line_rec: s.line_rec.load(Ordering::Relaxed),
            line: self.line_view(),
            line_pos: if s.bass_mode.load(Ordering::Relaxed) == BASS_LINE && playing { s.line_pos.load(Ordering::Relaxed) as i32 } else { -1 },
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

impl PlayHost for TinkertoneApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS.len()
    }
    fn kit_label(&self, i: usize) -> String {
        CONTROLS[i % CONTROLS.len()].1.to_string()
    }
    fn kit_value(&self, i: usize) -> String {
        self.leaf_value(CONTROLS[i % CONTROLS.len()].0)
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        let s = &self.shared;
        Some(match CONTROLS[i % CONTROLS.len()].0 {
            Sel::Rhythm => usize_norm(s.rhythm.load(Ordering::Relaxed), RHYTHMS.len()),
            Sel::Preset => usize_norm(self.preset_index(), 4),
            Sel::PresetTone => usize_norm(s.tone(), TONES.len()),
            Sel::Chord => usize_norm(s.chord.load(Ordering::Relaxed), 3),
            Sel::BassMode => usize_norm(s.bass_mode.load(Ordering::Relaxed), BASS_MODE_NAMES.len()),
            Sel::KeyWindow => usize_norm(self.window, WINDOWS),
            _ => return self.knob(i).norm(),
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        !matches!(CONTROLS[i % CONTROLS.len()].0, Sel::Volume | Sel::Accomp | Sel::Tempo)
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit(CONTROLS[i % CONTROLS.len()].0, delta);
    }
    /// Tinkertone's menu has no reset (knob 2 press *presses* a button
    /// there), so these are `Shared::new`'s power-on settings.
    fn kit_reset(&mut self, i: usize) {
        let s = Arc::clone(&self.shared);
        match CONTROLS[i % CONTROLS.len()].0 {
            Sel::Volume => s.volume.set(0.8),
            Sel::Accomp => s.accomp.set(0.7),
            Sel::Tempo => s.tempo.set(TEMPO_DEFAULT),
            Sel::Rhythm => s.rhythm.store(0, Ordering::Relaxed),
            Sel::Preset => s.preset.store(0, Ordering::Relaxed),
            Sel::PresetTone => {
                let p = self.preset_index();
                s.preset_tones[p].store(DEFAULT_PRESETS[p], Ordering::Relaxed);
            }
            Sel::Vibrato => s.vibrato.store(false, Ordering::Relaxed),
            Sel::Sustain => s.sustain.store(false, Ordering::Relaxed),
            Sel::BassMode => s.bass_mode.store(BASS_MANUAL, Ordering::Relaxed),
            Sel::LineRec => s.line_rec.store(false, Ordering::Relaxed),
            Sel::Chord => s.chord.store(0, Ordering::Relaxed),
            Sel::Synchro => s.synchro.store(false, Ordering::Relaxed),
            Sel::KeyWindow => {
                if self.window != 12 {
                    self.release_pads();
                    self.window = 12;
                }
            }
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let s = Arc::clone(&self.shared);
        match CONTROLS[i % CONTROLS.len()].0 {
            Sel::Rhythm => s.rhythm.store(usize_pick(v, RHYTHMS.len()), Ordering::Relaxed),
            Sel::Preset => s.preset.store(usize_pick(v, 4), Ordering::Relaxed),
            // Into whichever preset is selected -- Preset sits before Tone
            // in CONTROLS, so a recalled moment picks the preset first.
            Sel::PresetTone => s.preset_tones[self.preset_index()].store(usize_pick(v, TONES.len()), Ordering::Relaxed),
            Sel::Chord => s.chord.store(usize_pick(v, 3), Ordering::Relaxed),
            Sel::BassMode => s.bass_mode.store(usize_pick(v, BASS_MODE_NAMES.len()), Ordering::Relaxed),
            Sel::LineRec => self.set_line_rec(v >= 0.5),
            Sel::KeyWindow => {
                let w = usize_pick(v, WINDOWS);
                if w != self.window {
                    // Same as the menu's Key Window: moving it lets go of
                    // the keys the pads were holding.
                    self.release_pads();
                    self.window = w;
                }
            }
            _ => self.knob(i).set(v),
        }
    }
    /// KEYS: a key presses the pad that plays it in the current window;
    /// BASS: the bass key of that pitch (never the Fill-in pad). Keys
    /// outside the pads' range fold in by octaves.
    fn kit_midi_pad(&self, note: u8) -> Option<usize> {
        let fold = |d: i32, span: i32| if (0..span).contains(&d) { d } else { d.rem_euclid(12) };
        Some(if self.bass_layer {
            fold(note as i32 - BASS_LOW, BASS_KEYS as i32) as usize
        } else {
            fold(note as i32 - (KEY_LOW + self.window as i32), 16) as usize
        })
    }
    fn kit_pad_label(&self, layer: u8, pad: usize) -> String {
        if layer == LAYER_BASS {
            if pad == 15 { "FILL".into() } else { note_name(BASS_LOW + pad as i32) }
        } else {
            note_name(KEY_LOW + (self.window + pad) as i32)
        }
    }
    /// The colours the pads always had on each layer.
    fn kit_pad_color(&self, layer: u8, pad: usize, _held: bool) -> PadColor {
        let s = &self.shared;
        if layer == LAYER_BASS {
            if pad == 15 {
                if s.fill_held.load(Ordering::Relaxed) { PadColor::Yellow } else { PadColor::Blue }
            } else if s.bass_key.load(Ordering::Relaxed) == pad as i32 {
                PadColor::Green
            } else if s.bass_sounding.load(Ordering::Relaxed) == pad as i32 {
                PadColor::Yellow
            } else {
                PadColor::Off
            }
        } else {
            let key = self.window + pad;
            if s.keys[key].load(Ordering::Relaxed) {
                PadColor::Green
            } else if (KEY_LOW + key as i32) % 12 == 0 {
                PadColor::Blue // mark each C so the window is readable
            } else {
                PadColor::Off
            }
        }
    }
    fn kit_line(&self) -> String {
        let s = &self.shared;
        let rhythm = if s.playing.load(Ordering::Relaxed) {
            let r = &RHYTHMS[s.rhythm.load(Ordering::Relaxed)];
            format!("{} beat {}", RHYTHM_NAMES[s.rhythm.load(Ordering::Relaxed)], s.step.load(Ordering::Relaxed) / r.steps_per_beat + 1)
        } else {
            "stopped".to_string()
        };
        format!("{}  {rhythm}", TONES[s.tone()].name)
    }
}

impl App for TinkertoneApp {
    fn tick(&mut self, input: &Input) {
        // The play view takes the knobs and D-pad first; in the menu they
        // pass straight through. On KEYS/BASS the pads reach the keyboard
        // exactly as before; on the kit's layers the kit hands us an
        // empty grid, which lets go of anything held.
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        if let Some(id) = step.native {
            self.set_layer(id == LAYER_BASS);
        }
        let input = &step.input;
        if self.menu_fill_frames > 0 {
            self.menu_fill_frames -= 1;
            if self.menu_fill_frames == 0 && !(self.bass_layer && input.grid[15]) {
                self.shared.fill_held.store(false, Ordering::Relaxed);
            }
        }
        self.handle_pads(&input.grid);
        self.autosave_line();
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
        if self.kit.menu {
            self.list.draw(fb, 16, 68, 24, 10, &rows);
        } else if let Some(col) = self.play_column() {
            // Ends at x=352, left of the keyboard strip at x=360.
            let pal = kit::draw::Palette { bg: BG, ink: TITLE, accent: ACCENT, dim: DIM, faint: Rgb565::new(8, 14, 7) };
            kit::draw::column(fb, &col, 16, 40, 336, 290, pal);
        }

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
            match s.bass_mode.load(Ordering::Relaxed) {
                BASS_AUTO => CHORD_NAMES[s.chord.load(Ordering::Relaxed)],
                BASS_LINE => if s.line_rec.load(Ordering::Relaxed) { "my line REC" } else { "my line" },
                _ => "manual bass",
            },
            if s.vibrato.load(Ordering::Relaxed) { "VIB " } else { "" },
            if s.sustain.load(Ordering::Relaxed) { "SUS" } else { "" },
        );
        Text::new(&info, Point::new(x0, by + 62), dim).draw(fb).ok();
        let hint = if !self.kit.menu {
            "knobs: dials   D-pad: tone preset   F2: keys/bass/pads   F3: rhythm   R1: menu"
        } else if self.bass_layer {
            "pads 1-15: bass keys   pad 16: fill-in   F2: melody"
        } else {
            "pads: 16 melody keys   F2: bass keys   F3: rhythm"
        };
        Text::new(hint, Point::new(16, 337), dim).draw(fb).ok();
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let tables: Vec<Vec<f32>> = TONES.iter().map(|t| build_table(&t.harmonics)).collect();
        Some(Box::new(Processor {
            shared: Arc::clone(&self.shared),
            tables: Arc::new(tables),
            voices: [Voice::OFF; VOICES],
            prev_keys: [false; KEYS],
            clock: 0,
            vib_phase: 0.0,
            bass_phase: 0.0,
            bass_freq: 65.0,
            bass_env: 0.0,
            bass_gate: false,
            bass_attack: false,
            bass_lpf: Svf::default(),
            bass_hp: Hp1::default(),
            prev_bass_key: -1,
            auto_root: -1,
            rec_key: -1,
            rec_step: 0,
            last_idx: 0,
            step_pos: 0.0,
            seq_step: 0,
            seq_bar: 0,
            was_playing: false,
            drums: Drums::default(),
            noise: 0x1234_5678,
            out_lpf: Svf::default(),
            out_hp: Hp1::default(),
            tuned_sr: 0.0,
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

    fn play_surface(&self) -> bool {
        true
    }

    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (!self.kit.menu).then(|| self.kit.column(self))
    }

    /// F2 cycles KEYS -> BASS -> Controls -> Moments.
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }

    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
        self.sync_layer();
    }

    fn grid_led_overlay(&self) -> [PadColor; 16] {
        self.kit.led_overlay(self)
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
        a.shared.bass_mode.store(BASS_AUTO, Ordering::Relaxed);
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

    #[test]
    fn opens_playable_and_knob1_turns_the_volume() {
        let mut a = app();
        assert!(a.play_column().is_some(), "play view first");
        let v = a.shared.volume.get();
        a.tick(&Input { knob1: -3, ..Default::default() });
        assert!(a.shared.volume.get() < v, "knob 1 is Volume on the play view");
        a.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(a.shared.preset.load(Ordering::Relaxed), 1, "D-pad up = next tone preset");
        a.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(a.play_column().is_none(), "R1 opens the full menu");
    }

    /// F2 walks KEYS -> BASS -> Controls -> Moments -> KEYS, the app's own
    /// mode following the two native layers, and kit layers keep the pads
    /// off the keyboard.
    #[test]
    fn f2_cycles_keys_bass_and_the_kit_layers() {
        let mut a = app();
        assert_eq!(a.grid_mode_label(), Some("KEYS"));
        a.toggle_grid_mode();
        assert!(a.bass_layer && a.grid_mode_label() == Some("BASS"));
        a.toggle_grid_mode();
        assert_eq!(a.grid_mode_label(), Some("CONTROLS"));
        a.tick(&pads(&[3]));
        assert_eq!(a.shared.bass_key.load(Ordering::Relaxed), -1, "Controls pads don't play bass");
        a.toggle_grid_mode();
        a.toggle_grid_mode();
        assert_eq!(a.grid_mode_label(), Some("KEYS"));
        assert!(!a.bass_layer);
        a.tick(&pads(&[0]));
        assert!(a.shared.keys[a.window].load(Ordering::Relaxed), "back on KEYS the pads play again");
        // The menu's Pads row moves F2's layer too.
        a.press(Sel::PadLayer);
        assert_eq!(a.grid_mode_label(), Some("BASS"));
    }

    #[test]
    fn a_midi_key_plays_its_own_pitch_on_both_layers() {
        let mut a = app();
        let mut keys = crate::app::MidiKeys::default();
        keys.0[(KEY_LOW + a.window as i32 + 5) as usize] = 100;
        a.tick(&Input { midi_keys: keys, ..Default::default() });
        assert!(a.shared.keys[a.window + 5].load(Ordering::Relaxed));
        a.tick(&Input::default());
        a.toggle_grid_mode();
        let mut keys = crate::app::MidiKeys::default();
        keys.0[(BASS_LOW + 4) as usize] = 100;
        a.tick(&Input { midi_keys: keys, ..Default::default() });
        assert_eq!(a.shared.bass_key.load(Ordering::Relaxed), 4);
    }

    /// Magnitude of one frequency in a signal (a single DFT bin).
    fn bin(x: &[f32], hz: f32, sr: f32) -> f32 {
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (n, v) in x.iter().enumerate() {
            let ph = TAU * hz * n as f32 / sr;
            re += v * ph.cos();
            im += v * ph.sin();
        }
        (re * re + im * im).sqrt() / x.len() as f32
    }

    #[test]
    fn the_kick_rings_low_and_dies_away_and_the_hats_are_bright() {
        let mut d = Drums::default();
        d.retune(48_000.0);
        d.kick.tick(1.0);
        let ring: Vec<f32> = (0..4800).map(|_| d.kick.tick(0.0)).collect();
        let crossings = ring.windows(2).filter(|w| w[0].signum() != w[1].signum()).count();
        let hz = crossings as f32 / 2.0 / 0.1;
        assert!((hz - KICK_HZ).abs() < 10.0, "kick rings at {hz} Hz");
        let late: Vec<f32> = (0..4800).map(|_| d.kick.tick(0.0)).collect();
        assert!(rms(&late) < rms(&ring) * 0.5, "the ring decays");
        let mut noise = 1u32;
        let hiss: Vec<f32> = (0..4800)
            .map(|_| {
                noise ^= noise << 13;
                noise ^= noise >> 17;
                noise ^= noise << 5;
                let n = (noise as f32 / u32::MAX as f32) * 2.0 - 1.0;
                let thin = d.hat_hp[0].tick(n);
                d.hat_hp[1].tick(thin)
            })
            .collect();
        let zc = hiss.windows(2).filter(|w| w[0].signum() != w[1].signum()).count() as f32 / 2.0 / 0.1;
        assert!(zc > 6000.0, "hat noise is high-passed (~{zc} Hz)");
    }

    #[test]
    fn the_bass_is_a_filtered_square_round_up_top() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.shared.accomp.set(1.0);
        a.toggle_grid_mode();
        a.tick(&pads(&[0])); // C2
        render(&mut p, 10);
        let x = render(&mut p, 40);
        let f = midi_hz(BASS_LOW as f32);
        let (h1, h3, h5, h7) = (bin(&x, f, 48_000.0), bin(&x, 3.0 * f, 48_000.0), bin(&x, 5.0 * f, 48_000.0), bin(&x, 7.0 * f, 48_000.0));
        assert!(h1 > 0.02, "fundamental present ({h1})");
        assert!(h3 / h1 > 0.15, "a square's third harmonic survives ({})", h3 / h1);
        assert!(h5 / h1 < 0.15, "the low-pass rounds off the fifth ({}, a raw square has 0.2)", h5 / h1);
        assert!(h7 / h1 < 0.06, "and the seventh ({})", h7 / h1);
        assert!(bin(&x, 2.0 * f, 48_000.0) / h1 < 0.05, "a square has no even harmonics");
    }

    #[test]
    fn power_on_tempo_sits_in_the_dancehall_range() {
        let a = app();
        assert!((80.0..=110.0).contains(&a.shared.tempo.get()));
    }

    #[test]
    fn minor_chords_bend_a_recorded_line() {
        assert_eq!(line_interval(4, 0), 4);
        assert_eq!(line_interval(4, 1), 3);
        assert_eq!(line_interval(16, 2), 15);
        assert_eq!(line_interval(-1, 1), -2, "a major 7th below becomes a minor 7th below");
        assert_eq!(line_interval(7, 1), 7, "fifths stay");
    }

    /// Records two notes, then checks the loop plays them back in time
    /// and moves with whatever bass key is held.
    #[test]
    fn record_your_own_line_then_it_loops_and_transposes() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.shared.tempo.set(120.0);
        a.shared.line_bars.store(0, Ordering::Relaxed); // 1 bar
        a.shared.accomp.set(1.0);
        a.toggle_grid_mode(); // BASS pads
        a.set_line_rec(true);
        assert_eq!(a.shared.bass_mode.load(Ordering::Relaxed), BASS_LINE);
        // first key starts the rhythm and lands on step 0
        a.tick(&pads(&[2]));
        assert_eq!(a.running(), Some(true));
        render(&mut p, 4);
        a.tick(&Input::default());
        // 120 BPM sixteenths = 6000 samples a step; wait to around step 8
        render(&mut p, 85);
        a.tick(&pads(&[9]));
        render(&mut p, 4);
        a.tick(&Input::default());
        render(&mut p, 2);
        let line = a.line_view();
        assert_eq!(line.len(), 16);
        assert_eq!(line[0], 2, "first note on the downbeat: {line:?}");
        let second = line.iter().position(|&v| v == 9).expect("second note recorded");
        assert!((7..=9).contains(&second), "second note near step 8: {line:?}");
        assert!(line.contains(&-2), "letting go writes a rest");
        // play it back
        a.set_line_rec(false);
        let mut heard = std::collections::BTreeSet::new();
        for _ in 0..200 {
            render(&mut p, 1);
            heard.insert(a.shared.bass_sounding.load(Ordering::Relaxed));
        }
        assert!(heard.contains(&2) && heard.contains(&9), "loop plays both notes: {heard:?}");
        // hold D#.. two keys up from the line's first note: it moves up two
        a.tick(&pads(&[4]));
        let mut moved = std::collections::BTreeSet::new();
        for _ in 0..200 {
            render(&mut p, 1);
            moved.insert(a.shared.bass_sounding.load(Ordering::Relaxed));
        }
        assert!(moved.contains(&4) && moved.contains(&11), "transposed by the held key: {moved:?}");
        a.clear_line();
        assert!(a.line_view().iter().all(|&v| v == -1));
    }

    #[test]
    fn the_line_saves_and_comes_back() {
        let path = std::env::temp_dir().join(format!("tinkertone-line-{}.json", std::process::id()));
        let mut a = app();
        a.line_path = Some(path.clone());
        a.shared.line_ref.store(3, Ordering::Relaxed);
        a.shared.line[0].store(0, Ordering::Relaxed);
        a.shared.line[4].store(LINE_REST, Ordering::Relaxed);
        a.shared.line[8].store(7, Ordering::Relaxed);
        a.shared.line_dirty.fetch_add(1, Ordering::Relaxed);
        for _ in 0..40 {
            a.tick(&Input::default());
        }
        assert!(path.exists(), "saved after it went quiet");
        let mut b = app();
        b.line_path = Some(path.clone());
        b.load_line();
        let _ = std::fs::remove_file(&path);
        assert_eq!(b.shared.bass_mode.load(Ordering::Relaxed), BASS_LINE);
        let v = b.line_view();
        assert_eq!((v[0], v[4], v[8], v[1]), (3, -2, 10, -1));
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(TinkertoneApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}
