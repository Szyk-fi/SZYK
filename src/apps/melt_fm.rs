//! Melt FM: a generative radio station that composes forever and gets
//! stranger on demand. Ported from the Melt FM web page (Web Audio) to a
//! native `AudioProcessor`; the composition rules, voices and effects
//! follow that page one for one.
//!
//! Pick a genre (ten of them) and the app writes a 32-bar song for it: a
//! drum pattern, a bass line, chords, a lead, and a "polymeter ghost", a glass
//! arpeggio whose length (5, 7, 9 or 11 steps) doesn't divide the bar. Songs
//! run Intro, Groove, Lift, Break, Drop and then mutate. **Weirdness** is the
//! one dial that decides how long it stays normal: it adds a seasick pitch
//! drift, ring modulation, drive, swing flips, off-key chord slips, borrowed
//! drums from other genres, stutters, tape stops, insects, a choir, and
//! eventually switches the scale to one with no name. **Hold to melt** slows
//! the tape (pitch and tempo) while it is held.
//!
//! Everything runs inside the audio callback with no allocation: voices live
//! in a fixed pool, the song is plain arrays, and the reverb/delay lines are
//! sized once. The UI side only sets a handful of atomics and reads back
//! counters for the screen.
//!
//! Where it differs from the web page, on purpose:
//! - the page's convolution reverb (a decaying noise impulse) is a stereo
//!   comb/all-pass reverb here, which is what such an impulse sounds like and
//!   costs a fraction of the CPU;
//! - oscillators are PolyBLEP band-limited (the browser's are too);
//! - the master compressor is a plain feed-forward one with a fixed makeup
//!   gain standing in for the browser's automatic makeup.
//!
//! The screen is the page's: a full-screen kaleidoscope tunnel that moves with
//! the music, a heavy wordmark, and a glass panel with Play, New track, Hold to
//! melt, the genre chips, Wander, and the Weirdness and Space sliders. Every
//! control works with the mouse (press, drag, release) and from the hardware:
//! pads 1-10 pick a genre, 13 new track, 14 hold to melt, 15 wander, 16
//! play/stop; knob 1 is Weirdness, knob 2 is Space; F3 starts and stops.

use crate::app::{App, Input, Setting, SlintExtra};
use crate::apps::kids_kit as kit;
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16};
use crate::util::AtomicF32;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle, RoundedRectangle};
use embedded_graphics::text::Text;
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const NAME: &str = "Melt FM";
const MAX_VOICES: usize = 128;
const MAX_CHOIRS: usize = 3;
const SONG_BARS: i64 = 32;
const OUT_LEVEL: f32 = 0.72;

// ---------------------------------------------------------------- data

#[derive(Clone, Copy, PartialEq)]
enum BassStyle { Roll, Offbeat, Octave, Sparse, Walk, Sub, Dub, Bossa }
#[derive(Clone, Copy, PartialEq)]
enum PadStyle { Stab, Rhodes, Swell, Skank, Pad, Comp, Pluck }
#[derive(Clone, Copy, PartialEq)]
enum LeadStyle { Acid, Melody, Bell, Arp }
#[derive(Clone, Copy, PartialEq)]
enum Drum { Kick, Snare, Clap, Hat, Ghost, Oh, Ride, Rim, Perc }

struct Genre {
    id: &'static str,
    name: &'static str,
    bpm: f32,
    swing: f32,
    scale: usize,
    root: f32,
    prog: [i32; 4],
    bars: i64,
    sev: bool,
    drums: &'static [(Drum, &'static str)],
    bass: BassStyle,
    pad: PadStyle,
    lead: LeadStyle,
    drive: f32,
    hue: f32,
    lp: f32,
    crackle: bool,
    dub: bool,
    hat_rolls: bool,
}

use BassStyle as B;
use Drum::*;
use LeadStyle as L;
use PadStyle as P;

static GENRES: [Genre; 10] = [
    Genre { id: "techno", name: "Techno", bpm: 130.0, swing: 0.0, scale: 3, root: 38.0, prog: [0, 0, 3, 5], bars: 2, sev: false,
        drums: &[(Kick, "x...x...x...x..."), (Clap, "....x.......x..."), (Hat, "..x...x...x...x."), (Ghost, "o.oo.oo.o.oo.o?o"), (Perc, "...x.....x..?...")],
        bass: B::Roll, pad: P::Stab, lead: L::Acid, drive: 0.35, hue: 0.72, lp: 1.0, crackle: false, dub: false, hat_rolls: false },
    Genre { id: "house", name: "House", bpm: 123.0, swing: 0.15, scale: 2, root: 41.0, prog: [0, 3, 4, 3], bars: 1, sev: true,
        drums: &[(Kick, "x...x...x...x..."), (Clap, "....x.......x..."), (Oh, "..x...x...x...x."), (Ghost, "o.o.o.o.o.o.o.o."), (Perc, ".......?......x.")],
        bass: B::Offbeat, pad: P::Rhodes, lead: L::Melody, drive: 0.15, hue: 0.02, lp: 1.0, crackle: false, dub: false, hat_rolls: false },
    Genre { id: "lofi", name: "Lo-fi", bpm: 80.0, swing: 0.5, scale: 2, root: 43.0, prog: [1, 4, 0, 5], bars: 1, sev: true,
        drums: &[(Kick, "x......x..x....."), (Snare, "....x.......x..."), (Ghost, "o.o.o.o.o.o.o.oo")],
        bass: B::Sparse, pad: P::Rhodes, lead: L::Bell, drive: 0.25, hue: 0.52, lp: 0.3, crackle: true, dub: false, hat_rolls: false },
    Genre { id: "ambient", name: "Ambient", bpm: 68.0, swing: 0.0, scale: 4, root: 40.0, prog: [0, 4, 5, 3], bars: 4, sev: true,
        drums: &[(Perc, "x.......?......."), (Ghost, "....?.......?...")],
        bass: B::Sparse, pad: P::Swell, lead: L::Bell, drive: 0.0, hue: 0.45, lp: 0.7, crackle: false, dub: false, hat_rolls: false },
    Genre { id: "dnb", name: "Drum & Bass", bpm: 172.0, swing: 0.0, scale: 1, root: 38.0, prog: [0, 5, 3, 4], bars: 2, sev: true,
        drums: &[(Kick, "x.........x....."), (Snare, "....x.......x..."), (Hat, "x.x.x.x.x.x.x.x."), (Rim, "..o....o.o....o.")],
        bass: B::Sub, pad: P::Swell, lead: L::Arp, drive: 0.3, hue: 0.6, lp: 1.0, crackle: false, dub: false, hat_rolls: false },
    Genre { id: "dub", name: "Dub", bpm: 76.0, swing: 0.2, scale: 1, root: 40.0, prog: [0, 3, 0, 4], bars: 2, sev: false,
        drums: &[(Kick, "........x......."), (Rim, "........x......."), (Ghost, "o.o.o.o.o.o.o.o."), (Oh, "..?.......?.....")],
        bass: B::Dub, pad: P::Skank, lead: L::Melody, drive: 0.2, hue: 0.3, lp: 0.6, crackle: false, dub: true, hat_rolls: false },
    Genre { id: "synthwave", name: "Synthwave", bpm: 102.0, swing: 0.0, scale: 1, root: 40.0, prog: [0, 5, 2, 6], bars: 1, sev: false,
        drums: &[(Kick, "x...x...x...x..."), (Snare, "....x.......x..."), (Hat, "x.x.x.x.x.x.x.x.")],
        bass: B::Octave, pad: P::Pad, lead: L::Arp, drive: 0.15, hue: 0.85, lp: 1.0, crackle: false, dub: false, hat_rolls: false },
    Genre { id: "jazz", name: "Jazz", bpm: 126.0, swing: 0.6, scale: 2, root: 41.0, prog: [1, 4, 0, 0], bars: 1, sev: true,
        drums: &[(Ride, "x...x.xxx...x.xx"), (Rim, "....o.......o..."), (Kick, "?.......?.....?."), (Snare, "......?....?..?.")],
        bass: B::Walk, pad: P::Comp, lead: L::Melody, drive: 0.05, hue: 0.1, lp: 0.75, crackle: false, dub: false, hat_rolls: false },
    Genre { id: "trap", name: "Trap", bpm: 140.0, swing: 0.0, scale: 5, root: 37.0, prog: [0, 5, 3, 4], bars: 2, sev: false,
        drums: &[(Kick, "x......x..x....."), (Clap, "........x......."), (Hat, "xxxxxxxxxxxxxxxx"), (Oh, "..............?.")],
        bass: B::Sub, pad: P::Swell, lead: L::Bell, drive: 0.35, hue: 0.9, lp: 1.0, crackle: false, dub: false, hat_rolls: true },
    Genre { id: "bossa", name: "Bossa Nova", bpm: 132.0, swing: 0.1, scale: 0, root: 43.0, prog: [1, 4, 0, 5], bars: 1, sev: true,
        drums: &[(Kick, "x..xx..xx..xx..x"), (Rim, "x..x..x...x..x.."), (Ghost, "oooooooooooooooo")],
        bass: B::Bossa, pad: P::Pluck, lead: L::Melody, drive: 0.0, hue: 0.18, lp: 0.8, crackle: false, dub: false, hat_rolls: false },
];

struct Scale {
    name: &'static str,
    len: usize,
    s: [f32; 7],
}

/// The six scales the genres use, then the five the Weirdness dial can
/// swap in (indices 0..6 and 6..11). Quarter tones are real fractions of a
/// semitone, not rounded.
static SCALES: [Scale; 11] = [
    Scale { name: "major", len: 7, s: [0., 2., 4., 5., 7., 9., 11.] },
    Scale { name: "minor", len: 7, s: [0., 2., 3., 5., 7., 8., 10.] },
    Scale { name: "dorian", len: 7, s: [0., 2., 3., 5., 7., 9., 10.] },
    Scale { name: "phrygian", len: 7, s: [0., 1., 3., 5., 7., 8., 10.] },
    Scale { name: "lydian", len: 7, s: [0., 2., 4., 6., 7., 9., 11.] },
    Scale { name: "harmonic minor", len: 7, s: [0., 2., 3., 5., 7., 8., 11.] },
    Scale { name: "Hungarian minor", len: 7, s: [0., 2., 3., 6., 7., 8., 11.] },
    Scale { name: "whole tone", len: 6, s: [0., 2., 4., 6., 8., 10., 0.] },
    Scale { name: "Locrian", len: 7, s: [0., 1., 3., 5., 6., 8., 10.] },
    Scale { name: "Maqam Rast, in quarter tones", len: 7, s: [0., 2., 3.5, 5., 7., 9., 10.5] },
    Scale { name: "a scale with no name", len: 7, s: [0., 1.3, 3.1, 4.9, 6.6, 8.2, 10.4] },
];
const WEIRD_SCALE_FIRST: usize = 6;

const LAYER_K: u8 = 1;
const LAYER_S: u8 = 2;
const LAYER_H: u8 = 4;
const LAYER_P: u8 = 8;
const LAYER_BASS: u8 = 16;
const LAYER_PAD: u8 = 32;
const LAYER_LEAD: u8 = 64;
const LAYER_WEIRD: u8 = 128;
// The choir only appears in the Break, which has no spare bit in a u8 set;
// it is a property of that section rather than a layer.

struct Section {
    name: &'static str,
    start: i64,
    end: i64,
    layers: u8,
    build: bool,
    choir: bool,
}

static SECTIONS: [Section; 5] = [
    Section { name: "Intro", start: 0, end: 4, layers: LAYER_H | LAYER_P | LAYER_PAD, build: false, choir: false },
    Section { name: "Groove", start: 4, end: 12, layers: LAYER_K | LAYER_S | LAYER_H | LAYER_P | LAYER_BASS | LAYER_PAD, build: false, choir: false },
    Section { name: "Lift", start: 12, end: 20, layers: LAYER_K | LAYER_S | LAYER_H | LAYER_P | LAYER_BASS | LAYER_PAD | LAYER_LEAD, build: false, choir: false },
    Section { name: "Break", start: 20, end: 24, layers: LAYER_H | LAYER_PAD | LAYER_LEAD | LAYER_WEIRD, build: true, choir: true },
    Section { name: "Drop", start: 24, end: 32, layers: LAYER_K | LAYER_S | LAYER_H | LAYER_P | LAYER_BASS | LAYER_PAD | LAYER_LEAD | LAYER_WEIRD, build: false, choir: false },
];

fn section_at(cb: i64) -> usize {
    SECTIONS.iter().position(|s| cb >= s.start && cb < s.end).unwrap_or(0)
}

const WEIRD_WORDS: [&str; 6] = ["Played straight", "Slightly off", "Tilted", "Seasick", "Unravelling", "Fully melted"];

fn weird_word(w: f32) -> &'static str {
    WEIRD_WORDS[((w * 5.999) as usize).min(5)]
}

// ------------------------------------------------------------ random

#[derive(Clone, Copy)]
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Self {
        Rng(seed.max(1))
    }
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// Uniform in [0, 1).
    fn f(&mut self) -> f32 {
        (self.next() >> 8) as f32 / 16_777_216.0
    }
    fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }
    fn pick<T: Copy>(&mut self, a: &[T]) -> T {
        a[(self.f() * a.len() as f32) as usize % a.len()]
    }
}

const ADJ: [&str; 18] = ["Liminal", "Velvet", "Inverted", "Soluble", "Upside-down", "Phosphorescent", "Melted", "Nocturnal", "Elastic", "Haunted", "Translucent", "Backwards", "Molten", "Weightless", "Fractal", "Feral", "Lukewarm", "Humming"];
const NOUN: [&str; 16] = ["Escalator", "Lighthouse", "Jellyfish", "Cathedral", "Vending Machine", "Swimming Pool", "Satellite", "Staircase", "Tangerine", "Telephone", "Glacier", "Waiting Room", "Carousel", "Lava Lamp", "Mirror", "Greenhouse"];
const PLUR: [&str; 12] = ["Moths", "Astronauts", "Houseplants", "Ghosts", "Snails", "Night Buses", "Lost Socks", "Lizards", "Clocks", "Strangers", "Tourists", "Mushrooms"];

fn make_title(seed: u32) -> String {
    let mut r = Rng::new(seed ^ 0x9E37_79B9);
    for _ in 0..4 {
        r.next();
    }
    let a = r.pick(&ADJ);
    let n = r.pick(&NOUN);
    let p = r.pick(&PLUR);
    let b = r.pick(&ADJ);
    match (r.f() * 5.0) as u32 {
        0 => format!("{a} {n} for {p}"),
        1 => format!("The {n} Is {b}"),
        2 => format!("Music for {a} {p}"),
        3 => format!("{n} at the Edge of {p}"),
        _ => format!("{a} {n}"),
    }
}

// ------------------------------------------------------- the composer

#[derive(Clone, Copy, Default)]
struct BassNote {
    on: bool,
    walk: bool,
    d: i32,
    len: f32,
    acc: bool,
    slide: bool,
}

#[derive(Clone, Copy, Default)]
struct AcidNote {
    on: bool,
    d: i32,
    acc: bool,
    slide: bool,
    o: i32,
}

