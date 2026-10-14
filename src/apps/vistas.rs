//! Vistas: a generative cinematic-scape instrument, built from scratch.
//!
//! Choose a mood and a key, press play, and Vistas composes evolving ambient
//! music for you: slow, voice-led chord progressions played by five synthesised
//! layers (pad, bass, noise, ambience, shimmer) through a long reverb. It is in
//! the same family as sample-library scape instruments such as Rast Sound's
//! Cinescapes PRO, but nothing here is sampled or copied: every sound is
//! synthesised and the composing engine is this app's own.
//!
//! **The composer.** Each of the ten moods is a real mode (Hopeful = Ionian,
//! Dreamy = Lydian, Peaceful = major pentatonic, Nostalgic = Dorian,
//! Melancholy = Aeolian, Mysterious = harmonic minor, Epic = Mixolydian,
//! Tense = Phrygian dominant, Dark = Phrygian, Ominous = Locrian). Chords are
//! stacked in scale thirds on a scale degree, so every note belongs to the
//! mode. Roots move by a weighted random walk that favours the tonic, fourths,
//! fifths and neighbours and avoids repeating a chord. **Variation** is how many
//! scale degrees may be used, **Complexity** is how many notes a chord has (a
//! bare fifth up to an eleventh chord) and **Duration** is how long each chord
//! lasts. Voicings are led smoothly: each voice moves to the nearest pitch of
//! the next chord, and notes common to both chords simply keep sounding, so
//! changes are never abrupt.
//!
//! Three modes: **Loop** plays one eight-chord progression forever, **Compose**
//! writes a new eight-chord progression every time it comes round, **Evolve**
//! never repeats: the next chord is always chosen live from the last.
//!
//! **The layers.** Each has a Level, a Tone, a Motion (two slow LFOs: one
//! sweeps its filter, the other sways its loudness and pan) and eight Timbre
//! recipes (40 in all, each a parameter set for that layer's synth, not a
//! sample): Pad (detuned saw/triangle through a filter), Bass (a saturated sub
//! drone on the chord root), Noise (noise through band-pass filters tuned to the
//! chord, plus wind), Ambience (slowly breathing additive partials) and Shimmer
//! (random FM pings on notes of the scale).
//!
//! **Space.** A stereo feedback-delay-network reverb (eight lines, Householder
//! mixing, RT60 2-25 s) and a filtered ping-pong delay. Transport is explicit:
//! F3 (or the PLAY pad) starts and stops; stopping lets the layers fade and the
//! reverb ring out.
//!
//! Pads: the MOOD layer picks a mood (ten pads), starts/stops, writes a new
//! progression, skips to the next chord, and picks the mode; the ROOT layer
//! picks the key. A MIDI key sets the key too.

use crate::{
    app::play_kit::{self as kit, pad_rank, KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12},
    util::AtomicF32,
};
use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, PrimitiveStyle, Rectangle},
    text::Text,
};
use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};
use std::sync::{
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "Vistas";

// ---------------------------------------------------------------- controls

const C_MOOD: usize = 0;
const C_ROOT: usize = 1;
const C_MODE: usize = 2;
const C_VARIATION: usize = 3;
const C_COMPLEXITY: usize = 4;
const C_DURATION: usize = 5;
const C_PAD: usize = 6;
const C_BASS: usize = 7;
const C_NOISE: usize = 8;
const C_AMB: usize = 9;
const C_SHIM: usize = 10;
const C_REVERB: usize = 11;
const C_SPACE: usize = 12;
const C_DELAY: usize = 13;
const C_MASTER: usize = 14;
const C_WIDTH: usize = 15;
const C_SCAPE: usize = 16;
/// Per layer (pad, bass, noise, ambience, shimmer): tone, motion, timbre.
const C_LAYER0: usize = 17;
const C_DENSITY: usize = 32;
const C_OCTAVE: usize = 33;
const N: usize = 34;

const LAYERS: usize = 5;
const LAYER_NAMES: [&str; LAYERS] = ["Pad", "Bass", "Noise", "Ambience", "Shimmer"];
const LEVEL_CTL: [usize; LAYERS] = [C_PAD, C_BASS, C_NOISE, C_AMB, C_SHIM];

const fn tone_ctl(layer: usize) -> usize {
    C_LAYER0 + layer * 3
}
const fn motion_ctl(layer: usize) -> usize {
    C_LAYER0 + layer * 3 + 1
}
const fn timbre_ctl(layer: usize) -> usize {
    C_LAYER0 + layer * 3 + 2
}

struct Ctl {
    name: &'static str,
    steps: usize,
    default: f32,
}

const fn c(name: &'static str, steps: usize, default: f32) -> Ctl {
    Ctl { name, steps, default }
}

const CTLS: [Ctl; N] = [
    c("Mood", 10, 4.0 / 9.0),
    c("Key", 12, 0.0),
    c("Mode", 3, 0.5),
    c("Variation", 0, 0.5),
    c("Complexity", 0, 0.5),
    c("Duration", 0, 0.45),
    c("Pad", 0, 0.75),
    c("Bass", 0, 0.6),
    c("Noise", 0, 0.35),
    c("Ambience", 0, 0.55),
    c("Shimmer", 0, 0.4),
    c("Reverb", 0, 0.55),
    c("Space", 0, 0.7),
    c("Delay", 0, 0.25),
    c("Master", 0, 0.8),
    c("Width", 0, 0.8),
    c("Scape", 12, 0.0),
    c("Pad Tone", 0, 0.5),
    c("Pad Motion", 0, 0.4),
    c("Pad Timbre", 8, 0.0),
    c("Bass Tone", 0, 0.5),
    c("Bass Motion", 0, 0.3),
    c("Bass Timbre", 8, 0.0),
    c("Noise Tone", 0, 0.5),
    c("Noise Motion", 0, 0.5),
    c("Noise Timbre", 8, 0.0),
    c("Amb Tone", 0, 0.5),
    c("Amb Motion", 0, 0.4),
    c("Amb Timbre", 8, 0.0),
    c("Shim Tone", 0, 0.5),
    c("Shim Motion", 0, 0.3),
    c("Shim Timbre", 8, 0.0),
    c("Density", 0, 0.35),
    c("Octave", 3, 0.5),
];

const MODE_NAMES: [&str; 3] = ["LOOP", "COMPOSE", "EVOLVE"];

/// Each mood is a real mode; the chords are built from its own notes.
const MOODS: [(&str, &[u8]); 10] = [
    ("Hopeful", &[0, 2, 4, 5, 7, 9, 11]),
    ("Dreamy", &[0, 2, 4, 6, 7, 9, 11]),
    ("Peaceful", &[0, 2, 4, 7, 9]),
    ("Nostalgic", &[0, 2, 3, 5, 7, 9, 10]),
    ("Melancholy", &[0, 2, 3, 5, 7, 8, 10]),
    ("Mysterious", &[0, 2, 3, 5, 7, 8, 11]),
    ("Epic", &[0, 2, 4, 5, 7, 9, 10]),
    ("Tense", &[0, 1, 4, 5, 7, 8, 10]),
    ("Dark", &[0, 1, 3, 5, 7, 8, 10]),
    ("Ominous", &[0, 1, 3, 5, 6, 8, 10]),
];

const MOOD_ABBR: [&str; 10] = ["HOPE", "DREM", "PEAC", "NOST", "MELA", "MYST", "EPIC", "TENS", "DARK", "OMIN"];
const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

const TIMBRE_NAMES: [[&str; 8]; LAYERS] = [
    ["Glass", "Velvet", "Ash", "Halo", "Tide", "Dusk", "Cinder", "Aurora"],
    ["Sub", "Cello Drone", "Rumble", "Tape Drone", "Hollow", "Tectonic", "Organ Floor", "Deep Sea"],
    ["Wind", "Surf", "Breath", "Static Choir", "Rain Veil", "Embers", "Whisper", "Storm"],
    ["Glass Organ", "Bowed Air", "Cathedral", "Vapor", "Ice", "Starlight", "Reed", "Mist"],
    ["Bell", "Chime", "Music Box", "Droplet", "Crystal", "Kalimba", "Pluck", "Glint"],
];

/// Modulation inputs: controls they add to.
const CV: [usize; 8] = [C_VARIATION, C_COMPLEXITY, C_DURATION, C_REVERB, C_SPACE, C_DELAY, C_DENSITY, C_WIDTH];
const CV_NAMES: [&str; 8] = ["Variation", "Complexity", "Duration", "Reverb", "Space", "Delay", "Density", "Width"];

fn cv_slot(control: usize) -> Option<usize> {
    CV.iter().position(|&x| x == control)
}

fn step_norm(idx: usize, steps: usize) -> f32 {
    if steps < 2 { 0.0 } else { idx as f32 / (steps - 1) as f32 }
}

fn norm_step(v: f32, steps: usize) -> usize {
    if steps < 2 { 0 } else { (v.clamp(0.0, 1.0) * (steps - 1) as f32).round() as usize }
}

// ---------------------------------------------------------------- scapes (presets)

type ScapeDef = (&'static str, &'static [(usize, f32)]);

const fn mood(i: usize) -> f32 {
    i as f32 / 9.0
}
const fn key(i: usize) -> f32 {
    i as f32 / 11.0
}
const fn timbre(i: usize) -> f32 {
    i as f32 / 7.0
}

/// Original starting points. Anything not listed is the control's default.
const SCAPES: [ScapeDef; 12] = [
    ("Open Sky", &[(C_MOOD, mood(0)), (C_KEY_, key(7)), (C_MODE, 0.5), (C_DURATION, 0.5), (C_VARIATION, 0.45), (C_COMPLEXITY, 0.45), (C_REVERB, 0.6), (C_SPACE, 0.75)]),
    ("Frozen Lake", &[(C_MOOD, mood(2)), (C_KEY_, key(2)), (C_MODE, 0.0), (C_DURATION, 0.62), (C_VARIATION, 0.3), (C_COMPLEXITY, 0.3), (C_PAD, 0.6), (C_BASS, 0.45), (C_NOISE, 0.2), (C_AMB, 0.75), (C_SHIM, 0.55), (timbre_ctl(0), timbre(3)), (timbre_ctl(3), timbre(4)), (C_DENSITY, 0.3), (C_SPACE, 0.85)]),
    ("Last Light", &[(C_MOOD, mood(4)), (C_KEY_, key(9)), (C_DURATION, 0.55), (C_COMPLEXITY, 0.6), (timbre_ctl(0), timbre(1)), (timbre_ctl(1), timbre(1)), (C_SHIM, 0.25)]),
    ("Event Horizon", &[(C_MOOD, mood(9)), (C_KEY_, key(4)), (C_MODE, 1.0), (C_DURATION, 0.75), (C_VARIATION, 0.35), (C_COMPLEXITY, 0.35), (C_PAD, 0.55), (C_BASS, 0.9), (C_NOISE, 0.6), (C_AMB, 0.4), (C_SHIM, 0.15), (timbre_ctl(0), timbre(6)), (timbre_ctl(1), timbre(5)), (timbre_ctl(2), timbre(7)), (C_SPACE, 0.95), (C_REVERB, 0.7)]),
    ("Rising Tide", &[(C_MOOD, mood(6)), (C_KEY_, key(3)), (C_MODE, 1.0), (C_DURATION, 0.4), (C_VARIATION, 0.7), (C_COMPLEXITY, 0.55), (C_PAD, 0.85), (C_BASS, 0.75), (timbre_ctl(0), timbre(4)), (timbre_ctl(1), timbre(3)), (C_SHIM, 0.5)]),
    ("Glass Cathedral", &[(C_MOOD, mood(1)), (C_KEY_, key(5)), (C_DURATION, 0.7), (C_COMPLEXITY, 0.7), (C_PAD, 0.5), (C_AMB, 0.85), (timbre_ctl(3), timbre(2)), (timbre_ctl(0), timbre(0)), (C_SHIM, 0.5), (timbre_ctl(4), timbre(0)), (C_SPACE, 0.9)]),
    ("Night Train", &[(C_MOOD, mood(8)), (C_KEY_, key(0)), (C_MODE, 0.0), (C_DURATION, 0.35), (C_VARIATION, 0.4), (C_COMPLEXITY, 0.35), (C_BASS, 0.85), (C_NOISE, 0.5), (timbre_ctl(1), timbre(2)), (timbre_ctl(2), timbre(1)), (C_AMB, 0.3), (C_SHIM, 0.2), (C_DELAY, 0.4)]),
    ("Dust and Embers", &[(C_MOOD, mood(5)), (C_KEY_, key(11)), (C_DURATION, 0.5), (C_NOISE, 0.55), (timbre_ctl(2), timbre(5)), (timbre_ctl(0), timbre(5)), (C_SHIM, 0.45), (timbre_ctl(4), timbre(5)), (C_DENSITY, 0.5)]),
    ("Old Photographs", &[(C_MOOD, mood(3)), (C_KEY_, key(10)), (C_MODE, 0.0), (C_DURATION, 0.5), (C_VARIATION, 0.5), (timbre_ctl(0), timbre(5)), (timbre_ctl(4), timbre(2)), (C_SHIM, 0.55), (C_DENSITY, 0.4), (C_DELAY, 0.4)]),
    ("Edge of Reason", &[(C_MOOD, mood(7)), (C_KEY_, key(6)), (C_MODE, 1.0), (C_DURATION, 0.3), (C_VARIATION, 0.85), (C_COMPLEXITY, 0.65), (C_PAD, 0.6), (timbre_ctl(0), timbre(2)), (C_NOISE, 0.55), (timbre_ctl(2), timbre(6)), (C_SHIM, 0.3)]),
    ("Deep Blue", &[(C_MOOD, mood(2)), (C_KEY_, key(8)), (C_MODE, 0.5), (C_DURATION, 0.8), (C_VARIATION, 0.2), (C_COMPLEXITY, 0.25), (C_PAD, 0.5), (C_BASS, 0.85), (timbre_ctl(1), timbre(7)), (timbre_ctl(0), timbre(7)), (C_NOISE, 0.45), (timbre_ctl(2), timbre(1)), (C_AMB, 0.7), (timbre_ctl(3), timbre(7)), (C_SHIM, 0.2), (C_SPACE, 0.95)]),
    ("Heroes at Dawn", &[(C_MOOD, mood(6)), (C_KEY_, key(2)), (C_MODE, 0.5), (C_DURATION, 0.45), (C_VARIATION, 0.6), (C_COMPLEXITY, 0.5), (C_PAD, 0.8), (C_BASS, 0.8), (timbre_ctl(0), timbre(7)), (timbre_ctl(1), timbre(1)), (C_AMB, 0.6), (timbre_ctl(3), timbre(5)), (C_SHIM, 0.5), (timbre_ctl(4), timbre(0))]),
];

// The scapes name the "Key" control by a short alias.
const C_KEY_: usize = C_ROOT;

/// Controls a scape leaves alone: they belong to the player.
fn scape_keeps(i: usize) -> bool {
    i == C_SCAPE || i == C_MASTER || i == C_WIDTH || i == C_OCTAVE
}

// ---------------------------------------------------------------- shared state

struct Shared {
    vals: [AtomicF32; N],
    cv: [Arc<AtomicF32>; 8],
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    peak: AtomicF32,
    meters: [AtomicF32; LAYERS],
    running: AtomicBool,
    regen: AtomicBool,
    skip: AtomicBool,
    /// The chord now sounding, for the screen.
    chord: [AtomicU8; 6],
    chord_n: AtomicUsize,
    chord_root: AtomicU8,
    chord_bass: AtomicU8,
    chord_gen: AtomicUsize,
    chord_step: AtomicUsize,
    progress: AtomicF32,
}

impl Shared {
    fn eff(&self, i: usize) -> f32 {
        let mut v = self.vals[i].get();
        if let Some(s) = cv_slot(i) {
            v += self.cv[s].get();
        }
        v.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Default)]
struct LayerP {
    level: f32,
    tone: f32,
    motion: f32,
    timbre: usize,
}

#[derive(Clone, Copy)]
struct Patch {
    mood: usize,
    root: u8,
    mode: usize,
    variation: f32,
    complexity: f32,
    dur_s: f32,
    layers: [LayerP; LAYERS],
    reverb: f32,
    rt60: f32,
    delay: f32,
    width: f32,
    density: f32,
    octave: i32,
}

impl Patch {
    fn read(p: &Shared) -> Patch {
        let e = |i| p.eff(i);
        let mut layers = [LayerP::default(); LAYERS];
        for (l, lp) in layers.iter_mut().enumerate() {
            *lp = LayerP { level: e(LEVEL_CTL[l]), tone: e(tone_ctl(l)), motion: e(motion_ctl(l)), timbre: norm_step(e(timbre_ctl(l)), 8) };
        }
        Patch {
            mood: norm_step(e(C_MOOD), 10),
            root: norm_step(e(C_ROOT), 12) as u8,
            mode: norm_step(e(C_MODE), 3),
            variation: e(C_VARIATION),
            complexity: e(C_COMPLEXITY),
            // 1.5 s to 24 s per chord.
            dur_s: 1.5 * 16.0f32.powf(e(C_DURATION)),
            layers,
            reverb: e(C_REVERB),
            rt60: 2.0 * 12.5f32.powf(e(C_SPACE)),
            delay: e(C_DELAY),
            width: e(C_WIDTH),
            density: e(C_DENSITY),
            octave: norm_step(e(C_OCTAVE), 3) as i32 - 1,
        }
    }
}

// ---------------------------------------------------------------- small DSP parts

fn coef(tau: f32, sr: f32) -> f32 {
    1.0 - (-1.0 / (sr * tau.max(1e-4))).exp()
}

fn hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
    fn signed(&mut self) -> f32 {
        self.next() * 2.0 - 1.0
    }
}

