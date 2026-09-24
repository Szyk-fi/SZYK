//! A 6-operator FM synth in the spirit of Yamaha's DX7 -- not a clone
//! of it (no ROM/firmware access, and no interest in reverse-
//! engineering one; this is standard, publicly-known FM synthesis
//! built from first principles, same "in the spirit of" approach as
//! Bloom/Madness/Prism used for their own references). Honest gaps
//! versus the real thing, stated up front rather than glossed over:
//!
//! - **40 algorithms** -- eight original layouts plus all 32 Yamaha
//!   topologies. Imported patches retain their original routing.
//! - Imported voices retain six rate/level envelopes. Their timing and
//!   attenuation use continuous approximations, not exact Yamaha tables.
//!   The global ADSR is an additional overall amplitude envelope.
//! - **Continuous Ratio, not coarse+fine**-- one knob per operator
//!   (0.5-61.69x) instead of the DX7's separate coarse (integer/common
//!   fraction) and fine (percentage) controls.
//! - **One global Feedback amount**, applied to whichever operator
//!   the current algorithm designates as its feedback op -- same
//!   "exactly one feedback path per algorithm" rule the real DX7
//!   follows, just one knob instead of the DX7's separate per-voice
//!   feedback level living alongside the algorithm choice.
//!
//! Each operator's Level does double duty exactly like the real
//! thing: it's that operator's own output amplitude *and*, when it's
//! feeding another operator, that's the modulation index -- one
//! parameter, two roles, matching the DX7's own design rather than a
//! separate "mod depth" knob.
//!
//! Fully polyphonic across all 16 grid pads -- one voice per pad,
//! gated by whether it's currently held, exactly the same "16 fixed
//! voices, no stealing needed" convention Plaits' own Poly mode uses
//! -- chords are the whole point of a DX7-style instrument.
//!
//! Can also import real DX7 patches (32-voice bulk `.syx` banks or
//! single-voice dumps) from `dx7_presets/` -- see `scan_presets` and
//! the module-level `dx7_presets/README.md` for exactly what that
//! import can and can't carry over given the simplifications above.

use crate::app::{App, Input};
use crate::arpeggiator::{Arpeggiator, PATTERN_NAMES as ARP_PATTERN_NAMES};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::f32::consts::TAU;
use std::path::Path;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Where imported DX7 patches live: `dx7_presets/*.syx`, scanned once
/// at startup -- same "absolute path anchored at the crate root"
/// convention `SAMPLES_DIR` in sequencer.rs uses, independent of
/// whatever the current working directory happens to be.
const DX7_PRESETS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/dx7_presets");

const NUM_OPS: usize = 6;
const MAX_MODS_PER_OP: usize = 3;
/// How strongly a modulator's Level drives another operator's phase.
/// Was 7.0, tuned only against `ALGORITHMS[0]` ("Stack", one
/// modulator per operator) -- real DX7 patches (imported via
/// `dx7_presets/`) routinely land on algorithms that either sum
/// *multiple* modulators into one operator ("Two Modulate One") or
/// chain several stages deep (any of the stacks), and at 7.0 either
/// shape pushes the instantaneous phase deviation far past what a
/// plain, non-oversampled `sin()` can represent without aliasing --
/// audible as harsh broadband noise rather than a "rich" tone,
/// reported directly against real imported patches, most of which
/// use Operator 5 as a modulator somewhere. Lowered to a still-
/// clearly-FM-but-not-shattered level; see `mod_input`'s own
/// `soft_mod_ceiling` clamp below for the other half of the fix
/// (taming the *summed* case specifically).
const MOD_DEPTH_SCALE: f32 = 3.5;
/// Caps the *summed* modulation index (after `MOD_DEPTH_SCALE`, before
/// feedback) via a smooth `tanh` compression rather than a hard clip
/// -- a single modulator at full Level (index up to `MOD_DEPTH_SCALE`)
/// barely touches this ceiling and sounds unchanged, but an operator
/// fed by two or three modulators at once (their contributions summed
/// linearly first) gets pulled back down toward one modulator's worth
/// of deviation instead of stacking straight through to 2-3x the
/// index -- the specific case that made multi-modulator DX7 imports
/// noisiest.
const MOD_INPUT_CEILING: f32 = MOD_DEPTH_SCALE * 1.5;
const BASE_NOTE: i32 = 48; // roughly C3, same convention plaits.rs uses
const NOTE_MIN: i32 = -119;
const NOTE_MAX: i32 = 120;

/// One classic FM topology: which operators modulate which (indices
/// into `modulators[op]`, always higher-numbered feeding lower so a
/// single high-to-low processing pass gets causality right for free),
/// which operators are carriers (summed to the output), and which one
/// operator the algorithm's single feedback path applies to.
struct Algorithm {
    name: &'static str,
    modulators: [[i8; MAX_MODS_PER_OP]; NUM_OPS],
    carriers: [bool; NUM_OPS],
    feedback_op: usize,
    feedback_source: usize,
}
const NO_MOD: i8 = -1;

// Copyright 2021 Emilie Gillet.
//
// Author: Emilie Gillet (emilie.o.gillet@gmail.com)
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
// 
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
// 
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
// 
// See http://creativecommons.org/licenses/MIT/ for more information.
//

