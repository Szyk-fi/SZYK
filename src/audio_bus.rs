//! Lets one app's rendered audio be tapped by another before the final
//! mix -- e.g. Clouds granulating Plaits' live output. Parallel to
//! modbus.rs, but carries a whole per-block signal instead of one
//! number: a source app overwrites its published buffer with its
//! rendered mono block every `process()` call; a consumer (Clouds)
//! reads whatever's currently there.
//!
//! Since every app's processor runs once per callback in the shared
//! MixBus (see audio.rs) in a fixed order, a consumer sees the
//! source's block for the *same* callback if the source happens to run
//! first, or the previous callback's block otherwise -- at most one
//! block (a few ms) of latency, same tradeoff already accepted for
//! ModBus-based modulation.

use std::sync::{Arc, Mutex};

struct AudioSource {
    name: String,
    buffer: Arc<Mutex<Vec<f32>>>,
}

pub struct AudioBus {
    sources: Mutex<Vec<AudioSource>>,
}

impl AudioBus {
    pub fn new() -> Self {
        Self { sources: Mutex::new(Vec::new()) }
    }

    /// Registers a new tappable output and returns the buffer the
    /// owning app should overwrite (not append to) with its rendered
    /// mono block every `process()` call. Call once per source during
    /// app construction, not per-block.
    pub fn register(&self, name: impl Into<String>) -> Arc<Mutex<Vec<f32>>> {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        self.sources.lock().unwrap().push(AudioSource { name: name.into(), buffer: Arc::clone(&buffer) });
        buffer
    }

    pub fn names(&self) -> Vec<String> {
        self.sources.lock().unwrap().iter().map(|s| s.name.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.sources.lock().unwrap().len()
    }

    pub fn get(&self, idx: usize) -> Option<Arc<Mutex<Vec<f32>>>> {
        self.sources.lock().unwrap().get(idx).map(|s| Arc::clone(&s.buffer))
    }
}
