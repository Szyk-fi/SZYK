//! Shared real-MIDI-input wiring for the live Slint prototypes --
//! connects to every available MIDI input port, same as main.rs's own
//! `run_midi_listener`/`handle_midi_message`, trimmed to just the
//! knob CCs and grid/keyboard notes (a live prototype doesn't need
//! Prism's macro CCs or the mod-wheel filter-cutoff mapping, which are
//! specific to the real firmware's always-on registry of every app at
//! once). Feeds the same `ControllerState` the real firmware uses, so
//! a real controller -- an Ableton Push 2, a MIDI keyboard, any
//! class-compliant device -- drives a live prototype exactly the way
//! it would drive the real device, alongside (not instead of) the
//! on-screen mouse-driven knobs/pads.
//!
//! Included via `#[path = "slint_common/live_midi.rs"] mod
//! live_midi;` at each live example's own crate root (flat, same
//! reasoning as every other shared module -- see slint_mixer_live.rs's
//! doc comment), alongside its own `#[path = "../src/controller.rs"]
//! mod controller;`. Lives under `slint_common/` (not directly in
//! `examples/`) so cargo's example auto-discovery doesn't also try to
//! build it as its own standalone example binary.
//!
//! Usage:
//! ```ignore
//! let controller = Arc::new(controller::ControllerState::new());
//! let _midi_connections = live_midi::connect_all(Arc::clone(&controller));
//! // ... each timer tick:
//! let input = Input {
//!     grid: std::array::from_fn(|i| controller.grid[i].load(Ordering::Relaxed) || mouse_grid[i]),
//!     knob1: mouse_knob1_delta + controller.take_knob1_delta(),
//!     knob2: mouse_knob2_delta + controller.take_knob2_delta(),
//!     knob1_press: mouse_knob1_press || controller.take_knob1_press(),
//!     knob2_press: mouse_knob2_press || controller.take_knob2_press(),
//!     ..Default::default()
//! };
//! ```

use crate::controller::{self, ControllerState};
use midir::{Ignore, MidiInput, MidiInputConnection};
use std::sync::atomic::Ordering;
use std::sync::Arc;

const KNOB1_CC: u8 = 77;
const KNOB2_CC: u8 = 78;
const KNOB1_BUTTON_CC: u8 = 108;
const KNOB2_BUTTON_CC: u8 = 109;

/// Push 2's touch-sensitive encoders send Note On on very low note
/// numbers the instant you touch one -- well below any real keyboard's
/// lowest key -- so resting a finger on a knob would otherwise trigger
/// the fallback keyboard mapping below. MIDI 21 (A0) is the bottom of
/// a full 88-key piano.
const MIN_FALLBACK_NOTE: u8 = 21;

/// Push 2 encoders in User Mode send relative "2's complement" ticks:
/// 1..63 = clockwise by that many, 65..127 = counter-clockwise.
fn decode_relative(value: u8) -> i32 {
    if value < 64 {
        value as i32
    } else {
        value as i32 - 128
    }
}

/// Any note that isn't one of the 16 pads/4 top buttons `GRID_NOTES`/
/// `TOP_NOTES` were hand-tuned for still plays *something* instead of
/// being silently dropped -- see main.rs's own copy of this function
/// for the fuller explanation.
fn fallback_grid_pad(note: u8) -> Option<usize> {
    if note < MIN_FALLBACK_NOTE {
        return None;
    }
    Some(note as usize % 16)
}

fn handle_midi_message(message: &[u8], controller: &Arc<ControllerState>, midi_map: &Arc<crate::midi_map::MidiMap>, modbus: &Arc<crate::modbus::ModBus>) {
    if message.len() < 2 {
        return;
    }
    let status = message[0] & 0xF0;
    let data1 = message[1];
    let data2 = message.get(2).copied().unwrap_or(0);

    match status {
        0x80 | 0x90 => {
            let is_on = status == 0x90 && data2 > 0;
            if let Some(i) = controller::GRID_NOTES.iter().position(|&n| n == data1) {
                controller.grid[i].store(is_on, Ordering::Relaxed);
            } else if let Some(i) = controller::TOP_NOTES.iter().position(|&n| n == data1) {
                if is_on {
                    controller.set_top(i);
                }
            } else if let Some(i) = fallback_grid_pad(data1) {
                controller.grid[i].store(is_on, Ordering::Relaxed);
            } else {
                eprintln!("midi: unmapped note {data1} (on={is_on})");
            }
        }
        0xB0 => {
            if data1 == KNOB1_CC {
                controller.add_knob1_delta(decode_relative(data2));
            } else if data1 == KNOB2_CC {
                controller.add_knob2_delta(decode_relative(data2));
            } else if data1 == KNOB1_BUTTON_CC && data2 >= 64 {
                controller.set_knob1_press();
            } else if data1 == KNOB2_BUTTON_CC && data2 >= 64 {
                controller.set_knob2_press();
            } else {
                midi_map.observe_cc(data1, data2, modbus);
            }
        }
        _ => {}
    }
}

/// Connects to *every* available MIDI input port at once (an audio
/// interface's MIDI thru, a keyboard, a Push 2 can all be plugged in
/// simultaneously) -- same as main.rs's `run_midi_listener`. Returns
/// the live connections; drop them (or let the returned `Vec` go out
/// of scope) to disconnect. Prints what it found/connected to, same
/// as the real firmware, so a prototype's terminal output tells you
/// whether MIDI actually came through.
pub fn connect_all(controller: Arc<ControllerState>, midi_map: Arc<crate::midi_map::MidiMap>, modbus: Arc<crate::modbus::ModBus>) -> Vec<MidiInputConnection<()>> {
    let mut connections = Vec::new();

    let probe = match MidiInput::new("portamax-sim-live-probe") {
        Ok(p) => p,
        Err(e) => {
            eprintln!("midi: couldn't probe input ports: {e}");
            return connections;
        }
    };
    let port_count = probe.ports().len();
    if port_count == 0 {
        println!("No MIDI input ports found -- on-screen mouse control only.");
        return connections;
    }

    println!("MIDI ports found:");
    for (i, port) in probe.ports().iter().enumerate() {
        println!("  [{i}] {}", probe.port_name(port).unwrap_or_default());
    }

    for i in 0..port_count {
        let mut midi_in = match MidiInput::new("portamax-sim-live") {
            Ok(m) => m,
            Err(e) => {
                eprintln!("midi: couldn't open input: {e}");
                continue;
            }
        };
        midi_in.ignore(Ignore::None);
        let Some(port) = midi_in.ports().into_iter().nth(i) else { continue };
        let Ok(name) = midi_in.port_name(&port) else { continue };

        let controller = Arc::clone(&controller);
        let midi_map = Arc::clone(&midi_map);
        let modbus = Arc::clone(&modbus);
        let conn = midi_in.connect(
            &port,
            "portamax-sim-live-in",
            move |_stamp, message, _| handle_midi_message(message, &controller, &midi_map, &modbus),
            (),
        );
        match conn {
            Ok(conn) => {
                println!("Connected to MIDI input: {name}");
                connections.push(conn);
            }
            Err(e) => eprintln!("midi: failed to connect to {name}: {e}"),
        }
    }

    connections
}