#[derive(Clone, Copy, Default)]
struct LeadNote {
    on: bool,
    d: i32,
    len: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum ArpOrder { Up, Down, UpDown, Random }

#[derive(Clone, Copy)]
struct Song {
    prog: [i32; 4],
    poly_len: usize,
    poly: [Option<i32>; 11],
    bass: [BassNote; 16],
    bass2: [BassNote; 16],
    pad: [f32; 16],
    acid: [AcidNote; 16],
    lead: [LeadNote; 32],
    arp: ArpOrder,
}

fn make_bass(style: BassStyle, g: &Genre, r: &mut Rng) -> [BassNote; 16] {
    let mut p = [BassNote::default(); 16];
    let n = |d: i32, len: f32| BassNote { on: true, walk: false, d, len, acc: false, slide: false };
    match style {
        B::Roll => {
            for i in 0..16 {
                if i % 4 != 0 {
                    let d = if r.f() < 0.15 { 7 } else { 0 };
                    p[i] = BassNote { acc: i % 4 == 2, ..n(d, 0.7) };
                }
            }
        }
        B::Offbeat => {
            for i in [2, 6, 10, 14] {
                p[i] = n(0, 1.6);
            }
            if r.f() < 0.6 {
                p[7] = n(r.pick(&[4, 7]), 0.8);
            }
            if r.f() < 0.5 {
                p[15] = n(r.pick(&[2, 4, 7]), 0.8);
            }
        }
        B::Octave => {
            for i in (0..16).step_by(2) {
                p[i] = n(if i % 4 == 2 { 7 } else { 0 }, 1.2);
            }
        }
        B::Sparse => {
            p[0] = n(0, 10.0);
            if r.f() < 0.6 {
                p[10] = n(r.pick(&[4, 2, -3]), 5.0);
            }
        }
        B::Walk => {
            for i in (0..16).step_by(4) {
                p[i] = BassNote { walk: true, ..n(0, 3.4) };
            }
        }
        B::Sub => {
            let kick = g.drums.iter().find(|(d, _)| *d == Kick).map(|(_, s)| *s).unwrap_or("x.......x.......");
            let hits: Vec<usize> = kick.bytes().take(16).enumerate().filter(|(_, c)| *c == b'x').map(|(i, _)| i).collect();
            for (j, &s) in hits.iter().enumerate() {
                let nx = hits.get(j + 1).copied().unwrap_or(16);
                let d = if j == 0 { 0 } else { r.pick(&[0, 0, 4, -3, 7]) };
                let slide = j > 0 && r.f() < 0.5;
                p[s] = BassNote { slide, ..n(d, (nx - s).max(1) as f32 - 0.2) };
            }
            if r.f() < 0.5 {
                p[14] = BassNote { slide: true, ..n(7, 1.5) };
            }
        }
        B::Dub => {
            p[0] = n(0, 3.0);
            p[3] = n(0, 1.0);
            p[6] = n(r.pick(&[2, 4]), 2.0);
            p[10] = n(r.pick(&[4, 5]), 2.0);
            p[12] = n(r.pick(&[0, -1]), 3.0);
        }
        B::Bossa => {
            p[0] = n(0, 3.0);
            p[6] = n(4, 2.0);
            p[8] = n(0, 3.0);
            p[14] = n(4, 2.0);
        }
    }
    p
}

fn make_pad(style: PadStyle, r: &mut Rng) -> [f32; 16] {
    let mut p = [0.0f32; 16];
    match style {
        P::Stab => {
            let c = [2usize, 3, 6, 7, 10, 11, 14];
            for _ in 0..3 {
                p[r.pick(&c)] = 1.0;
            }
        }
        P::Skank => {
            p[4] = 1.0;
            p[12] = 1.0;
            if r.f() < 0.4 {
                p[14] = 1.0;
            }
        }
        P::Rhodes | P::Comp | P::Pluck => {
            let c = [0usize, 3, 6, 8, 10, 11, 14];
            let n = 2 + (r.f() * 3.0) as usize;
            for _ in 0..n {
                p[r.pick(&c)] = r.pick(&[2.0, 3.0, 4.0, 6.0]);
            }
        }
        P::Swell | P::Pad => {}
    }
    p
}

fn compose(g: &Genre, r: &mut Rng) -> Song {
    let mut prog = g.prog;
    if r.f() < 0.35 {
        let i = (r.f() * prog.len() as f32) as usize % prog.len();
        prog[i] = r.pick(&[0, 1, 2, 3, 4, 5]);
    }
    let poly_len = r.pick(&[5usize, 7, 9, 11]);
    let mut poly = [None; 11];
    for slot in poly.iter_mut().take(poly_len) {
        *slot = if r.f() < 0.75 { Some((r.f() * 9.0) as i32) } else { None };
    }
    let mut song = Song {
        prog,
        poly_len,
        poly,
        bass: make_bass(g.bass, g, r),
        bass2: make_bass(g.bass, g, r),
        pad: make_pad(g.pad, r),
        acid: [AcidNote::default(); 16],
        lead: [LeadNote::default(); 32],
        arp: ArpOrder::Up,
    };
    match g.lead {
        L::Acid => {
            for i in 0..16 {
                if r.f() < 0.65 {
                    song.acid[i] = AcidNote {
                        on: true,
                        d: r.pick(&[0, 0, 2, 4, 5, 7, -1, 9]),
                        acc: r.f() < 0.3,
                        slide: r.f() < 0.25,
                        o: if r.f() < 0.2 { 1 } else { 0 },
                    };
                }
            }
        }
        L::Arp => {
            song.arp = r.pick(&[ArpOrder::Up, ArpOrder::Down, ArpOrder::UpDown, ArpOrder::Random]);
        }
        L::Bell => {
            let n = 4 + (r.f() * 3.0) as usize;
            for _ in 0..n {
                let i = (r.f() * 32.0) as usize % 32;
                song.lead[i] = LeadNote { on: true, d: r.pick(&[0, 2, 4, 6, 7, 9]), len: 4.0 };
            }
        }
        L::Melody => {
            let mut deg = r.pick(&[0, 2, 4]);
            let mut i = 0usize;
            while i < 16 {
                if r.f() < if i % 2 == 1 { 0.3 } else { 0.55 } {
                    let len = r.pick(&[1usize, 2, 2, 3, 4]);
                    song.lead[i] = LeadNote { on: true, d: deg, len: len as f32 };
                    i += len;
                    deg = (deg + r.pick(&[-2, -1, 1, 1, 2, 0, 3, -3])).clamp(-2, 9);
                } else {
                    i += 1;
                }
            }
            for i in 0..16 {
                song.lead[16 + i] = if i < 12 {
                    song.lead[i]
                } else if r.f() < 0.4 {
                    LeadNote { on: true, d: r.pick(&[0, 2, 4, 7]), len: 1.0 }
                } else {
                    LeadNote::default()
                };
            }
        }
    }
    song
}

// ------------------------------------------------------------- shared

/// Everything the UI and the audio thread tell each other. Controls go in as
/// atomics; the screen reads back counters and meters. No locks anywhere.
struct Shared {
    playing: AtomicBool,
    genre_sel: AtomicU32,
    weird: AtomicF32,
    space: AtomicF32,
    hold: AtomicBool,
    wander: AtomicBool,
    track_seed: AtomicU32,
    // engine -> UI
    section: AtomicU32,
    bar: AtomicU32,
    bpm: AtomicF32,
    melt: AtomicF32,
    kicks: AtomicU32,
    snares: AtomicU32,
    lo: AtomicF32,
    mid: AtomicF32,
    hi: AtomicF32,
    scale: AtomicU32,
    events: [AtomicU32; 16],
    event_head: AtomicU32,
}

impl Shared {
    fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            genre_sel: AtomicU32::new(0),
            weird: AtomicF32::new(0.35),
            space: AtomicF32::new(0.5),
            hold: AtomicBool::new(false),
            wander: AtomicBool::new(false),
            track_seed: AtomicU32::new(0),
            section: AtomicU32::new(0),
            bar: AtomicU32::new(0),
            bpm: AtomicF32::new(130.0),
            melt: AtomicF32::new(0.0),
            kicks: AtomicU32::new(0),
            snares: AtomicU32::new(0),
            lo: AtomicF32::new(0.0),
            mid: AtomicF32::new(0.0),
            hi: AtomicF32::new(0.0),
            scale: AtomicU32::new(0),
            events: std::array::from_fn(|_| AtomicU32::new(0)),
            event_head: AtomicU32::new(0),
        }
    }
}

#[derive(Clone, Copy)]
enum Ev { Switched = 1, Wandered, Mutated, SwingFlipped, ScaleChanged, ChordSlipped, DrumsBorrowed, TapeStopped, Insects, Choir, Stutter, NewTrack }

fn event_text(kind: u32, arg: u32) -> String {
    let genre = GENRES.get(arg as usize).map(|g| g.name).unwrap_or("?");
    match kind {
        1 => format!("Switched to {genre}"),
        2 => format!("Wandered into {genre}"),
        3 => "The song mutated".into(),
        4 => "Swing flipped backwards".into(),
        5 => format!("Scale changed to {}", SCALES.get(arg as usize).map(|s| s.name).unwrap_or("?")),
        6 => if arg == 1 { "Chord slid a tritone" } else { "Chord slipped off its key" }.into(),
        7 => format!("Drums borrowed from {genre}"),
        8 => "Tape slowed to a stop".into(),
        9 => "Insects in the left channel".into(),
        10 => "A choir appeared".into(),
        11 => "Stutter".into(),
        12 => "New track".into(),
        _ => String::new(),
    }
}

// ------------------------------------------------------------- DSP bits

fn mtof(m: f32) -> f32 {
    440.0 * ((m - 69.0) / 12.0).exp2()
}

