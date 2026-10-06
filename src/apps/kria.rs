//! Kria: monome's four-track step sequencer (the grid app on Ansible),
//! made to sit beside Teletype.
//!
//! Four tracks, each driving a TR (gate) and a CV (pitch). Every track has
//! seven parameters, each with its own 16 steps, loop, clock division and
//! per-step probability, so the parameters drift against each other:
//!
//! - **Trigger**: whether a step fires. **Ratchet**: 1-5 sub-triggers.
//! - **Note**: scale degree 0-6. **Alt note**: a second degree added to it
//!   (the sum wraps every 7 degrees into the next octave).
//! - **Octave**: 0-5 octaves up. **Glide**: 0-6, x 20 ms of slew.
//! - **Duration**: 0-5, times the track's duration multiplier, as a fraction
//!   of the step: `(dur + 1) x (mul x 4) / 384` of the step's length, so at
//!   the default multiplier of 4 a gate is 1/24 to 1/4 of a step and at 16 a
//!   full step.
//! - **Loop** (start/length per parameter, wrapping past step 16), **Time**
//!   (divide the clock by 1-16 per parameter), **Probability** (100/50/25/0 %
//!   per step), **direction** per track (forward, reverse, triangle, drunk,
//!   random).
//! - **Scales**: 16 slots, each a root and six intervals. The first seven
//!   are the church modes (Ionian..Locrian) and the rest chromatic: that is
//!   what Ansible ships with as far as its firmware shows (the interval
//!   table itself lives in a library not checked here).
//! - **Patterns**: 16, switched at once or cued to land on the next cue
//!   boundary (every 4 clocks by default).
//! - **Note sync** (on by default): giving a step a note turns its trigger
//!   on. **Loop sync** (All by default): setting a loop sets it for every
//!   parameter of every track (or every parameter of the track, or just the
//!   one).
//!
//! How a step plays follows Ansible's sequencer: each parameter advances on
//! its own division and loop; a step's probability decides whether its new
//! value is taken; the duration is measured against the time since the
//! track's last clock; ratchets split the step evenly; glide is set before
//! the note. Written from the Kria documentation with the details checked
//! against the Ansible firmware's behaviour; no firmware code is copied
//! (it is GPL-2.0, Portamax is MIT). Not here: the meta-sequencer, Kria's
//! division-sync and division-cue options, the duration "tie" mode, the
//! fine per-degree scale adjust, Ansible's clock/config grid screens, and
//! the second view on a 256's lower half.
//!
//! **On a grid** (the Grid app's screen grid, or a real monome grid): Kria
//! plays with Ansible's layout on the top-left 16 x 8. Bottom row: keys 1-4
//! pick the track (with LOOP held they mute it), 6 trigger/ratchet, 7 note/
//! alt note, 8 octave/glide (press again for the second), 9 duration, 11
//! LOOP, 12 TIME and 13 PROB (held), 15 scale, 16 pattern (held: taps cue).
//! Rows 1-7 are the page: on trigger, a row per track; on note, alt note and
//! glide the value is the height; octave and duration are bars, with the
//! octave shift and the duration multiplier on the top row; ratchet toggles
//! sub-triggers per step (top/bottom rows add/remove one). Held LOOP: press
//! the first step and the last (one press sets the start, the start again
//! makes a one-step loop). TIME: the column is the division. PROB: rows 3-6
//! are 100/50/25/0 %. Scale: per track the direction and "notes on triggers
//! only" (left), the 16 scales (bottom-left) and the selected scale's root
//! and intervals (right). Pattern: top row picks (hold to copy), row 2 sets
//! the cue length.
//!
//! **On Portamax**: the 16 pads are the 16 steps (top-left is step 1).
//! F2 turns the pages; D-pad left/right picks the track, up/down changes the
//! selected step's value. On TRIG a pad toggles a trigger; on a value page
//! a pad selects the step (tap it again to toggle its trigger). LOOP: tap
//! the first step then the last. TIME: pad n divides by n. PROB: a pad
//! steps through 100/50/25/0 %. SCALE: pad n picks scale n, up/down moves
//! the root. PATTERN: tap to switch (or cue), hold to copy the current one
//! there. SELECT toggles the selected step's trigger.
//!
//! The clock is the device transport (a step per sixteenth, F3 starts and
//! stops it), Kria's internal clock (a period in ms), the `Kria: Clock` mod
//! input, or Teletype (`KR.CLK`). Each TR plays its track's pitch to the
//! built-in voice or an app the Plays row names; TR and CV can also be
//! routed to any mod input.
//!
//! **Teletype**: the `KR.*` ops (KR.PAT, KR.POS, KR.L.ST, KR.L.LEN, KR.RES,
//! KR.CV, KR.MUTE, KR.TMUTE, KR.CLK, KR.SCALE, KR.PERIOD, KR.PRE, KR.PG,
//! KR.CUE, KR.DIR, KR.DUR) reach this app through `Link`, the way Teletype
//! reaches Ansible over I2C.

use super::grid_kit::{self, Grid, Leds};
use super::kids_kit;
use crate::{
    app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input, SlintExtra},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    clock::{Clock, StepFollower},
    display::FrameBuffer,
    led_output::PadColor,
    mixer_bus::MixerBus,
    modbus::{ModBus, Patch},
    note_bus::{NoteBus, NoteOut, NoteRoute},
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16},
    util::AtomicF32,
};
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}, text::Text};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicI16, AtomicU64, AtomicU8, AtomicUsize, Ordering},
    Arc, Mutex, OnceLock,
};

const APP_NAME: &str = "Kria";
const SAVE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/saves/kria");

pub const TRACKS: usize = 4;
const STEPS: usize = 16;
const PARAMS: usize = 7;
const PATTERNS: usize = 16;
const SCALES: usize = 16;
const PRESETS: usize = 8;

// Parameters, in Ansible's order (KR.POS numbers 1-4 are the first four).
const TR: usize = 0;
const NOTE: usize = 1;
const OCT: usize = 2;
const DUR: usize = 3;
const RPT: usize = 4;
const ALT: usize = 5;
const GLIDE: usize = 6;
const PARAM_NAMES: [&str; PARAMS] = ["TRIG", "NOTE", "OCTAVE", "DUR", "RATCHET", "ALT NOTE", "GLIDE"];
/// Highest value of each parameter (trigger is on/off; ratchet counts 1-5).
const MAX: [u8; PARAMS] = [1, 6, 5, 5, 5, 6, 6];

const DIRS: [&str; 5] = ["Forward", "Reverse", "Triangle", "Drunk", "Random"];
const LOOP_SYNC: [&str; 3] = ["None", "Track", "All"];
const CLOCKS: [&str; 4] = ["Device", "Internal", "Clock input", "Teletype"];
const C_DEVICE: u8 = 0;
const C_INTERNAL: u8 = 1;
const C_INPUT: u8 = 2;
const C_TELETYPE: u8 = 3;

/// Interval tables for the first seven scales: the church modes.
const MODES: [[u8; 6]; 7] = [
    [2, 2, 1, 2, 2, 2],
    [2, 1, 2, 2, 2, 1],
    [1, 2, 2, 2, 1, 2],
    [2, 2, 2, 1, 2, 2],
    [2, 2, 1, 2, 2, 1],
    [2, 1, 2, 2, 1, 2],
    [1, 2, 2, 1, 2, 2],
];

// Own palette: grid-key amber on black, like a varibright grid.
const BG: Rgb565 = Rgb565::new(1, 1, 1);
const KEY: Rgb565 = Rgb565::new(7, 12, 5);
const LIT: Rgb565 = Rgb565::new(31, 50, 12);
const MID: Rgb565 = Rgb565::new(18, 30, 7);
const HEAD: Rgb565 = Rgb565::new(31, 62, 28);
const ACCENT: Rgb565 = Rgb565::new(8, 44, 31);
const DIM: Rgb565 = Rgb565::new(10, 18, 6);
const FAINT: Rgb565 = Rgb565::new(4, 7, 3);

// ---------------------------------------------------------------- data

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Track {
    /// Step values per parameter (TR: 0/1, RPT: 1-5).
    v: [[u8; STEPS]; PARAMS],
    /// Which ratchet sub-triggers fire, per step (bit 0 = the first).
    rpt_bits: [u8; STEPS],
    /// 0 = never, 1 = 25 %, 2 = 50 %, 3 = always.
    prob: [[u8; STEPS]; PARAMS],
    lstart: [u8; PARAMS],
    llen: [u8; PARAMS],
    tmul: [u8; PARAMS],
    octshift: u8,
    dur_mul: u8,
    dir: u8,
    /// The note parameters advance only on triggers.
    trigger_clocked: bool,
}

impl Default for Track {
    fn default() -> Self {
        let mut v = [[0u8; STEPS]; PARAMS];
        v[RPT] = [1; STEPS];
        Track { v, rpt_bits: [1; STEPS], prob: [[3; STEPS]; PARAMS], lstart: [0; PARAMS], llen: [6; PARAMS], tmul: [1; PARAMS], octshift: 0, dur_mul: 4, dir: 0, trigger_clocked: false }
    }
}

impl Track {
    fn lend(&self, p: usize) -> u8 {
        ((self.lstart[p] as usize + self.llen[p].max(1) as usize - 1) % STEPS) as u8
    }
    fn in_loop(&self, p: usize, s: usize) -> bool {
        (s + STEPS - self.lstart[p] as usize) % STEPS < self.llen[p].max(1) as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Pattern {
    t: [Track; TRACKS],
    scale: u8,
}

impl Default for Pattern {
    fn default() -> Self {
        Pattern { t: [Track::default(); TRACKS], scale: 0 }
    }
}

/// Everything a preset holds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
struct Data {
    p: [Pattern; PATTERNS],
    /// Root (0-7 semitones), then six intervals.
    scales: [[u8; 7]; SCALES],
    note_sync: bool,
    loop_sync: u8,
    /// Clocks between cue points, minus one.
    cue_steps: u8,
}

impl Default for Data {
    fn default() -> Self {
        let mut scales = [[0, 1, 1, 1, 1, 1, 1]; SCALES];
        for (i, m) in MODES.iter().enumerate() {
            scales[i][1..].copy_from_slice(m);
        }
        Data { p: [Pattern::default(); PATTERNS], scales, note_sync: true, loop_sync: 2, cue_steps: 3 }
    }
}

/// Semitones above the root of each of the seven degrees.
fn degrees(scale: &[u8; 7]) -> [i32; 7] {
    let mut d = [scale[0] as i32; 7];
    for i in 1..7 {
        d[i] = d[i - 1] + scale[i] as i32;
    }
    d
}

/// A note and alt note on a scale and octave -> semitones.
fn pitch(scale: &[u8; 7], note: u8, alt: u8, oct: u8) -> i32 {
    let n = note as usize + alt as usize;
    degrees(scale)[n % 7] + (oct as i32 + (n / 7) as i32) * 12
}

// ---------------------------------------------------------------- the Teletype link

/// The `KR.*` ops Teletype has for Kria (as it has for Ansible).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KrOp {
    Pre,
    Period,
    Pat,
    Scale,
    Pos,
    LSt,
    LLen,
    Res,
    Cv,
    Mute,
    TMute,
    Clk,
    Pg,
    Cue,
    Dir,
    Dur,
}

/// A fixed ring of encoded commands, one writer and one reader, so neither
/// side's audio thread ever blocks or allocates.
struct Ring {
    slots: [AtomicU64; 64],
    head: AtomicUsize,
    tail: AtomicUsize,
}

impl Ring {
    fn new() -> Ring {
        Ring { slots: std::array::from_fn(|_| AtomicU64::new(0)), head: AtomicUsize::new(0), tail: AtomicUsize::new(0) }
    }
    fn push(&self, v: u64) {
        let h = self.head.load(Ordering::Relaxed);
        let t = self.tail.load(Ordering::Acquire);
        if h.wrapping_sub(t) < self.slots.len() {
            self.slots[h % self.slots.len()].store(v, Ordering::Relaxed);
            self.head.store(h.wrapping_add(1), Ordering::Release);
        }
    }
    fn pop(&self) -> Option<u64> {
        let t = self.tail.load(Ordering::Relaxed);
        if t == self.head.load(Ordering::Acquire) {
            return None;
        }
        let v = self.slots[t % self.slots.len()].load(Ordering::Relaxed);
        self.tail.store(t.wrapping_add(1), Ordering::Release);
        Some(v)
    }
}

/// A command for Kria: op, two small arguments and a value.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cmd {
    op: u8,
    a: u8,
    b: u8,
    v: i16,
}

impl Cmd {
    fn pack(self) -> u64 {
        self.op as u64 | (self.a as u64) << 8 | (self.b as u64) << 16 | ((self.v as u16) as u64) << 32
    }
    fn unpack(x: u64) -> Cmd {
        Cmd { op: x as u8, a: (x >> 8) as u8, b: (x >> 16) as u8, v: (x >> 32) as u16 as i16 }
    }
}

const OP_PAT: u8 = 1;
const OP_CUE: u8 = 2;
const OP_POS: u8 = 3;
const OP_RES: u8 = 4;
const OP_CLK: u8 = 5;
const OP_SCALE: u8 = 6;
const OP_LST: u8 = 7;
const OP_LLEN: u8 = 8;
const OP_DIR: u8 = 9;
const OP_PRE: u8 = 10;
const OP_PG: u8 = 11;

/// What Kria shares with Teletype: commands in, its state out. Every field
/// is an atomic, so Teletype's ops (which run on its audio thread) never
/// wait on Kria.
pub struct Link {
    /// True while a Kria app exists to answer.
    alive: AtomicBool,
    /// Commands for Kria's audio thread (clocks, positions, patterns).
    rt: Ring,
    /// Commands for Kria's editor (loops, scale, direction, preset, page).
    ui: Ring,
    period: AtomicI16,
    pattern: AtomicU8,
    /// Cued pattern, or -1.
    cue: AtomicI16,
    scale: AtomicU8,
    preset: AtomicU8,
    page: AtomicU8,
    dir: AtomicU8,
    pos: [[AtomicU8; PARAMS]; TRACKS],
    lstart: [[AtomicU8; PARAMS]; TRACKS],
    llen: [[AtomicU8; PARAMS]; TRACKS],
    /// The CV each track is at (Teletype's 0..16383 = 0..10 V) and its last
    /// gate length in ms.
    cv: [AtomicI16; TRACKS],
    dur: [AtomicI16; TRACKS],
    mute: [AtomicBool; TRACKS],
}

