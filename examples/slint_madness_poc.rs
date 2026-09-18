//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Madness: notes drifting around a ring, connected by
//! lines that bend as their positions diverge, plus 8 fixed trigger-
//! bar markers on the ring itself -- see madness.rs's own `draw()`.
//!
//! Run interactively:
//!     cargo run --example slint_madness_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_madness_poc
//!
//! Static note positions -- not wired to the real Madness app's
//! drift simulation. See slint_launcher_poc.rs's doc comment for the
//! fuller "why Slint" / "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component MadnessScreen inherits DeviceFrame {
        app-name: "MADNESS";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2  STOP", active: true },
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
                    { text: "\u{25b8}  Master Clock", value: "1.5 Hz", chip: false },
                    { text: "\u{203a}  Shape 1", value: "5 notes", chip: true },
                    { text: "\u{25b8}  Shape 2", value: "3 notes", chip: false },
                    { text: "\u{25b8}  Shape 3", value: "off", chip: false },
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

            // --- Right: the drifting polygon. ---
            Rectangle {
                width: 280px;
                Rectangle {
                    x: 30px; y: 5px;
                    width: 220px; height: 220px;
                    border-radius: 110px;
                    border-width: 1px;
                    border-color: rgba(255, 255, 255, 0.06);
                }
                // 8 fixed trigger-bar markers sitting on the ring.
                for b in [
                    { x: 130, y: 0 }, { x: 199, y: 32 }, { x: 234, y: 100 }, { x: 199, y: 168 },
                    { x: 130, y: 200 }, { x: 61, y: 168 }, { x: 26, y: 100 }, { x: 61, y: 32 },
                ] : Rectangle {
                    x: 30px + b.x * 1px - 3px; y: 5px + b.y * 1px - 3px;
                    width: 6px; height: 6px;
                    border-radius: 3px;
                    background: rgba(255, 255, 255, 0.12);
                }
                // 5 drifting note-connector lines -- a slightly
                // irregular pentagon (each note has drifted off an
                // even split), one vertex lit.
                Path {
                    x: 30px; y: 5px;
                    width: 220px; height: 220px;
                    stroke: rgba(255, 255, 255, 0.18);
                    stroke-width: 1px;
                    viewbox-width: 220; viewbox-height: 220;
                    MoveTo { x: 122; y: 8; }
                    LineTo { x: 205; y: 78; }
                    LineTo { x: 178; y: 185; }
                    LineTo { x: 58; y: 195; }
                    LineTo { x: 20; y: 90; }
                    LineTo { x: 122; y: 8; }
                }
                for n in [
                    { x: 122, y: 8, lit: true },
                    { x: 205, y: 78, lit: false },
                    { x: 178, y: 185, lit: false },
                    { x: 58, y: 195, lit: false },
                    { x: 20, y: 90, lit: false },
                ] : Rectangle {
                    x: 30px + n.x * 1px - (n.lit ? 6px : 3px);
                    y: 5px + n.y * 1px - (n.lit ? 6px : 3px);
                    width: n.lit ? 12px : 6px;
                    height: n.lit ? 12px : 6px;
                    border-radius: n.lit ? 6px : 3px;
                    background: n.lit ? root.accent : rgba(255, 255, 255, 0.25);
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
    let ui = MadnessScreen::new().unwrap();
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

    let ui = MadnessScreen::new().unwrap();
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
