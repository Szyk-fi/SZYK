//! 32 independent CV outputs, sent out over MIDI Control Change to an
//! external MIDI-to-CV interface (an Expert Sleepers-style box, a
//! Kenton, anything that maps incoming CC values to analog voltage
//! out) -- the same "MIDI out to real hardware" idea `led_output.rs`
//! already uses for a Push 2's pad LEDs, applied to control voltage
//! instead of light.
//!
//! Each channel is a plain 0-127 (7-bit, the CC spec's native
//! resolution) value on one shared, user-picked MIDI channel, at a
//! fixed CC number per channel (`CC_BASE + channel index`, so CV 1 is
//! always CC 0 regardless of which channel number you've picked --
//! only the destination MIDI channel moves). What that raw 0-127
//! becomes in actual volts is entirely the connected interface's own
//! job (its mapping software sets the scale/offset per output) --
//! this app has no idea what's patched to which output or what range
//! that patch expects, the same way a mixer fader has no idea what
//! speaker is on the other end of the cable.
//!
//! Every channel is also a ModBus modulation target (see modbus.rs),
//! the same as any other continuous knob in this build -- Pam's can
//! drive a CV output as an LFO/envelope generator, stacking on top of
//! this app's own manual per-channel offset exactly the way Plaits'
//! Harmonics/Timbre/Morph/Decay already do.
//!
//! Sending happens from `background_tick` (see app.rs), not `tick`,
//! and not the audio processor: it has to keep running no matter which
//! app is on screen (an unattended CV patch shouldn't freeze the
//! moment you look at another app), but MIDI send is blocking OS I/O
//! that must never run on the real-time audio callback thread -- see
//! `background_tick`'s own doc comment for why. This app has no
//! `audio_processor` at all; it makes no sound.

use crate::app::{App, Input};
use crate::display::FrameBuffer;
use crate::led_output::LedOutput;
use crate::modbus::ModBus;
use crate::paramlist::{ParamList, ACCENT};
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub const NUM_CHANNELS: usize = 32;
/// CC 0 through 31 -- deliberately its own untouched block, clear of
/// every other CC this build already sends/reads for something else
/// (mod wheel on 1, the Push 2 encoders around 77-109, Prism's macros
/// on 20-25) even though none of that would actually collide here --
/// this goes out its own dedicated connection to a dedicated box, not
/// back into this build's own input parsing. Freely reassignable if
/// the connected interface wants different numbers; it's one constant.
const CC_BASE: u8 = 0;
const MIN_LEVEL: f32 = 0.0;
const MAX_LEVEL: f32 = 1.0;
/// Unlike a mixer fader (unity/100% = "unchanged"), a CV output has no
/// natural non-zero default -- a fresh patch should read 0V, not some
/// arbitrary offset, so Reset (knob2 press) goes to 0 here, not 1.
const DEFAULT_LEVEL: f32 = 0.0;
const NUM_GROUPS: usize = 2;

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.01).clamp(MIN_LEVEL, MAX_LEVEL);
    value.set(next);
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    MidiChannel,
    Channel(usize),
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

