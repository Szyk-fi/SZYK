//! Tiny helpers shared across apps.

use std::sync::atomic::{AtomicU32, Ordering};

/// An f32 readable/writable from multiple threads without a lock — f32 has
/// no native atomic type, so this stores it as raw bits behind an
/// AtomicU32. Used wherever an app's audio processor needs to hand a
/// number back to that app's own `draw()` (a filter's cutoff, a meter's
/// peak level) across the audio/UI thread boundary.
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    pub fn set(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

/// Turning a knob quickly should change its value by more than turning
/// it slowly by the same number of raw ticks. `delta` (ticks
/// accumulated since the last frame -- see controller.rs) already
/// grows with turning speed, since a faster physical turn packs more
/// MIDI ticks into one frame; raising its magnitude to a power above 1
/// turns that raw correlation into real acceleration -- a single tick
/// still moves by the base amount (`1.0.powf(_) == 1.0`, so slow,
/// precise moves are unchanged), but several ticks in one frame move
/// by much more than proportionally.
///
/// Only for continuous knob edits (bump()-style value changes, BPM,
/// swing, and similar). Discrete/cyclic steppers -- an engine list, a
/// chord type -- deliberately use `delta.signum()` instead, one step
/// per tick regardless of speed; that was a considered fix for a
/// previous bug where a fast burst of ticks flew wildly around a short
/// list, and applying acceleration there would reintroduce it.
const KNOB_ACCEL_EXPONENT: f32 = 1.6;

pub fn accelerate(delta: i32) -> f32 {
    delta.signum() as f32 * (delta.unsigned_abs() as f32).powf(KNOB_ACCEL_EXPONENT)
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Standard MIDI-style note naming: note 60 = "C4". Used to label grid
/// keys and show what's currently sounding, in both Synth and Plaits.
pub fn note_name(note: i32) -> String {
    let pitch_class = note.rem_euclid(12) as usize;
    let octave = note.div_euclid(12) - 1;
    format!("{}{octave}", NOTE_NAMES[pitch_class])
}
