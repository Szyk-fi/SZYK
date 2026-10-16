//! Real app display audit; no audio device, game controller or window server required.
use super::*;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter};
use slint::{PhysicalSize, Rgb8Pixel, SharedPixelBuffer};

pub fn render(directory: &str) {
    struct Preview(Rc<MinimalSoftwareWindow>);
    impl Platform for Preview {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> { Ok(self.0.clone()) }
    }
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Preview(window.clone()))).unwrap();
    let ui = LiveHomeScreen::new().unwrap();
    ui.set_splash_active(false);
    ui.set_preview_render(true);
    ui.show().unwrap();
    std::fs::create_dir_all(directory).unwrap();
    verify_parameter_hit_targets(&ui, &window);
    let manifests = manifest::discover(std::path::Path::new(APPS_DIR));
    let mut report = String::from("app,view,width,height\n");
    for manifest in &manifests {
        if let Ok(filter) = std::env::var("PORTAMAX_RENDER_APP") {
            if !filter.split(',').any(|name| name == manifest.name || name == manifest.id) { continue; }
        }
        let (registry, _) = instrument_preview::isolated_registry();
        let (_, mut app) = registry.build(std::slice::from_ref(manifest)).pop().unwrap();
        // Device enumeration is outside this offline layout audit.
        if manifest.id != "settings" && manifest.id != "controller" { app.on_enter(); }
        let mut processor = app.audio_processor();
        let mut samples = [0.0; 1024];
        let startup_blocks = if matches!(manifest.id.as_str(), "oc" | "oc2" | "oc3" | "oc4") { 200 } else { 8 };
        for _ in 0..startup_blocks {
            if let Some(p) = processor.as_mut() { p.process(&mut samples, 2, 48000.); }
            assert!(samples.iter().all(|x| x.is_finite()), "{}: non-finite startup audio", manifest.id);
        }
        app.tick(&Input::default());
        for menu in [false, true] {
            if menu { app.tick(&Input { shoulder_press: [false, true], ..Default::default() }); }
            let (rows, selected, above, below) = app.slint_windowed_rows(VISIBLE_ROWS);
            ui.set_row_names(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.0.clone().into()).collect::<Vec<slint::SharedString>>())).into());
            ui.set_row_values(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.1.clone().into()).collect::<Vec<slint::SharedString>>())).into());
            ui.set_row_is_group(Rc::new(slint::VecModel::from(rows.iter().map(|r| r.2).collect::<Vec<_>>())).into());
            ui.set_selected_row(selected as i32); ui.set_more_above(above); ui.set_more_below(below);
            ui.set_active_app_name(manifest.name.as_str().into());
            ui.set_on_home(false);
            ui.set_hint_text(app.hint().into());
            ui.set_transport_action(app.transport_action().unwrap_or("").into());
            let (bg, ink, accent, _) = app_palette(&manifest.name).unwrap_or((slint::Color::from_rgb_u8(20,25,31), slint::Color::from_rgb_u8(231,237,244), slint::Color::from_rgb_u8(165,188,233), slint::Color::from_rgb_u8(137,148,170)));
            ui.set_live_bg(bg); ui.set_live_ink(ink); ui.set_live_accent(accent);
            apply_scale_visual(&ui, app.slint_scale_info());
            apply_play_column(&ui, app.play_column());
            apply_instrument_visual(&ui, app.slint_extra());
            for (w, h) in [(800,480), (1000,600), (1460,656), (1600,900)] {
                window.set_size(PhysicalSize::new(w,h));
                slint::platform::update_timers_and_animations();
                assert_eq!(ui.get_live_display_width(), if w < 1460 {w as f32} else {800.});
                assert_eq!(ui.get_live_display_height(), if w < 1460 {h as f32} else {480.});
                let slug = format!("{}-{}-{w}x{h}", manifest.id, if menu {"menu"} else {"play"});
                save(&window, directory, &slug, w, h);
                report.push_str(&format!("{},{},{w},{h}\n",manifest.id,if menu {"menu"} else {"play"}));
            }
        }
        app.on_exit();
        println!("Display audit: {}", manifest.name);
    }
    // The launcher and both logos exercise shell chrome and full-panel overlays.
    for (name, splash, szyk) in [("launcher",false,false),("startup-szyk",true,true),("startup-rove",true,false)] {
        ui.set_on_home(true); ui.set_pc_active(false); ui.set_active_kind(0);
        ui.set_hint_text(app::hints::HOME.into());
        ui.set_splash_active(splash); ui.set_splash_is_szyk(szyk);
        ui.set_home_total(manifests.len() as i32); ui.set_home_count(manifests.len() as i32);
        ui.set_home_more_below(manifests.len() > 6);
        ui.set_home_title(manifests[0].name.as_str().into());
        ui.set_home_description(manifests[0].description.as_str().into());
        ui.set_home_names(Rc::new(slint::VecModel::from(manifests.iter().take(6).map(|m|m.name.clone().into()).collect::<Vec<slint::SharedString>>())).into());
        for (w,h) in [(800,480),(1460,656)] {
            window.set_size(PhysicalSize::new(w,h)); slint::platform::update_timers_and_animations();
            save(&window,directory,&format!("{name}-{w}x{h}"),w,h);
        }
    }
    verify_launcher_touch(&ui, &window);
    verify_play_dial_touch(&ui, &window);
    std::fs::write(std::path::Path::new(directory).join("coverage.csv"),report).unwrap();
}
fn save(window: &MinimalSoftwareWindow, directory: &str, slug: &str, w: u32, h: u32) {
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(w,h);
    window.request_redraw();
    window.draw_if_needed(|renderer| {renderer.render(pixels.make_mut_slice(), w as usize);});
    let path = std::path::Path::new(directory).join(format!("{slug}.png"));
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).unwrap()),w,h);
    encoder.set_color(png::ColorType::Rgb); encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(pixels.as_bytes()).unwrap();
}

