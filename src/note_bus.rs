//! The note bus: any app that makes notes can play any app that takes them.
//!
//! The ModBus carries continuous values and the AudioBus carries sound; this
//! carries *notes*. Two sides:
//!
//! - **Instruments** (manifest `notes_in = true`) are declared at startup,
//!   one inbox each, in stable slots. An instrument never has to know the
//!   bus exists: the app runtime (app_runtime.rs) reads its inbox every
//!   frame and hands the held notes to the app exactly like a MIDI keyboard
//!   (`Input::midi_keys`), on screen or not. So every instrument that
//!   already plays from a keyboard plays from Bloom, Swarm or a sequencer
//!   too, and a new instrument gets all of them for free.
//! - **Note sources** (manifest `note_outputs = [...]`) each hold a route:
//!   their own built-in voices (`INTERNAL`), nothing (`NONE`, the notes only
//!   drive whatever else the app sends, e.g. CV), or an instrument slot.
//!   A source sends through a `NoteOut`, which is safe on the audio thread
//!   (no locks, no allocation) and releases its notes when the route
//!   changes, so nothing is left hanging.
//!
//! Inboxes count holders per note, so two sources (or a source and the
//! pads) can hold the same note, and keep a strike counter, so a note that
//! starts and ends between two polls still sounds.
//!
//! Devices are just more endpoints: a MIDI output port registers an
//! instrument ("MIDI Out: <port>") and a MIDI input port a source, so a new
//! controller or synth shows up in every picker the moment it's plugged in.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// Instrument slots. Fixed, so the audio thread can index without locks.
pub const MAX_INSTRUMENTS: usize = 96;
/// Route: the source plays its own built-in voices.
pub const INTERNAL: usize = usize::MAX;
/// Route: the source plays nothing (it may still send CV/gates).
pub const NONE: usize = usize::MAX - 1;

/// How long after the last note an instrument stays awake with no note held.
const WAKE_MS: u64 = 1500;

fn now_ms() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// One instrument's incoming notes. Per MIDI note, packed in one atomic:
/// bits 0..8 how many holders, 8..16 the last velocity, 16..32 a strike
/// counter bumped on every note-on.
pub struct NoteInbox {
    notes: [AtomicU32; 128],
    last_use: AtomicU64,
}

impl Default for NoteInbox {
    fn default() -> Self {
        Self { notes: std::array::from_fn(|_| AtomicU32::new(0)), last_use: AtomicU64::new(0) }
    }
}

impl NoteInbox {
    pub fn note_on(&self, note: u8, velocity: u8) {
        let Some(slot) = self.notes.get(note as usize) else { return };
        let vel = velocity.clamp(1, 127) as u32;
        let _ = slot.fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
            let count = (v & 0xff).saturating_add(1).min(255);
            let seq = (v >> 16).wrapping_add(1) & 0xffff;
            Some(count | (vel << 8) | (seq << 16))
        });
        self.last_use.store(now_ms(), Ordering::Relaxed);
    }

    pub fn note_off(&self, note: u8) {
        let Some(slot) = self.notes.get(note as usize) else { return };
        let _ = slot.fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| Some((v & !0xff) | (v & 0xff).saturating_sub(1)));
        self.last_use.store(now_ms(), Ordering::Relaxed);
    }

    /// Any note held right now.
    pub fn any_held(&self) -> bool {
        self.notes.iter().any(|n| n.load(Ordering::Relaxed) & 0xff != 0)
    }

    /// Whether this instrument should be awake: a note held, or one played
    /// in the last moment (so its release tail finishes).
    pub fn busy(&self) -> bool {
        self.any_held() || {
            let t = self.last_use.load(Ordering::Relaxed);
            t != 0 && now_ms().saturating_sub(t) < WAKE_MS
        }
    }

    /// Fold what changed since `view` last looked into keyboard state.
    /// A note struck and released between two polls is held for this one
    /// poll and released on the next, so it is never lost.
    pub fn poll(&self, view: &mut NoteView) {
        for n in 0..128 {
            let cur = self.notes[n].load(Ordering::Acquire);
            let last = view.last[n];
            let (count, vel, seq) = (cur & 0xff, ((cur >> 8) & 0x7f) as u8, cur >> 16);
            if seq != last >> 16 {
                view.keys[n] = vel.max(1);
                view.strikes[n] = view.strikes[n].wrapping_add(1);
                view.release_next[n] = count == 0;
            } else if view.release_next[n] || count == 0 {
                view.keys[n] = 0;
                view.release_next[n] = false;
            }
            view.last[n] = cur;
        }
    }
}

