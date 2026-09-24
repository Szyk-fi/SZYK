//! A *working* prototype, not a look-and-feel mockup like the other
//! `slint_*_poc.rs` examples: this drives the real `MixerApp`, the
//! real `MixerBus`, and a real `PlaitsApp` voice through Slint instead
//! of `embedded_graphics` -- turning the on-screen knob for real
//! changes the actual gain reaching your speakers, and now shows a
//! real meter bar per row (not just the printed percentage).
//!
//! Reuses the real source files directly via `#[path]` (pointing at
//! the same files `main.rs` compiles), rather than restructuring the
//! crate into a lib+bin split -- that was tried first and broke the
//! real binary's native Plaits/Clouds FFI linking (the build script's
//! static-lib link directives stopped reaching the `portamax-sim`
//! bin target once a sibling `src/lib.rs` existed; reverted
//! immediately). Each file compiles a second time here, as its own
//! copy local to this example's own binary -- harmless, since this
//! and the real `portamax-sim` binary are always separate processes.
//!
//! Run it:
//!     cargo run --example slint_mixer_live
//!
//! Controls: left knob scrolls/expands the real menu (Master, then
//! Channels -- press to expand and see Plaits' own fader once it's
//! sounding); right knob edits the selected row's level in real time;
//! hold a pad to play Plaits so there's something to hear the fader
//! move on. A real MIDI controller (if one's plugged in) drives the
//! same knobs/pads alongside the mouse -- see live_midi.rs.

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
#[path = "../src/midi_map.rs"]
mod midi_map;
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
// Flat, sibling modules at this example's own crate root -- not
// nested under an `apps` module -- since `#[path]` resolves relative
// to a *real* directory on disk, and a virtual `examples/apps/`
// directory (which nesting `mod apps { #[path = "../../src/apps/..."]
// ... }` would imply) doesn't exist. `plaits.rs`'s own
// `use super::plaits_layout::...` still resolves correctly this way,
// since `super` of a crate-root module is that same crate root, where
// `plaits_layout` is also declared, right below.
#[path = "../src/apps/mixer.rs"]
mod mixer;
#[path = "../src/apps/plaits.rs"]
mod plaits;
#[path = "../src/apps/plaits_layout.rs"]
mod plaits_layout;
#[path = "slint_common/live_midi.rs"]
mod live_midi;
/// `audio_devices.rs` calls `crate::run_inference` -- same no-op
/// stand-in `main.rs` defines at its own crate root (see its doc
/// comment); this example's crate root needs the same.
pub fn run_inference(block: &mut [f32]) {
    let _ = block;
}

use app::{App, Input};
use audio_bus::AudioBus;
use audio_devices::AudioHost;
use controller::ControllerState;
use mixer_bus::MixerBus;
use modbus::ModBus;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use util::AtomicF32;

slint::slint! {
    import { DeviceFrame, BarSegment, ParamListColumn } from "slint_common/device_frame.slint";

    export component LiveMixerScreen inherits DeviceFrame {
        app-name: "MIXER";
        breadcrumb: "LIVE -- REAL AUDIO";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3", active: false },
            { label: "F4  MIXER", active: true },
        ];

        in-out property <[string]> row-names: [];
        in-out property <[string]> row-values: [];
        in-out property <[bool]> row-is-group: [];
        in-out property <[float]> row-levels: [];
        in-out property <int> selected-row: 0;
        in-out property <bool> more-above: false;
        in-out property <bool> more-below: false;
        // Knob deltas accumulated between polls -- drained by the
        // Rust side's timer, same "accumulate ticks, apply on the
        // next tick" shape the real controller already uses.
        in-out property <float> knob1-delta: 0;
        in-out property <float> knob2-delta: 0;

        // Re-exposed so Rust can hook them: callbacks/properties
        // declared on the inherited `DeviceFrame` aren't automatically
        // visible to this component's own generated Rust API.
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
            ParamListColumn {
                width: 604px;
                row-names: root.row-names;
                row-values: root.row-values;
                row-is-group: root.row-is-group;
                row-levels: root.row-levels;
                selected-row: root.selected-row;
                more-above: root.more-above;
                more-below: root.more-below;
                accent: root.accent;
            }
        }

        // The physical knobs' real drag gesture (see
        // device_frame.slint's own `knob1-dragged`/`knob2-dragged`
        // callbacks) accumulates here -- both look right (the tick
        // still spins, handled inside DeviceFrame itself) and now
        // does something real (the Rust side's timer drains this into
        // an actual `Input` fed to the real `MixerApp`).
        knob1-dragged(delta) => { root.knob1-delta += delta; }
        knob2-dragged(delta) => { root.knob2-delta += delta; }
    }
}

