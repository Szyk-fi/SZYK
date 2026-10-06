//! The arc: one shared monome-style arc that apps play, the grid's sibling
//! (see grid_kit.rs). It shows on screen (the Arc app, `apps/arc.rs`) and on
//! a real monome arc when one is plugged in, both at once.
//!
//! An arc is endless encoders, each inside a ring of 64 LEDs with 16 levels
//! (0 = off, 15 = full). Turning an encoder sends deltas (+1 for a step
//! clockwise, -1 anticlockwise, 1024 steps a turn on the hardware); pushing it
//! (on arcs that have the push) sends a key down and up. Like a grid, the arc
//! does nothing itself: the app holding it (its *focus*) reads the turns and
//! draws every ring. LED 0 is at the top, counting clockwise.
//!
//! Four encoders by default (an arc 4), or two (an arc 2); with Follow
//! hardware on, a plugged-in arc sets the count. The transports are the
//! grid's: serialosc (`/enc/delta`, `/enc/key`, `/ring/map`) and monome's
//! serial protocol straight over USB (encoder deltas 0x50, keys 0x51/0x52,
//! ring maps 0x92), both in grid_kit.rs.
//!
//! Threads: the UI thread and the transport threads share it through one
//! mutex; the audio thread never touches it.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

pub const LEDS: usize = 64;
pub const MAX_ENCODERS: usize = 4;
/// Events waiting for the focused app before the oldest are dropped.
const MAX_EVENTS: usize = 1024;

/// Every ring's 64 levels.
pub type Rings = [[u8; LEDS]; MAX_ENCODERS];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Encoder `n` turned `d` steps (positive is clockwise).
    Delta { n: usize, d: i32 },
    /// Encoder `n`'s push went down or up.
    Key { n: usize, down: bool },
}

/// A real arc a transport found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub kind: String,
    pub encoders: usize,
}

struct State {
    encoders: usize,
    follow: bool,
    leds: Rings,
    held: [bool; MAX_ENCODERS],
    gen: u64,
    events: VecDeque<Event>,
    clients: Vec<String>,
    focus: Option<String>,
    device: Option<Device>,
}

/// What the screen needs to draw the arc.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub encoders: usize,
    pub leds: Rings,
    pub held: [bool; MAX_ENCODERS],
    pub focus: Option<String>,
    pub device: Option<Device>,
}

pub struct ArcHub {
    s: Mutex<State>,
}

impl Default for ArcHub {
    fn default() -> Self {
        ArcHub::new()
    }
}

