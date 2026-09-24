//! A generative circular sequencer built from two groups of the same
//! kind of element -- **dots** -- never a separate "arm" concept.
//!
//! **Outer dots** sit on the boundary circle at fixed, evenly-spaced
//! angles -- they never move, each can be switched on or off, and
//! there's a count for how many exist. They're the trigger
//! references, same role the old "arms" played, just static and
//! togglable instead of rotating and bendable.
//!
//! **Inner dots** carry the notes: each orbits on its own fixed
//! concentric ring (ring radius set by its index -- innermost =
//! lowest note, outermost = highest, spread across an adjustable
//! Octave Range), each spinning at its own multiple of the shape's
//! base dot speed. By default every dot starts at the same fixed
//! 12-o'clock position, so they start bunched together on the
//! 12-o'clock radial, gradually spread apart as the faster ones lap
//! the slower ones, and -- because the per-dot speed multipliers are
//! simple rational steps -- periodically re-converge. Turning on
//! `Selection::RandomStart` for a shape instead gives each dot its
//! own random starting phase on the next Retrigger/dot-count change,
//! so the same shape doesn't unfold into an identical pattern every
//! time -- Randomize always turns this on for exactly that reason.
//!
//! A note fires whenever an inner dot's orbit crosses an *active*
//! outer dot's fixed angle. Since the outer dot's position never
//! moves, the trigger reference -- and the line that flashes from it
//! to the center when it fires -- are both static, like the boundary
//! circle itself; only the inner dots visibly orbit.
//!
//! Changing either dot count (inner or outer) resets every inner
//! dot's phase back to its 12-o'clock start -- ring/angle assignments
//! shift when a count changes, so a clean restart avoids a dot
//! jumping mid-orbit into a different ring.
//!
//! 8 independent shape instances, each with its own dots/scale/voice,
//! all mixing together, same architecture as Madness/Pam's: a shared
//! Master Clock BPM, per-shape Running (off by default, deliberately
//! first in that shape's tree) and Randomize, and Speed/Level exposed
//! to the shared ModBus.
//!
//! Simplified: one global trigger probability, not per-dot; up to 16
//! inner dots and 8 outer dots; Ping-Pong direction's trigger timing
//! is approximate right at the bounce instant (see `crosses`' doc
//! comment) -- everything else about crossing detection here is
//! exact, not a visual approximation.

use crate::app::{App, Input};
use crate::apps::plaits::{ENGINE_NAMES, ROOT_NAMES, SCALE_TYPES};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::plaits_ffi::{PlaitsParams, PlaitsVoice};
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NUM_SHAPES: usize = 8;
const MIN_DOTS: usize = 2;
/// Doubled from the original 16 -- Harmony Bloom (the real generative
/// MIDI plugin this app takes inspiration from) can run dense enough
/// patterns to feel like a woven web rather than a simple orbit; see
/// `INNER_DOT_PX`/`OUTER_DOT_PX` for the matching visual shrink that
/// keeps a fully-populated shape legible instead of a smear.
const MAX_DOTS: usize = 32;
const DEFAULT_DOTS: usize = 8;
const MIN_OUTER: usize = 1;
/// Matches Harmony Bloom's own "Trigger Bars x8" exactly -- 8 is the
/// real count there too, not a simplification.
const MAX_OUTER: usize = 8;
const DEFAULT_OUTER: usize = 3;
const MIN_OCTAVE_RANGE: u32 = 1;
const MAX_OCTAVE_RANGE: u32 = 4;
const DEFAULT_OCTAVE_RANGE: u32 = 2;
/// Harmony Bloom's own Octave Transpose is a fixed 5-stop control
/// (-24/-12/0/+12/+24 semitones), not a continuous knob -- ported
/// exactly, since a continuous transpose would mostly just retune
/// mid-scale rather than cleanly shift octaves.
const OCTAVE_TRANSPOSE_STEPS: [i32; 5] = [-24, -12, 0, 12, 24];
const DEFAULT_OCTAVE_TRANSPOSE_INDEX: usize = 2; // 0
/// Absolute note-number clamp range (Min/Max Note) -- generous enough
/// to cover every note `note_for_position` can produce even at max
/// Octave Range/Transpose, without being unbounded.
const MIN_ABS_NOTE: i32 = 0;
const MAX_ABS_NOTE: i32 = 127;
/// Per-trigger-bar (outer dot) octave offset range -- Harmony Bloom's
/// "Trigger Bars" each carry their own Octave (+ Random Octave)
/// setting; ported as a small fixed range rather than the full
/// absolute-note range, since this is a per-trigger *offset* on top
/// of whatever the shape's own pitch mapping already produced.
const MIN_OUTER_OCTAVE: i32 = -2;
const MAX_OUTER_OCTAVE: i32 = 2;
/// How many scale degrees a shape's own Custom Scale can toggle --
/// one per semitone in an octave, edited via the first 12 of the 16
/// physical pads (see `Selection::ScaleCustomEdit`), the same
/// "repurpose the grid while a mode's active" idiom Sequencer's Grid
/// Edit/Pad Perform already use.
const NUM_CUSTOM_DEGREES: usize = 12;
/// `Selection::Scale`'s value one past `SCALE_TYPES`' real presets --
/// Bloom's own "Custom Scale" slot, backed by `ShapeParams::
/// custom_scale`, since `SCALE_TYPES` itself is a shared `&'static`
/// table other apps also read and shouldn't be mutated/extended here.
fn custom_scale_index() -> u32 {
    SCALE_TYPES.len() as u32
}
/// Base multiplier applied to every dot's speed, on top of its own
/// per-index step below.
const DOT_SPEED_MULT: f32 = 0.35;
/// Each dot's Linear-curve (`speed_curve == 0`) speed multiple is
/// `1 + dot_index * DOT_SPEED_STEP`. All dots start at the same phase
/// by default (see `ShapeParams::new`), so this is what makes them
/// gradually spread apart and then periodically snap back into
/// alignment -- see `dot_speed_mult` for the other curves.
const DOT_SPEED_STEP: f32 = 0.2;

/// A dot's speed multiple (on top of `DOT_SPEED_MULT`), by `curve`
/// (`ShapeParams::speed_curve`, set as part of a `Pattern`):
///   0 Linear -- `1 + d * DOT_SPEED_STEP`, a smooth ramp (the
///     original/"Spiral" behavior).
///   1 Alternating -- even and odd dots ramp at very different rates,
///     interleaving fast and slow orbits instead of a smooth spread.
///   2 Clustered -- dots in groups of 4 share nearly the same speed,
///     so they orbit (and fire) together as a chord-like cluster that
///     itself drifts around the circle, rather than each dot tracing
///     its own independent path.
fn dot_speed_mult(d: usize, curve: u32) -> f32 {
    match curve {
        1 => {
            let half = (d / 2) as f32;
            if d % 2 == 0 {
                1.0 + half * DOT_SPEED_STEP * 0.5
            } else {
                1.0 + half * DOT_SPEED_STEP * 2.5
            }
        }
        2 => {
            let cluster = (d / 4) as f32;
            1.0 + cluster * DOT_SPEED_STEP * 3.0
        }
        _ => 1.0 + d as f32 * DOT_SPEED_STEP,
    }
}

/// One curated starting shape -- see `Selection::Pattern`.
struct PatternPreset {
    dots: usize,
    outer: usize,
    outer_active: [bool; MAX_OUTER],
    speed_curve: u32,
    dot_angle_offset: f32,
    ring_rotation_offset: f32,
    outer_angle_offset: f32,
}

const PATTERN_NAMES: [&str; 50] = [
    "Spiral", "Fan", "Pinwheel", "Mandala", "Web",
    "Cluster", "Lattice", "Vortex", "Corona", "Helix",
    "Starburst", "Lotus", "Fractal", "Orbit", "Ripple",
    "Weave", "Prism", "Aurora", "Comet", "Whirl",
    "Bramble", "Thicket", "Trellis", "Canopy", "Frost",
    "Crystal", "Echo", "Pulse", "Drift", "Tangle",
    "Coral", "Rosette", "Chrysanthemum", "Fern", "Ivy",
    "Vine", "Meadow", "Garden", "Dandelion", "Marigold",
    "Poppy", "Iris", "Orchid", "Tulip", "Peony",
    "Sunflower", "Clover", "Wisteria", "Jasmine", "Camellia",
];

