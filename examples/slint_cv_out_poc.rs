//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- CV Out, the newest real app this session (see
//! apps/cv_out.rs): 32 CV channels, same fader-bank shape Mixer's
//! screen already uses since both are "a bank of levels" UIs.
//!
//! Run interactively:
//!     cargo run --example slint_cv_out_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_cv_out_poc
//!
//! Static data -- not wired to the real CvOutApp's ModBus-driven
//! values. See slint_launcher_poc.rs's doc comment for the fuller
//! "why Slint" / "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component CvOutScreen inherits DeviceFrame {
        app-name: "CV OUT";
        breadcrumb: "GLOBAL  \u{203a}  CV OUTPUTS";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2", active: false },
            { label: "F3  MIDI: OFF", active: false },
            { label: "F4  MIXER", active: false },
        ];

        // A plain two-column list, same shape every other app's menu
        // already uses -- not a fader/level-meter bank. CV isn't a
        // loudness control the way a mixer channel is; a bar implying
        // "how loud" is the wrong mental model for "what voltage is
        // this outputting", so this reads it the same way Voltage
        // reads a filter cutoff or an envelope stage: a value in a
        // list. Each row is one channel's actual combined value (base
        // + external mod, see cv_out.rs's `combined()`), with the raw
        // CC number/value alongside it since that's literally what's
        // going out the wire and worth seeing precisely, not just a
        // percentage.
        VerticalLayout {
            height: 100%;
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 2px;
            spacing: 1px;

            Rectangle {
                height: 24px;
                Text {
                    x: 8px;
                    text: "\u{25b8}  Global: Ch 1";
                    color: rgba(255, 255, 255, 0.6);
                    font-family: "JetBrains Mono";
                    font-size: 14px;
                    vertical-alignment: center;
                    height: 100%;
                }
            }
            Rectangle {
                height: 26px;
                Text {
                    x: 8px;
                    text: "\u{25be}  CV Outputs";
                    color: rgba(255, 255, 255, 0.6);
                    font-family: "JetBrains Mono";
                    font-size: 14px;
                    vertical-alignment: center;
                    height: 100%;
                }
            }
            for ch[i] in [
                { value: 50, cc: 0, raw: 64, selected: true },
                { value: 82, cc: 1, raw: 104, selected: false },
                { value: 20, cc: 2, raw: 25, selected: false },
                { value: 65, cc: 3, raw: 83, selected: false },
                { value: 0, cc: 4, raw: 0, selected: false },
                { value: 40, cc: 5, raw: 51, selected: false },
                { value: 100, cc: 6, raw: 127, selected: false },
                { value: 12, cc: 7, raw: 15, selected: false },
            ] : Rectangle {
                height: 24px;
                border-radius: 4px;
                background: ch.selected ? root.accent.with-alpha(0.13) : transparent;
                HorizontalLayout {
                    padding-left: 24px;
                    padding-right: 6px;
                    Text {
                        text: (ch.selected ? "\u{203a} " : "") + "CV " + (i + 1);
                        color: ch.selected ? root.accent : rgba(255, 255, 255, 0.72);
                        font-family: "JetBrains Mono";
                        font-size: 14px;
                        font-weight: ch.selected ? 700 : 400;
                        vertical-alignment: center;
                    }
                    Rectangle { horizontal-stretch: 1; }
                    Text {
                        text: ch.value + "% (CC" + ch.cc + "=" + ch.raw + ")";
                        color: ch.selected ? root.accent : rgba(255, 255, 255, 0.4);
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
    let ui = CvOutScreen::new().unwrap();
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

    let ui = CvOutScreen::new().unwrap();
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