impl ArcHub {
    pub fn new() -> ArcHub {
        ArcHub {
            s: Mutex::new(State {
                encoders: MAX_ENCODERS,
                follow: true,
                leds: [[0; LEDS]; MAX_ENCODERS],
                held: [false; MAX_ENCODERS],
                gen: 1,
                events: VecDeque::new(),
                clients: Vec::new(),
                focus: None,
                device: None,
            }),
        }
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn encoders(&self) -> usize {
        self.st().encoders
    }

    /// Two or four encoders (anything else rounds to one of those).
    pub fn set_encoders(&self, n: usize) {
        let mut s = self.st();
        Self::resize(&mut s, n);
    }

    fn resize(s: &mut State, n: usize) {
        let n = if n <= 2 { 2 } else { 4 };
        if n != s.encoders {
            s.encoders = n;
            s.held = [false; MAX_ENCODERS];
            s.events.clear();
            s.gen += 1;
        }
    }

    pub fn follows_device(&self) -> bool {
        self.st().follow
    }

    pub fn set_follow(&self, on: bool) {
        let mut s = self.st();
        s.follow = on;
        if let (true, Some(n)) = (on, s.device.as_ref().map(|d| d.encoders)) {
            Self::resize(&mut s, n);
        }
    }

    /// An app that can play the arc. The first one to ask gets it.
    pub fn register(&self, name: &str) {
        let mut s = self.st();
        if !s.clients.iter().any(|c| c == name) {
            s.clients.push(name.to_string());
        }
        if s.focus.is_none() {
            s.focus = Some(name.to_string());
        }
    }

    pub fn clients(&self) -> Vec<String> {
        self.st().clients.clone()
    }

    pub fn focus(&self) -> Option<String> {
        self.st().focus.clone()
    }

    /// Hands the arc to another app: the rings go dark and held pushes are
    /// forgotten, so the new app never sees a release it didn't press.
    pub fn set_focus(&self, name: &str) {
        let mut s = self.st();
        if s.focus.as_deref() == Some(name) || !s.clients.iter().any(|c| c == name) {
            return;
        }
        s.focus = Some(name.to_string());
        s.events.clear();
        s.held = [false; MAX_ENCODERS];
        s.leds = [[0; LEDS]; MAX_ENCODERS];
        s.gen += 1;
    }

    fn push(s: &mut State, e: Event) {
        if s.events.len() >= MAX_EVENTS {
            s.events.pop_front();
        }
        s.events.push_back(e);
    }

    /// Encoder `n` turned `d` steps (from the screen or the hardware).
    pub fn turn(&self, n: usize, d: i32) {
        let mut s = self.st();
        if n >= s.encoders || d == 0 {
            return;
        }
        // Consecutive turns of one encoder merge, so a fast spin can't
        // flood the queue.
        if let Some(Event::Delta { n: last, d: dd }) = s.events.back_mut() {
            if *last == n && (*dd > 0) == (d > 0) {
                *dd += d;
                return;
            }
        }
        Self::push(&mut s, Event::Delta { n, d });
    }

    /// Encoder `n`'s push. A repeat of the state it's in is ignored.
    pub fn key(&self, n: usize, down: bool) {
        let mut s = self.st();
        if n >= s.encoders || s.held[n] == down {
            return;
        }
        s.held[n] = down;
        Self::push(&mut s, Event::Key { n, down });
        s.gen += 1;
    }

    /// The turns and pushes waiting for `name`, if it holds the arc.
    pub fn events(&self, name: &str) -> Vec<Event> {
        let mut s = self.st();
        if s.focus.as_deref() != Some(name) {
            return Vec::new();
        }
        s.events.drain(..).collect()
    }

    /// `name`'s rings, if it holds the arc. Returns whether they showed.
    pub fn show(&self, name: &str, rings: &Rings) -> bool {
        let mut s = self.st();
        if s.focus.as_deref() != Some(name) {
            return false;
        }
        let mut r = *rings;
        r.iter_mut().flatten().for_each(|l| *l = (*l).min(15));
        for ring in r.iter_mut().skip(s.encoders) {
            *ring = [0; LEDS];
        }
        if r != s.leds {
            s.leds = r;
            s.gen += 1;
        }
        true
    }

    pub fn snapshot(&self) -> Snapshot {
        let s = self.st();
        Snapshot { encoders: s.encoders, leds: s.leds, held: s.held, focus: s.focus.clone(), device: s.device.clone() }
    }

    pub fn device(&self) -> Option<Device> {
        self.st().device.clone()
    }

    pub fn set_device(&self, d: Option<Device>) {
        let mut s = self.st();
        if let (true, Some(n)) = (s.follow, d.as_ref().map(|d| d.encoders)) {
            Self::resize(&mut s, n);
        }
        s.device = d;
        s.gen += 1;
    }

    /// The hardware's view: its encoder count, the rings and a change count.
    pub fn hardware_frame(&self) -> Option<(usize, Rings, u64)> {
        let s = self.st();
        let d = s.device.as_ref()?;
        Some((d.encoders.min(MAX_ENCODERS), s.leds, s.gen))
    }
}

/// The one arc, without starting the hardware search (grid_kit's `grid()`
/// starts that, for both).
#[cfg(not(test))]
pub fn hub() -> Arc<ArcHub> {
    static ARC: std::sync::OnceLock<Arc<ArcHub>> = std::sync::OnceLock::new();
    Arc::clone(ARC.get_or_init(|| Arc::new(ArcHub::new())))
}

/// Under test each test thread gets its own arc.
#[cfg(test)]
pub fn hub() -> Arc<ArcHub> {
    thread_local! {
        static ARC: Arc<ArcHub> = Arc::new(ArcHub::new());
    }
    ARC.with(Arc::clone)
}

/// The one arc. Opening it also starts looking for a real arc (and grid).
pub fn arc() -> Arc<ArcHub> {
    let _ = super::grid_kit::grid();
    hub()
}

/// Draws the way monome's libraries do, for apps' convenience.
pub mod draw {
    use super::LEDS;