fn verify_parameter_hit_targets(ui: &LiveHomeScreen, window: &MinimalSoftwareWindow) {
    use slint::{LogicalPosition, platform::{PointerEventButton, WindowEvent}};
    window.set_size(PhysicalSize::new(800,480));
    ui.set_on_home(false); ui.set_pc_active(false); ui.set_active_kind(0);
    ui.set_row_names(Rc::new(slint::VecModel::from(vec![slint::SharedString::from("Frequency")])).into());
    ui.set_row_values(Rc::new(slint::VecModel::from(vec![slint::SharedString::from("440 Hz")])).into());
    ui.set_row_is_group(Rc::new(slint::VecModel::from(vec![false])).into());
    ui.set_more_above(false); ui.set_more_below(false);
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    ui.on_parameter_touched(move |row, delta| sink.borrow_mut().push((row,delta)));
    slint::platform::update_timers_and_animations();
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(800,480);
    window.request_redraw(); window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),800); });
    for x in [80.,307.,353.] {
        let position = LogicalPosition::new(x,70.);
        ui.window().dispatch_event(WindowEvent::PointerPressed {position,button:PointerEventButton::Left});
        ui.window().dispatch_event(WindowEvent::PointerReleased {position,button:PointerEventButton::Left});
    }
    assert_eq!(*events.borrow(),vec![(0,0),(0,-1),(0,1)],"native-size row selection and both edit hit targets");
    events.borrow_mut().clear();
    let position=LogicalPosition::new(80.,80.);
    ui.window().dispatch_event(WindowEvent::PointerPressed{position,button:PointerEventButton::Left});
    ui.window().dispatch_event(WindowEvent::PointerMoved{position:LogicalPosition::new(80.,60.)});
    ui.window().dispatch_event(WindowEvent::PointerReleased{position:LogicalPosition::new(80.,60.),button:PointerEventButton::Left});
    assert_eq!(*events.borrow(),vec![(0,5)],"parameter drag edits without extra tap");
    println!("800x480 shared parameter hit targets and drag passed");
}

fn verify_launcher_touch(ui: &LiveHomeScreen, window: &MinimalSoftwareWindow) {
    use slint::platform::{WindowEvent, PointerEventButton};
    use slint::LogicalPosition;
    window.set_size(PhysicalSize::new(800,480));
    ui.set_splash_active(false); ui.set_on_home(true);
    ui.set_home_count(100); ui.set_home_offset(0); ui.set_home_selected(0);
    ui.set_home_names(Rc::new(slint::VecModel::from(vec![slint::SharedString::from("Touch app");6])).into());
    ui.set_home_ids(Rc::new(slint::VecModel::from(vec![0,1,2,3,4,5])).into());
    let opened=Rc::new(std::cell::Cell::new(0)); let events=opened.clone();
    ui.on_home_open(move |_|events.set(events.get()+1));
    let scroll=Rc::new(std::cell::Cell::new(-1f32)); let events=scroll.clone();
    ui.on_home_scroll_to(move |p|events.set(p));
    slint::platform::update_timers_and_animations();
    let send=|kind,x,y|ui.window().dispatch_event(match kind {
        0=>WindowEvent::PointerPressed{position:LogicalPosition::new(x,y),button:PointerEventButton::Left},
        1=>WindowEvent::PointerMoved{position:LogicalPosition::new(x,y)},
        _=>WindowEvent::PointerReleased{position:LogicalPosition::new(x,y),button:PointerEventButton::Left},
    });
    send(0,220.,140.);
    // The real timer replaces these models while a pointer is held.
    ui.set_home_names(Rc::new(slint::VecModel::from(vec![slint::SharedString::from("Updated app");6])).into());
    ui.set_home_ids(Rc::new(slint::VecModel::from(vec![0,1,2,3,4,5])).into());
    slint::platform::update_timers_and_animations();
    send(1,220.,80.);
    ui.set_home_names(Rc::new(slint::VecModel::from(vec![slint::SharedString::from("Scrolled app");6])).into());
    send(2,220.,80.);
    assert!(scroll.get()>0.,"swiping app rows scrolls");
    assert_eq!(opened.get(),0,"swipe must not launch an app");
    send(0,220.,100.);
    ui.set_home_names(Rc::new(slint::VecModel::from(vec![slint::SharedString::from("Refreshed app");6])).into());
    send(2,220.,100.);
    assert_eq!(opened.get(),1,"tap launches app across model replacement");
    send(0,600.,395.);send(2,600.,395.);
    assert_eq!(opened.get(),2,"visible Open application button opens selected app");
    send(0,443.,390.);send(2,443.,390.);
    assert!(scroll.get()>0.9,"scroll rail reaches bottom");
    println!("Launcher touch passed: swipe scrolls without opening, tap opens, scrollbar seeks");
}

