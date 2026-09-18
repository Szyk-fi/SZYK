//! Same scoping-only exercise as the other `slint_*_poc.rs` examples,
//! in the same hardware chrome/enclosure (`slint_common/device_frame.
//! slint`) -- Cascade: a 6-operator FM node graph, the "Stack"
//! algorithm (6->5->4->3->2->1, only Op1 audible, feedback on Op6) --
//! see cascade.rs's own `ALGORITHMS[0]` and `draw()` for the source
//! topology/layout this is ported from.
//!
//! Run interactively:
//!     cargo run --example slint_cascade_poc
//! Or headless: SLINT_RENDER_PNG=/tmp/out.png cargo run --example slint_cascade_poc
//!
//! Static data -- not wired to the real Cascade app's synth logic. See
//! slint_launcher_poc.rs's doc comment for the fuller "why Slint" /
//! "why this text size" context.

slint::slint! {
    import { DeviceFrame, BarSegment } from "slint_common/device_frame.slint";

    export component CascadeScreen inherits DeviceFrame {
        app-name: "CASCADE";
        breadcrumb: "ALGORITHM: STACK";
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

            // --- Left: the list. ---
            VerticalLayout {
                width: 260px;
                alignment: start;
                spacing: 1px;

                for row in [
                    { text: "\u{25b8}  Presets", value: "Init", chip: false },
                    { text: "\u{203a}  Algorithm", value: "Stack", chip: true },
                    { text: "\u{25b8}  Feedback", value: "0.30", chip: false },
                    { text: "\u{25b8}  Attack", value: "10ms", chip: false },
                    { text: "\u{25b8}  Decay", value: "300ms", chip: false },
                    { text: "\u{25b8}  Operator 1", value: "Ratio 1.00", chip: false },
                    { text: "\u{25b8}  Operator 6", value: "Ratio 2.00", chip: false },
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

            // --- Right: the operator node graph -- 6 nodes, 2 rows of
            // 3 (same layout cascade.rs's own `positions` array
            // computes), Op1 the only carrier (green), the rest
            // modulators-only (dim), Op6 marked FB.
            Rectangle {
                width: 340px;
                // Plain axis-aligned Rectangles, not `Path` -- a
                // `Path` with multiple `MoveTo`/`LineTo` subpaths (or
                // several single-segment `Path`s) silently misplaced
                // or dropped segments in this Slint version (a real,
                // reproducible renderer bug, not a coordinate
                // mistake: the diagonal Op4->Op3 connector's far end
                // consistently landed at Op6's position instead of
                // Op3's). Rectangles have no such ambiguity, so the
                // one connector that isn't a straight horizontal run
                // (Op4->Op3, jumping from row 2 back to row 1) is
                // routed as two segments -- up, then across -- instead
                // of a true diagonal.
                Rectangle { x: 49px; y: 49px; width: 102px; height: 2px; background: rgba(255, 255, 255, 0.18); } // Op2 -> Op1
                Rectangle { x: 149px; y: 49px; width: 102px; height: 2px; background: rgba(255, 255, 255, 0.18); } // Op3 -> Op2
                Rectangle { x: 49px; y: 50px; width: 2px; height: 70px; background: rgba(255, 255, 255, 0.18); } // Op4 -> Op3 (up to the top row -- the horizontal run to Op3's x is already covered by the two segments above)
                Rectangle { x: 49px; y: 119px; width: 102px; height: 2px; background: rgba(255, 255, 255, 0.18); } // Op5 -> Op4
                Rectangle { x: 149px; y: 119px; width: 102px; height: 2px; background: rgba(255, 255, 255, 0.18); } // Op6 -> Op5
                for node in [
                    { label: "Op1", cx: 50, cy: 50, carrier: true },
                    { label: "Op2", cx: 150, cy: 50, carrier: false },
                    { label: "Op3", cx: 250, cy: 50, carrier: false },
                    { label: "Op4", cx: 50, cy: 120, carrier: false },
                    { label: "Op5", cx: 150, cy: 120, carrier: false },
                    { label: "Op6 FB", cx: 250, cy: 120, carrier: false },
                ] : Rectangle {
                    x: node.cx * 1px - 22px;
                    y: node.cy * 1px - 22px;
                    width: 44px; height: 44px;
                    border-radius: 22px;
                    background: node.carrier ? root.accent.with-alpha(0.22) : rgba(255, 255, 255, 0.05);
                    border-width: 2px;
                    border-color: node.carrier ? root.accent : rgba(255, 255, 255, 0.25);
                    Text {
                        text: node.label;
                        color: node.carrier ? root.accent : rgba(255, 255, 255, 0.6);
                        font-family: "JetBrains Mono";
                        font-size: 11px;
                        font-weight: node.carrier ? 700 : 400;
                        horizontal-alignment: center;
                        vertical-alignment: center;
                        width: 100%;
                        height: 100%;
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
    let ui = CascadeScreen::new().unwrap();
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

    let ui = CascadeScreen::new().unwrap();
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