/// An instrument's view of its inbox: the notes held as of the last poll
/// (velocity per MIDI note, 0 = off), like `Input::midi_keys`.
#[derive(Clone)]
pub struct NoteView {
    pub keys: [u8; 128],
    /// Note-ons seen per note, so a re-strike of a held note is visible.
    pub strikes: [u8; 128],
    last: [u32; 128],
    release_next: [bool; 128],
}

impl Default for NoteView {
    fn default() -> Self {
        Self { keys: [0; 128], strikes: [0; 128], last: [0; 128], release_next: [false; 128] }
    }
}

impl NoteView {
    pub fn any(&self) -> bool {
        self.keys.iter().any(|v| *v != 0)
    }
}

struct InstrumentMeta {
    name: String,
    owner: String,
    claimed: bool,
}

struct SourceMeta {
    name: String,
    owner: String,
    route: Arc<AtomicUsize>,
    claimed: bool,
    /// The source has voices of its own (`INTERNAL` is a choice).
    has_internal: bool,
}

pub struct NoteBus {
    inboxes: Arc<Vec<NoteInbox>>,
    instruments: Mutex<Vec<InstrumentMeta>>,
    sources: Mutex<Vec<SourceMeta>>,
}

impl Default for NoteBus {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)] // the preview binaries use a subset
impl NoteBus {
    pub fn new() -> Self {
        Self { inboxes: Arc::new((0..MAX_INSTRUMENTS).map(|_| NoteInbox::default()).collect()), instruments: Mutex::new(Vec::new()), sources: Mutex::new(Vec::new()) }
    }

    /// Declares an installed app's instrument before the app exists, so
    /// sources can pick it. Returns its slot. Idempotent per name.
    pub fn declare_instrument(&self, owner: &str, name: &str) -> Option<usize> {
        let mut list = self.instruments.lock().unwrap();
        if let Some(i) = list.iter().position(|m| m.name == name) {
            return Some(i);
        }
        if list.len() >= MAX_INSTRUMENTS {
            eprintln!("notes: no slot left for instrument {name:?}");
            return None;
        }
        list.push(InstrumentMeta { name: name.into(), owner: owner.into(), claimed: false });
        Some(list.len() - 1)
    }

    /// An instrument's inbox (declaring it if it wasn't). For apps and
    /// devices that read their own notes rather than through the runtime.
    pub fn register_instrument(&self, owner: &str, name: &str) -> Option<Arc<NoteInboxRef>> {
        let idx = self.declare_instrument(owner, name)?;
        self.instruments.lock().unwrap()[idx].claimed = true;
        Some(Arc::new(NoteInboxRef { inboxes: Arc::clone(&self.inboxes), idx }))
    }

    /// The slot of an instrument by name.
    pub fn instrument_index(&self, name: &str) -> Option<usize> {
        self.instruments.lock().unwrap().iter().position(|m| m.name == name)
    }

    pub fn inbox(&self, idx: usize) -> Option<&NoteInbox> {
        self.inboxes.get(idx).filter(|_| idx < self.instruments.lock().unwrap().len())
    }

    /// A shareable handle to slot `idx`'s inbox.
    pub fn inbox_ref(&self, idx: usize) -> NoteInboxRef {
        NoteInboxRef { inboxes: Arc::clone(&self.inboxes), idx }
    }

    /// (slot, name, owner app id) of every instrument, in slot order.
    pub fn instruments(&self) -> Vec<(usize, String, String)> {
        self.instruments.lock().unwrap().iter().enumerate().map(|(i, m)| (i, m.name.clone(), m.owner.clone())).collect()
    }

    /// Whether any instrument owned by `owner` has notes for it.
    pub fn requested(&self, owner: &str) -> bool {
        let list = self.instruments.lock().unwrap();
        list.iter().enumerate().any(|(i, m)| m.owner == owner && self.inboxes[i].busy())
    }