/// Index 0 ("Spiral") intentionally matches every `ShapeParams::new`
/// default exactly -- selecting it is a real no-op, a documented
/// "back to the original look" choice, not a placeholder. Indices 1-5
/// are hand-tuned hero shapes; the rest are generated across a wide
/// spread of dot/outer counts, speed curves, and offsets so scrolling
/// through all 50 turns up something genuinely different each time,
/// not near-duplicates.
const PATTERN_PRESETS: [PatternPreset; 50] = [
    // 0: Spiral
    PatternPreset {
        dots: DEFAULT_DOTS,
        outer: DEFAULT_OUTER,
        outer_active: [true, true, true, false, false, false, false, false],
        speed_curve: 0,
        dot_angle_offset: 0.000,
        ring_rotation_offset: 0.000,
        outer_angle_offset: 0.000,
    },
    // 1: Fan
    PatternPreset {
        dots: 12,
        outer: 4,
        outer_active: [true, true, true, true, false, false, false, false],
        speed_curve: 0,
        dot_angle_offset: 0.080,
        ring_rotation_offset: 0.000,
        outer_angle_offset: 0.000,
    },
    // 2: Pinwheel
    PatternPreset {
        dots: 16,
        outer: 5,
        outer_active: [true, true, true, true, true, false, false, false],
        speed_curve: 1,
        dot_angle_offset: 0.150,
        ring_rotation_offset: 0.000,
        outer_angle_offset: 0.000,
    },
    // 3: Mandala
    PatternPreset {
        dots: 12,
        outer: 6,
        outer_active: [true, true, true, true, true, true, false, false],
        speed_curve: 0,
        dot_angle_offset: 0.000,
        ring_rotation_offset: 0.500,
        outer_angle_offset: 0.000,
    },
    // 4: Web
    PatternPreset {
        dots: 20,
        outer: 8,
        outer_active: [true, true, true, true, true, true, true, true],
        speed_curve: 2,
        dot_angle_offset: 0.050,
        ring_rotation_offset: 0.250,
        outer_angle_offset: 0.000,
    },
    // 5: Cluster
    PatternPreset {
        dots: 16,
        outer: 4,
        outer_active: [true, true, true, true, false, false, false, false],
        speed_curve: 2,
        dot_angle_offset: 0.000,
        ring_rotation_offset: 0.000,
        outer_angle_offset: 0.125,
    },
    // 6: Lattice
    PatternPreset {
        dots: 13,
        outer: 3,
        outer_active: [true, true, true, false, false, false, false, false],
        speed_curve: 0,
        dot_angle_offset: -0.128,
        ring_rotation_offset: -0.082,
        outer_angle_offset: 0.076,
    },
    // 7: Vortex
    PatternPreset {
        dots: 20,
        outer: 6,
        outer_active: [true, true, false, true, true, true, false, false],
        speed_curve: 1,
        dot_angle_offset: -0.091,
        ring_rotation_offset: -0.029,
        outer_angle_offset: 0.147,
    },
    // 8: Corona
    PatternPreset {
        dots: 27,
        outer: 1,
        outer_active: [true, false, false, false, false, false, false, false],
        speed_curve: 2,
        dot_angle_offset: -0.054,
        ring_rotation_offset: 0.024,
        outer_angle_offset: 0.218,
    },
    // 9: Helix
    PatternPreset {
        dots: 3,
        outer: 4,
        outer_active: [true, true, true, true, false, false, false, false],
        speed_curve: 0,
        dot_angle_offset: -0.017,
        ring_rotation_offset: 0.077,
        outer_angle_offset: 0.289,
    },
    // 10: Starburst
    PatternPreset {
        dots: 10,
        outer: 7,
        outer_active: [true, true, true, true, false, true, true, false],
        speed_curve: 1,
        dot_angle_offset: 0.020,
        ring_rotation_offset: 0.130,
        outer_angle_offset: 0.360,
    },
    // 11: Lotus
    PatternPreset {
        dots: 17,
        outer: 2,
        outer_active: [true, true, false, false, false, false, false, false],
        speed_curve: 2,
        dot_angle_offset: 0.057,
        ring_rotation_offset: 0.183,
        outer_angle_offset: -0.369,
    },
    // 12: Fractal
    PatternPreset {
        dots: 24,
        outer: 5,
        outer_active: [true, true, false, true, true, false, false, false],
        speed_curve: 0,
        dot_angle_offset: 0.094,
        ring_rotation_offset: 0.236,
        outer_angle_offset: -0.298,
    },
    // 13: Orbit
    PatternPreset {
        dots: 31,
        outer: 8,
        outer_active: [true, false, true, true, true, true, false, true],
        speed_curve: 1,
        dot_angle_offset: 0.131,
        ring_rotation_offset: 0.289,
        outer_angle_offset: -0.227,
    },
    // 14: Ripple
    PatternPreset {
        dots: 7,
        outer: 3,
        outer_active: [true, true, true, false, false, false, false, false],
        speed_curve: 2,
        dot_angle_offset: 0.168,
        ring_rotation_offset: 0.342,
        outer_angle_offset: -0.156,
    },
    // 15: Weave
    PatternPreset {
        dots: 14,
        outer: 6,
        outer_active: [true, true, true, true, false, true, false, false],
        speed_curve: 0,
        dot_angle_offset: 0.205,
        ring_rotation_offset: 0.395,
        outer_angle_offset: -0.085,
    },
    // 16: Prism
    PatternPreset {
        dots: 21,
        outer: 1,
        outer_active: [true, false, false, false, false, false, false, false],
        speed_curve: 1,
        dot_angle_offset: 0.242,
        ring_rotation_offset: 0.448,
        outer_angle_offset: -0.014,
    },
    // 17: Aurora
    PatternPreset {
        dots: 28,
        outer: 4,
        outer_active: [true, true, false, true, false, false, false, false],
        speed_curve: 2,
        dot_angle_offset: 0.279,
        ring_rotation_offset: -0.499,
        outer_angle_offset: 0.057,
    },
    // 18: Comet
    PatternPreset {
        dots: 4,
        outer: 7,
        outer_active: [true, false, true, true, true, true, false, false],
        speed_curve: 0,
        dot_angle_offset: 0.316,
        ring_rotation_offset: -0.446,
        outer_angle_offset: 0.128,
    },
    // 19: Whirl
    PatternPreset {
        dots: 11,
        outer: 2,
        outer_active: [true, true, false, false, false, false, false, false],
        speed_curve: 1,
        dot_angle_offset: -0.347,
        ring_rotation_offset: -0.393,
        outer_angle_offset: 0.199,
    },
    // 20: Bramble
    PatternPreset {
        dots: 18,
        outer: 5,
        outer_active: [true, true, true, true, false, false, false, false],
        speed_curve: 2,
        dot_angle_offset: -0.310,
        ring_rotation_offset: -0.340,
        outer_angle_offset: 0.270,
    },
    // 21: Thicket
    PatternPreset {
        dots: 25,
        outer: 8,
        outer_active: [true, true, true, false, true, true, true, true],
        speed_curve: 0,
        dot_angle_offset: -0.273,
        ring_rotation_offset: -0.287,
        outer_angle_offset: 0.341,
    },
    // 22: Trellis
    PatternPreset {
        dots: 32,
        outer: 3,
        outer_active: [true, true, false, false, false, false, false, false],
        speed_curve: 1,
        dot_angle_offset: -0.236,
        ring_rotation_offset: -0.234,
        outer_angle_offset: -0.388,
    },
    // 23: Canopy
    PatternPreset {
        dots: 8,
        outer: 6,
        outer_active: [true, false, true, true, true, true, false, false],
        speed_curve: 2,
        dot_angle_offset: -0.199,
        ring_rotation_offset: -0.181,
        outer_angle_offset: -0.317,
    },
    // 24: Frost
    PatternPreset {
        dots: 15,
        outer: 1,
        outer_active: [true, false, false, false, false, false, false, false],
        speed_curve: 0,
        dot_angle_offset: -0.162,
        ring_rotation_offset: -0.128,
        outer_angle_offset: -0.246,
    },
    // 25: Crystal
    PatternPreset {
        dots: 22,
        outer: 4,
        outer_active: [true, true, true, true, false, false, false, false],
        speed_curve: 1,
        dot_angle_offset: -0.125,
        ring_rotation_offset: -0.075,
        outer_angle_offset: -0.175,
    },
    // 26: Echo
    PatternPreset {
        dots: 29,
        outer: 7,
        outer_active: [true, true, true, false, true, true, true, false],
        speed_curve: 2,
        dot_angle_offset: -0.088,
        ring_rotation_offset: -0.022,
        outer_angle_offset: -0.104,
    },
    // 27: Pulse
    PatternPreset {
        dots: 5,
        outer: 2,
        outer_active: [true, true, false, false, false, false, false, false],
        speed_curve: 0,
        dot_angle_offset: -0.051,
        ring_rotation_offset: 0.031,
        outer_angle_offset: -0.033,
    },
    // 28: Drift
    PatternPreset {
        dots: 12,
        outer: 5,
        outer_active: [true, false, true, true, true, false, false, false],
        speed_curve: 1,
        dot_angle_offset: -0.014,
        ring_rotation_offset: 0.084,
        outer_angle_offset: 0.038,
    },
    // 29: Tangle
    PatternPreset {
        dots: 19,
        outer: 8,
        outer_active: [true, true, true, true, true, false, true, true],
        speed_curve: 2,
        dot_angle_offset: 0.023,
        ring_rotation_offset: 0.137,
        outer_angle_offset: 0.109,
    },
    // 30: Coral
    PatternPreset {
        dots: 26,
        outer: 3,
        outer_active: [true, true, true, false, false, false, false, false],
        speed_curve: 0,
        dot_angle_offset: 0.060,
        ring_rotation_offset: 0.190,
        outer_angle_offset: 0.180,
    },
    // 31: Rosette
    PatternPreset {
        dots: 2,
        outer: 6,
        outer_active: [true, true, true, false, true, true, false, false],
        speed_curve: 1,
        dot_angle_offset: 0.097,
        ring_rotation_offset: 0.243,
        outer_angle_offset: 0.251,
    },
    // 32: Chrysanthemum
    PatternPreset {
        dots: 9,
        outer: 1,
        outer_active: [true, false, false, false, false, false, false, false],
        speed_curve: 2,
        dot_angle_offset: 0.134,
        ring_rotation_offset: 0.296,
        outer_angle_offset: 0.322,
    },
    // 33: Fern
    PatternPreset {
        dots: 16,
        outer: 4,
        outer_active: [true, false, true, true, false, false, false, false],
        speed_curve: 0,
        dot_angle_offset: 0.171,
        ring_rotation_offset: 0.349,
        outer_angle_offset: 0.393,
    },
    // 34: Ivy
    PatternPreset {
        dots: 23,
        outer: 7,
        outer_active: [true, true, true, true, true, false, true, false],
        speed_curve: 1,
        dot_angle_offset: 0.208,
        ring_rotation_offset: 0.402,
        outer_angle_offset: -0.336,
    },
    // 35: Vine
    PatternPreset {
        dots: 30,
        outer: 2,
        outer_active: [true, true, false, false, false, false, false, false],
        speed_curve: 2,
        dot_angle_offset: 0.245,
        ring_rotation_offset: 0.455,
        outer_angle_offset: -0.265,
    },
    // 36: Meadow
    PatternPreset {
        dots: 6,
        outer: 5,
        outer_active: [true, true, true, false, true, false, false, false],
        speed_curve: 0,
        dot_angle_offset: 0.282,
        ring_rotation_offset: -0.492,
        outer_angle_offset: -0.194,
    },
    // 37: Garden
    PatternPreset {
        dots: 13,
        outer: 8,
        outer_active: [true, true, false, true, true, true, true, false],
        speed_curve: 1,
        dot_angle_offset: 0.319,
        ring_rotation_offset: -0.439,
        outer_angle_offset: -0.123,
    },
    // 38: Dandelion
    PatternPreset {
        dots: 20,
        outer: 3,
        outer_active: [true, false, true, false, false, false, false, false],
        speed_curve: 2,
        dot_angle_offset: -0.344,
        ring_rotation_offset: -0.386,
        outer_angle_offset: -0.052,
    },
    // 39: Marigold
    PatternPreset {
        dots: 27,
        outer: 6,
        outer_active: [true, true, true, true, true, false, false, false],
        speed_curve: 0,
        dot_angle_offset: -0.307,
        ring_rotation_offset: -0.333,
        outer_angle_offset: 0.019,
    },
    // 40: Poppy
    PatternPreset {
        dots: 3,
        outer: 1,
        outer_active: [true, false, false, false, false, false, false, false],
        speed_curve: 1,
        dot_angle_offset: -0.270,
        ring_rotation_offset: -0.280,
        outer_angle_offset: 0.090,
    },
    // 41: Iris
    PatternPreset {
        dots: 10,
        outer: 4,
        outer_active: [true, true, true, false, false, false, false, false],
        speed_curve: 2,
        dot_angle_offset: -0.233,
        ring_rotation_offset: -0.227,
        outer_angle_offset: 0.161,
    },
    // 42: Orchid
    PatternPreset {
        dots: 17,
        outer: 7,
        outer_active: [true, true, false, true, true, true, true, false],
        speed_curve: 0,
        dot_angle_offset: -0.196,
        ring_rotation_offset: -0.174,
        outer_angle_offset: 0.232,
    },
    // 43: Tulip
    PatternPreset {
        dots: 24,
        outer: 2,
        outer_active: [true, false, false, false, false, false, false, false],
        speed_curve: 1,
        dot_angle_offset: -0.159,
        ring_rotation_offset: -0.121,
        outer_angle_offset: 0.303,
    },
    // 44: Peony
    PatternPreset {
        dots: 31,
        outer: 5,
        outer_active: [true, true, true, true, true, false, false, false],
        speed_curve: 2,
        dot_angle_offset: -0.122,
        ring_rotation_offset: -0.068,
        outer_angle_offset: 0.374,
    },
    // 45: Sunflower
    PatternPreset {
        dots: 7,
        outer: 8,
        outer_active: [true, true, true, true, false, true, true, true],
        speed_curve: 0,
        dot_angle_offset: -0.085,
        ring_rotation_offset: -0.015,
        outer_angle_offset: -0.355,
    },
    // 46: Clover
    PatternPreset {
        dots: 14,
        outer: 3,
        outer_active: [true, true, true, false, false, false, false, false],
        speed_curve: 1,
        dot_angle_offset: -0.048,
        ring_rotation_offset: 0.038,
        outer_angle_offset: -0.284,
    },
    // 47: Wisteria
    PatternPreset {
        dots: 21,
        outer: 6,
        outer_active: [true, true, false, true, true, true, false, false],
        speed_curve: 2,
        dot_angle_offset: -0.011,
        ring_rotation_offset: 0.091,
        outer_angle_offset: -0.213,
    },
    // 48: Jasmine
    PatternPreset {
        dots: 28,
        outer: 1,
        outer_active: [true, false, false, false, false, false, false, false],
        speed_curve: 0,
        dot_angle_offset: 0.026,
        ring_rotation_offset: 0.144,
        outer_angle_offset: -0.142,
    },
    // 49: Camellia
    PatternPreset {
        dots: 4,
        outer: 4,
        outer_active: [true, true, true, true, false, false, false, false],
        speed_curve: 1,
        dot_angle_offset: 0.063,
        ring_rotation_offset: 0.197,
        outer_angle_offset: -0.071,
    },
];
const MIN_SPEED_HZ: f32 = 0.02;
const MAX_SPEED_HZ: f32 = 4.0;
const DEFAULT_SPEED_HZ: f32 = 0.25;
const MIN_BPM: f32 = 40.0;
const MAX_BPM: f32 = 300.0;
const DEFAULT_BPM: f32 = 120.0;
const DIRECTION_NAMES: [&str; 3] = ["Forward", "Reverse", "Ping-Pong"];
const CLOCK_MODS: [(&str, f32); 9] =
    [("/8", 0.125), ("/4", 0.25), ("/2", 0.5), ("x1", 1.0), ("x2", 2.0), ("x3", 3.0), ("x4", 4.0), ("x8", 8.0), ("x16", 16.0)];

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * (max - min) * 0.05).clamp(min, max);
    value.set(next);
}

fn note_for_position(pos: usize, intervals: &[i32], root: u32) -> i32 {
    let len = intervals.len().max(1);
    let degree = pos % len;
    let octave = pos / len;
    60 + root as i32 + intervals.get(degree).copied().unwrap_or(0) + octave as i32 * 12
}

/// The real interval list a shape's current Scale selection resolves
/// to -- either straight from the shared `SCALE_TYPES` preset table,
/// or (once Scale reaches `custom_scale_index()`) this shape's own
/// `custom_scale` degree toggles, read live off the atomics. An empty
/// custom scale (every degree off) falls back to a single root-only
/// "interval" rather than an empty slice, so `note_for_position`'s
/// `% len` never divides by zero.
fn resolved_scale_intervals(sp: &ShapeParams) -> Vec<i32> {
    let scale = sp.scale.load(Ordering::Relaxed);
    if scale == custom_scale_index() {
        let degrees: Vec<i32> = (0..NUM_CUSTOM_DEGREES as i32).filter(|&d| sp.custom_scale[d as usize].load(Ordering::Relaxed)).collect();
        if degrees.is_empty() { vec![0] } else { degrees }
    } else {
        SCALE_TYPES[scale as usize % SCALE_TYPES.len()].1.to_vec()
    }
}

/// Maps a raw, unbounded, monotonically-accumulating phase to the
/// actual 0..1 angle used for *drawing* -- Forward/Reverse just wrap,
/// Ping-Pong triangle-wraps (bounces at 0 and 1). Only used for
/// visual placement; crossing detection uses the raw value directly
/// (see `crosses`), not this.
fn display_phase(raw: f32, direction: u32) -> f32 {
    if direction == 2 {
        let m = raw.rem_euclid(2.0);
        if m <= 1.0 {
            m
        } else {
            2.0 - m
        }
    } else {
        raw.rem_euclid(1.0)
    }
}

/// Whether the continuously-swept **raw** (unwrapped) interval from
/// `prev_raw` to `new_raw` passes through any point congruent to
/// `target` modulo 1 turn.
///
/// This replaces an earlier, buggy approach (independently wrapping
/// each endpoint to 0..1 with `rem_euclid`, then taking their plain
/// min/max as the swept interval): whenever a block's sweep itself
/// crossed the 0/1 wrap point, that produced the *complementary* arc
/// -- silently reporting false crossings for targets in the short arc
/// between the two wrapped endpoints, and missing real crossings for
/// targets in the long arc that was actually swept. Working in raw
/// (never-wrapped) space and checking every `target + k` near the
/// current window sidesteps that entirely, however many full turns
/// have already accumulated.
///
/// Not adjusted for Ping-Pong's bounce (the displayed angle isn't a
/// linear function of the raw accumulator there) -- trigger timing is
/// exact for Forward/Reverse, and only approximate right at a
/// Ping-Pong bounce instant. A fully correct fix would track the
/// reflected phase family separately; given how briefly that instant
/// lasts, this was judged not worth the added complexity.
fn crosses(prev_raw: f32, new_raw: f32, target: f32) -> bool {
    let lo = prev_raw.min(new_raw);
    let hi = prev_raw.max(new_raw);
    if hi - lo > 3.0 {
        return true; // pathologically large jump this block -- just fire
    }
    let start_k = (lo - target).floor() - 1.0;
    for i in 0..5 {
        let t = target + start_k + i as f32;
        if t > lo && t <= hi {
            return true;
        }
    }
    false
}

/// The fixed ring radius (as a fraction of the outer circle) a given
/// inner dot orbits on -- assigned by index, spread evenly from near
/// the center (lowest note) to near the edge (highest), never
/// changing while the dot spins. This is what note pitch is keyed to.
fn dot_radius_frac(d: usize, num_dots: usize) -> f32 {
    (d as f32 + 0.5) / num_dots.max(1) as f32
}

/// The fixed angle (in turns) an outer dot sits at -- evenly spaced
/// around the boundary, index 0 at 12 o'clock, never changing.
fn outer_angle_turns(o: usize, num_outer: usize) -> f32 {
    o as f32 / num_outer.max(1) as f32
}

/// Dot `d`'s current angular bias (in turns) from `DotAngleOffset`
/// (a linear fan/pinwheel stagger, `dot_angle_offset * d`) and
/// `RingRotationOffset` (an alternating flip added to odd-indexed
/// dots only) -- both real Pam's-modulation targets, so this is
/// recomputed fresh every block/frame from the *current* (possibly
/// externally-modulated) offset values, not baked into the dot's own
/// phase accumulator. Added directly to a dot's raw phase for
/// display; subtracted from an outer dot's target angle for the
/// equivalent effect in `crosses`-based trigger detection (see
/// `process`'s own comment on why).
fn dot_angle_bias(d: usize, dot_angle_offset: f32, ring_rotation_offset: f32) -> f32 {
    let stagger = dot_angle_offset * d as f32;
    let flip = if d % 2 == 1 { ring_rotation_offset } else { 0.0 };
    stagger + flip
}

/// This shape's real (possibly Pam's-modulated) Direction index --
/// the same "base + external, wrapped" combination `process` uses for
/// audio, shared here so both visual renderers (`bloom_visual`/
/// `draw`) show the direction actually playing, not just the manual
/// base setting.
fn modulated_direction(sp: &ShapeParams) -> u32 {
    (sp.direction.load(Ordering::Relaxed) as i32 + sp.ext_direction.get().round() as i32).rem_euclid(DIRECTION_NAMES.len() as i32) as u32
}

// --- Bloom's own palette: flat solid colors, not gradients -- warm
// cream ground, one bold orange accent, 4 solid terracotta-to-orange
// "petal" bands -- the same Teenage-Engineering-inspired cream/orange
// treatment the Visualizer app uses, not the earlier purple
// night-garden scheme. This replaces this sim's shared green-
// monochrome LCD look -- this app's own visual personality, not a
// device-wide theme change. Every other app keeps the shared look
// from paramlist.rs's `ACCENT`. The Slint live panel
// (`slint_home_live.rs`'s `BloomPalette`) mirrors these same hues so
// the two renderers read as one design, not two. ---

