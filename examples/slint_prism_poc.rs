//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Prism: an input-mixer list plus a tap-time axis -- a
//! horizontal line with one tick per active delay tap, height/
//! position showing its relative time -- see prism.rs's own `draw()`.
//!
//! Run interactively:
//!     cargo run --example slint_prism_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_prism_poc
//!
//! Static tap positions -- not wired to the real Prism app's delay
//! engine. See slint_launcher_poc.rs's doc comment for the fuller
//! "why Slint" / "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component PrismScreen inherits DeviceFrame {
        app-name: "PRISM";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3", active: false },
            { label: "F4  MIXER", active: false },
        ];

        HorizontalLayout {
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 4px;
            spacing: 20px;

            // --- Left: the list. ---
            VerticalLayout {
                width: 230px;
                alignment: start;
                spacing: 1px;

                for row in [
                    { text: "\u{25b8}  Mixer", value: "2 active", chip: false, indent: 0 },
                    { text: "\u{25be}  Multidelay", value: "Pattern A", chip: false, indent: 0 },
                    { text: "\u{203a} Taps", value: "5", chip: true, indent: 1 },
                    { text: "Time", value: "340 ms", chip: false, indent: 1 },
                    { text: "\u{25b8}  Utility", value: "Mix 0.50", chip: false, indent: 0 },
                ] : Rectangle {
                    height: row.chip ? 26px : 22px;
                    border-radius: 4px;
                    background: row.chip ? root.accent.with-alpha(0.13) : transparent;
                    HorizontalLayout {
                        padding-left: row.indent * 18px + 6px;
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

            // --- Right: the tap-time axis. ---
            VerticalLayout {
                spacing: 10px;
                padding-top: 60px;
                Text {
                    text: "TAP MAP";
                    color: rgba(255, 255, 255, 0.4);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                Rectangle {
                    height: 120px;
                    Rectangle {
                        x: 0px; y: 60px;
                        width: 100%; height: 1px;
                        background: rgba(255, 255, 255, 0.15);
                    }
                    for tap in [
                        { x: 10, drop: 20, lit: true },
                        { x: 60, drop: 45, lit: false },
                        { x: 110, drop: 15, lit: false },
                        { x: 160, drop: 55, lit: false },
                        { x: 210, drop: 30, lit: false },
                    ] : Rectangle {
                        x: tap.x * 1px; y: 60px;
                        width: 2px; height: tap.drop * 1px;
                        background: tap.lit ? root.accent : rgba(255, 255, 255, 0.3);
                        Rectangle {
                            x: -3px; y: parent.height - 3px;
                            width: 8px; height: 8px;
                            border-radius: 4px;
                            background: tap.lit ? root.accent : rgba(255, 255, 255, 0.3);
                        }
                    }
                }
                Text {
                    text: "5 taps -- longest 340 ms";
                    color: rgba(255, 255, 255, 0.4);
                    font-family: "JetBrains Mono";
                    font-size: 12px;
                }
            }
        }
    }
}

fn main() {
    if let Ok(path) = std::env::var("SLINT_RENDER_PNG") {
        render_to_png(&path);
        return;
    }
    let ui = PrismScreen::new().unwrap();
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
    window.set_size(PhysicalSize::new(1240, 560));

    let ui = PrismScreen::new().unwrap();
    ui.show().unwrap();

    let mut buffer = SharedPixelBuffer::<Rgb8Pixel>::new(1240, 560);
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