impl Link {
    pub fn new() -> Link {
        let grid = || std::array::from_fn(|_| std::array::from_fn(|_| AtomicU8::new(0)));
        Link {
            alive: AtomicBool::new(false),
            rt: Ring::new(),
            ui: Ring::new(),
            period: AtomicI16::new(120),
            pattern: AtomicU8::new(0),
            cue: AtomicI16::new(-1),
            scale: AtomicU8::new(0),
            preset: AtomicU8::new(0),
            page: AtomicU8::new(0),
            dir: AtomicU8::new(0),
            pos: grid(),
            lstart: grid(),
            llen: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU8::new(6))),
            cv: std::array::from_fn(|_| AtomicI16::new(0)),
            dur: std::array::from_fn(|_| AtomicI16::new(0)),
            mute: std::array::from_fn(|_| AtomicBool::new(false)),
        }
    }

    /// Teletype `KR.*` op shapes: (params, returns a value, can be set).
    pub const fn shape(op: KrOp) -> (u8, bool, bool) {
        match op {
            KrOp::Pre | KrOp::Period | KrOp::Pat | KrOp::Scale | KrOp::Pg | KrOp::Cue | KrOp::Dir => (0, true, true),
            KrOp::Pos | KrOp::LSt | KrOp::LLen => (2, true, true),
            KrOp::Res => (2, false, false),
            KrOp::Cv | KrOp::Dur => (1, true, false),
            KrOp::Mute => (1, true, true),
            KrOp::TMute | KrOp::Clk => (1, false, false),
        }
    }

    /// Tracks a Teletype track argument names: 1-4, or 0 for all.
    fn tracks(x: i16) -> std::ops::Range<usize> {
        match x {
            0 => 0..TRACKS,
            1..=4 => (x as usize - 1)..x as usize,
            _ => 0..0,
        }
    }

    /// Parameters a `KR.POS`-style argument names: 1 trigger, 2 note,
    /// 3 octave, 4 duration, 0 for all.
    fn params(y: i16) -> std::ops::Range<usize> {
        match y {
            0 => 0..PARAMS,
            1..=4 => (y as usize - 1)..y as usize,
            _ => 0..0,
        }
    }

    /// A get: `args` in the order they are written.
    pub fn get(&self, op: KrOp, args: &[i16]) -> i16 {
        if !self.alive.load(Ordering::Relaxed) {
            return 0;
        }
        let a = |i: usize| args.get(i).copied().unwrap_or(0);
        let first = |x: i16| Link::tracks(x).next();
        let at = |m: &[[AtomicU8; PARAMS]; TRACKS]| match (first(a(0)), Link::params(a(1)).next()) {
            (Some(t), Some(p)) => m[t][p].load(Ordering::Relaxed) as i16,
            _ => 0,
        };
        match op {
            KrOp::Pre => self.preset.load(Ordering::Relaxed) as i16,
            KrOp::Period => self.period.load(Ordering::Relaxed),
            KrOp::Pat => self.pattern.load(Ordering::Relaxed) as i16,
            KrOp::Scale => self.scale.load(Ordering::Relaxed) as i16,
            KrOp::Pg => self.page.load(Ordering::Relaxed) as i16,
            KrOp::Cue => self.cue.load(Ordering::Relaxed),
            KrOp::Dir => self.dir.load(Ordering::Relaxed) as i16,
            KrOp::Pos => at(&self.pos),
            KrOp::LSt => at(&self.lstart),
            KrOp::LLen => at(&self.llen),
            KrOp::Cv => first(a(0)).map_or(0, |t| self.cv[t].load(Ordering::Relaxed)),
            KrOp::Dur => first(a(0)).map_or(0, |t| self.dur[t].load(Ordering::Relaxed)),
            KrOp::Mute => first(a(0)).map_or(0, |t| self.mute[t].load(Ordering::Relaxed) as i16),
            _ => 0,
        }
    }

    /// A set (or an op that only acts): `args` as written, then the value.
    pub fn set(&self, op: KrOp, args: &[i16], v: i16) {
        let a = |i: usize| args.get(i).copied().unwrap_or(0);
        let small = |x: i16| x.clamp(0, 255) as u8;
        let cmd = |op: u8, x: i16, y: i16| Cmd { op, a: small(x), b: small(y), v };
        match op {
            KrOp::Period => self.period.store(v.clamp(20, 32767), Ordering::Relaxed),
            KrOp::Mute => {
                for t in Link::tracks(a(0)) {
                    self.mute[t].store(v != 0, Ordering::Relaxed);
                }
            }
            KrOp::TMute => {
                for t in Link::tracks(a(0)) {
                    self.mute[t].fetch_xor(true, Ordering::Relaxed);
                }
            }
            KrOp::Pat => self.rt.push(cmd(OP_PAT, 0, 0).pack()),
            KrOp::Cue => self.rt.push(cmd(OP_CUE, 0, 0).pack()),
            KrOp::Pos => self.rt.push(cmd(OP_POS, a(0), a(1)).pack()),
            KrOp::Res => self.rt.push(cmd(OP_RES, a(0), a(1)).pack()),
            KrOp::Clk => self.rt.push(cmd(OP_CLK, a(0), 0).pack()),
            KrOp::Scale => self.ui.push(cmd(OP_SCALE, 0, 0).pack()),
            KrOp::LSt => self.ui.push(cmd(OP_LST, a(0), a(1)).pack()),
            KrOp::LLen => self.ui.push(cmd(OP_LLEN, a(0), a(1)).pack()),
            KrOp::Dir => self.ui.push(cmd(OP_DIR, 0, 0).pack()),
            KrOp::Pre => self.ui.push(cmd(OP_PRE, 0, 0).pack()),
            KrOp::Pg => self.ui.push(cmd(OP_PG, 0, 0).pack()),
            KrOp::Cv | KrOp::Dur => {}
        }
    }
}

impl Default for Link {
    fn default() -> Self {
        Link::new()
    }
}

/// The link the device's Kria and Teletype share.
pub fn link() -> Arc<Link> {
    static L: OnceLock<Arc<Link>> = OnceLock::new();
    Arc::clone(L.get_or_init(|| Arc::new(Link::new())))
}

// ---------------------------------------------------------------- the sequencer

/// One track's playing state.
#[derive(Clone, Copy, Debug)]
struct TrackRt {
    pos: [u8; PARAMS],
    pos_mul: [u8; PARAMS],
    advancing: [bool; PARAMS],
    note: u8,
    alt: u8,
    oct: u8,
    glide: u8,
    /// Gate length for the current step, in samples.
    dur: f64,
    rpt: u8,
    rpt_bits: u8,
    last_clock: Option<u64>,
    /// Samples between this track's last two clocks.
    delta: f64,
    gate: bool,
    gate_off: u64,
    /// Bumped on every new gate, so a retrigger is seen even when the gate
    /// never went low in between.
    strikes: u32,
    repeats: u8,
    active_rpt: u8,
    next_repeat: f64,
    rpt_period: f64,
    /// Pitch in semitones: where it is (gliding) and where it's going.
    cv: f32,
    cv_target: f32,
    slew: f32,
}

impl Default for TrackRt {
    fn default() -> Self {
        TrackRt {
            pos: [5; PARAMS],
            pos_mul: [1; PARAMS],
            advancing: [true; PARAMS],
            note: 0,
            alt: 0,
            oct: 0,
            glide: 0,
            dur: 0.0,
            rpt: 1,
            rpt_bits: 1,
            last_clock: None,
            delta: 0.0,
            gate: false,
            gate_off: 0,
            strikes: 0,
            repeats: 0,
            active_rpt: 1,
            next_repeat: 0.0,
            rpt_period: 0.0,
            cv: 0.0,
            cv_target: 0.0,
            slew: 0.0,
        }
    }
}

/// The sequencer itself. Runs on the audio thread; time is in samples.
struct Seq {
    d: Data,
    pattern: usize,
    cue_next: Option<usize>,
    cue_count: u32,
    t: [TrackRt; TRACKS],
    mute: [bool; TRACKS],
    now: u64,
    rate: f32,
    /// A guess at the step length before a track has had two clocks.
    step_guess: f64,
    rng: u32,
}

impl Seq {
    fn new() -> Seq {
        let mut s = Seq { d: Data::default(), pattern: 0, cue_next: None, cue_count: 0, t: [TrackRt::default(); TRACKS], mute: [false; TRACKS], now: 0, rate: 48_000.0, step_guess: 6000.0, rng: 0x9e37_79b9, };
        s.reset();
        s
    }

    fn rand(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    fn track(&self, t: usize) -> &Track {
        &self.d.p[self.pattern].t[t]
    }

    /// Back to the top: every parameter sits on its loop end, one division
    /// along, so the next clock plays each loop's first step.
    fn reset(&mut self) {
        for t in 0..TRACKS {
            let tr = *self.track(t);
            for p in 0..PARAMS {
                self.t[t].pos[p] = tr.lend(p);
                self.t[t].pos_mul[p] = tr.tmul[p].max(1);
                self.t[t].advancing[p] = true;
            }
            self.t[t].last_clock = None;
        }
        self.cue_count = 0;
    }

    /// Moves parameter `p` of track `t` on by one clock. True when it lands
    /// on a new step and that step's probability says to take its value.
    fn next_step(&mut self, t: usize, p: usize) -> bool {
        let tr = *self.track(t);
        let rt = &mut self.t[t];
        rt.pos_mul[p] = rt.pos_mul[p].saturating_add(1);
        if rt.pos_mul[p] < tr.tmul[p].max(1) {
            return false;
        }
        rt.pos_mul[p] = 0;
        let (start, end, len) = (tr.lstart[p], tr.lend(p), tr.llen[p].max(1));
        let forward = |pos: u8| if pos == end { start } else { (pos + 1) % STEPS as u8 };
        let reverse = |pos: u8| if pos == start { end } else { (pos + STEPS as u8 - 1) % STEPS as u8 };
        let pos = rt.pos[p];
        let r = self.rand();
        let (coin, pick) = (r & 1 == 1, r >> 8);
        let rt = &mut self.t[t];
        rt.pos[p] = match tr.dir {
            1 => reverse(pos),
            2 => {
                if pos == end {
                    rt.advancing[p] = false;
                }
                if pos == start {
                    rt.advancing[p] = true;
                }
                if rt.advancing[p] { forward(pos) } else { reverse(pos) }
            }
            3 => if coin { forward(pos) } else { reverse(pos) },
            4 => ((start as u32 + pick % len as u32) % STEPS as u32) as u8,
            _ => forward(pos),
        };
        let prob = tr.prob[p][rt.pos[p] as usize];
        let r = self.rand() & 0xff;
        match prob {
            0 => false,
            1 => r > 192,
            2 => r > 128,
            _ => true,
        }
    }

    /// The note parameters' clock.
    fn clock_note(&mut self, t: usize) {
        let tr = *self.track(t);
        if self.next_step(t, DUR) {
            let rt = &mut self.t[t];
            let unscaled = (tr.v[DUR][rt.pos[DUR] as usize] as f64 + 1.0) * (tr.dur_mul.max(1) as f64 * 4.0);
            rt.dur = unscaled / 384.0 * rt.delta * tr.tmul[TR].max(1) as f64;
        }
        if self.next_step(t, OCT) {
            let rt = &mut self.t[t];
            rt.oct = (tr.octshift + tr.v[OCT][rt.pos[OCT] as usize]).min(5);
        }
        if self.next_step(t, NOTE) {
            self.t[t].note = tr.v[NOTE][self.t[t].pos[NOTE] as usize];
        }
        if self.next_step(t, ALT) {
            self.t[t].alt = tr.v[ALT][self.t[t].pos[ALT] as usize];
        }
        if self.next_step(t, GLIDE) {
            self.t[t].glide = tr.v[GLIDE][self.t[t].pos[GLIDE] as usize];
        }
    }

    fn set_note(&mut self, t: usize) {
        let scale = self.d.scales[self.d.p[self.pattern].scale as usize % SCALES];
        let rt = &mut self.t[t];
        rt.cv_target = pitch(&scale, rt.note, rt.alt, rt.oct) as f32;
        // Glide: 20 ms a step of the glide parameter, set with the note.
        let samples = rt.glide as f32 * 0.020 * self.rate;
        rt.slew = if samples < 1.0 { 0.0 } else { (rt.cv_target - rt.cv) / samples };
        if rt.slew == 0.0 {
            rt.cv = rt.cv_target;
        }
    }

    fn gate_on(&mut self, t: usize, len: f64) {
        let now = self.now;
        let rt = &mut self.t[t];
        rt.gate = true;
        rt.strikes = rt.strikes.wrapping_add(1);
        rt.gate_off = now + len.max(1.0) as u64;
    }

    /// One clock for one track.
    fn clock_track(&mut self, t: usize) {
        let now = self.now;
        let rt = &mut self.t[t];
        rt.delta = match rt.last_clock {
            Some(last) if now > last => (now - last) as f64,
            _ => self.step_guess,
        };
        rt.last_clock = Some(now);
        let tr = *self.track(t);
        let tr_next = self.next_step(t, TR);
        let is_trig = tr.v[TR][self.t[t].pos[TR] as usize] != 0;
        if !tr.trigger_clocked {
            self.clock_note(t);
        }
        if self.next_step(t, RPT) {
            let i = self.t[t].pos[RPT] as usize;
            self.t[t].rpt = tr.v[RPT][i].clamp(1, 5);
            self.t[t].rpt_bits = tr.rpt_bits[i];
        }
        if tr_next && is_trig && !self.mute[t] {
            let rt = &mut self.t[t];
            rt.active_rpt = rt.rpt;
            rt.repeats = rt.rpt - 1;
            if rt.repeats > 0 {
                rt.rpt_period = rt.delta * tr.tmul[TR].max(1) as f64 / rt.rpt as f64;
                rt.next_repeat = now as f64 + rt.rpt_period;
            }
            if rt.rpt_bits & 1 != 0 {
                if tr.trigger_clocked {
                    self.clock_note(t);
                }
                self.set_note(t);
                let len = self.t[t].dur / self.t[t].rpt as f64;
                self.gate_on(t, len);
            }
        }
    }

    /// The master clock: cue points, then every track it drives.
    fn clock_all(&mut self, tracks: bool) {
        self.cue_count += 1;
        if self.cue_count > self.d.cue_steps as u32 {
            self.cue_count = 0;
            if let Some(n) = self.cue_next.take() {
                self.pattern = n;
            }
        }
        if tracks {
            for t in 0..TRACKS {
                self.clock_track(t);
            }
        }
    }

    /// Gate ends and ratchets due by now.
    fn timers(&mut self) {
        let now = self.now;
        for t in 0..TRACKS {
            if self.t[t].repeats > 0 && now as f64 >= self.t[t].next_repeat {
                let rt = &mut self.t[t];
                let bit = rt.active_rpt - rt.repeats;
                rt.repeats -= 1;
                rt.next_repeat += rt.rpt_period;
                if rt.rpt_bits & (1 << bit) != 0 {
                    if self.track(t).trigger_clocked {
                        self.clock_note(t);
                    }
                    self.set_note(t);
                    let len = self.t[t].dur / self.t[t].rpt.max(1) as f64;
                    self.gate_on(t, len);
                }
            }
            let rt = &mut self.t[t];
            if rt.gate && now >= rt.gate_off {
                rt.gate = false;
            }
        }
    }

    /// Glides move on by `n` samples.
    fn glide(&mut self, n: usize) {
        for rt in self.t.iter_mut() {
            if rt.slew != 0.0 {
                rt.cv += rt.slew * n as f32;
                if (rt.slew > 0.0 && rt.cv >= rt.cv_target) || (rt.slew < 0.0 && rt.cv <= rt.cv_target) {
                    rt.cv = rt.cv_target;
                    rt.slew = 0.0;
                }
            }
        }
    }
}

// ---------------------------------------------------------------- the app

// Pages, numbered as Teletype's KR.PG numbers them where Kria has one.
const P_TRIG: u8 = 0;
const P_RPT: u8 = 1;
const P_NOTE: u8 = 2;
const P_ALT: u8 = 3;
const P_OCT: u8 = 4;
const P_GLIDE: u8 = 5;
const P_DUR: u8 = 6;
const P_LOOP: u8 = 7;
const P_SCALE: u8 = 8;
const P_PATTERN: u8 = 9;
const P_TIME: u8 = 10;
const P_PROB: u8 = 11;

/// The parameter a page edits.
fn page_param(page: u8) -> Option<usize> {
    match page {
        P_TRIG => Some(TR),
        P_RPT => Some(RPT),
        P_NOTE => Some(NOTE),
        P_ALT => Some(ALT),
        P_OCT => Some(OCT),
        P_GLIDE => Some(GLIDE),
        P_DUR => Some(DUR),
        _ => None,
    }
}

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "kria",
        layers: vec![
            Layer::Native(P_TRIG, "TRIG"),
            Layer::Native(P_NOTE, "NOTE"),
            Layer::Native(P_OCT, "OCTAVE"),
            Layer::Native(P_DUR, "DUR"),
            Layer::Native(P_RPT, "RATCHET"),
            Layer::Native(P_ALT, "ALT NOTE"),
            Layer::Native(P_GLIDE, "GLIDE"),
            Layer::Native(P_LOOP, "LOOP"),
            Layer::Native(P_TIME, "TIME"),
            Layer::Native(P_PROB, "PROB"),
            Layer::Native(P_SCALE, "SCALE"),
            Layer::Native(P_PATTERN, "PATTERN"),
            Layer::Controls,
            Layer::Moments,
        ],
        // The D-pad picks the track and edits steps, so no dials ride on it.
        hero: Vec::new(),
        browse: None,
        routes: Routes { stick_x: None, stick_y: None, hand_l: None, hand_r: None },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: true,
    }
}