    /// Declares a note output before its app exists, so routing UIs can
    /// list it. Routes start at `INTERNAL`.
    pub fn declare_source(&self, owner: &str, name: &str) {
        let mut list = self.sources.lock().unwrap();
        if !list.iter().any(|s| s.name == name) {
            list.push(SourceMeta { name: name.into(), owner: owner.into(), route: Arc::new(AtomicUsize::new(INTERNAL)), claimed: false, has_internal: true });
        }
    }

    /// A note output for an app to send through. The route is shared with
    /// every routing UI (the app's own row, Portal's Notes page).
    pub fn register_source(&self, name: &str) -> NoteOut {
        self.register_source_routed(name, INTERNAL)
    }

    /// As `register_source`, with the route a newly made source starts on
    /// (an app with no voices of its own starts on `NONE`).
    pub fn register_source_routed(&self, name: &str, initial: usize) -> NoteOut {
        let mut list = self.sources.lock().unwrap();
        let route = match list.iter_mut().find(|s| s.name == name) {
            Some(s) => {
                if !s.claimed && initial != INTERNAL {
                    s.route.store(initial, Ordering::Relaxed);
                }
                s.claimed = true;
                s.has_internal = initial == INTERNAL;
                Arc::clone(&s.route)
            }
            None => {
                let route = Arc::new(AtomicUsize::new(initial));
                list.push(SourceMeta { name: name.into(), owner: String::new(), route: Arc::clone(&route), claimed: true, has_internal: initial == INTERNAL });
                route
            }
        };
        NoteOut::new(Arc::clone(&self.inboxes), route)
    }

    /// (name, owner, route) of every note source.
    pub fn sources(&self) -> Vec<(String, String, Arc<AtomicUsize>)> {
        self.sources.lock().unwrap().iter().map(|s| (s.name.clone(), s.owner.clone(), Arc::clone(&s.route))).collect()
    }

    /// Step note source `index`'s route (for routing pages like Portal's).
    pub fn step_source(&self, index: usize, dir: i32) {
        let (route, owner, has_internal) = {
            let list = self.sources.lock().unwrap();
            let Some(s) = list.get(index) else { return };
            (Arc::clone(&s.route), s.owner.clone(), s.has_internal)
        };
        route.store(self.step_route(route.load(Ordering::Relaxed), dir, &owner, has_internal), Ordering::Relaxed);
    }

    /// Display name of a route.
    pub fn route_name(&self, route: usize) -> String {
        match route {
            INTERNAL => "Own sound".into(),
            NONE => "None (signal only)".into(),
            i => self.instruments.lock().unwrap().get(i).map(|m| m.name.clone()).unwrap_or_else(|| "?".into()),
        }
    }

    /// The next route from `current`, stepping through `INTERNAL` (only if
    /// the source has voices of its own), `NONE`, then every instrument
    /// except the source's own app.
    pub fn step_route(&self, current: usize, dir: i32, self_owner: &str, has_internal: bool) -> usize {
        let mut choices = Vec::new();
        if has_internal {
            choices.push(INTERNAL);
        }
        choices.push(NONE);
        for (i, m) in self.instruments.lock().unwrap().iter().enumerate() {
            if m.owner != self_owner || self_owner.is_empty() {
                choices.push(i);
            }
        }
        let pos = choices.iter().position(|c| *c == current).unwrap_or(0) as i32;
        choices[(pos + dir.signum()).rem_euclid(choices.len() as i32) as usize]
    }
}

/// The UI side of a note source: what its "Plays" row shows and steps.
/// Made together with the `NoteOut` the source's audio thread sends on.
pub struct NoteRoute {
    bus: Option<Arc<NoteBus>>,
    route: Arc<AtomicUsize>,
    owner: String,
    has_internal: bool,
}