fn verify_play_dial_touch(ui: &LiveHomeScreen, window: &MinimalSoftwareWindow) {
    use slint::{LogicalPosition,platform::{WindowEvent,PointerEventButton}};
    ui.set_on_home(false);ui.set_active_kind(0);ui.set_pc_active(true);
    ui.set_pc_dial_labels(Rc::new(slint::VecModel::from(vec![slint::SharedString::from("Tone")])).into());
    ui.set_pc_dial_values(Rc::new(slint::VecModel::from(vec![slint::SharedString::from("50%")])).into());
    let edits=Rc::new(RefCell::new(Vec::new()));let sink=edits.clone();
    ui.on_play_dial_edited(move |i,d|sink.borrow_mut().push((i,d)));
    slint::platform::update_timers_and_animations();
    let mut pixels=SharedPixelBuffer::<Rgb8Pixel>::new(800,480);
    window.request_redraw();window.draw_if_needed(|r|{r.render(pixels.make_mut_slice(),800);});
    ui.window().dispatch_event(WindowEvent::PointerPressed{position:LogicalPosition::new(40.,85.),button:PointerEventButton::Left});
    ui.window().dispatch_event(WindowEvent::PointerMoved{position:LogicalPosition::new(40.,65.)});
    ui.window().dispatch_event(WindowEvent::PointerReleased{position:LogicalPosition::new(40.,65.),button:PointerEventButton::Left});
    assert_eq!(*edits.borrow(),vec![(0,5)],"play knob drag emits edit steps");
    ui.set_pc_pad_labels(Rc::new(slint::VecModel::from(vec![slint::SharedString::from("Pad");16])).into());
    let pads=Rc::new(RefCell::new(Vec::new()));let sink=pads.clone();
    ui.on_pad_toggled(move |i,down|sink.borrow_mut().push((i,down)));
    ui.window().dispatch_event(WindowEvent::PointerPressed{position:LogicalPosition::new(40.,155.),button:PointerEventButton::Left});
    ui.window().dispatch_event(WindowEvent::PointerReleased{position:LogicalPosition::new(40.,155.),button:PointerEventButton::Left});
    assert_eq!(*pads.borrow(),vec![(0,true),(0,false)],"on-screen play pad has both gate edges");
    let functions=Rc::new(RefCell::new(Vec::new()));let sink=functions.clone();
    ui.on_f_clicked(move |i|sink.borrow_mut().push(i));
    ui.window().dispatch_event(WindowEvent::PointerPressed{position:LogicalPosition::new(70.,52.),button:PointerEventButton::Left});
    ui.window().dispatch_event(WindowEvent::PointerReleased{position:LogicalPosition::new(70.,52.),button:PointerEventButton::Left});
    assert_eq!(*functions.borrow(),vec![1],"on-screen layer control uses F2 action");
    ui.window().dispatch_event(WindowEvent::PointerPressed{position:LogicalPosition::new(58.,275.),button:PointerEventButton::Left});
    assert!(ui.get_live_stick_x()>0.5,"on-screen stick responds");
    ui.window().dispatch_event(WindowEvent::PointerReleased{position:LogicalPosition::new(58.,275.),button:PointerEventButton::Left});
    assert_eq!(ui.get_live_stick_x(),0.,"on-screen stick releases");
    let before_menu=ui.get_r1_presses();
    ui.window().dispatch_event(WindowEvent::PointerPressed{position:LogicalPosition::new(280.,52.),button:PointerEventButton::Left});
    ui.window().dispatch_event(WindowEvent::PointerReleased{position:LogicalPosition::new(280.,52.),button:PointerEventButton::Left});
    assert_eq!(ui.get_r1_presses(),before_menu+1,"on-screen Menu uses shoulder edge");
    let manifests=manifest::discover(std::path::Path::new(APPS_DIR));
    let manifest=manifests.into_iter().find(|m|m.name=="Plaits").unwrap();
    let(registry,_)=instrument_preview::isolated_registry();
    let(_,mut app)=registry.build(&[manifest]).pop().unwrap();
    app.on_enter();
    let before=app.play_column().unwrap().dials[0].norm;
    assert!(edit_play_dial(app.as_mut(),0,1));
    assert_ne!(app.play_column().unwrap().dials[0].norm,before,"knob updates real app parameter");
    println!("Shared play touch passed: knob, pad press/release, layer, menu, stick and real Plaits edit");
}
