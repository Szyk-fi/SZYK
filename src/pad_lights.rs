//! One rule for what colour each pad is, used by both the screen and a
//! connected controller's LEDs (an Ableton Push 2), so the two always
//! agree.
//!
//! - A pad being held shows red: a note sounding, or a step being
//!   played. It wins over everything else, so you can always see what
//!   your fingers are doing.
//! - Otherwise the active app's own colour for that pad
//!   (`App::grid_led_overlay`): the Sequencer's programmed steps, the
//!   play view's layer colours, C notes in blue on the keyboards...
//! - The top row (F1-F4) is always lit, each button in its own colour.
//!
//! `LightSync` sends only what changed, resends everything whenever the
//! MIDI ports change (a Push plugged in, or power-cycled, starts dark),
//! and refreshes everything every few seconds in case a message was lost.
//! The Push only has palette colours, not RGB, so `rgb` gives the screen
//! the colour each palette slot actually looks like on the hardware.

use crate::controller::{GRID_NOTES, TOP_NOTES};
use crate::led_output::{LedOutput, PadColor};
use std::time::{Duration, Instant};

/// F1 Home, F2 pads/context, F3 start/stop, F4 mixer.
pub const TOP_COLORS: [PadColor; 4] = [PadColor::Yellow, PadColor::Green, PadColor::Blue, PadColor::Red];

/// The held colour, which wins over the app's overlay.
pub const HELD: PadColor = PadColor::Red;

const REFRESH_EVERY: Duration = Duration::from_secs(4);

/// Each pad's colour from the app's overlay and the pads being held.
pub fn compose(overlay: [PadColor; 16], held: [bool; 16]) -> [PadColor; 16] {
    std::array::from_fn(|i| if held[i] { HELD } else { overlay[i] })
}

/// What each palette slot looks like on the Push, for the screen.
#[allow(dead_code)] // not used by the main binary
pub fn rgb(c: PadColor) -> (u8, u8, u8) {
    match c {
        PadColor::Off => (0x23, 0x23, 0x23),
        PadColor::Green => (0x2E, 0xCC, 0x55),
        PadColor::Red => (0xFF, 0x4D, 0x4D),
        PadColor::Yellow => (0xE0, 0xC0, 0x30),
        PadColor::Blue => (0x40, 0x90, 0xE0),
    }
}

/// Keeps a controller's LEDs equal to the pads on screen.
pub struct LightSync {
    leds: LedOutput,
    /// What each pad was last sent as; None = unknown, send it.
    sent: [Option<PadColor>; 16],
    top_sent: bool,
    last_refresh: Instant,
    /// Every LED message sent, for tests.
    #[cfg(test)]
    pub log: Vec<(u8, u8)>,
}

#[allow(dead_code)] // not used by the main binary
impl LightSync {
    pub fn new(leds: LedOutput) -> Self {
        LightSync {
            leds,
            sent: [None; 16],
            top_sent: false,
            last_refresh: Instant::now(),
            #[cfg(test)]
            log: Vec::new(),
        }
    }

    fn note(&mut self, note: u8, color: PadColor) {
        #[cfg(test)]
        self.log.push((note, color.velocity()));
        self.leds.note_on(note, color.velocity());
    }

    /// Call once per frame with the colours the screen is showing.
    pub fn update(&mut self, pads: [PadColor; 16]) {
        self.leds.poll();
        if self.leds.take_reconnected() || self.last_refresh.elapsed() >= REFRESH_EVERY {
            self.sent = [None; 16];
            self.top_sent = false;
            self.last_refresh = Instant::now();
        }
        for i in 0..16 {
            if self.sent[i] != Some(pads[i]) {
                self.note(GRID_NOTES[i], pads[i]);
                self.sent[i] = Some(pads[i]);
            }
        }
        if !self.top_sent {
            for (i, &note) in TOP_NOTES.iter().enumerate() {
                self.note(note, TOP_COLORS[i]);
            }
            self.top_sent = true;
        }
    }

    /// Everything off (on exit), so the Push isn't left showing stale pads.
    pub fn clear(&mut self) {
        for i in 0..16 {
            self.note(GRID_NOTES[i], PadColor::Off);
        }
        for &note in TOP_NOTES.iter() {
            self.note(note, PadColor::Off);
        }
        self.sent = [Some(PadColor::Off); 16];
        self.top_sent = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_wins_over_the_apps_colour() {
        let mut overlay = [PadColor::Off; 16];
        overlay[0] = PadColor::Green;
        overlay[1] = PadColor::Blue;
        let mut held = [false; 16];
        held[0] = true;
        held[2] = true;
        let c = compose(overlay, held);
        assert_eq!(c[0], HELD);
        assert_eq!(c[1], PadColor::Blue);
        assert_eq!(c[2], HELD);
        assert_eq!(c[3], PadColor::Off);
    }

    #[test]
    fn it_sends_the_screens_colours_and_only_changes_after_that() {
        let mut s = LightSync::new(LedOutput::none());
        let mut pads = [PadColor::Off; 16];
        pads[5] = PadColor::Green;
        s.update(pads);
        assert_eq!(s.log.len(), 20, "16 pads and the 4 top buttons");
        assert!(s.log.contains(&(GRID_NOTES[5], PadColor::Green.velocity())));
        s.log.clear();
        s.update(pads);
        assert!(s.log.is_empty(), "nothing changed, nothing sent");
        pads[5] = HELD;
        s.update(pads);
        assert_eq!(s.log, vec![(GRID_NOTES[5], HELD.velocity())]);
    }

    #[test]
    fn it_refreshes_everything_now_and_then() {
        let mut s = LightSync::new(LedOutput::none());
        s.update([PadColor::Off; 16]);
        s.log.clear();
        s.last_refresh = Instant::now() - REFRESH_EVERY;
        s.update([PadColor::Off; 16]);
        assert_eq!(s.log.len(), 20, "a lost message gets corrected");
    }
}