#[allow(dead_code)] // each source uses what it needs
impl NoteRoute {
    /// A route named `name` for app `owner`. `has_internal`: the app has
    /// voices of its own (else it starts on `NONE`). With no bus (a
    /// preview binary) the source just plays itself.
    pub fn new(bus: Option<Arc<NoteBus>>, name: &str, owner: &str, has_internal: bool) -> (Self, NoteOut) {
        let initial = if has_internal { INTERNAL } else { NONE };
        let out = match bus.as_ref() {
            Some(b) => b.register_source_routed(name, initial),
            None => {
                let o = NoteOut::detached();
                o.set_route(initial);
                o
            }
        };
        (Self { bus, route: out.route_handle(), owner: owner.into(), has_internal }, out)
    }

    pub fn route(&self) -> usize {
        self.route.load(Ordering::Relaxed)
    }

    pub fn label(&self) -> String {
        match &self.bus {
            Some(b) => b.route_name(self.route()),
            None => if self.has_internal { "Own sound".into() } else { "None (signal only)".into() },
        }
    }

    pub fn step(&self, dir: i32) {
        if let Some(b) = &self.bus {
            self.route.store(b.step_route(self.route(), dir, &self.owner, self.has_internal), Ordering::Relaxed);
        }
    }

    pub fn reset(&self) {
        self.route.store(if self.has_internal { INTERNAL } else { NONE }, Ordering::Relaxed);
    }

    /// Sends somewhere other than its own voices.
    pub fn external(&self) -> bool {
        self.route() < MAX_INSTRUMENTS
    }
}

/// A shareable handle to one inbox (devices and apps that read their own).
pub struct NoteInboxRef {
    inboxes: Arc<Vec<NoteInbox>>,
    idx: usize,
}

impl std::ops::Deref for NoteInboxRef {
    type Target = NoteInbox;
    fn deref(&self) -> &NoteInbox {
        &self.inboxes[self.idx]
    }
}

/// A source's sending end. Lock- and allocation-free, for the audio thread.
/// Tracks what it holds so a route change (or `all_off`) releases exactly
/// its own notes, and can time note-offs for one-shot triggers.
pub struct NoteOut {
    inboxes: Arc<Vec<NoteInbox>>,
    route: Arc<AtomicUsize>,
    /// Holds per note on `held_on`.
    held: [u8; 128],
    held_on: usize,
    /// Samples left before an automatic note-off, per note (0 = none).
    timers: [u32; 128],
}

#[allow(dead_code)] // each source uses what it needs
impl NoteOut {
    fn new(inboxes: Arc<Vec<NoteInbox>>, route: Arc<AtomicUsize>) -> Self {
        let held_on = route.load(Ordering::Relaxed);
        Self { inboxes, route, held: [0; 128], held_on, timers: [0; 128] }
    }

    /// A detached output that goes nowhere (tests, previews).
    pub fn detached() -> Self {
        Self::new(Arc::new(Vec::new()), Arc::new(AtomicUsize::new(INTERNAL)))
    }

    pub fn route(&self) -> usize {
        self.route.load(Ordering::Relaxed)
    }

    pub fn route_handle(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.route)
    }

    pub fn set_route(&self, route: usize) {
        self.route.store(route, Ordering::Relaxed);
    }

    /// The app should sound its own voices.
    pub fn internal(&self) -> bool {
        self.route() == INTERNAL
    }

    /// Notes go to another app.
    pub fn external(&self) -> bool {
        self.route() < MAX_INSTRUMENTS
    }

    /// Releases everything if the route moved since the last call.
    fn follow_route(&mut self) {
        let r = self.route();
        if r != self.held_on {
            self.all_off();
            self.held_on = r;
        }
    }

    fn inbox(&self) -> Option<&NoteInbox> {
        self.inboxes.get(self.held_on)
    }

    pub fn note_on(&mut self, note: u8, velocity: u8) {
        self.follow_route();
        let n = note.min(127) as usize;
        if let Some(inbox) = self.inboxes.get(self.held_on) {
            inbox.note_on(n as u8, velocity);
            self.held[n] = self.held[n].saturating_add(1);
        }
    }

    pub fn note_off(&mut self, note: u8) {
        self.follow_route();
        let n = note.min(127) as usize;
        if self.held[n] > 0 {
            if let Some(inbox) = self.inbox() {
                inbox.note_off(n as u8);
            }
            self.held[n] -= 1;
        }
    }

    /// A note that ends by itself after `gate_samples` (for sources that
    /// only make triggers). Re-triggering restarts it.
    pub fn trigger(&mut self, note: u8, velocity: u8, gate_samples: u32) {
        let n = note.min(127) as usize;
        if self.timers[n] > 0 {
            self.note_off(n as u8);
        }
        self.note_on(n as u8, velocity);
        self.timers[n] = gate_samples.max(1);
    }

    /// Advance trigger gates by `samples` (call once per audio block).
    pub fn advance(&mut self, samples: u32) {
        self.follow_route();
        for n in 0..128 {
            let t = self.timers[n];
            if t > 0 {
                if t <= samples {
                    self.timers[n] = 0;
                    self.note_off(n as u8);
                } else {
                    self.timers[n] = t - samples;
                }
            }
        }
    }

    /// Release every note this output holds.
    pub fn all_off(&mut self) {
        let target = self.held_on;
        for n in 0..128 {
            while self.held[n] > 0 {
                if let Some(inbox) = self.inboxes.get(target) {
                    inbox.note_off(n as u8);
                }
                self.held[n] -= 1;
            }
            self.timers[n] = 0;
        }
    }

    /// Whether it holds anything right now.
    pub fn holding(&self) -> bool {
        self.held.iter().any(|h| *h > 0)
    }
}