#[derive(Clone, Copy, Default)]
struct Svf {
    s1: f32,
    s2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Svf {
    fn set(&mut self, f: f32, q: f32, sr: f32) {
        let g = (PI * f.clamp(10.0, sr * 0.45) / sr).tan();
        self.k = 1.0 / q.max(0.3);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    /// (low-pass, band-pass with gain 1/k at the centre, high-pass).
    fn run(&mut self, x: f32) -> (f32, f32, f32) {
        let v3 = x - self.s2;
        let v1 = self.a1 * self.s1 + self.a2 * v3;
        let v2 = self.s2 + self.a2 * self.s1 + self.a3 * v3;
        self.s1 = 2.0 * v1 - self.s1;
        self.s2 = 2.0 * v2 - self.s2;
        (v2, v1, x - self.k * v1 - v2)
    }
}

fn poly_blep(t: f32, dt: f32) -> f32 {
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

fn pan_gains(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    (((1.0 - p) * 0.5).sqrt(), ((1.0 + p) * 0.5).sqrt())
}

/// One sounding note of a layer.
#[derive(Clone, Copy)]
struct NV {
    note: u8,
    gate: bool,
    amp: f32,
    ph: [f32; 4],
    lfo: [f32; 4],
}

/// A pool of voices that holds a set of notes. A note already sounding keeps
/// its voice (so notes common to two chords don't restart); notes that leave
/// are released and fade at the layer's own release time.
struct Pool<const M: usize> {
    v: [NV; M],
}

impl<const M: usize> Pool<M> {
    fn new() -> Self {
        let mut v = [NV { note: 0, gate: false, amp: 0.0, ph: [0.0; 4], lfo: [0.0; 4] }; M];
        for (i, x) in v.iter_mut().enumerate() {
            for k in 0..4 {
                x.lfo[k] = ((i * 4 + k) as f32 * 0.37).fract();
                x.ph[k] = ((i * 4 + k) as f32 * 0.61).fract();
            }
        }
        Pool { v }
    }

    fn set(&mut self, notes: &[u8]) {
        for v in self.v.iter_mut() {
            if v.gate && !notes.contains(&v.note) {
                v.gate = false;
            }
        }
        for &n in notes {
            if self.v.iter().any(|v| v.gate && v.note == n) {
                continue;
            }
            let quietest = |pool: &Self, only_free: bool| (0..M).filter(|&i| !only_free || !pool.v[i].gate).min_by(|&a, &b| pool.v[a].amp.total_cmp(&pool.v[b].amp));
            let slot = (0..M).find(|&i| !self.v[i].gate && self.v[i].amp < 1e-3).or_else(|| quietest(self, true)).or_else(|| quietest(self, false));
            if let Some(i) = slot {
                self.v[i].note = n;
                self.v[i].gate = true;
            }
        }
    }

    fn pan(i: usize) -> f32 {
        // A fixed scatter, so neighbouring slots land on different sides.
        ((i * 5 % 7) as f32 / 3.0 - 1.0) * 0.8
    }

}

// ---------------------------------------------------------------- composer

#[derive(Clone, Copy, Default)]
struct Chord {
    notes: [u8; 6],
    n: usize,
    /// Pitch class of the chord's root.
    root_pc: u8,
    bass: u8,
    step: usize,
}

struct Composer {
    rng: Rng,
    seq: [(u8, u8); 8],
    pos: usize,
    last_deg: u8,
    prev: [u8; 6],
    prev_n: usize,
    key: (usize, usize, usize),
    have: bool,
}

impl Composer {
    fn new() -> Composer {
        Composer { rng: Rng(0x9E37_79B9), seq: [(0, 0); 8], pos: 0, last_deg: 0, prev: [0; 6], prev_n: 0, key: (usize::MAX, 0, 0), have: false }
    }

    fn order(len: usize) -> &'static [u8] {
        // Tonic first, then the most usual neighbours, then the colour tones.
        if len >= 7 { &[0, 5, 3, 4, 1, 2, 6] } else { &[0, 3, 2, 4, 1] }
    }

    /// Weighted random step from `cur` within the first `allowed` degrees.
    fn markov(&mut self, cur: u8, len: usize, allowed: usize) -> u8 {
        let order = Self::order(len);
        let mut w = [0.0f32; 7];
        let mut total = 0.0;
        for (i, &d) in order.iter().take(allowed).enumerate() {
            let step = (d as usize + len - cur as usize) % len;
            let mut x = 1.0;
            if d == 0 {
                x += 2.0;
            }
            if (len >= 7 && (step == 3 || step == 4)) || (len < 7 && (step == 2 || step == 3)) {
                x += 1.5;
            }
            if step == 1 || step == len - 1 {
                x += 0.8;
            }
            if d == cur {
                x *= 0.15;
            }
            w[i] = x;
            total += x;
        }
        let mut r = self.rng.next() * total;
        for (i, &d) in order.iter().take(allowed).enumerate() {
            if r < w[i] {
                return d;
            }
            r -= w[i];
        }
        0
    }

    fn flavour(&mut self, variation: f32) -> u8 {
        if self.rng.next() < 0.4 * variation {
            1 + (self.rng.next() < 0.5) as u8
        } else {
            0
        }
    }

    fn gen_seq(&mut self, len: usize, allowed: usize, variation: f32) {
        self.seq[0] = (0, 0);
        for i in 1..8 {
            let prev = self.seq[i - 1].0;
            let d = self.markov(prev, len, allowed);
            let f = self.flavour(variation);
            self.seq[i] = (d, f);
        }
        self.pos = 0;
    }

    fn invalidate(&mut self) {
        self.have = false;
    }

    fn next(&mut self, p: &Patch) -> Chord {
        let scale = MOODS[p.mood].1;
        let len = scale.len();
        let allowed = (2.0 + p.variation * (len as f32 - 2.0)).round().clamp(2.0, len as f32) as usize;
        let n = (2.0 + p.complexity * 4.0).round().clamp(2.0, 6.0) as usize;
        let key = (p.mood, allowed, n);
        if key != self.key || !self.have {
            self.key = key;
            self.gen_seq(len, allowed, p.variation);
            self.have = true;
            self.last_deg = 0;
        }
        let (deg, flav, step) = match p.mode {
            2 => {
                // Evolve: always live, from the last chord.
                let d = if self.pos == 0 && self.last_deg == 0 && self.prev_n == 0 { 0 } else { self.markov(self.last_deg, len, allowed) };
                let f = self.flavour(p.variation);
                let s = self.pos;
                self.pos = (self.pos + 1) % 8;
                (d, f, s)
            }
            _ => {
                if p.mode == 1 && self.pos == 0 && self.prev_n > 0 {
                    // Compose: a fresh progression each time round.
                    self.gen_seq(len, allowed, p.variation);
                }
                let (d, f) = self.seq[self.pos];
                let s = self.pos;
                self.pos = (self.pos + 1) % 8;
                (d, f, s)
            }
        };
        self.last_deg = deg;
        self.build(scale, p.root, deg as usize, flav, n, step)
    }

    fn build(&mut self, scale: &[u8], root: u8, deg: usize, flav: u8, n: usize, step: usize) -> Chord {
        let len = scale.len();
        let mut idx = [0usize, 2, 4, 6, 8, 10];
        if n == 2 {
            idx[1] = 4;
        } else if flav == 1 {
            idx[1] = 1;
        } else if flav == 2 {
            idx[1] = 3;
        }
        let root_pc = (root + scale[deg % len]) % 12;
        let mut pcs = [0u8; 6];
        for k in 0..n {
            let i = deg + idx[k];
            pcs[k] = (root + scale[i % len]) % 12;
        }
        let mut out = [0u8; 6];
        if self.prev_n == 0 {
            // First chord: close position above the root, near the middle.
            let mut cur = 48 + (root_pc as i32 - 48).rem_euclid(12);
            for k in 0..n {
                if k > 0 {
                    cur += (pcs[k] as i32 - cur).rem_euclid(12).max(1);
                }
                out[k] = cur as u8;
            }
            while out[n - 1] > 79 {
                for o in out.iter_mut().take(n) {
                    *o -= 12;
                }
            }
        } else {
            for k in 0..n {
                let target = if k < self.prev_n { self.prev[k] as i32 } else { out[k - 1] as i32 + 3 };
                let pc = pcs[k] as i32;
                // The octave of this pitch class nearest the voice it replaces.
                let mut best = pc + 12 * ((target - pc) as f32 / 12.0).round() as i32;
                // Stay inside 45..=84 by whole octaves, so the pitch class is kept.
                while best < 45 {
                    best += 12;
                }
                while best > 84 {
                    best -= 12;
                }
                while out[..k].contains(&(best as u8)) {
                    best += if best + 12 <= 84 { 12 } else { -12 };
                }
                out[k] = best as u8;
            }
            out[..n].sort_unstable();
        }
        self.prev = out;
        self.prev_n = n;
        let bass = 28 + (root_pc as i32 - 28).rem_euclid(12) as u8;
        Chord { notes: out, n, root_pc, bass, step }
    }
}

/// A chord's name from its notes, e.g. "Dm9", "Fsus2", "Bdim".
fn chord_name(root: u8, notes: &[u8]) -> String {
    let has = |i: u8| notes.iter().any(|&n| (n + 120 - root) % 12 == i);
    let third = if has(4) { "" } else if has(3) { "m" } else if has(2) { "sus2" } else if has(5) { "sus4" } else { "5" };
    let mut s = format!("{}{}", NOTE_NAMES[root as usize % 12], third);
    if has(3) && has(6) && !has(7) {
        s = format!("{}dim", NOTE_NAMES[root as usize % 12]);
    } else if has(8) && !has(7) && has(4) {
        s.push_str("aug");
    }
    // A ninth on top of a seventh is named as one chord ("Dm9", "Fmaj9").
    let ninth = has(2) && third != "sus2" && third != "5";
    match (has(11), has(10), ninth) {
        (true, _, true) => s.push_str("maj9"),
        (true, _, false) => s.push_str("maj7"),
        (false, true, true) => s.push('9'),
        (false, true, false) => s.push('7'),
        (false, false, true) => s.push_str("add9"),
        _ => {}
    }
    s
}

// ---------------------------------------------------------------- layers

const CH: usize = 32;

/// What one chunk of one layer needs from the engine.
struct Ctx<'a> {
    sr: f32,
    n: usize,
    p: &'a LayerP,
}

/// Pad, bass, ambience and noise each have two slow LFOs: A sweeps the
/// filter, B sways loudness and pan.
struct Lfos {
    a: f32,
    b: f32,
}

impl Lfos {
    fn step(&mut self, ra: f32, rb: f32, n: usize, sr: f32) -> (f32, f32) {
        self.a = (self.a + ra * n as f32 / sr).fract();
        self.b = (self.b + rb * n as f32 / sr).fract();
        ((self.a * TAU).sin(), (self.b * TAU).sin())
    }
}

// (saw mix, detune cents, cutoff Hz, Q, attack s, release s, sub level)
const PAD_T: [[f32; 7]; 8] = [
    [0.3, 7.0, 3200.0, 0.8, 3.0, 4.0, 0.0],
    [0.7, 9.0, 1100.0, 0.9, 4.0, 5.0, 0.3],
    [1.0, 14.0, 700.0, 1.1, 5.0, 6.0, 0.2],
    [0.15, 5.0, 5200.0, 0.7, 2.5, 4.0, 0.0],
    [0.8, 12.0, 1800.0, 1.5, 6.0, 7.0, 0.1],
    [0.6, 8.0, 900.0, 0.8, 3.5, 6.0, 0.5],
    [1.0, 20.0, 450.0, 1.8, 4.0, 8.0, 0.4],
    [0.5, 11.0, 2600.0, 1.2, 4.5, 5.5, 0.0],
];

struct PadLayer {
    pool: Pool<16>,
    lfo: Lfos,
    f: [Svf; 2],
}

impl PadLayer {
    fn render(&mut self, cx: &Ctx, l: &mut [f32; CH], r: &mut [f32; CH]) {
        let t = PAD_T[cx.p.timbre];
        let sr = cx.sr;
        let (la, lb) = self.lfo.step(0.071, 0.113, cx.n, sr);
        let cutoff = t[2] * ((cx.p.tone - 0.5) * 4.0 + cx.p.motion * la * 1.5).exp2();
        for f in self.f.iter_mut() {
            f.set(cutoff.clamp(80.0, 16_000.0), t[3], sr);
        }
        let (ca, cr) = (coef(t[4] / 3.0, sr), coef(t[5] / 3.0, sr));
        let det = (t[1] / 1200.0).exp2() - 1.0;
        let sway = lb * cx.p.motion * 0.5;
        for (i, v) in self.pool.v.iter_mut().enumerate() {
            if !v.gate && v.amp < 1e-5 {
                v.amp = 0.0;
                continue;
            }
            let (gl, gr) = pan_gains(Pool::<16>::pan(i) + sway);
            let dt = hz(v.note as f32) / sr;
            let (mut a0, mut a1, mut a2) = (v.ph[0], v.ph[1], v.ph[2]);
            let mut amp = v.amp;
            for k in 0..cx.n {
                amp += (if v.gate { 1.0 } else { 0.0 } - amp) * if v.gate { ca } else { cr };
                let (d1, d2) = (dt * (1.0 + det), dt * (1.0 - det));
                let saw = ((2.0 * a0 - 1.0 - poly_blep(a0, d1)) + (2.0 * a1 - 1.0 - poly_blep(a1, d2))) * 0.5;
                let tri = ((1.0 - 4.0 * (a0 - 0.5).abs()) + (1.0 - 4.0 * (a1 - 0.5).abs())) * 0.5;
                let s = (saw * t[0] + tri * (1.0 - t[0]) + (a2 * TAU).sin() * t[6]) * amp * 0.16;
                l[k] += s * gl;
                r[k] += s * gr;
                a0 = (a0 + d1).fract();
                a1 = (a1 + d2).fract();
                a2 = (a2 + dt * 0.5).fract();
            }
            v.ph[0] = a0;
            v.ph[1] = a1;
            v.ph[2] = a2;
            v.amp = amp;
        }
        for k in 0..cx.n {
            l[k] = self.f[0].run(l[k]).0;
            r[k] = self.f[1].run(r[k]).0;
        }
    }
}

// (drive, triangle mix, 2nd harmonic, second voice interval, attack s, release s, cutoff Hz)
const BASS_T: [[f32; 7]; 8] = [
    [1.0, 0.0, 0.0, 0.0, 3.0, 4.0, 400.0],
    [3.0, 0.5, 0.35, 7.0, 3.5, 5.0, 900.0],
    [4.0, 0.8, 0.2, 12.0, 4.0, 6.0, 300.0],
    [2.0, 0.3, 0.5, 12.0, 2.5, 4.0, 700.0],
    [1.5, 0.0, 0.8, 7.0, 3.0, 5.0, 1200.0],
    [5.0, 0.9, 0.1, 0.0, 5.0, 8.0, 220.0],
    [1.2, 0.2, 0.9, 12.0, 2.5, 3.5, 1500.0],
    [2.5, 0.4, 0.15, 7.0, 6.0, 9.0, 260.0],
];

struct BassLayer {
    pool: Pool<4>,
    lfo: Lfos,
    f: Svf,
}

impl BassLayer {
    fn render(&mut self, cx: &Ctx, l: &mut [f32; CH], r: &mut [f32; CH]) {
        let t = BASS_T[cx.p.timbre];
        let sr = cx.sr;
        let (la, lb) = self.lfo.step(0.043, 0.067, cx.n, sr);
        self.f.set((t[6] * ((cx.p.tone - 0.5) * 3.0 + cx.p.motion * la).exp2()).clamp(40.0, 8000.0), 0.7, sr);
        let (ca, cr) = (coef(t[4] / 3.0, sr), coef(t[5] / 3.0, sr));
        let trem = 1.0 - cx.p.motion * 0.35 * (0.5 + 0.5 * lb);
        let drive_norm = 1.0 / t[0].tanh();
        for v in self.pool.v.iter_mut() {
            if !v.gate && v.amp < 1e-5 {
                v.amp = 0.0;
                continue;
            }
            let dt = hz(v.note as f32) / sr;
            let mut ph = v.ph[0];
            let mut amp = v.amp;
            for k in 0..cx.n {
                amp += (if v.gate { 1.0 } else { 0.0 } - amp) * if v.gate { ca } else { cr };
                let sine = (ph * TAU).sin();
                let tri = 1.0 - 4.0 * (ph - 0.5).abs();
                let base = sine * (1.0 - t[1]) + tri * t[1];
                let s = (t[0] * base).tanh() * drive_norm + (2.0 * ph * TAU).sin() * t[2] * 0.4;
                let s = s * amp * trem * 0.34;
                l[k] += s;
                r[k] += s;
                ph = (ph + dt).fract();
            }
            v.ph[0] = ph;
            v.amp = amp;
        }
        for k in 0..cx.n {
            l[k] = self.f.run(l[k]).0;
            r[k] = l[k];
        }
    }
}

// (tonal weight, band Q, wind cutoff Hz, wind weight, high-pass Hz, tremolo depth)
const NOISE_T: [[f32; 6]; 8] = [
    [0.2, 8.0, 1400.0, 0.8, 200.0, 0.25],
    [0.1, 5.0, 700.0, 1.0, 80.0, 0.6],
    [0.7, 14.0, 2500.0, 0.3, 300.0, 0.2],
    [1.0, 30.0, 4000.0, 0.1, 500.0, 0.15],
    [0.0, 3.0, 7000.0, 0.9, 2000.0, 0.1],
    [0.3, 10.0, 1800.0, 0.5, 600.0, 0.9],
    [0.5, 22.0, 5000.0, 0.4, 1500.0, 0.3],
    [0.1, 4.0, 500.0, 1.2, 60.0, 0.8],
];

struct NoiseLayer {
    rng: Rng,
    bands: [Svf; 4],
    band_f: [f32; 4],
    band_t: [f32; 4],
    wind_lp: [Svf; 2],
    wind_hp: [Svf; 2],
    gate: bool,
    amp: f32,
    lfo: Lfos,
}

impl NoiseLayer {
    fn render(&mut self, cx: &Ctx, l: &mut [f32; CH], r: &mut [f32; CH]) {
        let t = NOISE_T[cx.p.timbre];
        let sr = cx.sr;
        let (la, lb) = self.lfo.step(0.057, 0.131, cx.n, sr);
        let tone = ((cx.p.tone - 0.5) * 3.0).exp2();
        for i in 0..4 {
            // Retune slowly so a chord change glides.
            self.band_f[i] += (self.band_t[i] - self.band_f[i]) * 0.01;
            self.bands[i].set((self.band_f[i] * tone * (cx.p.motion * la * 0.5).exp2()).clamp(100.0, 14_000.0), t[1], sr);
        }
        for i in 0..2 {
            self.wind_lp[i].set((t[2] * tone * (cx.p.motion * la * 1.2).exp2()).clamp(60.0, 16_000.0), 0.6, sr);
            self.wind_hp[i].set(t[4], 0.7, sr);
        }
        let ca = coef(if self.gate { 4.0 } else { 6.0 } / 3.0, sr);
        let trem = 1.0 - t[5] * (0.3 + 0.7 * cx.p.motion) * (0.5 + 0.5 * lb);
        // A narrow band-pass keeps only about sqrt(bandwidth / Nyquist) of white
        // noise, so each band is scaled back up to a steady level.
        let norm: Vec<f32> = (0..4).map(|i| self.bands[i].k * (t[1] * sr * 0.5 / self.band_f[i].max(100.0)).sqrt() * 0.5).collect();
        for k in 0..cx.n {
            self.amp += (if self.gate { 1.0 } else { 0.0 } - self.amp) * ca;
            let (n1, n2) = (self.rng.signed(), self.rng.signed());
            let mut sl = self.wind_hp[0].run(self.wind_lp[0].run(n1).0).2 * t[3] * 0.9;
            let mut sr_ = self.wind_hp[1].run(self.wind_lp[1].run(n2).0).2 * t[3] * 0.9;
            for i in 0..4 {
                let x = if i % 2 == 0 { n1 } else { n2 };
                let (_, bp, _) = self.bands[i].run(x);
                let y = bp * norm[i] * t[0] * 0.5;
                if i % 2 == 0 {
                    sl += y;
                } else {
                    sr_ += y;
                }
            }
            l[k] += sl * self.amp * trem * 0.22;
            r[k] += sr_ * self.amp * trem * 0.22;
        }
    }
}

// (partial weights x3, LFO rate Hz, detune cents, attack s, release s, cutoff Hz)
const AMB_T: [[f32; 7]; 8] = [
    [1.0, 0.5, 0.35, 0.12, 4.0, 3.5, 5.0],
    [1.0, 0.8, 0.5, 0.2, 6.0, 5.0, 6.0],
    [1.0, 0.7, 0.2, 0.08, 3.0, 6.0, 8.0],
    [0.8, 0.3, 0.6, 0.25, 9.0, 4.0, 6.0],
    [0.6, 0.9, 0.7, 0.15, 5.0, 3.0, 5.0],
    [0.4, 0.6, 1.0, 0.3, 7.0, 4.0, 6.0],
    [1.0, 0.2, 0.7, 0.18, 4.0, 2.5, 4.5],
    [0.7, 0.5, 0.4, 0.1, 12.0, 7.0, 9.0],
];
const AMB_CUTOFF: [f32; 8] = [6000.0, 3500.0, 4500.0, 5500.0, 9000.0, 10000.0, 3000.0, 2500.0];

struct AmbLayer {
    pool: Pool<16>,
    lfo: Lfos,
    f: [Svf; 2],
}

impl AmbLayer {
    fn render(&mut self, cx: &Ctx, l: &mut [f32; CH], r: &mut [f32; CH]) {
        let t = AMB_T[cx.p.timbre];
        let sr = cx.sr;
        let (la, lb) = self.lfo.step(0.05, 0.09, cx.n, sr);
        let cutoff = AMB_CUTOFF[cx.p.timbre] * ((cx.p.tone - 0.5) * 3.5 + cx.p.motion * la * 1.2).exp2();
        for f in self.f.iter_mut() {
            f.set(cutoff.clamp(200.0, 16_000.0), 0.7, sr);
        }
        let (ca, cr) = (coef(t[5] / 3.0, sr), coef(t[6] / 3.0, sr));
        let sway = lb * cx.p.motion * 0.4;
        let det = [1.0, (t[4] / 1200.0).exp2(), 1.0 / (t[4] / 1200.0).exp2()];
        for (i, v) in self.pool.v.iter_mut().enumerate() {
            if !v.gate && v.amp < 1e-5 {
                v.amp = 0.0;
                continue;
            }
            let (gl, gr) = pan_gains(Pool::<16>::pan(i + 3) + sway);
            let f0 = hz(v.note as f32 + 12.0) / sr;
            let mut amp = v.amp;
            let mut ph = v.ph;
            let mut lf = v.lfo;
            for k in 0..cx.n {
                amp += (if v.gate { 1.0 } else { 0.0 } - amp) * if v.gate { ca } else { cr };
                let mut s = 0.0;
                for p in 0..3 {
                    // Each partial breathes on its own slow cycle.
                    let breath = 0.6 + 0.4 * (lf[p] * TAU).sin();
                    s += (ph[p] * TAU).sin() * t[p] * breath;
                    ph[p] = (ph[p] + f0 * (p + 1) as f32 * det[p]).fract();
                    lf[p] = (lf[p] + t[3] * (0.7 + 0.3 * p as f32) / sr).fract();
                }
                let s = s * amp * 0.06;
                l[k] += s * gl;
                r[k] += s * gr;
            }
            v.amp = amp;
            v.ph = ph;
            v.lfo = lf;
        }
        for k in 0..cx.n {
            l[k] = self.f[0].run(l[k]).0;
            r[k] = self.f[1].run(r[k]).0;
        }
    }
}

// (modulator ratio, index, decay s, pitch drop)
const SHIM_T: [[f32; 4]; 8] = [
    [3.5, 1.2, 5.0, 0.0],
    [2.76, 0.9, 4.0, 0.0],
    [2.0, 0.35, 2.2, 0.0],
    [1.5, 2.0, 0.9, 0.4],
    [5.04, 0.8, 6.0, 0.0],
    [4.0, 0.6, 1.6, 0.0],
    [1.0, 1.5, 1.2, 0.0],
    [7.0, 0.5, 3.0, 0.0],
];

#[derive(Clone, Copy, Default)]
struct Ping {
    on: bool,
    f: f32,
    pc: f32,
    pm: f32,
    env: f32,
    dec: f32,
    idx: f32,
    ratio: f32,
    drop: f32,
    pan: f32,
}

struct ShimLayer {
    rng: Rng,
    pings: [Ping; 10],
    pitches: [u8; 24],
    n_pitches: usize,
    gate: bool,
    lfo: Lfos,
    f: [Svf; 2],
}

impl ShimLayer {
    fn render(&mut self, cx: &Ctx, density: f32, l: &mut [f32; CH], r: &mut [f32; CH]) {
        let t = SHIM_T[cx.p.timbre];
        let sr = cx.sr;
        let (la, lb) = self.lfo.step(0.06, 0.1, cx.n, sr);
        self.f[0].set((9000.0 * ((cx.p.tone - 0.5) * 3.0 + cx.p.motion * la).exp2()).clamp(500.0, 16_000.0), 0.7, sr);
        self.f[1] = Svf { s1: self.f[1].s1, s2: self.f[1].s2, ..self.f[0] };
        // Pings per second, from one every ten seconds to about two a second.
        let rate = 0.1 * 20.0f32.powf(density);
        if self.gate && self.n_pitches > 0 && self.rng.next() < rate * cx.n as f32 / sr {
            let pitch = self.pitches[(self.rng.next() * self.n_pitches as f32) as usize % self.n_pitches] as f32;
            let slot = self.pings.iter().position(|p| !p.on).unwrap_or(0);
            self.pings[slot] = Ping { on: true, f: hz(pitch), pc: 0.0, pm: 0.0, env: 0.7 + 0.3 * self.rng.next(), dec: (-1.0 / (t[2] * sr)).exp(), idx: t[1], ratio: t[0], drop: t[3], pan: self.rng.signed() * 0.8 + lb * cx.p.motion * 0.3 };
        }
        let mut sl = [0.0f32; CH];
        let mut sr_ = [0.0f32; CH];
        for p in self.pings.iter_mut().filter(|p| p.on) {
            let (gl, gr) = pan_gains(p.pan);
            for k in 0..cx.n {
                // A short soft attack, then an exponential ring.
                p.env *= p.dec;
                let x = (p.pc * TAU + p.idx * p.env * (p.pm * TAU).sin()).sin() * p.env * 0.13;
                sl[k] += x * gl;
                sr_[k] += x * gr;
                p.pc = (p.pc + p.f / sr).fract();
                p.pm = (p.pm + p.f * p.ratio / sr).fract();
            }
            p.f *= 1.0 - p.drop * cx.n as f32 / sr;
            if p.env < 1e-4 {
                p.on = false;
            }
        }
        for k in 0..cx.n {
            l[k] += self.f[0].run(sl[k]).0;
            r[k] += self.f[1].run(sr_[k]).0;
        }
    }
}

// ---------------------------------------------------------------- effects

struct Reverb {
    lines: Vec<Vec<f32>>,
    pos: usize,
    len: [usize; 8],
    damp: [f32; 8],
    lfo: [f32; 8],
    pre: Vec<f32>,
    pre_pos: usize,
    sr: f32,
    g: [f32; 8],
    last_rt: f32,
}

impl Reverb {
    fn new() -> Reverb {
        Reverb { lines: vec![vec![0.0; 16384]; 8], pos: 0, len: [0; 8], damp: [0.0; 8], lfo: [0.0; 8], pre: vec![0.0; 8192], pre_pos: 0, sr: 0.0, g: [0.0; 8], last_rt: -1.0 }
    }

