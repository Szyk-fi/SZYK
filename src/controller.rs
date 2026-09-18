//! Shared state written by the MIDI listener thread (main.rs) and read
//! once per frame by `Input::poll` — this is how an external controller
//! (an Ableton Push 2 used as a grid/knob controller, in this case) feeds
//! the same `Input` the keyboard does. See main.rs for the exact note/CC
//! numbers this expects from the controller.
//!
//! `grid` is level state (mirrors Note On/Off directly, same semantics as
//! the keyboard's held-key state). `top`/`knob*_press` are latched: the
//! MIDI callback sets them true, and `Input::poll` consumes (clears) them
//! once per frame so a single press reads as exactly one edge no matter
//! how many render frames land between the Note On and the next poll.
//! `knob*_delta` accumulates encoder ticks the same way.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// Ableton Push 2 note map for the region you picked (per your chart):
/// bottom-left 4x4 pads = the note grid, the row directly above = the 4
/// top buttons. Shared between MIDI input parsing (main.rs's
/// `handle_midi_message`) and LED output (`led_output.rs`/os.rs) so
/// both sides agree on which physical pad a given index means --
/// duplicating this table between "what note means pad 5" and "what
/// note lights pad 5" would drift the moment one got tuned against
/// real hardware and the other didn't.
///
/// Assumes scientific pitch notation (C4 = MIDI 60) converting your
/// chart's note names to numbers. Ableton's own UI sometimes labels
/// middle C as "C3" instead of "C4" -- if pads don't respond (or light
/// up in the wrong place), watch the "midi: unmapped note/CC" lines
/// while you press things, and shift these tables by the offset you
/// see (probably +/-12 for notes).
pub const GRID_NOTES: [u8; 16] = [
    60, 61, 62, 63, // C4 C#4 D4 D#4
    52, 53, 54, 55, // E3 F3 F#3 G3
    44, 45, 46, 47, // G#2 A2 A#2 B2
    36, 37, 38, 39, // C2 C#2 D2 D#2
];
pub const TOP_NOTES: [u8; 4] = [68, 69, 70, 71]; // G#4 A4 A#4 B4

pub struct ControllerState {
    pub grid: [AtomicBool; 16],
    top: [AtomicBool; 4],
    knob1_delta: AtomicI32,
    knob2_delta: AtomicI32,
    knob1_press: AtomicBool,
    knob2_press: AtomicBool,
    home: AtomicBool,
}

impl ControllerState {
    pub fn new() -> Self {
        Self {
            grid: std::array::from_fn(|_| AtomicBool::new(false)),
            top: std::array::from_fn(|_| AtomicBool::new(false)),
            knob1_delta: AtomicI32::new(0),
            knob2_delta: AtomicI32::new(0),
            knob1_press: AtomicBool::new(false),
            knob2_press: AtomicBool::new(false),
            home: AtomicBool::new(false),
        }
    }

    pub fn set_top(&self, i: usize) {
        self.top[i].store(true, Ordering::Relaxed);
    }
    pub fn set_home(&self) {
        self.home.store(true, Ordering::Relaxed);
    }
    pub fn take_home(&self) -> bool {
        self.home.swap(false, Ordering::Relaxed)
    }
    pub fn add_knob1_delta(&self, delta: i32) {
        self.knob1_delta.fetch_add(delta, Ordering::Relaxed);
    }
    pub fn add_knob2_delta(&self, delta: i32) {
        self.knob2_delta.fetch_add(delta, Ordering::Relaxed);
    }
    pub fn set_knob1_press(&self) {
        self.knob1_press.store(true, Ordering::Relaxed);
    }
    pub fn set_knob2_press(&self) {
        self.knob2_press.store(true, Ordering::Relaxed);
    }

    pub fn take_top(&self, i: usize) -> bool {
        self.top[i].swap(false, Ordering::Relaxed)
    }
    pub fn take_knob1_delta(&self) -> i32 {
        self.knob1_delta.swap(0, Ordering::Relaxed)
    }
    pub fn take_knob2_delta(&self) -> i32 {
        self.knob2_delta.swap(0, Ordering::Relaxed)
    }
    pub fn take_knob1_press(&self) -> bool {
        self.knob1_press.swap(false, Ordering::Relaxed)
    }
    pub fn take_knob2_press(&self) -> bool {
        self.knob2_press.swap(false, Ordering::Relaxed)
    }
}