impl Drop for NoteOut {
    fn drop(&mut self) {
        self.all_off();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_plays_a_declared_instrument_and_releases_on_reroute() {
        let bus = NoteBus::new();
        let a = bus.declare_instrument("voltage", "Voltage").unwrap();
        let b = bus.declare_instrument("plaits", "Plaits").unwrap();
        let mut out = bus.register_source("Bloom");
        assert!(out.internal(), "sources start on their own sound");
        out.set_route(a);
        out.note_on(60, 100);
        let mut view = NoteView::default();
        bus.inbox(a).unwrap().poll(&mut view);
        assert_eq!(view.keys[60], 100);
        assert!(bus.requested("voltage"));
        assert!(!bus.requested("plaits"));
        // re-route: the held note is released on the old instrument
        out.set_route(b);
        out.note_on(64, 90);
        bus.inbox(a).unwrap().poll(&mut view);
        assert_eq!(view.keys[60], 0, "nothing left hanging on Voltage");
        assert!(bus.inbox(b).unwrap().any_held());
    }

    #[test]
    fn a_note_shorter_than_a_poll_still_sounds_for_one_poll() {
        let bus = NoteBus::new();
        let a = bus.declare_instrument("x", "X").unwrap();
        let mut out = bus.register_source("S");
        out.set_route(a);
        out.note_on(48, 80);
        out.note_off(48);
        let mut view = NoteView::default();
        bus.inbox(a).unwrap().poll(&mut view);
        assert_eq!(view.keys[48], 80, "the strike is seen");
        bus.inbox(a).unwrap().poll(&mut view);
        assert_eq!(view.keys[48], 0, "and released on the next poll");
    }

    #[test]
    fn triggers_end_by_themselves_and_two_holders_share_a_note() {
        let bus = NoteBus::new();
        let a = bus.declare_instrument("x", "X").unwrap();
        let mut s1 = bus.register_source("S1");
        let mut s2 = bus.register_source("S2");
        s1.set_route(a);
        s2.set_route(a);
        s1.trigger(60, 100, 480);
        s2.note_on(60, 100);
        s1.advance(512);
        assert!(bus.inbox(a).unwrap().any_held(), "S2 still holds it");
        s2.note_off(60);
        assert!(!bus.inbox(a).unwrap().any_held());
    }

    #[test]
    fn route_choices_skip_the_sources_own_app_and_offer_none() {
        let bus = NoteBus::new();
        bus.declare_instrument("bloom", "Bloom");
        let v = bus.declare_instrument("voltage", "Voltage").unwrap();
        let r = bus.step_route(INTERNAL, 1, "bloom", true);
        assert_eq!(r, NONE);
        assert_eq!(bus.step_route(r, 1, "bloom", true), v, "Bloom's own instrument is skipped");
        assert_eq!(bus.step_route(v, 1, "bloom", true), INTERNAL, "wraps around");
        assert_eq!(bus.route_name(NONE), "None (signal only)");
    }
}