pub struct CvOutApp {
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    /// Manual per-channel offset, 0.0-1.0 -- what knob2 edits.
    channels: [AtomicF32; NUM_CHANNELS],
    /// External modulation input per channel (see modbus.rs) -- stacks
    /// on top of `channels[i]` exactly the way Plaits' Harmonics/
    /// Timbre/Morph/Decay combine their own base value with theirs.
    ext_mod: [Arc<AtomicF32>; NUM_CHANNELS],
    /// 0-15 (displayed as 1-16) -- which MIDI channel all 32 CCs go
    /// out on. A discrete stepper, so edited one detent at a time (see
    /// Plaits' Engine selector) rather than `bump()`'s scaled sweep.
    midi_channel: AtomicU32,
    /// This app's own MIDI output connection, independent of `Os`'s
    /// (which drives the Push 2's LEDs) -- connects to every available
    /// port the same "harmless no-op for anything not listening" way,
    /// so it doesn't need Os to hand it a reference to share one.
    leds: LedOutput,
    /// What was last actually sent per channel, so a value that isn't
    /// changing doesn't get re-sent 60 times a second -- same
    /// "diff before you send" discipline `Os::update_leds` uses for
    /// the grid LEDs.
    last_sent: [Option<u8>; NUM_CHANNELS],
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

impl CvOutApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>) -> Self {
        Self {
            sensitivity,
            nav_speed,
            channels: std::array::from_fn(|_| AtomicF32::new(DEFAULT_LEVEL)),
            ext_mod: std::array::from_fn(|i| modbus.register(format!("CV Out: {}", i + 1))),
            midi_channel: AtomicU32::new(0),
            leds: LedOutput::open_all(),
            last_sent: [None; NUM_CHANNELS],
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
        }
    }

    /// This channel's actual output value right now: manual offset
    /// plus whatever's routed to modulate it externally, clamped to
    /// what a 0-127 CC can represent -- the single source of truth
    /// both `draw` and `background_tick` read from.
    fn combined(&self, i: usize) -> f32 {
        (self.channels[i].get() + self.ext_mod[i].get()).clamp(MIN_LEVEL, MAX_LEVEL)
    }

    fn midi_value(&self, i: usize) -> u8 {
        (self.combined(i) * 127.0).round() as u8
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Group(0)];
        if self.expanded[0] {
            rows.push(Row::Leaf(Selection::MidiChannel));
        }
        rows.push(Row::Group(1));
        if self.expanded[1] {
            rows.extend((0..NUM_CHANNELS).map(|i| Row::Leaf(Selection::Channel(i))));
        }
        rows
    }

    fn group_name(&self, g: usize) -> &'static str {
        if g == 0 { "Global" } else { "CV Outputs" }
    }

    fn group_summary(&self, g: usize) -> String {
        if g == 0 {
            format!("Ch {}", self.midi_channel.load(Ordering::Relaxed) + 1)
        } else {
            format!("{NUM_CHANNELS} channels")
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::MidiChannel => "MIDI Channel".into(),
            Selection::Channel(i) => format!("CV {}", i + 1),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::MidiChannel => format!("{}", self.midi_channel.load(Ordering::Relaxed) + 1),
            Selection::Channel(i) => format!("{:.0}% (CC{}={})", self.combined(i) * 100.0, CC_BASE + i as u8, self.midi_value(i)),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        match sel {
            Selection::MidiChannel => {
                let cur = self.midi_channel.load(Ordering::Relaxed) as i32;
                let next = (cur + delta.signum()).rem_euclid(16);
                self.midi_channel.store(next as u32, Ordering::Relaxed);
            }
            Selection::Channel(i) => bump(&self.channels[i], delta, self.sensitivity.get()),
        }
    }

    fn reset(&mut self, sel: Selection) {
        if let Selection::Channel(i) = sel {
            self.channels[i].set(DEFAULT_LEVEL);
        }
    }

    /// Which channels actually need sending right now -- only ones
    /// whose combined value has changed since the last call -- and
    /// updates `last_sent` to match. Split out from `background_tick`
    /// purely so this (the part with real logic in it) is unit-
    /// testable without a live `LedOutput` connection.
    fn pending_sends(&mut self) -> Vec<(u8, u8, u8)> {
        let channel = self.midi_channel.load(Ordering::Relaxed) as u8;
        let mut out = Vec::new();
        for i in 0..NUM_CHANNELS {
            let value = self.midi_value(i);
            if self.last_sent[i] != Some(value) {
                out.push((channel, CC_BASE + i as u8, value));
                self.last_sent[i] = Some(value);
            }
        }
        out
    }
}

