//! The cross-app modulation registry. Any app can expose a parameter
//! as a modulation target by calling `register` during its own
//! construction and keeping the returned handle as an "external
//! modulation input" -- an atomic it adds into its own computation
//! every block, on top of whatever its own knobs/internal modulators
//! already contribute (see plaits.rs's Harmonics/Timbre/Morph/Decay and
//! sequencer.rs's per-track Volume for the pattern).
//!
//! A source app (Pam's) enumerates `names()` to show the user what's
//! available, then writes into the handle for whichever target a
//! channel is routed to. Because every app's processor now runs
//! continuously in the mix bus (see audio.rs) rather than only the one
//! on screen, this is live modulation, not just a value that sits there
//! until the target app happens to be active.

use crate::util::AtomicF32;
use std::sync::{Arc, Mutex};

struct ModTarget {
    name: String,
    value: Arc<AtomicF32>,
}

pub struct ModBus {
    targets: Mutex<Vec<ModTarget>>,
}

impl ModBus {
    pub fn new() -> Self {
        Self { targets: Mutex::new(Vec::new()) }
    }

    /// Registers a new modulation target and returns the atomic the
    /// owning app should read (and add into its own value) every block.
    /// Call once per target during app construction, not per-frame.
    pub fn register(&self, name: impl Into<String>) -> Arc<AtomicF32> {
        let value = Arc::new(AtomicF32::new(0.0));
        self.targets.lock().unwrap().push(ModTarget { name: name.into(), value: Arc::clone(&value) });
        value
    }

    pub fn names(&self) -> Vec<String> {
        self.targets.lock().unwrap().iter().map(|t| t.name.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.targets.lock().unwrap().len()
    }

    /// The handle to write into for target `idx`, or `None` once
    /// nothing is routed there (or the index is out of range).
    pub fn get(&self, idx: usize) -> Option<Arc<AtomicF32>> {
        self.targets.lock().unwrap().get(idx).map(|t| Arc::clone(&t.value))
    }
}
