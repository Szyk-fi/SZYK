//! A shared, monophonic-step arpeggiator: turns a chord of currently-
//! held pads into a single, cycling "which pad sounds right now" gate,
//! at a configurable rate and pattern. Any pitched app (Plaits,
//! Cascade, Voltage -- the ones whose pads are a 16-note keyboard)
//! owns one of these and steps it once per audio block, from inside
//! its own real-time `AudioProcessor::process`, so the timing is
//! sample-block-accurate rather than tied to the ~33ms UI poll rate.
//! When `enabled` is false it's inert -- `step` always returns `None`,
//! so a disabled arp costs nothing beyond the atomic load and callers
//! fall back to their normal (chord/mono) behavior unchanged.
//!
//! `step` walks `held` in plain ascending index order for Up/Down/Up-
//! Down, so for "Up" to mean "ascending pitch" the caller must pass
//! `held` already reordered into pitch-rank order (each app already
//! has a physical-pad-index <-> pitch-rank mapping for its own 4x4
//! keyboard -- e.g. Plaits' `pad_rank`, which is its own inverse) and
//! convert the returned index back the same way.

use crate::util::AtomicF32;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

pub const PATTERN_NAMES: [&str; 4] = ["Up", "Down", "Up/Down", "Random"];
const PATTERN_UP: u32 = 0;
const PATTERN_DOWN: u32 = 1;
const PATTERN_UP_DOWN: u32 = 2;

pub struct Arpeggiator {
    pub enabled: AtomicBool,
    /// Steps per second.
    pub rate_hz: AtomicF32,
    /// Index into `PATTERN_NAMES`.
    pub pattern: AtomicU32,
    state: Mutex<StepState>,
}

struct StepState {
    /// Seconds accumulated since the last step.
    phase: f32,
    /// Index into the *currently held* list (not the pad grid).
    step: usize,
    /// Up/Down direction memory.
    going_up: bool,
    rng: u32,
}

impl Arpeggiator {
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            rate_hz: AtomicF32::new(8.0),
            pattern: AtomicU32::new(PATTERN_UP),
            state: Mutex::new(StepState { phase: 0.0, step: 0, going_up: true, rng: 0x9E3779B9 }),
        }
    }

    /// Advances the arp by `dt` seconds (a real-time audio block's
    /// duration) and returns which single pad index (0..16) should
    /// gate on right now, given `held` (the pads actually, physically
    /// held) -- `None` if the arp is off or nothing is held, meaning
    /// the caller should fall back to its normal behavior.
    pub fn step(&self, held: &[bool; 16], dt: f32) -> Option<usize> {
        if !self.enabled.load(Ordering::Relaxed) {
            return None;
        }
        let indices: Vec<usize> = held.iter().enumerate().filter(|(_, h)| **h).map(|(i, _)| i).collect();
        if indices.is_empty() {
            return None;
        }
        let mut state = self.state.lock().unwrap();
        if state.step >= indices.len() {
            state.step = 0;
        }
        let rate = self.rate_hz.get().max(0.1);
        let step_len = 1.0 / rate;
        state.phase += dt;
        if state.phase >= step_len {
            state.phase %= step_len;
            let pattern = self.pattern.load(Ordering::Relaxed);
            match pattern {
                PATTERN_UP => state.step = (state.step + 1) % indices.len(),
                PATTERN_DOWN => state.step = (state.step + indices.len() - 1) % indices.len(),
                PATTERN_UP_DOWN => {
                    if indices.len() == 1 {
                        state.step = 0;
                    } else if state.going_up {
                        if state.step + 1 >= indices.len() {
                            state.going_up = false;
                            state.step -= 1;
                        } else {
                            state.step += 1;
                        }
                    } else if state.step == 0 {
                        state.going_up = true;
                        state.step = 1;
                    } else {
                        state.step -= 1;
                    }
                }
                _ => {
                    // Random.
                    state.rng ^= state.rng << 13;
                    state.rng ^= state.rng >> 17;
                    state.rng ^= state.rng << 5;
                    state.step = (state.rng as usize) % indices.len();
                }
            }
        }
        Some(indices[state.step])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_arp_always_returns_none() {
        let arp = Arpeggiator::new();
        let held = [true; 16];
        assert_eq!(arp.step(&held, 1.0), None);
    }

    #[test]
    fn nothing_held_returns_none_even_when_enabled() {
        let arp = Arpeggiator::new();
        arp.enabled.store(true, Ordering::Relaxed);
        let held = [false; 16];
        assert_eq!(arp.step(&held, 1.0), None);
    }

    #[test]
    fn up_pattern_cycles_through_held_pads_in_ascending_index_order() {
        let arp = Arpeggiator::new();
        arp.enabled.store(true, Ordering::Relaxed);
        arp.rate_hz.set(1.0); // one step per second
        let mut held = [false; 16];
        held[2] = true;
        held[5] = true;
        held[9] = true;

        let first = arp.step(&held, 0.0).unwrap();
        let second = arp.step(&held, 1.0).unwrap();
        let third = arp.step(&held, 1.0).unwrap();
        let fourth = arp.step(&held, 1.0).unwrap(); // wraps back to the first

        assert_eq!([first, second, third, fourth], [2, 5, 9, 2]);
    }

    #[test]
    fn a_held_pad_dropping_out_never_panics_and_stays_within_the_shrunk_set() {
        let arp = Arpeggiator::new();
        arp.enabled.store(true, Ordering::Relaxed);
        arp.rate_hz.set(1.0);
        let mut held = [false; 16];
        held[0] = true;
        held[1] = true;
        held[2] = true;
        arp.step(&held, 0.0);
        arp.step(&held, 1.0);
        arp.step(&held, 1.0);
        // Only pad 0 stays held -- the next step must land on it, not
        // panic on an out-of-range index into the now-shorter list.
        held = [false; 16];
        held[0] = true;
        let step = arp.step(&held, 1.0).unwrap();
        assert_eq!(step, 0);
    }

    #[test]
    fn up_down_pattern_reverses_at_each_end_without_repeating_the_endpoint() {
        let arp = Arpeggiator::new();
        arp.enabled.store(true, Ordering::Relaxed);
        arp.rate_hz.set(1.0);
        arp.pattern.store(PATTERN_UP_DOWN, Ordering::Relaxed);
        let mut held = [false; 16];
        held[0] = true;
        held[1] = true;
        held[2] = true;

        let mut sequence = vec![arp.step(&held, 0.0).unwrap()];
        for _ in 0..6 {
            sequence.push(arp.step(&held, 1.0).unwrap());
        }
        assert_eq!(sequence, vec![0, 1, 2, 1, 0, 1, 2]);
    }
}
