//! Scoping-only proof of concept: the launcher screen, framed in the
//! actual "Portamax UI Mockup" design artifact's hardware chrome (see
//! `slint_common/device_frame.slint`) instead of a bare black window --
//! Space Grotesk + JetBrains Mono, the #E7E2D6 bezel, the #5CF07A
//! accent, physical knob/F-button strip.
//!
//! Deliberately kept as a standalone example, not wired into the real
//! `src/os.rs` -- this exists purely to look at and decide from, with
//! zero risk to the working simulator. Run it with:
//!
//!     cargo run --example slint_launcher_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_launcher_poc
//!
//! Same 15 installed apps (alphabetical, matching `apps/`). Up/Down
//! move the selection; Enter is a no-op stand-in for "select" --
//! there's nothing to launch here, this only renders.
//!
//! Text throughout is sized well past what the desktop mockup used --
//! this is a real 3-inch panel on the actual hardware (roughly 245
//! PPI at 640x360, ~2.5x a typical desktop's density), so legibility
//! at arm's length matters more than matching the mockup's
//! proportions pixel-for-pixel. See `device_frame.slint`'s own note
//! on the bottom bar for the math.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component Launcher inherits DeviceFrame {
        app-name: "";
        breadcrumb: "LAUNCHER";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3", active: false },
            { label: "F4  MIXER", active: false },
        ];

        in-out property <[string]> apps: [
            "Bloom", "Cascade", "Clouds", "CV Out", "Madness", "Mixer", "Nebula",
            "Pam's Workout", "Plaits", "Prism", "Sequencer", "Settings",
            "Singularity", "Tape", "Voltage",
        ];
        in-out property <int> selected: 0;

        forward-focus: key-handler;
        key-handler := FocusScope {
            key-pressed(event) => {
                if (event.text == Key.DownArrow) {
                    root.selected = Math.min(root.selected + 1, root.apps.length - 1);
                    accept
                } else if (event.text == Key.UpArrow) {
                    root.selected = Math.max(root.selected - 1, 0);
                    accept
                } else {
                    reject
                }
            }
        }

        // 15 apps at a legible 15px row height don't all fit in the
        // ~280px this body area actually has -- rather than shrink
        // the text back down to make them all fit at once (the exact
        // mistake the real embedded_graphics launcher had to get
        // fixed away from earlier this session), this scrolls, always
        // keeping the selection roughly centered in view.
        property <length> row-h: 28px;
        property <int> visible-rows: 9;
        property <int> scroll: max(0, min(root.selected - root.visible-rows / 2, apps.length - root.visible-rows));

        Rectangle {
            vertical-stretch: 1;
            clip: true;

            VerticalLayout {
                y: -root.scroll * root.row-h;
                padding-left: 18px;
                padding-right: 18px;
                padding-top: 2px;
                spacing: 0px;
                for name[i] in apps : Rectangle {
                    height: root.row-h;
                    border-radius: 4px;
                    background: i == root.selected ? root.accent.with-alpha(0.13) : transparent;

                    Text {
                        x: 10px;
                        text: (i == root.selected ? "\u{203a} " : "   ") + name;
                        color: i == root.selected ? root.accent : rgba(255, 255, 255, 0.6);
                        font-family: "JetBrains Mono";
                        font-size: 15px;
                        font-weight: i == root.selected ? 700 : 400;
                        vertical-alignment: center;
                        height: 100%;
                        animate color { duration: 120ms; easing: ease-out; }
                    }
                    animate background { duration: 120ms; easing: ease-out; }
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
    let ui = Launcher::new().unwrap();
    ui.run().unwrap();
}

/// Renders one frame through Slint's own software (CPU) renderer --
/// the same renderer Slint's MCU/no_std backends use, so this is also
/// an honest preview of what the *embedded* target would actually
/// draw, not just what the desktop GPU backend happens to produce --
/// into an offscreen buffer with no OS window at all, then writes it
/// to a PNG.
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
    // Must happen before the first component is created -- Slint picks
    // a real backend (winit) the moment one exists otherwise.
    slint::platform::set_platform(Box::new(HeadlessPlatform)).expect("platform already set");
    let window = WINDOW.with(Rc::clone);
    window.set_size(PhysicalSize::new(1240, 560));

    let ui = Launcher::new().unwrap();
    ui.set_selected(3); // CV Out -- exercises the selected-row chip/animation-endpoint
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
