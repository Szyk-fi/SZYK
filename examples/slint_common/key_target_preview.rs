//! Software-render the real selector and verify its callback with real apps/audio.
use super::*;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::{PhysicalSize, Rgb8Pixel, SharedPixelBuffer};

pub fn render(directory: &str) {
    struct Preview(Rc<MinimalSoftwareWindow>);
    impl Platform for Preview {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(self.0.clone())
        }
    }
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Preview(window.clone()))).unwrap();
    window.set_size(PhysicalSize::new(1460, 656));
    let ui = LiveHomeScreen::new().unwrap();
    ui.set_splash_active(false);
    ui.set_preview_render(true);
    ui.set_on_home(false);
    ui.set_active_app_name("Beads".into());
    let mut context = app::AppContext::new();
    context
        .provide(Arc::new(AudioBus::new()))
        .provide(Arc::new(ModBus::new()))
        .provide(Arc::new(MixerBus::new()))
        .provide(Arc::new(note_bus::NoteBus::new()));
    context
        .provide_named("sensitivity", Arc::new(util::AtomicF32::new(0.1)))
        .provide_named("nav_speed", Arc::new(util::AtomicF32::new(3.)));
    let registry = Registry::with_factories(
        context,
        &[
            ("bebot", apps::bebot::create),
            ("beads", apps::beads::create),
        ],
    );
    let apps = Rc::new(RefCell::new(registry.build(&[
        manifest::AppManifest {
            id: "bebot".into(),
            name: "Bebot".into(),
            notes_in: true,
            ..Default::default()
        },
        manifest::AppManifest {
            id: "beads".into(),
            name: "Beads".into(),
            ..Default::default()
        },
    ])));
    apps.borrow_mut()[1].1.on_enter();
    let routing = Rc::new(RefCell::new(controller::performance::Routing::default()));
    let ids = Rc::new(RefCell::new(controller::performance::Routing::options(
        &apps.borrow(),
    )));
    ui.set_keyboard_options(
        Rc::new(slint::VecModel::from(
            controller::performance::Routing::option_names(&apps.borrow())
                .into_iter()
                .map(slint::SharedString::from)
                .collect::<Vec<_>>(),
        ))
        .into(),
    );
    bind_keyboard_selector(
        &ui,
        routing.clone(),
        apps.clone(),
        ids.clone(),
        Rc::new(RefCell::new(Some(1))),
    );
    let choice = ids
        .borrow()
        .iter()
        .position(|i| *i == controller::performance::Target::Instrument(0))
        .unwrap() as i32;
    ui.set_keyboard_choice(choice);
    ui.invoke_keyboard_target_picked(choice);
    assert_eq!(routing.borrow().pinned(), Some(0));
    assert_eq!(
        ui.get_active_app_name(),
        "Beads",
        "picking an instrument must not switch the control screen"
    );
    let mut input = Input::default();
    input.midi_keys.0[60] = 100;
    let screen = routing
        .borrow_mut()
        .route(Some(1), &mut apps.borrow_mut(), input);
    assert_eq!(screen.midi_keys.0[60], 0);
    let mut dsp = apps.borrow_mut()[0].1.audio_processor().unwrap();
    let mut peak = 0f32;
    for _ in 0..60 {
        let mut audio = [0.; 1024];
        dsp.process(&mut audio, 2, 48000.);
        peak = audio.iter().fold(peak, |a, v| a.max(v.abs()));
    }
    assert!(
        peak > 0.01,
        "the selector must wake and play a cold hidden Bebot"
    );
    apps.borrow_mut()[1].1.tick(&screen);
    {
        let mut apps = apps.borrow_mut();
        let app = &mut apps[1].1;
        let (rows, selected, above, below) = app.slint_windowed_rows(14);
        ui.set_row_names(
            Rc::new(slint::VecModel::from(
                rows.iter()
                    .map(|r| slint::SharedString::from(r.0.as_str()))
                    .collect::<Vec<_>>(),
            ))
            .into(),
        );
        ui.set_row_values(
            Rc::new(slint::VecModel::from(
                rows.iter()
                    .map(|r| slint::SharedString::from(r.1.as_str()))
                    .collect::<Vec<_>>(),
            ))
            .into(),
        );
        ui.set_row_is_group(
            Rc::new(slint::VecModel::from(
                rows.iter().map(|r| r.2).collect::<Vec<_>>(),
            ))
            .into(),
        );
        ui.set_selected_row(selected as i32);
        ui.set_more_above(above);
        ui.set_more_below(below);
        apply_play_column(&ui, app.play_column());
        ui.set_grid_mode_label(app.grid_mode_label().unwrap_or("").into());
        ui.set_pad_lock_available(app.supports_pad_lock());
        apply_instrument_visual(&ui, app.slint_extra());
    }
    ui.set_midi_target_label("Bebot".into());
    ui.show().unwrap();
    std::fs::create_dir_all(directory).unwrap();
    let colors = pad_lights::compose(apps.borrow()[0].1.grid_led_overlay(), [false; 16])
        .iter()
        .map(|c| {
            let (r, g, b) = pad_lights::rgb(*c);
            slint::Color::from_rgb_u8(r, g, b)
        })
        .collect::<Vec<_>>();
    ui.set_live_pad_colors(Rc::new(slint::VecModel::from(colors)).into());
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(1460, 656);
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(pixels.make_mut_slice(), 1460);
    });
    let file = std::fs::File::create(
        std::path::Path::new(directory).join("rove-bebot-keys-beads-controls.png"),
    )
    .unwrap();
    let mut encoder = png::Encoder::new(file, 1460, 656);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(pixels.as_bytes())
        .unwrap();
    ui.invoke_midi_arm_clicked();
    assert_eq!(
        routing.borrow().selection(),
        controller::performance::Target::Off
    );
    let screen = routing
        .borrow_mut()
        .route(Some(1), &mut apps.borrow_mut(), input);
    assert_eq!(
        screen.midi_keys.0[60], 0,
        "Disarm must stop MIDI rather than arm the edit screen"
    );
    ui.set_keyboard_choice(0);
    ui.invoke_keyboard_target_picked(0);
    assert_eq!(routing.borrow().pinned(), None);
    let screen = routing
        .borrow_mut()
        .route(Some(1), &mut apps.borrow_mut(), input);
    assert_eq!(
        screen.midi_keys.0[60], 0,
        "reassignment must wait for a held key's release"
    );
    println!("ROVE MIDI Arm: cold Bebot plays while Beads stays visible; Disarm stops notes; Follow screen restores automatic routing. Preview rendered.");
}
