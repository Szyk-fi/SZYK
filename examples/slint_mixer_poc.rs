//! Same scoping-only exercise as `slint_launcher_poc.rs`, in the same
//! hardware chrome (`slint_common/device_frame.slint`), for a richer
//! in-app screen instead of the plain launcher list -- Mixer, chosen
//! because it's the screen with both a list AND a graphic panel (the
//! fader bank), so it's a fairer stress-test against what
//! `mixer_new.png` (the real app) actually looks like today.
//!
//! Run interactively:
//!     cargo run --example slint_mixer_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_mixer_poc
//!
//! Not wired to the real Mixer's logic (no live audio levels, no
//! knob input) -- this is a look at the rendering, not a working
//! screen. See slint_launcher_poc.rs's doc comment for the fuller
//! "why Slint" / "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component MixerScreen inherits DeviceFrame {
        app-name: "MIXER";
        breadcrumb: "MASTER  \u{203a}  CHANNELS";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3  MIDI: OFF", active: false },
            { label: "F4  MIXER", active: true },
        ];

        HorizontalLayout {
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 4px;
            spacing: 22px;

            // --- Left: the list, Master selected. ---
            VerticalLayout {
                width: 260px;
                alignment: start;
                spacing: 2px;

                Rectangle {
                    height: 30px;
                    border-radius: 4px;
                    background: root.accent.with-alpha(0.13);
                    Text {
                        x: 8px;
                        text: "\u{203a} \u{203a} Master: 100%";
                        color: root.accent;
                        font-family: "JetBrains Mono";
                        font-size: 15px;
                        font-weight: 700;
                        vertical-alignment: center;
                        height: 100%;
                    }
                }
                Rectangle {
                    height: 30px;
                    Text {
                        x: 8px;
                        text: "\u{203a} Channels: 11 apps";
                        color: rgba(255, 255, 255, 0.55);
                        font-family: "JetBrains Mono";
                        font-size: 15px;
                        vertical-alignment: center;
                        height: 100%;
                    }
                }
            }

            // --- Right: the fader bank. ---
            HorizontalLayout {
                spacing: 14px;
                alignment: start;
                for ch in [
                    { label: "Master", value: 0.667, lit: true },
                    { label: "Bloom", value: 0.667, lit: false },
                    { label: "Cascade", value: 0.667, lit: false },
                    { label: "Clouds", value: 0.667, lit: false },
                ] : VerticalLayout {
                    width: 60px;
                    spacing: 6px;
                    HorizontalLayout {
                        height: 190px;
                        alignment: center;
                        Rectangle {
                            width: 30px;
                            border-radius: 4px;
                            border-width: 1px;
                            border-color: rgba(255, 255, 255, 0.14);
                            background: rgba(255, 255, 255, 0.03);

                            Rectangle {
                                y: parent.height - parent.height * ch.value;
                                width: parent.width;
                                height: parent.height * ch.value;
                                border-radius: 4px;
                                background: ch.lit ? root.accent : root.accent.with-alpha(0.55);
                                drop-shadow-color: ch.lit ? root.accent.with-alpha(0.55) : transparent;
                                drop-shadow-blur: 8px;
                            }
                            Rectangle {
                                y: parent.height - parent.height * 0.667;
                                width: parent.width;
                                height: 2px;
                                background: root.accent;
                            }
                        }
                    }
                    Text {
                        text: ch.label;
                        color: rgba(255, 255, 255, 0.45);
                        font-family: "JetBrains Mono";
                        font-size: 12px;
                        horizontal-alignment: center;
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
    let ui = MixerScreen::new().unwrap();
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

    let ui = MixerScreen::new().unwrap();
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