    fn setup(&mut self, sr: f32, rt60: f32) {
        if sr != self.sr {
            self.sr = sr;
            // Mutually prime lengths, 35-85 ms at 48 kHz.
            const BASE: [f32; 8] = [1687.0, 1999.0, 2333.0, 2683.0, 3037.0, 3413.0, 3797.0, 4177.0];
            for i in 0..8 {
                self.len[i] = ((BASE[i] * sr / 48_000.0) as usize).min(16384 - 64);
            }
            self.last_rt = -1.0;
        }
        if (rt60 - self.last_rt).abs() > 1e-3 {
            self.last_rt = rt60;
            for i in 0..8 {
                // Gain per pass so the loop decays 60 dB in `rt60` seconds.
                self.g[i] = 10.0f32.powf(-3.0 * self.len[i] as f32 / (sr * rt60));
            }
        }
    }

    fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mask = 16383usize;
        let mono = (l + r) * 0.5;
        self.pre[self.pre_pos] = mono;
        let pre = self.pre[(self.pre_pos + self.pre.len() - (0.03 * self.sr) as usize % self.pre.len()) % self.pre.len()];
        self.pre_pos = (self.pre_pos + 1) % self.pre.len();
        let mut y = [0.0f32; 8];
        for i in 0..8 {
            self.lfo[i] = (self.lfo[i] + (0.07 + 0.013 * i as f32) / self.sr).fract();
            let d = self.len[i] as f32 + 6.0 * (self.lfo[i] * TAU).sin();
            let rp = self.pos as f32 + 16384.0 - d;
            let i0 = rp.floor();
            let fr = rp - i0;
            let a = self.lines[i][(i0 as usize) & mask];
            let b = self.lines[i][(i0 as usize + 1) & mask];
            let tap = a + (b - a) * fr;
            // High frequencies die faster than lows, as in a real hall.
            self.damp[i] += (tap - self.damp[i]) * 0.42;
            y[i] = self.damp[i];
        }
        let sum: f32 = y.iter().sum();
        let h = sum * (2.0 / 8.0);
        for i in 0..8 {
            let mix = (y[i] - h) * self.g[i];
            let inp = if i % 2 == 0 { l * 0.5 + pre * 0.3 } else { r * 0.5 + pre * 0.3 };
            self.lines[i][self.pos] = mix + inp * if i % 4 < 2 { 1.0 } else { -1.0 };
        }
        self.pos = (self.pos + 1) & mask;
        let (mut ol, mut or) = (0.0, 0.0);
        for i in 0..8 {
            if i % 2 == 0 {
                ol += y[i] * if i % 4 == 0 { 1.0 } else { -1.0 };
            } else {
                or += y[i] * if i % 4 == 1 { 1.0 } else { -1.0 };
            }
        }
        (ol * 0.35, or * 0.35)
    }
}

