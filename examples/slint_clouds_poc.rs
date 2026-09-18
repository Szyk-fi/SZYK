//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Clouds: a list (Mixer/Grain/Output/Playback groups,
//! Mixer expanded to show the live input-mixer levels -- same
//! `AudioBus`-tap pattern Singularity/Prism also use) plus a single
//! wide scrolling waveform monitor instead of a 2x2 scope grid.
//!
//! Run interactively:
//!     cargo run --example slint_clouds_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_clouds_poc
//!
//! Static data/waveform -- not wired to the real Clouds app's granular
//! engine. See slint_launcher_poc.rs's doc comment for the fuller
//! "why Slint" / "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component CloudsScreen inherits DeviceFrame {
        app-name: "CLOUDS";
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

            // --- Left: the list, Mixer expanded. ---
            VerticalLayout {
                width: 230px;
                alignment: start;
                spacing: 1px;

                for row in [
                    { text: "\u{25be}  Mixer", value: "4 inputs", chip: false, indent: 0 },
                    { text: "Bloom", value: "80%", chip: true, indent: 1 },
                    { text: "Cascade", value: "100%", chip: false, indent: 1 },
                    { text: "Tape", value: "60%", chip: false, indent: 1 },
                    { text: "Voltage", value: "100%", chip: false, indent: 1 },
                    { text: "\u{25b8}  Grain", value: "Size 0.30", chip: false, indent: 0 },
                    { text: "\u{25b8}  Output", value: "Dry/Wet 0.50", chip: false, indent: 0 },
                    { text: "\u{25b8}  Playback", value: "Looping", chip: false, indent: 0 },
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

            // --- Right: the output monitor -- one wide scrolling
            // waveform instead of Voltage's 2x2 scope grid. ---
            VerticalLayout {
                spacing: 6px;
                Text {
                    text: "OUTPUT";
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
                        background: rgba(255, 255, 255, 0.08);
                    }
                    Path {
                        x: 8px; y: 8px;
                        width: parent.width - 16px;
                        height: parent.height - 16px;
                        stroke: root.accent;
                        stroke-width: 2px;
                        viewbox-width: 100; viewbox-height: 40;
                        MoveTo { x: 0; y: 20; }
                        LineTo { x: 6; y: 8; } LineTo { x: 12; y: 30; } LineTo { x: 18; y: 14; }
                        LineTo { x: 24; y: 24; } LineTo { x: 30; y: 6; } LineTo { x: 36; y: 26; }
                        LineTo { x: 42; y: 18; } LineTo { x: 48; y: 32; } LineTo { x: 54; y: 12; }
                        LineTo { x: 60; y: 22; } LineTo { x: 66; y: 10; } LineTo { x: 72; y: 28; }
                        LineTo { x: 78; y: 16; } LineTo { x: 84; y: 24; } LineTo { x: 90; y: 8; }
                        LineTo { x: 100; y: 20; }
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
    let ui = CloudsScreen::new().unwrap();
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

    let ui = CloudsScreen::new().unwrap();
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
