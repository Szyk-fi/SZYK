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

//!
//! Receivers are declared up front. Each app's manifest lists the
//! modulation inputs it registers (`mod_inputs`), and the registry
//! declares them all at startup, before any app is constructed. So every
//! installed app's inputs show in every source's picker from the start,
//! in stable positions, even though apps are only built when first used;
//! and writing into a declared input wakes its app (see `requested`, used
//! by app_runtime.rs) the same way reading an audio output does.
//!
//! Pickers browse two levels -- app, then that app's input -- through
//! `apps()` / `inputs_of()` and the `Patch` helper below.

use crate::util::AtomicF32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

struct ModTarget {
    name: String,
    value: Arc<AtomicF32>,
    /// Installed app id that declared it ("" if only registered).
    owner: String,
    /// The owning app has been constructed and registered it.
    claimed: bool,
    /// Milliseconds since `epoch()` of the last write through `get` /
    /// `contribution`, or 0 if never.
    last_use: Arc<AtomicU64>,
}

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

fn now_ms() -> u64 {
    epoch().elapsed().as_millis() as u64 + 1
}

/// How long after the last write a declared input keeps its app awake.
const WAKE_MS: u64 = 1000;

/// A receiver name split for display: "Plaits: Harmonics" ->
/// ("Plaits", "Harmonics"). Names without ": " are their own app.
pub fn split_name(name: &str) -> (&str, &str) {
    match name.split_once(": ") {
        Some((app, param)) => (app, param),
        None => (name, name),
    }
}

pub struct ModBus {
    targets: Mutex<Vec<ModTarget>>,
}

impl ModBus {
    pub fn new() -> Self {
        Self { targets: Mutex::new(Vec::new()) }
    }

    /// Declares an installed app's input before the app exists, so it can
    /// be listed and patched. Idempotent per name.
    pub fn declare(&self, owner: &str, name: &str) {
        let mut targets = self.targets.lock().unwrap();
        if targets.iter().any(|t| t.name == name) {
            return;
        }
        targets.push(ModTarget {
            name: name.into(),
            value: Arc::new(AtomicF32::new(0.0)),
            owner: owner.into(),
            claimed: false,
            last_use: Arc::new(AtomicU64::new(0)),
        });
    }

    /// Registers a new modulation target and returns the atomic the
    /// owning app should read (and add into its own value) every block.
    /// Call once per target during app construction, not per-frame. If
    /// the input was declared, this hands back the declared handle, so
    /// anything already patched to it keeps working.
    pub fn register(&self, name: impl Into<String>) -> Arc<AtomicF32> {
        let name = name.into();
        let mut targets = self.targets.lock().unwrap();
        if let Some(t) = targets.iter_mut().find(|t| t.name == name && !t.claimed) {
            t.claimed = true;
            return Arc::clone(&t.value);
        }
        let value = Arc::new(AtomicF32::new(0.0));
        targets.push(ModTarget { name, value: Arc::clone(&value), owner: String::new(), claimed: true, last_use: Arc::new(AtomicU64::new(0)) });
        value
    }

    pub fn names(&self) -> Vec<String> {
        self.targets.lock().unwrap().iter().map(|t| t.name.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.targets.lock().unwrap().len()
    }

    /// Whether the app behind input `idx` has been built and registered it.
    #[allow(dead_code)] // tests and tools
    pub fn is_claimed(&self, idx: usize) -> bool {
        self.targets.lock().unwrap().get(idx).is_some_and(|t| t.claimed)
    }

    /// Index of a receiver by its full name.
    #[allow(dead_code)] // tests and tools
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.targets.lock().unwrap().iter().position(|t| t.name == name)
    }

    /// The apps that have inputs, in order of first appearance, each with
    /// its inputs' indices in registration order. A source's picker shows
    /// this as two levels: pick the app, then the input.
    pub fn apps(&self) -> Vec<(String, Vec<usize>)> {
        let targets = self.targets.lock().unwrap();
        let mut out: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, t) in targets.iter().enumerate() {
            let app = split_name(&t.name).0;
            match out.iter_mut().find(|(a, _)| a == app) {
                Some((_, v)) => v.push(i),
                None => out.push((app.to_string(), vec![i])),
            }
        }
        out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        out
    }

    /// True while something has written into one of `owner`'s declared
    /// inputs within the last second: the app should be awake to hear it.
    pub fn requested(&self, owner: &str) -> bool {
        let now = now_ms();
        self.targets.lock().unwrap().iter().any(|t| {
            t.owner == owner && {
                let used = t.last_use.load(Ordering::Relaxed);
                used != 0 && now.saturating_sub(used) < WAKE_MS
            }
        })
    }

    /// The handle to write into for target `idx`, or `None` once
    /// nothing is routed there (or the index is out of range).
    pub fn contribution(&self, idx: usize) -> Option<crate::util::AtomicContribution> {
        self.get(idx)?.contribution()
    }

    pub fn get(&self, idx: usize) -> Option<Arc<AtomicF32>> {
        let targets = self.targets.lock().unwrap();
        let t = targets.get(idx)?;
        if !t.owner.is_empty() {
            t.last_use.store(now_ms(), Ordering::Relaxed);
        }
        Some(Arc::clone(&t.value))
    }
}

impl Default for ModBus {
    fn default() -> Self {
        Self::new()
    }
}

/// A source's route, held as the usual `0 = none, else index + 1` and
/// edited in two steps: the App row moves between apps (landing on the
/// app's first input), the Input row moves only within the chosen app.
pub struct Patch;

impl Patch {
    /// Value for an "App" row.
    pub fn app_label(bus: &ModBus, route: usize) -> String {
        if route == 0 {
            return "None".into();
        }
        bus.names().get(route - 1).map_or_else(|| "None".into(), |n| split_name(n).0.to_string())
    }