    /// LEDs `from` to `to` inclusive, going clockwise and wrapping past 63.
    #[allow(dead_code)] // not used by the main binary
    pub fn range(ring: &mut [u8; LEDS], from: i32, to: i32, level: u8) {
        let (a, b) = (from.rem_euclid(LEDS as i32), to.rem_euclid(LEDS as i32));
        let n = (b - a).rem_euclid(LEDS as i32);
        for k in 0..=n {
            ring[((a + k) % LEDS as i32) as usize] = level;
        }
    }

    /// An anti-aliased arc segment from angle `a1` to `a2` (radians, 0 at
    /// the top, clockwise), the way norns' `arc:segment` works: the whole
    /// ring is redrawn, each LED lit by how much of it the segment covers.
    pub fn segment(ring: &mut [u8; LEDS], a1: f64, a2: f64, level: u8) {
        let tau = std::f64::consts::TAU;
        let step = tau / LEDS as f64;
        let (a, mut b) = (a1.rem_euclid(tau), a2.rem_euclid(tau));
        if b < a {
            b += tau;
        }
        for (i, led) in ring.iter_mut().enumerate() {
            let mut cover = 0.0;
            for wrap in [0.0, tau] {
                let (l0, l1) = (i as f64 * step + wrap, (i + 1) as f64 * step + wrap);
                cover += (l1.min(b) - l0.max(a)).max(0.0) / step;
            }
            *led = (cover.min(1.0) * level.min(15) as f64).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_and_pushes_belong_to_the_app_holding_the_arc() {
        let a = ArcHub::new();
        a.register("Norns");
        a.register("Other");
        a.turn(0, 1);
        a.turn(0, 2);
        a.turn(0, -1);
        a.turn(3, 5);
        a.turn(7, 1); // no such encoder
        a.key(1, true);
        a.key(1, true);
        assert!(a.events("Other").is_empty());
        assert_eq!(a.events("Norns"), [Event::Delta { n: 0, d: 3 }, Event::Delta { n: 0, d: -1 }, Event::Delta { n: 3, d: 5 }, Event::Key { n: 1, down: true }]);
        let mut r = [[0u8; LEDS]; MAX_ENCODERS];
        r[2][10] = 20;
        assert!(!a.show("Other", &r));
        assert!(a.show("Norns", &r));
        assert_eq!(a.snapshot().leds[2][10], 15);
        a.set_focus("Other");
        let s = a.snapshot();
        assert_eq!((s.leds[2][10], s.held[1]), (0, false));
    }

    #[test]
    fn an_arc_2_has_two_rings() {
        let a = ArcHub::new();
        a.register("x");
        a.set_device(Some(Device { id: "m0000001".into(), kind: "monome arc 2".into(), encoders: 2 }));
        assert_eq!(a.encoders(), 2);
        a.turn(3, 1);
        assert!(a.events("x").is_empty());
        let r = [[9u8; LEDS]; MAX_ENCODERS];
        a.show("x", &r);
        assert_eq!((a.snapshot().leds[1][0], a.snapshot().leds[2][0]), (9, 0));
    }

    #[test]
    fn ranges_wrap_and_segments_fade_at_their_ends() {
        let mut r = [0u8; LEDS];
        draw::range(&mut r, 62, 1, 7);
        assert_eq!((r[61], r[62], r[63], r[0], r[1], r[2]), (0, 7, 7, 7, 7, 0));
        let mut r = [0u8; LEDS];
        let step = std::f64::consts::TAU / 64.0;
        draw::segment(&mut r, 2.0 * step, 4.5 * step, 15);
        assert_eq!(&r[1..6], &[0, 15, 15, 8, 0]);
    }
}
