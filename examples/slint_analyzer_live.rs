//! Live prototype for the standalone Analyzer app -- a real
//! oscilloscope/spectrum/level/pitch-detect instrument fed by its own
//! demo signal generator (sine/noise/sweep), not a pass-through of
//! another app's audio. Same `#[path]` pattern as every other live
//! prototype -- see slint_mixer_live.rs's doc comment for the fuller
//! "why" explanation.
//!
//! Run it:
//!     cargo run --example slint_analyzer_live
//!
//! Controls: left knob scrolls the 3-row list (Mode, Demo Signal,
//! Frequency); right knob edits the selected row -- try scrolling to
//! "Mode" and turning the right knob to cycle through the 5 real
//! analyzer views.

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
#[path = "../src/spleen_fonts.rs"]
mod spleen_fonts;
#[path = "../src/util.rs"]
mod util;
#[path = "../src/apps/analyzer.rs"]
mod analyzer;
pub fn run_inference(block: &mut [f32]) {
    let _ = block;
}

use analyzer::AnalyzerApp;
use app::{App, Input};
use audio_bus::AudioBus;
use audio_devices::AudioHost;
use mixer_bus::MixerBus;
use modbus::ModBus;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use util::AtomicF32;

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component LiveAnalyzerScreen inherits DeviceFrame {
        app-name: "ANALYZER";
        breadcrumb: "LIVE -- REAL SIGNAL";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3", active: false },
            { label: "F4  MIXER", active: false },
        ];

        in-out property <[string]> row-names: ["Mode", "Demo Signal", "Frequency"];
        in-out property <[string]> row-values: ["Spectrum", "Sine", "220 Hz"];
        in-out property <int> selected-row: 0;
        in-out property <float> knob1-delta: 0;
        in-out property <float> knob2-delta: 0;

        in-out property <int> mode-kind: 0;
        in-out property <string> mode-name: "Spectrum";
        in-out property <[float]> spectrum: [];
        in-out property <[float]> waveform: [];
        in-out property <float> peak-level: 0;
        in-out property <float> rms-level: 0;
        in-out property <string> pitch-name: "--";

        callback knob1-clicked();
        callback knob2-clicked();
        knob1-pressed => { root.knob1-clicked(); }
        knob2-pressed => { root.knob2-clicked(); }

        HorizontalLayout {
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 4px;
            spacing: 20px;

            VerticalLayout {
                width: 220px;
                alignment: start;
                spacing: 1px;
                for name[i] in root.row-names : Rectangle {
                    height: i == root.selected-row ? 26px : 22px;
                    border-radius: 4px;
                    background: i == root.selected-row ? root.accent.with-alpha(0.13) : transparent;
                    VerticalLayout {
                        padding-left: 4px;
                        alignment: center;
                        Text {
                            text: (i == root.selected-row ? "\u{203a} " : "") + name;
                            color: i == root.selected-row ? root.accent : rgba(255, 255, 255, 0.72);
                            font-family: "JetBrains Mono";
                            font-size: 13px;
                            font-weight: i == root.selected-row ? 700 : 400;
                        }
                        Text {
                            text: i < root.row-values.length ? root.row-values[i] : "";
                            color: i == root.selected-row ? root.accent : rgba(255, 255, 255, 0.4);
                            font-family: "JetBrains Mono";
                            font-size: 12px;
                        }
                    }
                }
            }

            VerticalLayout {
                spacing: 6px;
                Text {
                    text: root.mode-name;
                    color: root.accent;
                    font-family: "Space Grotesk";
                    font-weight: 700;
                    font-size: 18px;
                }

                Rectangle {
                    vertical-stretch: 1;

                    if root.mode-kind == 0 : HorizontalLayout {
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

                    if root.mode-kind == 1 : HorizontalLayout {
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

                    if root.mode-kind == 2 : VerticalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        alignment: center;
                        Text {
                            horizontal-alignment: center;
                            text: "Spectrogram";
                            color: rgba(255, 255, 255, 0.35);
                            font-family: "JetBrains Mono";
                            font-size: 13px;
                        }
                        Text {
                            horizontal-alignment: center;
                            text: "(not rendered in this prototype -- see Spectrum)";
                            color: rgba(255, 255, 255, 0.25);
                            font-family: "JetBrains Mono";
                            font-size: 11px;
                        }
                    }

                    if root.mode-kind == 3 : VerticalLayout {
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

                    if root.mode-kind == 4 : VerticalLayout {
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
    let _ = (modbus, audio_bus, mixer_bus); // Analyzer is self-contained -- no bus wiring needed.

    let engine = audio::new_engine(Arc::clone(&master_volume));

    let mut analyzer_app = AnalyzerApp::new();
    if let Some(p) = analyzer_app.audio_processor() {
        engine.add(p);
    }
    analyzer_app.on_enter();

    let (audio_host, device_state) = AudioHost::open_default(Arc::clone(&engine)).expect("open default audio output");
    println!("Live Analyzer prototype -- output device: {}", device_state.current_output());
    let _audio_host = audio_host;

    let ui = LiveAnalyzerScreen::new().unwrap();
    let analyzer_app = Rc::new(RefCell::new(analyzer_app));

    let knob1_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let knob2_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    // Analyzer's own `tick` doesn't read knob1_press (no groups to
    // expand), so only knob2_press (reset the selected row) matters,
    // but both are drained the same way every other live prototype
    // does for consistency.
    let k1p = Rc::clone(&knob1_press);
    ui.on_knob1_clicked(move || { *k1p.borrow_mut() = true; });
    let k2p = Rc::clone(&knob2_press);
    ui.on_knob2_clicked(move || { *k2p.borrow_mut() = true; });

    let ui_weak = ui.as_weak();
    let app_for_timer = Rc::clone(&analyzer_app);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(33), move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        let k1 = ui.get_knob1_delta().round() as i32;
        let k2 = ui.get_knob2_delta().round() as i32;
        ui.set_knob1_delta(0.0);
        ui.set_knob2_delta(0.0);
        let knob1_pressed = std::mem::take(&mut *knob1_press.borrow_mut());
        let knob2_pressed = std::mem::take(&mut *knob2_press.borrow_mut());
        let input = Input { knob1: k1, knob2: k2, knob1_press: knob1_pressed, knob2_press: knob2_pressed, ..Default::default() };

        let mut app = app_for_timer.borrow_mut();
        app.tick(&input);

        let rows = app.display_rows();
        let names: Vec<slint::SharedString> = rows.iter().map(|(n, _)| n.as_str().into()).collect();
        let values: Vec<slint::SharedString> = rows.iter().map(|(_, v)| v.as_str().into()).collect();
        ui.set_row_names(Rc::new(slint::VecModel::from(names)).into());
        ui.set_row_values(Rc::new(slint::VecModel::from(values)).into());
        ui.set_selected_row(app.selected_row() as i32);

        let (kind, kind_name) = app.mode_kind();
        ui.set_mode_kind(kind as i32);
        ui.set_mode_name(kind_name.into());
        match kind {
            0 => {
                let spectrum = app.spectrum_levels();
                ui.set_spectrum(Rc::new(slint::VecModel::from(spectrum)).into());
            }
            1 => {
                const SCOPE_BARS: usize = 48;
                let raw = app.waveform_samples();
                if !raw.is_empty() {
                    let step = (raw.len() as f32 / SCOPE_BARS as f32).max(1.0);
                    let resampled: Vec<f32> = (0..SCOPE_BARS).map(|i| raw[((i as f32 * step) as usize).min(raw.len() - 1)]).collect();
                    ui.set_waveform(Rc::new(slint::VecModel::from(resampled)).into());
                }
            }
            3 => {
                let (peak, rms) = app.level_meters();
                ui.set_peak_level(peak);
                ui.set_rms_level(rms);
            }
            4 => {
                ui.set_pitch_name(app.detected_note_name().into());
            }
            _ => {}
        }
    });

    ui.run().unwrap();
}
