//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Singularity: an input-mixer list (same `AudioBus`-tap
//! pattern Clouds/Prism also use) plus a chaotic orbiting point inside
//! a fixed circle -- see singularity.rs's own `draw()` (the point's
//! radius/angle both driven by the logistic map's current value, so
//! its never-repeating trajectory is visible, not just numeric).
//!
//! Run interactively:
//!     cargo run --example slint_singularity_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_singularity_poc
//!
//! Static orbit position -- not wired to the real Singularity app's
//! chaos generator. See slint_launcher_poc.rs's doc comment for the
//! fuller "why Slint" / "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component SingularityScreen inherits DeviceFrame {
        app-name: "SINGULARITY";
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
                    { text: "\u{25be}  Mixer", value: "3 active", chip: false, indent: 0 },
                    { text: "Bloom", value: "70%", chip: true, indent: 1 },
                    { text: "Plaits", value: "100%", chip: false, indent: 1 },
                    { text: "Madness", value: "45%", chip: false, indent: 1 },
                    { text: "\u{25b8}  Effect", value: "Ring Mod", chip: false, indent: 0 },
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

            // --- Right: the chaotic orbit. ---
            Rectangle {
                width: 280px;
                Rectangle {
                    x: 30px; y: 5px;
                    width: 220px; height: 220px;
                    border-radius: 110px;
                    border-width: 1px;
                    border-color: rgba(255, 255, 255, 0.1);
                }
                Rectangle {
                    x: 168px; y: 78px;
                    width: 16px; height: 16px;
                    border-radius: 8px;
                    background: root.accent;
                    drop-shadow-color: root.accent.with-alpha(0.6);
                    drop-shadow-blur: 10px;
                }
                Text {
                    x: 40px; y: 235px;
                    text: "chaos: 0.7241";
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
    let ui = SingularityScreen::new().unwrap();
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

    let ui = SingularityScreen::new().unwrap();
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