fn poly_blep(t: f32, dt: f32) -> f32 {
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

#[derive(Clone, Copy, PartialEq, Default)]
enum Wave { #[default] Saw, Square, Tri, Sine }

fn osc(wave: Wave, p: f32, dt: f32) -> f32 {
    match wave {
        Wave::Saw => 2.0 * p - 1.0 - poly_blep(p, dt),
        Wave::Square => {
            let q = if p + 0.5 >= 1.0 { p - 0.5 } else { p + 0.5 };
            (if p < 0.5 { 1.0 } else { -1.0 }) + poly_blep(p, dt) - poly_blep(q, dt)
        }
        Wave::Tri => 4.0 * (p - 0.5).abs() - 1.0,
        Wave::Sine => (TAU * p).sin(),
    }
}

#[derive(Clone, Copy, PartialEq, Default)]
enum FKind { None, #[default] Lp, Bp, Hp }

/// RBJ biquad coefficients [b0, b1, b2, a1, a2], normalised. Low and high
/// pass take Q in decibels of resonance, band pass a plain Q: the same split
/// the Web Audio filters the original page was tuned with use.
fn biquad(kind: FKind, f: f32, q: f32, sr: f32) -> [f32; 5] {
    let f = f.clamp(10.0, sr * 0.49);
    let w = TAU * f / sr;
    let (s, c) = w.sin_cos();
    match kind {
        FKind::None => [1.0, 0.0, 0.0, 0.0, 0.0],
        FKind::Lp | FKind::Hp => {
            let alpha = s / (2.0 * 10f32.powf(q / 20.0));
            let a0 = 1.0 + alpha;
            let (b0, b1) = if kind == FKind::Lp { ((1.0 - c) / 2.0, 1.0 - c) } else { ((1.0 + c) / 2.0, -(1.0 + c)) };
            [b0 / a0, b1 / a0, b0 / a0, -2.0 * c / a0, (1.0 - alpha) / a0]
        }
        FKind::Bp => {
            let alpha = s / (2.0 * q.max(0.05));
            let a0 = 1.0 + alpha;
            [alpha / a0, 0.0, -alpha / a0, -2.0 * c / a0, (1.0 - alpha) / a0]
        }
    }
}

#[inline]
fn run_biquad(c: &[f32; 5], z: &mut [f32; 2], x: f32) -> f32 {
    let y = c[0] * x + z[0];
    z[0] = c[1] * x - c[3] * y + z[1];
    z[1] = c[2] * x - c[4] * y;
    y
}

#[derive(Clone, Copy, PartialEq, Default)]
enum Src { #[default] Osc, Fm, Noise }

#[derive(Clone, Copy, PartialEq, Default)]
enum Amp { #[default] Adsr, Exp, Swell }

#[derive(Clone, Copy, PartialEq, Default)]
enum Bus { Drums, #[default] Syn, Lead }

/// One note or hit. A fixed pool of these is the whole polyphony; a voice
/// is plain data, so starting one on the audio thread allocates nothing.
#[derive(Clone, Copy, Default)]
struct Voice {
    active: bool,
    delay: i32,
    age: u32,
    bus: Bus,
    pan: [f32; 2],
    src: Src,
    wave: Wave,
    nosc: usize,
    ph: [f32; 3],
    detr: [f32; 3],
    wob: bool,
    fcur: f32,
    gk: f32,
    glide_n: u32,
    nrate: f32,
    nph: f32,
    nval: f32,
    mph: f32,
    mfreq: f32,
    mdepth: f32,
    mdend: f32,
    mdk: f32,
    fk: FKind,
    fq: f32,
    fcut: f32,
    fenv: f32,
    fenv_k: f32,
    sweep_k: f32,
    fcur_f: f32,
    bq: [f32; 5],
    z: [f32; 2],
    fctr: u32,
    amp: Amp,
    level: f32,
    peak: f32,
    sus: f32,
    a_n: u32,
    rel_n: u32,
    end_n: u32,
    d_k: f32,
    r_k: f32,
    e_k: f32,
}

impl Voice {
    #[inline]
    fn tick(&mut self, sr: f32, wob: f32, rng: &mut Rng) -> f32 {
        if self.delay > 0 {
            self.delay -= 1;
            return 0.0;
        }
        let mut s = 0.0;
        match self.src {
            Src::Osc => {
                let m = if self.wob { wob } else { 1.0 };
                for i in 0..self.nosc {
                    let dt = (self.fcur * self.detr[i] * m / sr).min(0.45);
                    s += osc(self.wave, self.ph[i], dt);
                    let mut p = self.ph[i] + dt;
                    if p >= 1.0 {
                        p -= 1.0;
                    }
                    self.ph[i] = p;
                }
            }
            Src::Fm => {
                let m = if self.wob { wob } else { 1.0 };
                let depth = self.mdend + self.mdepth;
                self.mdepth *= self.mdk;
                s = (TAU * self.ph[0]).sin();
                let inst = self.fcur * self.detr[0] * m + depth * (TAU * self.mph).sin();
                let mut p = self.ph[0] + inst / sr;
                p -= p.floor();
                self.ph[0] = p;
                let mut mp = self.mph + self.mfreq / sr;
                if mp >= 1.0 {
                    mp -= 1.0;
                }
                self.mph = mp;
            }
            Src::Noise => {
                self.nph += self.nrate;
                if self.nph >= 1.0 {
                    self.nph -= 1.0;
                    self.nval = rng.f() * 2.0 - 1.0;
                }
                s = self.nval;
            }
        }
        if self.age < self.glide_n {
            self.fcur *= self.gk;
        }
        // filter
        if self.fk != FKind::None {
            if self.fctr == 0 {
                let f = if self.sweep_k != 1.0 {
                    self.fcur_f *= self.sweep_k.powi(16);
                    self.fcur_f
                } else {
                    self.fenv *= self.fenv_k;
                    self.fcut + self.fenv
                };
                self.bq = biquad(self.fk, f, self.fq, sr);
                self.fctr = 16;
            }
            self.fctr -= 1;
            s = run_biquad(&self.bq, &mut self.z, s);
        }
        // amplitude
        let age = self.age;
        match self.amp {
            Amp::Adsr => {
                if age < self.a_n {
                    self.level = self.peak * (age + 1) as f32 / self.a_n as f32;
                } else if age < self.rel_n {
                    self.level = self.sus + (self.level - self.sus) * self.d_k;
                } else {
                    self.level *= self.r_k;
                }
            }
            Amp::Exp => {
                if age < self.a_n {
                    self.level = self.peak * (age + 1) as f32 / self.a_n as f32;
                } else {
                    self.level *= self.e_k;
                }
            }
            Amp::Swell => self.level *= self.e_k,
        }
        self.age += 1;
        if self.age >= self.end_n || !self.level.is_finite() {
            self.active = false;
        }
        s * self.level
    }
}

fn pan_gains(p: f32) -> [f32; 2] {
    let x = (p.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
    [x.cos(), x.sin()]
}

/// A choir: up to four detuned sawtooth pairs through three vowel formants
/// that slide from one vowel to another over the note's length.
#[derive(Clone, Copy, Default)]
struct Choir {
    active: bool,
    delay: i32,
    age: u32,
    n: usize,
    ph: [f32; 8],
    fr: [f32; 8],
    pan: [[f32; 2]; 8],
    f0: [f32; 3],
    f1: [f32; 3],
    dur_n: u32,
    end_n: u32,
    z: [[[f32; 2]; 3]; 2],
    coefs: [[f32; 5]; 3],
    fctr: u32,
}

const VOWELS: [[f32; 3]; 5] = [[800., 1150., 2900.], [400., 1600., 2700.], [300., 2200., 3000.], [450., 800., 2830.], [325., 700., 2530.]];
const FORMANT_GAIN: [f32; 3] = [1.0, 0.55, 0.3];

impl Choir {
    fn tick(&mut self, sr: f32, wob: f32) -> [f32; 2] {
        if self.delay > 0 {
            self.delay -= 1;
            return [0.0; 2];
        }
        let mut x = [0.0f32; 2];
        for i in 0..self.n * 2 {
            let dt = (self.fr[i] * wob / sr).min(0.45);
            let y = osc(Wave::Saw, self.ph[i], dt);
            let mut p = self.ph[i] + dt;
            if p >= 1.0 {
                p -= 1.0;
            }
            self.ph[i] = p;
            x[0] += y * self.pan[i][0];
            x[1] += y * self.pan[i][1];
        }
        if self.fctr == 0 {
            let u = (self.age as f32 / self.dur_n.max(1) as f32).min(1.0);
            for k in 0..3 {
                self.coefs[k] = biquad(FKind::Bp, self.f0[k] + (self.f1[k] - self.f0[k]) * u, 9.0, sr);
            }
            self.fctr = 32;
        }
        self.fctr -= 1;
        let mut out = [0.0f32; 2];
        for ch in 0..2 {
            for k in 0..3 {
                out[ch] += run_biquad(&self.coefs[k], &mut self.z[ch][k], x[ch]) * FORMANT_GAIN[k];
            }
        }
        let a = self.age as f32;
        let d = self.dur_n as f32;
        let g = if a < d * 0.35 {
            0.32 * a / (d * 0.35).max(1.0)
        } else if a < d * 0.7 {
            0.32
        } else {
            0.32 * (-(a - d * 0.7) / (0.35 * sr)).exp()
        };
        self.age += 1;
        if self.age >= self.end_n {
            self.active = false;
        }
        [out[0] * g, out[1] * g]
    }
}

struct Comb {
    buf: Vec<f32>,
    len: usize,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(max: usize) -> Self {
        Comb { buf: vec![0.0; max], len: 1, pos: 0, store: 0.0 }
    }
    #[inline]
    fn run(&mut self, x: f32, fb: f32, damp: f32) -> f32 {
        let out = self.buf[self.pos];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.pos] = x + self.store * fb;
        self.pos += 1;
        if self.pos >= self.len {
            self.pos = 0;
        }
        out
    }
}

struct Allpass {
    buf: Vec<f32>,
    len: usize,
    pos: usize,
}

impl Allpass {
    fn new(max: usize) -> Self {
        Allpass { buf: vec![0.0; max], len: 1, pos: 0 }
    }
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let b = self.buf[self.pos];
        let y = b - x;
        self.buf[self.pos] = x + b * 0.5;
        self.pos += 1;
        if self.pos >= self.len {
            self.pos = 0;
        }
        y
    }
}

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const MAX_SR: f32 = 96_000.0;

struct Reverb {
    combs: [Vec<Comb>; 2],
    allpass: [Vec<Allpass>; 2],
}

impl Reverb {
    fn new() -> Self {
        let scale = MAX_SR / 44_100.0;
        let mk = |ch: usize| -> (Vec<Comb>, Vec<Allpass>) {
            (
                COMB_TUNING.iter().map(|&t| Comb::new(((t + ch * STEREO_SPREAD) as f32 * scale) as usize + 8)).collect(),
                ALLPASS_TUNING.iter().map(|&t| Allpass::new(((t + ch * STEREO_SPREAD) as f32 * scale) as usize + 8)).collect(),
            )
        };
        let (c0, a0) = mk(0);
        let (c1, a1) = mk(1);
        Reverb { combs: [c0, c1], allpass: [a0, a1] }
    }
    fn configure(&mut self, sr: f32) {
        let scale = sr / 44_100.0;
        for ch in 0..2 {
            for (i, c) in self.combs[ch].iter_mut().enumerate() {
                c.len = (((COMB_TUNING[i] + ch * STEREO_SPREAD) as f32 * scale) as usize).clamp(1, c.buf.len());
                c.buf.fill(0.0);
                c.pos = 0;
                c.store = 0.0;
            }
            for (i, a) in self.allpass[ch].iter_mut().enumerate() {
                a.len = (((ALLPASS_TUNING[i] + ch * STEREO_SPREAD) as f32 * scale) as usize).clamp(1, a.buf.len());
                a.buf.fill(0.0);
                a.pos = 0;
            }
        }
    }
    #[inline]
    fn run(&mut self, input: [f32; 2]) -> [f32; 2] {
        let mut out = [0.0; 2];
        for ch in 0..2 {
            let x = input[ch] * 0.06;
            let mut acc = 0.0;
            for c in self.combs[ch].iter_mut() {
                acc += c.run(x, 0.93, 0.25);
            }
            for a in self.allpass[ch].iter_mut() {
                acc = a.run(acc);
            }
            out[ch] = acc;
        }
        out
    }
}

// ------------------------------------------------------------- the engine

/// Parameters of a pitched note, mirroring the page's `synth()`.
#[derive(Clone, Copy)]
struct SynthP {
    midi: f32,
    dur: f32,
    wave: Wave,
    voices: usize,
    spread: f32,
    ft: FKind,
    q: f32,
    cut: f32,
    cenv: f32,
    cdec: f32,
    a: f32,
    d: f32,
    s: f32,
    r: f32,
    v: f32,
    pan: f32,
    bus: Bus,
    from: Option<f32>,
    glide: f32,
}

impl Default for SynthP {
    fn default() -> Self {
        SynthP {
            midi: 60.0, dur: 0.2, wave: Wave::Saw, voices: 1, spread: 14.0, ft: FKind::Lp, q: 0.8, cut: 3000.0, cenv: 0.0, cdec: 0.08,
            a: 0.005, d: 0.15, s: 0.6, r: 0.12, v: 0.2, pan: 0.0, bus: Bus::Syn, from: None, glide: 0.06,
        }
    }
}

/// Parameters of an FM bell/keys note, mirroring the page's `fm()`.
#[derive(Clone, Copy)]
struct FmP {
    midi: f32,
    dur: f32,
    ratio: f32,
    index: f32,
    dec: f32,
    a: f32,
    d: f32,
    s: f32,
    r: f32,
    v: f32,
    pan: f32,
    bus: Bus,
}

impl Default for FmP {
    fn default() -> Self {
        FmP { midi: 60.0, dur: 0.2, ratio: 2.0, index: 2.0, dec: 0.2, a: 0.003, d: 0.4, s: 0.15, r: 0.4, v: 0.15, pan: 0.0, bus: Bus::Syn }
    }
}

fn bass_voice(style: BassStyle) -> SynthP {
    let base = SynthP { bus: Bus::Drums, ..SynthP::default() };
    match style {
        B::Roll => SynthP { cut: 420.0, cenv: 2.5, cdec: 0.05, q: 5.0, d: 0.08, s: 0.3, v: 0.32, ..base },
        B::Offbeat => SynthP { cut: 600.0, cenv: 1.5, cdec: 0.08, q: 2.0, d: 0.12, s: 0.5, v: 0.34, ..base },
        B::Octave => SynthP { cut: 700.0, cenv: 1.2, cdec: 0.1, q: 2.0, d: 0.1, s: 0.5, v: 0.3, voices: 2, ..base },
        B::Sparse => SynthP { wave: Wave::Tri, cut: 600.0, d: 0.4, s: 0.7, v: 0.42, r: 0.4, ..base },
        B::Walk => SynthP { wave: Wave::Tri, cut: 850.0, d: 0.18, s: 0.45, v: 0.45, ..base },
        B::Sub => SynthP { wave: Wave::Sine, cut: 1200.0, d: 0.4, s: 0.9, v: 0.6, r: 0.06, ..base },
        B::Dub => SynthP { wave: Wave::Sine, cut: 500.0, d: 0.2, s: 0.85, v: 0.62, ..base },
        B::Bossa => SynthP { wave: Wave::Tri, cut: 700.0, d: 0.25, s: 0.4, v: 0.45, ..base },
    }
}

struct Engine {
    sr: f32,
    rng: Rng,
    voices: Vec<Voice>,
    choirs: [Choir; MAX_CHOIRS],
    // controls, snapshotted once per block
    weird: f32,
    space: f32,
    hold: bool,
    wander: bool,
    // transport and song state
    playing: bool,
    step: i64,
    clock: f32,
    song_bar: i64,
    cycle_bar: i64,
    sec: usize,
    genre: usize,
    song: Song,
    chord_deg: i32,
    shift: f32,
    walk: [i32; 4],
    borrow: Option<usize>,
    borrow_left: i32,
    tape_bar: i64,
    h_amt: f32,
    m: f32,
    pm: f32,
    bpm_now: f32,
    swing_flip: bool,
    scale: Option<usize>,
    last_bass: Option<f32>,
    last_acid: Option<f32>,
    time: f64,
    last_log: [f64; 16],
    // master chain
    lp_freq: f32,
    lp_target: f32,
    lp_bq: [f32; 5],
    lp_z: [[f32; 2]; 2],
    lp_ctr: u32,
    out_gain: f32,
    ring_ph: f32,
    ring_f: f32,
    ring_ft: f32,
    drive: f32,
    drive_k: f32,
    drive_norm: f32,
    wob_ph: [f32; 2],
    wob_g: [f32; 2],
    comp_env: f32,
    reverb: Reverb,
    dly: Vec<f32>,
    dly_pos: usize,
    dly_time: f32,
    dly_target: f32,
    dly_fb: f32,
    dly_lp: f32,
    dly_pan_ph: f32,
    // metering for the screen
    m_lo: f32,
    m_hi: f32,
    lo_env: f32,
    mid_env: f32,
    hi_env: f32,
    kicks: u32,
    snares: u32,
}

impl Engine {
    fn new() -> Self {
        let g = &GENRES[0];
        let mut rng = Rng::new(seed_from_clock());
        let song = compose(g, &mut rng);
        Engine {
            sr: 48_000.0,
            rng,
            voices: vec![Voice::default(); MAX_VOICES],
            choirs: [Choir::default(); MAX_CHOIRS],
            weird: 0.35,
            space: 0.5,
            hold: false,
            wander: false,
            playing: false,
            step: 0,
            clock: 0.0,
            song_bar: 0,
            cycle_bar: 0,
            sec: 0,
            genre: 0,
            song,
            chord_deg: 0,
            shift: 0.0,
            walk: [0, 2, 4, 5],
            borrow: None,
            borrow_left: 0,
            tape_bar: -1,
            h_amt: 0.0,
            m: 0.0,
            pm: 1.0,
            bpm_now: g.bpm,
            swing_flip: false,
            scale: None,
            last_bass: None,
            last_acid: None,
            time: 0.0,
            last_log: [-100.0; 16],
            lp_freq: 18_000.0,
            lp_target: 18_000.0,
            lp_bq: [1.0, 0.0, 0.0, 0.0, 0.0],
            lp_z: [[0.0; 2]; 2],
            lp_ctr: 0,
            out_gain: 0.0,
            ring_ph: 0.0,
            ring_f: 220.0,
            ring_ft: 220.0,
            drive: -1.0,
            drive_k: 1.0,
            drive_norm: 1.0,
            wob_ph: [0.0; 2],
            wob_g: [0.0; 2],
            comp_env: 0.0,
            reverb: Reverb::new(),
            dly: vec![0.0; (MAX_SR * 2.0) as usize + 8],
            dly_pos: 0,
            dly_time: 0.3,
            dly_target: 0.3,
            dly_fb: 0.4,
            dly_lp: 0.0,
            dly_pan_ph: 0.0,
            m_lo: 0.0,
            m_hi: 0.0,
            lo_env: 0.0,
            mid_env: 0.0,
            hi_env: 0.0,
            kicks: 0,
            snares: 0,
        }
    }

    fn configure(&mut self, sr: f32) {
        self.sr = sr.clamp(8_000.0, MAX_SR);
        self.reverb.configure(self.sr);
        self.dly.fill(0.0);
        self.lp_z = [[0.0; 2]; 2];
        self.lp_ctr = 0;
    }

    // ------------------------------------------------------ harmony

    fn scale_now(&self) -> &'static Scale {
        &SCALES[self.scale.unwrap_or(GENRES[self.genre].scale)]
    }

    fn deg_midi(&self, deg: i32, oct: i32) -> f32 {
        let sc = self.scale_now();
        let n = sc.len as i32;
        let o = deg.div_euclid(n);
        let i = deg.rem_euclid(n) as usize;
        GENRES[self.genre].root + sc.s[i] + 12.0 * (o + oct) as f32 + self.shift
    }

    fn chord_midis(&self, deg: i32, oct: i32, sev: bool) -> ([f32; 4], usize) {
        let mut out = [0.0; 4];
        let n = if sev { 4 } else { 3 };
        for (i, o) in out.iter_mut().enumerate().take(n) {
            *o = self.deg_midi(deg + 2 * i as i32, oct);
        }
        (out, n)
    }

    // ------------------------------------------------------ events

    fn ev(&mut self, sh: &Shared, kind: Ev, arg: u32) {
        let k = kind as usize;
        let exempt = matches!(kind, Ev::Switched | Ev::Wandered | Ev::NewTrack);
        if !exempt && self.time - self.last_log[k] < 6.0 {
            return;
        }
        self.last_log[k] = self.time;
        let bar = (self.step / 16 - self.song_bar + 1).clamp(0, 255) as u32;
        let head = sh.event_head.load(Ordering::Relaxed);
        sh.events[(head % 16) as usize].store((k as u32) << 24 | (arg & 0xff) << 16 | bar, Ordering::Relaxed);
        sh.event_head.store(head.wrapping_add(1), Ordering::Release);
    }

    // ------------------------------------------------------ voices

    fn alloc(&mut self) -> usize {
        if let Some(i) = self.voices.iter().position(|v| !v.active) {
            return i;
        }
        let mut best = 0;
        for (i, v) in self.voices.iter().enumerate() {
            if v.age > self.voices[best].age {
                best = i;
            }
        }
        best
    }

    fn jitter(&mut self) -> f32 {
        (self.rng.f() * 2.0 - 1.0) * self.weird * self.weird * 38.0
    }

    fn adsr(&self, v: &mut Voice, a: f32, d: f32, s: f32, dur: f32, r: f32, peak: f32) {
        let sr = self.sr;
        v.amp = Amp::Adsr;
        v.peak = peak;
        v.sus = peak * s;
        v.a_n = ((a * sr) as u32).max(1);
        v.d_k = (-1.0 / (sr * (d / 3.0).max(0.005))).exp();
        v.rel_n = (dur.max(a) * sr) as u32;
        v.r_k = (-1.0 / (sr * (r / 4.0).max(0.005))).exp();
        v.end_n = v.rel_n + (r * 1.6 * sr) as u32 + (0.05 * sr) as u32;
        v.level = 0.0;
    }

    fn synth(&mut self, delay: f32, p: SynthP) {
        let sr = self.sr;
        let j = self.jitter();
        let f = mtof(p.midi) * self.pm;
        let n = p.voices.clamp(1, 3);
        let i = self.alloc();
        let mut v = Voice { active: true, delay: (delay * sr) as i32, bus: p.bus, pan: pan_gains(p.pan), src: Src::Osc, wave: p.wave, nosc: n, wob: true, ..Voice::default() };
        for k in 0..n {
            let cents = if n > 1 { (k as f32 / (n - 1) as f32 - 0.5) * p.spread } else { 0.0 } + j;
            v.detr[k] = (cents / 1200.0).exp2();
        }
        v.fcur = f;
        if let Some(from) = p.from {
            let f0 = mtof(from) * self.pm;
            v.fcur = f0;
            v.glide_n = (p.glide * sr) as u32;
            v.gk = (f / f0).powf(1.0 / (p.glide * sr).max(1.0));
        }
        v.fk = p.ft;
        v.fq = p.q;
        let cut = p.cut.min(18_000.0);
        v.fcut = cut;
        v.fenv = cut * p.cenv;
        v.fenv_k = (-16.0 / (sr * p.cdec.max(0.001))).exp();
        v.sweep_k = 1.0;
        v.bq = biquad(p.ft, cut + v.fenv, p.q, sr);
        v.fctr = 16;
        self.adsr(&mut v, p.a, p.d, p.s, p.dur, p.r, p.v / (n as f32).sqrt());
        v.end_n += (0.03 * sr) as u32;
        self.voices[i] = v;
    }

    fn fm(&mut self, delay: f32, p: FmP) {
        let sr = self.sr;
        let j = self.jitter();
        let f = mtof(p.midi) * self.pm;
        let i = self.alloc();
        let mut v = Voice { active: true, delay: (delay * sr) as i32, bus: p.bus, pan: pan_gains(p.pan), src: Src::Fm, nosc: 1, wob: true, ..Voice::default() };
        v.detr[0] = (j / 1200.0).exp2();
        v.fcur = f;
        v.mfreq = f * p.ratio;
        let d0 = f * p.index;
        v.mdend = d0 * 0.08;
        v.mdepth = d0 - v.mdend;
        v.mdk = (-1.0 / (sr * p.dec.max(0.001))).exp();
        v.fk = FKind::None;
        self.adsr(&mut v, p.a, p.d, p.s, p.dur, p.r, p.v);
        v.end_n += (0.05 * sr) as u32;
        self.voices[i] = v;
    }

    fn noise_hit(&mut self, delay: f32, fk: FKind, f: f32, q: f32, dur: f32, vol: f32, bus: Bus) {
        let sr = self.sr;
        let i = self.alloc();
        let mut v = Voice { active: true, delay: (delay * sr) as i32, bus, pan: pan_gains(0.0), src: Src::Noise, nrate: self.pm.clamp(0.05, 1.0), fk, fq: q, fcut: f, sweep_k: 1.0, fenv_k: 1.0, ..Voice::default() };
        v.bq = biquad(fk, f, q, sr);
        v.fctr = u32::MAX / 2;
        v.amp = Amp::Exp;
        v.level = vol;
        v.peak = vol;
        v.a_n = 0;
        v.e_k = (0.0008f32 / vol.max(0.001)).powf(1.0 / (dur * sr).max(1.0));
        v.end_n = ((dur + 0.03) * sr) as u32;
        self.voices[i] = v;
    }

    fn tone(&mut self, delay: f32, wave: Wave, f0: f32, f1: f32, dur: f32, vol: f32, bus: Bus) {
        let sr = self.sr;
        let i = self.alloc();
        let a = f0 * self.pm;
        let b = (f1 * self.pm).max(20.0);
        let sweep = (dur * 0.6 * sr).max(1.0);
        let mut v = Voice { active: true, delay: (delay * sr) as i32, bus, pan: pan_gains(0.0), src: Src::Osc, wave, nosc: 1, fcur: a, ..Voice::default() };
        v.detr[0] = 1.0;
        v.glide_n = sweep as u32;
        v.gk = (b / a).powf(1.0 / sweep);
        v.fk = FKind::None;
        v.amp = Amp::Exp;
        v.level = vol;
        v.peak = vol;
        v.e_k = (0.0008f32 / vol.max(0.001)).powf(1.0 / (dur * sr).max(1.0));
        v.end_n = ((dur + 0.03) * sr) as u32;
        self.voices[i] = v;
    }

    fn drum(&mut self, sh: &Shared, k: Drum, delay: f32, v: f32) {
        use FKind::*;
        let (d, s) = (Bus::Drums, Bus::Syn);
        match k {
            Kick => {
                self.tone(delay, Wave::Sine, 160.0, 43.0, 0.42, 0.95 * v, d);
                self.tone(delay, Wave::Square, 1200.0, 300.0, 0.012, 0.08 * v, d);
                self.kicks = self.kicks.wrapping_add(1);
                sh.kicks.store(self.kicks, Ordering::Relaxed);
            }
            Snare => {
                self.noise_hit(delay, Bp, 1900.0, 0.8, 0.19, 0.5 * v, d);
                self.tone(delay, Wave::Tri, 210.0, 160.0, 0.09, 0.3 * v, d);
                self.snares = self.snares.wrapping_add(1);
                sh.snares.store(self.snares, Ordering::Relaxed);
            }
            Clap => {
                for i in 0..3 {
                    self.noise_hit(delay + i as f32 * 0.011, Bp, 1400.0, 1.4, 0.03, 0.45 * v, d);
                }
                self.noise_hit(delay + 0.033, Bp, 1300.0, 1.2, 0.22, 0.4 * v, d);
                self.snares = self.snares.wrapping_add(1);
                sh.snares.store(self.snares, Ordering::Relaxed);
            }
            Hat => {
                let dur = 0.035 + self.rng.f() * 0.02;
                self.noise_hit(delay, Hp, 7600.0, 0.7, dur, 0.16 * v, d);
            }
            Ghost => self.noise_hit(delay, Hp, 8500.0, 0.7, 0.025, 0.09 * v, d),
            Oh => self.noise_hit(delay, Hp, 6200.0, 0.7, 0.32, 0.13 * v, d),
            Ride => {
                self.noise_hit(delay, Bp, 5200.0, 1.5, 0.45, 0.14 * v, d);
                self.tone(delay, Wave::Square, 3150.0, 3150.0, 0.25, 0.012 * v, d);
            }
            Rim => {
                self.tone(delay, Wave::Square, 820.0, 800.0, 0.03, 0.12 * v, d);
                self.noise_hit(delay, Bp, 2400.0, 3.0, 0.03, 0.18 * v, d);
            }
            Perc => {
                let off = self.rng.pick(&[0, 2, 4]);
                let m = self.deg_midi(self.chord_deg + off, 2);
                self.tone(delay, Wave::Sine, mtof(m) * 1.6, mtof(m), 0.22, 0.28 * v, d);
            }
        }
        let _ = s;
    }

    fn choir(&mut self, delay: f32, dur: f32) {
        let sr = self.sr;
        let (midis, n) = self.chord_midis(self.chord_deg, 1, true);
        let slot = self.choirs.iter().position(|c| !c.active).unwrap_or_else(|| {
            let mut best = 0;
            for (i, c) in self.choirs.iter().enumerate() {
                if c.age > self.choirs[best].age {
                    best = i;
                }
            }
            best
        });
        let v1 = self.rng.pick(&VOWELS);
        let v2 = self.rng.pick(&VOWELS);
        let mut c = Choir { active: true, delay: (delay * sr) as i32, n, f0: v1, f1: v2, dur_n: (dur * sr) as u32, end_n: ((dur + 1.2) * sr) as u32, ..Choir::default() };
        for k in 0..n {
            let pan = pan_gains((k as f32 / (n.max(2) - 1) as f32) * 1.4 - 0.7);
            for d in 0..2 {
                let idx = k * 2 + d;
                let cents = if d == 1 { 9.0 } else { -9.0 } + self.jitter();
                c.fr[idx] = mtof(midis[k]) * self.pm * (cents / 1200.0).exp2();
                c.pan[idx] = pan;
            }
        }
        self.choirs[slot] = c;
    }

    fn reverse_swell(&mut self, delay: f32, dur: f32) {
        let sr = self.sr;
        let i = self.alloc();
        let mut v = Voice { active: true, delay: (delay * sr) as i32, bus: Bus::Syn, pan: pan_gains(0.0), src: Src::Noise, nrate: 1.0, fk: FKind::Bp, fq: 2.0, fcut: 300.0, fenv_k: 1.0, ..Voice::default() };
        v.fcur_f = 300.0;
        v.sweep_k = (7000.0f32 / 300.0).powf(1.0 / (dur * sr).max(1.0));
        v.bq = biquad(FKind::Bp, 300.0, 2.0, sr);
        v.amp = Amp::Swell;
        v.level = 0.0008;
        v.e_k = (0.35f32 / 0.0008).powf(1.0 / (dur * sr).max(1.0));
        v.end_n = (dur * sr) as u32;
        self.voices[i] = v;
    }

    fn insect(&mut self, delay: f32) {
        let sr = self.sr;
        let f = 2500.0 + self.rng.f() * 4500.0;
        let f1 = f * (0.5 + self.rng.f() * 1.5);
        let mod_f = 25.0 + self.rng.f() * 90.0;
        let depth = 1500.0 + self.rng.f() * 2500.0;
        let pan = self.rng.f() * 1.8 - 0.9;
        let dur = 0.05 + self.rng.f() * 0.18;
        let i = self.alloc();
        let mut v = Voice { active: true, delay: (delay * sr) as i32, bus: Bus::Lead, pan: pan_gains(pan), src: Src::Fm, nosc: 1, fcur: f, ..Voice::default() };
        v.detr[0] = 1.0;
        v.glide_n = (0.15 * sr) as u32;
        v.gk = (f1 / f).powf(1.0 / (0.15 * sr));
        v.mfreq = mod_f;
        v.mdend = depth;
        v.mdk = 1.0;
        v.fk = FKind::None;
        v.amp = Amp::Exp;
        v.peak = 0.05;
        v.a_n = (0.01 * sr) as u32;
        v.e_k = (0.0008f32 / 0.05).powf(1.0 / ((dur - 0.01).max(0.01) * sr));
        v.end_n = ((dur + 0.05) * sr) as u32;
        self.voices[i] = v;
    }

    // ------------------------------------------------- sequencer

    fn compose_now(&mut self) {
        self.song = compose(&GENRES[self.genre], &mut self.rng);
    }

    fn start(&mut self, sh: &Shared) {
        self.playing = true;
        self.genre = (sh.genre_sel.load(Ordering::Relaxed) as usize).min(GENRES.len() - 1);
        self.compose_now();
        self.step = 0;
        self.clock = 0.0;
        self.time = 0.0;
        self.song_bar = 0;
        self.h_amt = 0.0;
        self.borrow = None;
        self.borrow_left = 0;
        self.scale = None;
        self.tape_bar = -1;
        self.last_bass = None;
        self.last_acid = None;
        self.last_log = [-100.0; 16];
        self.apply_mix(true);
    }

    fn stop(&mut self) {
        self.playing = false;
        for v in self.voices.iter_mut() {
            v.active = false;
        }
        for c in self.choirs.iter_mut() {
            c.active = false;
        }
    }

    fn step_dur(&mut self) -> f32 {
        let g = &GENRES[self.genre];
        let s16 = self.step.rem_euclid(16);
        let bar = self.step.div_euclid(16);
        let tp = if bar == self.tape_bar { s16 as f32 / 16.0 } else { 0.0 };
        self.h_amt += ((if self.hold { 1.0 } else { 0.0 }) - self.h_amt) * 0.3;
        let m = tp.max(self.h_amt);
        self.m = m;
        self.pm = 1.0 - 0.72 * m;
        let wob = 1.0 + self.weird * self.weird * 0.08 * (self.time as f32 * 0.31).sin();
        self.bpm_now = g.bpm * wob * (1.0 - 0.6 * m);
        let base = 60.0 / self.bpm_now / 4.0;
        let sw = g.swing * 0.33 * if self.swing_flip { -1.0 } else { 1.0 };
        base * if s16 % 2 == 0 { 1.0 + sw } else { 1.0 - sw }
    }

    fn on_bar(&mut self, sh: &Shared, bar: i64, sd: f32) {
        let w = self.weird;
        let mut cb = bar - self.song_bar;
        let sel = (sh.genre_sel.load(Ordering::Relaxed) as usize).min(GENRES.len() - 1);
        if sel != self.genre {
            self.genre = sel;
            self.song_bar = bar - 4;
            cb = 4;
            self.compose_now();
            self.scale = None;
            self.apply_mix(false);
            self.ev(sh, Ev::Switched, sel as u32);
        }
        if cb >= SONG_BARS {
            self.song_bar = bar;
            cb = 0;
            if self.wander && self.rng.chance(0.7) {
                let mut n = (self.rng.f() * (GENRES.len() - 1) as f32) as usize;
                if n >= self.genre {
                    n += 1;
                }
                self.genre = n.min(GENRES.len() - 1);
                sh.genre_sel.store(self.genre as u32, Ordering::Relaxed);
                self.apply_mix(false);
                self.ev(sh, Ev::Wandered, self.genre as u32);
            } else {
                self.ev(sh, Ev::Mutated, 0);
            }
            self.compose_now();
        }
        self.cycle_bar = cb;
        let si = section_at(cb);
        let sec = &SECTIONS[si];
        let new_sec = sec.start == cb;
        self.sec = si;
        let g = &GENRES[self.genre];
        let prog_i = cb.div_euclid(g.bars).rem_euclid(4) as usize;
        self.chord_deg = self.song.prog[prog_i];
        if new_sec {
            self.swing_flip = w > 0.5 && self.rng.chance(w * 0.4);
            if self.swing_flip && g.swing > 0.05 {
                self.ev(sh, Ev::SwingFlipped, 0);
            }
            let ns = if w > 0.5 && self.rng.chance(w - 0.35) { Some(WEIRD_SCALE_FIRST + (self.rng.f() * 5.0) as usize % 5) } else { None };
            if let Some(n) = ns {
                if ns != self.scale {
                    self.ev(sh, Ev::ScaleChanged, n as u32);
                }
            }
            self.scale = ns;
        }
        self.shift = 0.0;
        if w > 0.25 && self.rng.chance(w * 0.2) {
            self.shift = self.rng.pick(&[1.0, -1.0, 6.0, 3.0]);
            self.ev(sh, Ev::ChordSlipped, if self.shift == 6.0 { 1 } else { 0 });
        }
        self.walk = [0, self.rng.pick(&[1, 2]), self.rng.pick(&[2, 4]), self.rng.pick(&[3, 5, 6, -1])];
        if self.borrow_left > 0 {
            self.borrow_left -= 1;
            if self.borrow_left == 0 {
                self.borrow = None;
            }
        } else if w > 0.15 && self.rng.chance(w * 0.22) {
            let mut n = (self.rng.f() * (GENRES.len() - 1) as f32) as usize;
            if n >= self.genre {
                n += 1;
            }
            n = n.min(GENRES.len() - 1);
            if GENRES[n].id == "ambient" {
                n = (n + 1) % GENRES.len();
                if n == self.genre {
                    n = (n + 1) % GENRES.len();
                }
            }
            self.borrow = Some(n);
            self.borrow_left = self.rng.pick(&[1, 2]);
            self.ev(sh, Ev::DrumsBorrowed, n as u32);
        }
        self.tape_bar = if w > 0.4 && cb % 8 == 7 && self.rng.chance(w * 0.55) { bar } else { -1 };
        if self.tape_bar == bar {
            self.ev(sh, Ev::TapeStopped, 0);
        }
        if cb == sec.end - 1 && self.rng.chance(0.35 + w * 0.5) {
            self.reverse_swell(0.0, 16.0 * sd);
        }
        let bugs = (w * w * 7.0 * self.rng.f()) as usize;
        for _ in 0..bugs {
            let at = self.rng.f() * 16.0 * sd;
            self.insect(at);
        }
        if bugs > 3 {
            self.ev(sh, Ev::Insects, 0);
        }
        if cb % 2 == 0 && ((sec.choir && w > 0.15) || (w > 0.55 && self.rng.chance(w * 0.25))) {
            self.choir(0.0, 32.0 * sd);
            self.ev(sh, Ev::Choir, 0);
        }
        self.ring_ft = mtof(self.deg_midi(self.chord_deg, 1)) * (1.5 + w * 1.3);
        sh.section.store(si as u32, Ordering::Relaxed);
        sh.bar.store(cb.max(0) as u32, Ordering::Relaxed);
        sh.scale.store(self.scale.map(|s| s as u32 + 1).unwrap_or(0), Ordering::Relaxed);
    }

    fn schedule_step(&mut self, sh: &Shared, sd: f32) {
        let seed = sh.track_seed.swap(0, Ordering::AcqRel);
        if seed != 0 {
            self.rng = Rng::new(seed);
            self.compose_now();
            self.song_bar = self.step.div_euclid(16) + 1;
            self.ev(sh, Ev::NewTrack, 0);
        }
        let s16 = self.step.rem_euclid(16) as usize;
        let bar = self.step.div_euclid(16);
        if s16 == 0 {
            self.on_bar(sh, bar, sd);
        }
        let g = &GENRES[self.genre];
        let sec = &SECTIONS[self.sec];
        let layers = sec.layers;
        let w = self.weird;
        let song = self.song;
        let deg = self.chord_deg;

        // master low-pass: genre colour, build-up sweep, melt
        let lp_base = if g.lp >= 1.0 { 18_000.0 } else { 600.0 + g.lp * 12_000.0 };
        let mut target = lp_base;
        if sec.build && self.cycle_bar == sec.end - 1 {
            target = 900.0 + (lp_base - 900.0) * (s16 as f32 / 16.0).powi(2);
        }
        self.lp_target = (target * (1.0 - 0.88 * self.m)).max(180.0);
        if g.crackle && self.rng.chance(0.35) {
            let at = self.rng.f() * sd;
            let v = 0.05 + self.rng.f() * 0.12;
            let dur = 0.004 + self.rng.f() * 0.006;
            self.noise_hit(at, FKind::Hp, 2500.0, 0.7, dur, v, Bus::Syn);
        }

        // drums
        let dg = &GENRES[self.borrow.unwrap_or(self.genre)];
        let mut hits = [(Kick, 0.0f32); 12];
        let mut nh = 0;
        for (k, pat) in dg.drums.iter() {
            let c = pat.as_bytes().get(s16).copied().unwrap_or(b'.');
            let v = match c {
                b'x' => 1.0,
                b'o' => 0.5,
                b'?' => if self.rng.chance(0.5) { 0.8 } else { 0.0 },
                _ => 0.0,
            };
            if v == 0.0 {
                continue;
            }
            let layer = match k {
                Kick => LAYER_K,
                Snare | Clap | Rim => LAYER_S,
                Perc => LAYER_P,
                _ => LAYER_H,
            };
            if layers & layer != 0 && nh < hits.len() {
                hits[nh] = (*k, v);
                nh += 1;
            }
        }
        if sec.build && self.cycle_bar == sec.end - 1 && (s16 >= 8 || s16 % 2 == 0) && nh < hits.len() {
            let rv = [Snare, Clap, Rim].into_iter().find(|r| dg.drums.iter().any(|(d, _)| d == r));
            if let Some(r) = rv {
                hits[nh] = (r, 0.25 + 0.75 * s16 as f32 / 16.0);
                nh += 1;
            }
        }
        for &(k, v) in hits.iter().take(nh) {
            if k == Hat && dg.hat_rolls && self.rng.chance(0.12) {
                let n = self.rng.pick(&[3usize, 4, 6]);
                for j in 0..n {
                    self.drum(sh, Hat, j as f32 * sd / n as f32, v * (0.55 + 0.45 * j as f32 / n as f32));
                }
            } else {
                self.drum(sh, k, 0.0, v);
            }
        }
        if w > 0.35 && nh > 0 && self.rng.chance(w * w * 0.1) {
            let n = self.rng.pick(&[3usize, 4, 6, 8]);
            for j in 1..n {
                for &(k, v) in hits.iter().take(nh) {
                    self.drum(sh, k, j as f32 * sd * 2.0 / n as f32, v * 0.8f32.powi(j as i32));
                }
            }
            self.ev(sh, Ev::Stutter, 0);
        }

        // bass
        if layers & LAYER_BASS != 0 {
            let b = (if self.cycle_bar % 4 == 3 { song.bass2 } else { song.bass })[s16];
            if b.on {
                let d = if b.walk { self.walk[s16 >> 2] } else { b.d };
                let midi = self.deg_midi(deg + d, 0);
                let mut p = bass_voice(g.bass);
                p.midi = midi;
                p.dur = b.len * sd;
                p.from = if b.slide { self.last_bass } else { None };
                p.glide = 0.09;
                p.cut *= if b.acc { 1.6 } else { 1.0 };
                self.synth(0.0, p);
                self.last_bass = Some(midi);
            }
        }

        // chords
        if layers & LAYER_PAD != 0 {
            match g.pad {
                P::Pad | P::Swell => {
                    if s16 == 0 && self.cycle_bar % g.bars == 0 {
                        let dur = g.bars as f32 * 16.0 * sd * 0.98;
                        let swell = g.pad == P::Swell;
                        let (ms, n) = self.chord_midis(deg, 1, g.sev);
                        for (i, &m) in ms.iter().enumerate().take(n) {
                            self.synth(0.0, SynthP {
                                midi: m, dur, voices: 3, spread: if swell { 24.0 } else { 14.0 },
                                cut: if swell { 900.0 } else { 1800.0 * g.lp + 300.0 },
                                a: if swell { 1.6 } else { 0.3 }, d: 1.0, s: 0.8, r: if swell { 2.5 } else { 1.2 }, v: 0.1,
                                pan: (i as f32 - 1.5) * 0.3, ..SynthP::default()
                            });
                        }
                    }
                }
                style => {
                    let len = song.pad[s16];
                    if len > 0.0 {
                        let (ms, n) = self.chord_midis(deg, 1, g.sev);
                        let dur = len * sd;
                        for (i, &m) in ms.iter().enumerate().take(n) {
                            match style {
                                P::Stab => self.synth(0.0, SynthP {
                                    midi: m + 12.0, dur: sd * 0.6, voices: 2, cut: 900.0 + w * 1500.0, cenv: 3.0, cdec: 0.06, q: 3.0,
                                    d: 0.1, s: 0.0, r: 0.15, v: 0.11, ..SynthP::default()
                                }),
                                P::Skank => self.synth(0.0, SynthP {
                                    midi: m + 12.0, dur: sd * 0.5, wave: Wave::Square, voices: 2, ft: FKind::Bp, cut: 1500.0, q: 1.2,
                                    d: 0.08, s: 0.0, r: 0.1, v: 0.16, bus: Bus::Lead, ..SynthP::default()
                                }),
                                P::Pluck => self.synth(i as f32 * 0.012, SynthP {
                                    midi: m, dur, wave: Wave::Tri, cut: 2400.0, cenv: 1.0, cdec: 0.1, d: 0.3, s: 0.0, r: 0.3, v: 0.12, ..SynthP::default()
                                }),
                                _ => self.fm(i as f32 * 0.008, FmP {
                                    midi: m, dur, ratio: 1.0, index: 1.4, dec: 0.5, d: 0.8, s: 0.3, r: 0.5, v: 0.09, pan: (i as f32 - 1.5) * 0.25, ..FmP::default()
                                }),
                            }
                        }
                    }
                }
            }
        }

        // lead
        if layers & LAYER_LEAD != 0 {
            let i32_ = (self.cycle_bar.rem_euclid(2) as usize) * 16 + s16;
            match g.lead {
                L::Acid => {
                    let n = song.acid[s16];
                    if n.on {
                        let midi = self.deg_midi(deg + n.d, 1 + n.o);
                        let cut = 300.0 + 1600.0 * (0.5 + 0.5 * (bar as f32 * 0.45).sin()) + w * 1400.0;
                        self.synth(0.0, SynthP {
                            midi, dur: sd * if n.slide { 1.05 } else { 0.6 }, q: 13.0, cut, cenv: if n.acc { 4.0 } else { 2.0 }, cdec: 0.12,
                            d: 0.15, s: 0.4, r: 0.05, v: if n.acc { 0.2 } else { 0.13 }, from: if n.slide { self.last_acid } else { None },
                            glide: 0.07, bus: Bus::Lead, ..SynthP::default()
                        });
                        self.last_acid = Some(midi);
                    }
                }
                L::Arp => {
                    let (lo, nl) = self.chord_midis(deg, 2, g.sev);
                    let (hi, nh2) = self.chord_midis(deg, 3, g.sev);
                    let mut tones = [0.0f32; 8];
                    tones[..nl].copy_from_slice(&lo[..nl]);
                    tones[nl..nl + nh2].copy_from_slice(&hi[..nh2]);
                    let n = nl + nh2;
                    let mut i = s16 % n;
                    match song.arp {
                        ArpOrder::Down => i = n - 1 - i,
                        ArpOrder::UpDown => {
                            let c = s16 % (2 * n - 2);
                            i = if c < n { c } else { 2 * n - 2 - c };
                        }
                        ArpOrder::Random => i = (self.rng.f() * n as f32) as usize % n,
                        ArpOrder::Up => {}
                    }
                    self.synth(0.0, SynthP {
                        midi: tones[i], dur: sd * 0.7, wave: if g.id == "dnb" { Wave::Tri } else { Wave::Saw }, voices: 2, cut: 2400.0, cenv: 1.0,
                        cdec: 0.06, d: 0.1, s: 0.2, r: 0.12, v: 0.1, bus: Bus::Lead, pan: (self.step as f32 * 0.7).sin() * 0.5, ..SynthP::default()
                    });
                }
                L::Bell => {
                    let n = song.lead[i32_];
                    if n.on {
                        let midi = self.deg_midi(deg + n.d, 3);
                        let pan = self.rng.f() - 0.5;
                        self.fm(0.0, FmP { midi, dur: 1.2, ratio: 3.5, index: 3.0, dec: 0.5, v: 0.11, pan, bus: Bus::Lead, ..FmP::default() });
                    }
                }
                L::Melody => {
                    let n = song.lead[i32_];
                    if n.on {
                        let midi = self.deg_midi(deg + n.d, 2);
                        let dur = n.len * sd * 0.9;
                        match g.id {
                            "jazz" => self.fm(0.0, FmP { midi, dur, ratio: 4.0, index: 1.6, dec: 0.25, v: 0.13, bus: Bus::Lead, ..FmP::default() }),
                            "bossa" => self.synth(0.0, SynthP { midi, dur, wave: Wave::Tri, cut: 3000.0, a: 0.04, d: 0.2, s: 0.8, r: 0.15, v: 0.14, bus: Bus::Lead, ..SynthP::default() }),
                            "dub" => self.synth(0.0, SynthP { midi, dur, wave: Wave::Square, voices: 2, cut: 1600.0, a: 0.03, d: 0.2, s: 0.7, r: 0.1, v: 0.09, bus: Bus::Lead, ..SynthP::default() }),
                            _ => self.synth(0.0, SynthP { midi, dur, wave: Wave::Square, cut: 2200.0, cenv: 1.0, cdec: 0.1, d: 0.2, s: 0.35, r: 0.1, v: 0.1, bus: Bus::Lead, ..SynthP::default() }),
                        }
                    }
                }
            }
        }

        // the polymeter ghost: a glass arpeggio in 5, 7, 9 or 11 over the 4/4
        if w > 0.3 && (layers & LAYER_WEIRD != 0 || w > 0.55) {
            let idx = self.step.rem_euclid(song.poly_len as i64) as usize;
            if let Some(pd) = song.poly[idx] {
                if self.rng.chance(0.35 + w * 0.5) {
                    let midi = self.deg_midi(deg + pd, 3);
                    self.fm(0.0, FmP {
                        midi, dur: sd * 1.5, ratio: 2.71 + w, index: 2.0 + w * 5.0, dec: 0.15, v: 0.04 + w * 0.07,
                        pan: if idx % 2 == 1 { -0.75 } else { 0.75 }, bus: Bus::Lead, ..FmP::default()
                    });
                }
            }
        }
    }

    // ------------------------------------------------- mixing

    /// Re-derives everything that depends on the Weirdness, Space and genre
    /// settings and isn't touched per sample.
    fn apply_mix(&mut self, force: bool) {
        let g = &GENRES[self.genre];
        let (w, sp) = (self.weird, self.space);
        self.dly_target = 60.0 / g.bpm * 0.75;
        self.dly_fb = (0.28 + sp * 0.3 + w * 0.18).min(0.85);
        self.wob_g = [w * w * 48.0, w * 7.0];
        let drive = ((g.drive + w * 0.3) * 40.0).round() / 40.0;
        if force || drive != self.drive {
            self.drive = drive;
            self.drive_k = 1.0 + drive * 22.0;
            self.drive_norm = self.drive_k.tanh();
        }
    }

    fn begin_block(&mut self, sh: &Shared, target_gain: f32) {
        let playing = sh.playing.load(Ordering::Relaxed);
        self.weird = sh.weird.get().clamp(0.0, 1.0);
        self.space = sh.space.get().clamp(0.0, 1.0);
        self.hold = sh.hold.load(Ordering::Relaxed);
        self.wander = sh.wander.load(Ordering::Relaxed);
        if playing && !self.playing {
            self.start(sh);
        } else if !playing && self.playing {
            self.stop();
        }
        self.apply_mix(false);
        let _ = target_gain;
    }

    fn sample(&mut self, sh: &Shared, gain: f32) -> [f32; 2] {
        let sr = self.sr;
        if self.playing {
            self.clock -= 1.0;
            if self.clock <= 0.0 {
                let sd = self.step_dur();
                self.schedule_step(sh, sd);
                self.clock += sd * sr;
                self.time += sd as f64;
                self.step += 1;
            }
        }
        // shared seasick pitch drift for every tonal voice
        self.wob_ph[0] = (self.wob_ph[0] + 0.19 / sr).fract();
        self.wob_ph[1] = (self.wob_ph[1] + 5.3 / sr).fract();
        let cents = self.wob_g[0] * (TAU * self.wob_ph[0]).sin() + self.wob_g[1] * (TAU * self.wob_ph[1]).sin();
        let wob = (cents / 1200.0).exp2();

        let mut bus = [[0.0f32; 2]; 3];
        for v in self.voices.iter_mut() {
            if v.active {
                let s = v.tick(sr, wob, &mut self.rng);
                let b = v.bus as usize;
                bus[b][0] += s * v.pan[0];
                bus[b][1] += s * v.pan[1];
            }
        }
        for c in self.choirs.iter_mut() {
            if c.active {
                let s = c.tick(sr, wob);
                bus[Bus::Lead as usize][0] += s[0];
                bus[Bus::Lead as usize][1] += s[1];
            }
        }
        let g = &GENRES[self.genre];
        let sp = self.space;
        let dub_d = if g.dub { 2.2 } else { 1.0 };
        let dub_l = if g.dub { 1.7 } else { 1.0 };
        let mut pre = [0.0f32; 2];
        let mut rev_in = [0.0f32; 2];
        let mut dly_in = 0.0;
        for ch in 0..2 {
            let d = bus[Bus::Drums as usize][ch] * 0.85;
            let s = bus[Bus::Syn as usize][ch] * 0.55;
            let l = bus[Bus::Lead as usize][ch] * 0.6;
            pre[ch] = d + s + l;
            rev_in[ch] = d * (0.06 + sp * 0.16) + s * (0.18 + sp * 0.6) + l * (0.22 + sp * 0.6);
            dly_in += 0.5 * (s * sp * 0.22 * dub_d + l * (0.12 + sp * 0.4 * dub_l));
        }

        // ring modulation fades in from Weirdness 0.3
        let w = self.weird;
        let ring = if w > 0.3 { (w - 0.3) * 0.75 } else { 0.0 };
        self.ring_f += (self.ring_ft - self.ring_f) * (1.0 - (-1.0 / (0.05 * sr)).exp());
        self.ring_ph = (self.ring_ph + self.ring_f / sr).fract();
        let rm = (TAU * self.ring_ph).sin();
        let mut x = [0.0f32; 2];
        for ch in 0..2 {
            let mixed = pre[ch] * (1.0 - ring * 0.6) + pre[ch] * rm * ring;
            x[ch] = (mixed * self.drive_k).tanh() / self.drive_norm;
        }

        // reverb and the wandering-pan delay join after the shaper
        let rv = self.reverb.run(rev_in);
        let n = self.dly.len();
        self.dly_time += (self.dly_target - self.dly_time) * (1.0 - (-1.0 / (0.06 * sr)).exp());
        let d_samples = (self.dly_time * sr).clamp(1.0, n as f32 - 4.0);
        let rp = (self.dly_pos as f32 + n as f32 - d_samples) % n as f32;
        let i0 = rp as usize;
        let fr = rp - i0 as f32;
        let dl = self.dly[i0 % n] * (1.0 - fr) + self.dly[(i0 + 1) % n] * fr;
        let lp_k = 1.0 - (-TAU * 2600.0 / sr).exp();
        self.dly_lp += (dl - self.dly_lp) * lp_k;
        self.dly[self.dly_pos] = dly_in + self.dly_lp * self.dly_fb;
        self.dly_pos = (self.dly_pos + 1) % n;
        self.dly_pan_ph = (self.dly_pan_ph + 0.23 / sr).fract();
        let dpg = pan_gains((TAU * self.dly_pan_ph).sin() * 0.85);
        let dly_out = [self.dly_lp * dpg[0], self.dly_lp * dpg[1]];

        // master low-pass
        self.lp_freq += (self.lp_target - self.lp_freq) * (1.0 - (-1.0 / (0.04 * sr)).exp());
        if self.lp_ctr == 0 {
            self.lp_bq = biquad(FKind::Lp, self.lp_freq, 0.9, sr);
            self.lp_ctr = 16;
        }
        self.lp_ctr -= 1;
        let mut y = [0.0f32; 2];
        for ch in 0..2 {
            y[ch] = run_biquad(&self.lp_bq, &mut self.lp_z[ch], x[ch] + rv[ch] + dly_out[ch]);
        }

        // compressor: -16 dB, 4:1, 5 ms / 200 ms, with fixed makeup
        let det = y[0].abs().max(y[1].abs());
        let k = if det > self.comp_env { 1.0 - (-1.0 / (0.005 * sr)).exp() } else { 1.0 - (-1.0 / (0.2 * sr)).exp() };
        self.comp_env += (det - self.comp_env) * k;
        const THRESH: f32 = 0.158;
        let gr = if self.comp_env > THRESH { (self.comp_env / THRESH).powf(0.25 - 1.0) } else { 1.0 };

        let tg = if self.playing { OUT_LEVEL } else { 0.0 };
        self.out_gain += (tg - self.out_gain) * (1.0 - (-1.0 / ((if self.playing { 0.02 } else { 0.08 }) * sr)).exp());
        let g_total = gr * 2.3 * self.out_gain * gain;
        let out = [soft_clip(y[0] * g_total), soft_clip(y[1] * g_total)];

        // three crude bands for the screen
        let mono = 0.5 * (out[0] + out[1]);
        self.m_lo += (mono - self.m_lo) * 0.012;
        self.m_hi += (mono - self.m_hi) * 0.4;
        let hi = mono - self.m_hi;
        let mid = self.m_hi - self.m_lo;
        let rel = 1.0 - (-1.0 / (0.12 * sr)).exp();
        self.lo_env = (self.m_lo.abs() * 3.0).max(self.lo_env + (0.0 - self.lo_env) * rel);
        self.mid_env = (mid.abs() * 2.0).max(self.mid_env + (0.0 - self.mid_env) * rel);
        self.hi_env = (hi.abs() * 2.0).max(self.hi_env + (0.0 - self.hi_env) * rel);
        out
    }

    fn publish_meters(&mut self, sh: &Shared) {
        sh.lo.set(self.lo_env.min(1.0));
        sh.mid.set(self.mid_env.min(1.0));
        sh.hi.set(self.hi_env.min(1.0));
        sh.bpm.set(if self.playing { self.bpm_now } else { GENRES[self.genre].bpm });
        sh.melt.set(self.m);
    }
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a < 0.8 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * ((a - 0.8) / 0.2).tanh())
    }
}

fn seed_from_clock() -> u32 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    ((t ^ (t >> 32)) as u32).max(1)
}

// --------------------------------------------------------- audio side

struct MeltProcessor {
    shared: Arc<Shared>,
    engine: Box<Engine>,
    level: Arc<AtomicF32>,
    ext: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    gain: f32,
}

impl AudioProcessor for MeltProcessor {
    fn is_active(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed) || self.engine.playing || self.engine.out_gain > 1e-4
    }

    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        if channels == 0 || !sample_rate.is_finite() || sample_rate < 8000.0 {
            buffer.fill(0.0);
            return;
        }
        if (sample_rate - self.engine.sr).abs() > 1.0 {
            self.engine.configure(sample_rate);
        }
        let target = (self.level.get() + self.ext.get()).clamp(0.0, 2.0);
        self.engine.begin_block(&self.shared, target);
        let coef = 1.0 - (-1.0 / (0.01 * sample_rate)).exp();
        let mut published = self.output.try_lock().ok();
        if let Some(b) = published.as_mut() {
            let len = (buffer.len() / channels).min(b.capacity());
            b.resize(len, 0.0);
            b.fill(0.0);
        }
        for (i, frame) in buffer.chunks_mut(channels).enumerate() {
            self.gain += (target - self.gain) * coef;
            let o = self.engine.sample(&self.shared, self.gain);
            let (l, r) = if o[0].is_finite() && o[1].is_finite() { (o[0], o[1]) } else { (0.0, 0.0) };
            for (ch, s) in frame.iter_mut().enumerate() {
                *s = if channels == 1 { 0.5 * (l + r) } else if ch % 2 == 0 { l } else { r };
            }
            if let Some(b) = published.as_mut() {
                if i < b.len() {
                    b[i] = 0.5 * (l + r);
                }
            }
        }
        self.engine.publish_meters(&self.shared);
    }
}

