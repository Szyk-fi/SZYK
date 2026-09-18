//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Nebula: a circular particle arena with 3 gravity wells
//! (see nebula.rs's own `NUM_WELLS`/`well_position`), one lit to show
//! it just fired.
//!
//! Run interactively:
//!     cargo run --example slint_nebula_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_nebula_poc
//!
//! Static particle positions -- not wired to the real Nebula app's
//! physics simulation. See slint_launcher_poc.rs's doc comment for
//! the fuller "why Slint" / "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component NebulaScreen inherits DeviceFrame {
        app-name: "NEBULA";
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
                    { text: "\u{203a}  Particles", value: "24", chip: true },
                    { text: "\u{25b8}  Gravity", value: "0.60", chip: false },
                    { text: "\u{25b8}  Wells", value: "2/3 active", chip: false },
                    { text: "\u{25b8}  Capture", value: "0.40", chip: false },
                    { text: "\u{25b8}  Fire Pitch", value: "+0", chip: false },
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

            // --- Right: the circular particle arena. ---
            Rectangle {
                width: 280px;
                Rectangle {
                    x: 40px; y: 10px;
                    width: 210px; height: 210px;
                    border-radius: 105px;
                    border-width: 1px;
                    border-color: rgba(255, 255, 255, 0.12);
                }
                // 3 wells at 0/120/240 degrees -- top one "just fired".
                Rectangle {
                    x: 133px; y: 3px;
                    width: 24px; height: 24px;
                    border-radius: 12px;
                    border-width: 3px;
                    border-color: #00e879;
                }
                Rectangle {
                    x: 213px; y: 155px;
                    width: 20px; height: 20px;
                    border-radius: 10px;
                    border-width: 2px;
                    border-color: #0084ba;
                }
                Rectangle {
                    x: 55px; y: 155px;
                    width: 20px; height: 20px;
                    border-radius: 10px;
                    border-width: 2px;
                    border-color: rgba(255, 255, 255, 0.15);
                }
                // A scattering of particles inside the arena.
                for p in [
                    { x: 145, y: 70 }, { x: 100, y: 90 }, { x: 180, y: 100 },
                    { x: 120, y: 140 }, { x: 160, y: 150 }, { x: 145, y: 180 },
                    { x: 90, y: 130 }, { x: 190, y: 60 }, { x: 110, y: 60 },
                    { x: 170, y: 170 }, { x: 145, y: 115 }, { x: 75, y: 100 },
                ] : Rectangle {
                    x: p.x * 1px; y: p.y * 1px;
                    width: 6px; height: 6px;
                    border-radius: 3px;
                    background: root.accent.with-alpha(0.7);
                }
                Text {
                    x: 20px; y: 235px;
                    text: "24 particles, 2 wells active";
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
    let ui = NebulaScreen::new().unwrap();
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

    let ui = NebulaScreen::new().unwrap();
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
