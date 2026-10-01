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

use crate::util::AtomicF32;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};

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
    // --- Play surface (see `Input`'s play-surface fields). Written by the
    // gamepad and MIDI threads, read once per frame. ---
    /// Set by the UI each frame: the app on screen plays the surface
    /// itself, so MIDI notes go to `midi_keys` (not the pads) and the
    /// gamepad's stick/shoulders/triggers stop navigating.
    pub play_surface: AtomicBool,
    pub stick: [AtomicF32; 2],
    stick_click: AtomicBool,
    pub hands: [AtomicF32; 2],
    pub shoulders: [AtomicBool; 2],
    shoulder_press: [AtomicBool; 2],
    pub midi_keys: [AtomicU8; 128],
    pub pitch_bend: AtomicF32,
    pub mod_wheel: AtomicF32,
    pub aftertouch: AtomicF32,
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
            play_surface: AtomicBool::new(false),
            stick: std::array::from_fn(|_| AtomicF32::new(0.0)),
            stick_click: AtomicBool::new(false),
            hands: std::array::from_fn(|_| AtomicF32::new(0.0)),
            shoulders: std::array::from_fn(|_| AtomicBool::new(false)),
            shoulder_press: std::array::from_fn(|_| AtomicBool::new(false)),
            midi_keys: std::array::from_fn(|_| AtomicU8::new(0)),
            pitch_bend: AtomicF32::new(0.0),
            mod_wheel: AtomicF32::new(0.0),
            aftertouch: AtomicF32::new(0.0),
        }
    }

    /// Called by the UI each frame with whether the app on screen plays
    /// the surface. Leaving such an app releases any MIDI keys it held so
    /// nothing sticks.
    pub fn set_play_surface(&self, on: bool) {
        if self.play_surface.swap(on, Ordering::Relaxed) && !on {
            for k in &self.midi_keys {
                k.store(0, Ordering::Relaxed);
            }
            self.pitch_bend.set(0.0);
            self.aftertouch.set(0.0);
        }
    }

    // Only the macOS gamepad backend has a clickable stick / shoulders.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn set_stick_click(&self) {
        self.stick_click.store(true, Ordering::Relaxed);
    }
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn set_shoulder_press(&self, side: usize) {
        self.shoulder_press[side].store(true, Ordering::Relaxed);
    }

    /// The controller's play-surface values merged with another source's
    /// (keyboard, on-screen frame): sticks/hands take whichever is
    /// pushed further, buttons OR together, edges are consumed.
    pub fn play_surface_input(&self, other: crate::app::Input) -> crate::app::Input {
        let r = Ordering::Relaxed;
        let pick = |a: f32, b: f32| if a.abs() >= b.abs() { a } else { b };
        let mut midi = other.midi_keys;
        for (k, v) in midi.0.iter_mut().enumerate() {
            *v = (*v).max(self.midi_keys[k].load(r));
        }
        crate::app::Input {
            stick: [pick(self.stick[0].get(), other.stick[0]), pick(self.stick[1].get(), other.stick[1])],
            stick_click: self.stick_click.swap(false, r) || other.stick_click,
            hands: [self.hands[0].get().max(other.hands[0]), self.hands[1].get().max(other.hands[1])],
            shoulders: [self.shoulders[0].load(r) || other.shoulders[0], self.shoulders[1].load(r) || other.shoulders[1]],
            shoulder_press: [self.shoulder_press[0].swap(false, r) || other.shoulder_press[0], self.shoulder_press[1].swap(false, r) || other.shoulder_press[1]],
            midi_keys: midi,
            pitch_bend: pick(self.pitch_bend.get(), other.pitch_bend),
            mod_wheel: self.mod_wheel.get().max(other.mod_wheel),
            aftertouch: self.aftertouch.get().max(other.aftertouch),
            ..other
        }
    }

    /// One MIDI channel message for a play-surface app: note on/off into
    /// `midi_keys`, pitch bend, mod wheel, channel aftertouch. Returns
    /// true if it was consumed (so the caller skips its pad mapping).
    pub fn play_surface_midi(&self, status: u8, d1: u8, d2: u8) -> bool {
        if !self.play_surface.load(Ordering::Relaxed) {
            return false;
        }
        let r = Ordering::Relaxed;
        match status & 0xF0 {
            0x90 if d2 > 0 => self.midi_keys[(d1 & 0x7F) as usize].store(d2, r),
            0x80 | 0x90 => self.midi_keys[(d1 & 0x7F) as usize].store(0, r),
            0xE0 => {
                let v = ((d2 as i32) << 7 | d1 as i32) - 8192;
                self.pitch_bend.set((v as f32 / 8192.0).clamp(-1.0, 1.0));
            }
            0xB0 if d1 == 1 => self.mod_wheel.set(d2 as f32 / 127.0),
            0xD0 => self.aftertouch.set(d1 as f32 / 127.0),
            _ => return false,
        }
        true
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
