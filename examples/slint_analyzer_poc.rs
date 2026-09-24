//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Analyzer: a different shape than every other screen --
//! a short 3-row list across the *top* (Mode/Demo Signal/Frequency),
//! then a full-width plot filling the rest (spectrum bars, the
//! default mode) -- see analyzer.rs's own `draw()`/`draw_spectrum()`.
//!
//! Run interactively:
//!     cargo run --example slint_analyzer_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_analyzer_poc
//!
//! Static bar heights -- not wired to the real Analyzer app's FFT.
//! See slint_launcher_poc.rs's doc comment for the fuller "why Slint"
//! / "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component AnalyzerScreen inherits DeviceFrame {
        app-name: "ANALYZER";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3", active: false },
            { label: "F4  MIXER", active: false },
        ];

        VerticalLayout {
            height: 100%;
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 2px;
            spacing: 6px;

            // --- Top: the short 3-row list -- full width, unlike
            // every other screen's list+panel side-by-side shape. ---
            for row in [
                { name: "Mode", value: "Spectrum" },
                { name: "Demo Signal", value: "Sweep" },
                { name: "Frequency", value: "440 Hz" },
            ] : Rectangle {
                height: 22px;
                HorizontalLayout {
                    Text {
                        text: row.name;
                        color: rgba(255, 255, 255, 0.6);
                        font-family: "JetBrains Mono";
                        font-size: 14px;
                        vertical-alignment: center;
                    }
                    Rectangle { horizontal-stretch: 1; }
                    Text {
                        text: row.value;
                        color: root.accent;
                        font-family: "JetBrains Mono";
                        font-size: 14px;
                        vertical-alignment: center;
                    }
                }
            }

            // --- Bottom: the full-width spectrum plot. ---
            Rectangle {
                vertical-stretch: 1;
                border-width: 1px;
                border-color: rgba(255, 255, 255, 0.1);
                border-radius: 4px;
                HorizontalLayout {
                    padding: 6px;
                    spacing: 3px;
                    // Each bar is a plain Rectangle bottom-anchored
                    // *inside* a HorizontalLayout-managed cell, not a
                    // direct child of the layout itself -- a direct
                    // child's own `y`/`height` get overridden by the
                    // layout (same class of bug the launcher/Settings
                    // screens hit earlier: a layout silently ignoring
                    // manual geometry on its own direct children).
                    for h in [
                        30, 55, 80, 65, 95, 110, 130, 100, 85, 120, 140, 115,
                        90, 105, 75, 60, 45, 70, 50, 35, 25, 40, 20, 15,
                    ] : Rectangle {
                        vertical-stretch: 1;
                        Rectangle {
                            y: parent.height - h * 1px;
                            height: h * 1px;
                            width: 100%;
                            background: root.accent.with-alpha(0.75);
                        }
                    }
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
    let ui = AnalyzerScreen::new().unwrap();
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

    let ui = AnalyzerScreen::new().unwrap();
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
