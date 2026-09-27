//! Tiny helpers shared across apps.

use std::sync::{atomic::{AtomicU32, Ordering}, Arc, OnceLock};

/// An f32 readable/writable from multiple threads without a lock — f32 has
/// no native atomic type, so this stores it as raw bits behind an
/// AtomicU32. Used wherever an app's audio processor needs to hand a
/// number back to that app's own `draw()` (a filter's cutoff, a meter's
/// peak level) across the audio/UI thread boundary.
pub struct AtomicF32(AtomicU32, OnceLock<Box<[AtomicU32; 16]>>, AtomicU32);

/// One cable's independent contribution. Dropping it disconnects only that cable.
pub struct AtomicContribution { target: Arc<AtomicF32>, slot: usize }
impl AtomicContribution {
    pub fn set(&self, value:f32) {
        let value=if value.is_finite(){value.clamp(-4.,4.)}else{0.};
        self.target.1.get().unwrap()[self.slot].store(value.to_bits(),Ordering::Relaxed);
    }
}
impl Drop for AtomicContribution {
    fn drop(&mut self) {
        self.set(0.);
        self.target.2.fetch_and(!(1<<self.slot),Ordering::Release);
    }
}

impl AtomicF32 {
    pub fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()), OnceLock::new(), AtomicU32::new(0))
    }

    /// Allocate off the audio thread; ordinary parameters allocate no slot storage.
    pub fn contribution(self:&Arc<Self>) -> Option<AtomicContribution> {
        self.1.get_or_init(||Box::new(std::array::from_fn(|_|AtomicU32::new(0))));
        let mut active=self.2.load(Ordering::Acquire);
        loop {
            let slot=(!active & 0xffff).trailing_zeros() as usize;
            if slot>=16{return None;}
            match self.2.compare_exchange_weak(active,active|(1<<slot),Ordering::AcqRel,Ordering::Acquire) {
                Ok(_)=>return Some(AtomicContribution{target:self.clone(),slot}),Err(next)=>active=next,
            }
        }
    }
    pub fn set(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn get(&self) -> f32 {
        let mut value=f32::from_bits(self.0.load(Ordering::Relaxed));
        let mut active=self.2.load(Ordering::Acquire);
        if active!=0 {if let Some(slots)=self.1.get() {while active!=0 {let i=active.trailing_zeros() as usize;value+=f32::from_bits(slots[i].load(Ordering::Relaxed));active&=active-1;}}}
        value
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
