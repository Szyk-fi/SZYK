//! Live prototype for Natural Gate. Same pattern as slint_mixer_live.rs --
//! see its doc comment for the fuller "why `#[path]`" explanation.
//! Drives the real `NaturalGateApp` -- an audio-to-gate/envelope
//! utility, not a sound source -- through Slint: left knob
//! scrolls/expands the real menu, right knob edits the selected row.
//! A real Plaits voice is wired in as the tapped source (see
//! audio_bus.rs) -- hold a pad to hear it, and watch the gate/
//! envelope meter react to your playing.
//!
//! Run it:
//!     cargo run --example slint_natural_gate_live
//!
//! Controls: hold a pad to play Plaits (the tapped source); scroll to
//! "Source" and confirm it reads "Plaits"; scroll to "Threshold" and
//! turn the right knob to change how loud the pad has to be before
//! the gate opens.

#[path = "../src/app.rs"]
mod app;
#[path = "../src/audio.rs"]
mod audio;
#[path = "../src/audio_bus.rs"]
mod audio_bus;
#[path = "../src/audio_devices.rs"]
mod audio_devices;
#[path = "../src/controller.rs"]
mod controller;
#[path = "../src/display.rs"]
mod display;
#[path = "../src/led_output.rs"]
mod led_output;
#[path = "../src/mixer_bus.rs"]
mod mixer_bus;
#[path = "../src/modbus.rs"]
mod modbus;
#[path = "../src/paramlist.rs"]
mod paramlist;
#[path = "../src/plaits_ffi.rs"]
mod plaits_ffi;
#[path = "../src/spleen_fonts.rs"]
mod spleen_fonts;
#[path = "../src/util.rs"]
mod util;
#[path = "../src/arpeggiator.rs"]
mod arpeggiator;
#[path = "../src/apps/plaits.rs"]
mod plaits;
#[path = "../src/apps/plaits_layout.rs"]
mod plaits_layout;
#[path = "../src/apps/natural_gate.rs"]
mod natural_gate;
pub fn run_inference(block: &mut [f32]) {
    let _ = block;
}

use app::{App, Input};
use audio_bus::AudioBus;
use audio_devices::AudioHost;
use natural_gate::NaturalGateApp;
use mixer_bus::MixerBus;
use modbus::ModBus;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use util::AtomicF32;

slint::slint! {
    import { DeviceFrame, BarSegment, ParamListColumn } from "slint_common/device_frame.slint";

    export component LiveNaturalGateScreen inherits DeviceFrame {
        app-name: "NATURAL GATE";
        breadcrumb: "LIVE -- REAL AUDIO";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3", active: false },
            { label: "F4  MIXER", active: false },
        ];

        in-out property <[string]> row-names: [];
        in-out property <[string]> row-values: [];
        in-out property <[bool]> row-is-group: [];
        in-out property <int> selected-row: 0;
        in-out property <bool> more-above: false;
        in-out property <bool> more-below: false;
        in-out property <float> knob1-delta: 0;
        in-out property <float> knob2-delta: 0;

        in-out property <float> envelope: 0;
        in-out property <bool> gate: false;

        callback pad-toggled(int, bool);
        pad-pressed(i, down) => { root.pad-toggled(i, down); }
        callback knob1-clicked();
        callback knob2-clicked();
        knob1-pressed => { root.knob1-clicked(); }
        knob2-pressed => { root.knob2-clicked(); }

        HorizontalLayout {
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 4px;
            spacing: 20px;

            ParamListColumn {
                width: 420px;
                row-names: root.row-names;
                row-values: root.row-values;
                row-is-group: root.row-is-group;
                selected-row: root.selected-row;
                more-above: root.more-above;
                more-below: root.more-below;
                accent: root.accent;
            }

            // Real gate LED + envelope meter -- what the audio thread's
            // real envelope follower/hysteresis comparator is doing
            // right now.
            VerticalLayout {
                spacing: 10px;
                padding-top: 10px;
                Rectangle {
                    width: 28px; height: 28px;
                    border-radius: 14px;
                    background: root.gate ? root.accent : rgba(255, 255, 255, 0.1);
                    drop-shadow-color: root.gate ? root.accent.with-alpha(0.7) : transparent;
                    drop-shadow-blur: 10px;
                    animate background { duration: 40ms; }
                }
                Text {
                    text: "GATE";
                    color: rgba(255, 255, 255, 0.4);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                }
                Rectangle { height: 10px; }
                Rectangle {
                    width: 140px; height: 14px;
                    border-radius: 3px;
                    border-width: 1px;
                    border-color: rgba(255, 255, 255, 0.15);
                    Rectangle {
                        x: 0px; y: 0px;
                        width: parent.width * max(0.0, min(1.0, root.envelope));
                        height: 100%;
                        background: root.accent;
                    }
                }
                Text {
                    text: "ENVELOPE";
                    color: rgba(255, 255, 255, 0.4);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                }
            }
        }

        knob1-dragged(delta) => { root.knob1-delta += delta; }
        knob2-dragged(delta) => { root.knob2-delta += delta; }
    }
}

