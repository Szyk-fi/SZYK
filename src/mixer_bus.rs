//! The master mixer's per-app channel levels. Parallel to modbus.rs
//! and audio_bus.rs, but combines a piece of both: a source app
//! (anything with an audible output) calls `register` once during
//! construction and gets back two atomics to add together and
//! multiply into its own final output gain every block -- `level`
//! (the fader value, edited from the Mixer app, default 1.0 so an
//! unregistered/untouched channel changes nothing) and `ext_level`
//! (a modulation input on the shared ModBus, exactly like every other
//! continuous knob in this build -- see modbus.rs). `register` needs
//! a `&ModBus` to set that up, so this sits one layer above it rather
//! than beside it.
//!
//! The Mixer app itself never calls `register` -- it only enumerates
//! `names()`/`level(idx)` live (same "query live, don't snapshot"
//! pattern Clouds/Prism already use for AudioBus/ModBus) to display
//! and edit whatever channels have shown up, in whatever order the
//! registry.rs construction order put them in.

use crate::modbus::ModBus;
use crate::util::AtomicF32;
use std::sync::{Arc, Mutex};

struct MixerChannel {
    name: String,
    level: Arc<AtomicF32>,
}

pub struct MixerBus {
    channels: Mutex<Vec<MixerChannel>>,
}

impl MixerBus {
    pub fn new() -> Self {
        Self { channels: Mutex::new(Vec::new()) }
    }

    /// Registers a new mixer channel and returns (level, ext_level)
    /// -- the owning app should read both every block and multiply
    /// their sum (clamped to a sane range) into its final output,
    /// same additive-modulation convention used everywhere else. Call
    /// once per app during construction, not per-block.
    pub fn register(&self, name: impl Into<String>, modbus: &ModBus) -> (Arc<AtomicF32>, Arc<AtomicF32>) {
        let name = name.into();
        let level = Arc::new(AtomicF32::new(1.0));
        let ext_level = modbus.register(format!("Mixer: {name} Level"));
        self.channels.lock().unwrap().push(MixerChannel { name, level: Arc::clone(&level) });
        (level, ext_level)
    }

    pub fn names(&self) -> Vec<String> {
        self.channels.lock().unwrap().iter().map(|c| c.name.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.channels.lock().unwrap().len()
    }

    /// The fader handle for channel `idx`, for the Mixer app to
    /// display/edit -- not the same handle as `ext_level`, which stays
    /// with the owning app and is never touched from here.
    pub fn level(&self, idx: usize) -> Option<Arc<AtomicF32>> {
        self.channels.lock().unwrap().get(idx).map(|c| Arc::clone(&c.level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh channel must default to unity (1.0) -- an app that
    /// registers but is never touched from the Mixer app must sound
    /// exactly as if the Mixer didn't exist.
    #[test]
    fn fresh_channel_defaults_to_unity() {
        let modbus = ModBus::new();
        let bus = MixerBus::new();
        let (level, ext_level) = bus.register("Plaits", &modbus);
        assert_eq!(level.get(), 1.0);
        assert_eq!(ext_level.get(), 0.0); // modulation input starts neutral, additive
    }

    /// `names()`/`level(idx)` must reflect every registered channel,
    /// in registration order, and `level(idx)` must be the *same*
    /// handle the owning app was given -- editing it from "the Mixer
    /// app's" side must be visible to the owning app immediately.
    #[test]
    fn names_and_level_reflect_registration_order_and_share_the_handle() {
        let modbus = ModBus::new();
        let bus = MixerBus::new();
        let (plaits_level, _) = bus.register("Plaits", &modbus);
        let (_seq_level, _) = bus.register("Sequencer", &modbus);

        assert_eq!(bus.names(), vec!["Plaits", "Sequencer"]);
        assert_eq!(bus.len(), 2);

        let fetched = bus.level(0).expect("channel 0 should exist");
        fetched.set(0.5);
        assert_eq!(plaits_level.get(), 0.5, "editing via level(idx) must be visible through the owning app's own handle");

        assert!(bus.level(2).is_none(), "an out-of-range index must return None, not panic");
    }

    /// The modulation target's name must include the app's name, so
    /// Pam's (or anything else browsing ModBus) can tell channels
    /// apart.
    #[test]
    fn register_names_the_modbus_target_after_the_app() {
        let modbus = ModBus::new();
        let bus = MixerBus::new();
        bus.register("Bloom", &modbus);
        assert!(modbus.names().iter().any(|n| n.contains("Bloom")), "expected a ModBus target mentioning 'Bloom', got {:?}", modbus.names());
    }
}
