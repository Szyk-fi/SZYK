//! Drives the *real* running portamax-sim app over a virtual MIDI port,
//! so real-usage patterns (including the audio-callback-thread timing
//! that plaits_smoke.rs can't reach) can be reproduced without a human
//! at the keyboard. Run `cargo run` in one terminal, then
//! `cargo run --bin midi_poke` in another.

use midir::os::unix::VirtualOutput;
use std::thread;
use std::time::Duration;

const GRID_NOTES: [u8; 16] = [
    60, 61, 62, 63, 52, 53, 54, 55, 44, 45, 46, 47, 36, 37, 38, 39,
];
const TOP_NOTES: [u8; 4] = [68, 69, 70, 71];
const KNOB1_CC: u8 = 77;
const KNOB2_CC: u8 = 78;

fn main() {
    let out = midir::MidiOutput::new("portamax-poke").expect("midi output init");
    let port = out
        .create_virtual("portamax-poke-out")
        .expect("failed to create virtual MIDI port -- is this platform's MIDI backend available?");
    let mut conn = port;
    println!("Virtual MIDI port open. Select 'Synth' or 'Plaits' in the app, then watch here.");

    let note_on = |conn: &mut midir::MidiOutputConnection, n: u8| {
        conn.send(&[0x90, n, 100]).ok();
    };
    let note_off = |conn: &mut midir::MidiOutputConnection, n: u8| {
        conn.send(&[0x80, n, 0]).ok();
    };
    let cc = |conn: &mut midir::MidiOutputConnection, c: u8, v: u8| {
        conn.send(&[0xB0, c, v]).ok();
    };

    thread::sleep(Duration::from_secs(2));

    println!("--- phase 1: hammer the grid rapidly, overlapping notes ---");
    for round in 0..300 {
        let a = GRID_NOTES[round % 16];
        let b = GRID_NOTES[(round * 7 + 3) % 16];
        note_on(&mut conn, a);
        note_on(&mut conn, b); // overlap before releasing a
        note_off(&mut conn, a);
        note_off(&mut conn, b);
        if round % 50 == 0 {
            println!("  round {round}");
        }
    }

    println!("--- phase 2: switch top buttons + spin knobs while notes held ---");
    for round in 0..200 {
        let n = GRID_NOTES[round % 16];
        note_on(&mut conn, n);
        note_on(&mut conn, TOP_NOTES[round % 4]);
        cc(&mut conn, KNOB1_CC, (round % 127) as u8);
        cc(&mut conn, KNOB2_CC, ((round * 3) % 127) as u8);
        if round % 3 == 0 {
            note_off(&mut conn, n);
        }
    }
    for n in GRID_NOTES {
        note_off(&mut conn, n);
    }

    println!("--- phase 3: rapid-fire everything at once, no delays ---");
    for round in 0..1000 {
        for &n in &GRID_NOTES {
            note_on(&mut conn, n);
        }
        cc(&mut conn, KNOB1_CC, (round % 127) as u8);
        for &n in &GRID_NOTES {
            note_off(&mut conn, n);
        }
        if round % 200 == 0 {
            println!("  round {round}");
        }
    }

    println!("DONE POKING -- if the app is still alive, check its terminal.");
}