fn main() {
    // --- Real engine, real buses, real registry wiring -- the same
    // shape registry.rs uses, just for two apps instead of sixteen. ---
    let master_volume = Arc::new(AtomicF32::new(1.0));
    let sensitivity = Arc::new(AtomicF32::new(0.1));
    let nav_speed = Arc::new(AtomicF32::new(3.0));
    let modbus = Arc::new(ModBus::new());
    let audio_bus = Arc::new(AudioBus::new());
    let mixer_bus = Arc::new(MixerBus::new());

    let engine = audio::new_engine(Arc::clone(&master_volume));

    let mut plaits_app = plaits::PlaitsApp::new(Arc::clone(&sensitivity), Arc::clone(&nav_speed), Arc::clone(&modbus), Arc::clone(&audio_bus), Arc::clone(&mixer_bus));
    if let Some(p) = plaits_app.audio_processor() {
        engine.add(p);
    }
    let mixer_app = mixer::MixerApp::new(Arc::clone(&master_volume), Arc::clone(&mixer_bus), Arc::clone(&nav_speed), Arc::clone(&audio_bus));

    // Real output device -- this is genuinely coming out of your
    // speakers, not a diagnostic render.
    let (audio_host, device_state) = AudioHost::open_default(Arc::clone(&engine)).expect("open default audio output");
    println!("Live Mixer prototype -- output device: {}", device_state.current_output());
    let _audio_host = audio_host; // kept alive for the process lifetime

    plaits_app.on_enter();

    let ui = LiveMixerScreen::new().unwrap();
    let mixer_app = Rc::new(RefCell::new(mixer_app));
    let plaits_app = Rc::new(RefCell::new(plaits_app));

    // Real MIDI input, alongside the mouse -- see live_midi.rs.
    let controller = Arc::new(ControllerState::new());
    let midi_map = Arc::new(midi_map::MidiMap::new());
    let _midi_connections = live_midi::connect_all(Arc::clone(&controller), Arc::clone(&midi_map), Arc::clone(&modbus));

    // Pads hold real Plaits notes -- press and hold to hear something
    // for the fader to move on, same as the real device's keyboard.
    let grid_held: Rc<RefCell<[bool; 16]>> = Rc::new(RefCell::new([false; 16]));
    let grid_for_pad = Rc::clone(&grid_held);
    ui.on_pad_toggled(move |i, down| {
        grid_for_pad.borrow_mut()[i as usize] = down;
    });

    // Edge-triggered, same one-shot shape as the live Plaits
    // prototype's own knob-press handling.
    let knob1_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let knob2_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let k1p_for_click = Rc::clone(&knob1_press);
    ui.on_knob1_clicked(move || { *k1p_for_click.borrow_mut() = true; });
    let k2p_for_click = Rc::clone(&knob2_press);
    ui.on_knob2_clicked(move || { *k2p_for_click.borrow_mut() = true; });

    // Redraw + gate the Plaits note every frame, same cadence the
    // real `Os::run()` loop drives everything at.
    let ui_weak = ui.as_weak();
    let mixer_for_timer = Rc::clone(&mixer_app);
    let plaits_for_timer = Rc::clone(&plaits_app);
    let grid_for_timer = Rc::clone(&grid_held);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(33), move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        // Mouse pads OR real MIDI notes -- either keeps a pad "held".
        let grid: [bool; 16] = std::array::from_fn(|i| grid_for_timer.borrow()[i] || controller.grid[i].load(Ordering::Relaxed));
        plaits_for_timer.borrow_mut().tick(&Input { grid, ..Default::default() });

        // Drain whatever the knobs accumulated since the last tick
        // (mouse drag + real MIDI encoder ticks) and feed it into the
        // real MixerApp exactly the way a real frame's `Input` would.
        let k1 = ui.get_knob1_delta().round() as i32 + controller.take_knob1_delta();
        let k2 = ui.get_knob2_delta().round() as i32 + controller.take_knob2_delta();
        ui.set_knob1_delta(0.0);
        ui.set_knob2_delta(0.0);
        let knob1_pressed = std::mem::take(&mut *knob1_press.borrow_mut()) || controller.take_knob1_press();
        let knob2_pressed = std::mem::take(&mut *knob2_press.borrow_mut()) || controller.take_knob2_press();
        let input = Input { knob1: k1, knob2: k2, knob1_press: knob1_pressed, knob2_press: knob2_pressed, ..Default::default() };
        let mut app = mixer_for_timer.borrow_mut();
        app.tick(&input);

        const VISIBLE_ROWS: usize = 10;
        let (rows, selected_in_window, more_above, more_below) = app.windowed_rows(VISIBLE_ROWS);
        let levels = app.windowed_levels(VISIBLE_ROWS);
        let names: Vec<slint::SharedString> = rows.iter().map(|(n, _, _)| n.as_str().into()).collect();
        let values: Vec<slint::SharedString> = rows.iter().map(|(_, v, _)| v.as_str().into()).collect();
        let is_group: Vec<bool> = rows.iter().map(|(_, _, g)| *g).collect();
        let level_values: Vec<f32> = levels.iter().map(|l| l.unwrap_or(-1.0)).collect();
        ui.set_row_names(Rc::new(slint::VecModel::from(names)).into());
        ui.set_row_values(Rc::new(slint::VecModel::from(values)).into());
        ui.set_row_is_group(Rc::new(slint::VecModel::from(is_group)).into());
        ui.set_row_levels(Rc::new(slint::VecModel::from(level_values)).into());
        ui.set_selected_row(selected_in_window as i32);
        ui.set_more_above(more_above);
        ui.set_more_below(more_below);
    });

    ui.run().unwrap();
}
