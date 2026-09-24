//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Tape: 4 track lanes with a shared playhead line, each
//! colored by state (recording/has-content/empty) -- see tape.rs's
//! own `draw()`.
//!
//! Run interactively:
//!     cargo run --example slint_tape_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_tape_poc
//!
//! Static track states -- not wired to the real Tape app's loop
//! engine. See slint_launcher_poc.rs's doc comment for the fuller
//! "why Slint" / "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component TapeScreen inherits DeviceFrame {
        app-name: "TAPE";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3", active: false },
            { label: "F4  MIXER", active: false },
        ];

        property <float> playhead: 0.4;

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
                    { text: "\u{203a}  Track 1", value: "recording", chip: true },
                    { text: "\u{25b8}  Track 2", value: "playing", chip: false },
                    { text: "\u{25b8}  Track 3", value: "muted", chip: false },
                    { text: "\u{25b8}  Track 4", value: "empty", chip: false },
                    { text: "\u{25b8}  Loop Length", value: "2.0 s", chip: false },
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

            // --- Right: 4 track lanes with a shared playhead. ---
            VerticalLayout {
                spacing: 14px;
                alignment: start;
                for t in [
                    { label: "T1 REC", state: "rec" },
                    { label: "T2 PLAY", state: "play" },
                    { label: "T3 MUTE", state: "mute" },
                    { label: "T4 EMPTY", state: "empty" },
                ] : Rectangle {
                    height: 40px;
                    border-radius: 4px;
                    border-width: 1px;
                    border-color: rgba(255, 255, 255, 0.14);
                    background: t.state == "rec" ? rgba(120, 20, 20, 0.6) : t.state == "play" ? root.accent.with-alpha(0.18) : rgba(255, 255, 255, 0.02);

                    Rectangle {
                        x: parent.width * root.playhead;
                        width: 2px; height: 100%;
                        background: root.accent;
                    }
                    Text {
                        x: 8px; y: 8px;
                        text: t.label;
                        color: t.state == "rec" ? #ff6b6b : t.state == "play" ? root.accent : rgba(255, 255, 255, 0.4);
                        font-family: "JetBrains Mono";
                        font-size: 13px;
                        font-weight: 700;
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
    let ui = TapeScreen::new().unwrap();
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

    let ui = TapeScreen::new().unwrap();
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