enum UiCmd {
    Pattern(usize),
    Cue(usize),
    Reset,
}

/// What the audio thread shows the editor.
#[derive(Clone, Copy, Default)]
struct View {
    pattern: usize,
    cue: Option<usize>,
    /// Clocks since the last cue point.
    cue_count: u32,
    pos: [[u8; PARAMS]; TRACKS],
    gate: [bool; TRACKS],
    semis: [f32; TRACKS],
}

const N_OUTS: usize = 8;

struct Shared {
    data: Mutex<Data>,
    cmds: Mutex<VecDeque<UiCmd>>,
    view: Mutex<View>,
    clock_src: AtomicU8,
    /// The transport for the Internal / Clock input / Teletype clocks (the
    /// Device clock follows the device transport instead).
    running: AtomicBool,
    clock: Arc<Clock>,
    clock_in: Arc<AtomicF32>,
    /// MIDI note for 0 V (scale root, octave 0).
    base: AtomicU8,
    wave: AtomicUsize,
    release: AtomicF32,
    level: AtomicF32,
    /// TR 1-4 then CV 1-4: 0 = not routed, else modbus target + 1.
    outs: [AtomicUsize; N_OUTS],
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
}

const C_CLOCK: usize = 1;
const C_PERIOD: usize = 2;
const C_TRACK: usize = 3;
const C_DIR: usize = 4;
const C_DURMUL: usize = 5;
const C_OCTSHIFT: usize = 6;
const C_MUTE: usize = 7;
const C_TRIGCLK: usize = 8;
const C_NOTESYNC: usize = 9;
const C_LOOPSYNC: usize = 10;
const C_CUE: usize = 11;
const C_CUESTEPS: usize = 12;
const C_BASE: usize = 13;
const C_ROUTE: usize = 0;
const C_WAVE: usize = 14;
const C_RELEASE: usize = 15;
const C_LEVEL: usize = 16;
const C_PRESET: usize = 17;
const C_LOADPRE: usize = 18;
const C_SAVEPRE: usize = 19;
const C_SAVE: usize = 20;
const C_RESET: usize = 21;
const C_OUTS: usize = 22;
const N_CONTROLS: usize = C_OUTS + 2 * N_OUTS;

const WAVES: [&str; 4] = ["Sine", "Triangle", "Saw", "Square"];

pub struct KriaApp {
    p: Arc<Shared>,
    link: Arc<Link>,
    mods: Arc<ModBus>,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
    dir: PathBuf,
    track: usize,
    step: usize,
    /// The parameter LOOP, TIME and PROB act on: the last one edited.
    target: usize,
    loop_first: Option<usize>,
    /// Pattern pads: when each went down, for hold-to-copy.
    held_since: [Option<std::time::Instant>; 16],
    cue_mode: bool,
    preset: usize,
    pads_down: [bool; 16],
    route: NoteRoute,
    note_out: Option<NoteOut>,
    status: String,
    grid: Arc<Grid>,
    gk: GridKeys,
}

/// Ansible's modifier keys on the grid's bottom row (held).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum GridMod {
    #[default]
    None,
    Loop,
    Time,
    Prob,
}

/// What the grid's held keys are in the middle of.
#[derive(Clone, Debug, Default)]
struct GridKeys {
    modifier: GridMod,
    /// Pattern key held: a pattern tap cues instead of switching.
    cue: bool,
    /// Keys down in the current loop gesture, its first and last step and
    /// the row it started on (the track, on the trigger page).
    loop_count: u8,
    loop_first: usize,
    loop_last: Option<usize>,
    loop_row: usize,
    /// When each key went down (row-major 8 x 16): taps vs holds.
    down_at: Vec<Option<std::time::Instant>>,
}

#[allow(dead_code)] // not used by the main binary
impl KriaApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        Self::with(Path::new(SAVE_DIR), link(), sensitivity, nav, mods, bus, mixer)
    }

    pub fn with(dir: &Path, link: Arc<Link>, sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let (route, out) = NoteRoute::new(None, APP_NAME, "kria", true);
        let data = std::fs::read_to_string(dir.join("current.json")).ok().and_then(|t| serde_json::from_str::<Data>(&t).ok()).unwrap_or_else(demo);
        link.alive.store(true, Ordering::Relaxed);
        let app = Self {
            p: Arc::new(Shared {
                data: Mutex::new(data),
                cmds: Mutex::new(VecDeque::new()),
                view: Mutex::new(View::default()),
                clock_src: AtomicU8::new(C_DEVICE),
                running: AtomicBool::new(false),
                clock: Clock::shared(),
                clock_in: mods.register(format!("{APP_NAME}: Clock")),
                base: AtomicU8::new(36),
                wave: AtomicUsize::new(3),
                release: AtomicF32::new(0.2),
                level: AtomicF32::new(0.7),
                outs: std::array::from_fn(|_| AtomicUsize::new(0)),
                mix_level,
                ext_mix_level,
                output,
            }),
            link,
            mods,
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            dir: dir.to_path_buf(),
            track: 0,
            step: 0,
            target: TR,
            loop_first: None,
            held_since: [None; 16],
            cue_mode: false,
            preset: 0,
            pads_down: [false; 16],
            route,
            note_out: Some(out),
            status: String::new(),
            grid: grid_kit::grid(),
            gk: GridKeys { down_at: vec![None; GRID_ROWS * GRID_COLS], ..GridKeys::default() },
        };
        app.grid.register(APP_NAME);
        app.publish_data();
        app
    }

    /// Follows `clock` instead of the device transport (tests).
    pub fn following(mut self, clock: Arc<Clock>) -> Self {
        Arc::get_mut(&mut self.p).expect("set the clock before anything else holds Kria's state").clock = clock;
        self
    }

    pub fn with_notes(mut self, bus: Option<Arc<NoteBus>>) -> Self {
        let (route, out) = NoteRoute::new(bus, APP_NAME, "kria", true);
        self.route = route;
        self.note_out = Some(out);
        self
    }

    fn view(&self) -> View {
        self.p.view.lock().map(|v| *v).unwrap_or_default()
    }

    fn data(&self) -> Data {
        self.p.data.lock().map(|d| *d).unwrap_or_default()
    }

    /// The pattern being played is the one being edited.
    fn pattern(&self) -> usize {
        self.view().pattern
    }

    fn edit(&mut self, f: impl FnOnce(&mut Data, usize)) {
        let pat = self.pattern();
        if let Ok(mut d) = self.p.data.lock() {
            f(&mut d, pat);
        }
        self.publish_data();
    }

    fn send(&self, c: UiCmd) {
        if let Ok(mut q) = self.p.cmds.lock() {
            q.push_back(c);
        }
    }

    /// What Teletype reads that lives in the data (loops, scale, direction).
    fn publish_data(&self) {
        let d = self.data();
        let pat = &d.p[self.pattern() % PATTERNS];
        for t in 0..TRACKS {
            for p in 0..PARAMS {
                self.link.lstart[t][p].store(pat.t[t].lstart[p], Ordering::Relaxed);
                self.link.llen[t][p].store(pat.t[t].llen[p], Ordering::Relaxed);
            }
        }
        self.link.scale.store(pat.scale, Ordering::Relaxed);
        self.link.dir.store(pat.t[self.track].dir, Ordering::Relaxed);
        self.link.preset.store(self.preset as u8, Ordering::Relaxed);
    }

    /// Sets the loop of `param` on `track`, and wherever Loop Sync spreads it.
    fn set_loop(&mut self, track: usize, param: usize, start: usize, len: usize) {
        let (start, len) = ((start % STEPS) as u8, len.clamp(1, STEPS) as u8);
        self.edit(|d, pat| {
            let sync = d.loop_sync;
            for t in 0..TRACKS {
                for p in 0..PARAMS {
                    let hit = match sync {
                        0 => t == track && p == param,
                        1 => t == track,
                        _ => true,
                    };
                    if hit {
                        d.p[pat].t[t].lstart[p] = start;
                        d.p[pat].t[t].llen[p] = len;
                    }
                }
            }
        });
    }

    fn step_value(&mut self, d: i32) {
        let (t, s) = (self.track, self.step);
        let Some(param) = self.kit_page().and_then(page_param) else { return };
        self.target = param;
        self.edit(|data, pat| {
            let tr = &mut data.p[pat].t[t];
            let lo = if param == RPT { 1 } else { 0 };
            let v = (tr.v[param][s] as i32 + d).clamp(lo, MAX[param] as i32) as u8;
            tr.v[param][s] = v;
            if param == RPT {
                tr.rpt_bits[s] = ((1u16 << v) - 1) as u8;
            }
            // Note sync: giving a step a note means you want to hear it.
            if param == NOTE && data.note_sync {
                tr.v[TR][s] = 1;
            }
        });
    }

    fn toggle_trigger(&mut self, s: usize) {
        let t = self.track;
        self.edit(|d, pat| {
            let v = &mut d.p[pat].t[t].v[TR][s];
            *v = 1 - (*v).min(1);
        });
    }

    fn kit_page(&self) -> Option<u8> {
        match self.kit.layer() {
            Layer::Native(id, _) => Some(id),
            _ => None,
        }
    }

    fn pad(&mut self, page: u8, s: usize) {
        match page {
            P_TRIG => {
                self.step = s;
                self.target = TR;
                self.toggle_trigger(s);
            }
            P_LOOP => match self.loop_first.take() {
                None => {
                    self.loop_first = Some(s);
                    self.status = format!("loop from {}: tap the last step", s + 1);
                }
                Some(a) => {
                    let len = (s + STEPS - a) % STEPS + 1;
                    let (t, p) = (self.track, self.target);
                    self.set_loop(t, p, a, len);
                    self.status = format!("{} loop {}-{}", PARAM_NAMES[p], a + 1, s + 1);
                }
            },
            P_TIME => {
                let (t, p) = (self.track, self.target);
                self.edit(|d, pat| d.p[pat].t[t].tmul[p] = s as u8 + 1);
            }
            P_PROB => {
                let (t, p) = (self.track, self.target);
                self.step = s;
                self.edit(|d, pat| {
                    let v = &mut d.p[pat].t[t].prob[p][s];
                    *v = if *v == 0 { 3 } else { *v - 1 };
                });
            }
            P_SCALE => self.edit(|d, pat| d.p[pat].scale = s as u8),
            P_PATTERN => {} // on release: tap switches, hold copies
            _ => {
                // A value page: select the step; tap it again for its trigger.
                if let Some(p) = page_param(page) {
                    self.target = p;
                }
                if self.step == s {
                    self.toggle_trigger(s);
                }
                self.step = s;
            }
        }
    }

    fn pattern_release(&mut self, s: usize, held: std::time::Duration) {
        if held.as_millis() >= 500 {
            let from = self.pattern();
            self.edit(|d, _| d.p[s] = d.p[from]);
            self.status = format!("pattern {} copied to {}", from + 1, s + 1);
        } else if self.cue_mode {
            self.send(UiCmd::Cue(s));
        } else {
            self.send(UiCmd::Pattern(s));
        }
    }

    fn preset_path(&self, n: usize) -> PathBuf {
        self.dir.join(format!("preset_{}.json", n + 1))
    }

    fn load_preset(&mut self, n: usize) {
        match std::fs::read_to_string(self.preset_path(n)).ok().and_then(|t| serde_json::from_str::<Data>(&t).ok()) {
            Some(d) => {
                if let Ok(mut cur) = self.p.data.lock() {
                    *cur = d;
                }
                self.preset = n;
                self.status = format!("preset {} loaded", n + 1);
                self.publish_data();
            }
            None => self.status = format!("preset {} is empty", n + 1),
        }
    }

    fn save_to(&mut self, path: PathBuf, what: String) {
        let text = serde_json::to_string(&self.data()).unwrap_or_default();
        self.status = match std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(&path, text)) {
            Ok(()) => format!("saved {what}"),
            Err(e) => format!("could not save: {e}"),
        };
    }

    /// Teletype's commands for the editor side.
    fn link_commands(&mut self) {
        while let Some(x) = self.link.ui.pop() {
            let c = Cmd::unpack(x);
            match c.op {
                OP_SCALE => self.edit(|d, pat| d.p[pat].scale = c.v.clamp(0, SCALES as i16 - 1) as u8),
                OP_LST | OP_LLEN => {
                    let (ts, ps) = (Link::tracks(c.a as i16), Link::params(c.b as i16));
                    self.edit(|d, pat| {
                        for t in ts {
                            for p in ps.clone() {
                                if c.op == OP_LST {
                                    d.p[pat].t[t].lstart[p] = c.v.clamp(0, 15) as u8;
                                } else {
                                    d.p[pat].t[t].llen[p] = c.v.clamp(1, 16) as u8;
                                }
                            }
                        }
                    });
                }
                OP_DIR => {
                    let t = self.track;
                    self.edit(|d, pat| d.p[pat].t[t].dir = c.v.clamp(0, 4) as u8);
                }
                OP_PRE => self.load_preset(c.v.clamp(0, PRESETS as i16 - 1) as usize),
                OP_PG => {
                    if (0..=11).contains(&c.v) {
                        self.kit.set_native(c.v as u8);
                    }
                }
                _ => {}
            }
        }
        if let Some(p) = self.kit_page() {
            self.link.page.store(p, Ordering::Relaxed);
        }
    }
}

