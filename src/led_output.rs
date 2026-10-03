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
    pending: Option<std::sync::mpsc::Receiver<Vec<MidiOutputConnection>>>,
    queued: Vec<[u8; 3]>,
    /// Watch for ports appearing or disappearing (a Push plugged in after
    /// start-up, or power-cycled). Off for `none()`.
    rescan: bool,
    /// The port names `connections` were made from.
    ports: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    last_scan: std::time::Instant,
    /// A rescan in flight. Kept apart from `pending` so the existing
    /// connections keep working while it runs.
    scan: Option<std::sync::mpsc::Receiver<Vec<MidiOutputConnection>>>,
    /// Set when a new set of connections arrives, until `take_reconnected`.
    reconnected: bool,
}

/// How often to look for MIDI ports coming and going.
const RESCAN_EVERY: std::time::Duration = std::time::Duration::from_secs(3);

impl LedOutput {
    /// Connects to every available MIDI output port at once. Never
    /// fails outright -- with no ports (or no controller with
    /// addressable LEDs plugged in), this is just an empty set and
    /// every `note_on` below becomes a no-op.
    pub fn open_all() -> Self {
        let (send, receive) = std::sync::mpsc::channel();
        let ports = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let known = std::sync::Arc::clone(&ports);
        std::thread::spawn(move || {
            *known.lock().unwrap() = Self::port_names();
            let _ = send.send(Self::connect_all());
        });
        Self { connections: Vec::new(), pending: Some(receive), queued: Vec::new(), rescan: true, ports, last_scan: std::time::Instant::now(), scan: None, reconnected: false }
    }

    fn port_names() -> Vec<String> {
        let Ok(probe) = MidiOutput::new("portamax-sim-led-scan") else { return Vec::new() };
        probe.ports().iter().map(|p| probe.port_name(p).unwrap_or_default()).collect()
    }

    /// True once after the set of connected ports changed, so the caller
    /// can send its whole state again (a freshly plugged-in Push starts
    /// dark).
    pub fn take_reconnected(&mut self) -> bool {
        std::mem::take(&mut self.reconnected)
    }

    /// Every few seconds, off the UI thread: if the MIDI ports changed,
    /// reconnect to all of them.
    fn maybe_rescan(&mut self) {
        if let Some(scan) = self.scan.as_ref() {
            match scan.try_recv() {
                Ok(connections) => {
                    self.connections = connections;
                    self.scan = None;
                    self.reconnected = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.scan = None,
            }
            return;
        }
        if !self.rescan || self.pending.is_some() || self.last_scan.elapsed() < RESCAN_EVERY {
            return;
        }
        self.last_scan = std::time::Instant::now();
        let (send, receive) = std::sync::mpsc::channel();
        let known = std::sync::Arc::clone(&self.ports);
        std::thread::spawn(move || {
            let now = Self::port_names();
            let changed = *known.lock().unwrap() != now;
            if changed {
                *known.lock().unwrap() = now;
                let _ = send.send(Self::connect_all());
            }
            // unchanged: dropping `send` ends the scan with no new set
        });
        self.scan = Some(receive);
    }

    // CoreMIDI discovery can block when the server is unavailable. Keep it
    // off the UI thread; retain the latest control values until it completes.
    fn connect_all() -> Vec<MidiOutputConnection> {
        let mut connections = Vec::new();
        let probe = match MidiOutput::new("portamax-sim-led-probe") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("MIDI output unavailable ({e}) -- controller LEDs won't light, everything else is unaffected.");
                return connections;
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
        connections
    }

    /// An `Os` with no real MIDI hardware (headless PNG/WAV diagnostic
    /// renders, tests) still needs an `Os::new` to hand *something* to
    /// -- an empty output set that quietly no-ops every send.
    pub fn none() -> Self {
        Self { connections: Vec::new(), pending: None, queued: Vec::new(), rescan: false, ports: Default::default(), last_scan: std::time::Instant::now(), scan: None, reconnected: false }
    }

    pub fn poll(&mut self) {
        self.maybe_rescan();
        if let Some(pending) = self.pending.as_ref() {
            match pending.try_recv() {
                Ok(connections) => {
                    self.connections = connections;
                    self.pending = None;
                    self.reconnected = true;
                    for message in self.queued.drain(..) {
                        for conn in &mut self.connections { conn.send(&message).ok(); }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => (),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.queued.clear();
                }
            }
        }
    }

    fn send(&mut self, message: [u8; 3]) {
        self.poll();
        if self.pending.is_some() {
            if let Some(previous) = self.queued.iter_mut().find(|m| m[..2] == message[..2]) {
                *previous = message;
            } else { self.queued.push(message); }
            return;
        }
        for conn in &mut self.connections { conn.send(&message).ok(); }
    }

    pub fn note_on(&mut self, note: u8, velocity: u8) {
        self.send([0x90, note, velocity]);
    }

    /// A Control Change message -- what a MIDI-to-CV interface (see
    /// `apps/cv_out.rs`) actually reads, as opposed to the Note On
    /// this same connection set also carries for controller LEDs.
    /// Connecting to every output port (see `open_all`) means this
    /// goes out to the Push 2 too, same harmless no-op as the reverse.
    /// `channel` is 0-15 (MIDI channel 1-16), `cc`/`value` are 0-127.
    pub fn control_change(&mut self, channel: u8, cc: u8, value: u8) {
        self.send([0xB0 | (channel & 0x0F), cc & 0x7F, value & 0x7F]);
    }
}

#[cfg(test)]
mod connection_tests {
    use super::*;
    #[test]
    fn pending_discovery_keeps_latest_values_without_blocking() {
        let (send, receive) = std::sync::mpsc::channel();
        let mut output = LedOutput { pending: Some(receive), ..LedOutput::none() };
        output.note_on(60, 127);
        output.note_on(60, 0);
        output.control_change(0, 1, 10);
        output.control_change(0, 1, 20);
        assert_eq!(output.queued, vec![[0x90, 60, 0], [0xB0, 1, 20]]);
        send.send(Vec::new()).unwrap();
        output.poll();
        assert!(output.pending.is_none());
        assert!(output.queued.is_empty());
        assert!(output.take_reconnected(), "a new port set is reported once");
        assert!(!output.take_reconnected());
    }
}
