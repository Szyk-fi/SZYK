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
        std::thread::sleep(POLL);
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
