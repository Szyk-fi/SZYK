//! Live prototype for Sequencer -- the deepest of this batch: real
//! step grid + playhead lights on the physical pad matrix (not just a
//! param list), a real Start/Stop transport wired to F3, and real
//! step-toggling from pad presses. Same `#[path]` pattern as every
//! other live prototype -- see slint_mixer_live.rs's doc comment for
//! the fuller "why" explanation.
//!
//! Run it:
//!     cargo run --example slint_sequencer_live
//!
//! Controls: press a pad to toggle that step on/off for the currently
//! browsed track (green = on, red = the step actually playing right
//! now); left knob scrolls/expands the real menu (Global + one group
//! per track); right knob edits the selected row; F3 starts/stops the
//! transport.

#[path = "../src/app.rs"]
mod app;
#[path = "../src/arpeggiator.rs"]
mod arpeggiator;
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
#[path = "../src/apps/plaits.rs"]
pub mod plaits;
#[path = "../src/apps/plaits_layout.rs"]
mod plaits_layout;
// Sequencer's own source refers to `crate::apps::plaits::...` (the
// real crate's module path) -- this re-export makes that path resolve
// to the same flat `plaits` module declared above, without needing a
// real `apps/` directory on disk. See slint_bloom_live.rs's own copy
// of this shim for the fuller explanation.
mod apps {
    pub use super::plaits;
}
#[path = "../src/apps/sequencer.rs"]
mod sequencer;
pub fn run_inference(block: &mut [f32]) {
    let _ = block;
}

use app::{App, Input};
use audio_bus::AudioBus;
use audio_devices::AudioHost;
use led_output::PadColor;
use mixer_bus::MixerBus;
use modbus::ModBus;
use sequencer::SequencerApp;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use util::AtomicF32;

slint::slint! {
    import { DeviceFrame, BarSegment, ParamListColumn } from "slint_common/device_frame.slint";

    export component LiveSequencerScreen inherits DeviceFrame {
        app-name: "SEQUENCER";
        in-out property <string> transport-label: "STOPPED";
        breadcrumb: root.transport-label;
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: root.transport-label == "RUNNING" ? "F3  STOP" : "F3  START", active: root.transport-label == "RUNNING" },
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

        callback pad-toggled(int, bool);
        pad-pressed(i, down) => { root.pad-toggled(i, down); }
        callback knob1-clicked();
        callback knob2-clicked();
        knob1-pressed => { root.knob1-clicked(); }
        knob2-pressed => { root.knob2-clicked(); }
        callback f3-clicked();
        f-pressed(i) => { if (i == 2) { root.f3-clicked(); } }

        // Re-exposed the same way `pad-pressed`/`knob1-pressed` are:
        // an inherited `in` property from `DeviceFrame` isn't visible
        // to this component's own generated Rust API, so it's bound
        // through a property of this component's own instead.
        in-out property <[color]> live-pad-colors: [
            #232323, #232323, #232323, #232323,
            #232323, #232323, #232323, #232323,
            #232323, #232323, #232323, #232323,
            #232323, #232323, #232323, #232323,
        ];
        pad-colors: root.live-pad-colors;

        HorizontalLayout {
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 4px;
            ParamListColumn {
                width: 604px;
                row-names: root.row-names;
                row-values: root.row-values;
                row-is-group: root.row-is-group;
                selected-row: root.selected-row;
                more-above: root.more-above;
                more-below: root.more-below;
                accent: root.accent;
            }
        }

        knob1-dragged(delta) => { root.knob1-delta += delta; }
        knob2-dragged(delta) => { root.knob2-delta += delta; }
    }
}

fn pad_color_to_slint(c: PadColor) -> slint::Color {
    match c {
        PadColor::Off => slint::Color::from_rgb_u8(0x23, 0x23, 0x23),
        PadColor::Green => slint::Color::from_rgb_u8(0x2E, 0xCC, 0x55),
        PadColor::Red => slint::Color::from_rgb_u8(0xFF, 0x4D, 0x4D),
        PadColor::Yellow => slint::Color::from_rgb_u8(0xE0, 0xC0, 0x30),
        PadColor::Blue => slint::Color::from_rgb_u8(0x40, 0x90, 0xE0),
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

    let mut seq_app = SequencerApp::new(Arc::clone(&sensitivity), Arc::clone(&nav_speed), Arc::clone(&modbus), Arc::clone(&audio_bus), Arc::clone(&mixer_bus));
    if let Some(p) = seq_app.audio_processor() {
        engine.add(p);
    }
    seq_app.on_enter();

    let (audio_host, device_state) = AudioHost::open_default(Arc::clone(&engine)).expect("open default audio output");
    println!("Live Sequencer prototype -- output device: {}", device_state.current_output());
    let _audio_host = audio_host;

    let ui = LiveSequencerScreen::new().unwrap();
    let seq_app = Rc::new(RefCell::new(seq_app));

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

    // Real transport toggle -- the same `App::toggle_running` the OS's
    // global F3 button calls.
    let seq_for_f3 = Rc::clone(&seq_app);
    ui.on_f3_clicked(move || {
        seq_for_f3.borrow_mut().toggle_running();
    });

    let ui_weak = ui.as_weak();
    let app_for_timer = Rc::clone(&seq_app);
    let grid_for_timer = Rc::clone(&grid_held);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(33), move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        let k1 = ui.get_knob1_delta().round() as i32;
        let k2 = ui.get_knob2_delta().round() as i32;
        ui.set_knob1_delta(0.0);
        ui.set_knob2_delta(0.0);
        let knob1_pressed = std::mem::take(&mut *knob1_press.borrow_mut());
        let knob2_pressed = std::mem::take(&mut *knob2_press.borrow_mut());
        let grid = *grid_for_timer.borrow();
        let input = Input {
            grid,
            knob1: k1,
            knob2: k2,
            knob1_press: knob1_pressed,
            knob2_press: knob2_pressed,
            ..Default::default()
        };

        let mut app = app_for_timer.borrow_mut();
        app.tick(&input);

        const VISIBLE_ROWS: usize = 10;
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
        ui.set_transport_label(if app.running().unwrap_or(false) { "RUNNING".into() } else { "STOPPED".into() });

        // Real step grid + playhead -- green for a programmed-on step,
        // red for the one actually sounding right now, off otherwise
        // (see `App::grid_led_overlay`'s own doc comment).
        let colors: Vec<slint::Color> = app.grid_led_overlay().iter().map(|c| pad_color_to_slint(*c)).collect();
        ui.set_live_pad_colors(Rc::new(slint::VecModel::from(colors)).into());
    });

    ui.run().unwrap();
}
