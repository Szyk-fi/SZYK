//! Verifies real mouse events through the live Slint shell into the real app.
use super::*;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{LogicalPosition, PhysicalSize, Rgb8Pixel, SharedPixelBuffer};
pub fn render(directory: &str) {
    struct TestPlatform(Rc<MinimalSoftwareWindow>);
    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(self.0.clone())
        }
    }
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
    window.set_size(PhysicalSize::new(1240, 560));
    let ui = LiveHomeScreen::new().unwrap();
    ui.set_splash_active(false);
    ui.set_preview_render(true);
    ui.set_on_home(false);
    ui.set_active_app_name("Bebot".into());
    ui.show().unwrap();
    let a = Rc::new(RefCell::new(apps::bebot::BebotApp::new(
        Arc::new(AudioBus::new()),
        Arc::new(MixerBus::new()),
        Arc::new(ModBus::new()),
    )));
    // Sine, free pitch, dry: an unambiguous check that dragging actually retunes DSP.
    for (i, d) in [(0, 2), (2, -1), (13, -6), (14, -6), (11, -4)] {
        a.borrow_mut().adjust_setting(i, d);
    }
    let mut processor = a.borrow_mut().audio_processor().unwrap();
    let target = a.clone();
    ui.on_screen_touched(move |x, y| target.borrow_mut().slint_pointer_pick(x, y));
    apply_instrument_visual(&ui, a.borrow_mut().slint_extra());
    let mut initial_pixels = SharedPixelBuffer::<Rgb8Pixel>::new(1240, 560);
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(initial_pixels.make_mut_slice(), 1240);
    });
    let position = |x: f32, y: f32| LogicalPosition::new(208. + x, 92. + y);
    let mut b = vec![0.; 24000];
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position: position(200., 120.),
        button: PointerEventButton::Right,
    });
    processor.process(&mut b, 2, 48000.);
    assert!(
        b.iter().all(|x| *x == 0.),
        "Right mouse button must not sing"
    );
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position: position(200., 120.),
        button: PointerEventButton::Right,
    });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position: position(100., 120.),
        button: PointerEventButton::Left,
    });
    processor.process(&mut b, 2, 48000.);
    assert!(
        b.iter().any(|x| x.abs() > 0.02),
        "Slint mouse press must produce audio"
    );
    let frequency = |b: &[f32]| {
        let mono: Vec<f32> = b.chunks(2).map(|f| f[0]).collect();
        mono[4000..]
            .windows(2)
            .filter(|s| s[0] <= 0. && s[1] > 0.)
            .count() as f32
            * 48000.
            / (mono.len() - 4000) as f32
    };
    let low = frequency(&b);
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: position(550., 120.),
    });
    processor.process(&mut b, 2, 48000.);
    let high = frequency(&b);
    assert!(
        high > low * 2.,
        "Slint mouse drag must change pitch: {low} -> {high}"
    );
    apply_instrument_visual(&ui, a.borrow_mut().slint_extra());
    std::fs::create_dir_all(directory).unwrap();
    let save = |name: &str| {
        let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(1240, 560);
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(pixels.make_mut_slice(), 1240);
        });
        let file = std::fs::File::create(std::path::Path::new(directory).join(name)).unwrap();
        let mut encoder = png::Encoder::new(file, 1240, 560);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(pixels.as_bytes())
            .unwrap();
    };
    save("Bebot-Slint-Play.png");
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position: position(550., 120.),
        button: PointerEventButton::Left,
    });
    for _ in 0..12 {
        processor.process(&mut b, 2, 48000.);
    }
    assert!(
        b.iter().all(|x| x.abs() < 0.0001),
        "Slint mouse release must stop audio"
    );
    let app::SlintExtra::Screen(before_settings) = a.borrow_mut().slint_extra() else {
        unreachable!()
    };
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position: position(580., 20.),
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position: position(580., 20.),
        button: PointerEventButton::Left,
    });
    let app::SlintExtra::Screen(settings) = a.borrow_mut().slint_extra() else {
        unreachable!()
    };
    assert!(
        settings
            .frame_rgba
            .iter()
            .zip(&before_settings.frame_rgba)
            .filter(|(a, b)| a != b)
            .count()
            > 10000,
        "Mouse toolbar must open settings in the actual Slint shell"
    );
    apply_instrument_visual(&ui, app::SlintExtra::Screen(settings));
    save("Bebot-Slint-Settings.png");
    println!("Bebot: real Slint mouse press / drag / release passed; {low:.1} -> {high:.1} Hz. Two live UI previews written.");
}
