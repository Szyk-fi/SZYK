//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome (`slint_common/device_frame.slint`) --
//! Sequencer, chosen as the most visually distinct remaining screen:
//! a 4x4 step grid instead of a list-only or list+scope layout.
//!
//! Run interactively:
//!     cargo run --example slint_sequencer_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_sequencer_poc
//!
//! Static data (a four-on-the-floor kick pattern, playhead frozen on
//! step 5) -- not wired to the real Sequencer's audio/transport. See
//! slint_launcher_poc.rs's doc comment for the fuller "why Slint" /
//! "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component SequencerScreen inherits DeviceFrame {
        app-name: "SEQUENCER";
        breadcrumb: "TRACK 1  \u{203a}  KICK";
        bar: [
            { label: "F1  HOME", active: false },
            { label: "F2  STOP", active: true },
            { label: "F3  MIDI: OFF", active: false },
            { label: "F4  MIXER", active: false },
        ];

        // Step 4 (0-indexed) is the playhead; steps 0/4/8/12 are the
        // programmed four-on-the-floor kick pattern.
        property <[int]> active-steps: [0, 4, 8, 12];
        property <int> playhead: 4;
        function is-active(i: int) -> bool {
            return active-steps[0] == i || active-steps[1] == i || active-steps[2] == i || active-steps[3] == i;
        }
        // A step only actually sounds -- and so only actually turns
        // red -- if it's both programmed on AND the playhead is
        // sitting on it right now; the playhead passing over an
        // unprogrammed step doesn't light anything, same as the
        // physical pads below.
        function is-playing(i: int) -> bool {
            return root.is-active(i) && root.playhead == i;
        }
        // Same green-for-activated/red-for-playing semantics as the
        // real device's physical pad LEDs (see led_output.rs's
        // `PadColor` and sequencer.rs's `grid_led_overlay`) -- the
        // on-screen step grid below and the device's actual pad
        // matrix (in DeviceFrame) show the same thing at once.
        function pad-color(i: int) -> color {
            if (!root.is-active(i)) {
                return #232323;
            }
            return i == root.playhead ? #ff4d4d : #00e839;
        }
        pad-colors: [
            pad-color(0), pad-color(1), pad-color(2), pad-color(3),
            pad-color(4), pad-color(5), pad-color(6), pad-color(7),
            pad-color(8), pad-color(9), pad-color(10), pad-color(11),
            pad-color(12), pad-color(13), pad-color(14), pad-color(15),
        ];

        HorizontalLayout {
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 4px;
            spacing: 24px;

            // --- Left: Global + 4 tracks, Track 1 selected. ---
            VerticalLayout {
                width: 230px;
                alignment: start;
                spacing: 1px;

                for row in [
                    { text: "\u{25b8}  GLOBAL", value: "120 BPM", chip: false },
                    { text: "\u{203a}  Track 1: Kick", value: "", chip: true },
                    { text: "   Track 2: Snare", value: "", chip: false },
                    { text: "   Track 3: Hat", value: "", chip: false },
                    { text: "   Track 4: Clap", value: "", chip: false },
                ] : Rectangle {
                    height: row.chip ? 30px : 26px;
                    border-radius: 4px;
                    background: row.chip ? root.accent.with-alpha(0.13) : transparent;
                    Text {
                        x: 8px;
                        text: row.value == "" ? row.text : row.text + ": " + row.value;
                        color: row.chip ? root.accent : rgba(255, 255, 255, 0.72);
                        font-family: "JetBrains Mono";
                        font-size: 15px;
                        font-weight: row.chip ? 700 : 400;
                        vertical-alignment: center;
                        height: 100%;
                    }
                }
            }

            // --- Right: 4x4 step grid, row-major, matching the
            // physical pad layout 1:1 (same as the real app's own
            // grid -- see sequencer.rs's own comment on why no
            // low/high flip is needed here, unlike Plaits' pitch
            // grid). ---
            GridLayout {
                spacing: 8px;
                padding-top: 2px;
                Row {
                    for col in [0, 1, 2, 3] : Rectangle {
                        property <int> i: col;
                        width: 62px; height: 62px;
                        border-radius: 5px;
                        border-width: root.is-playing(i) ? 2px : 1px;
                        border-color: root.is-playing(i) ? #ff4d4d : rgba(255, 255, 255, 0.12);
                        background: root.is-playing(i) ? rgba(255, 77, 77, 0.35) : root.is-active(i) ? root.accent.with-alpha(0.28) : rgba(255, 255, 255, 0.03);
                        Text {
                            x: 8px; y: 8px;
                            text: i + 1;
                            color: root.is-playing(i) ? #ff4d4d : root.is-active(i) ? root.accent : rgba(255, 255, 255, 0.35);
                            font-family: "JetBrains Mono";
                            font-size: 13px;
                            font-weight: root.is-active(i) ? 700 : 400;
                        }
                    }
                }
                Row {
                    for col in [4, 5, 6, 7] : Rectangle {
                        property <int> i: col;
                        width: 62px; height: 62px;
                        border-radius: 5px;
                        border-width: root.is-playing(i) ? 2px : 1px;
                        border-color: root.is-playing(i) ? #ff4d4d : rgba(255, 255, 255, 0.12);
                        background: root.is-playing(i) ? rgba(255, 77, 77, 0.35) : root.is-active(i) ? root.accent.with-alpha(0.28) : rgba(255, 255, 255, 0.03);
                        Text {
                            x: 8px; y: 8px;
                            text: i + 1;
                            color: root.is-playing(i) ? #ff4d4d : root.is-active(i) ? root.accent : rgba(255, 255, 255, 0.35);
                            font-family: "JetBrains Mono";
                            font-size: 13px;
                            font-weight: root.is-active(i) ? 700 : 400;
                        }
                    }
                }
                Row {
                    for col in [8, 9, 10, 11] : Rectangle {
                        property <int> i: col;
                        width: 62px; height: 62px;
                        border-radius: 5px;
                        border-width: root.is-playing(i) ? 2px : 1px;
                        border-color: root.is-playing(i) ? #ff4d4d : rgba(255, 255, 255, 0.12);
                        background: root.is-playing(i) ? rgba(255, 77, 77, 0.35) : root.is-active(i) ? root.accent.with-alpha(0.28) : rgba(255, 255, 255, 0.03);
                        Text {
                            x: 8px; y: 8px;
                            text: i + 1;
                            color: root.is-playing(i) ? #ff4d4d : root.is-active(i) ? root.accent : rgba(255, 255, 255, 0.35);
                            font-family: "JetBrains Mono";
                            font-size: 13px;
                            font-weight: root.is-active(i) ? 700 : 400;
                        }
                    }
                }
                Row {
                    for col in [12, 13, 14, 15] : Rectangle {
                        property <int> i: col;
                        width: 62px; height: 62px;
                        border-radius: 5px;
                        border-width: root.is-playing(i) ? 2px : 1px;
                        border-color: root.is-playing(i) ? #ff4d4d : rgba(255, 255, 255, 0.12);
                        background: root.is-playing(i) ? rgba(255, 77, 77, 0.35) : root.is-active(i) ? root.accent.with-alpha(0.28) : rgba(255, 255, 255, 0.03);
                        Text {
                            x: 8px; y: 8px;
                            text: i + 1;
                            color: root.is-playing(i) ? #ff4d4d : root.is-active(i) ? root.accent : rgba(255, 255, 255, 0.35);
                            font-family: "JetBrains Mono";
                            font-size: 13px;
                            font-weight: root.is-active(i) ? 700 : 400;
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
    let ui = SequencerScreen::new().unwrap();
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

    let ui = SequencerScreen::new().unwrap();
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