struct PingPong {
    l: Vec<f32>,
    r: Vec<f32>,
    pos: usize,
    lp: [f32; 2],
}

impl PingPong {
    fn new() -> PingPong {
        PingPong { l: vec![0.0; 1 << 18], r: vec![0.0; 1 << 18], pos: 0, lp: [0.0; 2] }
    }

    fn process(&mut self, l: f32, r: f32, sr: f32) -> (f32, f32) {
        let mask = (1 << 18) - 1;
        let (tl, tr) = ((0.43 * sr) as usize, (0.64 * sr) as usize);
        let dl = self.l[(self.pos + (1 << 18) - tl) & mask];
        let dr = self.r[(self.pos + (1 << 18) - tr) & mask];
        // Feedback is darkened each pass and crosses sides.
        self.lp[0] += (dl - self.lp[0]) * 0.3;
        self.lp[1] += (dr - self.lp[1]) * 0.3;
        self.l[self.pos] = (l + self.lp[1] * 0.5).clamp(-4.0, 4.0);
        self.r[self.pos] = (r + self.lp[0] * 0.5).clamp(-4.0, 4.0);
        self.pos = (self.pos + 1) & mask;
        (dl, dr)
    }
}

// ---------------------------------------------------------------- engine

struct Engine {
    sr: f32,
    comp: Composer,
    running: bool,
    timer: f32,
    chord: Chord,
    pad: PadLayer,
    bass: BassLayer,
    noise: NoiseLayer,
    amb: AmbLayer,
    shim: ShimLayer,
    gains: [f32; LAYERS],
    meter: [f32; LAYERS],
    reverb: Reverb,
    delay: PingPong,
    chord_changed: bool,
}