// ---------------------------------------------------------------- the grid

const GRID_ROWS: usize = 8;
const GRID_COLS: usize = 16;
/// Ansible's three standard brightnesses.
const L0: i32 = 4;
const L1: i32 = 8;
const L2: i32 = 12;
/// A pattern key held this long copies instead of switching.
const GRID_HOLD: std::time::Duration = std::time::Duration::from_millis(500);

/// The page that edits a parameter.
fn param_page(p: usize) -> u8 {
    match p {
        TR => P_TRIG,
        NOTE => P_NOTE,
        OCT => P_OCT,
        DUR => P_DUR,
        RPT => P_RPT,
        ALT => P_ALT,
        _ => P_GLIDE,
    }
}

/// A key's level on a value page: dimmer outside the loop, brighter while
/// the loop is being set.
fn in_loop_level(level: i32, inside: bool, m: GridMod) -> i32 {
    if !inside {
        level - 2
    } else if m == GridMod::Loop {
        level + 1
    } else {
        level
    }
}

impl KriaApp {
    /// The page the grid shows: the screen's page, or for the pad-only
    /// LOOP / TIME / PROB pages the parameter they act on (on the grid
    /// those are held modifiers instead).
    fn grid_page(&self) -> u8 {
        match self.kit_page() {
            Some(p) if p != P_LOOP && p != P_TIME && p != P_PROB => p,
            _ => param_page(self.target),
        }
    }

    fn set_page(&mut self, page: u8) {
        self.kit.set_native(page);
        if let Some(p) = page_param(page) {
            self.target = p;
        }
        self.loop_first = None;
    }

    /// Takes the grid's presses and draws it. Runs every frame whether
    /// Kria is on screen or not, so it plays from the Grid app's screen or
    /// a real grid while anything else is showing.
    fn grid_frame(&mut self) {
        if self.grid.focus().as_deref() != Some(APP_NAME) {
            return;
        }
        for k in self.grid.keys(APP_NAME) {
            if k.x < GRID_COLS && k.y < GRID_ROWS {
                self.grid_key(k.x, k.y, k.down);
            }
        }
        let leds = self.grid_leds();
        self.grid.show(APP_NAME, &leds);
    }

    fn grid_key(&mut self, x: usize, y: usize, z: bool) {
        let i = y * GRID_COLS + x;
        let held_for = if z {
            self.gk.down_at[i] = Some(std::time::Instant::now());
            None
        } else {
            self.gk.down_at[i].take().map(|t| t.elapsed())
        };
        if y == 7 {
            self.grid_nav(x, z);
            return;
        }
        let page = self.grid_page();
        let m = self.gk.modifier;
        let t = self.track;
        let row = 6 - y as u8; // a value counted up from the bottom of rows 1-7

        if page == P_PATTERN {
            if y == 0 {
                if let Some(held) = held_for {
                    if held >= GRID_HOLD {
                        let from = self.pattern();
                        self.edit(|d, _| d.p[x] = d.p[from]);
                        self.status = format!("pattern {} copied to {}", from + 1, x + 1);
                    } else if self.gk.cue {
                        self.send(UiCmd::Cue(x));
                    } else {
                        self.send(UiCmd::Pattern(x));
                    }
                }
            } else if y == 1 && z {
                self.edit(|d, _| d.cue_steps = x as u8);
            }
            return;
        }
        if page == P_SCALE {
            if z {
                self.grid_scale_key(x, y);
            }
            return;
        }
        let Some(param) = page_param(page) else { return };
        match m {
            GridMod::Time => {
                if z {
                    self.edit(|d, pat| d.p[pat].t[t].tmul[param] = x as u8 + 1);
                }
            }
            GridMod::Prob => {
                if z && (2..=5).contains(&y) {
                    self.edit(|d, pat| d.p[pat].t[t].prob[param][x] = 5 - y as u8);
                }
            }
            GridMod::Loop => {
                let note_sync = self.data().note_sync;
                match param {
                    // On the trigger page each row is a track.
                    TR if y < 4 || !z => {
                        let ps: &[usize] = if note_sync { &[TR, NOTE] } else { &[TR] };
                        if z || self.gk.loop_row == y {
                            self.grid_loop(x, y, z, y.min(TRACKS - 1), ps);
                        }
                    }
                    TR => {}
                    NOTE => {
                        let ps: &[usize] = if note_sync { &[NOTE, TR] } else { &[NOTE] };
                        self.grid_loop(x, y, z, t, ps);
                    }
                    DUR if y == 0 => {}
                    p => self.grid_loop(x, y, z, t, &[p]),
                }
            }
            GridMod::None => {
                if z {
                    self.grid_value_key(param, x, y, row);
                }
            }
        }
    }

    /// The bottom row: tracks, pages and the held modifiers.
    fn grid_nav(&mut self, x: usize, z: bool) {
        let page = self.grid_page();
        if !z {
            match x {
                10 | 11 | 12 => self.gk.modifier = GridMod::None,
                15 => self.gk.cue = false,
                _ => {}
            }
            return;
        }
        match x {
            0..=3 => {
                if self.gk.modifier == GridMod::Loop {
                    self.link.mute[x].fetch_xor(true, Ordering::Relaxed);
                } else {
                    self.track = x;
                    self.publish_data();
                }
            }
            5 => self.set_page(if page == P_TRIG { P_RPT } else { P_TRIG }),
            6 => self.set_page(if page == P_NOTE { P_ALT } else { P_NOTE }),
            7 => self.set_page(if page == P_OCT { P_GLIDE } else { P_OCT }),
            8 => self.set_page(P_DUR),
            10 => {
                self.gk.modifier = GridMod::Loop;
                self.gk.loop_count = 0;
            }
            11 => self.gk.modifier = GridMod::Time,
            12 => {
                if page_param(page).is_some() {
                    self.gk.modifier = GridMod::Prob;
                }
            }
            14 => self.set_page(P_SCALE),
            15 => {
                self.set_page(P_PATTERN);
                self.gk.cue = true;
            }
            _ => {}
        }
    }

    /// Held LOOP: press the first step, then the last. Let go after just
    /// one and it becomes the loop's start (same length), or, if it already
    /// was the start, a one-step loop.
    fn grid_loop(&mut self, x: usize, y: usize, z: bool, track: usize, params: &[usize]) {
        if z {
            if self.gk.loop_count == 0 {
                self.gk.loop_row = y;
                self.gk.loop_first = x;
                self.gk.loop_last = None;
            } else {
                let first = self.gk.loop_first;
                self.gk.loop_last = Some(x);
                let len = (x + STEPS - first) % STEPS + 1;
                for &p in params {
                    self.set_loop(track, p, first, len);
                }
            }
            self.gk.loop_count += 1;
            return;
        }
        self.gk.loop_count = self.gk.loop_count.saturating_sub(1);
        if self.gk.loop_count == 0 && self.gk.loop_last.is_none() {
            let first = self.gk.loop_first;
            let tr = self.data().p[self.pattern() % PATTERNS].t[track];
            let len = if tr.lstart[params[0]] as usize == first { 1 } else { tr.llen[params[0]] as usize };
            for &p in params {
                self.set_loop(track, p, first, len);
            }
        }
    }

    /// A key on a value page with no modifier held.
    fn grid_value_key(&mut self, param: usize, x: usize, y: usize, row: u8) {
        let t = self.track;
        self.step = x;
        self.edit(|d, pat| {
            let note_sync = d.note_sync;
            // On the trigger page each of the top four rows is a track.
            if param == TR {
                if y < TRACKS {
                    d.p[pat].t[y].v[TR][x] ^= 1;
                }
                return;
            }
            let tr = &mut d.p[pat].t[t];
            match param {
                NOTE => {
                    if note_sync {
                        // Pressing a step's own note again silences it.
                        if tr.v[TR][x] != 0 && tr.v[NOTE][x] == row {
                            tr.v[TR][x] = 0;
                        } else {
                            tr.v[TR][x] = 1;
                            tr.v[NOTE][x] = row;
                        }
                    } else {
                        tr.v[NOTE][x] = row;
                    }
                }
                ALT => tr.v[ALT][x] = row,
                GLIDE => tr.v[GLIDE][x] = row,
                OCT => {
                    if y == 0 {
                        if x <= 5 {
                            tr.octshift = x as u8;
                        }
                    } else {
                        // Ansible stores an octave below the shift as a
                        // negative offset; here offsets are 0-5, so the
                        // lowest a step goes is the shift itself.
                        tr.v[OCT][x] = row.saturating_sub(tr.octshift);
                    }
                }
                DUR => {
                    if y == 0 {
                        tr.dur_mul = x as u8 + 1;
                    } else {
                        tr.v[DUR][x] = y as u8 - 1;
                    }
                }
                RPT => match y {
                    0 => tr.v[RPT][x] = (tr.v[RPT][x] + 1).min(5),
                    6 => tr.v[RPT][x] = tr.v[RPT][x].saturating_sub(1).max(1),
                    _ => {
                        // Rows 2-6 toggle sub-triggers 5..1; the count is
                        // up to the highest one on.
                        let bits = tr.rpt_bits[x] ^ (1 << (5 - y));
                        tr.rpt_bits[x] = bits;
                        tr.v[RPT][x] = (8 - bits.leading_zeros() as u8).max(1);
                    }
                },
                _ => {}
            }
        });
    }

    fn grid_scale_key(&mut self, x: usize, y: usize) {
        self.edit(|d, pat| {
            if y < TRACKS && x <= 7 {
                let tr = &mut d.p[pat].t[y];
                match x {
                    1 => tr.trigger_clocked = !tr.trigger_clocked,
                    3..=7 => tr.dir = x as u8 - 3,
                    _ => {} // key 1 is Teletype clocking; Kria here has none per track
                }
            } else if x < 8 {
                if y > 4 {
                    d.p[pat].scale = ((y - 5) * 8 + x) as u8;
                }
            } else {
                // Right half: row 7 is the root, rows above it the six
                // intervals, each a column 0-7.
                let sc = d.p[pat].scale as usize % SCALES;
                d.scales[sc][6 - y] = (x - 8) as u8;
            }
        });
    }