fn main() {
    let sensitivity = Arc::new(AtomicF32::new(0.1));
    let nav_speed = Arc::new(AtomicF32::new(3.0));
    let master_volume = Arc::new(AtomicF32::new(1.0));
    let modbus = Arc::new(ModBus::new());
    let audio_bus = Arc::new(AudioBus::new());
    let mixer_bus = Arc::new(MixerBus::new());

    let engine = audio::new_engine(Arc::clone(&master_volume));

    let mut the_app = NaturalGateApp::new(Arc::clone(&sensitivity), Arc::clone(&nav_speed), Arc::clone(&modbus), Arc::clone(&audio_bus), Arc::clone(&mixer_bus));
    // A real tapped source (Plaits, held on a note) so there's
    // something for the envelope follower to actually react to.
    let mut plaits_app = plaits::PlaitsApp::new(Arc::clone(&sensitivity), Arc::clone(&nav_speed), Arc::clone(&modbus), Arc::clone(&audio_bus), Arc::clone(&mixer_bus));
    if let Some(p) = plaits_app.audio_processor() {
        engine.add(p);
    }
    plaits_app.on_enter();
    if let Some(p) = the_app.audio_processor() {
        engine.add(p);
    }
    the_app.on_enter();

    let (audio_host, device_state) = AudioHost::open_default(Arc::clone(&engine)).expect("open default audio output");
    println!("Live Natural Gate prototype -- output device: {}", device_state.current_output());
    let _audio_host = audio_host;

    let ui = LiveNaturalGateScreen::new().unwrap();
    let the_app = Rc::new(RefCell::new(the_app));
    let plaits_app = Rc::new(RefCell::new(plaits_app));

    let grid_held: Rc<RefCell<[bool; 16]>> = Rc::new(RefCell::new([false; 16]));
    let grid_for_pad = Rc::clone(&grid_held);
    ui.on_pad_toggled(move |i, down| {
        grid_for_pad.borrow_mut()[i as usize] = down;
    });

    let knob1_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let knob2_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let k1p = Rc::clone(&knob1_press);
    ui.on_knob1_clicked(move || { *k1p.borrow_mut() = true; });
    let k2p = Rc::clone(&knob2_press);
    ui.on_knob2_clicked(move || { *k2p.borrow_mut() = true; });

    let ui_weak = ui.as_weak();
    let app_for_timer = Rc::clone(&the_app);
    let plaits_for_timer = Rc::clone(&plaits_app);
    let grid_for_timer = Rc::clone(&grid_held);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(33), move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        let grid = *grid_for_timer.borrow();
        plaits_for_timer.borrow_mut().tick(&Input { grid, ..Default::default() });

        let k1 = ui.get_knob1_delta().round() as i32;
        let k2 = ui.get_knob2_delta().round() as i32;
        ui.set_knob1_delta(0.0);
        ui.set_knob2_delta(0.0);
        let knob1_pressed = std::mem::take(&mut *knob1_press.borrow_mut());
        let knob2_pressed = std::mem::take(&mut *knob2_press.borrow_mut());
        let input = Input { knob1: k1, knob2: k2, knob1_press: knob1_pressed, knob2_press: knob2_pressed, ..Default::default() };

        let mut app = app_for_timer.borrow_mut();
        app.tick(&input);

        const VISIBLE_ROWS: usize = 8;
        let (rows, selected_in_window, more_above, more_below) = app.windowed_rows(VISIBLE_ROWS);
        let names: Vec<slint::SharedString> = rows.iter().map(|(n, _, _)| n.as_str().into()).collect();
        let values: Vec<slint::SharedString> = rows.iter().map(|(_, v, _)| v.as_str().into()).collect();
        let is_group: Vec<bool> = rows.iter().map(|(_, _, g)| *g).collect();
        ui.set_row_names(Rc::new(slint::VecModel::from(names)).into());
        ui.set_row_values(Rc::new(slint::VecModel::from(values)).into());
        ui.set_row_is_group(Rc::new(slint::VecModel::from(is_group)).into());
        ui.set_selected_row(selected_in_window as i32);
        ui.set_more_above(more_above);
        ui.set_more_below(more_below);

        let (envelope, gate) = app.envelope_and_gate(0);
        ui.set_envelope(envelope);
        ui.set_gate(gate > 0.0);
    });

    ui.run().unwrap();
}
