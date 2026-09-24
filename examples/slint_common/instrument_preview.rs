//! Offline visual verification with real module state and processors.
//! Run the live example with --render-instruments OUTPUT_DIRECTORY.
use super::*;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::{PhysicalSize, Rgb8Pixel, SharedPixelBuffer};

pub fn render(directory: &str) {
    struct PreviewPlatform(Rc<MinimalSoftwareWindow>, Rc<std::cell::Cell<std::time::Duration>>);
    impl Platform for PreviewPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(self.0.clone())
        }
        fn duration_since_start(&self) -> std::time::Duration { self.1.get() }
    }
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(std::cell::Cell::new(std::time::Duration::ZERO));
    slint::platform::set_platform(Box::new(PreviewPlatform(window.clone(), clock.clone()))).unwrap();
    window.set_size(PhysicalSize::new(1240, 560));
    std::fs::create_dir_all(directory).unwrap();
    let ui = LiveHomeScreen::new().unwrap();
    ui.set_splash_active(false);
    ui.set_preview_render(true);
    ui.set_on_home(false);
    ui.show().unwrap();
    let (registry, preview_modbus) = isolated_registry();
    let mut manifests = manifest::discover(std::path::Path::new(APPS_DIR));
    for (id, name) in [("analyzer", "Analyzer"), ("synth", "Synth")] {
        if !manifests.iter().any(|m| m.id == id) {
            manifests.push(manifest::AppManifest { id: id.into(), name: name.into() });
        }
    }
    for manifest in &manifests {
        let (isolated, bus) = isolated_registry();
        let mut app: Box<dyn App> = if manifest.id == "cv_out" {
            Box::new(cv_out::CvOutApp::with_output(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), bus, led_output::LedOutput::none()))
        } else { isolated.build(std::slice::from_ref(manifest)).pop().unwrap().1 };
        app.tick(&Input::default());
        let _ = app.slint_windowed_rows(10);
        let _ = app.slint_extra();
        if let Some(mut processor) = app.audio_processor() {
            let mut samples = [0.0; 1024];
            for _ in 0..4 { processor.process(&mut samples, 2, 48000.0); }
            assert!(samples.iter().all(|sample| sample.is_finite()), "{}: invalid isolated audio", manifest.id);
        }
        if let Some(running) = app.running() {
            assert!(app.transport_action().is_some());
            app.toggle_running();
            assert_ne!(app.running(), Some(running), "{} transport did not change", manifest.id);
        }
        println!("{}: isolated app with absent peers passed", manifest.name);
    }
    let mut preview_apps = Vec::new();
    for manifest in &manifests {
        println!("Preparing {}", manifest.name);
        let mut built = if manifest.id == "cv_out" {
            vec![(manifest.name.clone(), Box::new(cv_out::CvOutApp::with_output(
                Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)),
                Arc::clone(&preview_modbus), led_output::LedOutput::none(),
            )) as Box<dyn App>)]
        } else { registry.build(std::slice::from_ref(manifest)) };
        preview_apps.push(built.pop().expect("registered preview app"));
    }
    for (name, mut app) in preview_apps {
        let name = name.as_str();
        // Device discovery is not part of a headless UI render. The real Settings
        // constructor and theme state remain in use; its device lists stay empty.
        if name != "Settings" { app.on_enter(); }
        if name == "Bloom" {
            assert_eq!(app.running(), Some(false));
            app.toggle_running();
            assert_eq!(app.running(), Some(true));
        }
        let mut processor = app.audio_processor();
        let mut buffer = [0.0f32; 1024];
        for _ in 0..(if name == "Bloom" { 413 } else { 100 }) {
            let mut input = Input::default();
            input.grid[0] = true;
            app.tick(&input);
            if let Some(processor) = processor.as_mut() {
                buffer.fill(0.0);
                processor.process(&mut buffer, 2, 48000.0);
            }
        }
        if name == "Bloom" {
            let app::SlintExtra::Bloom(before) = app.slint_extra() else { unreachable!() };
            processor.as_mut().unwrap().process(&mut buffer, 2, 48000.0);
            let app::SlintExtra::Bloom(after) = app.slint_extra() else { unreachable!() };
            assert_ne!(before.inner, after.inner, "Bloom orbits must follow live DSP");
            app.toggle_running();
            assert_eq!(app.running(), Some(false));
            app.toggle_running();
        }
        if ["Plaits", "Pam's Workout", "Beads", "Black Hole", "Bloom"].contains(&name) {
        // Exercise actual short pointer taps all the way into the module menu,
        // using the normal three-tick encoder setting that exposed the regression.
        let before = app.slint_selected();
        for (y, expected) in [(249.0, before + 1), (167.0, before)] {
            let position = slint::LogicalPosition::new(113.0, y);
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed {
                position, button: slint::platform::PointerEventButton::Left,
            });
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased {
                position, button: slint::platform::PointerEventButton::Left,
            });
            let steps = ui.get_navigation_delta();
            ui.set_navigation_delta(0);
            app.tick(&Input { navigation_steps: steps, ..Default::default() });
            assert_eq!(app.slint_selected(), expected, "{name}: one short D-pad tap must move one row");
            app.tick(&Input::default());
            assert_eq!(app.slint_selected(), expected, "{name}: no extra row after release");
        }
        app.tick(&Input { knob1: 1, ..Default::default() });
        assert_eq!(app.slint_selected(), before, "{name}: encoder sensitivity must be preserved");
        for _ in 0..2 { app.tick(&Input { knob1: 1, ..Default::default() }); }
        assert_eq!(app.slint_selected(), before + 1, "{name}: three encoder ticks move one row");
        app.tick(&Input { navigation_steps: -1, ..Default::default() });
        println!("{name}: short D-pad taps move one row; MIDI encoder sensitivity preserved");
        }
        if name != "Synth" && app.slint_rows().len() > 1 {
            let before = app.slint_selected();
            for (y, delta) in [(249.0, 1), (167.0, -1)] {
                let position = slint::LogicalPosition::new(113.0, y);
                ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button: slint::platform::PointerEventButton::Left });
                ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button: slint::platform::PointerEventButton::Left });
                assert_eq!(ui.get_navigation_delta(), delta);
                app.tick(&Input { navigation_steps: ui.get_navigation_delta(), ..Default::default() });
                ui.set_navigation_delta(0);
                assert_eq!(app.slint_selected(), if delta == 1 { before + 1 } else { before }, "{name}: short tap navigation");
            }
            println!("{name}: pointer tap navigation passed");
        }
        let (rows, selected, above, below) = app.slint_windowed_rows(10);
        ui.set_row_names(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.0.clone().into()).collect::<Vec<slint::SharedString>>())).into());
        ui.set_row_values(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.1.clone().into()).collect::<Vec<slint::SharedString>>())).into());
        ui.set_row_is_group(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.2).collect::<Vec<bool>>())).into());
        ui.set_selected_row(selected as i32);
        ui.set_more_above(above);
        ui.set_more_below(below);
        ui.set_active_app_name(name.into());
        ui.set_grid_mode_label(app.grid_mode_label().unwrap_or("").into());
        ui.set_transport_action(app.transport_action().unwrap_or("").into());
        ui.set_pad_lock_available(app.supports_pad_lock());
        let levels = app.slint_levels(10);
        ui.set_row_levels(Rc::new(slint::VecModel::from(levels.into_iter().map(|l| l.unwrap_or(-1.0)).collect::<Vec<_>>())).into());
        ui.set_transport_label(match app.running() { Some(true) => "RUNNING", Some(false) => "STOPPED", None => "" }.into());
        let (bg, ink, accent, _) = app_palette(name).unwrap_or((slint::Color::from_rgb_u8(20,25,31), slint::Color::from_rgb_u8(231,237,244), slint::Color::from_rgb_u8(165,188,233), slint::Color::from_rgb_u8(137,148,170)));
        ui.set_live_bg(bg); ui.set_live_ink(ink); ui.set_live_accent(accent);
        apply_instrument_visual(&ui, app.slint_extra());
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(1240, 560);
        window.draw_if_needed(|renderer| { renderer.render(pixels.make_mut_slice(), 1240); });
        // Save only the actual 640x360 display, omitting simulator controls.
        let mut screen = Vec::with_capacity(640 * 360 * 3);
        for y in 92..452 {
            let start = (y * 1240 + 208) * 3;
            screen.extend_from_slice(&pixels.as_bytes()[start..start + 640 * 3]);
        }
        let slug = name.to_lowercase().replace(' ', "-").replace('\'', "");
        let path = std::path::Path::new(directory).join(format!("{slug}.png"));
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&path).unwrap()), 640, 360);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&screen).unwrap();
        println!("Rendered {} from real module state", path.display());
        let enclosure = std::path::Path::new(directory).join(format!("{slug}-console.png"));
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&enclosure).unwrap()), 1240, 560);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(pixels.as_bytes()).unwrap();
        if name == "Plaits" {
            // Show the actual pressed-state styling, without synthetic audio data.
            let position = slint::LogicalPosition::new(913.0, 149.0);
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed {
                position, button: slint::platform::PointerEventButton::Left,
            });
            window.request_redraw();
            window.draw_if_needed(|renderer| { renderer.render(pixels.make_mut_slice(), 1240); });
            let pressed = std::path::Path::new(directory).join("plaits-pressed-console.png");
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(pressed).unwrap()), 1240, 560);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(pixels.as_bytes()).unwrap();
            ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased {
                position, button: slint::platform::PointerEventButton::Left,
            });
        }
        if name == "Bloom" {
            // Exercise the actual group expansion and value editing path, then
            // render the dense menu state with the same real orbital payload.
            app.tick(&Input { navigation_steps: 1, ..Default::default() });
            app.tick(&Input { knob1_press: true, ..Default::default() });
            let (rows, selected, above, below) = app.slint_windowed_rows(10);
            assert!(rows.iter().any(|r| !r.2), "Bloom group must expand into editable parameters");
            ui.set_row_names(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.0.clone().into()).collect::<Vec<slint::SharedString>>())).into());
            ui.set_row_values(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.1.clone().into()).collect::<Vec<slint::SharedString>>())).into());
            ui.set_row_is_group(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.2).collect::<Vec<bool>>())).into());
            ui.set_selected_row(selected as i32);
            ui.set_more_above(above); ui.set_more_below(below);
            let transport = Rc::new(std::cell::Cell::new(-1));
            let received = transport.clone();
            ui.on_f_clicked(move |index| received.set(index));
            let position = slint::LogicalPosition::new(793.0, 152.0);
            for pressed in [true, false] {
                ui.window().dispatch_event(if pressed {
                    slint::platform::WindowEvent::PointerPressed { position, button: slint::platform::PointerEventButton::Left }
                } else {
                    slint::platform::WindowEvent::PointerReleased { position, button: slint::platform::PointerEventButton::Left }
                });
            }
            assert_eq!(transport.get(), 2, "Bloom status button must route to F3 transport");
            window.request_redraw();
            window.draw_if_needed(|renderer| { renderer.render(pixels.make_mut_slice(), 1240); });
            let path = std::path::Path::new(directory).join("bloom-expanded-console.png");
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).unwrap()), 1240, 560);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(pixels.as_bytes()).unwrap();
            println!("Bloom: live orbital motion, transport, and parameter expansion passed");
        }
        if name != "Bloom" {
            let all_rows = app.slint_rows();
            let group = if name == "Settings" { Some(3) } else { all_rows.iter().position(|r| r.2) };
            if let Some(group) = group {
                app.tick(&Input { navigation_steps: group as i32 - app.slint_selected() as i32, ..Default::default() });
                app.tick(&Input { knob1_press: true, ..Default::default() });
                let (rows, selected, above, below) = app.slint_windowed_rows(10);
                ui.set_row_names(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.0.clone().into()).collect::<Vec<slint::SharedString>>())).into());
                ui.set_row_values(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.1.clone().into()).collect::<Vec<slint::SharedString>>())).into());
                ui.set_row_is_group(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.2).collect::<Vec<bool>>())).into());
                ui.set_selected_row(selected as i32); ui.set_more_above(above); ui.set_more_below(below);
                ui.set_row_levels(Rc::new(slint::VecModel::from(app.slint_levels(10).into_iter().map(|l| l.unwrap_or(-1.0)).collect::<Vec<_>>())).into());
                apply_instrument_visual(&ui, app.slint_extra());
                save_extra_frame(&window, directory, &format!("{slug}-expanded"));
                println!("{name}: expanded menu rendered");
            }
        }
        app.on_exit();
    }
    verify_console_controls(&ui, &clock);
    ui.set_on_home(true);
    ui.set_pad_lock_available(false);
    ui.set_transport_action("".into());
    ui.set_home_names(Rc::new(slint::VecModel::from(manifests.iter().take(5).map(|m| m.name.clone().into()).collect::<Vec<slint::SharedString>>())).into());
    ui.set_home_selected(0); ui.set_home_more_above(false); ui.set_home_more_below(true);
    save_extra_frame(&window, directory, "home");
    ui.set_audio_connected(false);
    ui.set_preview_render(false);
    save_extra_frame(&window, directory, "audio-unavailable");
    ui.set_home_names(Rc::new(slint::VecModel::from(["Beads", "Black Hole", "Bloom", "Cascade", "Clouds", "Madness"].map(slint::SharedString::from).to_vec())).into());
    ui.set_splash_active(true);
    ui.set_splash_is_szyk(true);
    save_extra_frame(&window, directory, "startup-szyk");
    ui.set_splash_is_szyk(false);
    ui.set_splash_image(logo_to_slint_image(&startup_logo::MX1));
    save_extra_frame(&window, directory, "startup-mx1");
}