const ALGORITHMS: [Algorithm; 40] = [
    Algorithm {
        name: "Stack",
        // 6->5->4->3->2->1, only op1 (index 0) audible -- a deep,
        // classic bell/brass-style chain.
        modulators: [[1, NO_MOD, NO_MOD], [2, NO_MOD, NO_MOD], [3, NO_MOD, NO_MOD], [4, NO_MOD, NO_MOD], [5, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD]],
        carriers: [true, false, false, false, false, false],
        feedback_op: 5,
        feedback_source: 5,
    },
    Algorithm {
        name: "Twin Stack",
        // 2->1 carrier, and 4->3 carrier, ops 5/6 idle-but-audible --
        // two independent 2-op FM pairs plus a couple of plain tones.
        modulators: [[1, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD], [3, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD]],
        carriers: [true, false, true, false, true, true],
        feedback_op: 1,
        feedback_source: 1,
    },
    Algorithm {
        name: "Three Pairs",
        // Classic DX7 electric-piano shape: 3 independent 2-op FM
        // pairs, all carriers.
        modulators: [[1, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD], [3, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD], [5, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD]],
        carriers: [true, false, true, false, true, false],
        feedback_op: 1,
        feedback_source: 1,
    },
    Algorithm {
        name: "One To Many",
        // Op6 alone modulates every other operator, all of which are
        // carriers -- organ-like, common modulation across the board.
        modulators: [[5, NO_MOD, NO_MOD], [5, NO_MOD, NO_MOD], [5, NO_MOD, NO_MOD], [5, NO_MOD, NO_MOD], [5, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD]],
        carriers: [true, true, true, true, true, false],
        feedback_op: 5,
        feedback_source: 5,
    },
    Algorithm {
        name: "Additive",
        // No modulation at all -- 6 plain sine partials, purely
        // additive. A useful reference point/ far end of the spectrum.
        modulators: [[NO_MOD; MAX_MODS_PER_OP]; NUM_OPS],
        carriers: [true; NUM_OPS],
        feedback_op: 0,
        feedback_source: 0,
    },
    Algorithm {
        name: "Deep Pair + Quad",
        // 6->5->4->3 carrier (a 3-deep stack) alongside 3 independent
        // carriers (ops 1/2 idle-but-audible... wait, kept simple:
        // op3 carrier fed by a 3-op chain, ops 0/1 independent tones.
        modulators: [[NO_MOD, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD], [3, NO_MOD, NO_MOD], [4, NO_MOD, NO_MOD], [5, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD]],
        carriers: [true, true, true, false, false, false],
        feedback_op: 5,
        feedback_source: 5,
    },
    Algorithm {
        name: "Two Modulate One",
        // Ops 5 and 6 both feed op4 (a carrier); ops 1/2/3 independent
        // carriers -- a fuller, denser single voice plus support tones.
        modulators: [[NO_MOD, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD], [4, 5, NO_MOD], [NO_MOD, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD]],
        carriers: [true, true, true, true, false, false],
        feedback_op: 5,
        feedback_source: 5,
    },
    Algorithm {
        name: "Full Stack Feedback",
        // Same deep 6-chain as Stack, but with feedback on the
        // *carrier* itself (op1) instead of the top of the chain --
        // a harsher, buzzier character at the same Feedback setting.
        modulators: [[1, NO_MOD, NO_MOD], [2, NO_MOD, NO_MOD], [3, NO_MOD, NO_MOD], [4, NO_MOD, NO_MOD], [5, NO_MOD, NO_MOD], [NO_MOD, NO_MOD, NO_MOD]],
        carriers: [true, false, false, false, false, false],
        feedback_op: 0,
        feedback_source: 0,
    },
    // Routing translated from the MIT-licensed vendored MI FM algorithms table.
    Algorithm { name: "DX7 01", modulators: [[1, -1, -1], [-1, -1, -1], [3, -1, -1], [4, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, false, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 02", modulators: [[1, -1, -1], [-1, -1, -1], [3, -1, -1], [4, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, false, false], feedback_op: 1, feedback_source: 1 },
    Algorithm { name: "DX7 03", modulators: [[1, -1, -1], [2, -1, -1], [-1, -1, -1], [4, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, false, true, false, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 04", modulators: [[1, -1, -1], [2, -1, -1], [-1, -1, -1], [4, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, false, true, false, false], feedback_op: 5, feedback_source: 3 },
    Algorithm { name: "DX7 05", modulators: [[1, -1, -1], [-1, -1, -1], [3, -1, -1], [-1, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, true, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 06", modulators: [[1, -1, -1], [-1, -1, -1], [3, -1, -1], [-1, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, true, false], feedback_op: 5, feedback_source: 4 },
    Algorithm { name: "DX7 07", modulators: [[1, -1, -1], [-1, -1, -1], [4, 3, -1], [-1, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, false, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 08", modulators: [[1, -1, -1], [-1, -1, -1], [4, 3, -1], [-1, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, false, false], feedback_op: 3, feedback_source: 3 },
    Algorithm { name: "DX7 09", modulators: [[1, -1, -1], [-1, -1, -1], [4, 3, -1], [-1, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, false, false], feedback_op: 1, feedback_source: 1 },
    Algorithm { name: "DX7 10", modulators: [[1, -1, -1], [2, -1, -1], [-1, -1, -1], [5, 4, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, false, false, true, false, false], feedback_op: 2, feedback_source: 2 },
    Algorithm { name: "DX7 11", modulators: [[1, -1, -1], [2, -1, -1], [-1, -1, -1], [5, 4, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, false, false, true, false, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 12", modulators: [[1, -1, -1], [-1, -1, -1], [5, 4, 3], [-1, -1, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, false, false], feedback_op: 1, feedback_source: 1 },
    Algorithm { name: "DX7 13", modulators: [[1, -1, -1], [-1, -1, -1], [5, 4, 3], [-1, -1, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, false, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 14", modulators: [[1, -1, -1], [-1, -1, -1], [3, -1, -1], [5, 4, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, false, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 15", modulators: [[1, -1, -1], [-1, -1, -1], [3, -1, -1], [5, 4, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, false, false], feedback_op: 1, feedback_source: 1 },
    Algorithm { name: "DX7 16", modulators: [[4, 2, 1], [-1, -1, -1], [3, -1, -1], [-1, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, false, false, false, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 17", modulators: [[4, 2, 1], [-1, -1, -1], [3, -1, -1], [-1, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, false, false, false, false], feedback_op: 1, feedback_source: 1 },
    Algorithm { name: "DX7 18", modulators: [[3, 2, 1], [-1, -1, -1], [-1, -1, -1], [4, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, false, false, false, false], feedback_op: 2, feedback_source: 2 },
    Algorithm { name: "DX7 19", modulators: [[1, -1, -1], [2, -1, -1], [-1, -1, -1], [5, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, false, true, true, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 20", modulators: [[2, -1, -1], [2, -1, -1], [-1, -1, -1], [5, 4, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, true, false, true, false, false], feedback_op: 2, feedback_source: 2 },
    Algorithm { name: "DX7 21", modulators: [[2, -1, -1], [2, -1, -1], [-1, -1, -1], [5, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, true, false, true, true, false], feedback_op: 2, feedback_source: 2 },
    Algorithm { name: "DX7 22", modulators: [[1, -1, -1], [-1, -1, -1], [5, -1, -1], [5, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, false, true, true, true, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 23", modulators: [[-1, -1, -1], [2, -1, -1], [-1, -1, -1], [5, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, true, false, true, true, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 24", modulators: [[-1, -1, -1], [-1, -1, -1], [5, -1, -1], [5, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, true, true, true, true, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 25", modulators: [[-1, -1, -1], [-1, -1, -1], [-1, -1, -1], [5, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, true, true, true, true, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 26", modulators: [[-1, -1, -1], [2, -1, -1], [-1, -1, -1], [5, 4, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, true, false, true, false, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 27", modulators: [[-1, -1, -1], [2, -1, -1], [-1, -1, -1], [5, 4, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, true, false, true, false, false], feedback_op: 2, feedback_source: 2 },
    Algorithm { name: "DX7 28", modulators: [[1, -1, -1], [-1, -1, -1], [3, -1, -1], [4, -1, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, false, true, false, false, true], feedback_op: 4, feedback_source: 4 },
    Algorithm { name: "DX7 29", modulators: [[-1, -1, -1], [-1, -1, -1], [3, -1, -1], [-1, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, true, true, false, true, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 30", modulators: [[-1, -1, -1], [-1, -1, -1], [3, -1, -1], [4, -1, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, true, true, false, false, true], feedback_op: 4, feedback_source: 4 },
    Algorithm { name: "DX7 31", modulators: [[-1, -1, -1], [-1, -1, -1], [-1, -1, -1], [-1, -1, -1], [5, -1, -1], [-1, -1, -1]], carriers: [true, true, true, true, true, false], feedback_op: 5, feedback_source: 5 },
    Algorithm { name: "DX7 32", modulators: [[-1, -1, -1], [-1, -1, -1], [-1, -1, -1], [-1, -1, -1], [-1, -1, -1], [-1, -1, -1]], carriers: [true, true, true, true, true, true], feedback_op: 5, feedback_source: 5 },
];

const MIN_RATIO: f32 = 0.5;
const MAX_RATIO: f32 = 61.69;
const DEFAULT_DECAY: f32 = 0.3;
const DEFAULT_SUSTAIN: f32 = 0.7;
const DEFAULT_RELEASE: f32 = 0.4;
const MAX_ENV_SECONDS: f32 = 4.0;
const MIN_LFO_RATE: f32 = 0.1;
const MAX_LFO_RATE: f32 = 12.0;

/// Shortens `s` to at most `max` characters (plus a "..." marker when
/// it actually had to cut something) -- needed for bank names, which
/// come straight from real folder names on disk and, unlike every
/// other string this app displays, have no length this codebase
/// controls. Character-counted, not byte-counted, so it can't panic
/// slicing into the middle of a multi-byte UTF-8 character.
fn truncate_display(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(max).collect::<String>())
    }
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * (max - min) * 0.05).clamp(min, max);
    value.set(next);
}

/// Maps a 0..1 knob value to a musically useful time range.
fn adsr_time(knob_value: f32, max_seconds: f32) -> f32 {
    0.001 + knob_value * max_seconds
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    /// Browses (turn) which folder/bank `Selection::Preset` browses
    /// within -- see `CascadeApp::banks`/`bank_browse`.
    Bank,
    /// Browses (turn) and loads (press) an imported DX7 patch from the
    /// current bank -- see `CascadeApp::presets`/`preset_browse`.
    Preset,
    Algorithm,
    Octave,
    Feedback,
    Attack,
    Decay,
    Sustain,
    Release,
    LfoRate,
    LfoDepth,
    OpRatio(usize),
    OpLevel(usize),
    ArpOn,
    ArpPattern,
    ArpRate,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 3 + NUM_OPS; // Presets, Global, one per operator, then Arp
const ARP_GROUP: usize = 2 + NUM_OPS;

struct OperatorParams {
    ratio: AtomicF32,
    level: AtomicF32,
}

impl OperatorParams {
    fn new(default_ratio: f32, default_level: f32) -> Self {
        Self { ratio: AtomicF32::new(default_ratio), level: AtomicF32::new(default_level) }
    }
}

struct Params {
    imported_envelopes: Mutex<Option<[[[u8; 4]; 2]; NUM_OPS]>>,
    algorithm: AtomicU32,
    /// Transposes every pad's note by this many octaves -- same
    /// pattern/range as Plaits' own Octave, letting the 16-pad range
    /// reach further up or down than its fixed base register alone.
    octave: AtomicI32,
    feedback: AtomicF32,
    ext_feedback: Arc<AtomicF32>,
    attack: AtomicF32,
    decay: AtomicF32,
    sustain: AtomicF32,
    release: AtomicF32,
    lfo_rate: AtomicF32,
    lfo_depth: AtomicF32,
    ops: [OperatorParams; NUM_OPS],
    held: Mutex<[bool; 16]>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    /// See arpeggiator.rs -- stepped once per audio block in `process`.
    arp: Arpeggiator,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Cascade", modbus);
        Self {
            imported_envelopes: Mutex::new(None),
            algorithm: AtomicU32::new(0),
            octave: AtomicI32::new(0),
            feedback: AtomicF32::new(0.2),
            ext_feedback: modbus.register("Cascade: Feedback".to_string()),
            attack: AtomicF32::new(0.0),
            decay: AtomicF32::new((DEFAULT_DECAY / MAX_ENV_SECONDS).clamp(0.0, 1.0)),
            sustain: AtomicF32::new(DEFAULT_SUSTAIN),
            release: AtomicF32::new((DEFAULT_RELEASE / MAX_ENV_SECONDS).clamp(0.0, 1.0)),
            lfo_rate: AtomicF32::new(5.0),
            lfo_depth: AtomicF32::new(0.0),
            ops: std::array::from_fn(|i| OperatorParams::new(1.0, if i == 0 { 0.9 } else { 0.5 })),
            held: Mutex::new([false; 16]),
            bus_out: audio_bus.register("Cascade"),
            mix_level,
            ext_mix_level,
            arp: Arpeggiator::new(),
        }
    }
}

/// Row-major pad layout, row 0 on top -- same convention Plaits'
/// grid uses. `rank` walks the 16 pads bottom-row-first/left-to-right
/// so pad 0 is the lowest note, matching a real keyboard's left-to-
/// right pitch order despite the physical grid's row-major storage.
fn pad_rank(physical_index: i32) -> i32 {
    let row = physical_index / 4;
    let col = physical_index % 4;
    (3 - row) * 4 + col
}

fn note_for(rank: i32, octave: i32) -> i32 {
    (BASE_NOTE + rank + octave * 12).clamp(NOTE_MIN, NOTE_MAX)
}

pub struct CascadeApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    /// Every patch found under `dx7_presets/` at startup -- see
    /// `scan_presets`. Empty (not an error) if the folder doesn't
    /// exist yet or has nothing in it.
    presets: Vec<Dx7Preset>,
    /// Every folder (plus a synthetic "(root)" for loose top-level
    /// files) found under `dx7_presets/` at startup, each pointing at
    /// its presets' positions in `presets` -- see `scan_presets`.
    banks: Vec<Dx7Bank>,
    /// Which bank `Selection::Bank`/`Selection::Preset` are currently
    /// browsing within.
    bank_browse: usize,
    /// Which preset within the current bank `Selection::Preset` is
    /// currently browsing to -- separate from "loaded," since turning
    /// the knob just previews a name until you press to actually
    /// apply it (same "press to act" idiom as every other load/action
    /// leaf in this build).
    preset_browse: usize,
}

// --- Cascade's own palette: icy glass-blue on deep navy, not a
// device-wide theme -- crystalline FM bells and electric-piano tones,
// the "glassy" character classic 6-op FM synthesis is known for. ---

const CASCADE_BG: Rgb565 = Rgb565::new(1, 4, 3);
const CASCADE_TITLE: Rgb565 = Rgb565::new(28, 60, 31);
const CASCADE_ACCENT: Rgb565 = Rgb565::new(15, 54, 31);
const CASCADE_DIM: Rgb565 = Rgb565::new(11, 26, 15);
const CASCADE_MODULATOR: Rgb565 = Rgb565::new(7, 18, 20);
const CASCADE_LINE: Rgb565 = Rgb565::new(5, 12, 15);

impl CascadeApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let (presets, banks) = scan_presets(Path::new(DX7_PRESETS_DIR));
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
            presets,
            banks,
            bank_browse: 0,
            preset_browse: 0,
        }
    }

    /// The flat `presets` index the current bank/preset browse
    /// position resolves to, or None if no banks were found at all.
    fn selected_preset_index(&self) -> Option<usize> {
        resolve_preset(&self.banks, self.bank_browse, self.preset_browse)
    }

    /// Copies preset `i`'s mapped values into every live knob --
    /// same "load = overwrite the current knobs" convention Prism's
    /// User Presets use.
    fn load_preset(&mut self, i: usize) {
        let Some(preset) = self.presets.get(i).cloned() else { return };
        *self.params.imported_envelopes.lock().unwrap() = preset.op_envelopes;
        self.params.algorithm.store(preset.algorithm, Ordering::Relaxed);
        self.params.feedback.set(preset.feedback);
        self.params.attack.set(preset.attack);
        self.params.decay.set(preset.decay);
        self.params.sustain.set(preset.sustain);
        self.params.release.set(preset.release);
        for op in 0..NUM_OPS {
            self.params.ops[op].ratio.set(preset.op_ratio[op]);
            self.params.ops[op].level.set(preset.op_level[op]);
        }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        if g == 0 {
            vec![Selection::Bank, Selection::Preset]
        } else if g == 1 {
            vec![Selection::Algorithm, Selection::Octave, Selection::Feedback, Selection::Attack, Selection::Decay, Selection::Sustain, Selection::Release, Selection::LfoRate, Selection::LfoDepth]
        } else if g == ARP_GROUP {
            vec![Selection::ArpOn, Selection::ArpPattern, Selection::ArpRate]
        } else {
            let op = g - 2;
            vec![Selection::OpRatio(op), Selection::OpLevel(op)]
        }
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for g in 0..NUM_GROUPS {
            rows.push(Row::Group(g));
            if self.expanded[g] {
                for sel in self.group_leaves(g) {
                    rows.push(Row::Leaf(sel));
                }
            }
        }
        rows
    }

    fn group_name(&self, g: usize) -> String {
        match g {
            0 => "Presets".into(),
            1 => "Global".into(),
            _ if g == ARP_GROUP => "Arp".into(),
            _ => format!("Operator {}", g - 1),
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            // Deliberately compact ("32b / 83706p", not "32 banks, 83706
            // presets") -- a real patch library can be tens of
            // thousands of presets, and this row's text is competing
            // with `node_x`'s operator graph for the same screen width
            // once `paramlist::TEXT_SCALE` is applied (see
            // `list_text_never_reaches_the_operator_graph`).
            0 => format!("{}b / {}p", self.banks.len(), self.presets.len()),
            1 => {
                let idx = self.params.algorithm.load(Ordering::Relaxed) as usize % ALGORITHMS.len();
                ALGORITHMS[idx].name.to_string()
            }
            _ if g == ARP_GROUP => self.leaf_value(Selection::ArpOn),
            _ => {
                let op = g - 2;
                format!("x{:.2}, {:.0}%", self.params.ops[op].ratio.get(), self.params.ops[op].level.get() * 100.0)
            }
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Bank => "Bank".into(),
            Selection::Preset => "Preset".into(),
            Selection::Algorithm => "Algorithm".into(),
            Selection::Octave => "Octave".into(),
            Selection::Feedback => "Feedback".into(),
            Selection::Attack => "Attack".into(),
            Selection::Decay => "Decay".into(),
            Selection::Sustain => "Sustain".into(),
            Selection::Release => "Release".into(),
            Selection::LfoRate => "LFO Rate".into(),
            Selection::LfoDepth => "LFO Depth".into(),
            Selection::OpRatio(_) => "Ratio".into(),
            Selection::OpLevel(_) => "Level".into(),
            Selection::ArpOn => "On/Off".into(),
            Selection::ArpPattern => "Pattern".into(),
            Selection::ArpRate => "Rate".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            // Just the (possibly truncated) name here, no "(idx/count)"
            // or "press to load" -- see the hint bar for that instead.
            // A bank name comes straight from a real folder on disk,
            // and even a preset name plus its browse position was
            // already enough to push this row's width right up
            // against `node_x`'s operator graph once
            // `paramlist::TEXT_SCALE` is applied (see
            // `list_text_never_reaches_the_operator_graph`).
            Selection::Bank => {
                if self.banks.is_empty() {
                    "none found".into()
                } else {
                    truncate_display(&self.banks[self.bank_browse].name, 14)
                }
            }
            Selection::Preset => {
                let Some(bank) = self.banks.get(self.bank_browse) else {
                    return "none found".into();
                };
                let Some(&idx) = bank.preset_indices.get(self.preset_browse) else {
                    return "empty bank".into();
                };
                self.presets[idx].name.clone()
            }
            Selection::Algorithm => {
                let idx = self.params.algorithm.load(Ordering::Relaxed) as usize % ALGORITHMS.len();
                ALGORITHMS[idx].name.to_string()
            }
            Selection::Octave => format!("{:+}", self.params.octave.load(Ordering::Relaxed)),
            Selection::Feedback => format!("{:.2}", self.params.feedback.get()),
            Selection::Attack => format!("{:.0} ms", adsr_time(self.params.attack.get(), MAX_ENV_SECONDS) * 1000.0),
            Selection::Decay => format!("{:.0} ms", adsr_time(self.params.decay.get(), MAX_ENV_SECONDS) * 1000.0),
            Selection::Sustain => format!("{:.2}", self.params.sustain.get()),
            Selection::Release => format!("{:.0} ms", adsr_time(self.params.release.get(), MAX_ENV_SECONDS) * 1000.0),
            Selection::LfoRate => format!("{:.1} Hz", self.params.lfo_rate.get()),
            Selection::LfoDepth => format!("{:.2}", self.params.lfo_depth.get()),
            Selection::OpRatio(op) => format!("{:.2}x", self.params.ops[op].ratio.get()),
            Selection::OpLevel(op) => format!("{:.0}%", self.params.ops[op].level.get() * 100.0),
            Selection::ArpOn => {
                if self.params.arp.enabled.load(Ordering::Relaxed) { "On".into() } else { "Off".into() }
            }
            Selection::ArpPattern => {
                let idx = self.params.arp.pattern.load(Ordering::Relaxed) as usize % ARP_PATTERN_NAMES.len();
                ARP_PATTERN_NAMES[idx].to_string()
            }
            Selection::ArpRate => format!("{:.1} Hz", self.params.arp.rate_hz.get()),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::Bank => {
                if !self.banks.is_empty() {
                    let cur = self.bank_browse as i32;
                    let next = (cur + step).rem_euclid(self.banks.len() as i32);
                    self.bank_browse = next as usize;
                    self.preset_browse = 0;
                }
            }
            Selection::Preset => {
                if let Some(bank) = self.banks.get(self.bank_browse) {
                    if !bank.preset_indices.is_empty() {
                        let cur = self.preset_browse as i32;
                        let next = (cur + step).rem_euclid(bank.preset_indices.len() as i32);
                        self.preset_browse = next as usize;
                    }
                }
            }
            Selection::Algorithm => {
                let cur = self.params.algorithm.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(ALGORITHMS.len() as i32);
                self.params.algorithm.store(next as u32, Ordering::Relaxed);
            }
            Selection::Octave => {
                let cur = self.params.octave.load(Ordering::Relaxed);
                self.params.octave.store(cur + step, Ordering::Relaxed);
            }
            Selection::Feedback => bump(&self.params.feedback, delta, sensitivity, 0.0, 1.0),
            Selection::Attack => bump(&self.params.attack, delta, sensitivity, 0.0, 1.0),
            Selection::Decay => bump(&self.params.decay, delta, sensitivity, 0.0, 1.0),
            Selection::Sustain => bump(&self.params.sustain, delta, sensitivity, 0.0, 1.0),
            Selection::Release => bump(&self.params.release, delta, sensitivity, 0.0, 1.0),
            Selection::LfoRate => bump(&self.params.lfo_rate, delta, sensitivity, MIN_LFO_RATE, MAX_LFO_RATE),
            Selection::LfoDepth => bump(&self.params.lfo_depth, delta, sensitivity, 0.0, 1.0),
            Selection::OpRatio(op) => bump(&self.params.ops[op].ratio, delta, sensitivity, MIN_RATIO, MAX_RATIO),
            Selection::OpLevel(op) => bump(&self.params.ops[op].level, delta, sensitivity, 0.0, 1.0),
            Selection::ArpOn => self.params.arp.enabled.store(delta > 0, Ordering::Relaxed),
            Selection::ArpPattern => {
                let cur = self.params.arp.pattern.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(ARP_PATTERN_NAMES.len() as i32);
                self.params.arp.pattern.store(next as u32, Ordering::Relaxed);
            }
            Selection::ArpRate => {
                let cur = self.params.arp.rate_hz.get();
                let next = (cur + accelerate(delta) * sensitivity * 0.2).clamp(0.5, 30.0);
                self.params.arp.rate_hz.set(next);
            }
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Bank => {} // no single sensible default among equal choices
            Selection::Preset => {
                if let Some(idx) = self.selected_preset_index() {
                    self.load_preset(idx);
                }
            }
            Selection::Octave => self.params.octave.store(0, Ordering::Relaxed),
            Selection::Feedback => self.params.feedback.set(0.2),
            Selection::Attack => self.params.attack.set(0.0),
            Selection::Decay => self.params.decay.set((DEFAULT_DECAY / MAX_ENV_SECONDS).clamp(0.0, 1.0)),
            Selection::Sustain => self.params.sustain.set(DEFAULT_SUSTAIN),
            Selection::Release => self.params.release.set((DEFAULT_RELEASE / MAX_ENV_SECONDS).clamp(0.0, 1.0)),
            Selection::LfoRate => self.params.lfo_rate.set(5.0),
            Selection::LfoDepth => self.params.lfo_depth.set(0.0),
            Selection::OpRatio(op) => self.params.ops[op].ratio.set(1.0),
            Selection::OpLevel(op) => self.params.ops[op].level.set(if op == 0 { 0.9 } else { 0.5 }),
            Selection::Algorithm => {} // no single sensible default among equal choices
            Selection::ArpOn => self.params.arp.enabled.store(false, Ordering::Relaxed),
            Selection::ArpPattern => self.params.arp.pattern.store(0, Ordering::Relaxed), // Up
            Selection::ArpRate => self.params.arp.rate_hz.set(8.0),
        }
    }
}

impl CascadeApp {
    /// Real, windowed `(name, value, is_group)` rows -- mirrors this
    /// app's own `draw()` row-building, exposed for an alternate
    /// renderer (a live Slint screen) instead of drawn.
    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g), true)
                }
                Row::Leaf(sel) => (self.leaf_name(*sel), self.leaf_value(*sel), false),
            })
            .collect()
    }

    pub(crate) fn selected_row(&self) -> usize {
        self.list.selected
    }

    /// `display_rows`, windowed to at most `visible` rows around the
    /// current selection -- see `ParamList::centered_scroll_window`. Returns
    /// `(window, selected_index_in_window, has_more_above,
    /// has_more_below)`.
    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }

    /// The current algorithm's real operator routing -- same data
    /// `draw()`'s own node-graph sketch reads (`ALGORITHMS[idx]`),
    /// exposed for an alternate renderer instead of drawn directly.
    pub(crate) fn operator_graph(&self) -> (String, [bool; NUM_OPS], Vec<(usize, usize)>, usize) {
        let idx = self.params.algorithm.load(Ordering::Relaxed) as usize % ALGORITHMS.len();
        let algo = &ALGORITHMS[idx];
        let mut connections = Vec::new();
        for op in 0..NUM_OPS {
            for &m in algo.modulators[op].iter() {
                if m >= 0 {
                    connections.push((m as usize, op));
                }
            }
        }
        (algo.name.to_string(), algo.carriers, connections, algo.feedback_op)
    }
}

impl App for CascadeApp {
    fn supports_pad_lock(&self) -> bool { true }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.selected_row()
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        self.windowed_rows(visible)
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let (algorithm_name, carriers, connections, feedback_op) = self.operator_graph();
        let connection_lines = crate::app::cascade_connection_lines(&connections);
        crate::app::SlintExtra::Cascade(crate::app::CascadeExtra {
            algorithm_name,
            carriers: carriers.to_vec(),
            connections,
            feedback_op,
            connection_lines,
        })
    }

    fn tick(&mut self, input: &Input) {
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
                self.reset(sel);
            }
        }

        // 16-pad chromatic keyboard, fully polyphonic -- every pad is
        // its own always-available voice (see `CascadeProcessor`),
        // so `tick` just needs to publish which ones are currently
        // held, same as Plaits' own Poly mode.
        {
            let mut held = self.params.held.lock().unwrap();
            for (i, pressed) in input.grid.iter().enumerate() {
                held[i] = *pressed;
            }
        }
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(CascadeProcessor {
            params: Arc::clone(&self.params),
            voices: std::array::from_fn(|_| FmVoice::new()),
            mono_buf: Vec::new(),
            chord_headroom: 1,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(CASCADE_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, CASCADE_TITLE);
        Text::new("Cascade", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, CASCADE_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, CASCADE_DIM);

        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", self.leaf_name(*sel)), self.leaf_value(*sel)),
            })
            .collect();
        self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, CASCADE_BG, CASCADE_DIM, CASCADE_ACCENT);

        // --- Right: the current algorithm's operator routing, drawn
        // as a small node graph -- carriers on the bottom row, higher
        // operators feeding down into whatever they modulate. ---
        let idx = self.params.algorithm.load(Ordering::Relaxed) as usize % ALGORITHMS.len();
        let algo = &ALGORITHMS[idx];
        Text::new(&format!("Algorithm: {}", algo.name), Point::new(360, 60), accent).draw(fb).ok();

        let node_x = 380;
        let node_y0 = 80;
        let row_h = 30;
        let positions: [Point; NUM_OPS] = std::array::from_fn(|op| Point::new(node_x + (op as i32 % 3) * 90, node_y0 + (op as i32 / 3) * row_h * 2));
        for op in 0..NUM_OPS {
            for &m in algo.modulators[op].iter() {
                if m >= 0 {
                    Line::new(positions[m as usize], positions[op]).into_styled(PrimitiveStyle::with_stroke(CASCADE_LINE, 1)).draw(fb).ok();
                }
            }
        }
        for op in 0..NUM_OPS {
            let color = if algo.carriers[op] { CASCADE_ACCENT } else { CASCADE_MODULATOR };
            let label = if op == algo.feedback_op { format!("Op{} FB", op + 1) } else { format!("Op{}", op + 1) };
            Text::new(&label, positions[op], MonoTextStyle::new(&SPLEEN_6X12, color)).draw(fb).ok();
        }

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(Selection::Bank)) => format!("knob2: browse banks ({}/{})", self.bank_browse + 1, self.banks.len().max(1)),
            Some(Row::Leaf(Selection::Preset)) => {
                let count = self.banks.get(self.bank_browse).map(|b| b.preset_indices.len()).unwrap_or(0).max(1);
                format!("knob2: browse presets ({}/{count})   press knob2: load", self.preset_browse + 1)
            }
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

/// One operator's persistent per-voice state.
#[derive(Clone, Copy)]
struct OperatorState {
    phase: f32,
    prev_output: f32,
}

impl OperatorState {
    fn new() -> Self {
        Self { phase: 0.0, prev_output: 0.0 }
    }
}

/// A standard block-rate ADSR -- same shape as plaits.rs's own
/// `AdsrState`, duplicated locally rather than shared since it's a
/// handful of lines and the two files don't otherwise depend on each
/// other.
#[derive(Default, Clone, Copy)]
struct AdsrState {
    stage: u8, // 0=idle 1=attack 2=decay 3=sustain 4=release
    level: f32,
    prev_gate: bool,
}

impl AdsrState {
    fn step(&mut self, gate: bool, attack: f32, decay: f32, sustain: f32, release: f32, dt: f32) -> f32 {
        if gate && !self.prev_gate {
            self.stage = 1;
        } else if !gate && self.prev_gate {
            self.stage = 4;
        }
        self.prev_gate = gate;
        match self.stage {
            1 => {
                self.level = (self.level + dt / attack.max(0.001)).min(1.0);
                if self.level >= 1.0 {
                    self.stage = 2;
                }
            }
            2 => {
                self.level = (self.level - dt * (1.0 - sustain) / decay.max(0.001)).max(sustain);
                if self.level <= sustain {
                    self.stage = 3;
                }
            }
            3 => self.level = sustain,
            4 => {
                self.level = (self.level - dt / release.max(0.001)).max(0.0);
                if self.level <= 0.0 {
                    self.stage = 0;
                }
            }
            _ => self.level = 0.0,
        }
        self.level
    }
}

// DX-style attenuation, rather than treating output level as linear gain.
// This is a continuous approximation; the hardware's discrete EG tables differ.
fn dx7_level(level:u8)->f32 { if level == 0 { 0.0 } else { 2.0f32.powf((level.min(99) as f32-99.0)*0.125) } }
#[derive(Clone, Copy, Default)]
struct ImportedEnvelope { stage:usize, level:f32, was_held:bool }
impl ImportedEnvelope {
    fn step(&mut self, gate:bool, data:[[u8;4];2], dt:f32)->f32 {
        if gate && !self.was_held { self.stage=0; }
        if !gate && self.was_held { self.stage=3; }
        self.was_held=gate;
        let target=dx7_level(data[1][self.stage]);
        let speed=dt/dx7_rate_to_seconds(data[0][self.stage]);
        let difference=target-self.level;
        self.level += difference.clamp(-speed,speed);
        if (self.level-target).abs()<1e-6 && self.stage<2 && gate { self.stage+=1; }
        self.level
    }
}

struct FmVoice {
    ops: [OperatorState; NUM_OPS],
    operator_env: [ImportedEnvelope; NUM_OPS],
    env: AdsrState,
    /// Independent per-voice LFO phase -- since every voice starts at
    /// the same phase and advances by the same rate every block,
    /// they stay in lockstep anyway, so this costs nothing versus a
    /// single shared phase while letting each voice render its whole
    /// block self-containedly (see `buf`).
    lfo_phase: f32,
    buf: Vec<f32>,
}

impl FmVoice {
    fn new() -> Self {
        Self { ops: std::array::from_fn(|_| OperatorState::new()), operator_env: [ImportedEnvelope::default(); NUM_OPS], env: AdsrState::default(), lfo_phase: 0.0, buf: Vec::new() }
    }
}

struct CascadeProcessor {
    params: Arc<Params>,
    voices: [FmVoice; 16],
    mono_buf: Vec<f32>,
    chord_headroom: usize,
}

impl AudioProcessor for CascadeProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        self.mono_buf.clear();
        self.mono_buf.resize(frames, 0.0);
        let dt = 1.0 / sample_rate;

        let algo_idx = self.params.algorithm.load(Ordering::Relaxed) as usize % ALGORITHMS.len();
        let algo = &ALGORITHMS[algo_idx];
        let octave = self.params.octave.load(Ordering::Relaxed);
        let feedback = (self.params.feedback.get() + self.params.ext_feedback.get()).clamp(0.0, 1.0);
        let attack = adsr_time(self.params.attack.get(), MAX_ENV_SECONDS);
        let decay = adsr_time(self.params.decay.get(), MAX_ENV_SECONDS);
        let sustain = self.params.sustain.get();
        let release = adsr_time(self.params.release.get(), MAX_ENV_SECONDS);
        let lfo_rate = self.params.lfo_rate.get();
        let lfo_depth = self.params.lfo_depth.get();
        let ratios: [f32; NUM_OPS] = std::array::from_fn(|op| self.params.ops[op].ratio.get().clamp(MIN_RATIO, MAX_RATIO));
        let levels: [f32; NUM_OPS] = std::array::from_fn(|op| self.params.ops[op].level.get().clamp(0.0, 1.0));
        let imported_envelopes = *self.params.imported_envelopes.lock().unwrap();
        let held_raw = *self.params.held.lock().unwrap();
        // Real arp step -- see Plaits' `process` for the fuller
        // explanation of why this runs here (sample-block-accurate,
        // real-time thread) rather than in `tick`. `held` is reordered
        // to pitch-rank first so Up/Down walk ascending/descending
        // pitch, not raw physical pad index.
        let held_by_rank: [bool; 16] = std::array::from_fn(|r| held_raw[pad_rank(r as i32) as usize]);
        let arp_rank = self.params.arp.step(&held_by_rank, frames as f32 / sample_rate);
        let held: [bool; 16] = match arp_rank {
            Some(r) => {
                let idx = pad_rank(r as i32) as usize;
                std::array::from_fn(|i| i == idx)
            }
            None => held_raw,
        };

        // Same active-voice headroom normalization Bloom's voice pool
        // needed: render every one of the 16 pad-voices' full block,
        // count how many actually produced audible signal, and
        // divide the sum by that count so a chord doesn't get louder
        // just for having more notes held.
        let mut active_voices: usize = 0;
        for i in 0..16 {
            let gate = held[i];
            let base_freq = 440.0 * 2f32.powf((note_for(pad_rank(i as i32), octave) as f32 - 69.0) / 12.0);
            let voice = &mut self.voices[i];
            voice.buf.clear();
            voice.buf.resize(frames, 0.0);

            if !gate && voice.env.stage == 0 { continue; }

            for n in 0..frames {
                voice.lfo_phase = (voice.lfo_phase + lfo_rate * dt).rem_euclid(1.0);
                let vibrato = (voice.lfo_phase * TAU).sin() * lfo_depth * 0.06; // up to ~+/-6% pitch at full depth
                let carrier_freq = base_freq * (1.0 + vibrato);

                let env = voice.env.step(gate, attack, decay, sustain, release, dt);

                let op_env: [f32; NUM_OPS] = std::array::from_fn(|op| match imported_envelopes {
                    Some(envelopes) => voice.operator_env[op].step(gate, envelopes[op], dt),
                    None => 1.0,
                });
                let mut op_out = [0.0f32; NUM_OPS];
                for op in (0..NUM_OPS).rev() {
                    let mut mod_input = 0.0f32;
                    for &m in algo.modulators[op].iter() {
                        if m >= 0 {
                            mod_input += op_out[m as usize] * levels[m as usize] * MOD_DEPTH_SCALE;
                        }
                    }
                    // Smooth ceiling on the *summed* index -- see
                    // `MOD_INPUT_CEILING`'s doc comment. A no-op for
                    // the common one-modulator case (`tanh` is still
                    // near-linear well under its own asymptote there);
                    // only actually compresses when multiple
                    // modulators stack into one operator.
                    mod_input = MOD_INPUT_CEILING * (mod_input / MOD_INPUT_CEILING).tanh();
                    let fb = if op == algo.feedback_op { voice.ops[algo.feedback_source].prev_output * feedback * 2.0 } else { 0.0 };
                    let freq = carrier_freq * ratios[op];
                    let phase = voice.ops[op].phase;
                    let out = (phase * TAU + mod_input + fb).sin();
                    voice.ops[op].phase = (phase + freq * dt).rem_euclid(1.0);
                    voice.ops[op].prev_output = out;
                    op_out[op] = out * op_env[op];
                }

                let mut sample = 0.0f32;
                let mut num_carriers = 0usize;
                for op in 0..NUM_OPS {
                    if algo.carriers[op] {
                        sample += op_out[op] * levels[op];
                        num_carriers += 1;
                    }
                }
                // Summing several carriers shouldn't itself scale
                // amplitude with carrier count either.
                sample /= num_carriers.max(1) as f32;
                voice.buf[n] = sample * env;
            }

            let peak = voice.buf.iter().fold(0.0f32, |a, s| a.max(s.abs()));
            if peak > 1e-4 {
                active_voices += 1;
            }
            for (m, s) in self.mono_buf.iter_mut().zip(voice.buf.iter()) {
                *m += *s;
            }
        }
        // Do not raise a sustained note's gain when another voice's release
        // crosses the silence threshold. Keep headroom for the whole phrase.
        if active_voices == 0 { self.chord_headroom = 1; }
        else { self.chord_headroom = self.chord_headroom.max(active_voices); }
        let headroom = self.chord_headroom as f32;
        for m in self.mono_buf.iter_mut() {
            *m /= headroom;
        }

        {
            let mut bus_out = self.params.bus_out.lock().unwrap();
            bus_out.clear();
            bus_out.extend_from_slice(&self.mono_buf);
        }

        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        for (frame, sample) in buffer.chunks_mut(channels).zip(self.mono_buf.iter()) {
            for out in frame.iter_mut() {
                *out = *sample * mix_level;
            }
        }
    }
}

/// One imported DX7 patch, already mapped down onto Cascade's own
/// simplified parameter set -- see the module doc comment and
/// `dx7_presets/README.md` for exactly what's approximated.
#[derive(Clone)]
struct Dx7Preset {
    name: String,
    algorithm: u32,
    feedback: f32,
    op_ratio: [f32; NUM_OPS],
    op_level: [f32; NUM_OPS],
    op_envelopes: Option<[[[u8;4];2];NUM_OPS]>,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

/// A single operator's raw, unmapped fields straight out of the
/// SysEx data -- kept separate from `Dx7Preset` since both the packed
/// and unpacked voice formats parse into this same shape before the
/// (format-independent) mapping step.
struct Dx7RawOp {
    eg_rate: [u8; 4],
    eg_level: [u8; 4],
    output_level: u8,
    freq_coarse: u8,
    freq_fine: u8,
}

/// DX7 rate (0=slowest, 99=fastest) to an approximate seconds value --
/// the real DX7's rate-to-time relationship is a non-linear table
/// that also depends on the level being moved *to*, which this
/// doesn't attempt to reproduce; this is just a monotonic, musically
/// reasonable stand-in (rate 99 -> ~1ms, rate 0 -> `MAX_ENV_SECONDS`).
fn dx7_rate_to_seconds(rate: u8) -> f32 {
    let t = (99 - rate.min(99)) as f32 / 99.0;
    0.001 + t * t * MAX_ENV_SECONDS
}

/// DX7 oscillator coarse+fine to a frequency ratio -- coarse 0 is
/// special-cased to 0.5x (per the real hardware), coarse 1-31 is
/// that integer multiple, and fine adds up to +99% on top.
fn dx7_ratio(coarse: u8, fine: u8) -> f32 {
    let base = if coarse == 0 { 0.5 } else { coarse as f32 };
    (base * (1.0 + fine as f32 * 0.01)).clamp(MIN_RATIO, MAX_RATIO)
}

/// Maps 6 raw operators (already reordered so index 0 is DX7's own
/// "Operator 1", the usual primary carrier -- see the two parse
/// functions below) plus algorithm/feedback into a `Dx7Preset`. The
/// global envelope comes from Operator 1; all six original envelopes are
/// retained separately for the operators, including modulator decay.
fn map_dx7_voice(name: String, algorithm: u8, feedback: u8, ops: &[Dx7RawOp; NUM_OPS]) -> Dx7Preset {
    let op1 = &ops[0];
    Dx7Preset {
        name,
        algorithm: 8 + (algorithm as u32 & 31),
        feedback: feedback as f32 / 7.0,
        op_ratio: std::array::from_fn(|i| dx7_ratio(ops[i].freq_coarse, ops[i].freq_fine)),
        op_level: std::array::from_fn(|i| dx7_level(ops[i].output_level)),
        op_envelopes: Some(std::array::from_fn(|i| [ops[i].eg_rate, ops[i].eg_level])),
        attack: dx7_rate_to_seconds(op1.eg_rate[0]) / MAX_ENV_SECONDS,
        decay: dx7_rate_to_seconds(op1.eg_rate[1]) / MAX_ENV_SECONDS,
        sustain: op1.eg_level[2] as f32 / 99.0,
        release: dx7_rate_to_seconds(op1.eg_rate[3]) / MAX_ENV_SECONDS,
    }
}

/// Unpacks one 128-byte packed voice block (32-voice bulk dump
/// format) into a `Dx7Preset`. Per-operator data is stored DX7-
/// Operator-6-first (bytes 0-16), ..., DX7-Operator-1-last (bytes
/// 85-101) -- reversed here so `ops[0]` is DX7's Operator 1, matching
/// `map_dx7_voice`'s expectation and this app's own convention that
/// index 0 is the primary carrier.
fn parse_packed_voice(b: &[u8]) -> Dx7Preset {
    let raw_ops: [Dx7RawOp; NUM_OPS] = std::array::from_fn(|dx7_op_6_first| {
        let base = dx7_op_6_first * 17;
        Dx7RawOp {
            eg_rate: [b[base], b[base + 1], b[base + 2], b[base + 3]],
            eg_level: [b[base + 4], b[base + 5], b[base + 6], b[base + 7]],
            output_level: b[base + 14] & 0x7f,
            freq_coarse: (b[base + 15] >> 1) & 0x1f,
            freq_fine: b[base + 16] & 0x7f,
        }
    });
    // Reverse so index 0 is DX7 Operator 1 (was raw_ops[5]).
    let ops: [Dx7RawOp; NUM_OPS] = std::array::from_fn(|i| {
        let src = &raw_ops[NUM_OPS - 1 - i];
        Dx7RawOp { eg_rate: src.eg_rate, eg_level: src.eg_level, output_level: src.output_level, freq_coarse: src.freq_coarse, freq_fine: src.freq_fine }
    });
    let algorithm = b[110] & 0x1f;
    // Yamaha packed format: feedback bits 0..2; oscillator sync is bit 3.
    let feedback = b[111] & 0x07;
    let name = decode_voice_name(&b[118..128]);
    map_dx7_voice(name, algorithm, feedback, &ops)
}

/// Unpacks one 155-byte unpacked voice (single-voice dump format).
/// Same DX7-Operator-6-first layout and reversal as the packed
/// format, just 21 bytes per operator instead of 17 since nothing is
/// bit-packed here -- every one of the 21 well-known DX7 operator
/// parameters gets its own byte, in the same order the packed
/// format's logical fields follow.
fn parse_unpacked_voice(b: &[u8]) -> Dx7Preset {
    let raw_ops: [Dx7RawOp; NUM_OPS] = std::array::from_fn(|dx7_op_6_first| {
        let base = dx7_op_6_first * 21;
        Dx7RawOp {
            eg_rate: [b[base], b[base + 1], b[base + 2], b[base + 3]],
            eg_level: [b[base + 4], b[base + 5], b[base + 6], b[base + 7]],
            output_level: b[base + 16] & 0x7f,
            freq_coarse: b[base + 18] & 0x1f,
            freq_fine: b[base + 19] & 0x7f,
        }
    });
    let ops: [Dx7RawOp; NUM_OPS] = std::array::from_fn(|i| {
        let src = &raw_ops[NUM_OPS - 1 - i];
        Dx7RawOp { eg_rate: src.eg_rate, eg_level: src.eg_level, output_level: src.output_level, freq_coarse: src.freq_coarse, freq_fine: src.freq_fine }
    });
    // 6 operators * 21 bytes = 126, so global params start at byte
    // 126: pitch EG rates (126-129), pitch EG levels (130-133), then
    // algorithm and feedback as their own full, unpacked bytes.
    let algorithm = b[134] & 0x1f;
    let feedback = b[135] & 0x07;
    let name = decode_voice_name(&b[145..155]);
    map_dx7_voice(name, algorithm, feedback, &ops)
}

/// The 10-byte ASCII voice name, trimmed of trailing spaces/nulls --
/// falls back to a placeholder if what's left is empty or the bytes
/// aren't printable ASCII (a corrupt/non-conforming file shouldn't
/// produce a blank, unselectable menu row).
fn decode_voice_name(bytes: &[u8]) -> String {
    let name: String = bytes.iter().map(|&b| (b & 0x7f) as char).collect();
    let trimmed = name.trim_end_matches(['\0', ' ']).trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Scans one `.syx` file's raw bytes for every SysEx message it
/// contains (from `F0` to the matching `F7`), parsing each as either
/// a 32-voice bulk dump or a single-voice dump per its header, and
/// skipping anything that doesn't match either shape (a non-DX7
/// SysEx file, a truncated one, or one addressed to a different
/// format) rather than panicking on it.
fn parse_syx_file(data: &[u8]) -> Vec<Dx7Preset> {
    let mut presets = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if data[i] != 0xF0 {
            i += 1;
            continue;
        }
        let Some(end) = data[i..].iter().position(|&b| b == 0xF7) else { break };
        let message = &data[i..i + end + 1]; // inclusive of F0..F7
        i += end + 1;

        // Header: F0 43 <sub-status/channel> <format> <count-msb> <count-lsb> <data...> <checksum> F7
        if message.len() < 6 || message[1] != 0x43 {
            continue;
        }
        let format = message[3];
        let byte_count = ((message[4] as usize) << 7) | (message[5] as usize);
        let body = &message[6..];
        if body.len() < byte_count + 1 {
            continue; // truncated -- not enough data for the declared count plus a checksum
        }
        match format {
            9 if byte_count == 4096 => {
                for voice in 0..32 {
                    presets.push(parse_packed_voice(&body[voice * 128..voice * 128 + 128]));
                }
            }
            0 if byte_count == 155 => {
                presets.push(parse_unpacked_voice(&body[..155]));
            }
            _ => {} // an unrecognized format/size -- skip rather than misparse it
        }
    }
    presets
}

fn is_syx(path: &Path) -> bool {
    path.is_file() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("syx"))
}

fn sorted_dir_entries(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out: Vec<_> = std::fs::read_dir(dir).map(|e| e.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    out.sort();
    out
}

/// One folder's worth of presets -- `name` is that folder's name (or
/// "(root)" for `.syx` files sitting loose directly under
/// `dx7_presets/`, same convention `sequencer.rs`'s sample packs use
/// for loose `.wav` files), `preset_indices` are positions into the
/// flat `presets` list `scan_presets` returns alongside this.
struct Dx7Bank {
    name: String,
    preset_indices: Vec<usize>,
}

/// Scans `dx7_presets/` one level deep: every `.syx` file directly
/// under it becomes an "(root)" bank, and every subfolder becomes its
/// own named bank grouping whatever `.syx` files are directly inside
/// it (real DX7 patch libraries are typically distributed as one
/// folder per bank/collection, each holding one or more 32-voice bank
/// dumps) -- same one-level-deep, "(root)" -for-loose-files shape
/// `sequencer.rs`'s sample-pack scanner uses, just without a second
/// "type" level underneath, since a `.syx` file doesn't subdivide the
/// way a sample-type folder does.
fn scan_presets(dir: &Path) -> (Vec<Dx7Preset>, Vec<Dx7Bank>) {
    let mut presets = Vec::new();
    let mut banks = Vec::new();

    let mut load_syx_into = |paths: Vec<std::path::PathBuf>| -> Vec<usize> {
        let mut indices = Vec::new();
        for path in paths {
            if !is_syx(&path) {
                continue;
            }
            if let Ok(data) = std::fs::read(&path) {
                for preset in parse_syx_file(&data) {
                    indices.push(presets.len());
                    presets.push(preset);
                }
            }
        }
        indices
    };

    let root_entries = sorted_dir_entries(dir);

    let root_loose = load_syx_into(root_entries.iter().filter(|p| is_syx(p)).cloned().collect());
    if !root_loose.is_empty() {
        banks.push(Dx7Bank { name: "(root)".into(), preset_indices: root_loose });
    }

    for bank_path in root_entries.iter().filter(|p| p.is_dir()) {
        let bank_name = bank_path.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
        let indices = load_syx_into(sorted_dir_entries(bank_path));
        if !indices.is_empty() {
            banks.push(Dx7Bank { name: bank_name, preset_indices: indices });
        }
    }

    (presets, banks)
}

/// The flat preset index a (bank, position-within-bank) selection
/// resolves to, or None if either is out of range (e.g. no banks
/// found at all) -- same shape as `sequencer.rs`'s `resolve_sample`.
fn resolve_preset(banks: &[Dx7Bank], bank: usize, preset: usize) -> Option<usize> {
    banks.get(bank)?.preset_indices.get(preset).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_processor(params: Arc<Params>) -> CascadeProcessor {
        CascadeProcessor { params, voices: std::array::from_fn(|_| FmVoice::new()), mono_buf: Vec::new(), chord_headroom: 1 }
    }

    fn hold_pad(params: &Params, i: usize, down: bool) {
        params.held.lock().unwrap()[i] = down;
    }

    /// With a pad held, output must actually reach the device buffer
    /// -- a silent synth would mean the operator graph or envelope
    /// is broken.
    #[test]
    fn held_pad_produces_audible_output() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        hold_pad(&params, 0, true);

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..20 {
            proc.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs())));
        }
        assert!(peak > 0.01, "expected audible output with a pad held, got peak {peak}");
    }

    /// Releasing a held pad must let its envelope decay to silence
    /// rather than cutting off abruptly or hanging forever.
    #[test]
    fn releasing_pad_decays_to_silence() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        hold_pad(&params, 0, true);
        params.release.set(0.05); // short but not instant

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..10 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        hold_pad(&params, 0, false);
        for _ in 0..300 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        let peak: f32 = buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(peak < 0.001, "expected the envelope to have decayed to silence, got peak {peak}");
    }

    /// Every algorithm must stay bounded under a sustained held note
    /// -- feedback (even at max) is a self-contained single-operator
    /// path, so this should never be able to run away regardless of
    /// which algorithm designates it.
    #[test]
    fn every_algorithm_stays_bounded_with_max_feedback() {
        for (i, _) in ALGORITHMS.iter().enumerate() {
            let modbus = ModBus::new();
            let audio_bus = AudioBus::new();
            let mixer_bus = MixerBus::new();
            let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
            hold_pad(&params, 0, true);
            params.algorithm.store(i as u32, Ordering::Relaxed);
            params.feedback.set(1.0);
            for op in params.ops.iter() {
                op.level.set(1.0);
            }

            let mut proc = new_processor(Arc::clone(&params));
            let mut buffer = vec![0.0f32; 512 * 2];
            let mut peak = 0.0f32;
            for _ in 0..50 {
                proc.process(&mut buffer, 2, 48000.0);
                for v in buffer.iter() {
                    assert!(v.is_finite(), "algorithm {i} ('{}') produced a non-finite sample", ALGORITHMS[i].name);
                    peak = peak.max(v.abs());
                }
            }
            assert!(peak < 10.0, "algorithm {i} ('{}') exceeded a sane bound: peak={peak}", ALGORITHMS[i].name);
        }
    }

    /// Holding several pads at once (a chord) must not sum to an
    /// amplitude that scales with how many notes are held -- same
    /// headroom regression class Bloom's voice pool hit.
    #[test]
    fn chords_stay_headroom_normalized() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        for op in params.ops.iter() {
            op.level.set(1.0);
        }
        for i in 0..16 {
            hold_pad(&params, i, true);
        }

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..20 {
            proc.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().cloned().fold(0.0f32, |a, x| a.max(x.abs())));
        }
        assert!(peak < 1.5, "peak grew with how many pads were held instead of staying headroom-normalized: {peak}");
    }

    /// The Mixer app's channel fader must only affect what reaches
    /// the device output, not what this app publishes to audio_bus.rs.
    #[test]
    fn mixer_fader_does_not_affect_audio_bus_publish() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        hold_pad(&params, 0, true);
        params.mix_level.set(0.0);

        let mut proc = new_processor(Arc::clone(&params));
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..10 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        let device_peak = buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert_eq!(device_peak, 0.0);

        let bus_out = params.bus_out.lock().unwrap();
        let bus_peak = bus_out.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(bus_peak > 0.0, "audio_bus publish should be unaffected by the Mixer channel fader");
    }

    /// Builds one 128-byte packed voice block with known Operator 1
    /// (bytes 85-101, the *last* 17-byte block -- see
    /// `parse_packed_voice`'s doc comment) and global fields, zeros
    /// everywhere else, for exercising the parser against exact,
    /// hand-verified byte offsets rather than a real captured file.
    fn make_packed_voice(algorithm: u8, feedback: u8, op1_coarse: u8, op1_level: u8, name: &str) -> [u8; 128] {
        let mut v = [0u8; 128];
        let op1_base = 5 * 17; // Operator 1 is the 6th (last) 17-byte block
        v[op1_base] = 80; // EG rate 1 (attack)
        v[op1_base + 1] = 40; // EG rate 2
        v[op1_base + 2] = 40; // EG rate 3
        v[op1_base + 3] = 20; // EG rate 4 (release)
        v[op1_base + 4] = 99; // EG level 1
        v[op1_base + 5] = 99; // EG level 2
        v[op1_base + 6] = 70; // EG level 3 (sustain)
        v[op1_base + 7] = 0; // EG level 4
        v[op1_base + 14] = op1_level; // output level
        v[op1_base + 15] = op1_coarse << 1; // freq coarse (oscillator mode bit left 0 = ratio mode)
        v[op1_base + 16] = 0; // freq fine
        v[110] = algorithm;
        v[111] = feedback; // feedback occupies bits 0..2
        for (i, b) in name.bytes().take(10).enumerate() {
            v[118 + i] = b;
        }
        v
    }

    /// Wraps one or more 128-byte voice blocks into a full 32-voice
    /// bulk-dump SysEx message (header + 32*128 data bytes + a
    /// placeholder checksum this parser doesn't require to be
    /// correct + F7).
    fn wrap_bulk_dump(voices: &[[u8; 128]; 32]) -> Vec<u8> {
        let mut msg = vec![0xF0, 0x43, 0x00, 0x09, 0x20, 0x00];
        for voice in voices {
            msg.extend_from_slice(voice);
        }
        msg.push(0x00); // checksum -- not validated, see parse_syx_file
        msg.push(0xF7);
        msg
    }

    /// The packed-format parser must read Operator 1's fields (not
    /// some other operator's, and not misaligned by even one byte),
    /// the algorithm number directly, and feedback from the correct
    /// bit range -- this is the single highest-risk part of the
    /// whole import feature, since a silent off-by-one here would
    /// corrupt every imported patch without ever panicking.
    #[test]
    fn packed_bulk_dump_decodes_known_fields_correctly() {
        let mut voices = [[0u8; 128]; 32];
        voices[3] = make_packed_voice(5, 6, 12, 70, "TESTPATCH");
        let data = wrap_bulk_dump(&voices);

        let presets = parse_syx_file(&data);
        assert_eq!(presets.len(), 32, "a 32-voice bulk dump should yield exactly 32 presets");

        let p = &presets[3];
        assert_eq!(p.name, "TESTPATCH");
        assert_eq!(p.algorithm, 13, "Yamaha algorithm 6 keeps its topology after the eight original Cascade layouts");
        assert!((p.feedback - 6.0 / 7.0).abs() < 1e-6, "feedback 6/7 (from the packed nibble), got {}", p.feedback);
        assert!((p.op_ratio[0] - 12.0).abs() < 1e-6, "coarse=12 with fine=0 should be a plain 12x ratio, got {}", p.op_ratio[0]);
        assert!((p.op_level[0] - dx7_level(70)).abs() < 1e-6, "output level 70/99, got {}", p.op_level[0]);
        assert!((p.sustain - 70.0 / 99.0).abs() < 1e-6, "sustain should come from Operator 1's EG level 3 (70), got {}", p.sustain);

        // A voice with algorithm >= 8 must still map somewhere valid
        // (this app only has 8 algorithms, the real DX7 has 32).
        voices[0] = make_packed_voice(29, 0, 1, 50, "HIGHALGO");
        let data2 = wrap_bulk_dump(&voices);
        let presets2 = parse_syx_file(&data2);
        assert!((presets2[0].algorithm as usize) < ALGORITHMS.len(), "algorithm 29 must be mapped into range, got {}", presets2[0].algorithm);
    }

    /// Loading a scanned preset must actually overwrite every knob
    /// it maps to.
    #[test]
    fn load_preset_applies_mapped_values() {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let mut app = CascadeApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);
        app.presets = vec![Dx7Preset {
            name: "Manual".into(),
            algorithm: 4,
            feedback: 0.5,
            op_ratio: [2.0, 3.0, 1.0, 1.0, 1.0, 1.0],
            op_level: [0.9, 0.4, 0.4, 0.4, 0.4, 0.4],
            op_envelopes: None,
            attack: 0.1,
            decay: 0.2,
            sustain: 0.6,
            release: 0.3,
        }];

        app.load_preset(0);

        assert_eq!(app.params.algorithm.load(Ordering::Relaxed), 4);
        assert_eq!(app.params.feedback.get(), 0.5);
        assert_eq!(app.params.ops[0].ratio.get(), 2.0);
        assert_eq!(app.params.ops[1].ratio.get(), 3.0);
        assert_eq!(app.params.ops[0].level.get(), 0.9);
        assert_eq!(app.params.attack.get(), 0.1);
        assert_eq!(app.params.sustain.get(), 0.6);
    }

    /// `scan_presets` must find every `.syx` file directly under the
    /// root (grouped into a synthetic "(root)" bank), ignore anything
    /// else, and never panic on a directory that doesn't exist.
    #[test]
    fn scan_presets_finds_syx_files_and_ignores_others() {
        let dir = std::env::temp_dir().join(format!("portamax_cascade_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut voices = [[0u8; 128]; 32];
        voices[0] = make_packed_voice(1, 0, 1, 80, "BANKVOICE");
        std::fs::write(dir.join("bank.syx"), wrap_bulk_dump(&voices)).unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a patch file").unwrap();

        let (presets, banks) = scan_presets(&dir);
        assert_eq!(presets.len(), 32, "expected exactly the 32 voices from the one .syx file, ignoring the .txt file");
        assert_eq!(presets[0].name, "BANKVOICE");
        assert_eq!(banks.len(), 1, "a loose top-level .syx file should form one synthetic root bank");
        assert_eq!(banks[0].name, "(root)");
        assert_eq!(banks[0].preset_indices.len(), 32);

        let _ = std::fs::remove_dir_all(&dir);

        let (missing_presets, missing_banks) = scan_presets(Path::new("/nonexistent/portamax/path"));
        assert!(missing_presets.is_empty(), "a missing folder should yield no presets, not panic");
        assert!(missing_banks.is_empty());
    }

    /// A subfolder under `dx7_presets/` must become its own named
    /// bank grouping just the `.syx` files directly inside it, kept
    /// separate from any loose top-level files' "(root)" bank -- this
    /// is the actual folder-grouping feature the user asked for.
    #[test]
    fn scan_presets_groups_subfolders_into_named_banks() {
        let dir = std::env::temp_dir().join(format!("portamax_cascade_banks_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bank_a = dir.join("Factory A");
        let bank_b = dir.join("Factory B");
        std::fs::create_dir_all(&bank_a).unwrap();
        std::fs::create_dir_all(&bank_b).unwrap();

        let mut voices_a = [[0u8; 128]; 32];
        voices_a[0] = make_packed_voice(1, 0, 1, 80, "APATCH");
        std::fs::write(bank_a.join("bank.syx"), wrap_bulk_dump(&voices_a)).unwrap();

        let mut voices_b = [[0u8; 128]; 32];
        voices_b[0] = make_packed_voice(2, 0, 1, 80, "BPATCH");
        std::fs::write(bank_b.join("bank.syx"), wrap_bulk_dump(&voices_b)).unwrap();

        let (presets, banks) = scan_presets(&dir);
        assert_eq!(presets.len(), 64, "32 voices from each of the two subfolders");
        assert_eq!(banks.len(), 2, "each subfolder should become its own bank, no root bank since nothing is loose");

        let names: Vec<&str> = banks.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"Factory A"));
        assert!(names.contains(&"Factory B"));

        let bank_a_entry = banks.iter().find(|b| b.name == "Factory A").unwrap();
        assert_eq!(bank_a_entry.preset_indices.len(), 32);
        assert_eq!(presets[bank_a_entry.preset_indices[0]].name, "APATCH");

        assert_eq!(resolve_preset(&banks, 0, 0), Some(banks[0].preset_indices[0]));
        assert_eq!(resolve_preset(&banks, 99, 0), None, "out-of-range bank must resolve to None, not panic");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The list column's text (at `paramlist::TEXT_SCALE`) must never
    /// reach `node_x` (380), where the operator-routing graph starts --
    /// bank names come straight from real folders on disk (unlike
    /// every other string in this app, nothing here bounds their
    /// length), so this drives an artificially long one through the
    /// exact same path `scan_presets` would, plus every other leaf/
    /// group's real text, to check the whole column at once rather
    /// than just guessing which row is worst.
    #[test]
    fn list_text_never_reaches_the_operator_graph() {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let mut app = CascadeApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);
        app.expanded = [true; NUM_GROUPS];
        app.banks = vec![Dx7Bank { name: "A Very Long Real-World Bank Folder Name Indeed".into(), preset_indices: vec![0] }];
        app.presets = vec![Dx7Preset {
            name: "GRANDPIANO".into(), // the real, and DX7-format-maximum-length, worst case
            algorithm: 0,
            feedback: 0.0,
            op_ratio: [1.0; NUM_OPS],
            op_level: [0.5; NUM_OPS],
            op_envelopes: None,
            attack: 0.0,
            decay: 0.3,
            sustain: 0.7,
            release: 0.3,
        }];
        app.bank_browse = 0;
        app.preset_browse = 0;

        let rows = app.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if app.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", app.group_name(*g)), app.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", app.leaf_name(*sel)), app.leaf_value(*sel)),
            })
            .collect();

        let mut list = ParamList::new();
        let mut fb = FrameBuffer::new();
        list.draw(&mut fb, 16, 44, 24, rows.len(), &display_rows);
        let mut max_x = 0i32;
        for (i, &p) in fb.buffer().iter().enumerate() {
            if p != 0 {
                max_x = max_x.max((i % crate::display::WIDTH) as i32);
            }
        }
        const NODE_X: i32 = 380;
        assert!(max_x < NODE_X, "list text reached x={max_x}, at or past node_x ({NODE_X})");
    }

    /// Every real preset actually shipped in `dx7_presets/` must be
    /// loadable and playable without panicking or producing
    /// non-finite audio -- this drives the exact browse-then-load
    /// sequence the real UI performs (pick a bank, pick a preset,
    /// press to load) across the whole real patch library in this
    /// checkout, not just hand-built synthetic test data, since
    /// real-world `.syx` files can carry byte values the synthetic
    /// tests never exercise.
    #[test]
    fn every_real_preset_loads_and_plays_without_panicking() {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let mut app = CascadeApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);

        if app.presets.is_empty() {
            return; // nothing shipped in this checkout to test against
        }

        let mut checked = 0usize;
        for bank in 0..app.banks.len() {
            app.bank_browse = bank;
            let count = app.banks[bank].preset_indices.len();
            for preset in 0..count {
                app.preset_browse = preset;
                app.reset(Selection::Preset); // load
                hold_pad(&app.params, 0, true);
                let mut proc = new_processor(Arc::clone(&app.params));
                let mut buffer = vec![0.0f32; 512 * 2];
                for _ in 0..4 {
                    proc.process(&mut buffer, 2, 48000.0);
                    for v in buffer.iter() {
                        assert!(v.is_finite(), "bank '{}' preset {preset} ('{}') produced non-finite audio", app.banks[bank].name, app.presets[app.selected_preset_index().unwrap()].name);
                        assert!(v.abs() < 10.0, "bank '{}' preset {preset} ('{}') exceeded a sane bound: sample={v}", app.banks[bank].name, app.presets[app.selected_preset_index().unwrap()].name);
                    }
                }
                checked += 1;
            }
        }
        assert!(checked > 0);
    }
}

#[cfg(test)]
mod imported_patch_regressions {
    use super::*;
    #[test]
    fn steelcans_operator_five_uses_yamaha_frequency_bitfield() {
        let data = include_bytes!("../../dx7_presets/Giorgio Robino/PERCDANZ.SYX");
        let presets = parse_syx_file(data);
        let patch = presets.iter().find(|p| p.name.eq_ignore_ascii_case("SteelCans")).unwrap();
        assert_eq!(patch.op_ratio[4],5.0);
        assert_eq!(patch.op_ratio[0],1.0);
        assert_eq!(patch.feedback,0.0);
    }
    #[test]
    fn packed_frequency_mode_and_sync_do_not_pollute_ratio_or_feedback() {
        let mut data = [0u8;128];
        data[85+15] = (7 << 1) | 1;
        data[111] = 0b0110_1011;
        let patch = parse_packed_voice(&data);
        assert_eq!(patch.op_ratio[0],7.0);
        assert!((patch.feedback-3.0/7.0).abs()<1e-6);
    }
}