/// Flat cream background fill, painted over the shared black clear at
/// the top of `draw` -- see `os.rs`'s per-frame `fb.clear(BLACK)`.
const BLOOM_BG: Rgb565 = Rgb565::new(18, 36, 17);
/// Near-black ink -- the title and this app's darkest, boldest text
/// (dark-on-cream, not light-on-dark like every other app's screen).
const BLOOM_TITLE: Rgb565 = Rgb565::new(4, 6, 2);
/// Bold orange -- this app's menu accent / status text.
const BLOOM_ACCENT: Rgb565 = Rgb565::new(11, 8, 1);
/// Muted warm taupe -- secondary/dim text.
const BLOOM_DIM: Rgb565 = Rgb565::new(5, 9, 4);
/// The selected menu row's highlight chip -- a solid orange fill (see
/// `draw`'s `draw_themed` call, which pairs this with cream text).
const BLOOM_CHIP_BG: Rgb565 = BLOOM_ACCENT;
/// The boundary circle's stroke -- one crisp flat line, no glow.
const BLOOM_RING: Rgb565 = Rgb565::new(17, 30, 11);
/// The spiral connecting the inner dots -- same flat line color.
const BLOOM_SPIRAL: Rgb565 = BLOOM_RING;
/// An outer dot (trigger bar) that's toggled off entirely.
const BLOOM_OUTER_INACTIVE: Rgb565 = Rgb565::new(26, 51, 23);
/// An outer dot that's active but hasn't just fired -- flat
/// terracotta, reusing `PETAL_BANDS[0]` rather than its own hue.
const BLOOM_OUTER_OFF: Rgb565 = PETAL_BANDS[0];
/// The flash an outer dot (and its line to center) makes the instant
/// it fires -- one flat bright amber, not a blended glow.
const BLOOM_OUTER_LIT: Rgb565 = Rgb565::new(31, 30, 7);
/// The flash an inner dot makes the instant it triggers a note.
const BLOOM_INNER_LIT: Rgb565 = BLOOM_OUTER_LIT;

/// The 4 solid "petal" bands an un-lit inner dot's ring falls into
/// (terracotta innermost/lowest note -> bold orange outermost/
/// highest), a flat color-block poster print rather than a smooth
/// per-pixel gradient.
const PETAL_BANDS: [Rgb565; 4] = [
    Rgb565::new(23, 25, 6), // terracotta
    Rgb565::new(26, 31, 6), // amber-orange
    Rgb565::new(28, 38, 7), // gold-orange
    Rgb565::new(31, 22, 4), // bold orange
];

