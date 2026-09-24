//! User-configurable MIDI CC -> modulation-target mapping, so any
//! class-compliant controller's knobs/faders can drive any app's
//! parameter without a code change -- the same "discover what's
//! registered, don't hardcode it" principle modbus.rs/audio_bus.rs
//! already follow for cross-app routing, extended out to real
//! external MIDI hardware. Complements (doesn't replace) the fixed
//! CCs main.rs's own `handle_midi_message` already reserves for the
//! two physical knobs, the pad grid, and Prism's macro knobs -- a CC
//! already claimed by one of those never reaches this map.
//!
//! Targets are stored and looked up **by name**, not by the modbus
//! index handed out at registration time -- that index depends on
//! which apps happened to construct first, and this build's whole
//! premise is that apps can be added or removed, so a saved mapping
//! must survive the registered-target list reshuffling around it.

use crate::modbus::ModBus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub struct MidiMap {
    /// (CC number, target name) -- a `Vec`, not a `HashMap`, since
    /// this is small (a handful of mappings) and needs a stable
    /// display order more than O(1) lookup.
    mappings: Mutex<Vec<(u8, String)>>,
    /// True while "Learn" is armed (see `midi_map.rs`'s app) -- the
    /// MIDI thread claims the *next* CC it sees (that isn't already
    /// mapped or one of the reserved control CCs) as the learned one.
    armed: AtomicBool,
    /// Set by the MIDI thread once Learn captures a CC; consumed
    /// (taken) by the app's own `tick()` on the UI side.
    learned_cc: Mutex<Option<u8>>,
}

impl MidiMap {
    pub fn new() -> Self {
        Self { mappings: Mutex::new(Vec::new()), armed: AtomicBool::new(false), learned_cc: Mutex::new(None) }
    }

    /// All current mappings, in the order they were added.
    pub fn mappings(&self) -> Vec<(u8, String)> {
        self.mappings.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.mappings.lock().unwrap().len()
    }

    /// Adds a mapping, replacing any existing one for the same CC
    /// (a CC can only ever drive one target at a time) or the same
    /// target (a target only makes sense driven by one CC, otherwise
    /// two controls would fight over the same value).
    pub fn set(&self, cc: u8, target_name: String) {
        let mut m = self.mappings.lock().unwrap();
        m.retain(|(c, n)| *c != cc && *n != target_name);
        m.push((cc, target_name));
    }

    pub fn remove_at(&self, idx: usize) {
        let mut m = self.mappings.lock().unwrap();
        if idx < m.len() {
            m.remove(idx);
        }
    }

    /// Arms Learn mode -- the next real CC (not already mapped, not
    /// one of the fixed control CCs) the MIDI thread sees becomes the
    /// pending learned CC.
    pub fn arm_learn(&self) {
        *self.learned_cc.lock().unwrap() = None;
        self.armed.store(true, Ordering::Relaxed);
    }

    pub fn is_armed(&self) -> bool {
        self.armed.load(Ordering::Relaxed)
    }

    pub fn cancel_learn(&self) {
        self.armed.store(false, Ordering::Relaxed);
    }

    /// Takes (consumes) the CC Learn just captured, if any -- called
    /// once per UI tick.
    pub fn take_learned(&self) -> Option<u8> {
        self.learned_cc.lock().unwrap().take()
    }

    /// Called by the MIDI thread for every CC not already claimed by
    /// a fixed control mapping. Completes a Learn if armed; otherwise
    /// applies this CC's value to its mapped target, if any.
    pub fn observe_cc(&self, cc: u8, value_0_127: u8, modbus: &ModBus) {
        if self.armed.swap(false, Ordering::Relaxed) {
            *self.learned_cc.lock().unwrap() = Some(cc);
            return;
        }
        let target_name = {
            let m = self.mappings.lock().unwrap();
            m.iter().find(|(c, _)| *c == cc).map(|(_, n)| n.clone())
        };
        let Some(name) = target_name else { return };
        // Looked up live against modbus's own current name list, not
        // a snapshotted index -- see module doc comment.
        if let Some(idx) = modbus.names().iter().position(|n| *n == name) {
            if let Some(handle) = modbus.get(idx) {
                handle.set(value_0_127 as f32 / 127.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mapped_cc_drives_its_named_target() {
        let map = MidiMap::new();
        let modbus = ModBus::new();
        let handle = modbus.register("Some App: Some Param");
        map.set(20, "Some App: Some Param".into());

        map.observe_cc(20, 127, &modbus);
        assert!((handle.get() - 1.0).abs() < 0.01, "expected the mapped target to reach ~1.0, got {}", handle.get());

        map.observe_cc(20, 0, &modbus);
        assert!(handle.get() < 0.01, "expected the mapped target to reach ~0.0, got {}", handle.get());
    }

    #[test]
    fn an_unmapped_cc_does_nothing() {
        let map = MidiMap::new();
        let modbus = ModBus::new();
        let handle = modbus.register("Some App: Some Param");
        map.observe_cc(20, 127, &modbus);
        assert_eq!(handle.get(), 0.0);
    }

    #[test]
    fn setting_a_cc_already_used_by_another_target_moves_it() {
        let map = MidiMap::new();
        map.set(20, "A".into());
        map.set(20, "B".into());
        assert_eq!(map.mappings(), vec![(20, "B".to_string())]);
    }

    #[test]
    fn setting_a_target_already_mapped_elsewhere_moves_it_too() {
        let map = MidiMap::new();
        map.set(20, "A".into());
        map.set(21, "A".into());
        assert_eq!(map.mappings(), vec![(21, "A".to_string())]);
    }

    #[test]
    fn learn_captures_the_next_real_cc_and_consumes_it_once() {
        let map = MidiMap::new();
        let modbus = ModBus::new();
        assert_eq!(map.take_learned(), None);
        map.arm_learn();
        assert!(map.is_armed());
        map.observe_cc(42, 100, &modbus);
        assert!(!map.is_armed(), "learn should disarm itself once it captures a CC");
        assert_eq!(map.take_learned(), Some(42));
        assert_eq!(map.take_learned(), None, "a second take should find nothing left");
    }

    #[test]
    fn removing_a_mapping_by_index_works() {
        let map = MidiMap::new();
        map.set(1, "A".into());
        map.set(2, "B".into());
        map.remove_at(0);
        assert_eq!(map.mappings(), vec![(2, "B".to_string())]);
    }
}