impl App for CvOutApp {
    fn tick(&mut self, input: &Input) {
        let rows = self.visible_rows();
        self.list.navigate(input.knob1, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();

        if input.knob1_press {
            if let Some(Row::Group(g)) = current {
                self.expanded[g] = !self.expanded[g];
            }
        }
        if let Some(Row::Leaf(sel)) = current {
            self.edit(sel, input.knob2);
            if input.knob2_press {
                self.reset(sel);
            }
        }
    }

    /// See the module doc comment and `App::background_tick` for why
    /// the actual MIDI sending lives here and not in `tick`/an audio
    /// processor. Just forwards `pending_sends`, kept separate so the
    /// change-detection logic is testable without a live MIDI
    /// connection.
    fn background_tick(&mut self) {
        for (channel, cc, value) in self.pending_sends() {
            self.leds.control_change(channel, cc, value);
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let title = MonoTextStyle::new(&SPLEEN_16X32, Rgb565::WHITE);
        Text::new("CV Out", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));

        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", self.leaf_name(*sel)), self.leaf_value(*sel)),
            })
            .collect();
        self.list.draw(fb, 16, 44, 24, 10, &display_rows);

        // --- Right: a bank of vertical bars, one per CV channel,
        // scrolling to keep the selected channel in view -- same
        // technique Mixer's fader bank uses (see mixer.rs), shown as
        // combined (manual + external mod) output since that's the
        // actual voltage about to go out the wire. ---
        let track_top = 60;
        let track_h = 235;
        let track_w = 26;
        let gap = 16;
        let stride = track_w + gap;
        let base_x = 16;
        let right_margin = 12;

