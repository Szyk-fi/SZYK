//! Live prototype for Settings. Same pattern as slint_mixer_live.rs --
//! see its doc comment for the fuller "why `#[path]`" explanation.
//! Drives the real `SettingsApp` through Slint: left knob
//! scrolls/expands the real menu, right knob edits the selected row.
//!
//! Run it:
//!     cargo run --example slint_settings_live

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
#[path = "../src/theme.rs"]
mod theme;
#[path = "../src/util.rs"]
mod util;
#[path = "../src/apps/settings.rs"]
mod settings;
pub fn run_inference(block: &mut [f32]) {
    let _ = block;
}

use app::{App, Input};
use audio_bus::AudioBus;
use audio_devices::AudioHost;
use settings::SettingsApp;
use mixer_bus::MixerBus;
use modbus::ModBus;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use util::AtomicF32;

slint::slint! {
    import { DeviceFrame, BarSegment, ParamListColumn } from "slint_common/device_frame.slint";

    export component LiveSettingsScreen inherits DeviceFrame {
        app-name: "SETTINGS";
        breadcrumb: "LIVE -- REAL AUDIO";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3", active: false },
            { label: "F4  MIXER", active: false },
        ];
        // Both driven live from `theme::ThemeColor` every tick --
        // see `SettingsApp`'s Hue/Saturation/Brightness rows and the
        // wheel below, which edit the same shared state.
        accent: root.live-accent;
        screen-bg: root.live-bg;
        in-out property <color> live-accent: #5CF07A;
        in-out property <color> live-bg: #0B100C;

        in-out property <[string]> row-names: [];
        in-out property <[string]> row-values: [];
        in-out property <[bool]> row-is-group: [];
        in-out property <int> selected-row: 0;
        in-out property <bool> more-above: false;
        in-out property <bool> more-below: false;
        in-out property <float> knob1-delta: 0;
        in-out property <float> knob2-delta: 0;

        // --- Settings' color wheel (see `SettingsApp::theme_extra`) ---
        in-out property <bool> theme-editing-bg: false;
        in-out property <string> theme-hex: "";
        in-out property <float> theme-marker-x: 0;
        in-out property <float> theme-marker-y: 0;
        callback theme-wheel-picked(float, float);

        callback knob1-clicked();
        callback knob2-clicked();
        knob1-pressed => { root.knob1-clicked(); }
        knob2-pressed => { root.knob2-clicked(); }

        HorizontalLayout {
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 4px;
            spacing: 16px;
            ParamListColumn {
                width: 380px;
                row-names: root.row-names;
                row-values: root.row-values;
                row-is-group: root.row-is-group;
                selected-row: root.selected-row;
                more-above: root.more-above;
                more-below: root.more-below;
                accent: root.accent;
            }
            VerticalLayout {
                width: 200px;
                alignment: start;
                spacing: 6px;
                padding-top: 8px;
                Text {
                    text: (root.theme-editing-bg ? "BACKGROUND " : "ACCENT ") + root.theme-hex;
                    color: root.accent;
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                Rectangle {
                    width: 140px; height: 140px;
                    wheel-area := TouchArea {
                        width: 100%; height: 100%;
                        pointer-event(event) => {
                            if (event.kind == PointerEventKind.down) {
                                root.theme-wheel-picked(self.mouse-x / 1px - 70, self.mouse-y / 1px - 70);
                            }
                        }
                        moved => {
                            if (self.pressed) {
                                root.theme-wheel-picked(self.mouse-x / 1px - 70, self.mouse-y / 1px - 70);
                            }
                        }
                    }
                    Rectangle {
                        width: 100%; height: 100%;
                        border-radius: 70px;
                        background: @conic-gradient(#ff0000 0deg, #ffff00 60deg, #00ff00 120deg, #00ffff 180deg, #0000ff 240deg, #ff00ff 300deg, #ff0000 360deg);
                    }
                    Rectangle {
                        width: 100%; height: 100%;
                        border-radius: 70px;
                        background: @radial-gradient(circle, #ffffff 0%, rgba(255, 255, 255, 0) 75%);
                    }
                    Rectangle {
                        width: 100%; height: 100%;
                        border-radius: 70px;
                        border-width: 1px;
                        border-color: rgba(255, 255, 255, 0.15);
                    }
                    Rectangle {
                        x: 70px + root.theme-marker-x * 1px - 5px;
                        y: 70px + root.theme-marker-y * 1px - 5px;
                        width: 10px; height: 10px;
                        border-radius: 5px;
                        background: root.theme-editing-bg ? root.live-bg : root.live-accent;
                        border-width: 2px;
                        border-color: #ffffff;
                    }
                }
                Text {
                    text: "click the wheel, or browse\nHue/Saturation/Brightness";
                    color: rgba(255, 255, 255, 0.35);
                    font-family: "JetBrains Mono";
                    font-size: 9px;
                    wrap: word-wrap;
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
    let show_cpu = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let master_volume = Arc::new(AtomicF32::new(1.0));
    let accent = Arc::new(theme::ThemeColor::new(theme::ACCENT_DEFAULT_HUE, theme::ACCENT_DEFAULT_SAT, theme::ACCENT_DEFAULT_VAL));
    let background = Arc::new(theme::ThemeColor::new(theme::BG_DEFAULT_HUE, theme::BG_DEFAULT_SAT, theme::BG_DEFAULT_VAL));

    // Settings has no audio of its own -- it edits real device state
    // (`AudioDeviceState`, the same handle `AudioHost::open_default`
    // hands back) instead of a `ModBus`/`AudioBus`/`MixerBus` triple.
    let engine = audio::new_engine(Arc::clone(&master_volume));
    let (audio_host, device_state) = AudioHost::open_default(Arc::clone(&engine)).expect("open default audio output");
    println!("Live Settings prototype -- output device: {}", device_state.current_output());
    let _audio_host = audio_host;
    let device_state = Arc::new(device_state);

    let mut the_app = SettingsApp::new(
        Arc::clone(&device_state),
        Arc::clone(&sensitivity),
        Arc::clone(&nav_speed),
        Arc::clone(&show_cpu),
        Arc::clone(&accent),
        Arc::clone(&background),
    );
    // Real device rescan -- populates the Output/Input lists (empty
    // otherwise; see `SettingsApp::on_enter`'s own doc comment).
    the_app.on_enter();

    let ui = LiveSettingsScreen::new().unwrap();
    let the_app = Rc::new(RefCell::new(the_app));

    let knob1_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let knob2_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let k1p = Rc::clone(&knob1_press);
    ui.on_knob1_clicked(move || { *k1p.borrow_mut() = true; });
    let k2p = Rc::clone(&knob2_press);
    ui.on_knob2_clicked(move || { *k2p.borrow_mut() = true; });

    let app_for_wheel = Rc::clone(&the_app);
    ui.on_theme_wheel_picked(move |x, y| {
        app_for_wheel.borrow_mut().slint_pointer_pick(x, y);
    });

    let ui_weak = ui.as_weak();
    let app_for_timer = Rc::clone(&the_app);
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

        if let app::SlintExtra::Theme(t) = app.slint_extra() {
            let (ar, ag, ab) = t.accent_rgb;
            let (br, bg, bb) = t.background_rgb;
            ui.set_live_accent(slint::Color::from_rgb_u8(ar, ag, ab));
            ui.set_live_bg(slint::Color::from_rgb_u8(br, bg, bb));
            ui.set_theme_editing_bg(t.editing_background);
            ui.set_theme_hex(if t.editing_background { format!("#{br:02X}{bg:02X}{bb:02X}") } else { format!("#{ar:02X}{ag:02X}{ab:02X}") }.into());
            ui.set_theme_marker_x(t.marker_x);
            ui.set_theme_marker_y(t.marker_y);
        }
    });

    ui.run().unwrap();
}