// ------------------------------------------------------------ UI side

const INK: Rgb565 = Rgb565::new(30, 60, 31);
const DIM: Rgb565 = Rgb565::new(20, 38, 26);
const SHADE: Rgb565 = Rgb565::new(1, 1, 3);
const CHIP_BG: Rgb565 = Rgb565::new(5, 10, 9);
const CHIP_LINE: Rgb565 = Rgb565::new(11, 22, 16);

/// The tunnel is computed on a 160x90 grid and blown up 4x with bilinear
/// filtering: smooth like the page's shader, at a fraction of the cost of
/// shading every pixel.
const LW: usize = 160;
const LH: usize = 90;
const CELL: usize = 4;

fn fract(x: f32) -> f32 {
    x - x.floor()
}

fn hash(x: f32, y: f32) -> f32 {
    let (px, py) = (fract(x * 123.34), fract(y * 456.21));
    let d = px * (px + 45.32) + py * (py + 45.32);
    fract((px + d) * (py + d))
}

fn vnoise(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (fx, fy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash(ix, iy) + (hash(ix + 1.0, iy) - hash(ix, iy)) * fx;
    let b = hash(ix, iy + 1.0) + (hash(ix + 1.0, iy + 1.0) - hash(ix, iy + 1.0)) * fx;
    a + (b - a) * fy
}

fn fbm(mut x: f32, mut y: f32) -> f32 {
    let (mut v, mut a) = (0.0, 0.5);
    for _ in 0..4 {
        v += a * vnoise(x, y);
        let (nx, ny) = (x * 2.03 + 1.7, y * 2.03 + 9.2);
        x = nx;
        y = ny;
        a *= 0.5;
    }
    v
}

/// The page's iridescent cosine palette.
fn palette(t: f32, hue: f32, sh: f32) -> [f32; 3] {
    [0.0, 0.33 + sh, 0.67 + sh * 2.0].map(|o| 0.5 + 0.5 * (TAU * (t + hue + o)).cos())
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hsv(h: f32, s: f32, v: f32) -> Rgb565 {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i as i32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Rgb565::new((r * 31.0) as u8, (g * 63.0) as u8, (b * 31.0) as u8)
}

/// What the tunnel reacts to: bass, mids and highs from the master out,
/// kick and snare punches, Weirdness, and how far the tape has melted.
struct Look {
    t: f32,
    b: f32,
    mi: f32,
    hi: f32,
    kick: f32,
    snare: f32,
    weird: f32,
    melt: f32,
    segs: f32,
    hue: f32,
}

/// One pixel of the kaleidoscope tunnel: a port of the page's fragment
/// shader (mirror-folded angle, fbm-warped bands, a bass-lit core).
fn tunnel(px: usize, py: usize, l: &Look) -> [u8; 3] {
    let min = LH as f32;
    let mut ux = (px as f32 + 0.5 - LW as f32 * 0.5) / min;
    let mut uy = (LH as f32 * 0.5 - py as f32 - 0.5) / min;
    let drip = l.melt * (0.3 + 0.25 * (ux * 9.0 + l.t * 0.7).sin()) * (0.6 + 0.4 * (ux * 23.0 - l.t * 1.3).sin());
    uy += drip * smoothstep(1.0, -0.8, uy);
    let zoom = 1.0 - l.kick * 0.07;
    ux *= zoom;
    uy *= zoom;
    let r = (ux * ux + uy * uy).sqrt();
    let seg = TAU / l.segs;
    let a = (uy.atan2(ux) + l.t * 0.04 * (1.0 + l.weird * 2.0)).rem_euclid(seg);
    let a = (a - seg * 0.5).abs();
    let (pxx, pyy) = (a.cos() * r, a.sin() * r);
    let depth = 0.4 / (r + 0.12);
    let qx = pxx * 1.6 + pyy * 0.4;
    let qy = depth - l.t * (0.25 + l.b * 0.6);
    let wx = fbm(qx * 1.3 + l.t * 0.07, qy * 1.3 + l.t * 0.07);
    let wy = fbm(qx * 1.3 - l.t * 0.05 + 4.1, qy * 1.3 - l.t * 0.05 + 4.1);
    let warp = 1.2 + l.weird * 3.5;
    let n = fbm(qx * 1.8 + wx * warp + l.mi, qy * 1.8 + wy * warp + l.mi);
    let bands = (n * (10.0 + l.weird * 14.0) + r * 6.0 - l.t * 1.5).sin();
    let sh = l.weird * 0.12;
    let c1 = palette(n * 1.3 + r * 0.35 - l.t * 0.02, l.hue, sh);
    let c2 = palette(n * 0.7 + 0.5, l.hue, -sh);
    let mixk = smoothstep(-0.3, 0.9, bands) * 0.55;
    let shade = 0.45 + 0.65 * smoothstep(-0.4, 1.0, bands);
    let glint = (1.0 - bands.abs()).max(0.0).powi(10) * (0.25 + l.hi * 1.8);
    let core = (-r * 5.0).exp() * (0.25 + l.b * 0.8);
    let core_c = palette(l.t * 0.07, l.hue, 0.0);
    let fade = smoothstep(1.55, 0.15, r) * 0.85 + 0.15;
    let ch = |i: usize| {
        let c = (c1[i] + (c2[i] - c1[i]) * mixk) * shade + glint * [1.0, 0.9, 1.0][i] + core * core_c[i] + l.kick * 0.12;
        ((c * fade).clamp(0.0, 1.0) * 255.0) as u8
    };
    [ch(0), ch(1), ch(2)]
}

/// Bilinear 4x upscale of the tunnel into the full frame.
fn upscale(low: &[[u8; 3]], out: &mut [Rgb565]) {
    for y in 0..HEIGHT {
        let fy = ((y as f32 + 0.5) / CELL as f32 - 0.5).max(0.0);
        let y0 = (fy as usize).min(LH - 1);
        let y1 = (y0 + 1).min(LH - 1);
        let ty = fy - y0 as f32;
        for x in 0..WIDTH {
            let fx = ((x as f32 + 0.5) / CELL as f32 - 0.5).max(0.0);
            let x0 = (fx as usize).min(LW - 1);
            let x1 = (x0 + 1).min(LW - 1);
            let tx = fx - x0 as f32;
            let mut c = [0u8; 3];
            for k in 0..3 {
                let a = low[y0 * LW + x0][k] as f32 * (1.0 - tx) + low[y0 * LW + x1][k] as f32 * tx;
                let b = low[y1 * LW + x0][k] as f32 * (1.0 - tx) + low[y1 * LW + x1][k] as f32 * tx;
                c[k] = (a + (b - a) * ty) as u8;
            }
            out[y * WIDTH + x] = Rgb565::new(c[0] >> 3, c[1] >> 2, c[2] >> 3);
        }
    }
}

// ---- the control panel: one layout shared by drawing and by the mouse

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl Rect {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x as f32 && x < (self.x + self.w) as f32 && y >= self.y as f32 && y < (self.y + self.h) as f32
    }
    fn rect(&self) -> Rectangle {
        Rectangle::new(Point::new(self.x, self.y), Size::new(self.w as u32, self.h as u32))
    }
}

const DOCK: Rect = Rect { x: 12, y: 226, w: 616, h: 130 };
const CHIP_ROWS: [&[usize]; 3] = [&[0, 1, 2, 3], &[4, 5, 6], &[7, 8, 9]];

struct Layout {
    chips: [Rect; 10],
    play: Rect,
    new_track: Rect,
    hold: Rect,
    wander: Rect,
    weird: Rect,
    space: Rect,
    hide: Rect,
}

fn button_w(label: &str) -> i32 {
    label.len() as i32 * 6 + 8
}

fn layout(hidden: bool) -> Layout {
    let mut chips = [Rect { x: 0, y: 0, w: 0, h: 0 }; 10];
    for (row, ids) in CHIP_ROWS.iter().enumerate() {
        let mut x = 184;
        for &i in ids.iter() {
            let w = GENRES[i].name.len() as i32 * 6 + 12;
            chips[i] = Rect { x, y: DOCK.y + 24 + row as i32 * 22, w, h: 18 };
            x += w + 6;
        }
    }
    let (px, py) = (428, DOCK.y + 10);
    let play = Rect { x: px, y: py, w: button_w("Play").max(button_w("Stop")), h: 20 };
    let new_track = Rect { x: play.x + play.w + 4, y: py, w: button_w("New track"), h: 20 };
    let hold = Rect { x: new_track.x + new_track.w + 4, y: py, w: button_w("Hold to melt"), h: 20 };
    let hide_label = if hidden { "Show controls" } else { "Hide controls" };
    Layout {
        chips,
        play,
        new_track,
        hold,
        wander: Rect { x: 184, y: DOCK.y + 92, w: 190, h: 14 },
        weird: Rect { x: 428, y: DOCK.y + 50, w: 188, h: 16 },
        space: Rect { x: 428, y: DOCK.y + 84, w: 188, h: 16 },
        hide: Rect { x: WIDTH as i32 - 10 - button_w(hide_label) - 4, y: 8, w: button_w(hide_label) + 4, h: 20 },
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Weird,
    Space,
    Hold,
}

/// Draws a mono-font text at an integer scale, so the wordmark can be
/// bigger than the biggest bitmap font.
struct Scaled<'a> {
    fb: &'a mut FrameBuffer,
    ox: i32,
    oy: i32,
    k: i32,
}

impl OriginDimensions for Scaled<'_> {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl DrawTarget for Scaled<'_> {
    type Color = Rgb565;
    type Error = core::convert::Infallible;
    fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(&mut self, pixels: I) -> Result<(), Self::Error> {
        for Pixel(p, c) in pixels {
            let r = Rectangle::new(Point::new(self.ox + p.x * self.k, self.oy + p.y * self.k), Size::new(self.k as u32, self.k as u32));
            self.fb.fill_solid(&r, c).ok();
        }
        Ok(())
    }
}

pub struct MeltFmApp {
    shared: Arc<Shared>,
    level: Arc<AtomicF32>,
    ext: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    title: String,
    ticker: Vec<String>,
    seen_events: u32,
    prev_pads: [bool; 16],
    seen_kicks: u32,
    seen_snares: u32,
    look: Look,
    last_frame: Instant,
    low: Vec<[u8; 3]>,
    full: Vec<Rgb565>,
    hidden: bool,
    pressed: bool,
    drag: Drag,
    mouse_hold: bool,
}

impl MeltFmApp {
    pub fn new(bus: Arc<AudioBus>, mixer: Arc<MixerBus>, mods: Arc<ModBus>) -> Self {
        let (level, ext) = mixer.register(NAME, &mods);
        let output = bus.register(NAME);
        output.lock().unwrap().reserve(8192);
        Self {
            shared: Arc::new(Shared::new()),
            level,
            ext,
            output,
            title: make_title(seed_from_clock()),
            ticker: vec!["Press Play, then raise Weirdness a little at a time.".into()],
            seen_events: 0,
            prev_pads: [false; 16],
            seen_kicks: 0,
            seen_snares: 0,
            look: Look { t: 0.0, b: 0.2, mi: 0.2, hi: 0.1, kick: 0.0, snare: 0.0, weird: 0.35, melt: 0.0, segs: 6.0, hue: GENRES[0].hue },
            last_frame: Instant::now(),
            low: vec![[0; 3]; LW * LH],
            full: vec![Rgb565::new(0, 0, 0); WIDTH * HEIGHT],
            hidden: false,
            pressed: false,
            drag: Drag::None,
            mouse_hold: false,
        }
    }

    fn genre(&self) -> usize {
        (self.shared.genre_sel.load(Ordering::Relaxed) as usize).min(GENRES.len() - 1)
    }

    fn playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    fn new_track(&mut self) {
        let seed = seed_from_clock().wrapping_add(self.last_frame.elapsed().subsec_nanos()).max(1);
        self.title = make_title(seed);
        self.shared.track_seed.store(seed, Ordering::Release);
    }

    fn toggle_wander(&self) {
        let w = self.shared.wander.load(Ordering::Relaxed);
        self.shared.wander.store(!w, Ordering::Relaxed);
    }

    /// The last few things the engine did, newest first, as in the page's ticker.
    fn drain_events(&mut self) {
        let head = self.shared.event_head.load(Ordering::Acquire);
        let behind = head.wrapping_sub(self.seen_events);
        if behind == 0 {
            return;
        }
        let mut i = if behind > 16 { head.wrapping_sub(16) } else { self.seen_events };
        while i != head {
            let e = self.shared.events[(i % 16) as usize].load(Ordering::Relaxed);
            let text = event_text(e >> 24, (e >> 16) & 0xff);
            if !text.is_empty() {
                let bar = e & 0xffff;
                self.ticker.insert(0, if bar > 0 { format!("bar {bar:<2} {text}") } else { text });
            }
            i = i.wrapping_add(1);
        }
        self.ticker.truncate(3);
        self.seen_events = head;
    }

    fn set_weird(&self, v: f32) {
        self.shared.weird.set(v.clamp(0.0, 1.0));
    }

    fn set_space(&self, v: f32) {
        self.shared.space.set(v.clamp(0.0, 1.0));
    }

    fn cycle_genre(&self, delta: i32) {
        let n = GENRES.len() as i32;
        self.shared.genre_sel.store((self.genre() as i32 + delta).rem_euclid(n) as u32, Ordering::Relaxed);
    }

    fn drag_to(&self, x: f32) {
        let lay = layout(self.hidden);
        match self.drag {
            Drag::Weird => self.set_weird((x - lay.weird.x as f32) / lay.weird.w as f32),
            Drag::Space => self.set_space((x - lay.space.x as f32) / lay.space.w as f32),
            _ => {}
        }
    }

    fn update_look(&mut self) {
        let sh = &self.shared;
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        let kicks = sh.kicks.load(Ordering::Relaxed);
        let snares = sh.snares.load(Ordering::Relaxed);
        let target_hue = GENRES[(sh.genre_sel.load(Ordering::Relaxed) as usize).min(GENRES.len() - 1)].hue;
        let l = &mut self.look;
        if kicks != self.seen_kicks {
            self.seen_kicks = kicks;
            l.kick = 1.0;
        }
        if snares != self.seen_snares {
            self.seen_snares = snares;
            l.snare = 1.0;
        }
        l.kick *= 0.02f32.powf(dt * 2.2);
        l.snare *= 0.01f32.powf(dt * 3.0);
        let playing = sh.playing.load(Ordering::Relaxed);
        // at rest it breathes slowly, as the page does with no audio
        let (b, mi, hi) = if playing { (sh.lo.get(), sh.mid.get(), sh.hi.get()) } else { (0.18, 0.2, 0.08) };
        l.b += (b - l.b) * 0.35;
        l.mi += (mi - l.mi) * 0.2;
        l.hi += (hi - l.hi) * 0.3;
        l.weird = sh.weird.get();
        let melt = if playing { sh.melt.get() } else { 0.0 };
        l.melt += (melt - l.melt) * 0.1;
        l.t += dt * (0.5 + l.b * 1.6 + l.weird * 0.6) * (1.0 - 0.6 * l.melt);
        l.hue += (target_hue - l.hue) * 0.02;
        // the mirror count changes with each new section, as in the page
        let sec = sh.section.load(Ordering::Relaxed);
        let span = 3 + (l.weird * 9.0) as u32;
        l.segs = (3 + (sec * 7 + 3) % span) as f32;
    }

    fn text(fb: &mut FrameBuffer, s: &str, x: i32, y: i32, font: &embedded_graphics::mono_font::MonoFont<'static>, col: Rgb565) {
        Text::new(s, Point::new(x, y), MonoTextStyle::new(font, col)).draw(fb).ok();
    }

    fn shadowed(fb: &mut FrameBuffer, s: &str, x: i32, y: i32, font: &embedded_graphics::mono_font::MonoFont<'static>, col: Rgb565) {
        Self::text(fb, s, x + 1, y + 1, font, SHADE);
        Self::text(fb, s, x, y, font, col);
    }

    fn pill(fb: &mut FrameBuffer, r: Rect, fill: Rgb565, line: Rgb565) {
        let shape = RoundedRectangle::with_equal_corners(r.rect(), Size::new((r.h / 2) as u32, (r.h / 2) as u32));
        shape.into_styled(PrimitiveStyleBuilder::new().fill_color(fill).stroke_color(line).stroke_width(1).build()).draw(fb).ok();
    }

    fn button(fb: &mut FrameBuffer, r: Rect, label: &str, on: bool, accent: Rgb565) {
        Self::pill(fb, r, if on { accent } else { CHIP_BG }, if on { accent } else { CHIP_LINE });
        Self::text(fb, label, r.x + 4, r.y + 14, &SPLEEN_6X12, if on { SHADE } else { INK });
    }

    fn slider(fb: &mut FrameBuffer, r: Rect, label: &str, value: f32, word: &str, hue: Option<f32>, accent: Rgb565) {
        Self::text(fb, label, r.x, r.y - 4, &SPLEEN_6X12, DIM);
        Self::text(fb, word, r.x + r.w - word.len() as i32 * 6, r.y - 4, &SPLEEN_6X12, INK);
        let ty = r.y + 6;
        match hue {
            // Weirdness: the track itself goes from grey through the spectrum
            Some(h) => {
                for sx in (0..r.w).step_by(2) {
                    let u = sx as f32 / r.w as f32;
                    let col = if u < 0.08 { Rgb565::new(17, 34, 20) } else { hsv(h + (u - 0.08) * 0.9, 0.55, 1.0) };
                    Rectangle::new(Point::new(r.x + sx, ty), Size::new(2, 3)).into_styled(PrimitiveStyle::with_fill(col)).draw(fb).ok();
                }
            }
            None => {
                Rectangle::new(Point::new(r.x, ty), Size::new(r.w as u32, 3)).into_styled(PrimitiveStyle::with_fill(CHIP_LINE)).draw(fb).ok();
            }
        }
        let kx = r.x + (value.clamp(0.0, 1.0) * r.w as f32) as i32;
        Circle::with_center(Point::new(kx, ty + 1), 11).into_styled(PrimitiveStyle::with_fill(INK)).draw(fb).ok();
        let _ = accent;
    }
}

impl App for MeltFmApp {
    fn wants_fullscreen(&self) -> bool {
        true
    }

    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kit::screen_extra(&fb)
    }

    fn hint(&self) -> String {
        "Pads 1-10 genre | 13 new | 14 melt | 15 wander | 16 play | K1 weird | K2 space".into()
    }

    fn running(&self) -> Option<bool> {
        Some(self.playing())
    }

    fn toggle_running(&mut self) {
        let p = !self.playing();
        self.shared.playing.store(p, Ordering::Relaxed);
    }

    fn on_exit(&mut self) {
        self.pressed = false;
        self.drag = Drag::None;
        self.mouse_hold = false;
    }

    /// The mouse (or a touch) on the panel. A negative position is the
    /// release; a position while a press is held is a drag, which only
    /// the sliders follow.
    fn slint_pointer_pick(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
            self.pressed = false;
            self.drag = Drag::None;
            self.mouse_hold = false;
            return;
        }
        if self.pressed {
            self.drag_to(x);
            return;
        }
        self.pressed = true;
        let lay = layout(self.hidden);
        if lay.hide.contains(x, y) {
            self.hidden = !self.hidden;
            return;
        }
        if self.hidden {
            return;
        }
        if lay.play.contains(x, y) {
            self.toggle_running();
        } else if lay.new_track.contains(x, y) {
            self.new_track();
        } else if lay.hold.contains(x, y) {
            self.drag = Drag::Hold;
            self.mouse_hold = true;
        } else if lay.wander.contains(x, y) {
            self.toggle_wander();
        } else if lay.weird.contains(x, y) {
            self.drag = Drag::Weird;
            self.drag_to(x);
        } else if lay.space.contains(x, y) {
            self.drag = Drag::Space;
            self.drag_to(x);
        } else if let Some(i) = lay.chips.iter().position(|c| c.contains(x, y)) {
            self.shared.genre_sel.store(i as u32, Ordering::Relaxed);
        }
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        let sh = &self.shared;
        vec![
            ("Genre".into(), GENRES[self.genre()].name.into(), false),
            ("Weirdness".into(), format!("{} ({:.0}%)", weird_word(sh.weird.get()), sh.weird.get() * 100.0), false),
            ("Space".into(), format!("{:.0}%", sh.space.get() * 100.0), false),
            ("Wander".into(), if sh.wander.load(Ordering::Relaxed) { "every 32 bars" } else { "off" }.into(), false),
            ("Now playing".into(), self.title.clone(), false),
        ]
    }

    fn instrument_settings(&self) -> Vec<Setting> {
        self.slint_rows().into_iter().take(4).map(|(label, value, _)| Setting { label, value }).collect()
    }

    fn adjust_setting(&mut self, index: usize, delta: i32) {
        match index {
            0 => self.cycle_genre(delta.signum()),
            1 => self.set_weird(self.shared.weird.get() + delta as f32 * 0.02),
            2 => self.set_space(self.shared.space.get() + delta as f32 * 0.02),
            3 => self.toggle_wander(),
            _ => {}
        }
    }

    fn tick(&mut self, input: &Input) {
        let prev = self.prev_pads;
        let edge = |i: usize| input.grid[i] && !prev[i];
        for g in 0..GENRES.len() {
            if edge(g) {
                self.shared.genre_sel.store(g as u32, Ordering::Relaxed);
            }
        }
        if edge(12) || input.knob1_press {
            self.new_track();
        }
        self.shared.hold.store(input.grid[13] || self.mouse_hold, Ordering::Relaxed);
        if edge(14) {
            self.toggle_wander();
        }
        if edge(15) || input.knob2_press {
            self.toggle_running();
        }
        self.prev_pads = input.grid;
        if input.knob1 != 0 {
            self.set_weird(self.shared.weird.get() + input.knob1 as f32 * 0.02);
        }
        if input.knob2 != 0 {
            self.set_space(self.shared.space.get() + input.knob2 as f32 * 0.02);
        }
        self.drain_events();
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(MeltProcessor {
            shared: Arc::clone(&self.shared),
            engine: Box::new(Engine::new()),
            level: Arc::clone(&self.level),
            ext: Arc::clone(&self.ext),
            output: Arc::clone(&self.output),
            gain: 1.0,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        self.update_look();
        for y in 0..LH {
            for x in 0..LW {
                self.low[y * LW + x] = tunnel(x, y, &self.look);
            }
        }
        upscale(&self.low, &mut self.full);

        let lay = layout(self.hidden);
        let sh = &self.shared;
        let g = &GENRES[self.genre()];
        let accent = hsv(self.look.hue + 0.05, 0.55, 1.0);

        // the glass: darken the tunnel under the dock, with rounded corners
        if !self.hidden {
            for y in DOCK.y..DOCK.y + DOCK.h {
                for x in DOCK.x..DOCK.x + DOCK.w {
                    let (dx, dy) = ((x - DOCK.x).min(DOCK.x + DOCK.w - 1 - x), (y - DOCK.y).min(DOCK.y + DOCK.h - 1 - y));
                    if dx < 10 && dy < 10 && (10 - dx) * (10 - dx) + (10 - dy) * (10 - dy) > 100 {
                        continue;
                    }
                    let c = self.full[y as usize * WIDTH + x as usize];
                    self.full[y as usize * WIDTH + x as usize] = Rgb565::new((c.r() as u16 * 7 / 32) as u8 + 1, (c.g() as u16 * 7 / 32) as u8 + 1, (c.b() as u16 * 7 / 32) as u8 + 2);
                }
            }
        }
        let area = Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32));
        fb.fill_contiguous(&area, self.full.iter().copied()).ok();

        // heavy wordmark: the biggest font at 2x, doubled up to fatten it
        {
            let st = MonoTextStyle::new(&SPLEEN_16X32, SHADE);
            let mut s = Scaled { fb, ox: 20, oy: 14, k: 2 };
            Text::new("Melt FM", Point::new(1, 26), st).draw(&mut s).ok();
            let st = MonoTextStyle::new(&SPLEEN_16X32, INK);
            for dx in 0..2 {
                Text::new("Melt FM", Point::new(dx, 25), st).draw(&mut s).ok();
            }
        }
        if !self.hidden {
            Self::shadowed(fb, "Pick a genre and it composes forever.", 18, 92, &SPLEEN_8X16, INK);
            Self::shadowed(fb, "The weirdness dial decides how long", 18, 110, &SPLEEN_8X16, INK);
            Self::shadowed(fb, "it stays normal.", 18, 128, &SPLEEN_8X16, INK);
        }
        Self::button(fb, lay.hide, if self.hidden { "Show controls" } else { "Hide controls" }, false, accent);
        if self.hidden {
            return;
        }

        // left: what is playing
        let x0 = 26;
        Self::text(fb, "NOW PLAYING", x0, DOCK.y + 16, &SPLEEN_6X12, DIM);
        let mut line = String::new();
        let mut y = DOCK.y + 32;
        let title_line = |fb: &mut FrameBuffer, l: &str, y: i32| {
            // the 6x12 font drawn twice, a pixel apart, reads as a bold title
            Self::text(fb, l, x0, y, &SPLEEN_6X12, INK);
            Self::text(fb, l, x0 + 1, y, &SPLEEN_6X12, INK);
        };
        for word in self.title.split(' ') {
            if line.len() + word.len() + 1 > 24 && !line.is_empty() {
                title_line(fb, &line, y);
                y += 13;
                line.clear();
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        title_line(fb, &line, y);
        let status = if self.playing() {
            let sec = SECTIONS[(sh.section.load(Ordering::Relaxed) as usize).min(SECTIONS.len() - 1)].name;
            format!("{} {:.0}bpm {} {}/32", g.name, sh.bpm.get(), sec, sh.bar.load(Ordering::Relaxed) + 1)
        } else {
            format!("{} {:.0}bpm stopped", g.name, g.bpm)
        };
        y += 16;
        Self::text(fb, &status, x0, y, &SPLEEN_6X12, accent);
        for (i, t) in self.ticker.iter().take(2).enumerate() {
            let shown: String = t.chars().take(26).collect();
            Self::text(fb, &shown, x0, y + 14 + i as i32 * 11, &SPLEEN_6X12, if i == 0 { INK } else { DIM });
        }

        // middle: genre chips and wander
        Self::text(fb, "GENRE", 184, DOCK.y + 16, &SPLEEN_6X12, DIM);
        for (i, c) in lay.chips.iter().enumerate() {
            let on = i == self.genre();
            Self::pill(fb, *c, if on { accent } else { CHIP_BG }, if on { accent } else { CHIP_LINE });
            Self::text(fb, GENRES[i].name, c.x + 6, c.y + 13, &SPLEEN_6X12, if on { SHADE } else { INK });
        }
        let w = lay.wander;
        let on = sh.wander.load(Ordering::Relaxed);
        Rectangle::new(Point::new(w.x, w.y), Size::new(11, 11)).into_styled(PrimitiveStyle::with_fill(if on { accent } else { CHIP_BG })).draw(fb).ok();
        Rectangle::new(Point::new(w.x, w.y), Size::new(11, 11)).into_styled(PrimitiveStyle::with_stroke(CHIP_LINE, 1)).draw(fb).ok();
        Self::text(fb, "Wander every 32 bars", w.x + 17, w.y + 10, &SPLEEN_6X12, INK);

        // right: transport and the two dials
        Self::button(fb, lay.play, if self.playing() { "Stop" } else { "Play" }, self.playing(), accent);
        Self::button(fb, lay.new_track, "New track", false, accent);
        Self::button(fb, lay.hold, "Hold to melt", sh.hold.load(Ordering::Relaxed), accent);
        Self::slider(fb, lay.weird, "WEIRDNESS", sh.weird.get(), weird_word(sh.weird.get()), Some(self.look.hue), accent);
        let sp = format!("{:.0}", sh.space.get() * 100.0);
        Self::slider(fb, lay.space, "SPACE", sh.space.get(), &sp, None, accent);

        Self::text(fb, "Pads 1-10 genre | 13 new | 14 melt | 15 wander | 16 play | knobs: weird, space", 26, DOCK.y + DOCK.h - 6, &SPLEEN_6X12, DIM);
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    Box::new(MeltFmApp::new(ctx.get(), ctx.get(), ctx.get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> MeltFmApp {
        MeltFmApp::new(Arc::new(AudioBus::new()), Arc::new(MixerBus::new()), Arc::new(ModBus::new()))
    }

    fn pad(i: usize) -> Input {
        Input { grid: std::array::from_fn(|k| k == i), ..Default::default() }
    }

    fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        let mut buf = vec![0.0f32; 512 * 2];
        for _ in 0..blocks {
            p.process(&mut buf, 2, 48_000.0);
            all.extend_from_slice(&buf);
        }
        all
    }

    fn peak(a: &[f32]) -> f32 {
        a.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn play_makes_real_sound_and_stop_goes_silent() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        assert!(peak(&render(&mut p, 20)) < 1e-6, "silent before Play");
        a.tick(&pad(15));
        let out = render(&mut p, 400);
        assert!(peak(&out) > 0.05, "playing must produce audible output, peak {}", peak(&out));
        assert!(out.iter().all(|s| s.is_finite()));
        a.tick(&Input::default());
        a.tick(&pad(15));
        render(&mut p, 40);
        let tail = render(&mut p, 20);
        assert!(peak(&tail) < 1e-3, "stopped must go quiet, peak {}", peak(&tail));
    }

    #[test]
    fn every_genre_stays_finite_and_bounded_at_full_weirdness() {
        for g in 0..GENRES.len() {
            let mut a = app();
            let mut p = a.audio_processor().unwrap();
            a.shared.genre_sel.store(g as u32, Ordering::Relaxed);
            a.shared.weird.set(1.0);
            a.shared.wander.store(true, Ordering::Relaxed);
            a.shared.hold.store(g % 2 == 0, Ordering::Relaxed);
            a.toggle_running();
            let out = render(&mut p, 1200);
            assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.01), "{} produced a bad sample", GENRES[g].name);
            assert!(peak(&out) > 0.02, "{} was silent", GENRES[g].name);
            let rms = (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt();
            eprintln!("{:>12}: peak {:.2} rms {:.3}", GENRES[g].name, peak(&out), rms);
        }
    }

    #[test]
    fn holding_melt_slows_the_tempo() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.shared.weird.set(0.0);
        a.toggle_running();
        render(&mut p, 100);
        let straight = a.shared.bpm.get();
        a.tick(&pad(13));
        render(&mut p, 200);
        let melted = a.shared.bpm.get();
        assert!(melted < straight * 0.6, "tempo {straight} -> {melted} should have dropped by well over a third");
        assert!(a.shared.melt.get() > 0.8);
    }

    /// The audio callback has a hard deadline; the busiest case (full
    /// Weirdness, a dense genre, voices piling up) must still render ten
    /// seconds of audio in well under ten seconds, here with a 4x margin.
    #[test]
    fn the_busiest_case_renders_at_least_four_times_faster_than_real_time() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.shared.genre_sel.store(8, Ordering::Relaxed);
        a.shared.weird.set(1.0);
        a.toggle_running();
        let t = Instant::now();
        render(&mut p, 940);
        let secs = t.elapsed().as_secs_f32();
        eprintln!("10 s of Trap at full Weirdness took {secs:.3} s");
        assert!(secs < 2.5, "took {secs} s for 10 s of audio");
    }

    /// The whole screen is the tunnel, so it must be lit and varied, and a
    /// frame must be cheap enough for the UI thread. Set MELT_FM_PPM to a path
    /// to keep the frame for a look.
    #[test]
    fn the_screen_is_a_lit_tunnel_and_draws_quickly() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.shared.weird.set(0.7);
        a.toggle_running();
        render(&mut p, 300);
        a.tick(&Input::default());
        let mut fb = FrameBuffer::new();
        a.draw(&mut fb);
        let t = Instant::now();
        for _ in 0..10 {
            a.draw(&mut fb);
        }
        let ms = t.elapsed().as_secs_f32() * 100.0;
        eprintln!("a frame took {ms:.1} ms");
        assert!(ms < 12.0, "a frame took {ms} ms");
        let px = fb.buffer();
        let mean = px.iter().map(|c| ((c >> 16) & 255) + ((c >> 8) & 255) + (c & 255)).sum::<u32>() as f32 / (px.len() * 3) as f32;
        assert!(mean > 20.0 && mean < 235.0, "mean brightness {mean}");
        let distinct: std::collections::HashSet<u32> = px.iter().copied().collect();
        assert!(distinct.len() > 500, "only {} colours", distinct.len());
        if let Ok(path) = std::env::var("MELT_FM_PPM") {
            let mut out = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
            for c in px {
                out.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, *c as u8]);
            }
            std::fs::write(path, out).unwrap();
        }
    }

    fn click(a: &mut MeltFmApp, r: Rect) {
        a.slint_pointer_pick(r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
        a.slint_pointer_pick(-1.0, -1.0);
    }

    #[test]
    fn the_on_screen_controls_work_with_the_mouse() {
        let mut a = app();
        let lay = layout(false);
        click(&mut a, lay.play);
        assert!(a.playing(), "Play starts it");
        click(&mut a, lay.play);
        assert!(!a.playing(), "and it becomes Stop");
        click(&mut a, lay.chips[7]);
        assert_eq!(GENRES[a.genre()].id, "jazz");
        let before = a.title.clone();
        click(&mut a, lay.new_track);
        assert_ne!(a.title, before);
        click(&mut a, lay.wander);
        assert!(a.shared.wander.load(Ordering::Relaxed));
        // a slider follows the drag and ends where the mouse does
        a.slint_pointer_pick(lay.weird.x as f32 + 10.0, lay.weird.y as f32 + 6.0);
        a.slint_pointer_pick(lay.weird.x as f32 + lay.weird.w as f32 * 0.75, 400.0);
        a.slint_pointer_pick(-1.0, -1.0);
        assert!((a.shared.weird.get() - 0.75).abs() < 0.01, "weirdness {}", a.shared.weird.get());
        // Hold to melt holds only while the button is down
        a.slint_pointer_pick(lay.hold.x as f32 + 5.0, lay.hold.y as f32 + 5.0);
        a.tick(&Input::default());
        assert!(a.shared.hold.load(Ordering::Relaxed));
        a.slint_pointer_pick(-1.0, -1.0);
        a.tick(&Input::default());
        assert!(!a.shared.hold.load(Ordering::Relaxed));
        // Hide controls leaves only the wordmark and the way back
        click(&mut a, lay.hide);
        assert!(a.hidden);
        click(&mut a, layout(true).hide);
        assert!(!a.hidden);
    }

    #[test]
    fn composing_is_deterministic_per_seed_and_titles_follow_the_seed() {
        let g = &GENRES[1];
        let (mut r1, mut r2) = (Rng::new(42), Rng::new(42));
        let (s1, s2) = (compose(g, &mut r1), compose(g, &mut r2));
        assert_eq!(s1.prog, s2.prog);
        assert_eq!(s1.poly_len, s2.poly_len);
        assert_eq!(make_title(7), make_title(7));
        assert_ne!(make_title(7), make_title(8));
    }

    #[test]
    fn pads_pick_a_genre_and_new_track_changes_the_title() {
        let mut a = app();
        a.tick(&pad(8));
        assert_eq!(GENRES[a.genre()].id, "trap");
        a.tick(&Input::default());
        let before = a.title.clone();
        a.tick(&pad(12));
        assert_ne!(a.title, before);
    }

    #[test]
    fn the_engine_reports_what_the_dial_did() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        a.shared.weird.set(1.0);
        a.toggle_running();
        render(&mut p, 3000);
        a.tick(&Input::default());
        assert!(a.ticker.len() > 1, "a fully weird song must have logged something");
    }
}
