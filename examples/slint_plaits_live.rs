//! A *working* prototype -- see `slint_mixer_live.rs`'s doc comment
//! for the fuller "why `#[path]`, not a lib target" explanation, which
//! applies here identically. This one drives the real `PlaitsApp`:
//! the physical pad matrix triggers real notes on a real Plaits voice
//! (Mutable Instruments' actual DSP, via `plaits_ffi`), the left knob
//! scrolls the real parameter list, and the right knob edits whatever
//! leaf is selected (Engine, Harmonics, Timbre, ...) in real time.
//!
//! Run it:
//!     cargo run --example slint_plaits_live
//!
//! Controls: click and hold a pad to play a note (mono -- last one
//! held wins, same as the real hardware's single voice); left knob
//! scrolls the list; right knob edits the selected row (try scrolling
//! to "Engine" and turning the right knob to hear it change engines
//! live).

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
mod plaits;
#[path = "../src/apps/plaits_layout.rs"]
mod plaits_layout;
/// Same no-op stand-in as `main.rs`'s own copy -- see its doc comment.
pub fn run_inference(block: &mut [f32]) {
    let _ = block;
}

use app::{App, Input};
use audio_bus::AudioBus;
use audio_devices::AudioHost;
use mixer_bus::MixerBus;
use modbus::ModBus;
use plaits::PlaitsApp;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use util::AtomicF32;

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component LivePlaitsScreen inherits DeviceFrame {
        app-name: "PLAITS";
        breadcrumb: "LIVE -- REAL VOICE";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3", active: false },
            { label: "F4  MIXER", active: false },
        ];

        in-out property <[string]> row-names: ["Engine"];
        in-out property <[string]> row-values: ["Virtual Analog"];
        in-out property <[bool]> row-is-group: [true];
        in-out property <int> selected-row: 0;
        in-out property <bool> more-above: false;
        in-out property <bool> more-below: false;
        in-out property <string> engine-name: "Virtual Analog";
        // Real bank/LED split -- see `PlaitsApp::engine_bank_and_led`.
        // Three banks of 8 engines each, same as the real firmware's
        // Model panel. Bank colors match the real firmware's own
        // per-bank palette (`BANK_COLORS` in plaits.rs): red, green,
        // yellow.
        in-out property <int> engine-bank: 0;
        in-out property <int> engine-led: 0;
        property <[color]> bank-colors: [#00ca21, #de0c19, #ffa600];
        // Real, mode-switching analyzer data -- see
        // `PlaitsApp::analyzer_kind`/`spectrum_levels`/
        // `waveform_samples`/`level_meters`/`detected_note_name`.
        in-out property <int> analyzer-kind: 0;
        in-out property <string> analyzer-name: "Spectrum";
        in-out property <[float]> spectrum: [];
        in-out property <[float]> waveform: [];
        in-out property <float> peak-level: 0;
        in-out property <float> rms-level: 0;
        in-out property <string> pitch-name: "--";
        in-out property <float> knob1-delta: 0;
        in-out property <float> knob2-delta: 0;

        // Re-exposed so Rust can hook them: callbacks declared on the
        // inherited `DeviceFrame` aren't automatically visible to this
        // component's own generated Rust API, so each needs an
        // explicit forwarding handler + a callback of this
        // component's own.
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

            // --- Left: the real list, windowed + scrolled by
            // `PlaitsApp::windowed_rows` so a list longer than fits
            // on screen scrolls with the selection instead of running
            // off the bottom. ---
            VerticalLayout {
                width: 290px;
                alignment: start;
                spacing: 1px;
                Text {
                    height: root.more-above ? 14px : 0px;
                    text: root.more-above ? "^ more" : "";
                    color: rgba(255, 255, 255, 0.3);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                }
                for name[i] in root.row-names : Rectangle {
                    height: i == root.selected-row ? 26px : 22px;
                    border-radius: 4px;
                    background: i == root.selected-row ? root.accent.with-alpha(0.13) : transparent;
                    HorizontalLayout {
                        padding-left: (i < root.row-is-group.length && root.row-is-group[i]) ? 6px : 20px;
                        padding-right: 6px;
                        Text {
                            text: (i == root.selected-row ? "\u{203a} " : "") + name;
                            color: i == root.selected-row ? root.accent : rgba(255, 255, 255, 0.72);
                            font-family: "JetBrains Mono";
                            font-size: 13px;
                            font-weight: i == root.selected-row ? 700 : 400;
                            vertical-alignment: center;
                        }
                        Rectangle { horizontal-stretch: 1; }
                        Text {
                            text: i < root.row-values.length ? root.row-values[i] : "";
                            color: i == root.selected-row ? root.accent : rgba(255, 255, 255, 0.4);
                            font-family: "JetBrains Mono";
                            font-size: 12px;
                            vertical-alignment: center;
                        }
                    }
                }
                Text {
                    height: root.more-below ? 14px : 0px;
                    text: root.more-below ? "v more" : "";
                    color: rgba(255, 255, 255, 0.3);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                }
            }

            // --- Right: the real engine-selection display up top
            // (compact -- bank legend inline, one LED row, engine
            // name), with the rest of the height given to a real
            // spectrum analyzer of Plaits' own live output. ---
            VerticalLayout {
                spacing: 6px;
                Text {
                    text: "MODEL";
                    color: rgba(255, 255, 255, 0.4);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                HorizontalLayout {
                    spacing: 10px;
                    height: 16px;
                    for b in [0, 1, 2] : HorizontalLayout {
                        spacing: 4px;
                        Rectangle {
                            width: 7px; height: 7px;
                            border-radius: 4px;
                            y: 5px;
                            background: b == root.engine-bank ? root.bank-colors[b] : rgba(255, 255, 255, 0.2);
                        }
                        Text {
                            text: "B" + (b + 1);
                            color: b == root.engine-bank ? root.bank-colors[b] : rgba(255, 255, 255, 0.4);
                            font-family: "JetBrains Mono";
                            font-size: 11px;
                            font-weight: b == root.engine-bank ? 700 : 400;
                        }
                    }
                }
                HorizontalLayout {
                    spacing: 8px;
                    height: 22px;
                    for dot in [0, 1, 2, 3, 4, 5, 6, 7] : Rectangle {
                        width: 20px; height: 20px;
                        border-radius: 10px;
                        background: dot == root.engine-led ? root.bank-colors[root.engine-bank] : rgba(255, 255, 255, 0.14);
                        animate background { duration: 60ms; }
                    }
                }
                Text {
                    text: root.engine-name;
                    color: root.bank-colors[root.engine-bank];
                    font-family: "Space Grotesk";
                    font-weight: 700;
                    font-size: 22px;
                    wrap: word-wrap;
                    width: 260px;
                }

                Rectangle { height: 6px; }
                Text {
                    text: root.analyzer-name;
                    color: rgba(255, 255, 255, 0.4);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                // Real, mode-switching analyzer -- mirrors the actual
                // firmware's `draw_current_analyzer`: Spectrum (FFT
                // bars), Oscilloscope (waveform), Level Meter (peak/
                // RMS bars), or Pitch Detect (autocorrelation), picked
                // by the same `Params.analyzer_type` the real screen
                // reads (browse to "Analyzer" in the list, turn knob2
                // to cycle). Wrapped in an outer layout-managed cell +
                // an inner manually positioned one, same "layout
                // children can't have manual y/height" workaround the
                // Analyzer POC uses.
                Rectangle {
                    vertical-stretch: 1;

                    if root.analyzer-kind == 0 : HorizontalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        spacing: 2px;
                        for level in root.spectrum : Rectangle {
                            horizontal-stretch: 1;
                            Rectangle {
                                y: parent.height - self.height;
                                width: 100%;
                                height: level * parent.height;
                                background: root.accent;
                            }
                        }
                    }

                    if root.analyzer-kind == 1 : HorizontalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        spacing: 1px;
                        for s in root.waveform : Rectangle {
                            horizontal-stretch: 1;
                            Rectangle {
                                property <length> mag: (s >= 0 ? s : -s) * (parent.height / 2);
                                y: s >= 0 ? parent.height / 2 - mag : parent.height / 2;
                                width: 100%;
                                height: mag;
                                background: root.accent;
                            }
                        }
                        Rectangle {
                            y: parent.height / 2;
                            width: 100%; height: 1px;
                            background: rgba(255, 255, 255, 0.12);
                        }
                    }

                    if root.analyzer-kind == 2 : VerticalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        spacing: 14px;
                        for row in [{ label: "Peak", level: root.peak-level }, { label: "RMS", level: root.rms-level }] : VerticalLayout {
                            height: 30px;
                            spacing: 4px;
                            Rectangle {
                                height: 20px;
                                border-width: 1px;
                                border-color: rgba(255, 255, 255, 0.15);
                                Rectangle {
                                    x: 0px; y: 0px;
                                    width: parent.width * row.level;
                                    height: 100%;
                                    background: root.accent;
                                }
                            }
                            Text {
                                text: row.label;
                                color: rgba(255, 255, 255, 0.5);
                                font-family: "JetBrains Mono";
                                font-size: 11px;
                            }
                        }
                    }

                    if root.analyzer-kind == 3 : VerticalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        alignment: center;
                        Text {
                            horizontal-alignment: center;
                            text: root.pitch-name;
                            color: root.accent;
                            font-family: "Space Grotesk";
                            font-weight: 700;
                            font-size: 36px;
                        }
                        Text {
                            horizontal-alignment: center;
                            text: "(autocorrelation)";
                            color: rgba(255, 255, 255, 0.35);
                            font-family: "JetBrains Mono";
                            font-size: 11px;
                        }
                    }
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

    let mut plaits_app = PlaitsApp::new(Arc::clone(&sensitivity), Arc::clone(&nav_speed), Arc::clone(&modbus), Arc::clone(&audio_bus), Arc::clone(&mixer_bus));
    if let Some(p) = plaits_app.audio_processor() {
        engine.add(p);
    }
    plaits_app.on_enter();

    let (audio_host, device_state) = AudioHost::open_default(Arc::clone(&engine)).expect("open default audio output");
    println!("Live Plaits prototype -- output device: {}", device_state.current_output());
    let _audio_host = audio_host;

    let ui = LivePlaitsScreen::new().unwrap();
    let plaits_app = Rc::new(RefCell::new(plaits_app));

    // Real, currently-held pad state -- updated by `pad-pressed`,
    // drained into a real `Input.grid` every tick, same "level state"
    // shape `Input::grid` already has (not edge-triggered).
    let grid_held: Rc<RefCell<[bool; 16]>> = Rc::new(RefCell::new([false; 16]));
    let grid_for_pad = Rc::clone(&grid_held);
    ui.on_pad_toggled(move |i, down| {
        grid_for_pad.borrow_mut()[i as usize] = down;
    });

    // Edge-triggered: set true by the knob's `clicked` callback, then
    // drained (read + reset) by the next timer tick -- same one-shot
    // shape `Input::knob1_press`/`knob2_press` already has.
    let knob1_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let knob2_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let k1p_for_click = Rc::clone(&knob1_press);
    ui.on_knob1_clicked(move || { *k1p_for_click.borrow_mut() = true; });
    let k2p_for_click = Rc::clone(&knob2_press);
    ui.on_knob2_clicked(move || { *k2p_for_click.borrow_mut() = true; });

    let ui_weak = ui.as_weak();
    let plaits_for_timer = Rc::clone(&plaits_app);
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

        let mut app = plaits_for_timer.borrow_mut();
        app.tick(&input);

        // Windowed + sticky-scrolled to `VISIBLE_ROWS` -- matches the
        // real firmware's `ParamList::draw` scrolling instead of
        // dumping every row into a list that runs off the screen.
        const VISIBLE_ROWS: usize = 9;
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
        ui.set_engine_name(app.engine_name().into());
        let (bank, led) = app.engine_bank_and_led();
        ui.set_engine_bank(bank as i32);
        ui.set_engine_led(led as i32);
        let (kind, kind_name) = app.analyzer_kind();
        ui.set_analyzer_kind(kind as i32);
        ui.set_analyzer_name(kind_name.into());
        match kind {
            0 => {
                let spectrum: Vec<f32> = app.spectrum_levels();
                ui.set_spectrum(Rc::new(slint::VecModel::from(spectrum)).into());
            }
            1 => {
                // Resampled to a fixed bar count -- the real 256-sample
                // waveform buffer is finer than useful to render as
                // this many discrete bars.
                const SCOPE_BARS: usize = 48;
                let raw = app.waveform_samples();
                let step = (raw.len() as f32 / SCOPE_BARS as f32).max(1.0);
                let resampled: Vec<f32> = (0..SCOPE_BARS).map(|i| raw[((i as f32 * step) as usize).min(raw.len() - 1)]).collect();
                ui.set_waveform(Rc::new(slint::VecModel::from(resampled)).into());
            }
            2 => {
                let (peak, rms) = app.level_meters();
                ui.set_peak_level(peak);
                ui.set_rms_level(rms);
            }
            _ => {
                ui.set_pitch_name(app.detected_note_name().into());
            }
        }
    });

    ui.run().unwrap();
}