        let mut draw_bar = |x: i32, level: f32, label: &str, lit: bool| {
            let color = if lit { Rgb565::new(0, 63, 10) } else { Rgb565::new(0, 40, 6) };
            Rectangle::new(Point::new(x, track_top), Size::new(track_w as u32, track_h as u32))
                .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(10, 20, 10), 1))
                .draw(fb)
                .ok();
            let fill_h = ((level / MAX_LEVEL).clamp(0.0, 1.0) * track_h as f32) as i32;
            Rectangle::new(Point::new(x + 1, track_top + track_h - fill_h), Size::new((track_w - 2) as u32, fill_h.max(0) as u32))
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(fb)
                .ok();
            let short: String = label.chars().take(4).collect();
            Text::new(&short, Point::new(x - 2, track_top + track_h + 12), dim).draw(fb).ok();
        };

        let selected_channel = match rows.get(self.list.selected) {
            Some(Row::Leaf(Selection::Channel(i))) => Some(*i),
            _ => None,
        };
        let available_w = (crate::display::WIDTH as i32 - base_x - right_margin).max(0);
        let visible_bars = (available_w / stride).max(1) as usize;
        let scroll = if NUM_CHANNELS <= visible_bars {
            0
        } else {
            let sel = selected_channel.unwrap_or(0);
            sel.saturating_sub(visible_bars.saturating_sub(1)).min(NUM_CHANNELS - visible_bars)
        };
        let end = (scroll + visible_bars).min(NUM_CHANNELS);

        for (slot, i) in (scroll..end).enumerate() {
            let x = base_x + slot as i32 * stride;
            draw_bar(x, self.combined(i), &format!("CV{}", i + 1), selected_channel == Some(i));
        }
        if scroll > 0 {
            Text::new("<", Point::new(base_x - stride + 4, track_top + track_h / 2), accent).draw(fb).ok();
        }
        if end < NUM_CHANNELS {
            Text::new(">", Point::new(base_x + visible_bars as i32 * stride + 2, track_top + track_h / 2), accent).draw(fb).ok();
        }

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::FrameBuffer;
    use crate::modbus::ModBus;

    fn new_app() -> CvOutApp {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(1.0));
        let modbus = Arc::new(ModBus::new());
        CvOutApp::new(sensitivity, nav_speed, modbus)
    }

    /// Every one of the 32 channels must actually register as its own
    /// distinct ModBus target -- the whole point of exposing CV Out
    /// to the rest of the modulation system (see the module doc
    /// comment), not just 32 unreachable local knobs.
    #[test]
    fn every_channel_registers_as_its_own_modulation_target() {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(1.0));
        let modbus = Arc::new(ModBus::new());
        let _app = CvOutApp::new(sensitivity, nav_speed, Arc::clone(&modbus));
        assert_eq!(modbus.len(), NUM_CHANNELS);
        assert_eq!(modbus.names()[0], "CV Out: 1");
        assert_eq!(modbus.names()[31], "CV Out: 32");
    }

    /// A channel's actual output must be the manual value plus
    /// whatever's routed to modulate it externally (same "stacks on
    /// top" combination every other ModBus-driven parameter uses),
    /// clamped to a valid 0-127 CC range either direction.
    #[test]
    fn external_modulation_stacks_on_top_of_the_manual_value_and_clamps() {
        let mut app = new_app();
        app.channels[0].set(0.5);
        assert_eq!(app.midi_value(0), 64, "0.5 * 127 should round to 64 with no modulation");

        app.ext_mod[0].set(1.0); // way past what channel 0's manual value + this could actually reach
        assert_eq!(app.midi_value(0), 127, "combined value must clamp at the top of the CC range");

        app.ext_mod[0].set(-1.0);
        assert_eq!(app.midi_value(0), 0, "combined value must clamp at the bottom of the CC range");
    }

    /// `pending_sends` (what `background_tick` forwards to the real
    /// MIDI connection -- see its own doc comment for why it's split
    /// out) must report every channel the first time (nothing has
    /// been sent yet), then only the one that actually changed, and
    /// nothing at all once every channel is stable -- an idle patch
    /// shouldn't flood the wire at 60 messages/sec/channel forever.
    #[test]
    fn pending_sends_reports_new_and_changed_channels_only() {
        let mut app = new_app();
        app.channels[0].set(0.5);

        let first = app.pending_sends();
        assert_eq!(first.len(), NUM_CHANNELS, "every channel is new, so all 32 must be reported the first time");
        assert!(first.contains(&(0, CC_BASE, 64)), "channel 0 must report its actual combined value on MIDI channel 1 (index 0)");

        assert_eq!(app.pending_sends(), Vec::new(), "nothing changed since the last call -- nothing should be pending");

        app.channels[5].set(0.25);
        let after_one_change = app.pending_sends();
        assert_eq!(after_one_change, vec![(0, CC_BASE + 5, 32)], "only the one channel that actually changed should be pending");
    }

    /// Reset (knob2 press) must zero a channel's manual offset, not
    /// snap it to some non-zero "unity" the way a mixer fader would --
    /// see `DEFAULT_LEVEL`'s doc comment for why 0 is the only sane
    /// default for a CV output.
    #[test]
    fn reset_zeros_the_manual_offset_not_a_nonzero_default() {
        let mut app = new_app();
        app.channels[3].set(0.9);
        app.reset(Selection::Channel(3));
        assert_eq!(app.channels[3].get(), 0.0);
    }

    /// Drawing with every channel visible (worst case: the full 32-bar
    /// bank, scrolled to the last one) must not panic, and must
    /// actually paint the list, bars, and labels.
    #[test]
    fn draws_all_channels_without_panicking_and_scrolls_to_the_last_one() {
        let mut app = new_app();
        app.expanded = [true, true];
        let rows = app.visible_rows();
        let last = rows.iter().position(|r| matches!(r, Row::Leaf(Selection::Channel(31)))).expect("channel 32 should be a row");
        app.list.selected = last;

        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
        let lit = fb.buffer().iter().filter(|&&p| p != 0).count();
        assert!(lit > 500, "expected the list + bar bank to draw something substantial, got {lit} lit pixels");
    }
}
