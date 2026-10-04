//! Screenshots for the Atlas how-to (docs/ATLAS_HOWTO.md), rendered from
//! the real app driven with real `Input` -- nothing is mocked up. Run the
//! live example with `--render-atlas-howto OUTPUT_DIRECTORY`; it writes
//! `NN-name.png`, each the whole device frame (1240x560) so the buttons the
//! text talks about are in the picture.
use super::*;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::{PhysicalSize, Rgb8Pixel, SharedPixelBuffer};

pub fn render(directory: &str) {
    // A clock the harness owns, so each shot can let the UI's animations
    // finish (bars and dials ease toward new values) before it renders.
    struct HowtoPlatform(Rc<MinimalSoftwareWindow>, Rc<std::cell::Cell<std::time::Duration>>);
    impl Platform for HowtoPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(self.0.clone())
        }
        fn duration_since_start(&self) -> std::time::Duration {
            self.1.get()
        }
    }
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let clock = Rc::new(std::cell::Cell::new(std::time::Duration::ZERO));
    slint::platform::set_platform(Box::new(HowtoPlatform(window.clone(), clock.clone()))).unwrap();
    window.set_size(PhysicalSize::new(1240, 560));
    std::fs::create_dir_all(directory).unwrap();
    // The run stores a moment and binds a route; keep those out of the
    // player's real saves/.
    let scratch = std::env::temp_dir().join(format!("atlas_howto_saves_{}", std::process::id()));
    std::env::set_var("PORTAMAX_SAVES_DIR", &scratch);
    let ui = LiveHomeScreen::new().unwrap();
    ui.set_splash_active(false);
    ui.set_preview_render(true);
    ui.set_on_home(false);
    ui.set_play_surface(true);
    ui.show().unwrap();

    let (registry, _modbus) = registry_with_audio(Arc::new(AudioBus::new()));
    let manifests = manifest::discover(std::path::Path::new(APPS_DIR));
    let manifest = manifests.iter().find(|m| m.id == "atlas").expect("the atlas manifest");
    let mut app: Box<dyn App> = registry.build(std::slice::from_ref(manifest)).pop().unwrap().1;
    app.on_enter();
    let mut dsp = app.audio_processor().expect("Atlas makes sound");
    let mut block = [0.0f32; 1024];

    // Runs `frames` frames of `input`, rendering audio alongside so the
    // spectrum, voice count and CPU readouts are live.
    let mut run = |app: &mut Box<dyn App>, input: &Input, frames: usize| {
        for _ in 0..frames {
            app.tick(input);
            block.fill(0.0);
            dsp.process(&mut block, 2, 48_000.0);
        }
    };

    let mut shot = |app: &mut Box<dyn App>, file: &str, stick: [f32; 2], hands: [f32; 2]| {
        let (rows, selected, above, below) = app.slint_windowed_rows(10);
        ui.set_row_names(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.0.clone().into()).collect::<Vec<slint::SharedString>>())).into());
        ui.set_row_values(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.1.clone().into()).collect::<Vec<slint::SharedString>>())).into());
        ui.set_row_is_group(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.2).collect::<Vec<bool>>())).into());
        ui.set_selected_row(selected as i32);
        ui.set_more_above(above);
        ui.set_more_below(below);
        ui.set_active_app_name("Atlas".into());
        ui.set_grid_mode_label(app.grid_mode_label().unwrap_or("").into());
        ui.set_transport_action(app.transport_action().unwrap_or("").into());
        ui.set_pad_lock_available(app.supports_pad_lock());
        let levels = app.slint_levels(10);
        ui.set_row_levels(Rc::new(slint::VecModel::from(levels.into_iter().map(|l| l.unwrap_or(-1.0)).collect::<Vec<_>>())).into());
        ui.set_transport_label("".into());
        // Atlas has no palette of its own in the table, so it gets the shell default,
        // as in the normal preview.
        let (bg, ink, accent, _) = app_palette("Atlas").unwrap_or((slint::Color::from_rgb_u8(20, 25, 31), slint::Color::from_rgb_u8(231, 237, 244), slint::Color::from_rgb_u8(165, 188, 233), slint::Color::from_rgb_u8(137, 148, 170)));
        ui.set_live_bg(bg);
        ui.set_live_ink(ink);
        ui.set_live_accent(accent);
        ui.set_live_stick_x(stick[0]);
        ui.set_live_stick_y(-stick[1]);
        ui.set_live_hand_l(hands[0]);
        ui.set_live_hand_r(hands[1]);
        apply_scale_visual(&ui, app.slint_scale_info());
        apply_play_column(&ui, app.play_column());
        apply_instrument_visual(&ui, app.slint_extra());
        slint::platform::update_timers_and_animations();
        clock.set(clock.get() + std::time::Duration::from_secs(2));
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(1240, 560);
        window.draw_if_needed(|renderer| {
            renderer.render(pixels.make_mut_slice(), 1240);
        });
        let path = std::path::Path::new(directory).join(file);
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&path).unwrap()), 1240, 560);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(pixels.as_bytes()).unwrap();
        println!("wrote {}", path.display());
    };

    let held = |pads: &[usize]| Input { grid: std::array::from_fn(|k| pads.contains(&k)), ..Default::default() };
    let none = Input::default();
    let r1 = Input { shoulder_press: [false, true], ..Default::default() };

    // 01 -- the play view, a chord held.
    run(&mut app, &held(&[0, 5, 10]), 90);
    shot(&mut app, "01-play.png", [0.0, 0.0], [0.0, 0.0]);

    // 02 -- playing it: stick, a hand over the sensor and a firm pad.
    let mut firm = held(&[0, 5, 10]);
    firm.stick = [0.7, 0.5];
    firm.hands = [0.6, 0.0];
    firm.pad_pressure[0] = 0.8;
    run(&mut app, &firm, 30);
    shot(&mut app, "02-expression.png", [0.7, 0.5], [0.6, 0.0]);
    run(&mut app, &none, 30);

    // 03 -- D-pad up/down browses presets (up = next).
    run(&mut app, &Input { navigation_steps: -1, ..Default::default() }, 1);
    run(&mut app, &Input { navigation_steps: -1, ..Default::default() }, 1);
    run(&mut app, &held(&[3, 6, 9]), 60);
    shot(&mut app, "03-browse.png", [0.0, 0.0], [0.0, 0.0]);

    // 04 -- SELECT moves the focused dial; D-pad right turns it.
    run(&mut app, &none, 5);
    run(&mut app, &Input { knob1_press: true, ..Default::default() }, 1);
    run(&mut app, &Input { knob2: 1, nav_x: 1, ..Default::default() }, 40);
    shot(&mut app, "04-focus.png", [0.0, 0.0], [0.0, 0.0]);

    // 04b -- hold L1 and push the joystick: the selected dial is set directly
    // (right of centre is above 50%). The Controls layer shows while L1 is down.
    let set = Input { shoulders: [true, false], stick: [0.5, -0.3], ..Default::default() };
    run(&mut app, &set, 3);
    shot(&mut app, "04b-set.png", [0.5, -0.3], [0.0, 0.0]);
    run(&mut app, &none, 5);

    // 05 -- F2: STATES, a state pad tapped and MORPH part way there.
    app.toggle_grid_mode();
    run(&mut app, &none, 2);
    shot(&mut app, "05-states.png", [0.0, 0.0], [0.0, 0.0]);
    let b_pad = crate::app::play_kit::rank_pad(1);
    run(&mut app, &held(&[b_pad]), 1);
    run(&mut app, &none, 14);
    shot(&mut app, "06-states-gliding.png", [0.0, 0.0], [0.0, 0.0]);
    run(&mut app, &none, 120);

    // 07 -- F2: CONTROLS, one pad grabbed; 08 -- wiggle the stick to bind.
    app.toggle_grid_mode();
    run(&mut app, &none, 2);
    let space_pad = crate::app::play_kit::rank_pad(3);
    run(&mut app, &held(&[space_pad]), 2);
    run(&mut app, &none, 2);
    shot(&mut app, "07-controls.png", [0.0, 0.0], [0.0, 0.0]);
    let sweep = Input { stick: [1.0, 0.0], ..Default::default() };
    run(&mut app, &sweep, 3);
    shot(&mut app, "08-bind.png", [1.0, 0.0], [0.0, 0.0]);
    run(&mut app, &none, 5);

    // 09 -- F2: MOMENTS, one stored by holding a pad.
    app.toggle_grid_mode();
    run(&mut app, &none, 2);
    let hold = held(&[crate::app::play_kit::rank_pad(0)]);
    run(&mut app, &hold, 1);
    std::thread::sleep(std::time::Duration::from_millis(700));
    run(&mut app, &hold, 2);
    run(&mut app, &none, 2);
    shot(&mut app, "09-moments.png", [0.0, 0.0], [0.0, 0.0]);

    // 10 -- R1: the full menu.
    app.toggle_grid_mode(); // back round to PLAY
    run(&mut app, &none, 2);
    run(&mut app, &r1, 1);
    run(&mut app, &none, 2);
    shot(&mut app, "10-menu.png", [0.0, 0.0], [0.0, 0.0]);

    // 11 -- Depth: Edit adds every parameter, voice settings and states.
    let go = |app: &mut Box<dyn App>, label: &str| {
        let at = app.slint_rows().iter().position(|r| r.0.trim() == label).unwrap_or_else(|| panic!("no row {label}"));
        let step = at as i32 - app.slint_selected() as i32;
        app.tick(&Input { navigation_steps: step, ..Default::default() });
        app.tick(&none);
    };
    go(&mut app, "Depth");
    run(&mut app, &Input { knob2: 1, nav_x: 1, ..Default::default() }, 1);
    run(&mut app, &none, 2);
    shot(&mut app, "11-menu-edit.png", [0.0, 0.0], [0.0, 0.0]);
    go(&mut app, "Store → A");
    run(&mut app, &none, 2);
    shot(&mut app, "12-menu-states.png", [0.0, 0.0], [0.0, 0.0]);

    // 13 -- Depth: Deep adds the compiled graph and the CPU budget.
    go(&mut app, "Depth");
    run(&mut app, &Input { knob2: 1, nav_x: 1, ..Default::default() }, 1);
    run(&mut app, &none, 2);
    go(&mut app, "CPU Budget");
    run(&mut app, &none, 2);
    shot(&mut app, "13-menu-deep.png", [0.0, 0.0], [0.0, 0.0]);
    std::fs::remove_dir_all(scratch).ok();
}