    /// Kria's picture for the grid, the way Ansible draws it.
    fn grid_leds(&self) -> Leds {
        let mut l = Leds::new(GRID_ROWS, GRID_COLS);
        let view = self.view();
        let d = self.data();
        let pat = &d.p[view.pattern % PATTERNS];
        let t = self.track;
        let tr = &pat.t[t];
        let page = self.grid_page();
        let m = self.gk.modifier;
        let pos = view.pos;

        // Bottom row.
        l.fill_row(7, 5..9, L0);
        l.set(10, 7, L0);
        l.set(11, 7, L0);
        if page_param(page).is_some() {
            l.set(12, 7, L0);
        }
        l.set(14, 7, L0);
        l.set(15, 7, L0);
        for i in 0..TRACKS {
            let muted = self.link.mute[i].load(Ordering::Relaxed);
            let lv = match (muted, i == t) {
                (true, true) => L1,
                (true, false) => 2,
                (false, true) => L2,
                (false, false) => L0,
            };
            l.set(i, 7, lv + if !muted && view.gate[i] { 2 } else { 0 });
        }
        let active = match page {
            P_TRIG | P_RPT => 5,
            P_NOTE | P_ALT => 6,
            P_OCT | P_GLIDE => 7,
            P_DUR => 8,
            P_SCALE => 14,
            _ => 15,
        };
        // The second page of a key blinks.
        let alt = matches!(page, P_RPT | P_ALT | P_GLIDE);
        let blink = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis()) / 300) % 2 == 0;
        l.set(active, 7, if alt && blink { L1 } else { L2 });

        let param = page_param(page);
        match m {
            GridMod::Loop => l.set(10, 7, L1),
            GridMod::Time => {
                l.set(11, 7, L1);
                l.fill_row(1, 0..16, 3);
                if let Some(p) = param {
                    l.set(tr.tmul[p] as usize - 1, 1, L1);
                }
                return l;
            }
            GridMod::Prob => {
                l.set(12, 7, L1);
                l.fill_row(5, 0..16, 3);
                if let Some(p) = param {
                    for i in 0..STEPS {
                        let pr = tr.prob[p][i] as usize;
                        if pr > 0 {
                            l.set(i, 5 - pr, if i == pos[t][p] as usize { 10 } else { 6 });
                        }
                    }
                }
                return l;
            }
            GridMod::None => {}
        }

        match page {
            P_TRIG => {
                for (row, lane) in pat.t.iter().enumerate() {
                    for s in 0..STEPS {
                        if lane.v[TR][s] != 0 {
                            l.set(s, row, 3);
                        }
                        if lane.in_loop(TR, s) {
                            l.add(s, row, 2 + (m == GridMod::Loop) as i32);
                        }
                    }
                    l.add(pos[row][TR] as usize, row, 4);
                }
            }
            P_NOTE | P_ALT => {
                let p = if page == P_NOTE { NOTE } else { ALT };
                for s in 0..STEPS {
                    let y = 6 - tr.v[p][s].min(6) as usize;
                    let lv = if p == NOTE && d.note_sync { tr.v[TR][s].min(1) as i32 * 3 } else { 3 };
                    l.set(s, y, lv);
                    if tr.in_loop(p, s) {
                        l.add(s, y, 3 + 2 * (m == GridMod::Loop) as i32);
                    }
                }
                let ph = pos[t][p] as usize;
                l.add(ph, 6 - tr.v[p][ph].min(6) as usize, 4);
            }
            P_OCT => {
                l.fill_row(0, 0..6, 2);
                l.set(tr.octshift as usize, 0, L1);
                for s in 0..STEPS {
                    let sum = (tr.v[OCT][s] + tr.octshift).min(5) as usize;
                    for j in tr.octshift as usize..=sum {
                        l.set(s, 6 - j, in_loop_level(3, tr.in_loop(OCT, s), m));
                    }
                    if s == pos[t][OCT] as usize {
                        l.add(s, 6 - sum, 4);
                    }
                }
            }
            P_DUR => {
                l.set((tr.dur_mul as usize).saturating_sub(1), 0, L1);
                for s in 0..STEPS {
                    let v = tr.v[DUR][s].min(5) as usize;
                    for j in 0..=v {
                        l.set(s, 1 + j, in_loop_level(3, tr.in_loop(DUR, s), m));
                    }
                    if s == pos[t][DUR] as usize {
                        l.add(s, 1 + v, 4);
                    }
                }
            }
            P_RPT => {
                for s in 0..STEPS {
                    let bits = tr.rpt_bits[s];
                    for j in 0..5 {
                        let mut lv = if bits & (1 << j) != 0 { L0 } else { 0 };
                        if j < tr.v[RPT][s] as usize {
                            lv = in_loop_level(lv + 2, tr.in_loop(RPT, s), m);
                        }
                        l.set(s, 5 - j, lv);
                    }
                    if s == pos[t][RPT] as usize {
                        l.add(s, 5, 4);
                    }
                    // The add and remove rows.
                    l.set(s, 0, 2);
                    l.set(s, 6, 2);
                }
            }
            P_GLIDE => {
                for s in 0..STEPS {
                    let g = tr.v[GLIDE][s].min(6) as usize;
                    for j in 0..=g {
                        l.set(s, 6 - j, in_loop_level(L1 - (g - j) as i32, tr.in_loop(GLIDE, s), m));
                    }
                    if s == pos[t][GLIDE] as usize {
                        l.add(s, 6 - g, 4);
                    }
                }
            }
            P_SCALE => {
                for (row, lane) in pat.t.iter().enumerate() {
                    l.set(0, row, L0);
                    l.set(1, row, if lane.trigger_clocked { L1 } else { L0 });
                    for x in 3..=7 {
                        l.set(x, row, if lane.dir as usize == x - 3 { 4 } else { 2 });
                    }
                }
                for y in 0..7 {
                    l.set(8, y, L0);
                }
                l.fill_row(5, 0..8, 2);
                l.fill_row(6, 0..8, 2);
                let sc = pat.scale as usize % SCALES;
                l.set(sc % 8, 5 + sc / 8, L1);
                for (i, &v) in d.scales[sc].iter().enumerate() {
                    l.set(8 + v.min(7) as usize, 6 - i, L1);
                }
            }
            _ => {
                // Pattern.
                l.fill_row(0, 0..16, 3);
                l.set(view.pattern % PATTERNS, 0, L1);
                if let Some(c) = view.cue {
                    l.set(c, 0, L2);
                }
                l.set(d.cue_steps as usize, 1, L0);
                l.set(view.cue_count as usize, 1, L1);
            }
        }
        l
    }
}

/// The opening pattern: a short bass line, an arpeggio and a hi part, so
/// a new Kria plays something the moment it's started.
fn demo() -> Data {
    let mut d = Data::default();
    let p = &mut d.p[0];
    p.t[0].v[TR] = [1, 0, 0, 1, 0, 0, 1, 0, 1, 0, 0, 1, 0, 0, 1, 0];
    p.t[0].v[NOTE] = [0, 0, 0, 4, 0, 0, 3, 0, 0, 0, 0, 4, 0, 0, 6, 0];
    p.t[0].v[DUR] = [4; STEPS];
    p.t[1].v[TR] = [1; STEPS];
    p.t[1].v[NOTE] = [0, 2, 4, 6, 4, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    p.t[1].v[OCT] = [2; STEPS];
    p.t[1].v[DUR] = [2; STEPS];
    p.t[2].v[TR] = [0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0];
    p.t[2].v[NOTE] = [4, 4, 2, 2, 6, 6, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0];
    p.t[2].v[OCT] = [3; STEPS];
    p.t[2].v[DUR] = [5; STEPS];
    for t in 0..3 {
        p.t[t].llen = [8, 6, 8, 8, 8, 8, 8];
        p.t[t].lstart = [0; PARAMS];
    }
    p.t[1].llen[NOTE] = 6;
    p.t[2].llen[NOTE] = 7;
    p.t[2].tmul[NOTE] = 2;
    d.loop_sync = 2;
    d
}

fn release_ms(p: f32) -> f32 {
    10.0 * 300f32.powf(p.clamp(0.0, 1.0))
}

fn note_name(n: i32) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[n.rem_euclid(12) as usize], n.div_euclid(12) - 1)
}

impl KriaApp {
    fn text(&self, i: usize) -> (String, String) {
        let d = self.data();
        let tr = d.p[self.pattern() % PATTERNS].t[self.track];
        let onoff = |b: bool| if b { "on" } else { "off" }.to_string();
        match i {
            C_CLOCK => ("Clock".into(), CLOCKS[self.p.clock_src.load(Ordering::Relaxed) as usize % 4].into()),
            C_PERIOD => ("Internal period (KR.PERIOD)".into(), format!("{} ms", self.link.period.load(Ordering::Relaxed))),
            C_TRACK => ("Track (menu rows below)".into(), format!("{}", self.track + 1)),
            C_DIR => ("Direction".into(), DIRS[tr.dir as usize % 5].into()),
            C_DURMUL => ("Duration multiplier".into(), format!("{}", tr.dur_mul)),
            C_OCTSHIFT => ("Octave shift".into(), format!("+{}", tr.octshift)),
            C_MUTE => ("Mute".into(), onoff(self.link.mute[self.track].load(Ordering::Relaxed))),
            C_TRIGCLK => ("Notes move on triggers only".into(), onoff(tr.trigger_clocked)),
            C_NOTESYNC => ("Note sync".into(), onoff(d.note_sync)),
            C_LOOPSYNC => ("Loop sync".into(), LOOP_SYNC[d.loop_sync as usize % 3].into()),
            C_CUE => ("Pattern pads cue".into(), onoff(self.cue_mode)),
            C_CUESTEPS => ("Cue every".into(), format!("{} clocks", d.cue_steps as u32 + 1)),
            C_BASE => ("0 V is".into(), note_name(self.p.base.load(Ordering::Relaxed) as i32)),
            C_ROUTE => ("Plays".into(), self.route.label()),
            C_WAVE => ("Voice".into(), WAVES[self.p.wave.load(Ordering::Relaxed) % 4].into()),
            C_RELEASE => ("Release".into(), format!("{:.0} ms", release_ms(self.p.release.get()))),
            C_LEVEL => ("Level".into(), format!("{:.0}%", self.p.level.get() * 100.0)),
            C_PRESET => ("Preset".into(), format!("{}", self.preset + 1)),
            C_LOADPRE => ("Load preset".into(), if self.preset_path(self.preset).exists() { "press to load" } else { "(empty)" }.into()),
            C_SAVEPRE => ("Save to preset".into(), "press to save".into()),
            C_SAVE => ("Save".into(), "press to save (reloads next time)".into()),
            C_RESET => ("Reset".into(), "press: back to step 1".into()),
            _ => {
                let k = (i - C_OUTS) / 2;
                let name = if k < 4 { format!("TR {}", k + 1) } else { format!("CV {}", k - 3) };
                let r = self.p.outs[k].load(Ordering::Relaxed);
                if (i - C_OUTS) % 2 == 0 {
                    (format!("{name} to"), Patch::app_label(&self.mods, r))
                } else {
                    (format!("{name} input"), Patch::input_label(&self.mods, r))
                }
            }
        }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        (0..N_CONTROLS).map(|i| {
            let (n, v) = self.text(i);
            (n, v, false)
        }).collect()
    }

    fn control(&mut self, i: usize, d: i32) {
        if d == 0 {
            return;
        }
        let t = self.track;
        let step = d as f32 * 0.01 * self.sensitivity.get().max(0.01) * 10.0;
        let cycle = |v: u8, n: u8| ((v as i32 + d.signum()).rem_euclid(n as i32)) as u8;
        match i {
            C_CLOCK => self.p.clock_src.store(cycle(self.p.clock_src.load(Ordering::Relaxed), 4), Ordering::Relaxed),
            C_PERIOD => {
                let v = self.link.period.load(Ordering::Relaxed) as i32;
                self.link.period.store((v + d * if v >= 200 { 10 } else { 2 }).clamp(20, 2000) as i16, Ordering::Relaxed);
            }
            C_TRACK => self.track = cycle(self.track as u8, 4) as usize,
            C_DIR => self.edit(|x, pat| x.p[pat].t[t].dir = cycle(x.p[pat].t[t].dir, 5)),
            C_DURMUL => self.edit(|x, pat| x.p[pat].t[t].dur_mul = (x.p[pat].t[t].dur_mul as i32 + d).clamp(1, 16) as u8),
            C_OCTSHIFT => self.edit(|x, pat| x.p[pat].t[t].octshift = (x.p[pat].t[t].octshift as i32 + d).clamp(0, 5) as u8),
            C_MUTE => {
                self.link.mute[t].fetch_xor(true, Ordering::Relaxed);
            }
            C_TRIGCLK => self.edit(|x, pat| x.p[pat].t[t].trigger_clocked ^= true),
            C_NOTESYNC => self.edit(|x, _| x.note_sync ^= true),
            C_LOOPSYNC => self.edit(|x, _| x.loop_sync = cycle(x.loop_sync, 3)),
            C_CUE => self.cue_mode ^= true,
            C_CUESTEPS => self.edit(|x, _| x.cue_steps = (x.cue_steps as i32 + d).clamp(0, 15) as u8),
            C_BASE => self.p.base.store((self.p.base.load(Ordering::Relaxed) as i32 + d).clamp(0, 96) as u8, Ordering::Relaxed),
            C_ROUTE => self.route.step(d),
            C_WAVE => self.p.wave.store(cycle(self.p.wave.load(Ordering::Relaxed) as u8, 4) as usize, Ordering::Relaxed),
            C_RELEASE => self.p.release.set((self.p.release.get() + step).clamp(0.0, 1.0)),
            C_LEVEL => self.p.level.set((self.p.level.get() + step).clamp(0.0, 1.0)),
            C_PRESET => self.preset = cycle(self.preset as u8, PRESETS as u8) as usize,
            C_LOADPRE => self.load_preset(self.preset),
            C_SAVEPRE => self.save_to(self.preset_path(self.preset), format!("preset {}", self.preset + 1)),
            C_SAVE => self.save_to(self.dir.join("current.json"), "Kria".into()),
            C_RESET => self.send(UiCmd::Reset),
            _ => {
                let k = (i - C_OUTS) / 2;
                let r = self.p.outs[k].load(Ordering::Relaxed);
                let next = if (i - C_OUTS) % 2 == 0 { Patch::step_app(&self.mods, r, d) } else { Patch::step_input(&self.mods, r, d) };
                self.p.outs[k].store(next, Ordering::Relaxed);
            }
        }
        self.publish_data();
    }

    fn is_running(&self) -> bool {
        if self.p.clock_src.load(Ordering::Relaxed) == C_DEVICE {
            self.p.clock.running()
        } else {
            self.p.running.load(Ordering::Relaxed)
        }
    }
}