impl Engine {
    fn new() -> Engine {
        let lfo = |a: f32, b: f32| Lfos { a, b };
        Engine {
            sr: 48_000.0,
            comp: Composer::new(),
            running: false,
            timer: 0.0,
            chord: Chord::default(),
            pad: PadLayer { pool: Pool::new(), lfo: lfo(0.0, 0.3), f: [Svf::default(); 2] },
            bass: BassLayer { pool: Pool::new(), lfo: lfo(0.2, 0.6), f: Svf::default() },
            noise: NoiseLayer { rng: Rng(0xC0FFEE11), bands: [Svf::default(); 4], band_f: [800.0; 4], band_t: [800.0; 4], wind_lp: [Svf::default(); 2], wind_hp: [Svf::default(); 2], gate: false, amp: 0.0, lfo: lfo(0.5, 0.1) },
            amb: AmbLayer { pool: Pool::new(), lfo: lfo(0.7, 0.2), f: [Svf::default(); 2] },
            shim: ShimLayer { rng: Rng(0xBADC0DE5), pings: [Ping::default(); 10], pitches: [0; 24], n_pitches: 0, gate: false, lfo: lfo(0.4, 0.9), f: [Svf::default(); 2] },
            gains: [0.0; LAYERS],
            meter: [0.0; LAYERS],
            reverb: Reverb::new(),
            delay: PingPong::new(),
            chord_changed: false,
        }
    }

    fn silence(&mut self) {
        self.pad.pool.set(&[]);
        self.bass.pool.set(&[]);
        self.amb.pool.set(&[]);
        self.noise.gate = false;
        self.shim.gate = false;
    }

    fn set_running(&mut self, on: bool) {
        if on == self.running {
            return;
        }
        self.running = on;
        if on {
            self.timer = 0.0;
            self.comp.invalidate();
            self.comp.prev_n = 0;
            self.comp.pos = 0;
        } else {
            self.silence();
        }
    }

    fn new_chord(&mut self, p: &Patch) {
        let c = self.comp.next(p);
        self.chord = c;
        self.chord_changed = true;
        let up = |n: u8, extra: i32| (n as i32 + extra * 12 + p.octave * 12).clamp(24, 108) as u8;
        let pad: Vec<u8> = c.notes[..c.n].iter().map(|&n| up(n, 0)).collect();
        self.pad.pool.set(&pad);
        let second = BASS_T[p.layers[1].timbre][3] as u8;
        let bass: Vec<u8> = if second == 0 { vec![c.bass] } else { vec![c.bass, c.bass + second] };
        self.bass.pool.set(&bass);
        let amb: Vec<u8> = c.notes[..c.n].iter().map(|&n| up(n, 0)).collect();
        self.amb.pool.set(&amb);
        // Noise bands sit on the chord's own notes, two octaves up.
        for i in 0..4 {
            let n = c.notes[i % c.n] as f32 + 24.0;
            self.noise.band_t[i] = hz(n);
        }
        self.noise.gate = true;
        // Shimmer plays the mode's notes in the octaves above, chord tones twice.
        let scale = MOODS[p.mood].1;
        let mut count = 0;
        for &n in &c.notes[..c.n] {
            for oct in [72, 84] {
                if count < 24 {
                    self.shim.pitches[count] = (oct + (n % 12)) as u8;
                    count += 1;
                }
            }
        }
        for &s in scale {
            if count < 24 {
                self.shim.pitches[count] = 72 + ((p.root + s) % 12);
                count += 1;
            }
        }
        self.shim.n_pitches = count;
        self.shim.gate = true;
    }

    fn render(&mut self, p: &Patch, out_l: &mut [f32], out_r: &mut [f32]) {
        let frames = out_l.len();
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(CH);
            self.chunk(p, n, &mut out_l[done..done + n], &mut out_r[done..done + n]);
            done += n;
        }
    }

    fn chunk(&mut self, p: &Patch, n: usize, out_l: &mut [f32], out_r: &mut [f32]) {
        let sr = self.sr;
        if self.running {
            self.timer -= n as f32;
            if self.timer <= 0.0 {
                self.new_chord(p);
                self.timer += p.dur_s * sr;
            }
        }
        let mut bufs = [[[0.0f32; CH]; 2]; LAYERS];
        {
            let cx = |i: usize| Ctx { sr, n, p: &p.layers[i] };
            let [[l0, r0], [l1, r1], [l2, r2], [l3, r3], [l4, r4]] = &mut bufs;
            self.pad.render(&cx(0), l0, r0);
            self.bass.render(&cx(1), l1, r1);
            self.noise.render(&cx(2), l2, r2);
            self.amb.render(&cx(3), l3, r3);
            self.shim.render(&cx(4), p.density, l4, r4);
        }
        let mut mix_l = [0.0f32; CH];
        let mut mix_r = [0.0f32; CH];
        for (i, b) in bufs.iter().enumerate() {
            let target = p.layers[i].level * p.layers[i].level * 1.6;
            let (g0, g1) = (self.gains[i], target);
            self.gains[i] = target;
            let mut peak = 0.0f32;
            for k in 0..n {
                let g = g0 + (g1 - g0) * (k + 1) as f32 / n as f32;
                mix_l[k] += b[0][k] * g;
                mix_r[k] += b[1][k] * g;
                peak = peak.max(b[0][k].abs().max(b[1][k].abs()) * g);
            }
            self.meter[i] = peak;
        }
        self.reverb.setup(sr, p.rt60);
        for k in 0..n {
            let (l, r) = (mix_l[k], mix_r[k]);
            let (dl, dr) = self.delay.process(l, r, sr);
            let (wl, wr) = self.reverb.process(l + dl * p.delay * 0.5, r + dr * p.delay * 0.5);
            let mut ol = l + dl * p.delay * 0.7 + wl * p.reverb * 1.4;
            let mut or = r + dr * p.delay * 0.7 + wr * p.reverb * 1.4;
            // Width: scale the side signal.
            let (m, s) = ((ol + or) * 0.5, (ol - or) * 0.5);
            let side = s * p.width * 1.4;
            ol = m + side;
            or = m - side;
            out_l[k] = ol;
            out_r[k] = or;
        }
    }
}

fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.6;
    let a = x.abs();
    if a <= KNEE { x } else { x.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh()) }
}

struct Processor {
    p: Arc<Shared>,
    engine: Engine,
    master_prev: f32,
    l: Vec<f32>,
    r: Vec<f32>,
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.0);
        if channels == 0 || !rate.is_finite() || rate < 1000.0 {
            return;
        }
        self.engine.sr = rate;
        let patch = Patch::read(&self.p);
        self.engine.set_running(self.p.running.load(Ordering::Relaxed));
        if self.p.regen.swap(false, Ordering::Relaxed) {
            self.engine.comp.invalidate();
        }
        if self.p.skip.swap(false, Ordering::Relaxed) && self.engine.running {
            self.engine.timer = 0.0;
        }
        let frames = out.len() / channels;
        self.l.resize(frames, 0.0);
        self.r.resize(frames, 0.0);
        self.engine.render(&patch, &mut self.l, &mut self.r);

        let master_level = self.p.vals[C_MASTER].get();
        let master = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0) * master_level * master_level * 2.0;
        let from = self.master_prev;
        let step = (master - from) / frames.max(1) as f32;
        self.master_prev = master;
        let mut bus = self.p.output.try_lock().ok();
        if let Some(b) = bus.as_mut() {
            b.clear();
        }
        let mut peak = 0.0f32;
        for (k, frame) in out.chunks_mut(channels).enumerate() {
            let gain = from + step * k as f32;
            let fix = |s: f32| {
                let s = soft_limit(s * gain);
                if s.is_finite() { s } else { 0.0 }
            };
            let (l, r) = (fix(self.l[k]), fix(self.r[k]));
            peak = peak.max(l.abs()).max(r.abs());
            if channels == 1 {
                frame[0] = (l + r) * 0.5;
            } else {
                frame[0] = l;
                frame[1] = r;
            }
            if let Some(b) = bus.as_mut() {
                b.push((l + r) * 0.5);
            }
        }
        self.p.peak.set(peak);
        for i in 0..LAYERS {
            self.p.meters[i].set(self.engine.meter[i]);
        }
        if self.engine.chord_changed {
            self.engine.chord_changed = false;
            let c = self.engine.chord;
            for i in 0..6 {
                self.p.chord[i].store(c.notes[i], Ordering::Relaxed);
            }
            self.p.chord_n.store(c.n, Ordering::Relaxed);
            self.p.chord_root.store(c.root_pc, Ordering::Relaxed);
            self.p.chord_bass.store(c.bass, Ordering::Relaxed);
            self.p.chord_step.store(c.step, Ordering::Relaxed);
            self.p.chord_gen.fetch_add(1, Ordering::Release);
        }
        let total = (patch.dur_s * rate).max(1.0);
        self.p.progress.set(if self.engine.running { (1.0 - self.engine.timer / total).clamp(0.0, 1.0) } else { 0.0 });
    }
}

// ---------------------------------------------------------------- the app

const SKY_TOP: Rgb565 = Rgb565::new(1, 2, 5);
const SKY_BOT: Rgb565 = Rgb565::new(3, 9, 12);
const INK: Rgb565 = Rgb565::new(27, 55, 29);
const DIM: Rgb565 = Rgb565::new(10, 24, 19);
const FAINT: Rgb565 = Rgb565::new(4, 11, 10);
const LAYER_C: [Rgb565; LAYERS] = [Rgb565::new(14, 32, 31), Rgb565::new(31, 20, 5), Rgb565::new(10, 46, 22), Rgb565::new(31, 38, 27), Rgb565::new(31, 56, 14)];
const ACCENT: Rgb565 = Rgb565::new(31, 56, 14);

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "vistas",
        layers: vec![Layer::Native(0, "MOOD"), Layer::Native(1, "KEY"), Layer::Controls, Layer::Moments],
        hero: vec![
            [C_MOOD, C_ROOT],
            [C_VARIATION, C_COMPLEXITY],
            [C_DURATION, C_MODE],
            [C_PAD, C_BASS],
            [C_NOISE, C_AMB],
            [C_SHIM, C_REVERB],
            [C_SPACE, C_DELAY],
            [C_MASTER, C_WIDTH],
        ],
        browse: Some(C_SCAPE),
        // Stick opens up the space; the hands thin out and swell the layers.
        routes: Routes { stick_x: Some(C_SPACE), stick_y: Some(C_REVERB), hand_l: Some(C_AMB), hand_r: Some(C_SHIM) },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: false,
    }
}

pub struct VistasApp {
    p: Arc<Shared>,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
    scape: usize,
    prev_grid: [bool; 16],
    prev_midi: Vec<u8>,
    seen_gen: usize,
    hist: VecDeque<([u8; 6], usize, u8, u8)>,
    frame: u32,
}

impl VistasApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let cv: [Arc<AtomicF32>; 8] = std::array::from_fn(|i| mods.register(format!("{APP_NAME}: {}", CV_NAMES[i])));
        let mut app = Self {
            p: Arc::new(Shared {
                vals: std::array::from_fn(|i| AtomicF32::new(CTLS[i].default)),
                cv,
                mix_level,
                ext_mix_level,
                output,
                peak: AtomicF32::new(0.0),
                meters: std::array::from_fn(|_| AtomicF32::new(0.0)),
                running: AtomicBool::new(false),
                regen: AtomicBool::new(false),
                skip: AtomicBool::new(false),
                chord: std::array::from_fn(|_| AtomicU8::new(0)),
                chord_n: AtomicUsize::new(0),
                chord_root: AtomicU8::new(0),
                chord_bass: AtomicU8::new(0),
                chord_gen: AtomicUsize::new(0),
                chord_step: AtomicUsize::new(0),
                progress: AtomicF32::new(0.0),
            }),
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            scape: 0,
            prev_grid: [false; 16],
            prev_midi: vec![0; 128],
            seen_gen: 0,
            hist: VecDeque::new(),
            frame: 0,
        };
        app.load_scape(0);
        app
    }

    fn load_scape(&mut self, idx: usize) {
        self.scape = idx.min(SCAPES.len() - 1);
        for i in 0..N {
            if !scape_keeps(i) {
                self.p.vals[i].set(CTLS[i].default);
            }
        }
        for &(i, v) in SCAPES[self.scape].1 {
            self.p.vals[i].set(v);
        }
        self.p.regen.store(true, Ordering::Relaxed);
    }

    fn step_scape(&mut self, d: i32) {
        if d != 0 {
            self.load_scape((self.scape as i32 + d.signum()).rem_euclid(SCAPES.len() as i32) as usize);
        }
    }

    fn val(&self, i: usize) -> f32 {
        self.p.vals[i].get()
    }

    fn step_of(&self, i: usize) -> usize {
        norm_step(self.val(i), CTLS[i].steps)
    }

    fn playing(&self) -> bool {
        self.p.running.load(Ordering::Relaxed)
    }

    fn text(&self, i: usize) -> (String, String) {
        let v = self.p.eff(i);
        let value = match i {
            C_MOOD => MOODS[self.step_of(i)].0.to_string(),
            C_ROOT => NOTE_NAMES[self.step_of(i)].to_string(),
            C_MODE => MODE_NAMES[self.step_of(i)].to_string(),
            C_SCAPE => format!("{:02} {}", self.scape + 1, SCAPES[self.scape].0),
            C_COMPLEXITY => format!("{} notes", 2 + (v * 4.0).round() as usize),
            C_DURATION => format!("{:.1} s", 1.5 * 16.0f32.powf(v)),
            C_SPACE => format!("RT60 {:.1} s", 2.0 * 12.5f32.powf(v)),
            C_OCTAVE => format!("{:+}", self.step_of(i) as i32 - 1),
            i if i >= C_LAYER0 && i < C_DENSITY && (i - C_LAYER0) % 3 == 2 => TIMBRE_NAMES[(i - C_LAYER0) / 3][self.step_of(i)].to_string(),
            _ => format!("{:.0}%", v * 100.0),
        };
        (CTLS[i].name.to_string(), value)
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        (0..N).map(|i| {
            let (n, v) = self.text(i);
            (n, v, false)
        }).collect()
    }

    fn edit(&mut self, i: usize, d: i32) {
        if d == 0 {
            return;
        }
        if i == C_SCAPE {
            return self.step_scape(d);
        }
        let steps = CTLS[i].steps;
        if steps > 0 {
            let wrap = i == C_MOOD || i == C_ROOT;
            let next = self.step_of(i) as i32 + d.signum();
            let next = if wrap { next.rem_euclid(steps as i32) } else { next.clamp(0, steps as i32 - 1) } as usize;
            self.p.vals[i].set(step_norm(next, steps));
            if i == C_MOOD {
                self.p.regen.store(true, Ordering::Relaxed);
            }
        } else {
            let step = d as f32 * 0.01 * self.sensitivity.get().max(0.01) * 10.0;
            self.p.vals[i].set((self.val(i) + step).clamp(0.0, 1.0));
        }
    }

    fn toggle(&mut self) {
        let on = !self.playing();
        self.p.running.store(on, Ordering::Relaxed);
    }

    fn pad_press(&mut self, layer: u8, rank: usize) {
        match (layer, rank) {
            (0, 0..=9) => {
                self.p.vals[C_MOOD].set(step_norm(rank, 10));
                self.p.regen.store(true, Ordering::Relaxed);
            }
            (1, 0..=11) => self.p.vals[C_ROOT].set(step_norm(rank, 12)),
            (0, 10) | (1, 12) => self.toggle(),
            (0, 11) | (1, 13) => self.p.regen.store(true, Ordering::Relaxed),
            (0, 12..=14) => self.p.vals[C_MODE].set(step_norm(rank - 12, 3)),
            (0, 15) | (1, 14) => self.p.skip.store(true, Ordering::Relaxed),
            (1, 15) => {
                let next = (self.step_of(C_OCTAVE) + 1) % 3;
                self.p.vals[C_OCTAVE].set(step_norm(next, 3));
            }
            _ => {}
        }
    }
}

