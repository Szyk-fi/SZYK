//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Settings: the simplest remaining screen, a plain list
//! with no side panel, but at the tighter 19px row height the real
//! Settings app uses (see settings.rs) for its longer item count.
//!
//! Run interactively:
//!     cargo run --example slint_settings_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_settings_poc
//!
//! Static data lifted from settings.rs's own group/leaf names (Output
//! Device, Input Device, Preferences -> Knob Sensitivity/List Nav
//! Speed) -- not wired to the real app/device enumeration. See
//! slint_launcher_poc.rs's doc comment for the fuller "why Slint" /
//! "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component SettingsScreen inherits DeviceFrame {
        app-name: "SETTINGS";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3", active: false },
            { label: "F4  MIXER", active: false },
        ];

        // Explicit height, not just relying on `vertical-stretch` on
        // DeviceFrame's own wrapping Rectangle -- a short list (a
        // handful of rows, unlike every other screen's fuller
        // content) doesn't naturally fill the body area on its own,
        // which let the on-screen bottom bar float up right under the
        // last row instead of staying pinned to the screen's actual
        // bottom edge. Caught by rendering and looking, not a test.
        VerticalLayout {
            height: 100%;
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 2px;
            spacing: 1px;

            for row in [
                { text: "\u{25be}  Output Device", value: "Scarlett 18i20 USB", chip: false, indent: 0 },
                { text: "Scarlett 18i20 USB", value: "active", chip: true, indent: 1 },
                { text: "MacBook Pro Speakers", value: "press to select", chip: false, indent: 1 },
                { text: "\u{25b8}  Input Device", value: "none (not wired to any app yet)", chip: false, indent: 0 },
                { text: "\u{25b8}  Preferences", value: "sens 0.10, nav 6", chip: false, indent: 0 },
            ] : Rectangle {
                height: row.chip ? 26px : 22px;
                border-radius: 4px;
                background: row.chip ? root.accent.with-alpha(0.13) : transparent;
                HorizontalLayout {
                    padding-left: row.indent * 18px + 6px;
                    padding-right: 6px;
                    Text {
                        text: row.text;
                        color: row.chip ? root.accent : rgba(255, 255, 255, 0.78);
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
    }
}

fn main() {
    if let Ok(path) = std::env::var("SLINT_RENDER_PNG") {
        render_to_png(&path);
        return;
    }
    let ui = SettingsScreen::new().unwrap();
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

    let ui = SettingsScreen::new().unwrap();
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