    /// Value for an "Input" row.
    pub fn input_label(bus: &ModBus, route: usize) -> String {
        if route == 0 {
            return "--".into();
        }
        bus.names().get(route - 1).map_or_else(|| "--".into(), |n| split_name(n).1.to_string())
    }

    /// One-line "App > Input" summary.
    pub fn label(bus: &ModBus, route: usize) -> String {
        if route == 0 {
            return "None".into();
        }
        match bus.names().get(route - 1) {
            Some(n) => {
                let (a, p) = split_name(n);
                if a == p { a.to_string() } else { format!("{a} > {p}") }
            }
            None => "None".into(),
        }
    }

    /// Step the App row: None, then each app in turn, wrapping.
    pub fn step_app(bus: &ModBus, route: usize, delta: i32) -> usize {
        if delta == 0 {
            return route;
        }
        let apps = bus.apps();
        let cur = if route == 0 { 0 } else { apps.iter().position(|(_, v)| v.contains(&(route - 1))).map_or(0, |p| p + 1) };
        let next = (cur as i32 + delta.signum()).rem_euclid(apps.len() as i32 + 1) as usize;
        if next == 0 { 0 } else { apps[next - 1].1[0] + 1 }
    }

    /// The chosen app's first input (what resetting the Input row does).
    pub fn first_input(bus: &ModBus, route: usize) -> usize {
        if route == 0 {
            return 0;
        }
        bus.apps().iter().find(|(_, v)| v.contains(&(route - 1))).map_or(route, |(_, v)| v[0] + 1)
    }

    /// Step the Input row within the current app (wrapping). With no app
    /// chosen, it does nothing.
    pub fn step_input(bus: &ModBus, route: usize, delta: i32) -> usize {
        if delta == 0 || route == 0 {
            return route;
        }
        let apps = bus.apps();
        let Some((_, inputs)) = apps.iter().find(|(_, v)| v.contains(&(route - 1))) else { return route };
        let at = inputs.iter().position(|&i| i == route - 1).unwrap_or(0);
        let next = (at as i32 + delta.signum()).rem_euclid(inputs.len() as i32) as usize;
        inputs[next] + 1
    }
}

#[cfg(test)]
mod patch_tests {
    use super::*;

    fn bus() -> ModBus {
        let b = ModBus::new();
        b.register("Plaits: Harmonics");
        b.register("Plaits: Timbre");
        b.declare("bloom", "Bloom: Density");
        b.register("Clouds: Size");
        b.declare("bloom", "Bloom: Pitch");
        b
    }

    #[test]
    fn app_row_walks_apps_then_input_row_walks_that_apps_inputs() {
        let b = bus();
        let mut r = 0;
        r = Patch::step_app(&b, r, 1);
        assert_eq!((Patch::app_label(&b, r), Patch::input_label(&b, r)), ("Bloom".into(), "Density".into()));
        r = Patch::step_input(&b, r, 1);
        assert_eq!(Patch::label(&b, r), "Bloom > Pitch");
        r = Patch::step_input(&b, r, 1);
        assert_eq!(Patch::label(&b, r), "Bloom > Density", "input row wraps inside the app");
        r = Patch::step_app(&b, r, 1);
        assert_eq!(Patch::label(&b, r), "Clouds > Size");
        r = Patch::step_app(&b, r, 1);
        assert_eq!(Patch::label(&b, r), "Plaits > Harmonics");
        r = Patch::step_app(&b, r, 1);
        assert_eq!(r, 0, "past the last app is None");
        assert_eq!(Patch::step_app(&b, 0, -1), b.index_of("Plaits: Harmonics").unwrap() + 1);
        assert_eq!(Patch::step_input(&b, 0, 1), 0);
    }

    #[test]
    fn a_declared_input_keeps_its_handle_when_the_app_registers_it() {
        let b = ModBus::new();
        b.declare("plaits", "Plaits: Harmonics");
        let patched = b.get(0).unwrap();
        patched.set(0.5);
        let mine = b.register("Plaits: Harmonics");
        assert_eq!(mine.get(), 0.5);
        assert_eq!(b.len(), 1);
        // a second instance registering the same name gets its own
        b.register("Plaits: Harmonics");
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn writing_a_declared_input_requests_its_app() {
        let b = ModBus::new();
        b.declare("plaits", "Plaits: Harmonics");
        assert!(!b.requested("plaits"));
        b.names();
        b.apps();
        assert!(!b.requested("plaits"), "browsing doesn't wake anything");
        b.get(0);
        assert!(b.requested("plaits"));
        assert!(!b.requested("bloom"));
    }
}

#[cfg(test)] mod cable_tests {
    use super::*;
    #[test] fn cables_sum_with_existing_writers_and_disconnect_independently() {
        let bus=ModBus::new();let target=bus.register("test");target.set(0.2);
        let a=bus.contribution(0).unwrap();let b=bus.contribution(0).unwrap();a.set(0.3);b.set(-0.1);
        assert!((target.get()-0.4).abs()<1e-6);
        target.set(0.5);assert!((target.get()-0.7).abs()<1e-6);
        drop(a);assert!((target.get()-0.4).abs()<1e-6);drop(b);assert!((target.get()-0.5).abs()<1e-6);
    }
    #[test] fn cable_slots_are_bounded_reusable_and_reject_nan() {
        let bus=ModBus::new();let target=bus.register("test");let mut cables:Vec<_>=(0..16).map(|_|bus.contribution(0).unwrap()).collect();
        assert!(bus.contribution(0).is_none());cables[0].set(f32::NAN);assert_eq!(target.get(),0.);cables.pop();assert!(bus.contribution(0).is_some());
    }
}