impl PlayHost for VistasApp {
    fn kit_control_count(&self) -> usize {
        N
    }
    fn kit_label(&self, i: usize) -> String {
        self.text(i.min(N - 1)).0
    }
    fn kit_value(&self, i: usize) -> String {
        self.text(i.min(N - 1)).1
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        Some(if i == C_SCAPE { step_norm(self.scape, SCAPES.len()) } else { self.val(i) })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        CTLS[i.min(N - 1)].steps > 0
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit(i.min(N - 1), delta);
    }
    fn kit_reset(&mut self, i: usize) {
        let i = i.min(N - 1);
        if i == C_SCAPE {
            self.load_scape(self.scape);
        } else {
            self.p.vals[i].set(CTLS[i].default);
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let i = i.min(N - 1);
        let v = v.clamp(0.0, 1.0);
        if i == C_SCAPE {
            let target = norm_step(v, SCAPES.len());
            if target != self.scape {
                self.load_scape(target);
            }
        } else if CTLS[i].steps > 0 {
            self.p.vals[i].set(step_norm(norm_step(v, CTLS[i].steps), CTLS[i].steps));
            if i == C_MOOD {
                self.p.regen.store(true, Ordering::Relaxed);
            }
        } else {
            self.p.vals[i].set(v);
        }
    }
    fn kit_line(&self) -> String {
        format!("{} · {} {}", SCAPES[self.scape].0, NOTE_NAMES[self.step_of(C_ROOT)], MOODS[self.step_of(C_MOOD)].0)
    }
    fn kit_pad_label(&self, layer: u8, pad: usize) -> String {
        let rank = pad_rank(pad);
        match (layer, rank) {
            (0, 0..=9) => MOOD_ABBR[rank].into(),
            (1, 0..=11) => NOTE_NAMES[rank].into(),
            (0, 10) | (1, 12) => if self.playing() { "STOP" } else { "PLAY" }.into(),
            (0, 11) | (1, 13) => "NEW".into(),
            (0, 12..=14) => ["LOOP", "COMP", "EVOL"][rank - 12].into(),
            (0, 15) | (1, 14) => "NEXT".into(),
            (1, 15) => "OCT".into(),
            _ => String::new(),
        }
    }
    fn kit_pad_color(&self, layer: u8, pad: usize, _held: bool) -> crate::led_output::PadColor {
        use crate::led_output::PadColor;
        let rank = pad_rank(pad);
        let on = match (layer, rank) {
            (0, 0..=9) => self.step_of(C_MOOD) == rank,
            (1, 0..=11) => self.step_of(C_ROOT) == rank,
            (0, 10) | (1, 12) => self.playing(),
            (0, 12..=14) => self.step_of(C_MODE) == rank - 12,
            _ => false,
        };
        if on {
            PadColor::Green
        } else if self.kit_pad_label(layer, pad).is_empty() {
            PadColor::Off
        } else {
            PadColor::Blue
        }
    }
}

fn rect(f: &mut FrameBuffer, x: i32, y: i32, w: u32, h: u32, c: Rgb565) {
    Rectangle::new(Point::new(x, y), Size::new(w, h)).into_styled(PrimitiveStyle::with_fill(c)).draw(f).ok();
}

fn label(f: &mut FrameBuffer, s: &str, x: i32, y: i32, c: Rgb565) {
    Text::new(s, Point::new(x, y), MonoTextStyle::new(&SPLEEN_6X12, c)).draw(f).ok();
}

fn mix565(a: Rgb565, b: Rgb565, t: f32) -> Rgb565 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)) as u8;
    Rgb565::new(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// A soft orb: a bright core inside two fainter halos.
fn orb(f: &mut FrameBuffer, x: i32, y: i32, r: i32, c: Rgb565, bg: Rgb565, strength: f32) {
    for (k, (grow, mix)) in [(3, 0.12), (2, 0.25), (1, 0.5)].into_iter().enumerate() {
        let rr = r + grow * 2;
        let _ = k;
        Circle::new(Point::new(x - rr, y - rr), (rr * 2) as u32).into_styled(PrimitiveStyle::with_fill(mix565(bg, c, mix * strength))).draw(f).ok();
    }
    Circle::new(Point::new(x - r, y - r), (r * 2) as u32).into_styled(PrimitiveStyle::with_fill(mix565(bg, c, 0.35 + 0.65 * strength))).draw(f).ok();
}

impl VistasApp {
    fn pull_history(&mut self) {
        let g = self.p.chord_gen.load(Ordering::Acquire);
        if g != self.seen_gen {
            self.seen_gen = g;
            let notes = std::array::from_fn(|i| self.p.chord[i].load(Ordering::Relaxed));
            self.hist.push_back((notes, self.p.chord_n.load(Ordering::Relaxed), self.p.chord_root.load(Ordering::Relaxed), self.p.chord_bass.load(Ordering::Relaxed)));
            while self.hist.len() > 6 {
                self.hist.pop_front();
            }
        }
    }

    fn draw_scape(&mut self, f: &mut FrameBuffer) {
        self.frame = self.frame.wrapping_add(1);
        self.pull_history();
        // Night sky: a vertical gradient with a few slow stars.
        for y in 0..360 {
            rect(f, 0, y, 640, 1, mix565(SKY_TOP, SKY_BOT, y as f32 / 359.0));
        }
        for i in 0..64u32 {
            let h = i.wrapping_mul(2_654_435_761);
            let (x, y) = ((h >> 8) % 640, (h >> 20) % 260 + 6);
            let tw = 0.5 + 0.5 * ((self.frame as f32 * 0.03) + i as f32 * 1.7).sin();
            rect(f, x as i32, y as i32, 1, 1, mix565(SKY_TOP, INK, 0.2 + 0.5 * tw));
        }

        let running = self.playing();
        let mood = self.step_of(C_MOOD);
        let progress = self.p.progress.get();
        // Header.
        Text::new("VISTAS", Point::new(16, 34), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(f).ok();
        label(f, "GENERATIVE CINEMATIC SCAPES", 18, 48, DIM);
        let head = format!("{} {}", NOTE_NAMES[self.step_of(C_ROOT)], MOODS[mood].0.to_uppercase());
        Text::new(&head, Point::new(630 - head.len() as i32 * 16, 34), MonoTextStyle::new(&SPLEEN_16X32, ACCENT)).draw(f).ok();
        let state = format!("{}   {}   {}", if running { "PLAYING" } else { "STOPPED" }, MODE_NAMES[self.step_of(C_MODE)], SCAPES[self.scape].0.to_uppercase());
        label(f, &state, 630 - state.len() as i32 * 6, 48, if running { ACCENT } else { DIM });
        rect(f, 14, 56, 612, 1, FAINT);

        // The chord constellation: columns of notes, pitch up the page, time to
        // the right, the current chord easing in from the right as it plays.
        let (top, bot) = (70, 250);
        let pitch_y = |p: f32| bot as f32 - (p - 26.0) / (84.0 - 26.0) * (bot - top) as f32;
        let n = self.hist.len();
        for (i, (notes, cnt, _root, bass)) in self.hist.iter().enumerate() {
            let age = (n - 1 - i) as f32;
            let x = 470.0 - (age + progress) * 104.0;
            // Keep clear of the chord name on the left.
            if !(170.0..=640.0).contains(&x) {
                continue;
            }
            let fade = if age == 0.0 { 1.0 } else { (1.0 - (age + progress) * 0.17).max(0.05) };
            for k in 0..*cnt {
                let y = pitch_y(notes[k] as f32) as i32;
                // Hue by height so a chord reads as a rising colour.
                let c = mix565(LAYER_C[0], LAYER_C[3], k as f32 / 5.0);
                orb(f, x as i32, y, 4, c, SKY_BOT, fade);
            }
            orb(f, x as i32, pitch_y(*bass as f32) as i32, 6, LAYER_C[1], SKY_BOT, fade);
        }
        // The playhead and progress through the current chord.
        rect(f, 470, top, 1, (bot - top) as u32, FAINT);
        rect(f, 330, bot + 14, 280, 3, FAINT);
        rect(f, 330, bot + 14, (280.0 * progress) as u32, 3, ACCENT);
        if let Some((notes, cnt, root, _)) = self.hist.back() {
            let name = chord_name(*root, &notes[..*cnt]);
            Text::new(&name, Point::new(18, 110), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(f).ok();
            label(f, &format!("CHORD {}/8  ·  {} NOTES", self.p.chord_step.load(Ordering::Relaxed) + 1, cnt), 18, 128, DIM);
        } else {
            Text::new("--", Point::new(18, 110), MonoTextStyle::new(&SPLEEN_16X32, DIM)).draw(f).ok();
            label(f, "PRESS F3 OR THE PLAY PAD", 18, 128, DIM);
        }

        // Layer meters.
        for i in 0..LAYERS {
            let x = 18 + i as i32 * 62;
            let level = self.val(LEVEL_CTL[i]);
            rect(f, x, 276, 50, 50, mix565(SKY_BOT, SKY_TOP, 0.5));
            let live = (self.p.meters[i].get() * 4.0).clamp(0.0, 1.0).sqrt();
            rect(f, x, 326 - (50.0 * level) as i32, 50, 1, mix565(SKY_BOT, LAYER_C[i], 0.7));
            rect(f, x + 1, 326 - (48.0 * live) as i32, 48, (48.0 * live) as u32, mix565(SKY_BOT, LAYER_C[i], 0.35 + 0.5 * live));
            label(f, &LAYER_NAMES[i].to_uppercase().chars().take(5).collect::<String>(), x + 2, 340, LAYER_C[i]);
            let tname = TIMBRE_NAMES[i][self.step_of(timbre_ctl(i))];
            label(f, &tname.chars().take(8).collect::<String>(), x + 2, 354, DIM);
        }

        // Right-hand readouts.
        let rows = [
            ("VAR", self.text(C_VARIATION).1),
            ("CPX", self.text(C_COMPLEXITY).1),
            ("DUR", self.text(C_DURATION).1),
            ("REV", self.text(C_REVERB).1),
            ("SPC", self.text(C_SPACE).1),
            ("DLY", self.text(C_DELAY).1),
        ];
        for (i, (k, v)) in rows.iter().enumerate() {
            label(f, k, 350, 284 + i as i32 * 13, DIM);
            label(f, v, 384, 284 + i as i32 * 13, INK);
        }
        let marked = if self.kit.menu { Some(self.list.selected.min(N - 1)) } else { None };
        let line = match marked {
            Some(i) => format!("{}: {}", self.text(i).0, self.text(i).1),
            None => self.kit.column(self).status,
        };
        label(f, &line.chars().take(40).collect::<String>(), 350, 270, ACCENT);
        let level = self.p.peak.get().clamp(0.0, 1.0).sqrt();
        rect(f, 480, 340, 146, 6, FAINT);
        rect(f, 480, 340, (level * 146.0) as u32, 6, ACCENT);
    }
}

impl App for VistasApp {
    fn instrument_settings(&self) -> Vec<crate::app::Setting> {
        kit::settings_of(self)
    }
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        kit::adjust_in(self, index, delta)
    }
    fn running(&self) -> Option<bool> {
        Some(self.playing())
    }
    fn toggle_running(&mut self) {
        self.toggle();
    }
    fn supports_pad_lock(&self) -> bool {
        true
    }
    fn play_surface(&self) -> bool {
        true
    }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (!self.kit.menu).then(|| self.kit.column(self))
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }
    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
    }
    fn grid_led_overlay(&self) -> [crate::led_output::PadColor; 16] {
        self.kit.led_overlay(self)
    }
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        // The pad layers act on a press, not a hold.
        if let Some(layer) = step.native {
            for pad in 0..16 {
                if step.input.grid[pad] && !self.prev_grid[pad] {
                    self.pad_press(layer as u8, pad_rank(pad));
                }
            }
        }
        for pad in 0..16 {
            self.prev_grid[pad] = step.native.is_some() && step.input.grid[pad];
        }
        // A MIDI key (from a keyboard or another app) sets the key.
        for k in 21..128 {
            let v = input.midi_keys.0[k];
            if v > 0 && self.prev_midi[k] == 0 {
                self.p.vals[C_ROOT].set(step_norm(k % 12, 12));
            }
            self.prev_midi[k] = v;
        }
        if step.menu {
            let input = &step.input;
            self.list.navigate_input(input, N, self.nav.get() as i32);
            let sel = self.list.selected.min(N - 1);
            self.kit_edit(sel, input.knob2);
            if input.knob2_press {
                self.kit_reset(sel);
            }
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        self.draw_scape(f);
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if self.taken {
            return None;
        }
        self.taken = true;
        Some(Box::new(Processor { p: Arc::clone(&self.p), engine: Engine::new(), master_prev: 0.0, l: Vec::new(), r: Vec::new() }))
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(VistasApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> VistasApp {
        VistasApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
    }

    fn render(p: &mut Box<dyn AudioProcessor>, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (Vec::new(), Vec::new());
        for _ in 0..((seconds * 48_000.0 / 512.0) as usize) {
            let mut buf = vec![0.0f32; 512 * 2];
            p.process(&mut buf, 2, 48_000.0);
            assert!(buf.iter().all(|v| v.is_finite() && v.abs() <= 1.0), "output stays finite and inside full scale");
            l.extend(buf.iter().step_by(2));
            r.extend(buf.iter().skip(1).step_by(2));
        }
        (l, r)
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn set(a: &mut VistasApp, i: usize, v: f32) {
        a.kit_set_norm(i, v);
    }

    fn solo(a: &mut VistasApp, layer: usize) {
        for l in 0..LAYERS {
            set(a, LEVEL_CTL[l], if l == layer { 0.9 } else { 0.0 });
        }
        set(a, C_REVERB, 0.0);
        set(a, C_DELAY, 0.0);
    }

    fn play(a: &mut VistasApp) {
        a.toggle_running();
    }

    #[test]
    fn nothing_sounds_until_it_is_started_then_it_plays_by_itself() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        assert_eq!(a.running(), Some(false));
        assert!(rms(&render(&mut p, 1.0).0) < 1e-6, "silent while stopped");
        play(&mut a);
        render(&mut p, 2.0);
        let (l, r) = render(&mut p, 3.0);
        assert!(rms(&l) > 0.003 && rms(&r) > 0.003, "a full scape is audible with no keys pressed ({}, {})", rms(&l), rms(&r));
        let diff: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        assert!(rms(&diff) > 0.0005, "and stereo ({})", rms(&diff));
    }

    #[test]
    fn stopping_lets_the_layers_fade_and_the_reverb_ring_out() {
        let mut a = app();
        set(&mut a, C_SPACE, 0.5);
        let mut p = a.audio_processor().unwrap();
        play(&mut a);
        render(&mut p, 6.0);
        let before = rms(&render(&mut p, 1.0).0);
        play(&mut a);
        render(&mut p, 1.0);
        let tail = rms(&render(&mut p, 0.5).0);
        assert!(tail > 0.0001 && tail < before * 1.1, "a tail remains after stopping ({tail} vs {before})");
        render(&mut p, 25.0);
        let late = rms(&render(&mut p, 1.0).0);
        assert!(late < before * 0.2, "and it dies away ({late} vs {before})");
    }

    #[test]
    fn every_chord_is_made_only_of_notes_of_its_mood_in_the_chosen_key() {
        for (m, (name, scale)) in MOODS.iter().enumerate() {
            for root in [0u8, 3, 7, 10] {
                for mode in 0..3 {
                    let mut comp = Composer::new();
                    let p = Patch { mood: m, root, mode, variation: 0.9, complexity: 0.9, dur_s: 4.0, layers: [LayerP::default(); LAYERS], reverb: 0.0, rt60: 3.0, delay: 0.0, width: 0.5, density: 0.0, octave: 0 };
                    for _ in 0..40 {
                        let c = comp.next(&p);
                        for &n in &c.notes[..c.n] {
                            assert!(scale.contains(&((n + 120 - root) % 12)), "{name} in {root} mode {mode}: note {n} is outside the scale");
                        }
                        assert!((45..=84).contains(&c.notes[0]) && c.notes[c.n - 1] <= 84, "voicing stays in range");
                    }
                }
            }
        }
    }

    #[test]
    fn chords_are_voice_led_and_complexity_sets_the_note_count() {
        let mut comp = Composer::new();
        let mut p = Patch { mood: 4, root: 2, mode: 2, variation: 1.0, complexity: 0.5, dur_s: 4.0, layers: [LayerP::default(); LAYERS], reverb: 0.0, rt60: 3.0, delay: 0.0, width: 0.5, density: 0.0, octave: 0 };
        let mut prev = comp.next(&p);
        let mut moved = 0.0;
        for _ in 0..60 {
            let c = comp.next(&p);
            assert_eq!(c.n, 4);
            moved += (0..4).map(|i| (c.notes[i] as f32 - prev.notes[i] as f32).abs()).sum::<f32>() / 4.0;
            prev = c;
        }
        assert!(moved / 60.0 < 3.5, "voices move a few semitones at most on average ({})", moved / 60.0);
        for (cx, n) in [(0.0, 2), (0.5, 4), (1.0, 6)] {
            p.complexity = cx;
            assert_eq!(comp.next(&p).n, n);
        }
    }

    #[test]
    fn loop_repeats_compose_rewrites_and_evolve_wanders() {
        let degrees = |mode: usize| -> Vec<u8> {
            let mut comp = Composer::new();
            let p = Patch { mood: 0, root: 0, mode, variation: 1.0, complexity: 0.5, dur_s: 4.0, layers: [LayerP::default(); LAYERS], reverb: 0.0, rt60: 3.0, delay: 0.0, width: 0.5, density: 0.0, octave: 0 };
            (0..24).map(|_| comp.next(&p).root_pc).collect()
        };
        let l = degrees(0);
        assert_eq!(l[0..8], l[8..16], "loop plays the same eight chords");
        assert_eq!(l[0..8], l[16..24]);
        let c = degrees(1);
        assert!(c[0..8] != c[8..16] || c[8..16] != c[16..24], "compose writes a new progression each pass");
        let e = degrees(2);
        assert!(e.windows(2).filter(|w| w[0] != w[1]).count() > 10, "evolve keeps moving");
    }

    #[test]
    fn the_key_transposes_the_whole_scape() {
        let roots = |root: u8| {
            let mut comp = Composer::new();
            let p = Patch { mood: 0, root, mode: 0, variation: 0.5, complexity: 0.5, dur_s: 4.0, layers: [LayerP::default(); LAYERS], reverb: 0.0, rt60: 3.0, delay: 0.0, width: 0.5, density: 0.0, octave: 0 };
            (0..8).map(|_| comp.next(&p).root_pc).collect::<Vec<_>>()
        };
        let (c, g) = (roots(0), roots(7));
        assert!(c.iter().zip(&g).all(|(a, b)| (*a + 7) % 12 == *b), "G is C moved up a fifth");
        assert_eq!(chord_name(2, &[50, 53, 57, 60, 64]), "Dm9");
        assert_eq!(chord_name(7, &[55, 59, 62]), "G");
        assert_eq!(chord_name(5, &[53, 55, 60]), "Fsus2");
    }

    #[test]
    fn duration_sets_how_often_the_chord_changes() {
        let changes = |dur: f32| {
            let mut a = app();
            set(&mut a, C_DURATION, dur);
            let mut p = a.audio_processor().unwrap();
            play(&mut a);
            render(&mut p, 20.0);
            a.p.chord_gen.load(Ordering::Relaxed)
        };
        let (fast, slow) = (changes(0.0), changes(0.6));
        assert!(fast >= 12 && slow <= 3 && fast > slow * 3, "short chords change often ({fast}) and long ones rarely ({slow})");
    }

    #[test]
    fn each_layer_sounds_on_its_own_and_in_its_own_register() {
        let mut bands = Vec::new();
        for layer in 0..LAYERS {
            let mut a = app();
            solo(&mut a, layer);
            set(&mut a, C_DENSITY, 1.0);
            let mut p = a.audio_processor().unwrap();
            play(&mut a);
            render(&mut p, 9.0);
            let (l, _) = render(&mut p, 6.0);
            assert!(rms(&l) > 0.0005, "{} makes sound alone ({})", LAYER_NAMES[layer], rms(&l));
            let d: f32 = l.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>() / l.iter().map(|v| v * v).sum::<f32>().max(1e-12);
            bands.push(d);
        }
        assert!(bands[1] < bands[0] * 0.2, "bass is much darker than the pad ({} vs {})", bands[1], bands[0]);
        assert!(bands[4] > bands[0], "shimmer is brighter than the pad ({} vs {})", bands[4], bands[0]);
    }

    #[test]
    fn every_scape_and_every_timbre_stays_inside_full_scale() {
        for i in 0..SCAPES.len() {
            let mut a = app();
            a.load_scape(i);
            assert!(a.kit_value(C_SCAPE).contains(SCAPES[i].0));
            let mut p = a.audio_processor().unwrap();
            play(&mut a);
            let (l, _) = render(&mut p, 6.0);
            assert!(rms(&l[l.len() / 2..]) > 0.0005, "{} makes sound", SCAPES[i].0);
        }
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        play(&mut a);
        set(&mut a, C_DURATION, 0.0);
        for t in 0..8 {
            for l in 0..LAYERS {
                set(&mut a, timbre_ctl(l), step_norm(t, 8));
                set(&mut a, LEVEL_CTL[l], 1.0);
                set(&mut a, motion_ctl(l), 1.0);
            }
            render(&mut p, 3.0);
        }
    }

    #[test]
    fn pads_pick_the_mood_the_key_and_the_transport() {
        let mut a = app();
        a.kit.menu = false;
        a.pad_press(0, 7);
        assert_eq!(a.kit_value(C_MOOD), "Tense");
        a.pad_press(1, 7);
        assert_eq!(a.kit_value(C_ROOT), "G");
        a.pad_press(0, 14);
        assert_eq!(a.kit_value(C_MODE), "EVOLVE");
        assert!(!a.playing());
        a.pad_press(0, 10);
        assert!(a.playing());
        let mut input = Input::default();
        input.midi_keys.0[63] = 90;
        a.tick(&input);
        assert_eq!(a.kit_value(C_ROOT), "D#", "a MIDI key sets the key");
    }

    #[test]
    fn it_opens_on_the_play_view_and_draws() {
        let mut a = app();
        assert!(!a.kit.menu);
        let before = a.val(C_MOOD);
        a.tick(&Input { knob1: 2, ..Default::default() });
        assert!(a.val(C_MOOD) > before, "knob 1 turns the first hero control");
        let mut fb = FrameBuffer::new();
        a.draw(&mut fb);
    }

    /// Renders the screen to PNGs in the folder named by PORTAMAX_VISTAS_SHOT.
    #[test]
    #[ignore = "writes screenshots to the folder in PORTAMAX_VISTAS_SHOT"]
    fn screenshot() {
        let Ok(dir) = std::env::var("PORTAMAX_VISTAS_SHOT") else { return };
        let mut a = app();
        a.load_scape(5);
        let mut p = a.audio_processor().unwrap();
        set(&mut a, C_DURATION, 0.0);
        play(&mut a);
        let mut fb = FrameBuffer::new();
        for _ in 0..80 {
            render(&mut p, 0.25);
            a.draw(&mut fb);
        }
        let file = std::fs::File::create(std::path::Path::new(&dir).join("vistas.png")).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), crate::display::WIDTH as u32, crate::display::HEIGHT as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let data: Vec<u8> = fb.buffer().iter().flat_map(|px| [(px >> 16) as u8, (px >> 8) as u8, *px as u8]).collect();
        enc.write_header().unwrap().write_image_data(&data).unwrap();
    }
}
