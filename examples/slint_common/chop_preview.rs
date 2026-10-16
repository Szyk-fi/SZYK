//! Actual Slint pointer -> Chop -> audio verification, using a labelled synthetic fixture.
use super::*;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, PhysicalSize, Rgb8Pixel, SharedPixelBuffer};

pub fn render(directory: &str) {
    struct Preview(Rc<MinimalSoftwareWindow>);
    impl Platform for Preview {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(self.0.clone())
        }
    }
    std::fs::create_dir_all(directory).unwrap();
    let fixture = std::path::Path::new(directory).join("test-media");
    std::fs::create_dir_all(&fixture).unwrap();
    let data: Vec<f32> = (0..96000)
        .map(|n| {
            let t = (n % 6000) as f32 / 48000.;
            (std::f32::consts::TAU * (65. * t + 2. * (1. - (-t * 50.).exp()))).sin()
                * (-t * 35.).exp()
                * 0.7
        })
        .collect();
    apps::chop::write_wav(&fixture.join("Synthetic drum test loop.wav"), &data).unwrap();
    let bus = Arc::new(AudioBus::new());
    let mixer = Arc::new(MixerBus::new());
    let modbus = Arc::new(ModBus::new());
    let sound = apps::kids_kit::Sound::new("Chop", &modbus, &mixer, &bus);
    let app = Rc::new(RefCell::new(apps::chop::Chop::new(
        sound, bus, None, None, fixture,
    )));
    app.borrow_mut().on_enter();
    let mut processor = app.borrow_mut().audio_processor().unwrap();
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Preview(window.clone()))).unwrap();
    window.set_size(PhysicalSize::new(800, 480));
    let ui = LiveHomeScreen::new().unwrap();
    ui.set_splash_active(false);
    ui.set_on_home(false);
    ui.set_preview_render(true);
    ui.set_active_app_name("Chop".into());
    ui.show().unwrap();
    let target = app.clone();
    ui.on_screen_touched(move |x, y| target.borrow_mut().slint_pointer_pick(x, y));
    let search_target=app.clone();ui.on_library_search(move |q|search_target.borrow_mut().slint_library_search(q.as_str()));
    let pads_target=app.clone();ui.on_f_clicked(move |i|{if i==1{pads_target.borrow_mut().toggle_grid_mode();}});
    let update = || {
        ui.set_grid_mode_label(app.borrow().grid_mode_label().unwrap_or("").into());
        ui.set_transport_action(app.borrow().transport_action().unwrap_or("").into());
        apply_instrument_visual(&ui, app.borrow_mut().slint_extra());
        window.request_redraw();
        let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(800, 480);
        window.draw_if_needed(|r| {
            r.render(pixels.make_mut_slice(), 800);
        });
    };
    update();
    let tap = |x: f32, y: f32| {
        let position = LogicalPosition::new(x, y);
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    };
    let save = |name: &str| {
        update();
        let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(800, 480);
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(pixels.make_mut_slice(), 800);
        });
        let file = std::fs::File::create(std::path::Path::new(directory).join(name)).unwrap();
        let mut encoder = png::Encoder::new(file, 800, 480);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(pixels.as_bytes())
            .unwrap();
    };
    // Hit the real on-screen Pads button and check its tab order and label.
    for (id,label) in [(4,"SLICES"),(1,"EDIT"),(2,"INPUT"),(3,"FX"),(5,"LOOP"),(0,"PLAY")] {
        tap(300.,456.);update();assert_eq!(ui.get_ch_view(),id);assert_eq!(app.borrow().grid_mode_label(),Some(label));
    }
    tap(572., 209.);
    let mut samples = [0.; 1024];
    processor.process(&mut samples, 2, 48000.);
    assert!(samples.iter().any(|x| x.abs() > 1e-6));
    assert!(app
        .borrow()
        .s
        .pads
        .lock()
        .unwrap()
        .iter()
        .all(|p| p.data.is_none()));
    save("chop-preview.png");
    tap(695., 209.);
    save("chop-auto-16.png");
    for _ in 0..7 {
        tap(762., 274.);
        update();
    }
    tap(472., 330.);
    update();
    assert_eq!(
        app.borrow()
            .s
            .pads
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.data.is_some())
            .count(),
        23
    );
    tap(378., 76.);
    update();
    tap(61., 126.);
    assert_eq!(app.borrow().pad, 16);
    processor.process(&mut samples, 2, 48000.);
    assert!(samples.iter().any(|x| x.abs() > 1e-6));
    save("chop-23-slices-bank-2.png");
    tap(724., 330.);
    update();
    for fraction in [0.25, 0.5, 0.75] {
        tap(430. + 340. * fraction, 150.);
        update();
    }
    tap(560., 386.);
    update();
    assert_eq!(
        app.borrow()
            .s
            .pads
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.data.is_some())
            .count(),
        4
    );
    save("chop-manual-markers.png");
    tap(108., 374.);
    update();
    assert!(app
        .borrow()
        .s
        .loop_armed
        .load(std::sync::atomic::Ordering::Acquire));
    for _ in 0..20 {
        processor.process(&mut samples, 2, 48000.);
    }
    assert_eq!(
        app.borrow()
            .s
            .rec_len
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    save("chop-armed.png");
    tap(61., 126.);
    for _ in 0..20 {
        processor.process(&mut samples, 2, 48000.);
    }
    update();
    tap(108., 374.);
    processor.process(&mut samples, 2, 48000.);
    app.borrow_mut().tick(&Input::default());
    assert!(app
        .borrow()
        .s
        .loop_playing
        .load(std::sync::atomic::Ordering::Relaxed));
    let mut energy = 0.;
    for _ in 0..100 {
        processor.process(&mut samples, 2, 48000.);
        energy += samples.iter().map(|x| x * x).sum::<f32>();
    }
    assert!(energy > 1.);
    save("chop-performance-loop.png");
    tap(160., 28.);
    update();
    tap(485., 312.);
    update();
    assert!(app
        .borrow()
        .s
        .beat_playing
        .load(std::sync::atomic::Ordering::Acquire));
    tap(250., 76.);
    update();
    tap(61., 126.);
    update();
    tap(705., 368.);
    update();
    assert!(app.borrow().s.pads.lock().unwrap()[app.borrow().pad]
        .data
        .is_some());
    save("chop-backing-and-added-bank.png");
    // Test the shared browser with enough entries to need scrolling.
    let media=std::path::Path::new(directory).join("test-media");
    for i in 0..24 {apps::chop::write_wav(&media.join(format!("Test sample {i:02}.wav")),&data).unwrap();}
    tap(720.,88.);update();save("chop-library.png");
    let old_selection=ui.get_ch_library_selected();
    let pos=|x,y|LogicalPosition::new(x,y);
    ui.window().dispatch_event(WindowEvent::PointerPressed{position:pos(300.,300.),button:PointerEventButton::Left});
    for y in [280.,260.,240.,220.] {ui.window().dispatch_event(WindowEvent::PointerMoved{position:pos(300.,y)});update();}
    ui.window().dispatch_event(WindowEvent::PointerReleased{position:pos(300.,220.),button:PointerEventButton::Left});
    assert_eq!(ui.get_ch_library_selected(),old_selection,"scrolling must not select a file");
    save("chop-library-scrolled.png");tap(300.,190.);update();assert!(ui.get_ch_library_selected()>old_selection,"drag must move the list to later files");
    ui.invoke_library_search("Synthetic".into());update();assert_eq!(slint::Model::row_count(&ui.get_ch_library_files()),1);
    tap(300.,173.);update();tap(680.,380.);update();
    assert!(ui.get_ch_loaded_backing());assert!(app.borrow().s.backing_clip.lock().unwrap().is_some());
    save("chop-loaded-backing.png");
    tap(100.,78.);update();tap(280.,264.);update();save("chop-confirm-erase.png");
    assert!(!ui.get_ch_erase().is_empty());tap(280.,322.);update();
    assert!(app.borrow().s.pads.lock().unwrap().iter().any(|p|p.data.is_some()),"cancel preserves samples");
    tap(100.,78.);update();tap(510.,264.);update();tap(510.,322.);update();
    assert!(app.borrow().s.pads.lock().unwrap().iter().all(|p|p.data.is_none()));save("chop-cleared.png");
    tap(622.,28.);update();save("chop-sync-settings.png");
    tap(200.,136.);update();assert!(ui.get_ch_sync());
    tap(610.,374.);update();
    println!("Chop native touch/audio passed: preview, slicing, markers, bank pads, armed loop, drums and adding/loading banks");
}
