//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Plaits, the most visually dense remaining screen: a
//! list, a bank/engine selector with per-engine dots, and a piano-roll
//! strip along the bottom, all at once.
//!
//! Run interactively:
//!     cargo run --example slint_plaits_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_plaits_poc
//!
//! Static data (Virtual Analog engine, bank 2, playing C4) -- not
//! wired to the real Plaits app's audio/voice logic. See
//! slint_launcher_poc.rs's doc comment for the fuller "why Slint" /
//! "why this text size" context, and slint_common/device_frame.slint
//! for the physical knob-drag/button-click behavior every screen
//! shares.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component PlaitsScreen inherits DeviceFrame {
        app-name: "PLAITS";
        breadcrumb: "PLAYING: C4";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3  MIDI: OFF", active: false },
            { label: "F4  MIXER", active: false },
        ];

        property <[int]> banks: [1, 2, 3];
        property <int> current-bank: 2;
        // 8 engine dots for the current bank; dot 0 (Virtual Analog)
        // is the one actually selected.
        property <int> selected-engine: 0;

        VerticalLayout {
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 2px;
            spacing: 8px;

            HorizontalLayout {
                spacing: 24px;

                // --- Left: the list. ---
                VerticalLayout {
                    width: 260px;
                    alignment: start;
                    spacing: 1px;

                    for row in [
                        { text: "\u{203a}  Engine", value: "Virtual Analog", chip: true },
                        { text: "   Envelope", value: "Decay 0.50", chip: false },
                        { text: "   Voice", value: "Mono", chip: false },
                        { text: "   Modulators", value: "0 active", chip: false },
                        { text: "   Piano Roll", value: "shown", chip: false },
                        { text: "   Analyzer", value: "shown", chip: false },
                    ] : Rectangle {
                        height: row.chip ? 28px : 24px;
                        border-radius: 4px;
                        background: row.chip ? root.accent.with-alpha(0.13) : transparent;
                        HorizontalLayout {
                            padding-left: 6px;
                            padding-right: 6px;
                            Text {
                                text: row.text;
                                color: row.chip ? root.accent : rgba(255, 255, 255, 0.72);
                                font-family: "JetBrains Mono";
                                font-size: 14px;
                                font-weight: row.chip ? 700 : 400;
                                vertical-alignment: center;
                            }
                            Rectangle { horizontal-stretch: 1; }
                            Text {
                                text: row.value;
                                color: row.chip ? root.accent : rgba(255, 255, 255, 0.4);
                                font-family: "JetBrains Mono";
                                font-size: 13px;
                                vertical-alignment: center;
                            }
                        }
                    }
                }

                // --- Right: Model -- bank picker + this bank's 8
                // engine dots. ---
                VerticalLayout {
                    spacing: 10px;
                    Text {
                        text: "MODEL";
                        color: rgba(255, 255, 255, 0.4);
                        font-family: "JetBrains Mono";
                        font-size: 11px;
                        letter-spacing: 0.5px;
                    }
                    for b in banks : HorizontalLayout {
                        spacing: 8px;
                        height: 20px;
                        Rectangle {
                            width: 8px; height: 8px;
                            border-radius: 4px;
                            y: 6px;
                            background: b == root.current-bank ? root.accent : rgba(255, 255, 255, 0.25);
                        }
                        Text {
                            text: "Bank " + b + " (" + (b * 8 - 7) + "-" + (b * 8) + ")";
                            color: b == root.current-bank ? root.accent : rgba(255, 255, 255, 0.45);
                            font-family: "JetBrains Mono";
                            font-size: 13px;
                            font-weight: b == root.current-bank ? 700 : 400;
                        }
                    }
                    HorizontalLayout {
                        spacing: 10px;
                        padding-top: 6px;
                        for dot in [0, 1, 2, 3, 4, 5, 6, 7] : Rectangle {
                            width: 22px; height: 22px;
                            border-radius: 11px;
                            background: dot == root.selected-engine ? root.accent : rgba(255, 255, 255, 0.14);
                        }
                    }
                    Text {
                        text: "Virtual Analog (9/24)";
                        color: rgba(255, 255, 255, 0.4);
                        font-family: "JetBrains Mono";
                        font-size: 12px;
                    }
                }
            }

            // --- Piano roll strip, full width, along the bottom of
            // the body area. ---
            Rectangle {
                vertical-stretch: 1;
            }
            Rectangle {
                height: 34px;
                border-width: 1px;
                border-color: rgba(255, 255, 255, 0.12);
                border-radius: 3px;
                Rectangle {
                    x: 240px; y: 0px;
                    width: 3px; height: 34px;
                    background: root.accent;
                }
                for tick in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19] : Rectangle {
                    x: tick * 30px;
                    y: 0px;
                    width: 1px; height: 34px;
                    background: rgba(255, 255, 255, 0.06);
                }
            }
            Text {
                text: "knob1: browse   press knob1: expand/collapse";
                color: rgba(255, 255, 255, 0.35);
                font-family: "JetBrains Mono";
                font-size: 12px;
            }
        }
    }
}

fn main() {
    if let Ok(path) = std::env::var("SLINT_RENDER_PNG") {
        render_to_png(&path);
        return;
    }
    let ui = PlaitsScreen::new().unwrap();
    ui.run().unwrap();
}

/// Same headless software-render recipe as `slint_launcher_poc.rs` --
/// see its doc comment for why this renderer specifically.
fn render_to_png(path: &str) {
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, PlatformError, WindowAdapter};
    use slint::{PhysicalSize, Rgb8Pixel, SharedPixelBuffer};
    use std::rc::Rc;

    thread_local! {
        static WINDOW: Rc<MinimalSoftwareWindow> =
            MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    }
    struct HeadlessPlatform;
    impl Platform for HeadlessPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(WINDOW.with(Rc::clone))
        }
    }
    slint::platform::set_platform(Box::new(HeadlessPlatform)).expect("platform already set");
    let window = WINDOW.with(Rc::clone);
    window.set_size(PhysicalSize::new(940, 1058));

    let ui = PlaitsScreen::new().unwrap();
    ui.show().unwrap();

    let mut buffer = SharedPixelBuffer::<Rgb8Pixel>::new(940, 1058);
    let stride = buffer.width() as usize;
    window.draw_if_needed(|renderer| {
        renderer.render(buffer.make_mut_slice(), stride);
    });

    let file = std::fs::File::create(path).expect("create png");
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, buffer.width(), buffer.height());
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(buffer.as_bytes()).unwrap();
    println!("render: wrote {path}");
}
