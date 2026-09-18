//! MIDI output back to whatever fed `controller.rs` -- drives an
//! Ableton Push 2's (or any other class-compliant MIDI controller's)
//! pad/button LEDs to mirror on-screen state, the same "connect to
//! everything, it's a harmless no-op for anything not listening"
//! philosophy `run_midi_listener` (main.rs) already uses for input.
//!
//! Runs on the main thread -- unlike MIDI input (which needs its own
//! thread to receive callbacks), sending output is just a blocking
//! write, and `Os::run`'s loop already has a natural once-per-frame
//! place to do it.

use midir::{MidiOutput, MidiOutputConnection};

/// A pad's color, addressed the way this controller's default palette
/// actually indexes it (a Note On velocity) rather than an RGB value --
/// matches the real hardware, which (without further Sysex palette
/// setup, not implemented here) only understands a fixed set of
/// palette slots, not arbitrary colors.
///
/// `RED_VELOCITY` (127), `GREEN_VELOCITY` (13), and `BLUE_VELOCITY`
/// (47) are all confirmed against real hardware -- watched and
/// reported back (13 was the second guess for green; the first, 122,
/// turned out to render white instead). `YELLOW_VELOCITY` is still an
/// unconfirmed guess -- same "watch what you actually see and adjust"
/// situation `controller::GRID_NOTES`'s doc comment already asks for.
/// If a pad meant to be one color shows up as another (or doesn't
/// light at all), that velocity is the constant to fix.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PadColor {
    #[default]
    Off,
    Green,
    Red,
    Yellow,
    Blue,
}

impl PadColor {
    const OFF_VELOCITY: u8 = 0;
    const GREEN_VELOCITY: u8 = 13;
    const RED_VELOCITY: u8 = 127;
    const YELLOW_VELOCITY: u8 = 9;
    const BLUE_VELOCITY: u8 = 47;

    pub fn velocity(self) -> u8 {
        match self {
            PadColor::Off => Self::OFF_VELOCITY,
            PadColor::Green => Self::GREEN_VELOCITY,
            PadColor::Red => Self::RED_VELOCITY,
            PadColor::Yellow => Self::YELLOW_VELOCITY,
            PadColor::Blue => Self::BLUE_VELOCITY,
        }
    }
}

pub struct LedOutput {
    connections: Vec<MidiOutputConnection>,
}

impl LedOutput {
    /// Connects to every available MIDI output port at once. Never
    /// fails outright -- with no ports (or no controller with
    /// addressable LEDs plugged in), this is just an empty set and
    /// every `note_on` below becomes a no-op.
    pub fn open_all() -> Self {
        let mut connections = Vec::new();
        let probe = match MidiOutput::new("portamax-sim-led-probe") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("MIDI output unavailable ({e}) -- controller LEDs won't light, everything else is unaffected.");
                return Self { connections };
            }
        };
        let port_count = probe.ports().len();
        for i in 0..port_count {
            let midi_out = match MidiOutput::new("portamax-sim-led") {
                Ok(m) => m,
                Err(_) => continue,
            };
            let Some(port) = midi_out.ports().into_iter().nth(i) else {
                continue; // port list changed since the probe; skip it
            };
            let name = midi_out.port_name(&port).unwrap_or_default();
            match midi_out.connect(&port, "portamax-sim-led-out") {
                Ok(conn) => {
                    println!("Connected to MIDI output (LEDs): {name}");
                    connections.push(conn);
                }
                Err(e) => eprintln!("Couldn't connect to MIDI output '{name}': {e}"),
            }
        }
        Self { connections }
    }

    /// An `Os` with no real MIDI hardware (headless PNG/WAV diagnostic
    /// renders, tests) still needs an `Os::new` to hand *something* to
    /// -- an empty output set that quietly no-ops every send.
    pub fn none() -> Self {
        Self { connections: Vec::new() }
    }

    pub fn note_on(&mut self, note: u8, velocity: u8) {
        for conn in &mut self.connections {
            conn.send(&[0x90, note, velocity]).ok();
        }
    }

    /// A Control Change message -- what a MIDI-to-CV interface (see
    /// `apps/cv_out.rs`) actually reads, as opposed to the Note On
    /// this same connection set also carries for controller LEDs.
    /// Connecting to every output port (see `open_all`) means this
    /// goes out to the Push 2 too, same harmless no-op as the reverse.
    /// `channel` is 0-15 (MIDI channel 1-16), `cc`/`value` are 0-127.
    pub fn control_change(&mut self, channel: u8, cc: u8, value: u8) {
        for conn in &mut self.connections {
            conn.send(&[0xB0 | (channel & 0x0F), cc & 0x7F, value & 0x7F]).ok();
        }
    }
}