impl PlayHost for KriaApp {
    fn kit_control_count(&self) -> usize {
        N_CONTROLS
    }
    fn kit_label(&self, i: usize) -> String {
        self.text(i.min(N_CONTROLS - 1)).0
    }
    fn kit_value(&self, i: usize) -> String {
        self.text(i.min(N_CONTROLS - 1)).1
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        Some(match i {
            C_PERIOD => (self.link.period.load(Ordering::Relaxed) as f32 - 20.0) / 1980.0,
            C_RELEASE => self.p.release.get(),
            C_LEVEL => self.p.level.get(),
            _ => return None,
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        !matches!(i, C_PERIOD | C_RELEASE | C_LEVEL)
    }
    fn kit_pads_play(&self, _layer: u8) -> bool {
        false
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.control(i, delta);
    }
    fn kit_reset(&mut self, i: usize) {
        match i {
            C_PERIOD => self.link.period.store(120, Ordering::Relaxed),
            C_BASE => self.p.base.store(36, Ordering::Relaxed),
            C_ROUTE => self.route.reset(),
            C_RELEASE => self.p.release.set(0.2),
            C_LEVEL => self.p.level.set(0.7),
            i if i >= C_OUTS => self.p.outs[(i - C_OUTS) / 2].store(0, Ordering::Relaxed),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        match i {
            C_PERIOD => self.link.period.store((20.0 + v.clamp(0.0, 1.0) * 1980.0) as i16, Ordering::Relaxed),
            C_RELEASE => self.p.release.set(v.clamp(0.0, 1.0)),
            C_LEVEL => self.p.level.set(v.clamp(0.0, 1.0)),
            _ => {}
        }
    }
    fn kit_line(&self) -> String {
        if self.status.is_empty() {
            format!("track {}", self.track + 1)
        } else {
            self.status.clone()
        }
    }
    fn kit_pad_label(&self, layer: u8, pad: usize) -> String {
        match layer {
            P_TIME => format!("/{}", pad + 1),
            P_SCALE | P_PATTERN => format!("{}", pad + 1),
            _ => format!("{}", pad + 1),
        }
    }
    fn kit_pad_color(&self, layer: u8, pad: usize, held: bool) -> PadColor {
        if held {
            return PadColor::Red;
        }
        let view = self.view();
        let d = self.data();
        let tr = d.p[view.pattern % PATTERNS].t[self.track];
        let pos = view.pos[self.track];
        match layer {
            P_TRIG => {
                if pos[TR] as usize == pad && self.is_running() {
                    PadColor::Yellow
                } else if tr.v[TR][pad] != 0 {
                    PadColor::Green
                } else {
                    PadColor::Off
                }
            }
            P_LOOP => {
                if self.loop_first == Some(pad) {
                    PadColor::Red
                } else if tr.in_loop(self.target, pad) {
                    PadColor::Blue
                } else {
                    PadColor::Off
                }
            }
            P_TIME => if pad < tr.tmul[self.target] as usize { PadColor::Green } else { PadColor::Off },
            P_PROB => match tr.prob[self.target][pad] {
                3 => PadColor::Green,
                2 => PadColor::Yellow,
                1 => PadColor::Red,
                _ => PadColor::Off,
            },
            P_SCALE => if d.p[view.pattern % PATTERNS].scale as usize == pad { PadColor::Green } else { PadColor::Off },
            P_PATTERN => {
                if view.pattern == pad {
                    PadColor::Green
                } else if view.cue == Some(pad) {
                    PadColor::Yellow
                } else {
                    PadColor::Off
                }
            }
            _ => {
                let p = page_param(layer).unwrap_or(NOTE);
                if pad == self.step {
                    PadColor::Red
                } else if pos[p] as usize == pad && self.is_running() {
                    PadColor::Yellow
                } else if tr.in_loop(p, pad) {
                    PadColor::Blue
                } else {
                    PadColor::Off
                }
            }
        }
    }
}

impl App for KriaApp {
    fn play_surface(&self) -> bool {
        true
    }
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        None
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }
    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
        self.loop_first = None;
    }
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        self.kit.led_overlay(self)
    }
    fn running(&self) -> Option<bool> {
        Some(self.is_running())
    }
    fn toggle_running(&mut self) {
        if self.p.clock_src.load(Ordering::Relaxed) == C_DEVICE {
            self.p.clock.toggle();
        } else {
            self.p.running.fetch_xor(true, Ordering::Relaxed);
        }
    }
    fn needs_background_audio(&self) -> bool {
        // Teletype can clock Kria while another app is on screen.
        self.is_running() || self.p.clock_src.load(Ordering::Relaxed) == C_TELETYPE || self.mods.requested(APP_NAME)
    }
    fn background_tick(&mut self) {
        self.link_commands();
        self.grid_frame();
    }
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        if step.menu {
            let i = &step.input;
            self.list.navigate_input(i, N_CONTROLS, self.nav.get() as i32);
            let sel = self.list.selected.min(N_CONTROLS - 1);
            self.kit_edit(sel, i.knob2);
            if i.knob2_press {
                self.kit_reset(sel);
            }
            self.pads_down = [false; 16];
            return;
        }
        let Some(page) = step.native else {
            self.pads_down = [false; 16];
            return;
        };
        if input.nav_x != 0 {
            self.track = (self.track as i32 + input.nav_x).clamp(0, TRACKS as i32 - 1) as usize;
            self.loop_first = None;
            self.publish_data();
        }
        // Up raises the value (the D-pad counts down the screen).
        let up = -input.navigation_steps;
        if up != 0 {
            let (t, p, s) = (self.track, self.target, self.step);
            match page {
                P_SCALE => self.edit(|d, pat| {
                    let sc = d.p[pat].scale as usize % SCALES;
                    d.scales[sc][0] = (d.scales[sc][0] as i32 + up).clamp(0, 7) as u8;
                }),
                P_TIME => self.edit(|d, pat| d.p[pat].t[t].tmul[p] = (d.p[pat].t[t].tmul[p] as i32 + up).clamp(1, 16) as u8),
                P_PROB => self.edit(|d, pat| d.p[pat].t[t].prob[p][s] = (d.p[pat].t[t].prob[p][s] as i32 + up).clamp(0, 3) as u8),
                P_TRIG | P_LOOP | P_PATTERN => {}
                _ => self.step_value(up),
            }
        }
        for pad in 0..16 {
            let down = step.input.grid[pad];
            if down && !self.pads_down[pad] {
                self.held_since[pad] = Some(std::time::Instant::now());
                self.pad(page, pad);
            }
            if !down && self.pads_down[pad] {
                if let Some(t0) = self.held_since[pad].take() {
                    if page == P_PATTERN {
                        self.pattern_release(pad, t0.elapsed());
                    }
                }
            }
            self.pads_down[pad] = down;
        }
        if input.knob1_press {
            let s = self.step;
            self.toggle_trigger(s);
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if self.kit.menu {
            Text::new(APP_NAME, Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, LIT)).draw(f).ok();
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 44, 24, 11, &r, BG, DIM, ACCENT);
            if !self.status.is_empty() {
                Text::new(&self.status, Point::new(16, 330), MonoTextStyle::new(&SPLEEN_6X12, ACCENT)).draw(f).ok();
            }
            return;
        }
        let Some(page) = self.kit_page() else {
            let col = self.kit.column(self);
            let pal = kit::draw::Palette { bg: BG, ink: MID, accent: ACCENT, dim: DIM, faint: FAINT };
            kit::draw::column(f, &col, 16, 40, 350, 280, pal);
            return;
        };
        self.draw_page(f, page);
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kids_kit::screen_extra(&fb)
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
        Some(Box::new(Processor {
            p: Arc::clone(&self.p),
            link: Arc::clone(&self.link),
            mods: Arc::clone(&self.mods),
            seq: Seq::new(),
            notes: self.note_out.take().unwrap_or_else(NoteOut::detached),
            voices: [Voice::default(); TRACKS],
            strikes: [0; TRACKS],
            gates: [false; TRACKS],
            sounding: [None; TRACKS],
            follower: StepFollower::default(),
            internal: 0.0,
            clk_prev: false,
            was_running: false,
        }))
    }
}

impl KriaApp {
    fn draw_page(&self, f: &mut FrameBuffer, page: u8) {
        let view = self.view();
        let d = self.data();
        let pat = &d.p[view.pattern % PATTERNS];
        let tr = &pat.t[self.track];
        let small = |c| MonoTextStyle::new(&SPLEEN_6X12, c);
        let big = |c| MonoTextStyle::new(&SPLEEN_8X16, c);
        let running = self.is_running();
        // Header.
        let clock = match self.p.clock_src.load(Ordering::Relaxed) {
            C_DEVICE => format!("{:.0} bpm", self.p.clock.bpm()),
            C_INTERNAL => format!("{} ms", self.link.period.load(Ordering::Relaxed)),
            C_INPUT => "clock input".into(),
            _ => "KR.CLK".into(),
        };
        let cue = view.cue.map_or(String::new(), |c| format!(" > {}", c + 1));
        Text::new("KRIA", Point::new(8, 18), big(LIT)).draw(f).ok();
        Text::new(&format!("pattern {}{cue}   scale {}   {clock} {}", view.pattern + 1, pat.scale + 1, if running { "playing" } else { "stopped (F3)" }), Point::new(56, 17), small(MID)).draw(f).ok();
        let title = match page_param(page) {
            Some(p) => PARAM_NAMES[p].to_string(),
            None => match page {
                P_LOOP => format!("LOOP: {}", PARAM_NAMES[self.target]),
                P_TIME => format!("TIME: {}", PARAM_NAMES[self.target]),
                P_PROB => format!("PROB: {}", PARAM_NAMES[self.target]),
                P_SCALE => "SCALE".into(),
                _ => "PATTERN".into(),
            },
        };
        Text::new(&title, Point::new(500, 18), big(ACCENT)).draw(f).ok();

        // The four trigger lanes, always: Kria's home view.
        for t in 0..TRACKS {
            let y = 30 + t as i32 * 18;
            let lane = &pat.t[t];
            let muted = self.link.mute[t].load(Ordering::Relaxed);
            let label_col = if t == self.track { LIT } else { DIM };
            Text::new(&format!("{}{}", t + 1, if muted { "m" } else { "" }), Point::new(8, y + 12), small(label_col)).draw(f).ok();
            for s in 0..STEPS {
                let x = 40 + s as i32 * 37;
                let on = lane.v[TR][s] != 0;
                let head = running && view.pos[t][TR] as usize == s;
                let fill = if head { HEAD } else if on { if muted { MID } else { LIT } } else if lane.in_loop(TR, s) { KEY } else { FAINT };
                Rectangle::new(Point::new(x, y), Size::new(33, 14)).into_styled(PrimitiveStyle::with_fill(fill)).draw(f).ok();
            }
            if view.gate[t] {
                Rectangle::new(Point::new(26, y + 3), Size::new(8, 8)).into_styled(PrimitiveStyle::with_fill(HEAD)).draw(f).ok();
            }
        }

        // The page itself.
        let (x0, y0, h) = (40, 108, 150);
        match page {
            P_SCALE => {
                for s in 0..SCALES {
                    let (x, y) = (x0 + (s as i32 % 8) * 74, y0 + (s as i32 / 8) * 40);
                    let sel = pat.scale as usize == s;
                    Rectangle::new(Point::new(x, y), Size::new(68, 32)).into_styled(if sel { PrimitiveStyle::with_fill(MID) } else { PrimitiveStyle::with_stroke(DIM, 1) }).draw(f).ok();
                    let name = if s < 7 { ["Ionian", "Dorian", "Phrygian", "Lydian", "Mixolyd", "Aeolian", "Locrian"][s] } else { "chrom" };
                    Text::new(&format!("{} {name}", s + 1), Point::new(x + 4, y + 20), small(if sel { BG } else { MID })).draw(f).ok();
                }
                let sc = d.scales[pat.scale as usize % SCALES];
                let deg = degrees(&sc);
                let names: Vec<String> = deg.iter().map(|&s| note_name(self.p.base.load(Ordering::Relaxed) as i32 + s)).collect();
                Text::new(&format!("degrees: {}   root +{} (up/down)", names.join(" "), sc[0]), Point::new(x0, y0 + 100), small(LIT)).draw(f).ok();
            }
            P_PATTERN => {
                for s in 0..PATTERNS {
                    let (x, y) = (x0 + (s as i32 % 8) * 74, y0 + (s as i32 / 8) * 52);
                    let fill = if view.pattern == s { PrimitiveStyle::with_fill(LIT) } else if view.cue == Some(s) { PrimitiveStyle::with_fill(MID) } else { PrimitiveStyle::with_stroke(DIM, 1) };
                    Rectangle::new(Point::new(x, y), Size::new(68, 44)).into_styled(fill).draw(f).ok();
                    let used = d.p[s] != Pattern::default();
                    Text::new(&format!("{}{}", s + 1, if used { " *" } else { "" }), Point::new(x + 6, y + 26), big(if view.pattern == s { BG } else { MID })).draw(f).ok();
                }
                let how = if self.cue_mode { "tap: cue for the next cue point" } else { "tap: switch now" };
                Text::new(&format!("{how}   hold: copy this pattern there   * = has notes"), Point::new(x0, y0 + 124), small(DIM)).draw(f).ok();
            }
            P_TIME => {
                for p in 0..PARAMS {
                    let y = y0 + p as i32 * 20;
                    let sel = p == self.target;
                    Text::new(PARAM_NAMES[p], Point::new(x0, y + 12), small(if sel { LIT } else { DIM })).draw(f).ok();
                    for k in 0..16 {
                        let on = k < tr.tmul[p] as usize;
                        Rectangle::new(Point::new(x0 + 70 + k as i32 * 30, y), Size::new(26, 14)).into_styled(PrimitiveStyle::with_fill(if on { if sel { LIT } else { MID } } else { FAINT })).draw(f).ok();
                    }
                    Text::new(&format!("/{}", tr.tmul[p]), Point::new(x0 + 560, y + 12), small(MID)).draw(f).ok();
                }
            }
            _ => {
                let p = page_param(page).unwrap_or(self.target);
                let pos = view.pos[self.track][p] as usize;
                let max = MAX[p].max(1) as i32;
                for s in 0..STEPS {
                    let x = x0 + s as i32 * 37;
                    let inside = tr.in_loop(p, s);
                    Rectangle::new(Point::new(x, y0), Size::new(33, h as u32)).into_styled(PrimitiveStyle::with_fill(if inside { FAINT } else { BG })).draw(f).ok();
                    let (val, frac) = match page {
                        P_PROB => (tr.prob[p][s] as i32, tr.prob[p][s] as f32 / 3.0),
                        _ => (tr.v[p][s] as i32, if p == TR { tr.v[TR][s] as f32 } else if p == RPT { tr.v[RPT][s] as f32 / 5.0 } else { (tr.v[p][s] as f32 + 1.0) / (max as f32 + 1.0) }),
                    };
                    let bar = (frac * (h - 18) as f32) as i32;
                    let colour = if running && pos == s { HEAD } else if tr.v[TR][s] != 0 || p == TR { if inside { LIT } else { MID } } else { DIM };
                    if bar > 0 {
                        Rectangle::new(Point::new(x + 3, y0 + h - 16 - bar), Size::new(27, bar as u32)).into_styled(PrimitiveStyle::with_fill(colour)).draw(f).ok();
                    }
                    let label = match page {
                        P_PROB => ["0%", "25", "50", "100"][val.clamp(0, 3) as usize].to_string(),
                        _ if p == TR => String::new(),
                        _ => val.to_string(),
                    };
                    Text::new(&label, Point::new(x + 10, y0 + h - 3), small(MID)).draw(f).ok();
                    if s == self.step && page != P_LOOP {
                        Rectangle::new(Point::new(x, y0), Size::new(33, h as u32)).into_styled(PrimitiveStyle::with_stroke(ACCENT, 2)).draw(f).ok();
                    }
                    if page == P_LOOP && self.loop_first == Some(s) {
                        Rectangle::new(Point::new(x, y0), Size::new(33, h as u32)).into_styled(PrimitiveStyle::with_stroke(HEAD, 2)).draw(f).ok();
                    }
                }
                let extra = match p {
                    NOTE | ALT => {
                        let scale = d.scales[pat.scale as usize % SCALES];
                        let sem = pitch(&scale, tr.v[NOTE][self.step], tr.v[ALT][self.step], (tr.octshift + tr.v[OCT][self.step]).min(5));
                        format!("step {}: {}", self.step + 1, note_name(self.p.base.load(Ordering::Relaxed) as i32 + sem))
                    }
                    DUR => format!("step {}: {}/24 of a step x {} (dur mul)", self.step + 1, tr.v[DUR][self.step] as u32 + 1, tr.dur_mul),
                    GLIDE => format!("step {}: {} ms glide", self.step + 1, tr.v[GLIDE][self.step] as u32 * 20),
                    _ => format!("loop {}-{}  /{}  {}", tr.lstart[p] + 1, tr.lend(p) + 1, tr.tmul[p], DIRS[tr.dir as usize % 5]),
                };
                Text::new(&extra, Point::new(x0, y0 + h + 14), small(MID)).draw(f).ok();
            }
        }
        let hint = match page {
            P_TRIG => "pads: triggers   D-pad < >: track   F2: page   R1: menu   F3: play",
            P_LOOP => "tap the first step, then the last   (applies to the last page edited)",
            P_TIME => "pad n: divide the clock by n   up/down: finer",
            P_PROB => "pad: 100 > 50 > 25 > 0 %   up/down: the selected step",
            P_SCALE => "pad: pick a scale   up/down: move the root",
            P_PATTERN => "",
            _ => "pad: select a step (again: its trigger)   up/down: value   SELECT: trigger",
        };
        Text::new(hint, Point::new(8, 336), small(DIM)).draw(f).ok();
        let line = if self.status.is_empty() { format!("track {}   {}", self.track + 1, self.route.label()) } else { self.status.clone() };
        Text::new(&line, Point::new(8, 352), small(ACCENT)).draw(f).ok();
    }
}

