//! The audio engine: a mixing bus, not a single swappable slot. Every
//! app that has a processor gets one, once, at startup -- and it keeps
//! running for the life of the program, mixed together with every other
//! app's, regardless of which app is on screen. This is what makes
//! cross-app modulation (Pam's driving a Plaits or Sequencer parameter)
//! actually audible even when Pam's own screen isn't the one showing --
//! matching the intent os.rs's own header comment already stated
//! ("the audio/MIDI pipeline keeps running regardless of what's
//! displayed"), just now delivered for sound generation too, not only
//! the callback itself staying alive.
//!
//! main.rs still owns the ring buffer bridging input->output and the
//! `run_inference` stub — those are OS-level plumbing, not per-app.

use crate::util::AtomicF32;
use std::sync::{Arc, Mutex};

pub trait AudioProcessor: Send {
    /// `buffer` is interleaved by `channels`. Called once per output block.
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32);
}

/// A gain above this is very likely a stuck/misbehaving control, not a
/// deliberate boost -- clamped here as a last-resort safety ceiling
/// regardless of what the Mixer app's own (narrower) knob range allows.
const MAX_MASTER_VOLUME: f32 = 2.0;

/// A processor's failure count resets on any block it survives --
/// this is "N in a row," not "N ever" -- but once it hits this many
/// *consecutive* panics, it's disabled for the rest of the session
/// rather than left to keep panicking (and unwinding -- not free)
/// every single block forever.
const MAX_CONSECUTIVE_PANICS: u32 = 8;

struct ProcessorSlot {
    processor: Box<dyn AudioProcessor>,
    consecutive_panics: u32,
    disabled: bool,
}

/// Sums every registered processor's output into the callback's buffer,
/// then applies one final master gain stage -- see apps/mixer.rs, the
/// only thing that ever writes to `master_volume`.
/// Each processor renders into its own reused scratch buffer (never a
/// fresh allocation in the callback) and gets added in.
pub struct MixBus {
    processors: Mutex<Vec<ProcessorSlot>>,
    scratch: Mutex<Vec<f32>>,
    master_volume: Arc<AtomicF32>,
}

impl MixBus {
    pub fn new(master_volume: Arc<AtomicF32>) -> Self {
        Self { processors: Mutex::new(Vec::new()), scratch: Mutex::new(Vec::new()), master_volume }
    }

    /// Registers a processor to run for the rest of the program's life.
    /// Called once per app at startup (see main.rs) -- apps don't get
    /// added or removed as the user navigates between them.
    pub fn add(&self, processor: Box<dyn AudioProcessor>) {
        self.processors.lock().unwrap().push(ProcessorSlot { processor, consecutive_panics: 0, disabled: false });
    }

    pub fn process(&self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        for out in buffer.iter_mut() {
            *out = 0.0;
        }
        let mut scratch = self.scratch.lock().unwrap();
        scratch.clear();
        scratch.resize(buffer.len(), 0.0);

        let mut processors = self.processors.lock().unwrap();
        for slot in processors.iter_mut() {
            if slot.disabled {
                continue;
            }
            for s in scratch.iter_mut() {
                *s = 0.0;
            }

            // Isolate a panic to *this one app* -- with N apps all
            // sharing this one audio callback (and someday possibly
            // far more), a bug in any single one of them must never
            // be able to take down the whole callback and silence
            // every other app along with it. `AssertUnwindSafe` is
            // the standard way to tell `catch_unwind` "yes, I know
            // `scratch` could be left half-written if this unwinds --
            // that's fine, it gets zeroed right below either way, so
            // there's no broken invariant for the *next* app (or the
            // next block) to inherit."
            let processor = &mut slot.processor;
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                processor.process(&mut scratch, channels, sample_rate);
            }))
            .is_err();
            if panicked {
                slot.consecutive_panics += 1;
                eprintln!("audio: a processor panicked ({}/{MAX_CONSECUTIVE_PANICS} consecutive); treating it as silent for this block", slot.consecutive_panics);
                for s in scratch.iter_mut() {
                    *s = 0.0;
                }
                if slot.consecutive_panics >= MAX_CONSECUTIVE_PANICS {
                    slot.disabled = true;
                    eprintln!("audio: a processor panicked {MAX_CONSECUTIVE_PANICS} blocks in a row -- disabled for the rest of this session");
                    continue;
                }
            } else {
                slot.consecutive_panics = 0;
            }

            // Sanitize *this app's own output* before it ever joins
            // the sum -- a single NaN (from, say, an unguarded 0/0 in
            // one app's DSP) would otherwise poison every sample it
            // touches forever after (NaN + anything = NaN), silently
            // wrecking every *other* app sharing the bus along with
            // it. `tanh` alone doesn't cover this: `tanh(NaN)` is
            // still NaN, only infinities are naturally tamed by it.
            for s in scratch.iter_mut() {
                if !s.is_finite() {
                    *s = 0.0;
                }
            }
            // Soft-clip *each app's own output* before it ever joins
            // the sum, not just the final mix below. An app's
            // internal headroom normalization (dividing by its own
            // active voice/track count) only bounds *that*, not a
            // single voice legitimately running hot -- a resonant
            // filter or a feedback loop pushed toward its ceiling can
            // still send this a signal several times unity on its
            // own, which the final limiter alone would then have to
            // squash so hard it's audibly fuzzed even when nothing
            // else is playing. Catching it per-app, at the source,
            // means one loud app can't ever force every *other* app
            // sharing the bus into that same squashed territory.
            for s in scratch.iter_mut() {
                *s = s.tanh();
            }
            for (out, s) in buffer.iter_mut().zip(scratch.iter()) {
                *out += *s;
            }
        }

        let volume = self.master_volume.get().clamp(0.0, MAX_MASTER_VOLUME);
        for out in buffer.iter_mut() {
            *out *= volume;
        }

        // A soft limiter on the very final output -- summing now
        // routinely spans a dozen-plus apps (each headroom-normalized
        // *within itself*, but nothing upstream ever accounts for
        // several of them being loud at once), so without this the
        // device buffer can exceed +/-1 by a wide margin and the
        // audio backend just hard-clips it into harsh digital
        // distortion. `tanh` is close to transparent (≈identity) for
        // normal listening levels and only starts rounding off peaks
        // as they approach and pass +/-1, rather than slicing them
        // off abruptly -- the standard "soft clip" a hardware mixer's
        // master bus would apply too.
        for out in buffer.iter_mut() {
            *out = out.tanh();
        }
    }
}

