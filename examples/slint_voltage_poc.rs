//! Same scoping-only exercise as `slint_launcher_poc.rs`/
//! `slint_mixer_poc.rs`, in the same hardware chrome
//! (`slint_common/device_frame.slint`) -- Voltage, because the
//! original "Portamax UI Mockup" design artifact's `Main.dc.html`
//! literally depicted this screen (list + 2x2 mini-waveform panel
//! grid), so this is close to a direct port of that reference rather
//! than a new layout guessed at in its style.
//!
//! Run interactively:
//!     cargo run --example slint_voltage_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_voltage_poc
//!
//! Not wired to the real Voltage app's logic -- static data lifted
//! straight from the mockup (Filter 1.2kHz/0.30, etc). See
//! slint_launcher_poc.rs's doc comment for the fuller "why Slint" /
//! "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component VoltageScreen inherits DeviceFrame {
        app-name: "VOLTAGE";
        breadcrumb: "FILTER  \u{203a}  ENV AMOUNT";
        bar: [
            { label: "F1  SETTINGS", active: false },
            { label: "F2  HOME", active: false },
            { label: "F3  STOP", active: true },
            { label: "F4  MIXER", active: false },
        ];

        HorizontalLayout {
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 4px;
            spacing: 20px;

            // --- Left: the list -- groups collapsed except Filter,
            // Env Amount selected (matches the mockup exactly). ---
            VerticalLayout {
                width: 330px;
                alignment: start;
                spacing: 1px;

                for row in [
                    { text: "\u{25b8}  OSCILLATORS", value: "Saw + Saw \u{b7} x1", dim: true, indent: 0, chip: false },
                    { text: "\u{25be}  FILTER", value: "1.2kHz \u{b7} 0.30", dim: false, indent: 0, chip: false },
                    { text: "Cutoff", value: "1.2 kHz", dim: true, indent: 1, chip: false },
                    { text: "Resonance", value: "0.30", dim: true, indent: 1, chip: false },
                    { text: "\u{203a}  Env Amount", value: "0.00", dim: false, indent: 1, chip: true },
                    { text: "\u{25b8}  AMP ENVELOPE", value: "A20 D300 S.80 R300", dim: true, indent: 0, chip: false },
                    { text: "\u{25b8}  LFO", value: "4.0Hz \u{2192} Pitch", dim: true, indent: 0, chip: false },
                    { text: "\u{25b8}  PRESETS", value: "3/8 saved", dim: true, indent: 0, chip: false },
                ] : Rectangle {
                    height: row.chip ? 28px : 24px;
                    border-radius: 4px;
                    background: row.chip ? root.accent.with-alpha(0.13) : transparent;
                    HorizontalLayout {
                        padding-left: row.indent * 18px + 6px;
                        padding-right: 6px;
                        Text {
                            text: row.text;
                            color: row.chip ? root.accent : row.dim ? rgba(255, 255, 255, 0.55) : rgba(255, 255, 255, 0.88);
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

            // --- Right: 2x2 mini-scope grid. ---
            GridLayout {
                spacing: 10px;
                padding-top: 2px;
                Row {
                    Rectangle {
                        border-width: 1px;
                        border-color: rgba(255, 255, 255, 0.14);
                        border-radius: 4px;
                        VerticalLayout {
                            padding: 6px;
                            Text { text: "OSC"; color: rgba(255, 255, 255, 0.4); font-family: "JetBrains Mono"; font-size: 11px; letter-spacing: 0.5px; }
                            Path {
                                vertical-stretch: 1;
                                stroke: root.accent;
                                stroke-width: 2px;
                                viewbox-width: 100; viewbox-height: 36;
                                MoveTo { x: 0; y: 30; }
                                LineTo { x: 18; y: 6; } LineTo { x: 18; y: 30; }
                                LineTo { x: 36; y: 6; } LineTo { x: 36; y: 30; }
                                LineTo { x: 54; y: 6; } LineTo { x: 54; y: 30; }
                                LineTo { x: 72; y: 6; } LineTo { x: 72; y: 30; }
                                LineTo { x: 90; y: 6; } LineTo { x: 90; y: 30; }
                                LineTo { x: 100; y: 18; }
                            }
                        }
                    }
                    Rectangle {
                        border-width: 1px;
                        border-color: rgba(255, 255, 255, 0.14);
                        border-radius: 4px;
                        VerticalLayout {
                            padding: 6px;
                            Text { text: "FILTER"; color: rgba(255, 255, 255, 0.4); font-family: "JetBrains Mono"; font-size: 11px; letter-spacing: 0.5px; }
                            Path {
                                vertical-stretch: 1;
                                stroke: root.accent;
                                stroke-width: 2px;
                                viewbox-width: 100; viewbox-height: 36;
                                MoveTo { x: 0; y: 32; }
                                CubicTo { x: 38; y: 4; control-1-x: 18; control-1-y: 32; control-2-x: 24; control-2-y: 4; }
                                CubicTo { x: 100; y: 30; control-1-x: 50; control-1-y: 4; control-2-x: 52; control-2-y: 26; }
                            }
                        }
                    }
                }
                Row {
                    Rectangle {
                        border-width: 1px;
                        border-color: rgba(255, 255, 255, 0.14);
                        border-radius: 4px;
                        VerticalLayout {
                            padding: 6px;
                            Text { text: "AMP ENV"; color: rgba(255, 255, 255, 0.4); font-family: "JetBrains Mono"; font-size: 11px; letter-spacing: 0.5px; }
                            Path {
                                vertical-stretch: 1;
                                stroke: root.accent;
                                stroke-width: 2px;
                                viewbox-width: 100; viewbox-height: 36;
                                MoveTo { x: 0; y: 32; }
                                LineTo { x: 14; y: 4; } LineTo { x: 30; y: 16; } LineTo { x: 62; y: 16; } LineTo { x: 100; y: 32; }
                            }
                        }
                    }
                    Rectangle {
                        border-width: 1px;
                        border-color: rgba(255, 255, 255, 0.14);
                        border-radius: 4px;
                        VerticalLayout {
                            padding: 6px;
                            Text { text: "LFO"; color: rgba(255, 255, 255, 0.4); font-family: "JetBrains Mono"; font-size: 11px; letter-spacing: 0.5px; }
                            Path {
                                vertical-stretch: 1;
                                stroke: root.accent;
                                stroke-width: 2px;
                                viewbox-width: 100; viewbox-height: 36;
                                MoveTo { x: 0; y: 18; }
                                CubicTo { x: 25; y: 18; control-1-x: 8; control-1-y: 3; control-2-x: 17; control-2-y: 3; }
                                CubicTo { x: 50; y: 18; control-1-x: 33; control-1-y: 33; control-2-x: 42; control-2-y: 33; }
                                CubicTo { x: 75; y: 18; control-1-x: 58; control-1-y: 3; control-2-x: 67; control-2-y: 3; }
                                CubicTo { x: 100; y: 18; control-1-x: 83; control-1-y: 33; control-2-x: 92; control-2-y: 33; }
                            }
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
    let ui = VoltageScreen::new().unwrap();
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

    let ui = VoltageScreen::new().unwrap();
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