/// An un-lit inner dot's resting color -- one of `PETAL_BANDS`,
/// picked by ring (`frac` = `dot_radius_frac`, 0 innermost/lowest
/// note, 1 outermost/highest). Quantized into flat bands, not a
/// smooth blend -- see the module palette doc comment.
fn bloom_petal_color(frac: f32) -> Rgb565 {
    let idx = (frac.clamp(0.0, 0.999) * PETAL_BANDS.len() as f32) as usize;
    PETAL_BANDS[idx.min(PETAL_BANDS.len() - 1)]
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    MasterBpm,
    /// Randomizes every one of the 8 shapes at once -- press-only,
    /// same "press knob2" idiom as the per-shape `Randomize`, just
    /// scoped to all of them. Lives in the Master Clock group since
    /// it isn't any one shape's own action.
    RandomizeAll,
    Running(usize),
    /// Applies a curated bundle (dot/outer counts, outer layout,
    /// speed curve, and all 3 offsets below) in one step -- a
    /// deliberate, named starting shape, unlike `Randomize`'s fully
    /// chaotic reassignment. See `PATTERN_PRESETS`.
    Pattern(usize),
    Dots(usize),
    OuterDots(usize),
    OuterActive(usize, usize),
    /// This trigger bar's fixed octave offset -- ignored while
    /// `OuterRandomOctave` is on for the same dot.
    OuterOctave(usize, usize),
    /// Picks a fresh random octave offset (within `MIN_OUTER_OCTAVE`.
    /// `MAX_OUTER_OCTAVE`) every time this trigger bar fires, instead
    /// of always using its fixed `OuterOctave`.
    OuterRandomOctave(usize, usize),
    /// Rotates the whole ring of outer trigger dots by this many
    /// turns -- changes which angles the inner dots cross (and so the
    /// rhythm) without touching dot/outer counts.
    OuterAngleOffset(usize),
    /// Staggers each inner dot's starting angle by `dot_index *
    /// this many turns` -- a fixed fan/pinwheel spread applied on
    /// top of whatever `RandomStart` would otherwise produce (0.0 for
    /// all, or a fresh random value per dot). See `reset_dot_phases`.
    DotAngleOffset(usize),
    /// Adds this many turns to every *odd*-indexed dot's starting
    /// angle (even dots unaffected) -- an alternating flip that
    /// produces a symmetric/radial look rather than a one-directional
    /// fan. Combines with `DotAngleOffset`. See `reset_dot_phases`.
    RingRotationOffset(usize),
    Scale(usize),
    /// Toggles Custom Scale Edit mode for this shape -- while on, the
    /// grid stops doing nothing (Bloom has no other grid use) and
    /// instead toggles this shape's 12 `custom_scale` degrees, pads
    /// 0-11 mapped straight to semitones. Same "takes over the grid"
    /// convention Sequencer's Grid Edit/Pad Perform use. Only
    /// meaningful once Scale is set to Custom.
    ScaleCustomEdit(usize),
    Root(usize),
    OctaveRange(usize),
    /// One of the 5 fixed `OCTAVE_TRANSPOSE_STEPS` stops.
    OctaveTranspose(usize),
    MinNote(usize),
    MaxNote(usize),
    /// While on, editing Min/Max Note moves the other bound by the
    /// same amount, keeping the range's width fixed.
    LinkRangeNotes(usize),
    Direction(usize),
    TempoSync(usize),
    Speed(usize),
    ClockMod(usize),
    Probability(usize),
    VelMin(usize),
    VelMax(usize),
    Engine(usize),
    Harmonics(usize),
    Timbre(usize),
    Decay(usize),
    Randomize(usize),
    /// Snaps every inner dot back to its start (12-o'clock, or a
    /// fresh random spread if `RandomStart` is on) on demand -- today
    /// that only happens automatically when a dot count changes (see
    /// `reset_dot_phases`); this exposes the same reset as its own
    /// action.
    Retrigger(usize),
    /// Off (default): every inner dot always restarts at the same
    /// 12-o'clock position -- the same shape unfolds identically
    /// every time a dot count changes or Retrigger fires. On: each
    /// dot gets its own random starting phase instead, so the same
    /// shape produces a different-looking (and different-sounding)
    /// pattern each time. See `reset_dot_phases`.
    RandomStart(usize),
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 1 + NUM_SHAPES;

struct ShapeParams {
    running: AtomicBool,
    /// Index into `PATTERN_PRESETS` -- purely a UI cursor/display
    /// value; applying a preset writes its bundle straight into this
    /// shape's other fields, same as `Randomize` does.
    pattern: AtomicUsize,
    dots: AtomicUsize,
    outer: AtomicUsize,
    outer_active: [AtomicBool; MAX_OUTER],
    /// See `Selection::OuterAngleOffset`/`DotAngleOffset`/
    /// `RingRotationOffset`.
    outer_angle_offset: AtomicF32,
    dot_angle_offset: AtomicF32,
    ring_rotation_offset: AtomicF32,
    /// Which formula `dot_speed_mult` uses for this shape's per-dot
    /// speed multiplier -- set by `Pattern`, not directly editable
    /// (keeps the menu from growing another whole row set for what's
    /// really just part of a preset's identity).
    speed_curve: AtomicU32,
    /// Fixed per-trigger-bar octave offset -- see `Selection::
    /// OuterOctave`.
    outer_octave: [AtomicI32; MAX_OUTER],
    /// See `Selection::OuterRandomOctave`.
    outer_random_octave: [AtomicBool; MAX_OUTER],
    scale: AtomicU32,
    /// This shape's own Custom Scale degree toggles (one per
    /// semitone) -- only read when `scale == custom_scale_index()`,
    /// see `resolved_scale_intervals`. Edited via the grid while
    /// `Selection::ScaleCustomEdit` is on for this shape.
    custom_scale: [AtomicBool; NUM_CUSTOM_DEGREES],
    root: AtomicU32,
    octave_range: AtomicU32,
    /// Index into `OCTAVE_TRANSPOSE_STEPS`.
    octave_transpose: AtomicUsize,
    min_note: AtomicI32,
    max_note: AtomicI32,
    link_range_notes: AtomicBool,
    direction: AtomicU32,
    tempo_sync: AtomicBool,
    speed_hz: AtomicF32,
    clock_mod: AtomicUsize,
    probability: AtomicF32,
    vel_min: AtomicF32,
    vel_max: AtomicF32,
    engine: AtomicU32,
    harmonics: AtomicF32,
    timbre: AtomicF32,
    decay: AtomicF32,
    /// Each inner dot's own raw (unwrapped) phase accumulator. All
    /// start at 0.0 (the same 12-o'clock spot), or a random spread
    /// if `random_start` is on, and spin at their own multiple of the
    /// base dot speed -- see `DOT_SPEED_STEP`.
    dot_phases: [AtomicF32; MAX_DOTS],
    /// See `Selection::RandomStart`.
    random_start: AtomicBool,
    last_fired_dot: AtomicUsize,
    last_fired_outer: AtomicUsize,
    /// This shape's real Pam's-assignable modulation targets --
    /// Speed, Direction, and the 3 pattern-shaping offsets. Each is
    /// added continuously into its own parameter every block (same
    /// additive convention every other modbus target in this build
    /// uses), so patching an LFO/clock into one actually sweeps it
    /// live, not just on the next Retrigger.
    ext_speed: Arc<AtomicF32>,
    ext_direction: Arc<AtomicF32>,
    ext_outer_angle_offset: Arc<AtomicF32>,
    ext_dot_angle_offset: Arc<AtomicF32>,
    ext_ring_rotation_offset: Arc<AtomicF32>,
}

impl ShapeParams {
    fn new(shape_num: usize, modbus: &ModBus) -> Self {
        Self {
            running: AtomicBool::new(false),
            pattern: AtomicUsize::new(0),
            dots: AtomicUsize::new(DEFAULT_DOTS),
            outer: AtomicUsize::new(DEFAULT_OUTER),
            outer_active: std::array::from_fn(|_| AtomicBool::new(true)),
            outer_angle_offset: AtomicF32::new(0.0),
            dot_angle_offset: AtomicF32::new(0.0),
            ring_rotation_offset: AtomicF32::new(0.0),
            speed_curve: AtomicU32::new(0),
            outer_octave: std::array::from_fn(|_| AtomicI32::new(0)),
            outer_random_octave: std::array::from_fn(|_| AtomicBool::new(false)),
            scale: AtomicU32::new(0),
            custom_scale: std::array::from_fn(|_| AtomicBool::new(false)),
            root: AtomicU32::new(0),
            octave_range: AtomicU32::new(DEFAULT_OCTAVE_RANGE),
            octave_transpose: AtomicUsize::new(DEFAULT_OCTAVE_TRANSPOSE_INDEX),
            min_note: AtomicI32::new(MIN_ABS_NOTE),
            max_note: AtomicI32::new(MAX_ABS_NOTE),
            link_range_notes: AtomicBool::new(false),
            direction: AtomicU32::new(0),
            tempo_sync: AtomicBool::new(false),
            speed_hz: AtomicF32::new(DEFAULT_SPEED_HZ),
            clock_mod: AtomicUsize::new(3),
            probability: AtomicF32::new(1.0),
            vel_min: AtomicF32::new(0.6),
            vel_max: AtomicF32::new(1.0),
            engine: AtomicU32::new(8),
            harmonics: AtomicF32::new(0.5),
            timbre: AtomicF32::new(0.5),
            decay: AtomicF32::new(0.4),
            dot_phases: std::array::from_fn(|_| AtomicF32::new(0.0)),
            random_start: AtomicBool::new(false),
            // usize::MAX, not 0 -- so nothing shows as "lit" before a
            // real trigger has ever fired.
            last_fired_dot: AtomicUsize::new(usize::MAX),
            last_fired_outer: AtomicUsize::new(usize::MAX),
            ext_speed: modbus.register(format!("Bloom {shape_num}: Speed")),
            ext_direction: modbus.register(format!("Bloom {shape_num}: Direction")),
            ext_outer_angle_offset: modbus.register(format!("Bloom {shape_num}: Outer Angle")),
            ext_dot_angle_offset: modbus.register(format!("Bloom {shape_num}: Dot Angle")),
            ext_ring_rotation_offset: modbus.register(format!("Bloom {shape_num}: Ring Rotation")),
        }
    }
}

struct Params {
    master_bpm: AtomicF32,
    shapes: [ShapeParams; NUM_SHAPES],
    /// This app's rendered mono output, republished every block for
    /// another app (Clouds) to tap -- see audio_bus.rs.
    bus_out: Arc<Mutex<Vec<f32>>>,
    /// This app's channel fader in the Mixer app, plus its own
    /// modulation input -- see mixer_bus.rs.
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Bloom", modbus);
        Self {
            master_bpm: AtomicF32::new(DEFAULT_BPM),
            shapes: std::array::from_fn(|i| ShapeParams::new(i + 1, modbus)),
            bus_out: audio_bus.register("Bloom"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct BloomApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    last_shape: usize,
    rng: u32,
    /// `Some(shape)` while Custom Scale Edit mode is active for that
    /// shape -- see `Selection::ScaleCustomEdit` and `tick()`'s
    /// early-return branch. `None` (the default) is normal menu/grid
    /// behavior (Bloom otherwise doesn't use the grid at all).
    custom_scale_edit: Option<usize>,
    prev_grid: [bool; 16],
}

impl BloomApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(24681);
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
            last_shape: 0,
            rng: seed | 1,
            custom_scale_edit: None,
            prev_grid: [false; 16],
        }
    }

    fn next_rand01(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    /// Restarts every inner dot at its 12-o'clock start, or -- if
    /// `Selection::RandomStart` is on for this shape -- gives each
    /// dot its own fresh random starting phase instead, so the same
    /// shape doesn't unfold into the identical pattern every time.
    /// Called whenever a dot count changes, since ring/angle
    /// assignments shift and a dot mid-orbit would otherwise jump.
    fn reset_dot_phases(&mut self, s: usize) {
        let random_start = self.params.shapes[s].random_start.load(Ordering::Relaxed);
        // Drawn unconditionally so this method never needs a second
        // borrow of `self` while `sp` is alive below.
        let random_phases: [f32; MAX_DOTS] = std::array::from_fn(|_| self.next_rand01());
        let sp = &self.params.shapes[s];
        // `DotAngleOffset`/`RingRotationOffset` are no longer baked in
        // here -- they're real Pam's-modulation targets now, applied
        // continuously every block (see `dot_angle_bias` in
        // `process`/`bloom_visual`/`draw`) so an LFO patched into one
        // actually sweeps live instead of only updating on the next
        // reset. This only ever resets the raw orbit position.
        for (d, p) in sp.dot_phases.iter().enumerate() {
            p.set(if random_start { random_phases[d] } else { 0.0 });
        }
        sp.last_fired_dot.store(usize::MAX, Ordering::Relaxed);
        sp.last_fired_outer.store(usize::MAX, Ordering::Relaxed);
    }

    /// Writes a curated `PATTERN_PRESETS` bundle into this shape and
    /// restarts its dots -- see `Selection::Pattern`.
    fn apply_pattern(&mut self, s: usize, idx: usize) {
        let idx = idx % PATTERN_PRESETS.len();
        let preset = &PATTERN_PRESETS[idx];
        let sp = &self.params.shapes[s];
        sp.pattern.store(idx, Ordering::Relaxed);
        sp.dots.store(preset.dots.clamp(MIN_DOTS, MAX_DOTS), Ordering::Relaxed);
        sp.outer.store(preset.outer.clamp(MIN_OUTER, MAX_OUTER), Ordering::Relaxed);
        for (o, active) in preset.outer_active.iter().enumerate() {
            sp.outer_active[o].store(*active, Ordering::Relaxed);
        }
        sp.speed_curve.store(preset.speed_curve, Ordering::Relaxed);
        sp.dot_angle_offset.set(preset.dot_angle_offset);
        sp.ring_rotation_offset.set(preset.ring_rotation_offset);
        sp.outer_angle_offset.set(preset.outer_angle_offset);
        self.reset_dot_phases(s);
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        if g == 0 {
            return vec![Selection::MasterBpm, Selection::RandomizeAll];
        }
        let s = g - 1;
        // Running comes first -- the master on/off for this shape.
        // Pattern right after it -- a curated starting shape (dots,
        // outer count/layout, speed curve, offsets) to build from,
        // distinct from Randomize's fully chaotic reassignment below.
        let mut v = vec![Selection::Running(s), Selection::Pattern(s), Selection::Dots(s), Selection::OuterDots(s)];
        let num_outer = self.params.shapes[s].outer.load(Ordering::Relaxed).clamp(MIN_OUTER, MAX_OUTER);
        for o in 0..num_outer {
            v.push(Selection::OuterActive(s, o));
            v.push(Selection::OuterOctave(s, o));
            v.push(Selection::OuterRandomOctave(s, o));
        }
        v.push(Selection::OuterAngleOffset(s));
        v.push(Selection::DotAngleOffset(s));
        v.push(Selection::RingRotationOffset(s));
        v.push(Selection::Scale(s));
        if self.params.shapes[s].scale.load(Ordering::Relaxed) == custom_scale_index() {
            v.push(Selection::ScaleCustomEdit(s));
        }
        v.push(Selection::Root(s));
        v.push(Selection::OctaveRange(s));
        v.push(Selection::OctaveTranspose(s));
        v.push(Selection::MinNote(s));
        v.push(Selection::MaxNote(s));
        v.push(Selection::LinkRangeNotes(s));
        v.push(Selection::Direction(s));
        v.push(Selection::TempoSync(s));
        if self.params.shapes[s].tempo_sync.load(Ordering::Relaxed) {
            v.push(Selection::ClockMod(s));
        } else {
            v.push(Selection::Speed(s));
        }
        v.push(Selection::Probability(s));
        v.push(Selection::VelMin(s));
        v.push(Selection::VelMax(s));
        v.push(Selection::Engine(s));
        v.push(Selection::Harmonics(s));
        v.push(Selection::Timbre(s));
        v.push(Selection::Decay(s));
        v.push(Selection::Randomize(s));
        v.push(Selection::Retrigger(s));
        v.push(Selection::RandomStart(s));
        v
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

    fn selection_shape(sel: Selection) -> Option<usize> {
        match sel {
            Selection::MasterBpm | Selection::RandomizeAll => None,
            Selection::Running(s)
            | Selection::Pattern(s)
            | Selection::Dots(s)
            | Selection::OuterDots(s)
            | Selection::OuterActive(s, _)
            | Selection::OuterOctave(s, _)
            | Selection::OuterRandomOctave(s, _)
            | Selection::OuterAngleOffset(s)
            | Selection::DotAngleOffset(s)
            | Selection::RingRotationOffset(s)
            | Selection::Scale(s)
            | Selection::ScaleCustomEdit(s)
            | Selection::Root(s)
            | Selection::OctaveRange(s)
            | Selection::OctaveTranspose(s)
            | Selection::MinNote(s)
            | Selection::MaxNote(s)
            | Selection::LinkRangeNotes(s)
            | Selection::Direction(s)
            | Selection::TempoSync(s)
            | Selection::Speed(s)
            | Selection::ClockMod(s)
            | Selection::Probability(s)
            | Selection::VelMin(s)
            | Selection::VelMax(s)
            | Selection::Engine(s)
            | Selection::Harmonics(s)
            | Selection::Timbre(s)
            | Selection::Decay(s)
            | Selection::Randomize(s)
            | Selection::Retrigger(s)
            | Selection::RandomStart(s) => Some(s),
        }
    }

    fn current_shape(&self, rows: &[Row]) -> usize {
        match rows.get(self.list.selected) {
            Some(Row::Group(g)) if *g >= 1 => *g - 1,
            Some(Row::Leaf(sel)) => Self::selection_shape(*sel).unwrap_or(self.last_shape),
            _ => self.last_shape,
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::MasterBpm => "BPM".into(),
            Selection::RandomizeAll => "Randomize All".into(),
            Selection::Running(_) => "Running".into(),
            Selection::Pattern(_) => "Pattern".into(),
            Selection::Dots(_) => "Dots".into(),
            Selection::OuterDots(_) => "Outer Dots".into(),
            Selection::OuterActive(_, o) => format!("Outer {} Active", o + 1),
            Selection::OuterOctave(_, o) => format!("Outer {} Octave", o + 1),
            Selection::OuterRandomOctave(_, o) => format!("Outer {} Rand Oct", o + 1),
            Selection::OuterAngleOffset(_) => "Outer Angle Offset".into(),
            Selection::DotAngleOffset(_) => "Dot Angle Offset".into(),
            Selection::RingRotationOffset(_) => "Ring Rotation".into(),
            Selection::Scale(_) => "Scale".into(),
            Selection::ScaleCustomEdit(_) => "Edit Custom Scale".into(),
            Selection::Root(_) => "Root".into(),
            Selection::OctaveRange(_) => "Octave Range".into(),
            Selection::OctaveTranspose(_) => "Octave Transpose".into(),
            Selection::MinNote(_) => "Min Note".into(),
            Selection::MaxNote(_) => "Max Note".into(),
            Selection::LinkRangeNotes(_) => "Link Range Notes".into(),
            Selection::Direction(_) => "Direction".into(),
            Selection::TempoSync(_) => "Tempo Sync".into(),
            Selection::Speed(_) => "Speed".into(),
            Selection::ClockMod(_) => "Clock Mod".into(),
            Selection::Probability(_) => "Probability".into(),
            Selection::VelMin(_) => "Vel Min".into(),
            Selection::VelMax(_) => "Vel Max".into(),
            Selection::Engine(_) => "Engine".into(),
            Selection::Harmonics(_) => "Harmonics".into(),
            Selection::Timbre(_) => "Timbre".into(),
            Selection::Decay(_) => "Decay".into(),
            Selection::Randomize(_) => "Randomize".into(),
            Selection::Retrigger(_) => "Retrigger".into(),
            Selection::RandomStart(_) => "Random Start".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::MasterBpm => format!("{:.0}", self.params.master_bpm.get()),
            Selection::RandomizeAll => "press knob2".into(),
            Selection::Running(s) => {
                if self.params.shapes[s].running.load(Ordering::Relaxed) { "running".into() } else { "stopped".into() }
            }
            Selection::Pattern(s) => PATTERN_NAMES[self.params.shapes[s].pattern.load(Ordering::Relaxed) % PATTERN_NAMES.len()].to_string(),
            Selection::Dots(s) => format!("{}", self.params.shapes[s].dots.load(Ordering::Relaxed)),
            Selection::OuterDots(s) => format!("{}", self.params.shapes[s].outer.load(Ordering::Relaxed)),
            Selection::OuterActive(s, o) => {
                if self.params.shapes[s].outer_active[o].load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::OuterOctave(s, o) => format!("{:+}", self.params.shapes[s].outer_octave[o].load(Ordering::Relaxed)),
            Selection::OuterRandomOctave(s, o) => {
                if self.params.shapes[s].outer_random_octave[o].load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::OuterAngleOffset(s) => format!("{:+.0}%", self.params.shapes[s].outer_angle_offset.get() * 100.0),
            Selection::DotAngleOffset(s) => format!("{:+.0}%", self.params.shapes[s].dot_angle_offset.get() * 100.0),
            Selection::RingRotationOffset(s) => format!("{:+.0}%", self.params.shapes[s].ring_rotation_offset.get() * 100.0),
            Selection::Scale(s) => {
                let scale = self.params.shapes[s].scale.load(Ordering::Relaxed);
                if scale == custom_scale_index() {
                    "Custom".into()
                } else {
                    SCALE_TYPES[scale as usize % SCALE_TYPES.len()].0.to_string()
                }
            }
            Selection::ScaleCustomEdit(s) => {
                if self.custom_scale_edit == Some(s) { "ON -- press knob1 to exit".into() } else { "off".into() }
            }
            Selection::Root(s) => ROOT_NAMES[self.params.shapes[s].root.load(Ordering::Relaxed) as usize % 12].to_string(),
            Selection::OctaveRange(s) => format!("{} oct", self.params.shapes[s].octave_range.load(Ordering::Relaxed)),
            Selection::OctaveTranspose(s) => {
                let idx = self.params.shapes[s].octave_transpose.load(Ordering::Relaxed) % OCTAVE_TRANSPOSE_STEPS.len();
                format!("{:+}", OCTAVE_TRANSPOSE_STEPS[idx])
            }
            Selection::MinNote(s) => format!("{}", self.params.shapes[s].min_note.load(Ordering::Relaxed)),
            Selection::MaxNote(s) => format!("{}", self.params.shapes[s].max_note.load(Ordering::Relaxed)),
            Selection::LinkRangeNotes(s) => {
                if self.params.shapes[s].link_range_notes.load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::Direction(s) => {
                let idx = self.params.shapes[s].direction.load(Ordering::Relaxed) as usize % DIRECTION_NAMES.len();
                DIRECTION_NAMES[idx].to_string()
            }
            Selection::TempoSync(s) => {
                if self.params.shapes[s].tempo_sync.load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
            Selection::Speed(s) => format!("{:.2} Hz", self.params.shapes[s].speed_hz.get()),
            Selection::ClockMod(s) => {
                CLOCK_MODS[self.params.shapes[s].clock_mod.load(Ordering::Relaxed) % CLOCK_MODS.len()].0.to_string()
            }
            Selection::Probability(s) => format!("{:.0}%", self.params.shapes[s].probability.get() * 100.0),
            Selection::VelMin(s) => format!("{:.2}", self.params.shapes[s].vel_min.get()),
            Selection::VelMax(s) => format!("{:.2}", self.params.shapes[s].vel_max.get()),
            Selection::Engine(s) => ENGINE_NAMES[self.params.shapes[s].engine.load(Ordering::Relaxed) as usize % 24].to_string(),
            Selection::Harmonics(s) => format!("{:.2}", self.params.shapes[s].harmonics.get()),
            Selection::Timbre(s) => format!("{:.2}", self.params.shapes[s].timbre.get()),
            Selection::Decay(s) => format!("{:.2}", self.params.shapes[s].decay.get()),
            Selection::Randomize(_) => "press knob2".into(),
            Selection::Retrigger(_) => "press knob2".into(),
            Selection::RandomStart(s) => {
                if self.params.shapes[s].random_start.load(Ordering::Relaxed) { "on".into() } else { "off".into() }
            }
        }
    }

    fn group_summary(&self, g: usize) -> String {
        if g == 0 {
            return format!("{:.0} BPM", self.params.master_bpm.get());
        }
        let s = g - 1;
        let running = if self.params.shapes[s].running.load(Ordering::Relaxed) { "" } else { " (stopped)" };
        format!(
            "{} dots, {} outer{}",
            self.params.shapes[s].dots.load(Ordering::Relaxed),
            self.params.shapes[s].outer.load(Ordering::Relaxed),
            running
        )
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::MasterBpm => {
                let next = (self.params.master_bpm.get() + accelerate(delta) * sensitivity * 2.0).clamp(MIN_BPM, MAX_BPM);
                self.params.master_bpm.set(next);
            }
            Selection::Running(s) => self.params.shapes[s].running.store(delta > 0, Ordering::Relaxed),
            Selection::Pattern(s) => {
                let cur = self.params.shapes[s].pattern.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(PATTERN_NAMES.len() as i32) as usize;
                self.apply_pattern(s, next);
            }
            Selection::Dots(s) => {
                let cur = self.params.shapes[s].dots.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_DOTS as i32, MAX_DOTS as i32);
                self.params.shapes[s].dots.store(next as usize, Ordering::Relaxed);
                self.reset_dot_phases(s);
            }
            Selection::OuterDots(s) => {
                let cur = self.params.shapes[s].outer.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_OUTER as i32, MAX_OUTER as i32);
                self.params.shapes[s].outer.store(next as usize, Ordering::Relaxed);
                self.reset_dot_phases(s);
            }
            Selection::OuterActive(s, o) => self.params.shapes[s].outer_active[o].store(delta > 0, Ordering::Relaxed),
            Selection::OuterOctave(s, o) => {
                let cur = self.params.shapes[s].outer_octave[o].load(Ordering::Relaxed);
                let next = (cur + step).clamp(MIN_OUTER_OCTAVE, MAX_OUTER_OCTAVE);
                self.params.shapes[s].outer_octave[o].store(next, Ordering::Relaxed);
            }
            Selection::OuterRandomOctave(s, o) => self.params.shapes[s].outer_random_octave[o].store(delta > 0, Ordering::Relaxed),
            Selection::OuterAngleOffset(s) => bump(&self.params.shapes[s].outer_angle_offset, delta, sensitivity, -1.0, 1.0),
            Selection::DotAngleOffset(s) => {
                bump(&self.params.shapes[s].dot_angle_offset, delta, sensitivity, -1.0, 1.0);
                // Immediate feedback, same convention `Dots`/`OuterDots`
                // already use -- otherwise the change wouldn't show
                // until the next unrelated Retrigger/count change.
                self.reset_dot_phases(s);
            }
            Selection::RingRotationOffset(s) => {
                bump(&self.params.shapes[s].ring_rotation_offset, delta, sensitivity, -1.0, 1.0);
                self.reset_dot_phases(s);
            }
            Selection::Scale(s) => {
                // +1 for the Custom slot past the real presets -- see
                // `custom_scale_index`.
                let cur = self.params.shapes[s].scale.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(SCALE_TYPES.len() as i32 + 1);
                self.params.shapes[s].scale.store(next as u32, Ordering::Relaxed);
            }
            Selection::ScaleCustomEdit(s) => self.custom_scale_edit = if delta > 0 { Some(s) } else { None },
            Selection::Root(s) => {
                let cur = self.params.shapes[s].root.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(12);
                self.params.shapes[s].root.store(next as u32, Ordering::Relaxed);
            }
            Selection::OctaveRange(s) => {
                let cur = self.params.shapes[s].octave_range.load(Ordering::Relaxed) as i32;
                let next = (cur + step).clamp(MIN_OCTAVE_RANGE as i32, MAX_OCTAVE_RANGE as i32);
                self.params.shapes[s].octave_range.store(next as u32, Ordering::Relaxed);
            }
            Selection::OctaveTranspose(s) => {
                let cur = self.params.shapes[s].octave_transpose.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(OCTAVE_TRANSPOSE_STEPS.len() as i32);
                self.params.shapes[s].octave_transpose.store(next as usize, Ordering::Relaxed);
            }
            Selection::MinNote(s) => {
                let sp = &self.params.shapes[s];
                let cur = sp.min_note.load(Ordering::Relaxed);
                let next = (cur + step).clamp(MIN_ABS_NOTE, sp.max_note.load(Ordering::Relaxed));
                sp.min_note.store(next, Ordering::Relaxed);
                if sp.link_range_notes.load(Ordering::Relaxed) {
                    let moved = next - cur;
                    let next_max = (sp.max_note.load(Ordering::Relaxed) + moved).clamp(next, MAX_ABS_NOTE);
                    sp.max_note.store(next_max, Ordering::Relaxed);
                }
            }
            Selection::MaxNote(s) => {
                let sp = &self.params.shapes[s];
                let cur = sp.max_note.load(Ordering::Relaxed);
                let next = (cur + step).clamp(sp.min_note.load(Ordering::Relaxed), MAX_ABS_NOTE);
                sp.max_note.store(next, Ordering::Relaxed);
                if sp.link_range_notes.load(Ordering::Relaxed) {
                    let moved = next - cur;
                    let next_min = (sp.min_note.load(Ordering::Relaxed) + moved).clamp(MIN_ABS_NOTE, next);
                    sp.min_note.store(next_min, Ordering::Relaxed);
                }
            }
            Selection::LinkRangeNotes(s) => self.params.shapes[s].link_range_notes.store(delta > 0, Ordering::Relaxed),
            Selection::Direction(s) => {
                let cur = self.params.shapes[s].direction.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(DIRECTION_NAMES.len() as i32);
                self.params.shapes[s].direction.store(next as u32, Ordering::Relaxed);
            }
            Selection::TempoSync(s) => self.params.shapes[s].tempo_sync.store(delta > 0, Ordering::Relaxed),
            Selection::Speed(s) => bump(&self.params.shapes[s].speed_hz, delta, sensitivity, MIN_SPEED_HZ, MAX_SPEED_HZ),
            Selection::ClockMod(s) => {
                let cur = self.params.shapes[s].clock_mod.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(CLOCK_MODS.len() as i32);
                self.params.shapes[s].clock_mod.store(next as usize, Ordering::Relaxed);
            }
            Selection::Probability(s) => bump(&self.params.shapes[s].probability, delta, sensitivity, 0.0, 1.0),
            Selection::VelMin(s) => bump(&self.params.shapes[s].vel_min, delta, sensitivity, 0.0, 1.0),
            Selection::VelMax(s) => bump(&self.params.shapes[s].vel_max, delta, sensitivity, 0.0, 1.0),
            Selection::Engine(s) => {
                let cur = self.params.shapes[s].engine.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(24);
                self.params.shapes[s].engine.store(next as u32, Ordering::Relaxed);
            }
            Selection::Harmonics(s) => bump(&self.params.shapes[s].harmonics, delta, sensitivity, 0.0, 1.0),
            Selection::Timbre(s) => bump(&self.params.shapes[s].timbre, delta, sensitivity, 0.0, 1.0),
            Selection::Decay(s) => bump(&self.params.shapes[s].decay, delta, sensitivity, 0.0, 1.0),
            Selection::RandomStart(s) => self.params.shapes[s].random_start.store(delta > 0, Ordering::Relaxed),
            Selection::Randomize(_) | Selection::RandomizeAll | Selection::Retrigger(_) => {} // action only fires on press -- see reset()
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::MasterBpm => self.params.master_bpm.set(DEFAULT_BPM),
            Selection::Pattern(s) => self.apply_pattern(s, 0),
            Selection::OuterAngleOffset(s) => self.params.shapes[s].outer_angle_offset.set(0.0),
            Selection::DotAngleOffset(s) => {
                self.params.shapes[s].dot_angle_offset.set(0.0);
                self.reset_dot_phases(s);
            }
            Selection::RingRotationOffset(s) => {
                self.params.shapes[s].ring_rotation_offset.set(0.0);
                self.reset_dot_phases(s);
            }
            Selection::Dots(s) => {
                self.params.shapes[s].dots.store(DEFAULT_DOTS, Ordering::Relaxed);
                self.reset_dot_phases(s);
            }
            Selection::OuterDots(s) => {
                self.params.shapes[s].outer.store(DEFAULT_OUTER, Ordering::Relaxed);
                self.reset_dot_phases(s);
            }
            Selection::OctaveRange(s) => self.params.shapes[s].octave_range.store(DEFAULT_OCTAVE_RANGE, Ordering::Relaxed),
            Selection::OctaveTranspose(s) => self.params.shapes[s].octave_transpose.store(DEFAULT_OCTAVE_TRANSPOSE_INDEX, Ordering::Relaxed),
            Selection::MinNote(s) => self.params.shapes[s].min_note.store(MIN_ABS_NOTE, Ordering::Relaxed),
            Selection::MaxNote(s) => self.params.shapes[s].max_note.store(MAX_ABS_NOTE, Ordering::Relaxed),
            Selection::LinkRangeNotes(s) => self.params.shapes[s].link_range_notes.store(false, Ordering::Relaxed),
            Selection::OuterOctave(s, o) => self.params.shapes[s].outer_octave[o].store(0, Ordering::Relaxed),
            Selection::OuterRandomOctave(s, o) => self.params.shapes[s].outer_random_octave[o].store(false, Ordering::Relaxed),
            Selection::ScaleCustomEdit(_) => self.custom_scale_edit = None,
            Selection::Speed(s) => self.params.shapes[s].speed_hz.set(DEFAULT_SPEED_HZ),
            Selection::ClockMod(s) => self.params.shapes[s].clock_mod.store(3, Ordering::Relaxed),
            Selection::Probability(s) => self.params.shapes[s].probability.set(1.0),
            Selection::VelMin(s) => self.params.shapes[s].vel_min.set(0.6),
            Selection::VelMax(s) => self.params.shapes[s].vel_max.set(1.0),
            Selection::Harmonics(s) => self.params.shapes[s].harmonics.set(0.5),
            Selection::Timbre(s) => self.params.shapes[s].timbre.set(0.5),
            Selection::Decay(s) => self.params.shapes[s].decay.set(0.4),
            Selection::Randomize(s) => self.randomize(s),
            Selection::RandomizeAll => {
                for s in 0..NUM_SHAPES {
                    self.randomize(s);
                }
            }
            Selection::Retrigger(s) => self.reset_dot_phases(s),
            Selection::RandomStart(s) => self.params.shapes[s].random_start.store(false, Ordering::Relaxed),
            // No sensible single default: the master switch shouldn't
            // get flipped by a "reset" press, and these have no one
            // obvious default.
            Selection::Running(_)
            | Selection::OuterActive(_, _)
            | Selection::Scale(_)
            | Selection::Root(_)
            | Selection::Direction(_)
            | Selection::TempoSync(_)
            | Selection::Engine(_) => {}
        }
    }

    /// Reassigns most of one shape's parameters at once for quick
    /// exploration -- deliberately leaves Running alone, so
    /// randomizing doesn't also start something running.
    fn randomize(&mut self, s: usize) {
        let dots = MIN_DOTS + (self.next_rand01() * (MAX_DOTS - MIN_DOTS + 1) as f32) as usize;
        let outer = MIN_OUTER + (self.next_rand01() * (MAX_OUTER - MIN_OUTER + 1) as f32) as usize;
        let outer_active: [bool; MAX_OUTER] = std::array::from_fn(|_| self.next_rand01() < 0.8);
        let outer_octave: [i32; MAX_OUTER] =
            std::array::from_fn(|_| MIN_OUTER_OCTAVE + (self.next_rand01() * (MAX_OUTER_OCTAVE - MIN_OUTER_OCTAVE + 1) as f32) as i32);
        let outer_random_octave: [bool; MAX_OUTER] = std::array::from_fn(|_| self.next_rand01() < 0.2);
        // Bounded to the real presets only -- never randomly lands on
        // the Custom slot (which would likely be silent: an untouched
        // custom scale starts with every degree off).
        let scale = (self.next_rand01() * SCALE_TYPES.len() as f32) as u32;
        let root = (self.next_rand01() * 12.0) as u32;
        let octave_range = MIN_OCTAVE_RANGE + (self.next_rand01() * (MAX_OCTAVE_RANGE - MIN_OCTAVE_RANGE + 1) as f32) as u32;
        let direction = (self.next_rand01() * DIRECTION_NAMES.len() as f32) as u32;
        let speed_hz = MIN_SPEED_HZ + self.next_rand01() * (MAX_SPEED_HZ - MIN_SPEED_HZ);
        let clock_mod = (self.next_rand01() * CLOCK_MODS.len() as f32) as usize;
        let probability = 0.4 + self.next_rand01() * 0.6;
        let v0 = self.next_rand01();
        let v1 = self.next_rand01();
        let engine = (self.next_rand01() * 24.0) as u32;
        let harmonics = self.next_rand01();
        let timbre = self.next_rand01();
        let decay = self.next_rand01();
        let speed_curve = (self.next_rand01() * 3.0) as u32;
        // Moderate ranges (not the full -1..1 a manual knob allows) --
        // Randomize already stacks a fully random start phase on top
        // (see `random_start` below), so a wild offset on top of that
        // would mostly just add noise rather than a legible shape.
        let dot_angle_offset = (self.next_rand01() - 0.5) * 0.4;
        let ring_rotation_offset = (self.next_rand01() - 0.5) * 0.6;
        let outer_angle_offset = (self.next_rand01() - 0.5) * 0.5;

        let sp = &self.params.shapes[s];
        sp.dots.store(dots.clamp(MIN_DOTS, MAX_DOTS), Ordering::Relaxed);
        sp.outer.store(outer.clamp(MIN_OUTER, MAX_OUTER), Ordering::Relaxed);
        for (o, active) in outer_active.iter().enumerate() {
            sp.outer_active[o].store(*active, Ordering::Relaxed);
            sp.outer_octave[o].store(outer_octave[o], Ordering::Relaxed);
            sp.outer_random_octave[o].store(outer_random_octave[o], Ordering::Relaxed);
        }
        sp.scale.store(scale, Ordering::Relaxed);
        sp.root.store(root, Ordering::Relaxed);
        sp.octave_range.store(octave_range.clamp(MIN_OCTAVE_RANGE, MAX_OCTAVE_RANGE), Ordering::Relaxed);
        sp.direction.store(direction, Ordering::Relaxed);
        sp.speed_hz.set(speed_hz);
        sp.clock_mod.store(clock_mod, Ordering::Relaxed);
        sp.probability.set(probability);
        sp.vel_min.set(v0.min(v1));
        sp.vel_max.set(v0.max(v1).max(v0.min(v1) + 0.05));
        sp.engine.store(engine, Ordering::Relaxed);
        sp.harmonics.set(harmonics);
        sp.timbre.set(timbre);
        sp.decay.set(decay);
        sp.speed_curve.store(speed_curve, Ordering::Relaxed);
        sp.dot_angle_offset.set(dot_angle_offset);
        sp.ring_rotation_offset.set(ring_rotation_offset);
        sp.outer_angle_offset.set(outer_angle_offset);
        // Randomize should always hand back a fresh-looking pattern,
        // not the same 12-o'clock bunch every time -- see
        // `Selection::RandomStart`'s doc comment for the complaint
        // this fixes.
        self.params.shapes[s].random_start.store(true, Ordering::Relaxed);
        self.reset_dot_phases(s);
    }
}

impl BloomApp {
    /// Real, windowed `(name, value, is_group)` rows -- mirrors this
    /// app's own `draw()` row-building, exposed for an alternate
    /// renderer (a live Slint screen) instead of drawn.
    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    let name = if *g == 0 { "Master Clock".to_string() } else { format!("Shape {g}") };
                    let summary = if *g == 0 { self.group_summary(*g) } else {
                        let sp = &self.params.shapes[*g - 1];
                        format!("{}n / {}g{}", sp.dots.load(Ordering::Relaxed),
                            sp.outer.load(Ordering::Relaxed),
                            if sp.running.load(Ordering::Relaxed) { "" } else { " · off" })
                    };
                    (format!("{arrow} {name}"), summary, true)
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

    /// The real orbiting-dots clock face for whichever shape is
    /// currently browsed -- same data `draw()`'s own circle sketch
    /// reads, exposed as plain center-relative coordinates for an
    /// alternate renderer instead of drawn directly.
    pub(crate) fn bloom_visual(&self) -> crate::app::BloomVisual {
        let rows = self.visible_rows();
        let shape = self.current_shape(&rows);
        let sp = &self.params.shapes[shape];
        let direction = modulated_direction(sp);
        let num_dots = sp.dots.load(Ordering::Relaxed).clamp(MIN_DOTS, MAX_DOTS);
        let num_outer = sp.outer.load(Ordering::Relaxed).clamp(MIN_OUTER, MAX_OUTER);
        let last_fired_dot = sp.last_fired_dot.load(Ordering::Relaxed);
        let last_fired_outer = sp.last_fired_outer.load(Ordering::Relaxed);
        let running = sp.running.load(Ordering::Relaxed);
        let pattern_name = PATTERN_NAMES[sp.pattern.load(Ordering::Relaxed) % PATTERN_NAMES.len()].to_string();
        let outer_angle_offset = sp.outer_angle_offset.get() + sp.ext_outer_angle_offset.get();
        let dot_angle_offset = sp.dot_angle_offset.get() + sp.ext_dot_angle_offset.get();
        let ring_rotation_offset = sp.ring_rotation_offset.get() + sp.ext_ring_rotation_offset.get();

        let outer = (0..num_outer)
            .map(|o| {
                let active = sp.outer_active[o].load(Ordering::Relaxed);
                let lit = o == last_fired_outer;
                let angle = (outer_angle_turns(o, num_outer) + outer_angle_offset) * TAU - FRAC_PI_2;
                (angle.cos(), angle.sin(), active, lit)
            })
            .collect();

        let inner = (0..num_dots)
            .map(|d| {
                let frac = dot_radius_frac(d, num_dots);
                let bias = dot_angle_bias(d, dot_angle_offset, ring_rotation_offset);
                let base = display_phase(sp.dot_phases[d].get() + bias, direction);
                let angle = base * TAU - FRAC_PI_2;
                (frac * angle.cos(), frac * angle.sin(), d == last_fired_dot, frac)
            })
            .collect();

        crate::app::BloomVisual { shape_index: shape, running, pattern_name, outer, inner }
    }

    /// Custom Scale Edit mode's entire input handling, replacing the
    /// normal menu-navigation body while active (see
    /// `custom_scale_edit`) -- pads 0-11 (row-major, same physical
    /// layout every other grid-editing app uses) each toggle one
    /// semitone of this shape's `custom_scale` degrees on a fresh
    /// press; pads 12-15 are unused (only 12 semitones exist).
    /// Pressing knob1 exits back to the normal menu, same convention
    /// Sequencer's Grid Edit uses.
    fn tick_custom_scale_edit(&mut self, shape: usize, input: &Input) {
        if input.knob1_press {
            self.custom_scale_edit = None;
        }
        for i in 0..NUM_CUSTOM_DEGREES {
            if input.grid[i] && !self.prev_grid[i] {
                let sp = &self.params.shapes[shape];
                let cur = sp.custom_scale[i].load(Ordering::Relaxed);
                sp.custom_scale[i].store(!cur, Ordering::Relaxed);
            }
        }
        self.prev_grid = input.grid;
    }
}

impl App for BloomApp {
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
        crate::app::SlintExtra::Bloom(self.bloom_visual())
    }

    /// Reflects/controls whichever shape is currently focused (the
    /// one the right-hand panel is showing) -- there's no single
    /// "the" Running for this app since it's per-shape, so the
    /// OS-level Start/Stop button (F3) acts on whichever one you're
    /// actually looking at, same target `Selection::Running` in the
    /// menu already edits.
    fn running(&self) -> Option<bool> {
        let rows = self.visible_rows();
        let shape = self.current_shape(&rows);
        Some(self.params.shapes[shape].running.load(Ordering::Relaxed))
    }

    fn toggle_running(&mut self) {
        let rows = self.visible_rows();
        let shape = self.current_shape(&rows);
        let cur = self.params.shapes[shape].running.load(Ordering::Relaxed);
        self.params.shapes[shape].running.store(!cur, Ordering::Relaxed);
    }

    fn tick(&mut self, input: &Input) {
        if let Some(shape) = self.custom_scale_edit {
            self.tick_custom_scale_edit(shape, input);
            return;
        }

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
        self.last_shape = self.current_shape(&rows);
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(BloomProcessor {
            params: Arc::clone(&self.params),
            shapes: std::array::from_fn(|i| ShapeRuntime::new(i as u32)),
            mono_buf: Vec::new(),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        // This app's own botanical palette, painted over the shared
        // black clear -- see the palette constants' doc comment.
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(BLOOM_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, BLOOM_TITLE);
        Text::new("Bloom", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, BLOOM_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, BLOOM_DIM);

        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    let name = if *g == 0 { "Master Clock".to_string() } else { format!("Shape {g}") };
                    (format!("{arrow} {name}"), self.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", self.leaf_name(*sel)), self.leaf_value(*sel)),
            })
            .collect();
        // `BLOOM_CHIP_BG` is a solid orange fill (not a translucent
        // tint), so the selected row's own text must be cream
        // (`BLOOM_BG`) to read against it, not `BLOOM_ACCENT` (which
        // -- now that the chip itself *is* that same orange -- would
        // be invisible orange-on-orange.
        self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, BLOOM_BG, BLOOM_DIM, BLOOM_CHIP_BG);

        // --- Right: the fixed circle; static outer dots on its
        // boundary (the trigger references); inner dots each
        // orbiting on their own fixed concentric ring (pitch). ---
        let shape = self.current_shape(&rows);
        let sp = &self.params.shapes[shape];
        let cx = 500;
        let cy = 170;
        let radius = 110.0f32;
        let direction = modulated_direction(sp);
        let num_dots = sp.dots.load(Ordering::Relaxed).clamp(MIN_DOTS, MAX_DOTS);
        let num_outer = sp.outer.load(Ordering::Relaxed).clamp(MIN_OUTER, MAX_OUTER);
        let last_fired_dot = sp.last_fired_dot.load(Ordering::Relaxed);
        let last_fired_outer = sp.last_fired_outer.load(Ordering::Relaxed);
        let running = sp.running.load(Ordering::Relaxed);
        let outer_angle_offset = sp.outer_angle_offset.get() + sp.ext_outer_angle_offset.get();
        let dot_angle_offset = sp.dot_angle_offset.get() + sp.ext_dot_angle_offset.get();
        let ring_rotation_offset = sp.ring_rotation_offset.get() + sp.ext_ring_rotation_offset.get();

        Circle::new(Point::new(cx - radius as i32, cy - radius as i32), (radius * 2.0) as u32)
            .into_styled(PrimitiveStyle::with_stroke(BLOOM_RING, 1))
            .draw(fb)
            .ok();

        // Outer dots -- static trigger references on the boundary.
        for o in 0..num_outer {
            let active = sp.outer_active[o].load(Ordering::Relaxed);
            let lit = o == last_fired_outer;
            let angle = (outer_angle_turns(o, num_outer) + outer_angle_offset) * TAU - FRAC_PI_2;
            let tip = Point::new(cx + (radius * angle.cos()) as i32, cy + (radius * angle.sin()) as i32);

            let color = if !active {
                BLOOM_OUTER_INACTIVE
            } else if lit {
                BLOOM_OUTER_LIT
            } else {
                BLOOM_OUTER_OFF
            };
            if active {
                // A permanent, subtle reference line from center to
                // this active outer dot -- always on (not just a
                // flash the instant it fires), thin and low-key so it
                // reads as a quiet guide line, not an event.
                Line::new(Point::new(cx, cy), tip).into_styled(PrimitiveStyle::with_stroke(BLOOM_RING, 1)).draw(fb).ok();
            }
            let dd: i32 = if lit { 8 } else { 5 };
            Circle::new(Point::new(tip.x - dd / 2, tip.y - dd / 2), dd as u32)
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(fb)
                .ok();
        }

        // Inner dots -- the notes, each on its own fixed ring. Since
        // ring radius grows with index and each dot has drifted to
        // its own angle, connecting them innermost-to-outermost in
        // index order traces a spiraling line through the current
        // moment -- purely visual, no bearing on triggering.
        let mut dot_points: [Point; MAX_DOTS] = [Point::new(cx, cy); MAX_DOTS];
        for d in 0..num_dots {
            let r = radius * dot_radius_frac(d, num_dots);
            let bias = dot_angle_bias(d, dot_angle_offset, ring_rotation_offset);
            let base = display_phase(sp.dot_phases[d].get() + bias, direction);
            let angle = base * TAU - FRAC_PI_2;
            dot_points[d] = Point::new(cx + (r * angle.cos()) as i32, cy + (r * angle.sin()) as i32);
        }

        for d in 1..num_dots {
            Line::new(dot_points[d - 1], dot_points[d])
                .into_styled(PrimitiveStyle::with_stroke(BLOOM_SPIRAL, 1))
                .draw(fb)
                .ok();
        }

        // Denser patterns (more inner dots -- up to 32 now, was 16)
        // shrink a step so a fully-populated shape reads as a woven
        // web instead of a smear of overlapping circles, same
        // adjustment the Slint panel makes.
        let dense = num_dots > 16;
        for d in 0..num_dots {
            let lit = d == last_fired_dot;
            let color = if lit { BLOOM_INNER_LIT } else { bloom_petal_color(dot_radius_frac(d, num_dots)) };
            let dd: i32 = match (lit, dense) {
                (true, false) => 9,
                (false, false) => 5,
                (true, true) => 6,
                (false, true) => 3,
            };
            let p = dot_points[d];
            Circle::new(Point::new(p.x - dd / 2, p.y - dd / 2), dd as u32)
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(fb)
                .ok();
        }

        let status = if running { "running" } else { "stopped" };
        Text::new(&format!("Shape {} -- {}", shape + 1, status), Point::new(420, 300), accent).draw(fb).ok();

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

/// How many notes a single shape can sound at once. Simultaneous
/// triggers are common here -- most strikingly right when the dots
/// periodically re-converge and several can cross an outer dot in
/// the same block -- so one voice per shape (retriggering, cutting
/// off whatever was still decaying) would lose most of that. 8 gives
/// real polyphony without turning every shape into a 16-voice choir.
const NUM_VOICES: usize = 8;

/// One of a shape's polyphonic voices. Engine/Harmonics/Timbre/Decay
/// are shared per shape (one "instrument"); only note, velocity and
/// the one-shot trigger pulse are per-voice.
struct PolyVoice {
    voice: PlaitsVoice,
    voice_buf: Vec<f32>,
    note: f32,
    velocity: f32,
    trigger: bool,
    /// Set to the shape's trigger counter each time this voice is
    /// (re)allocated -- lets `trigger_note` steal whichever voice was
    /// used longest ago (simple round-robin voice stealing).
    last_used: u64,
}

impl PolyVoice {
    fn new() -> Self {
        Self { voice: PlaitsVoice::new(), voice_buf: Vec::new(), note: 60.0, velocity: 1.0, trigger: false, last_used: 0 }
    }
}

struct ShapeRuntime {
    voices: [PolyVoice; NUM_VOICES],
    voice_gen: u64,
    rng: u32,
}

impl ShapeRuntime {
    fn new(seed: u32) -> Self {
        Self {
            voices: std::array::from_fn(|_| PolyVoice::new()),
            voice_gen: 0,
            rng: 0x9E3779B9 ^ (seed.wrapping_mul(0x85EBCA6B) | 1),
        }
    }

    fn next_rand01(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    /// Allocates a new note to whichever voice was least recently
    /// triggered, so overlapping/simultaneous notes each get their
    /// own decay instead of all retriggering a single shared voice.
    fn trigger_note(&mut self, note: f32, velocity: f32) {
        self.voice_gen += 1;
        let idx = self.voices.iter().enumerate().min_by_key(|(_, v)| v.last_used).map(|(i, _)| i).unwrap_or(0);
        let v = &mut self.voices[idx];
        v.note = note;
        v.velocity = velocity;
        v.trigger = true;
        v.last_used = self.voice_gen;
    }
}

struct BloomProcessor {
    params: Arc<Params>,
    shapes: [ShapeRuntime; NUM_SHAPES],
    mono_buf: Vec<f32>,
}

impl AudioProcessor for BloomProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        self.mono_buf.clear();
        self.mono_buf.resize(frames, 0.0);
        let dt = frames as f32 / sample_rate;
        let master_bpm = self.params.master_bpm.get().max(1.0);

        // Counts how many voices actually contributed audible signal
        // this block, across every shape's pool -- used below as a
        // headroom divisor, same normalize-by-active-voices approach
        // plaits.rs's poly mode already uses (see its `headroom`
        // there), just measured from rendered peak rather than a held
        // gate, since these are one-shot decaying triggers rather than
        // held notes. Without this, a re-convergence moment (several
        // dots crossing active outer dots close together, possibly
        // across more than one running shape) sums unboundedly and
        // clips no matter how low Vel Min/Max or the Mixer's Bloom
        // fader are set -- neither actually reduces how many voices
        // stack, only how loud each one is.
        let mut active_voices: usize = 0;

        for s in 0..NUM_SHAPES {
            let sp = &self.params.shapes[s];
            let rt = &mut self.shapes[s];
            let running = sp.running.load(Ordering::Relaxed);

            if running {
                let base_speed = if sp.tempo_sync.load(Ordering::Relaxed) {
                    let ratio = CLOCK_MODS[sp.clock_mod.load(Ordering::Relaxed) % CLOCK_MODS.len()].1;
                    (master_bpm / 60.0) * ratio
                } else {
                    sp.speed_hz.get()
                };
                let speed = (base_speed + sp.ext_speed.get()).max(0.0);
                let direction = (sp.direction.load(Ordering::Relaxed) as i32 + sp.ext_direction.get().round() as i32)
                    .rem_euclid(DIRECTION_NAMES.len() as i32) as u32;
                let dir_sign = if direction == 1 { -1.0 } else { 1.0 };
                let probability = sp.probability.get();
                let num_dots = sp.dots.load(Ordering::Relaxed).clamp(MIN_DOTS, MAX_DOTS);
                let num_outer = sp.outer.load(Ordering::Relaxed).clamp(MIN_OUTER, MAX_OUTER);
                // Resolved once per shape per block, not per crossing
                // -- doesn't change mid-block, and a Custom Scale with
                // many degrees toggled would otherwise re-filter the
                // same 12 atomics on every single crossing event.
                let intervals = resolved_scale_intervals(sp);
                let octave_transpose_semitones =
                    OCTAVE_TRANSPOSE_STEPS[sp.octave_transpose.load(Ordering::Relaxed) % OCTAVE_TRANSPOSE_STEPS.len()];
                let min_note = sp.min_note.load(Ordering::Relaxed);
                let max_note = sp.max_note.load(Ordering::Relaxed).max(min_note);

                // Each inner dot has its own phase and its own speed
                // multiple of the base dot rate -- all seeded at 0.0
                // (see ShapeParams::new), so they start bunched
                // together at 12 o'clock and gradually spread apart,
                // periodically re-converging since the multipliers
                // are simple rational steps. A note fires whenever a
                // dot's orbit crosses an *active* outer dot's fixed
                // (static) angle.
                let speed_curve = sp.speed_curve.load(Ordering::Relaxed);
                let outer_angle_offset = sp.outer_angle_offset.get() + sp.ext_outer_angle_offset.get();
                let dot_angle_offset = sp.dot_angle_offset.get() + sp.ext_dot_angle_offset.get();
                let ring_rotation_offset = sp.ring_rotation_offset.get() + sp.ext_ring_rotation_offset.get();
                for d in 0..num_dots {
                    let mult = dot_speed_mult(d, speed_curve);
                    let rate = speed * DOT_SPEED_MULT * mult * dir_sign;
                    let prev_raw = sp.dot_phases[d].get();
                    let new_raw = prev_raw + rate * dt;
                    sp.dot_phases[d].set(new_raw);

                    // Equivalent to adding this dot's current bias to
                    // its own phase before checking the crossing --
                    // subtracting it from the target instead means
                    // `dot_phases` itself never needs the (live,
                    // possibly modulated) bias baked in, see
                    // `dot_angle_bias`'s doc comment.
                    let bias = dot_angle_bias(d, dot_angle_offset, ring_rotation_offset);
                    for o in 0..num_outer {
                        if !sp.outer_active[o].load(Ordering::Relaxed) {
                            continue;
                        }
                        let target = outer_angle_turns(o, num_outer) + outer_angle_offset - bias;
                        if crosses(prev_raw, new_raw, target) && rt.next_rand01() < probability {
                            let t = dot_radius_frac(d, num_dots);
                            let root = sp.root.load(Ordering::Relaxed);
                            let oct_range = sp.octave_range.load(Ordering::Relaxed).max(MIN_OCTAVE_RANGE);
                            let scale_len = intervals.len().max(1);
                            let total_positions = scale_len * oct_range as usize;
                            let degree_pos = ((t * total_positions as f32) as usize).min(total_positions.saturating_sub(1));

                            // This trigger bar's own octave offset --
                            // either its fixed `OuterOctave`, or (if
                            // `OuterRandomOctave` is on) a fresh random
                            // pick within the same range every time it
                            // fires. See `Selection::OuterOctave`/
                            // `OuterRandomOctave`.
                            let outer_octaves = if sp.outer_random_octave[o].load(Ordering::Relaxed) {
                                let span = (MAX_OUTER_OCTAVE - MIN_OUTER_OCTAVE + 1) as f32;
                                MIN_OUTER_OCTAVE + (rt.next_rand01() * span) as i32
                            } else {
                                sp.outer_octave[o].load(Ordering::Relaxed)
                            };

                            let raw_note = note_for_position(degree_pos, &intervals, root) + octave_transpose_semitones + outer_octaves * 12;
                            let note = raw_note.clamp(min_note, max_note) as f32;

                            let vmin = sp.vel_min.get().min(sp.vel_max.get());
                            let vmax = sp.vel_min.get().max(sp.vel_max.get());
                            let velocity = vmin + rt.next_rand01() * (vmax - vmin);
                            rt.trigger_note(note, velocity);
                            sp.last_fired_dot.store(d, Ordering::Relaxed);
                            sp.last_fired_outer.store(o, Ordering::Relaxed);
                        }
                    }
                }
            }

            let engine = sp.engine.load(Ordering::Relaxed) as i32;
            let harmonics = sp.harmonics.get();
            let timbre = sp.timbre.get();
            let decay = sp.decay.get();

            // Render every voice in the shape's pool -- most are
            // silently decaying or idle most of the time, but this is
            // what lets several notes (e.g. a re-convergence moment)
            // actually overlap instead of one stealing the shape's
            // only voice out from under another.
            for v in rt.voices.iter_mut() {
                let params = PlaitsParams {
                    engine,
                    note: v.note,
                    harmonics,
                    timbre,
                    morph: 0.5,
                    decay,
                    lpg_colour: 0.5,
                    trigger: v.trigger,
                };
                v.voice_buf.clear();
                v.voice_buf.resize(frames, 0.0);
                v.voice.render(&mut v.voice_buf, sample_rate, &params);
                v.trigger = false; // one-shot pulse, consumed this block

                let peak = v.voice_buf.iter().fold(0.0f32, |a, s| a.max(s.abs()));
                if peak > 1e-4 {
                    active_voices += 1;
                }

                let gain = v.velocity;
                for (m, sample) in self.mono_buf.iter_mut().zip(v.voice_buf.iter()) {
                    *m += *sample * gain;
                }
            }
        }

        let headroom = active_voices.max(1) as f32;
        for m in self.mono_buf.iter_mut() {
            *m /= headroom;
        }

        {
            let mut bus_out = self.params.bus_out.lock().unwrap();
            bus_out.clear();
            bus_out.extend_from_slice(&self.mono_buf);
        }

        // The Mixer app's channel fader for this app -- applied only
        // to what reaches the device, not to `bus_out` above (see
        // plaits.rs for the same pattern).
        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        for (frame, sample) in buffer.chunks_mut(channels).zip(self.mono_buf.iter()) {
            for out in frame.iter_mut() {
                *out = *sample * mix_level;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the "nothing appears to move" report: with
    /// a shape's Running on, each inner dot's raw phase must actually
    /// advance across process() calls, and crossing an active outer
    /// dot's angle must fire a trigger with real (in-range) indices
    /// rather than the stale default.
    #[test]
    fn running_shape_advances_phases_and_fires() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.shapes[0].running.store(true, Ordering::Relaxed);
        params.shapes[0].speed_hz.set(2.0); // fast, so the test doesn't need many blocks

        let mut proc = BloomProcessor { params: Arc::clone(&params), shapes: std::array::from_fn(|i| ShapeRuntime::new(i as u32)), mono_buf: Vec::new() };

        let dot0_start = params.shapes[0].dot_phases[0].get();
        let gen_start = proc.shapes[0].voice_gen;

        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..200 {
            proc.process(&mut buffer, 2, 48000.0);
        }

        let dot0_end = params.shapes[0].dot_phases[0].get();

        assert!((dot0_end - dot0_start).abs() > 0.01, "dot phase did not advance: {dot0_start} -> {dot0_end}");
        assert!(proc.shapes[0].voice_gen > gen_start, "expected at least one trigger over 200 blocks at 2 Hz");

        let fired_dot = params.shapes[0].last_fired_dot.load(Ordering::Relaxed);
        let fired_outer = params.shapes[0].last_fired_outer.load(Ordering::Relaxed);
        assert!(fired_dot < MAX_DOTS, "last_fired_dot left at sentinel/out-of-range: {fired_dot}");
        assert!(fired_outer < MAX_OUTER, "last_fired_outer left at sentinel/out-of-range: {fired_outer}");
    }

    /// Before any trigger has occurred, nothing should read as "lit"
    /// -- this is the sentinel-default bug that made dot/outer 0 look
    /// permanently flashed from frame one.
    #[test]
    fn nothing_lit_before_first_trigger() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Params::new(&modbus, &audio_bus, &mixer_bus);
        let last_fired_dot = params.shapes[0].last_fired_dot.load(Ordering::Relaxed);
        let last_fired_outer = params.shapes[0].last_fired_outer.load(Ordering::Relaxed);
        assert!((0..MAX_DOTS).all(|d| d != last_fired_dot));
        assert!((0..MAX_OUTER).all(|o| o != last_fired_outer));
    }

    /// Changing either dot count must reset every inner dot back to
    /// its 12-o'clock start (and clear the "lit" markers), per the
    /// "when adding or subtracting dots, reset the other dots"
    /// requirement.
    #[test]
    fn changing_dot_count_resets_phases() {
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let mut app = BloomApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);

        app.params.shapes[0].dot_phases[3].set(0.42);
        app.params.shapes[0].last_fired_dot.store(3, Ordering::Relaxed);
        app.params.shapes[0].last_fired_outer.store(1, Ordering::Relaxed);

        app.edit(Selection::Dots(0), 1);

        assert_eq!(app.params.shapes[0].dot_phases[3].get(), 0.0);
        assert_eq!(app.params.shapes[0].last_fired_dot.load(Ordering::Relaxed), usize::MAX);
        assert_eq!(app.params.shapes[0].last_fired_outer.load(Ordering::Relaxed), usize::MAX);

        app.params.shapes[0].dot_phases[2].set(0.77);
        app.edit(Selection::OuterDots(0), 1);
        assert_eq!(app.params.shapes[0].dot_phases[2].get(), 0.0);
    }

    /// Two overlapping notes must sound as two distinct voices, not
    /// one stealing/retriggering a single shared voice -- the
    /// "only one note triggers at a time" report.
    #[test]
    fn overlapping_triggers_use_distinct_voices() {
        let mut rt = ShapeRuntime::new(0);
        rt.trigger_note(60.0, 1.0);
        let first_idx = rt.voices.iter().position(|v| v.trigger).expect("first trigger should mark a voice");

        rt.trigger_note(67.0, 0.8);
        let triggered: Vec<usize> = rt.voices.iter().enumerate().filter(|(_, v)| v.trigger).map(|(i, _)| i).collect();

        assert_eq!(triggered.len(), 2, "both notes should still show as triggered (nothing consumed them yet): {triggered:?}");
        assert!(triggered.contains(&first_idx), "the first voice must not have been overwritten by the second trigger");

        let notes: Vec<f32> = triggered.iter().map(|&i| rt.voices[i].note).collect();
        assert!(notes.contains(&60.0) && notes.contains(&67.0), "both distinct notes must be present: {notes:?}");
    }

    /// Regression test for "turned everything down but it's still
    /// distorting a lot more than expected": many notes converging at
    /// once (worst case -- every voice in every shape's pool, same
    /// note, same block) must not sum to an amplitude that grows with
    /// voice count. Neither Vel Min/Max nor the Mixer's Bloom fader
    /// can fix that on their own -- they scale every voice's gain
    /// uniformly, not the number of voices stacking -- so this
    /// active-voice headroom division (mirroring plaits.rs's poly
    /// mode) is what actually keeps a re-convergence moment bounded.
    #[test]
    fn many_simultaneous_voices_stay_headroom_normalized() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        for s in 0..NUM_SHAPES {
            params.shapes[s].vel_min.set(1.0);
            params.shapes[s].vel_max.set(1.0);
        }

        let mut proc = BloomProcessor { params: Arc::clone(&params), shapes: std::array::from_fn(|i| ShapeRuntime::new(i as u32)), mono_buf: Vec::new() };

        for s in 0..NUM_SHAPES {
            for _ in 0..NUM_VOICES {
                proc.shapes[s].trigger_note(60.0, 1.0);
            }
        }

        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..20 {
            proc.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().cloned().fold(0.0f32, |a, x| a.max(x.abs())));
        }

        assert!(peak < 1.5, "peak grew with voice count instead of staying headroom-normalized: {peak}");
    }

    /// The freshly-constructed, untouched `Params` -- the exact state
    /// a brand new app instance starts in -- must have every shape
    /// default to stopped. With N apps all loaded (and someday
    /// possibly far more), an app that defaults to *running* is a
    /// real risk on its own, not just an inconvenience.
    #[test]
    fn every_shape_defaults_to_stopped() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Params::new(&modbus, &audio_bus, &mixer_bus);
        for s in 0..NUM_SHAPES {
            assert!(!params.shapes[s].running.load(Ordering::Relaxed), "shape {s} must start stopped, not running");
        }
    }

    fn new_app() -> BloomApp {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(1.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        BloomApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus)
    }

    /// A Custom Scale with specific degrees toggled must resolve to
    /// exactly those degrees (in order), and an untouched (all-off)
    /// Custom Scale must fall back to a single root-only interval
    /// rather than an empty slice that would divide by zero.
    #[test]
    fn custom_scale_resolves_to_its_toggled_degrees() {
        let app = new_app();
        let sp = &app.params.shapes[0];
        sp.scale.store(custom_scale_index(), Ordering::Relaxed);

        assert_eq!(resolved_scale_intervals(sp), vec![0], "an all-off custom scale must fall back to a single root-only interval, not empty");

        sp.custom_scale[0].store(true, Ordering::Relaxed);
        sp.custom_scale[4].store(true, Ordering::Relaxed);
        sp.custom_scale[7].store(true, Ordering::Relaxed);
        assert_eq!(resolved_scale_intervals(sp), vec![0, 4, 7], "expected exactly the 3 toggled degrees, in order");
    }

    /// Pad presses while `ScaleCustomEdit` is active for a shape must
    /// toggle that shape's `custom_scale` degrees (pads 0-11 mapped
    /// straight to semitones), and knob1 must exit back to the normal
    /// menu -- the same "takes over the grid" contract Sequencer's
    /// Grid Edit already guarantees for its own mode.
    #[test]
    fn custom_scale_edit_mode_toggles_degrees_via_the_grid_and_exits_on_knob1() {
        let mut app = new_app();
        app.custom_scale_edit = Some(2);

        let mut grid = [false; 16];
        grid[5] = true;
        app.tick(&Input { grid, ..Default::default() });
        assert!(app.params.shapes[2].custom_scale[5].load(Ordering::Relaxed), "pad 5 should have toggled degree 5 on");

        // Release and a fresh press toggles it back off.
        app.tick(&Input { grid: [false; 16], ..Default::default() });
        app.tick(&Input { grid, ..Default::default() });
        assert!(!app.params.shapes[2].custom_scale[5].load(Ordering::Relaxed), "a second fresh press should toggle it back off");

        assert!(app.custom_scale_edit.is_some(), "must still be in edit mode before knob1 is pressed");
        app.tick(&Input { knob1_press: true, ..Default::default() });
        assert_eq!(app.custom_scale_edit, None, "knob1 press must exit Custom Scale Edit mode");
    }

    /// Octave Transpose must shift every triggered note by exactly
    /// that many semitones -- verified by comparing the note a fixed
    /// crossing produces at `+0` vs. the `+24` stop.
    #[test]
    fn octave_transpose_shifts_triggered_notes_by_exactly_that_many_semitones() {
        let trigger_once = |octave_transpose_index: usize| -> f32 {
            let modbus = ModBus::new();
            let audio_bus = AudioBus::new();
            let mixer_bus = MixerBus::new();
            let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
            params.shapes[0].running.store(true, Ordering::Relaxed);
            params.shapes[0].speed_hz.set(4.0);
            params.shapes[0].octave_transpose.store(octave_transpose_index, Ordering::Relaxed);
            let mut proc = BloomProcessor { params: Arc::clone(&params), shapes: std::array::from_fn(|i| ShapeRuntime::new(i as u32)), mono_buf: Vec::new() };
            let mut buffer = vec![0.0f32; 512 * 2];
            for _ in 0..400 {
                proc.process(&mut buffer, 2, 48000.0);
                let fired = params.shapes[0].last_fired_dot.load(Ordering::Relaxed);
                if fired != usize::MAX {
                    let idx = proc.shapes[0].voices.iter().enumerate().max_by_key(|(_, v)| v.last_used).map(|(i, _)| i).unwrap();
                    return proc.shapes[0].voices[idx].note;
                }
            }
            panic!("expected at least one trigger over 400 blocks");
        };

        let base = trigger_once(DEFAULT_OCTAVE_TRANSPOSE_INDEX); // +0
        let up_two_octaves = trigger_once(4); // OCTAVE_TRANSPOSE_STEPS[4] == +24
        assert!(
            (up_two_octaves - base - 24.0).abs() < 0.01,
            "expected the +24 stop to land exactly 24 semitones above +0: base={base}, +24={up_two_octaves}"
        );
    }

    /// Min/Max Note must clamp every triggered note into range, even
    /// when the shape's own pitch mapping (Root/Octave Range/Octave
    /// Transpose) would otherwise put it outside -- verified with a
    /// deliberately narrow window.
    #[test]
    fn min_max_note_clamps_triggered_notes_into_range() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.shapes[0].running.store(true, Ordering::Relaxed);
        params.shapes[0].speed_hz.set(4.0);
        params.shapes[0].octave_range.store(MAX_OCTAVE_RANGE, Ordering::Relaxed); // wide native range
        params.shapes[0].min_note.store(60, Ordering::Relaxed);
        params.shapes[0].max_note.store(61, Ordering::Relaxed); // deliberately narrow
        let mut proc = BloomProcessor { params: Arc::clone(&params), shapes: std::array::from_fn(|i| ShapeRuntime::new(i as u32)), mono_buf: Vec::new() };

        let mut buffer = vec![0.0f32; 512 * 2];
        let mut saw_a_trigger = false;
        for _ in 0..400 {
            proc.process(&mut buffer, 2, 48000.0);
            if params.shapes[0].last_fired_dot.load(Ordering::Relaxed) != usize::MAX {
                saw_a_trigger = true;
                for v in proc.shapes[0].voices.iter() {
                    if v.note != 60.0 {
                        // A voice that's never been triggered still
                        // holds its `PolyVoice::new()` default (60.0)
                        // -- only check ones this shape could plausibly
                        // have just written.
                        assert!((60.0..=61.0).contains(&v.note), "triggered note {} escaped the 60..=61 Min/Max Note window", v.note);
                    }
                }
            }
        }
        assert!(saw_a_trigger, "expected at least one trigger over 400 blocks");
    }

    /// A trigger bar's fixed `OuterOctave` must shift only notes
    /// crossing *that* bar, by exactly that many octaves -- proven by
    /// giving two outer dots different fixed octaves and confirming
    /// both real values appear over enough blocks to cross each.
    #[test]
    fn outer_octave_offsets_apply_per_trigger_bar() {
        let modbus = ModBus::new();
        let audio_bus = AudioBus::new();
        let mixer_bus = MixerBus::new();
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.shapes[0].running.store(true, Ordering::Relaxed);
        params.shapes[0].speed_hz.set(3.0);
        params.shapes[0].outer.store(2, Ordering::Relaxed);
        params.shapes[0].outer_octave[0].store(0, Ordering::Relaxed);
        params.shapes[0].outer_octave[1].store(2, Ordering::Relaxed); // +2 octaves = +24 semitones
        let mut proc = BloomProcessor { params: Arc::clone(&params), shapes: std::array::from_fn(|i| ShapeRuntime::new(i as u32)), mono_buf: Vec::new() };

        let mut notes_by_outer: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..1500 {
            proc.process(&mut buffer, 2, 48000.0);
            let outer = params.shapes[0].last_fired_outer.load(Ordering::Relaxed);
            if outer < 2 {
                let idx = proc.shapes[0].voices.iter().enumerate().max_by_key(|(_, v)| v.last_used).map(|(i, _)| i).unwrap();
                notes_by_outer[outer].push(proc.shapes[0].voices[idx].note);
            }
        }
        assert!(!notes_by_outer[0].is_empty() && !notes_by_outer[1].is_empty(), "expected both outer dots to fire at least once over 1500 blocks");
        let avg = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!(
            (avg(&notes_by_outer[1]) - avg(&notes_by_outer[0]) - 24.0).abs() < 6.0,
            "outer dot 1 (+2 oct) should average ~24 semitones above outer dot 0 (+0 oct): {} vs {}",
            avg(&notes_by_outer[1]),
            avg(&notes_by_outer[0])
        );
    }

    /// Retrigger must reset a shape's dot phases on demand, the same
    /// way changing a dot count already does automatically -- proven
    /// by advancing a running shape's phase, then confirming
    /// `Selection::Retrigger` zeroes it back out.
    #[test]
    fn retrigger_resets_dot_phases_on_demand() {
        let mut app = new_app();
        app.params.shapes[0].dot_phases[0].set(1.75);
        app.params.shapes[0].last_fired_dot.store(3, Ordering::Relaxed);
        app.reset(Selection::Retrigger(0));
        assert_eq!(app.params.shapes[0].dot_phases[0].get(), 0.0, "Retrigger must reset the phase back to 0.0");
        assert_eq!(app.params.shapes[0].last_fired_dot.load(Ordering::Relaxed), usize::MAX, "Retrigger must also clear the lit marker");
    }

    /// Random Start off (the default) must keep the exact old
    /// behavior -- every dot snaps back to 0.0 (12 o'clock), the same
    /// spot every time -- so a shape with it off still unfolds
    /// identically after every Retrigger, exactly as before this
    /// Preset index 0 ("Spiral") must be a true no-op against every
    /// `ShapeParams::new` default -- selecting it should never change
    /// a freshly-constructed shape's behavior, since it documents
    /// "back to the original look."
    #[test]
    fn spiral_preset_matches_every_fresh_shape_default() {
        let mut app = new_app();
        app.apply_pattern(0, 0);
        let sp = &app.params.shapes[0];
        assert_eq!(sp.dots.load(Ordering::Relaxed), DEFAULT_DOTS);
        assert_eq!(sp.outer.load(Ordering::Relaxed), DEFAULT_OUTER);
        assert_eq!(sp.speed_curve.load(Ordering::Relaxed), 0);
        assert_eq!(sp.dot_angle_offset.get(), 0.0);
        assert_eq!(sp.ring_rotation_offset.get(), 0.0);
        assert_eq!(sp.outer_angle_offset.get(), 0.0);
    }

    /// Applying a non-Spiral preset must actually change the shape's
    /// geometry (dots/outer/offsets) in one step, not just record
    /// which preset is selected.
    #[test]
    fn applying_a_pattern_changes_shape_geometry() {
        let mut app = new_app();
        app.apply_pattern(0, 3); // "Mandala"
        let sp = &app.params.shapes[0];
        assert_eq!(sp.dots.load(Ordering::Relaxed), PATTERN_PRESETS[3].dots);
        assert_eq!(sp.outer.load(Ordering::Relaxed), PATTERN_PRESETS[3].outer);
        assert_eq!(sp.ring_rotation_offset.get(), PATTERN_PRESETS[3].ring_rotation_offset);
        assert_eq!(sp.pattern.load(Ordering::Relaxed), 3);
    }

    /// Dot Angle Offset is the direct fix for "just spirals" -- with
    /// it set, dots must land at genuinely different angles from each
    /// other, not all bunched at the same spot the way the plain
    /// default does. It's also a real Pam's-modulation target now
    /// (applied continuously, not baked into `dot_phases` at reset --
    /// see `dot_angle_bias`'s doc comment), so this checks the bias
    /// helper directly rather than the raw phase accumulator.
    #[test]
    fn dot_angle_offset_spreads_dots_to_distinct_starting_angles() {
        let b0 = dot_angle_bias(0, 0.1, 0.0);
        let b1 = dot_angle_bias(1, 0.1, 0.0);
        let b2 = dot_angle_bias(2, 0.1, 0.0);
        assert_ne!(b0, b1);
        assert_ne!(b1, b2);
        assert!((b1 - b0 - 0.1).abs() < 1e-6, "each dot should be staggered by exactly one offset step from the last");
    }

    /// Ring Rotation Offset must only touch odd-indexed dots (the
    /// "alternating flip" this control is documented to do), leaving
    /// even-indexed dots unaffected.
    #[test]
    fn ring_rotation_offset_only_flips_odd_dots() {
        assert_eq!(dot_angle_bias(0, 0.0, 0.5), 0.0, "even dot 0 must be untouched");
        assert_eq!(dot_angle_bias(1, 0.0, 0.5), 0.5, "odd dot 1 must get the full flip");
        assert_eq!(dot_angle_bias(2, 0.0, 0.5), 0.0, "even dot 2 must be untouched");
    }

    /// Dot Angle Offset is a real Pam's target -- an externally
    /// patched value (`ext_dot_angle_offset`, standing in for an LFO)
    /// must visibly move the rendered dot position immediately,
    /// without needing a Retrigger/reset first. `bloom_visual` is what
    /// both live renderers actually read every frame, so this is the
    /// real end-to-end check that modulation isn't silently a no-op
    /// between resets.
    #[test]
    fn dot_angle_offset_modulation_moves_the_live_visual_without_a_reset() {
        let mut app = new_app();
        app.params.shapes[0].dots.store(4, Ordering::Relaxed);
        app.reset_dot_phases(0);
        let before = app.bloom_visual().inner[2];
        app.params.shapes[0].ext_dot_angle_offset.set(0.2);
        let after = app.bloom_visual().inner[2];
        assert_ne!((before.0, before.1), (after.0, after.1), "modulating Dot Angle Offset should move dot 2's rendered position without any reset");
    }

    /// Outer Angle Offset must actually rotate which raw phase value
    /// fires a trigger bar -- shifting the target by a fixed amount
    /// Direction is a real Pam's target too -- an external value
    /// (standing in for e.g. a step sequencer stepping through
    /// Forward/Reverse/Ping-Pong) must add into the base Direction
    /// index and wrap, the same additive-modulation convention every
    /// other target in this build uses.
    #[test]
    fn direction_modulation_adds_and_wraps() {
        let sp_direction = |app: &BloomApp| modulated_direction(&app.params.shapes[0]);
        let mut app = new_app();
        assert_eq!(sp_direction(&app), 0, "Forward by default");
        app.params.shapes[0].ext_direction.set(1.0);
        assert_eq!(sp_direction(&app), 1, "external +1 should select Reverse");
        app.params.shapes[0].ext_direction.set(3.0);
        assert_eq!(sp_direction(&app), 0, "external +3 should wrap back to Forward (3 % 3 == 0)");
    }

    /// Every one of the 5 Pam's-assignable Bloom targets (per shape)
    /// must actually be registered on the shared ModBus -- this is
    /// literally what makes them selectable from Pam's own target
    /// list (`ModBus::names`, queried live -- see pams.rs).
    #[test]
    fn all_5_pams_targets_are_registered_per_shape() {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(1.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let _app = BloomApp::new(sensitivity, nav_speed, Arc::clone(&modbus), audio_bus, mixer_bus);

        let names = modbus.names();
        for expected in ["Speed", "Direction", "Outer Angle", "Dot Angle", "Ring Rotation"] {
            let full = format!("Bloom 1: {expected}");
            assert!(names.contains(&full), "expected modbus target {full:?}, got {names:?}");
        }
        assert!(!names.iter().any(|n| n.contains("Bloom 1: Level")), "Level should no longer be a Bloom Pam's target");
    }

    /// should shift exactly where `crosses` reports a hit.
    #[test]
    fn outer_angle_offset_shifts_the_real_trigger_target() {
        let num_outer = 4;
        let base_target = outer_angle_turns(1, num_outer);
        assert!(crosses(base_target - 0.01, base_target + 0.01, base_target));

        let offset = 0.2;
        let shifted_target = base_target + offset;
        // The un-shifted crossing window must NOT catch the
        // offset-shifted target -- proving the offset actually moved
        // where a hit registers, not just relabeled the same spot.
        assert!(!crosses(base_target - 0.01, base_target + 0.01, shifted_target));
        assert!(crosses(shifted_target - 0.01, shifted_target + 0.01, shifted_target));
    }

    /// `dot_speed_mult`'s Clustered curve (2) is documented to group
    /// dots in 4s sharing nearly the same speed -- confirm dots 0-3
    /// All 50 patterns must have distinct names and every field within
    /// its real valid range -- a generated table is exactly the kind
    /// of data an off-by-one in the generating formula could quietly
    /// push out of bounds without ever panicking at the call site
    /// (`apply_pattern` clamps), so this checks the source data
    /// itself, not just that applying it survives.
    #[test]
    fn all_50_patterns_are_named_uniquely_and_stay_in_range() {
        assert_eq!(PATTERN_NAMES.len(), 50);
        assert_eq!(PATTERN_PRESETS.len(), 50);
        let mut names: Vec<&str> = PATTERN_NAMES.to_vec();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 50, "every pattern name must be unique");
        for (i, p) in PATTERN_PRESETS.iter().enumerate() {
            assert!((MIN_DOTS..=MAX_DOTS).contains(&p.dots), "pattern {i} ({}) dots {} out of range", PATTERN_NAMES[i], p.dots);
            assert!((MIN_OUTER..=MAX_OUTER).contains(&p.outer), "pattern {i} ({}) outer {} out of range", PATTERN_NAMES[i], p.outer);
            assert!(p.speed_curve <= 2, "pattern {i} ({}) speed_curve {} unknown", PATTERN_NAMES[i], p.speed_curve);
            assert!(p.outer_active[..p.outer].iter().any(|&a| a), "pattern {i} ({}) has no active outer dots -- it would never trigger anything", PATTERN_NAMES[i]);
        }
    }

    /// Every one of the 50 must actually apply and run a block without
    /// panicking -- the real end-to-end guarantee, beyond just the
    /// source data being in-range.
    #[test]
    fn every_pattern_applies_and_runs_without_panicking() {
        let mut app = new_app();
        let mut proc = BloomProcessor { params: Arc::clone(&app.params), shapes: std::array::from_fn(|i| ShapeRuntime::new(i as u32)), mono_buf: Vec::new() };
        let mut buffer = vec![0.0f32; 128 * 2];
        for idx in 0..PATTERN_PRESETS.len() {
            app.apply_pattern(0, idx);
            app.params.shapes[0].running.store(true, Ordering::Relaxed);
            for _ in 0..3 {
                proc.process(&mut buffer, 2, 48000.0);
            }
        }
    }

    /// share one multiple and dots 4-7 share a different one.
    #[test]
    fn clustered_speed_curve_groups_dots_in_fours() {
        assert_eq!(dot_speed_mult(0, 2), dot_speed_mult(3, 2), "dots 0-3 must share the same cluster speed");
        assert_ne!(dot_speed_mult(3, 2), dot_speed_mult(4, 2), "dot 4 must be a different cluster than dot 3");
    }

    #[test]
    fn random_start_off_keeps_every_dot_at_the_same_start() {
        let mut app = new_app();
        for i in 0..4 {
            app.params.shapes[0].dot_phases[i].set(0.42);
        }
        app.reset(Selection::Retrigger(0));
        for i in 0..4 {
            assert_eq!(app.params.shapes[0].dot_phases[i].get(), 0.0, "Random Start is off by default -- every dot should reset to 0.0");
        }
    }

    /// The actual fix for "the same maddening pattern over and over
    /// again": with Random Start on, a Retrigger must give at least
    /// some dots a different starting phase than plain 0.0 -- not the
    /// same 12-o'clock bunch every time.
    #[test]
    fn random_start_on_spreads_dot_phases_instead_of_bunching_at_zero() {
        let mut app = new_app();
        app.edit(Selection::RandomStart(0), 1);
        assert!(app.params.shapes[0].random_start.load(Ordering::Relaxed));
        app.reset_dot_phases(0);
        let any_nonzero = (0..MAX_DOTS).any(|i| app.params.shapes[0].dot_phases[i].get() != 0.0);
        assert!(any_nonzero, "Random Start on should give at least one dot a non-zero starting phase");
    }

    /// Randomize is the "give me a fresh pattern" action -- it must
    /// leave Random Start on afterward so the *next* Retrigger (or
    /// dot-count change) doesn't quietly snap back to the boring
    /// 12-o'clock start either.
    #[test]
    fn randomize_turns_on_random_start() {
        let mut app = new_app();
        assert!(!app.params.shapes[0].random_start.load(Ordering::Relaxed));
        app.reset(Selection::Randomize(0));
        assert!(app.params.shapes[0].random_start.load(Ordering::Relaxed), "Randomize should turn Random Start on so future retriggers stay varied");
    }

    /// Randomize All must reassign every one of the 8 shapes, not
    /// just one -- verified by giving every shape the exact same
    /// starting Dots count and confirming at least one differs after
    /// (astronomically unlikely for a real RNG to reassign all 8 back
    /// to the identical value it started from).
    #[test]
    fn randomize_all_touches_every_shape() {
        let mut app = new_app();
        for s in 0..NUM_SHAPES {
            app.params.shapes[s].dots.store(5, Ordering::Relaxed);
        }
        app.reset(Selection::RandomizeAll);
        let any_changed = (0..NUM_SHAPES).any(|s| app.params.shapes[s].dots.load(Ordering::Relaxed) != 5);
        assert!(any_changed, "expected Randomize All to reassign at least one shape's Dots away from its shared starting value");
    }

    /// Link Range Notes must move the other bound by the same amount
    /// when one is edited, keeping the range's width fixed -- and
    /// must leave it alone (normal clamp-only behavior) while off.
    #[test]
    fn link_range_notes_preserves_width_only_while_on() {
        let mut app = new_app();
        app.params.shapes[0].min_note.store(50, Ordering::Relaxed);
        app.params.shapes[0].max_note.store(60, Ordering::Relaxed);

        // Off (default): editing Min must not move Max.
        app.edit(Selection::MinNote(0), 1);
        assert_eq!(app.params.shapes[0].min_note.load(Ordering::Relaxed), 51);
        assert_eq!(app.params.shapes[0].max_note.load(Ordering::Relaxed), 60, "Max must stay put while Link is off");

        // On: editing Min must move Max by the same delta.
        app.params.shapes[0].link_range_notes.store(true, Ordering::Relaxed);
        app.edit(Selection::MinNote(0), 1);
        assert_eq!(app.params.shapes[0].min_note.load(Ordering::Relaxed), 52);
        assert_eq!(app.params.shapes[0].max_note.load(Ordering::Relaxed), 61, "Max must follow Min by the same +1 while Link is on");
    }
}