fn save_extra_frame(window: &MinimalSoftwareWindow, directory: &str, slug: &str) {
    window.request_redraw();
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(1240, 560);
    let started = std::time::Instant::now();
    window.draw_if_needed(|renderer| { renderer.render(pixels.make_mut_slice(), 1240); });
    println!("{slug}: full software frame {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
    for full in [false, true] {
        let (width, height, bytes) = if full { (1240, 560, pixels.as_bytes().to_vec()) } else {
            let mut crop = Vec::with_capacity(640 * 360 * 3);
            for y in 92..452 {
                let start = (y * 1240 + 208) * 3;
                crop.extend_from_slice(&pixels.as_bytes()[start..start + 640 * 3]);
            }
            (640, 360, crop)
        };
        let suffix = if full { "-console" } else { "" };
        let path = std::path::Path::new(directory).join(format!("{slug}{suffix}.png"));
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).unwrap()), width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&bytes).unwrap();
    }
}

// Headless input integration checks against the actual Slint hit targets.
fn verify_console_controls(ui: &LiveHomeScreen, clock: &std::cell::Cell<std::time::Duration>) {
    use slint::{LogicalPosition, platform::{PointerEventButton, WindowEvent}};
    let press = |x, y| ui.window().dispatch_event(WindowEvent::PointerPressed {
        position: LogicalPosition::new(x, y), button: PointerEventButton::Left,
    });
    let release = |x, y| ui.window().dispatch_event(WindowEvent::PointerReleased {
        position: LogicalPosition::new(x, y), button: PointerEventButton::Left,
    });
    let click = |x, y| { press(x, y); release(x, y); };
    click(113.0, 167.0);
    assert_eq!(ui.get_navigation_delta(), -1, "D-pad up");
    click(113.0, 249.0);
    assert_eq!(ui.get_navigation_delta(), 0, "D-pad down");
    click(157.0, 205.0);
    assert_eq!(ui.get_knob2_delta(), 1.0, "D-pad right edits value");
    click(75.0, 205.0);
    assert_eq!(ui.get_knob2_delta(), 0.0, "D-pad left edits value");
    // Deterministic timing: no real sleeps, and exact checks around the 250 ms boundary.
    let advance = |milliseconds| {
        clock.set(clock.get() + std::time::Duration::from_millis(milliseconds));
        slint::platform::update_timers_and_animations();
    };
    for (x, y, nav, edit) in [(113.0, 167.0, -1, 0.0), (157.0, 205.0, 0, 1.0),
                             (113.0, 249.0, 1, 0.0), (75.0, 205.0, 0, -1.0)] {
        ui.set_navigation_delta(0);
        ui.set_knob2_delta(0.0);
        press(x, y);
        advance(0);
        assert_eq!((ui.get_navigation_delta(), ui.get_knob2_delta()), (nav, edit), "immediate step");
        advance(249);
        assert_eq!((ui.get_navigation_delta(), ui.get_knob2_delta()), (nav, edit), "no early repeat");
        advance(1);
        assert_eq!((ui.get_navigation_delta(), ui.get_knob2_delta()), (nav * 2, edit * 2.0), "repeat at 250 ms");
        advance(100);
        assert_eq!((ui.get_navigation_delta(), ui.get_knob2_delta()), (nav * 3, edit * 3.0), "100 ms repeat cadence");
        release(0.0, 0.0);
        advance(500);
        assert_eq!((ui.get_navigation_delta(), ui.get_knob2_delta()), (nav * 3, edit * 3.0), "release outside stops repeat without an extra step");
        press(x, y);
        advance(0);
        advance(249);
        assert_eq!((ui.get_navigation_delta(), ui.get_knob2_delta()), (nav * 4, edit * 4.0), "new press gets a fresh delay");
        release(x, y);
        advance(500);
        assert_eq!((ui.get_navigation_delta(), ui.get_knob2_delta()), (nav * 4, edit * 4.0), "short tap remains one step");
    }
    ui.set_navigation_delta(0);
    ui.set_knob2_delta(0.0);
    println!("D-pad hold timing passed in all four directions: 250 ms delay, 100 ms repeats, release and re-press");
    let selections = Rc::new(std::cell::Cell::new(0));
    let count = selections.clone();
    ui.on_knob1_clicked(move || count.set(count.get() + 1));
    click(116.0, 208.0);
    click(1113.0, 41.0);
    click(116.0, 380.0);
    assert_eq!(selections.get(), 3, "D-pad center, R1 and stick click select");
    press(116.0, 380.0);
    ui.window().dispatch_event(WindowEvent::PointerMoved { position: LogicalPosition::new(142.0, 380.0) });
    assert!(ui.get_live_stick_x() > 0.9, "stick axis responds to drag");
    release(142.0, 380.0);
    assert_eq!(ui.get_live_stick_x(), 0.0, "stick returns to center");
    assert_eq!(ui.get_live_stick_y(), 0.0);
    assert_eq!(selections.get(), 3, "stick drag must not also select");
    let buttons = Rc::new(RefCell::new(Vec::new()));
    let events = buttons.clone();
    ui.on_f_clicked(move |i| events.borrow_mut().push(i));
    ui.set_transport_action("".into()); ui.set_pad_lock_available(false); ui.set_grid_mode_label("".into()); ui.set_midi_target_label("".into());
    click(446.5, 489.0); click(609.5, 489.0);
    assert!(buttons.borrow().is_empty(), "unsupported F2/F3 must not fire");
    ui.set_pad_lock_available(true); ui.set_transport_action("PLAY".into());
    for i in 0..4 { click(283.5 + i as f32 * 163.0, 489.0); }
    click(127.0, 41.0);
    assert_eq!(*buttons.borrow(), vec![0, 1, 2, 3, 0], "F1–F4 and L1 Home");
    let pads = Rc::new(RefCell::new(Vec::new()));
    let events = pads.clone();
    ui.on_pad_toggled(move |i, down| events.borrow_mut().push((i, down)));
    for i in 0..16 {
        let x = 913.0 + (i % 4) as f32 * 82.0;
        let y = 149.0 + (i / 4) as f32 * 82.0;
        click(x, y);
    }
    let expected: Vec<_> = (0..16).flat_map(|i| [(i, true), (i, false)]).collect();
    assert_eq!(*pads.borrow(), expected, "all pads retain row-major press/release gates");
    println!("Console controls passed: D-pad, joystick, L1/R1, F1–F4, and all 16 pads");
}

fn isolated_registry() -> (Registry, Arc<ModBus>) {
    let sensitivity = Arc::new(AtomicF32::new(0.1));
    let nav = Arc::new(AtomicF32::new(3.0));
    let preview_modbus = Arc::new(ModBus::new());
    let registry = Registry::new(
        Arc::new(AtomicF32::new(1000.0)),
        Arc::new(audio_devices::AudioDeviceState::new("Offline preview".into())),
        sensitivity, nav, Arc::clone(&preview_modbus), Arc::new(AudioBus::new()),
        Arc::new(AtomicF32::new(1.0)), Arc::new(MixerBus::new()),
        Arc::new(prism::PrismCcTargets::new()),
        Arc::new(std::sync::atomic::AtomicBool::new(false)), Arc::new(midi_map::MidiMap::new()),
        Arc::new(theme::ThemeColor::new(theme::ACCENT_DEFAULT_HUE, theme::ACCENT_DEFAULT_SAT, theme::ACCENT_DEFAULT_VAL)),
        Arc::new(theme::ThemeColor::new(theme::BG_DEFAULT_HUE, theme::BG_DEFAULT_SAT, theme::BG_DEFAULT_VAL)),
    );
    (registry, preview_modbus)
}
