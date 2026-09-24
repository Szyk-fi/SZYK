//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Bloom: a fixed circle with 8 outer trigger-reference
//! dots on its boundary and a handful of inner dots orbiting their
//! own concentric rings (pitch) -- see bloom.rs's own `draw()`.
//!
//! Run interactively:
//!     cargo run --example slint_bloom_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_bloom_poc
//!
//! Static dot positions -- not wired to the real Bloom app's clock/
//! orbit simulation. See slint_launcher_poc.rs's doc comment for the
//! fuller "why Slint" / "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component BloomScreen inherits DeviceFrame {
        app-name: "BLOOM";
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
                    { text: "\u{203a}  Shape 1", value: "6 dots, 8 outer", chip: true },
                    { text: "\u{25b8}  Shape 2", value: "4 dots, 6 outer", chip: false },
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

            // --- Right: the ring. ---
            Rectangle {
                width: 280px;
                Rectangle {
                    x: 30px; y: 5px;
                    width: 220px; height: 220px;
                    border-radius: 110px;
                    border-width: 1px;
                    border-color: rgba(255, 255, 255, 0.1);
                }
                // 8 outer trigger dots, evenly spaced, one lit.
                for o in [
                    { x: 130, y: 0, lit: true, active: true },
                    { x: 199, y: 32, lit: false, active: true },
                    { x: 234, y: 100, lit: false, active: false },
                    { x: 199, y: 168, lit: false, active: true },
                    { x: 130, y: 200, lit: false, active: true },
                    { x: 61, y: 168, lit: false, active: false },
                    { x: 26, y: 100, lit: false, active: true },
                    { x: 61, y: 32, lit: false, active: true },
                ] : Rectangle {
                    x: 30px + o.x * 1px - 5px; y: 5px + o.y * 1px - 5px;
                    width: 10px; height: 10px;
                    border-radius: 5px;
                    background: !o.active ? #1a1a1a : o.lit ? root.accent : root.accent.with-alpha(0.35);
                }
                // Inner dots, each on its own concentric ring.
                Rectangle { x: 140px; y: 60px; width: 8px; height: 8px; border-radius: 4px; background: root.accent; }
                Rectangle { x: 100px; y: 150px; width: 8px; height: 8px; border-radius: 4px; background: root.accent.with-alpha(0.7); }
                Rectangle { x: 175px; y: 130px; width: 8px; height: 8px; border-radius: 4px; background: root.accent.with-alpha(0.7); }
                Rectangle { x: 120px; y: 100px; width: 8px; height: 8px; border-radius: 4px; background: root.accent.with-alpha(0.7); }
                Rectangle { x: 90px; y: 90px; width: 8px; height: 8px; border-radius: 4px; background: root.accent.with-alpha(0.7); }
                Rectangle { x: 160px; y: 90px; width: 8px; height: 8px; border-radius: 4px; background: root.accent.with-alpha(0.7); }
            }
        }
    }
}

fn main() {
    if let Ok(path) = std::env::var("SLINT_RENDER_PNG") {
        render_to_png(&path);
        return;
    }
    let ui = BloomScreen::new().unwrap();
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

    let ui = BloomScreen::new().unwrap();
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