/// The built-in voice, one per track, following the track's (gliding) pitch.
#[derive(Clone, Copy, Default)]
struct Voice {
    phase: f32,
    amp: f32,
    gate: bool,
    active: bool,
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

impl Voice {
    fn next(&mut self, freq: f32, wave: usize, rate: f32, attack: f32, release: f32) -> f32 {
        if !self.active {
            return 0.0;
        }
        let dt = (freq / rate).clamp(0.0, 0.45);
        let s = match wave {
            0 => (self.phase * std::f32::consts::TAU).sin(),
            1 => 4.0 * (self.phase - 0.5).abs() - 1.0,
            2 => 2.0 * self.phase - 1.0 - poly_blep(self.phase, dt),
            _ => {
                let naive = if self.phase < 0.5 { 1.0 } else { -1.0 };
                naive + poly_blep(self.phase, dt) - poly_blep((self.phase + 0.5) % 1.0, dt)
            }
        };
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        if self.gate {
            self.amp = (self.amp + attack).min(1.0);
        } else {
            self.amp *= release;
            if self.amp < 1e-4 {
                self.active = false;
            }
        }
        s * self.amp
    }
}

struct Processor {
    p: Arc<Shared>,
    link: Arc<Link>,
    mods: Arc<ModBus>,
    seq: Seq,
    notes: NoteOut,
    voices: [Voice; TRACKS],
    strikes: [u32; TRACKS],
    gates: [bool; TRACKS],
    sounding: [Option<u8>; TRACKS],
    follower: StepFollower,
    /// Samples since the internal clock's last step.
    internal: f64,
    clk_prev: bool,
    was_running: bool,
}

impl Processor {
    fn commands(&mut self) {
        if let Ok(d) = self.p.data.try_lock() {
            self.seq.d = *d;
        }
        let shared = Arc::clone(&self.p);
        if let Ok(mut q) = shared.cmds.try_lock() {
            while let Some(c) = q.pop_front() {
                match c {
                    UiCmd::Pattern(n) => {
                        self.seq.pattern = n % PATTERNS;
                        self.seq.cue_next = None;
                    }
                    UiCmd::Cue(n) => self.seq.cue_next = Some(n % PATTERNS),
                    UiCmd::Reset => self.seq.reset(),
                }
            }
        }
        while let Some(x) = self.link.rt.pop() {
            let c = Cmd::unpack(x);
            match c.op {
                OP_PAT => {
                    self.seq.pattern = c.v.clamp(0, PATTERNS as i16 - 1) as usize;
                    self.seq.cue_next = None;
                }
                OP_CUE => self.seq.cue_next = Some(c.v.clamp(0, PATTERNS as i16 - 1) as usize),
                OP_POS | OP_RES => {
                    for t in Link::tracks(c.a as i16) {
                        for p in Link::params(c.b as i16) {
                            let tr = *self.seq.track(t);
                            let rt = &mut self.seq.t[t];
                            if c.op == OP_POS {
                                rt.pos[p] = c.v.rem_euclid(STEPS as i16) as u8;
                                rt.pos_mul[p] = 0;
                            } else {
                                // Back to the loop's top: the next clock plays its first step.
                                rt.pos[p] = tr.lend(p);
                                rt.pos_mul[p] = tr.tmul[p].max(1);
                            }
                        }
                    }
                }
                OP_CLK => {
                    for t in Link::tracks(c.a as i16) {
                        self.seq.clock_track(t);
                    }
                }
                _ => {}
            }
        }
        for t in 0..TRACKS {
            self.seq.mute[t] = self.link.mute[t].load(Ordering::Relaxed);
        }
    }

    /// Gate edges become notes: to the built-in voice or the routed app.
    fn notes(&mut self) {
        let base = self.p.base.load(Ordering::Relaxed) as i32;
        for t in 0..TRACKS {
            let rt = self.seq.t[t];
            let struck = rt.strikes != self.strikes[t];
            if !struck && rt.gate == self.gates[t] {
                continue;
            }
            self.strikes[t] = rt.strikes;
            self.gates[t] = rt.gate;
            if let Some(n) = self.sounding[t].take() {
                if self.notes.external() {
                    self.notes.note_off(n);
                }
            }
            self.voices[t].gate = false;
            if rt.gate {
                let note = (base + rt.cv_target.round() as i32).clamp(0, 127) as u8;
                if self.notes.internal() {
                    // A new strike re-opens the gate from wherever the
                    // envelope is, so fast ratchets don't click.
                    let v = &mut self.voices[t];
                    v.gate = true;
                    v.active = true;
                } else if self.notes.external() {
                    self.notes.note_on(note, 100);
                }
                self.sounding[t] = Some(note);
            }
        }
    }

    fn publish(&self) {
        if let Ok(mut v) = self.p.view.try_lock() {
            v.pattern = self.seq.pattern;
            v.cue = self.seq.cue_next;
            v.cue_count = self.seq.cue_count;
            v.pos = std::array::from_fn(|t| self.seq.t[t].pos);
            v.gate = std::array::from_fn(|t| self.seq.t[t].gate);
            v.semis = std::array::from_fn(|t| self.seq.t[t].cv);
        }
        let l = &self.link;
        l.pattern.store(self.seq.pattern as u8, Ordering::Relaxed);
        l.cue.store(self.seq.cue_next.map_or(-1, |c| c as i16), Ordering::Relaxed);
        for t in 0..TRACKS {
            let rt = &self.seq.t[t];
            for p in 0..PARAMS {
                l.pos[t][p].store(rt.pos[p], Ordering::Relaxed);
            }
            l.cv[t].store((rt.cv * 16384.0 / 120.0).round().clamp(0.0, 16383.0) as i16, Ordering::Relaxed);
            l.dur[t].store((rt.dur / self.seq.rate as f64 * 1000.0).round().min(32767.0) as i16, Ordering::Relaxed);
        }
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.0);
        if channels == 0 || !rate.is_finite() || rate < 1000.0 {
            return;
        }
        let frames = out.len() / channels;
        self.seq.rate = rate;
        self.commands();
        let src = self.p.clock_src.load(Ordering::Relaxed);
        let snap = self.p.clock.snap();
        let running = if src == C_DEVICE { snap.running } else { self.p.running.load(Ordering::Relaxed) };
        if running && !self.was_running {
            self.seq.reset();
            self.internal = f64::MAX;
            self.follower = StepFollower::default();
        }
        if !running && self.was_running {
            for t in 0..TRACKS {
                self.seq.t[t].gate = false;
                self.seq.t[t].repeats = 0;
            }
        }
        self.was_running = running;
        let period = self.link.period.load(Ordering::Relaxed).max(20) as f64 * rate as f64 / 1000.0;
        self.seq.step_guess = match src {
            C_DEVICE => rate as f64 * 60.0 / snap.bpm.max(1.0) as f64 / 4.0,
            _ => period,
        };
        // The clock input (and Teletype) work whenever something is sent.
        let high = self.p.clock_in.get() > 0.5;
        if src == C_INPUT && high && !self.clk_prev && running {
            self.seq.clock_all(true);
        }
        self.clk_prev = high;