pub type ActiveProcessor = Arc<MixBus>;

pub fn new_engine(master_volume: Arc<AtomicF32>) -> ActiveProcessor {
    Arc::new(MixBus::new(master_volume))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A processor that always writes a fixed constant sample.
    struct ConstProcessor(f32);
    impl AudioProcessor for ConstProcessor {
        fn process(&mut self, buffer: &mut [f32], _channels: usize, _sample_rate: f32) {
            for s in buffer.iter_mut() {
                *s = self.0;
            }
        }
    }

    /// A processor standing in for a real bug: an unguarded division
    /// producing NaN.
    struct NanProcessor;
    impl AudioProcessor for NanProcessor {
        fn process(&mut self, buffer: &mut [f32], _channels: usize, _sample_rate: f32) {
            for s in buffer.iter_mut() {
                *s = 0.0 / 0.0;
            }
        }
    }

    /// A processor that always panics -- standing in for any other
    /// kind of real bug (an out-of-bounds index, an unwrap on a None,
    /// etc.).
    struct PanicProcessor;
    impl AudioProcessor for PanicProcessor {
        fn process(&mut self, _buffer: &mut [f32], _channels: usize, _sample_rate: f32) {
            panic!("simulated bug in this processor");
        }
    }

    /// At ordinary listening levels, the limiter should be close to
    /// transparent -- a single, modest processor's output shouldn't
    /// be audibly squashed.
    #[test]
    fn stays_near_transparent_at_normal_levels() {
        let bus = MixBus::new(Arc::new(AtomicF32::new(1.0)));
        bus.add(Box::new(ConstProcessor(0.3)));
        let mut buffer = vec![0.0f32; 16];
        bus.process(&mut buffer, 2, 48000.0);
        for v in buffer {
            assert!((v - 0.3).abs() < 0.02, "expected near-transparent output at 0.3, got {v}");
        }
    }

    /// However many processors sum together, and however hot each
    /// one runs individually, the final output must never exceed the
    /// device's +/-1 range -- this is the whole point of the limiter.
    #[test]
    fn many_hot_processors_never_exceed_unity() {
        let bus = MixBus::new(Arc::new(AtomicF32::new(2.0))); // master volume maxed too
        for _ in 0..20 {
            bus.add(Box::new(ConstProcessor(1.0)));
        }
        let mut buffer = vec![0.0f32; 16];
        bus.process(&mut buffer, 2, 48000.0);
        for v in buffer {
            assert!(v.is_finite(), "output must stay finite");
            // tanh's true range is the *open* interval (-1, 1) -- it
            // never mathematically reaches the boundary -- but at
            // extreme inputs like this (sum=20, master volume=2,
            // pre-limiter=40) f32 precision itself rounds the result
            // to exactly 1.0. Either way it's bounded, which is the
            // property under test: never *exceeding* +/-1, unlike
            // the unbounded (e.g. 40.0) value this would have been
            // pre-fix.
            assert!(v.abs() <= 1.0, "output must never exceed +/-1, got {v}");
            assert!(v > 0.9, "20 hot processors summed should still saturate close to the ceiling, got {v}");
        }
    }

    /// A *single* app running hot (a resonant filter or feedback
    /// loop legitimately pushing several times unity gain -- an
    /// app's own internal headroom normalization bounds voice/track
    /// *count*, not one voice already running hot) must be tamed at
    /// the source, before it ever reaches the shared sum -- otherwise
    /// one loud app alone would still force the *final* limiter to
    /// squash it audibly even with every other app silent.
    #[test]
    fn one_hot_processor_is_tamed_before_summing() {
        let bus = MixBus::new(Arc::new(AtomicF32::new(1.0)));
        bus.add(Box::new(ConstProcessor(8.0)));
        let mut buffer = vec![0.0f32; 16];
        bus.process(&mut buffer, 2, 48000.0);
        for v in buffer {
            assert!(v.is_finite());
            assert!(v.abs() <= 1.0, "a single hot app must already be bounded near unity before the final mix, got {v}");
        }
    }

    /// Silence in, silence out -- the limiter shouldn't introduce any
    /// DC offset or noise floor of its own.
    #[test]
    fn silence_stays_silent() {
        let bus = MixBus::new(Arc::new(AtomicF32::new(1.0)));
        bus.add(Box::new(ConstProcessor(0.0)));
        let mut buffer = vec![0.0f32; 16];
        bus.process(&mut buffer, 2, 48000.0);
        for v in buffer {
            assert_eq!(v, 0.0);
        }
    }

    /// One app producing NaN must never poison another app's
    /// perfectly good signal sharing the same sum -- NaN + anything
    /// is NaN, so without sanitizing at the source this would
    /// silently wreck every app on the bus, forever, not just the
    /// broken one.
    #[test]
    fn one_nan_processor_does_not_poison_others() {
        let bus = MixBus::new(Arc::new(AtomicF32::new(1.0)));
        bus.add(Box::new(NanProcessor));
        bus.add(Box::new(ConstProcessor(0.3)));
        let mut buffer = vec![0.0f32; 16];
        bus.process(&mut buffer, 2, 48000.0);
        for v in buffer {
            assert!(v.is_finite(), "a NaN from one processor must not have poisoned the shared sum");
            assert!((v - 0.3).abs() < 0.02, "the other, good processor's signal should still come through cleanly, got {v}");
        }
    }

    /// A panicking processor must not take down the whole callback --
    /// every other app sharing it must keep working normally, and the
    /// callback itself must return instead of unwinding out of it.
    #[test]
    fn one_panicking_processor_does_not_silence_others() {
        let bus = MixBus::new(Arc::new(AtomicF32::new(1.0)));
        bus.add(Box::new(PanicProcessor));
        bus.add(Box::new(ConstProcessor(0.3)));
        let mut buffer = vec![0.0f32; 16];
        bus.process(&mut buffer, 2, 48000.0); // must not unwind out of this call
        for v in buffer {
            assert!((v - 0.3).abs() < 0.02, "the other, working processor should still be audible, got {v}");
        }
    }

    /// A processor that panics every single block must eventually be
    /// disabled rather than left to keep panicking (and unwinding --
    /// real CPU cost) forever, once every other app sharing the
    /// callback is what actually matters at scale.
    #[test]
    fn repeatedly_panicking_processor_gets_disabled() {
        let bus = MixBus::new(Arc::new(AtomicF32::new(1.0)));
        bus.add(Box::new(PanicProcessor));
        let mut buffer = vec![0.0f32; 16];
        for _ in 0..MAX_CONSECUTIVE_PANICS {
            bus.process(&mut buffer, 2, 48000.0);
        }
        assert!(bus.processors.lock().unwrap()[0].disabled, "should be disabled after {MAX_CONSECUTIVE_PANICS} consecutive panics");

        // Once disabled, further calls must be cheap no-ops (no more
        // panicking/unwinding), not just silent.
        bus.process(&mut buffer, 2, 48000.0);
        for v in buffer {
            assert_eq!(v, 0.0);
        }
    }

    /// A processor that panics only *occasionally* (not every block)
    /// must not accumulate toward disablement across unrelated,
    /// successful blocks in between -- only a *consecutive* run of
    /// failures should count.
    #[test]
    fn occasional_panics_do_not_accumulate_toward_disablement() {
        struct FlakyProcessor {
            block: u32,
        }
        impl AudioProcessor for FlakyProcessor {
            fn process(&mut self, buffer: &mut [f32], _channels: usize, _sample_rate: f32) {
                self.block += 1;
                if self.block % 2 == 0 {
                    panic!("simulated intermittent bug");
                }
                for s in buffer.iter_mut() {
                    *s = 0.1;
                }
            }
        }

        let bus = MixBus::new(Arc::new(AtomicF32::new(1.0)));
        bus.add(Box::new(FlakyProcessor { block: 0 }));
        let mut buffer = vec![0.0f32; 16];
        for _ in 0..(MAX_CONSECUTIVE_PANICS * 3) {
            bus.process(&mut buffer, 2, 48000.0);
        }
        assert!(!bus.processors.lock().unwrap()[0].disabled, "alternating success/panic should never reach a *consecutive* run long enough to disable it");
    }
}
