//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Pam's Workout: a list (Global BPM, per-channel Shape/
//! Target/etc) plus a bipolar scrolling CV monitor for the currently-
//! browsed channel -- see pams.rs's own `draw()`.
//!
//! Run interactively:
//!     cargo run --example slint_pams_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_pams_poc
//!
//! Static waveform -- not wired to the real Pam's Workout app's clock/
//! modulation engine. See slint_launcher_poc.rs's doc comment for the
//! fuller "why Slint" / "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component PamsScreen inherits DeviceFrame {
        app-name: "PAM'S WORKOUT";
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
                    { text: "\u{25b8}  Global", value: "120 BPM", chip: false, indent: 0 },
                    { text: "\u{25be}  Channel 1", value: "Sine \u{2192} Plaits", chip: false, indent: 0 },
                    { text: "\u{203a} Shape", value: "Sine", chip: true, indent: 1 },
                    { text: "Target", value: "Plaits: Harmonics", chip: false, indent: 1 },
                    { text: "Clock Mod", value: "/4", chip: false, indent: 1 },
                    { text: "\u{25b8}  Channel 2", value: "off", chip: false, indent: 0 },
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

            // --- Right: bipolar CV monitor for the browsed channel. ---
            VerticalLayout {
                spacing: 6px;
                Text {
                    text: "CHANNEL 1 MONITOR";
                    color: rgba(255, 255, 255, 0.4);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                Rectangle {
                    vertical-stretch: 1;
                    border-width: 1px;
                    border-color: rgba(255, 255, 255, 0.14);
                    border-radius: 4px;
                    Rectangle {
                        x: 0px; y: parent.height / 2;
                        width: parent.width; height: 1px;
                        background: rgba(255, 255, 255, 0.1);
                    }
                    Path {
                        x: 8px; y: 8px;
                        width: parent.width - 16px;
                        height: parent.height - 16px;
                        stroke: root.accent;
                        stroke-width: 2px;
                        viewbox-width: 100; viewbox-height: 40;
                        MoveTo { x: 0; y: 20; }
                        LineTo { x: 8; y: 6; } LineTo { x: 17; y: 3; } LineTo { x: 25; y: 8; }
                        LineTo { x: 33; y: 20; } LineTo { x: 42; y: 33; } LineTo { x: 50; y: 37; }
                        LineTo { x: 58; y: 32; } LineTo { x: 67; y: 20; } LineTo { x: 75; y: 7; }
                        LineTo { x: 83; y: 3; } LineTo { x: 92; y: 9; } LineTo { x: 100; y: 20; }
                    }
                }
                Text {
                    text: "value: 0.62";
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
    let ui = PamsScreen::new().unwrap();
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

    let ui = PamsScreen::new().unwrap();
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