        let wave = self.p.wave.load(Ordering::Relaxed) % 4;
        let release = (-1.0 / (release_ms(self.p.release.get()) / 1000.0 * rate).max(1.0)).exp();
        let attack = 1.0 / (0.003 * rate);
        let level = self.p.level.get();
        let master = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0) * level * level * 0.35;
        let base = self.p.base.load(Ordering::Relaxed) as f32;
        let ms = (rate / 1000.0).ceil() as usize;
        let shared = Arc::clone(&self.p);
        let mut bus = shared.output.try_lock().ok();
        if let Some(b) = bus.as_mut() {
            b.clear();
        }
        self.notes();
        let mut done = 0;
        while done < frames {
            let chunk = ms.min(frames - done);
            if running {
                match src {
                    C_DEVICE => {
                        if self.follower.poll(&snap, snap.beat_at(done, rate), 4, 0.0).is_some() {
                            self.seq.clock_all(true);
                        }
                    }
                    C_INTERNAL => {
                        if self.internal >= period {
                            self.internal = 0.0;
                            self.seq.clock_all(true);
                        }
                        self.internal += chunk as f64;
                    }
                    _ => {}
                }
            }
            self.seq.timers();
            self.notes();
            self.notes.advance(chunk as u32);
            for k in 0..chunk {
                let mut s = 0.0;
                for t in 0..TRACKS {
                    let freq = 440.0 * 2f32.powf((base + self.seq.t[t].cv - 69.0) / 12.0);
                    s += self.voices[t].next(freq, wave, rate, attack, release);
                }
                let s = s * master;
                for o in out[(done + k) * channels..(done + k + 1) * channels].iter_mut() {
                    *o = s;
                }
                if let Some(b) = bus.as_mut() {
                    b.push(s);
                }
            }
            self.seq.glide(chunk);
            self.seq.now += chunk as u64;
            done += chunk;
        }
        for k in 0..N_OUTS {
            let target = self.p.outs[k].load(Ordering::Relaxed);
            if target > 0 {
                let rt = &self.seq.t[k % 4];
                let value = if k < 4 { rt.gate as u8 as f32 } else { rt.cv / 120.0 };
                if let Some(h) = self.mods.get(target - 1) {
                    h.set(value);
                }
            }
        }
        self.publish();
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(KriaApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()).with_notes(ctx.try_get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sequencer on an empty preset, 6000-sample steps (120 bpm 16ths).
    fn seq() -> Seq {
        let mut s = Seq::new();
        s.d = Data::default();
        s.reset();
        s
    }

    /// Clocks every track `n` times, a step apart, recording the gates that
    /// open (track, step index of the clock).
    fn run(s: &mut Seq, n: usize) -> Vec<(usize, usize)> {
        let mut hits = Vec::new();
        for i in 0..n {
            let before: Vec<u32> = s.t.iter().map(|t| t.strikes).collect();
            s.clock_all(true);
            for (t, b) in before.iter().enumerate() {
                if s.t[t].strikes != *b {
                    hits.push((t, i));
                }
            }
            for _ in 0..6 {
                s.now += 1000;
                s.timers();
            }
        }
        hits
    }

    fn track0(hits: &[(usize, usize)]) -> Vec<usize> {
        hits.iter().filter(|h| h.0 == 0).map(|h| h.1).collect()
    }

    #[test]
    fn triggers_play_their_steps_and_loop_at_the_loop_end() {
        let mut s = seq();
        s.d.p[0].t[0].v[TR][0] = 1;
        s.d.p[0].t[0].v[TR][2] = 1;
        // The default loop is steps 1-6.
        let hits = track0(&run(&mut s, 12));
        assert_eq!(hits, [0, 2, 6, 8]);
        s.d.p[0].t[0].llen[TR] = 3;
        s.reset();
        assert_eq!(track0(&run(&mut s, 6)), [0, 2, 3, 5], "a 3-step loop");
    }

    #[test]
    fn parameters_loop_and_divide_independently() {
        let mut s = seq();
        let t = &mut s.d.p[0].t[0];
        t.v[TR] = [1; STEPS];
        t.v[NOTE][..4].copy_from_slice(&[0, 1, 2, 3]);
        t.llen[TR] = 4;
        t.llen[NOTE] = 3;
        t.tmul[OCT] = 2;
        t.v[OCT][..2].copy_from_slice(&[0, 1]);
        t.llen[OCT] = 2;
        s.reset();
        let mut notes = Vec::new();
        let mut octs = Vec::new();
        for _ in 0..8 {
            s.clock_all(true);
            notes.push(s.t[0].note);
            octs.push(s.t[0].oct);
        }
        assert_eq!(notes, [0, 1, 2, 0, 1, 2, 0, 1], "a 3-step note loop against a 4-step trigger loop");
        assert_eq!(octs, [0, 0, 1, 1, 0, 0, 1, 1], "octave divided by 2");
    }

    #[test]
    fn notes_come_from_the_scale_and_wrap_into_the_next_octave() {
        let d = Data::default();
        let ionian = d.scales[0];
        assert_eq!(degrees(&ionian), [0, 2, 4, 5, 7, 9, 11]);
        assert_eq!(pitch(&ionian, 2, 0, 1), 16, "the third, an octave up");
        assert_eq!(pitch(&ionian, 6, 1, 0), 12, "7th + 1 degree = the next octave's root");
        assert_eq!(pitch(&d.scales[5], 2, 0, 0), 3, "Aeolian has a minor third");
    }

    #[test]
    fn duration_is_a_fraction_of_the_step_and_ratchets_split_it() {
        let mut s = seq();
        let t = &mut s.d.p[0].t[0];
        t.v[TR][0] = 1;
        t.v[DUR][0] = 5; // (5+1) x (4 x 4) / 384 = a quarter of the step
        s.reset();
        s.t[0].last_clock = Some(0);
        s.now = 6000;
        s.clock_all(true);
        let opened = s.now;
        assert!(s.t[0].gate);
        assert_eq!(s.t[0].gate_off - opened, 1500);
        // Three ratchets: three strikes inside the one step.
        let mut s = seq();
        let t = &mut s.d.p[0].t[0];
        t.v[TR][0] = 1;
        t.v[RPT][0] = 3;
        t.rpt_bits[0] = 0b111;
        s.reset();
        s.t[0].last_clock = Some(0);
        s.now = 6000;
        let before = s.t[0].strikes;
        s.clock_all(true);
        for _ in 0..60 {
            s.now += 100;
            s.timers();
        }
        assert_eq!(s.t[0].strikes - before, 3);
    }

    #[test]
    fn probability_zero_never_plays_and_half_plays_about_half() {
        let mut s = seq();
        let t = &mut s.d.p[0].t[0];
        t.v[TR] = [1; STEPS];
        t.llen[TR] = 1;
        t.prob[TR][0] = 0;
        s.reset();
        assert!(track0(&run(&mut s, 50)).is_empty());
        s.d.p[0].t[0].prob[TR][0] = 2;
        let n = track0(&run(&mut s, 400)).len();
        assert!((140..260).contains(&n), "about 50%: {n}/400");
    }

    #[test]
    fn directions() {
        let order = |dir: u8| {
            let mut s = seq();
            s.d.p[0].t[0].dir = dir;
            s.d.p[0].t[0].llen[TR] = 4;
            s.reset();
            (0..8).map(|_| {
                s.clock_all(true);
                s.t[0].pos[TR]
            }).collect::<Vec<_>>()
        };
        assert_eq!(order(0), [0, 1, 2, 3, 0, 1, 2, 3]);
        assert_eq!(order(1), [2, 1, 0, 3, 2, 1, 0, 3]);
        // From the reset position (the loop end) a triangle heads back first.
        assert_eq!(order(2), [2, 1, 0, 1, 2, 3, 2, 1], "triangle bounces");
        assert!(order(4).iter().all(|&p| p < 4), "random stays in the loop");
    }

    #[test]
    fn a_cued_pattern_waits_for_the_cue_point() {
        let mut s = seq();
        s.cue_next = Some(5);
        s.clock_all(true);
        s.clock_all(true);
        s.clock_all(true);
        assert_eq!(s.pattern, 0);
        s.clock_all(true);
        assert_eq!(s.pattern, 5, "every 4 clocks by default");
    }

    // ---- the app

    fn dir() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("portamax-kria-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn app() -> (KriaApp, Arc<Clock>, Arc<Link>, Arc<ModBus>) {
        let clock = Arc::new(Clock::new());
        clock.set_bpm(120.0);
        let link = Arc::new(Link::new());
        let mods = Arc::new(ModBus::new());
        let a = KriaApp::with(&dir(), Arc::clone(&link), Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::clone(&mods), Arc::new(AudioBus::new()), Arc::new(MixerBus::new())).following(Arc::clone(&clock));
        (a, clock, link, mods)
    }

    fn play(p: &mut Box<dyn AudioProcessor>, clock: &Clock, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for _ in 0..blocks {
            let mut buf = vec![0.0f32; 480 * 2];
            p.process(&mut buf, 2, 48_000.0);
            clock.end_block(480, 48_000.0);
            assert!(buf.iter().all(|v| v.is_finite()));
            all.extend(buf.iter().step_by(2));
        }
        all
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn tap(a: &mut KriaApp, pad: usize) {
        a.tick(&Input { grid: std::array::from_fn(|i| i == pad), ..Default::default() });
        a.tick(&Input::default());
    }

    #[test]
    fn silent_until_started_then_the_demo_plays_on_the_transport() {
        let (mut a, clock, _, _) = app();
        let mut p = a.audio_processor().unwrap();
        assert!(rms(&play(&mut p, &clock, 50)) < 1e-6);
        a.toggle_running();
        let out = play(&mut p, &clock, 200);
        assert!(clock.running());
        assert!(rms(&out) > 0.01, "{}", rms(&out));
        let v = a.view();
        assert!(v.pos[0][TR] < 8, "the demo's 8-step trigger loop");
    }

    #[test]
    fn pads_edit_the_selected_track_and_loop_sync_spreads_a_loop() {
        let (mut a, _, _, _) = app();
        a.kit.set_native(P_TRIG);
        a.tick(&Input { nav_x: 3, ..Default::default() });
        assert_eq!(a.track, 3);
        tap(&mut a, 5);
        assert_eq!(a.data().p[0].t[3].v[TR][5], 1);
        tap(&mut a, 5);
        assert_eq!(a.data().p[0].t[3].v[TR][5], 0);
        // Note page: select step 2 and raise its note; note sync turns the trigger on.
        a.kit.set_native(P_NOTE);
        tap(&mut a, 2);
        a.tick(&Input { navigation_steps: -3, ..Default::default() });
        assert_eq!(a.data().p[0].t[3].v[NOTE][2], 3);
        assert_eq!(a.data().p[0].t[3].v[TR][2], 1);
        // Loop page (on NOTE): steps 3-10; Loop Sync All sets every loop.
        a.kit.set_native(P_LOOP);
        tap(&mut a, 2);
        tap(&mut a, 9);
        let d = a.data();
        for t in 0..TRACKS {
            assert_eq!((d.p[0].t[t].lstart[TR], d.p[0].t[t].llen[GLIDE]), (2, 8));
        }
        // TIME: pad 4 divides the note parameter by 4.
        a.kit.set_native(P_TIME);
        tap(&mut a, 3);
        assert_eq!(a.data().p[0].t[3].tmul[NOTE], 4);
    }

    #[test]
    fn teletype_drives_kria_through_the_link() {
        let (mut a, clock, link, _) = app();
        a.p.clock_src.store(C_TELETYPE, Ordering::Relaxed);
        a.p.running.store(true, Ordering::Relaxed);
        let mut p = a.audio_processor().unwrap();
        play(&mut p, &clock, 2);
        // KR.CLK 0 four times: every track moves on four steps from the top.
        for _ in 0..4 {
            link.set(KrOp::Clk, &[0], 0);
        }
        play(&mut p, &clock, 1);
        assert_eq!(link.get(KrOp::Pos, &[1, 1]), 3, "KR.POS 1 1");
        assert!(a.view().gate.iter().any(|g| *g) || link.get(KrOp::Dur, &[1]) > 0);
        // KR.L.LEN 2 2 3: track 2's note loop is 3 long (an editor command).
        link.set(KrOp::LLen, &[2, 2], 3);
        a.background_tick();
        assert_eq!(a.data().p[0].t[1].llen[NOTE], 3);
        assert_eq!(link.get(KrOp::LLen, &[2, 2]), 3);
        // KR.PAT 4 switches at once; KR.MUTE 1 1 mutes track 1.
        link.set(KrOp::Pat, &[], 4);
        link.set(KrOp::Mute, &[1], 1);
        play(&mut p, &clock, 1);
        assert_eq!(link.get(KrOp::Pat, &[]), 4);
        assert_eq!(a.view().pattern, 4);
        assert_eq!(link.get(KrOp::Mute, &[1]), 1);
        // KR.CV reads the track's pitch in Teletype's units.
        assert!(link.get(KrOp::Cv, &[2]) >= 0);
    }

    #[test]
    fn presets_save_and_load() {
        let (mut a, _, _, _) = app();
        a.edit(|d, _| d.p[0].t[0].v[TR] = [1; STEPS]);
        a.control(C_SAVEPRE, 1);
        assert!(a.preset_path(0).exists(), "{}", a.status);
        a.edit(|d, _| d.p[0].t[0].v[TR] = [0; STEPS]);
        a.load_preset(0);
        assert_eq!(a.data().p[0].t[0].v[TR], [1; STEPS]);
    }

    /// Down and up on one grid key, a frame each.
    fn key(a: &mut KriaApp, x: usize, y: usize) {
        a.grid.press(x, y, true);
        a.background_tick();
        a.grid.press(x, y, false);
        a.background_tick();
    }

    fn hold(a: &mut KriaApp, x: usize, y: usize, down: bool) {
        a.grid.press(x, y, down);
        a.background_tick();
    }

    #[test]
    fn kria_plays_on_the_grid_with_ansibles_layout() {
        let (mut a, _, link, _) = app();
        let g = grid_kit::grid();
        assert_eq!(g.focus().as_deref(), Some(APP_NAME), "the first grid app gets the grid");
        a.background_tick();
        let lit = |x: usize, y: usize| g.snapshot().leds[y * 16 + x];
        assert_eq!(lit(5, 7), L2 as u8, "the trigger page key is the bright one");

        // Trigger page: rows 1-4 are the four tracks.
        let before = a.data().p[0].t[2].v[TR][6];
        key(&mut a, 6, 2);
        assert_eq!(a.data().p[0].t[2].v[TR][6], before ^ 1);

        // Bottom row: track 2, then the note page; a note at row 2 is degree 5.
        key(&mut a, 1, 7);
        assert_eq!(a.track, 1);
        key(&mut a, 6, 7);
        assert_eq!(a.grid_page(), P_NOTE);
        assert_eq!(a.kit_page(), Some(P_NOTE), "the screen follows the grid's page");
        key(&mut a, 4, 1);
        assert_eq!((a.data().p[0].t[1].v[NOTE][4], a.data().p[0].t[1].v[TR][4]), (5, 1), "note sync turns the step on");
        assert!(lit(4, 1) >= 3);
        key(&mut a, 6, 7);
        assert_eq!(a.grid_page(), P_ALT, "pressing the note key again is alt note");
        key(&mut a, 6, 7);

        // Held LOOP: first step 3, last step 10.
        hold(&mut a, 10, 7, true);
        hold(&mut a, 2, 3, true);
        hold(&mut a, 9, 3, true);
        hold(&mut a, 9, 3, false);
        hold(&mut a, 2, 3, false);
        let tr = a.data().p[0].t[1];
        assert_eq!((tr.lstart[NOTE], tr.llen[NOTE]), (2, 8));
        // ...and with LOOP held, a track key mutes.
        key(&mut a, 0, 7);
        assert!(link.mute[0].load(Ordering::Relaxed));
        hold(&mut a, 10, 7, false);

        // Held TIME: the column is the division. Held PROB: row 5 is 25 %.
        hold(&mut a, 11, 7, true);
        key(&mut a, 3, 0);
        hold(&mut a, 11, 7, false);
        assert_eq!(a.data().p[0].t[1].tmul[NOTE], 4);
        hold(&mut a, 12, 7, true);
        assert_eq!(lit(0, 5), 3, "the 0 % row shows while PROB is held");
        key(&mut a, 5, 4);
        hold(&mut a, 12, 7, false);
        assert_eq!(a.data().p[0].t[1].prob[NOTE][5], 1);

        // Duration: the top row is the multiplier, the rest the length.
        key(&mut a, 8, 7);
        key(&mut a, 7, 0);
        key(&mut a, 2, 4);
        assert_eq!((a.data().p[0].t[1].dur_mul, a.data().p[0].t[1].v[DUR][2]), (8, 3));

        // Ratchet: toggling sub-trigger rows sets the count to the highest.
        key(&mut a, 5, 7);
        key(&mut a, 5, 7);
        assert_eq!(a.grid_page(), P_RPT);
        key(&mut a, 0, 3);
        assert_eq!((a.data().p[0].t[1].rpt_bits[0], a.data().p[0].t[1].v[RPT][0]), (0b101, 3));

        // Scale page: direction per track, the scale, and its root.
        key(&mut a, 14, 7);
        key(&mut a, 4, 1);
        key(&mut a, 2, 6);
        key(&mut a, 10, 6);
        let d = a.data();
        assert_eq!((d.p[0].t[1].dir, d.p[0].scale, d.scales[10][0]), (1, 10, 2));

        // Pattern page: row 2 is the cue length.
        key(&mut a, 15, 7);
        key(&mut a, 2, 1);
        assert_eq!(a.data().cue_steps, 2);

        // Another app takes the grid: Kria's keys stop.
        // (Screenshots: see `grid_screenshots`.)
        g.register("Other");
        g.set_focus("Other");
        key(&mut a, 5, 1);
        assert_eq!(a.data().cue_steps, 2);
    }

    /// Writes the Grid app's screen, with Kria playing on it, to the
    /// folder in PORTAMAX_GRID_SHOT.
    #[test]
    #[ignore = "writes screenshots to the folder in PORTAMAX_GRID_SHOT"]
    fn grid_screenshots() {
        let Ok(dir) = std::env::var("PORTAMAX_GRID_SHOT") else { return };
        let (mut a, clock, _, _) = app();
        let mut screen = crate::apps::grid::GridApp::new(grid_kit::grid(), Arc::new(AtomicF32::new(3.0)));
        let mut p = a.audio_processor().unwrap();
        a.toggle_running();
        play(&mut p, &clock, 37);
        let shoot = |screen: &mut crate::apps::grid::GridApp, name: &str| {
            let mut fb = FrameBuffer::new();
            screen.draw(&mut fb);
            let mut out = b"P6\n640 360\n255\n".to_vec();
            out.extend(fb.buffer().iter().flat_map(|px| [(px >> 16) as u8, (px >> 8) as u8, *px as u8]));
            std::fs::write(Path::new(&dir).join(name), out).unwrap();
        };
        a.background_tick();
        shoot(&mut screen, "grid_trig.ppm");
        key(&mut a, 1, 7);
        key(&mut a, 6, 7);
        play(&mut p, &clock, 11);
        a.background_tick();
        shoot(&mut screen, "grid_note.ppm");
        grid_kit::grid().set_size(64, 128);
        a.background_tick();
        shoot(&mut screen, "grid_64x128.ppm");
    }
}
