//! MIDI devices on the note bus. Every MIDI output port that appears (a
//! hardware synth over USB, a DAW's virtual port) becomes an instrument,
//! "MIDI Out: <port>", that any note source can play -- Bloom, a Sequencer
//! track, a norns script -- the same way it plays an app. Ports are
//! rescanned every couple of seconds, so plugging a synth in mid-session
//! makes it show up in every Plays row and in Portal's Notes page; unplug
//! it and its notes are released.
//!
//! On the device this is the USB-MIDI host and the DIN out; in the sim,
//! midir. The control surface's own ports (Push 2's LEDs, see
//! led_output.rs) are skipped.
//!
//! The same thread sends MIDI clock (24 pulses a beat, Start, Stop,
//! Continue) to every port when the shared transport's "send clock" is
//! on (see clock.rs). Pulses are timed from the transport's wall-clock
//! position, so they're as even as this 2 ms poll allows -- about the
//! jitter of a typical USB-MIDI interface -- and while stopped they keep
//! running at the tempo, as hardware masters do, so a follower can
//! show the tempo before you press play.

use crate::note_bus::{NoteBus, NoteInboxRef, NoteView};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RESCAN: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(2);

struct Port {
    name: String,
    inbox: Arc<NoteInboxRef>,
    view: NoteView,
    sent: [u8; 128],
    conn: Option<midir::MidiOutputConnection>,
}

/// Ports that aren't instruments: our own control surface, and the
/// system's through ports.
fn skip(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("ableton push") || n.contains("portamax") || n.contains("midi through")
}

/// Starts the background thread. Harmless with no MIDI at all.
pub fn spawn(bus: Arc<NoteBus>) {
    let _ = std::thread::Builder::new().name("midi-out".into()).spawn(move || run(bus));
}

fn run(bus: Arc<NoteBus>) {
    let mut ports: Vec<Port> = Vec::new();
    // first scan shortly after startup, once the apps' instruments are
    // declared, so they list first
    let mut last_scan = Instant::now() - RESCAN + Duration::from_millis(500);
    let mut out_clock = ClockOut::default();
    loop {
        if last_scan.elapsed() >= RESCAN {
            last_scan = Instant::now();
            rescan(&bus, &mut ports);
        }
        for p in ports.iter_mut() {
            p.inbox.poll(&mut p.view);
            for n in 0..128 {
                let want = p.view.keys[n];
                if want != p.sent[n] {
                    if let Some(c) = p.conn.as_mut() {
                        let msg = if want > 0 { [0x90, n as u8, want] } else { [0x80, n as u8, 0] };
                        let _ = c.send(&msg);
                    }
                    p.sent[n] = want;
                }
            }
        }
        let msgs = out_clock.poll(&crate::clock::Clock::shared());
        if !msgs.is_empty() {
            for p in ports.iter_mut() {
                if let Some(c) = p.conn.as_mut() {
                    for m in &msgs {
                        let _ = c.send(&[*m]);
                    }
                }
            }
        }
        std::thread::sleep(POLL);
    }
}

/// Turns the shared transport into MIDI realtime bytes.
#[derive(Default)]
struct ClockOut {
    was_running: bool,
    was_enabled: bool,
    /// Pulses sent since the transport started.
    pulses: u64,
    /// While stopped: fractional pulses owed, from wall time.
    free: f64,
    last: Option<Instant>,
}

impl ClockOut {
    fn poll(&mut self, clock: &crate::clock::Clock) -> Vec<u8> {
        let mut out = Vec::new();
        let now = Instant::now();
        let dt = self.last.map_or(0.0, |l| now.duration_since(l).as_secs_f64());
        self.last = Some(now);
        let enabled = clock.send_midi.load(std::sync::atomic::Ordering::Relaxed);
        if !enabled {
            if self.was_enabled && self.was_running {
                out.push(0xFC);
            }
            self.was_enabled = false;
            self.was_running = false;
            return out;
        }
        self.was_enabled = true;
        let s = clock.snap();
        if s.running != self.was_running {
            if s.running {
                // From the top is Start; from anywhere else, Continue
                // (the follower resumes where it stopped, as we do).
                let beat = clock.beat_now();
                if beat < 1.0 / crate::clock::PPQN {
                    out.push(0xFA);
                    self.pulses = 0;
                } else {
                    self.pulses = (beat * crate::clock::PPQN) as u64;
                    out.push(0xFB);
                }
            } else {
                out.push(0xFC);
            }
            self.was_running = s.running;
        }
        if s.running {
            let due = (clock.beat_now() * crate::clock::PPQN).floor().max(0.0) as u64;
            // after a jump (a restart) never fire a burst of catch-up pulses
            if due > self.pulses + 8 || due < self.pulses {
                self.pulses = due;
            }
            while self.pulses < due {
                out.push(0xF8);
                self.pulses += 1;
            }
        } else {
            self.free += dt * clock.bpm() as f64 / 60.0 * crate::clock::PPQN;
            let n = self.free.floor().min(8.0);
            for _ in 0..n as usize {
                out.push(0xF8);
            }
            self.free -= self.free.floor();
        }
        out
    }
}

fn rescan(bus: &NoteBus, ports: &mut Vec<Port>) {
    let Ok(probe) = midir::MidiOutput::new("portamax-midi-out-probe") else { return };
    let names: Vec<String> = probe.ports().iter().filter_map(|p| probe.port_name(p).ok()).filter(|n| !skip(n)).collect();
    // gone: release what it held and drop the connection
    ports.retain_mut(|p| {
        let present = names.contains(&p.name);
        if !present {
            if let Some(c) = p.conn.as_mut() {
                for n in 0..128 {
                    if p.sent[n] > 0 {
                        let _ = c.send(&[0x80, n as u8, 0]);
                    }
                }
            }
            println!("MIDI out: {} disconnected", p.name);
        }
        present
    });
    // new: an instrument slot (stable per name across replugs) and a connection
    for name in names {
        if ports.iter().any(|p| p.name == name) {
            continue;
        }
        let Some(inbox) = bus.register_instrument("midi", &format!("MIDI Out: {name}")) else { continue };
        let conn = midir::MidiOutput::new("portamax").ok().and_then(|out| {
            let port = out.ports().into_iter().find(|p| out.port_name(p).ok().as_deref() == Some(name.as_str()))?;
            out.connect(&port, "portamax-notes").ok()
        });
        println!("MIDI out: {name} {}", if conn.is_some() { "ready as an instrument" } else { "found, but couldn't connect" });
        ports.push(Port { name, inbox, view: NoteView::default(), sent: [0; 128], conn });
    }
}
