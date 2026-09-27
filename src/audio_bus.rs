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
use std::time::{Duration,Instant};

struct AudioSource {
    name: String,
    owner: Option<String>,
    claimed: bool,
    last_read: Option<Instant>,
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
        let name=name.into();let mut sources=self.sources.lock().unwrap();
        if let Some(source)=sources.iter_mut().find(|s|s.name==name) {source.claimed=true;return source.buffer.clone();}
        let buffer=Arc::new(Mutex::new(Vec::new()));
        sources.push(AudioSource{name,owner:None,claimed:true,last_read:None,buffer:buffer.clone()});buffer
    }

    /// Reserve only metadata and an empty buffer. No app or DSP is constructed.
    pub fn declare(&self, owner:&str, name:&str) {
        let mut sources=self.sources.lock().unwrap();
        if let Some(s)=sources.iter_mut().find(|s|s.name==name) {s.owner=Some(owner.into());return;}
        sources.push(AudioSource{name:name.into(),owner:Some(owner.into()),claimed:false,last_read:None,buffer:Arc::new(Mutex::new(Vec::new()))});
    }
    pub fn index_of(&self,name:&str)->Option<usize>{self.sources.lock().unwrap().iter().position(|s|s.name==name)}
    pub fn owned_indices(&self,owner:&str)->Vec<usize>{self.sources.lock().unwrap().iter().enumerate().filter(|(_,s)|s.owner.as_deref()==Some(owner)).map(|(i,_)|i).collect()}
    pub fn is_claimed(&self,index:usize)->bool{self.sources.lock().unwrap().get(index).is_some_and(|s|s.claimed)}
    pub fn requested(&self,owner:&str)->bool {self.sources.lock().unwrap().iter().any(|s|s.owner.as_deref()==Some(owner)&&s.last_read.is_some_and(|t|t.elapsed()<Duration::from_millis(500)))}
    /// Read without waking a source: metering and cleanup must not start engines.
    pub fn peek(&self,idx:usize)->Option<Arc<Mutex<Vec<f32>>>>{self.sources.lock().unwrap().get(idx).map(|s|s.buffer.clone())}

    pub fn names(&self) -> Vec<String> {
        self.sources.lock().unwrap().iter().map(|s| s.name.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.sources.lock().unwrap().len()
    }

    pub fn get(&self, idx: usize) -> Option<Arc<Mutex<Vec<f32>>>> {
        self.sources.lock().unwrap().get_mut(idx).map(|s| {s.last_read=Some(Instant::now());Arc::clone(&s.buffer)})
    }

    /// This index's display name, or `"none"` for `NO_SOURCE`/an
    /// out-of-range index.
    pub fn source_name(&self, idx: usize) -> String {
        if idx == NO_SOURCE {
            "none".to_string()
        } else {
            self.names().get(idx).cloned().unwrap_or_else(|| "none found".into())
        }
    }
}

/// Sentinel meaning "nothing patched into this input" -- every app
/// that taps another app's audio should default its own Source to
/// this, not to index `0`. `0` is whichever app the registry happens
/// to construct first (alphabetically, "Beads"), so an app defaulting
/// to `0` silently starts processing that app's output the moment the
/// engine runs -- audible, unprompted, and in Beads' own case a
/// literal unity-gain self-feedback loop (Beads registers its own bus
/// first, so its own default Source pointed at itself).
pub const NO_SOURCE: usize = usize::MAX;

/// Steps a Source selection (an index into a bus, or `NO_SOURCE`) by
/// `step`, cycling through every real index plus one extra "none"
/// position at the start of the wrap.
pub fn cycle_source(cur: usize, step: i32, len: usize) -> usize {
    if len == 0 { return NO_SOURCE; }
    let n = len as i32;
    // "None" occupies position 0, real index i occupies position i+1 --
    // a clean 0..=n range (n+1 positions) `rem_euclid` can cycle
    // through symmetrically in either direction.
    let cur_pos = if cur == NO_SOURCE { 0 } else { (cur as i32 + 1).min(n) };
    let next_pos = (cur_pos + step).rem_euclid(n + 1);
    if next_pos == 0 {
        NO_SOURCE
    } else {
        (next_pos - 1) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test: an earlier version of this function could
    /// only move *away* from "None" in one direction (turning the
    /// knob the other way from a fresh "None" state did nothing) --
    /// both directions must always move the selection.
    #[test]
    fn cycles_symmetrically_in_both_directions_from_none() {
        let len = 5;
        assert_eq!(cycle_source(NO_SOURCE, 1, len), 0, "forward from None must reach the first real source");
        assert_eq!(cycle_source(NO_SOURCE, -1, len), 4, "backward from None must wrap to the last real source");
    }

    #[test]
    fn cycles_symmetrically_at_the_real_index_boundaries() {
        let len = 5;
        assert_eq!(cycle_source(0, -1, len), NO_SOURCE, "backward from the first source must reach None");
        assert_eq!(cycle_source(4, 1, len), NO_SOURCE, "forward from the last source must reach None");
    }

    #[test]
    fn a_full_round_trip_returns_to_the_start() {
        let len = 5;
        let mut cur = NO_SOURCE;
        for _ in 0..(len + 1) {
            cur = cycle_source(cur, 1, len);
        }
        assert_eq!(cur, NO_SOURCE, "cycling forward len+1 times must return to the starting state");
    }
}

#[cfg(test)]
mod optional_source_tests {
    use super::*;
    #[test]
    fn no_installed_sources_stays_unpatched() {
        for old in [NO_SOURCE, 0, 99] { for step in [-1, 1] {
            assert_eq!(cycle_source(old, step, 0), NO_SOURCE);
        }}
    }
}

#[cfg(test)] mod catalog_tests {
 use super::*;
 #[test] fn declared_port_is_stable_and_browsing_never_requests_dsp(){let bus=AudioBus::new();bus.declare("bloom","Bloom");let before=bus.peek(0).unwrap();assert_eq!(bus.names(),vec!["Bloom"]);assert!(!bus.requested("bloom"));assert!(!bus.is_claimed(0));let published=bus.register("Bloom");assert!(Arc::ptr_eq(&before,&published));assert_eq!(bus.len(),1);assert!(!bus.requested("bloom"));bus.get(0);assert!(bus.requested("bloom"));bus.sources.lock().unwrap()[0].last_read=Some(Instant::now()-Duration::from_secs(1));assert!(!bus.requested("bloom"));}
}
