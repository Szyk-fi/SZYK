//! Offline visual verification with real module state and processors.
//! Run the live example with --render-instruments OUTPUT_DIRECTORY.
use super::*;
use crate::apps::{collection, cv_out, prism};
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
    let preview_audio=Arc::new(AudioBus::new());
    let tone=preview_audio.register("Verification tone");
    *tone.lock().unwrap()=(0..512).map(|i|(std::f32::consts::TAU*i as f32/32.).sin()*0.25).collect();
    let (registry, preview_modbus) = registry_with_audio(preview_audio.clone());
    let mut manifests = manifest::discover(std::path::Path::new(APPS_DIR));
    for (id, name) in [("analyzer", "Analyzer"), ("synth", "Synth")] {
        if !manifests.iter().any(|m| m.id == id) {
            manifests.push(manifest::AppManifest { id: id.into(), name: name.into(), audio_outputs: vec![name.into()], ..Default::default() });
        }
    }
    let _catalog=registry.build(&manifests);
    let catalog_names=preview_audio.names();
    assert!(catalog_names.iter().any(|n|n=="Bloom"));
    assert!(preview_audio.owned_indices("bloom").iter().all(|i|!preview_audio.is_claimed(*i)),"Catalog must not construct Bloom");
    let mut route_source=if std::env::var("PORTAMAX_RENDER_SOURCE").ok().as_deref()==Some("Bloom") {
        let manifest=manifests.iter().find(|m|m.id=="bloom").unwrap();
        let mut app=registry.build(std::slice::from_ref(manifest)).pop().unwrap().1;
        app.on_enter();app.toggle_running();let dsp=app.audio_processor();Some((app,dsp))
    }else{None};

    if let Ok(name)=std::env::var("PORTAMAX_RENDER_APP") {manifests.retain(|m|name.split(',').any(|n|m.name==n));assert!(!manifests.is_empty(),"Unknown preview app: {name}");}
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
            if matches!(manifest.id.as_str(), "forge"|"field"|"sample_hunter"|"studio"|"reference"|"vinyl"|"practice"|"radio"|"memories") {
                assert_eq!(app.running(),Some(false),"{} must not start without source/media",manifest.id);
            } else { assert_ne!(app.running(), Some(running), "{} transport did not change", manifest.id); }
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
        // Apps with a play view open on it, where the D-pad browses; the
        // menu-driven setup and list-navigation checks below belong to
        // their menu (R1); the render returns to the play view after.
        let has_play_view = app.play_column().is_some();
        if has_play_view {
            app.tick(&Input { shoulder_press: [false, true], ..Default::default() });
            assert!(app.play_column().is_none(), "{name}: R1 must open the menu");
        }
        if collection::APPS.iter().any(|a|a.2==name) || name=="Morph" {
            // Actual engine input, explicitly named as a verification tone.
            // Empty media libraries remain empty; no fabricated tracks or meters.
            if app.slint_rows().first().is_some_and(|r|r.0.starts_with("Source")) { app.tick(&Input{knob2:1,..Default::default()});
                if route_source.is_some(){let steps=preview_audio.index_of("Bloom").unwrap();for _ in 0..steps{app.tick(&Input{knob2:1,..Default::default()});}} }
            if ["Orbit","Swarm","Mutant","Constellation","Dream","Fracture","Ghosts","Tape Machine"].contains(&name) {app.toggle_running();}
        }
        if name == "Bloom" {
            assert_eq!(app.running(), Some(false));
            app.toggle_running();
            assert_eq!(app.running(), Some(true));
        }
        if name == "Portal" {
            // Patch the actual verification source into an audio send, then a real LFO rate.
            for (cable,source,target) in [(0,4,1),(1,1,7),(2,2,2)] {
                app.slint_pointer_pick(cable as f32,0.);
                app.tick(&Input{navigation_steps:1-app.slint_selected() as i32,..Default::default()});
                for _ in 0..source {app.tick(&Input{knob2:1,..Default::default()});}
                app.tick(&Input{navigation_steps:1,..Default::default()});
                for _ in 0..target {app.tick(&Input{knob2:1,..Default::default()});}
            }
            app.slint_pointer_pick(0.,0.);
            app.tick(&Input{navigation_steps: - (app.slint_selected() as i32),..Default::default()});
        }
        if name=="Vector Filter" {app.tick(&Input{knob2:1,..Default::default()});app.slint_pointer_pick(0.67,0.58);app.slint_pointer_pick(1000.6,0.);}
        if name=="Scope" {let row=app.slint_rows().iter().position(|r|r.0=="Display mode").unwrap();app.tick(&Input{navigation_steps:row as i32-app.slint_selected() as i32,knob2:std::env::var("PORTAMAX_SCOPE_MODE").ok().and_then(|s|s.parse().ok()).unwrap_or(1),..Default::default()});}
        assert_eq!(preview_audio.names(),catalog_names,"Opening {name} must claim its declared ports, not create new ones");
        let mut processor = app.audio_processor();
        let mut buffer = [0.0f32; 1024];
        for frame in 0..(if route_source.is_some(){1200} else if name == "Bloom" { 413 } else if name=="Ghosts" {900} else { 100 }) {
            if name=="Scope" {
                *tone.lock().unwrap()=(0..512).map(|n|(0..8).map(|band|{
                    let frequency=180.+band as f32*980.+(frame as f32*0.035+band as f32).sin()*140.;
                    let envelope=0.3+0.7*(frame as f32*0.045+band as f32*1.3).sin().abs();
                    (std::f32::consts::TAU*frequency*(n+frame*512) as f32/48000.).sin()*0.06*envelope
                }).sum::<f32>()).collect();
            }
            if let Some((source,dsp))=route_source.as_mut(){source.tick(&Input::default());if let Some(dsp)=dsp{dsp.process(&mut buffer,2,48000.);}}
            let mut input = Input::default();
            input.grid[0] = name != "Scope";
            if name=="Swarm" {for i in 0..8 {input.grid[i]=true;}}
            app.tick(&input);
            if let Some(processor) = processor.as_mut() {
                buffer.fill(0.0);
                processor.process(&mut buffer, 2, 48000.0);
            }
            if name=="Scope" {let view=app.slint_extra();if route_source.is_some() && frame>100 {if let app::SlintExtra::Collection(c)=view {if c.peak>0.01 {break;}}}}
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
        // A full-screen app (the Kids apps) has no menu column to tap.
        if name != "Synth" && app.slint_rows().len() > 1 && !matches!(app.slint_extra(), app::SlintExtra::Screen(_)) {
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
        if has_play_view && std::env::var("PORTAMAX_MENU").is_err() {
            app.tick(&Input { shoulder_press: [false, true], ..Default::default() });
            let mut input = Input::default();
            input.grid[0] = true;
            app.tick(&input);
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
        apply_scale_visual(&ui, app.slint_scale_info());
        apply_play_column(&ui, app.play_column());
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
        // Everything below drives the menu again.
        if has_play_view {
            app.tick(&Input { shoulder_press: [false, true], ..Default::default() });
            apply_play_column(&ui, app.play_column());
        }
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
        if ["Orbit","Swarm","Mutant","Constellation","Dream"].contains(&name) {
            let last=app.slint_rows().len()-1;
            app.tick(&Input {navigation_steps: last as i32 - app.slint_selected() as i32, ..Default::default()});
            let (rows,selected,above,below)=app.slint_windowed_rows(10);
            assert!(rows.len()<=10 && above && !below);
            ui.set_row_names(Rc::new(slint::VecModel::from(rows.iter().map(|r|r.0.clone().into()).collect::<Vec<slint::SharedString>>())).into());
            ui.set_row_values(Rc::new(slint::VecModel::from(rows.iter().map(|r|r.1.clone().into()).collect::<Vec<slint::SharedString>>())).into());
            ui.set_selected_row(selected as i32); ui.set_more_above(above); ui.set_more_below(below);
            save_extra_frame(&window,directory,&format!("{slug}-instruments"));
        }
        if ["Orbit","Swarm","Mutant","Constellation","Dream","Plaits","Bloom","Madness","Nebula","Pam's Workout"].contains(&name) {
            for _ in 0..64 {
                let rows=app.slint_rows();
                if let Some(scale)=rows.iter().position(|r|r.0.trim()=="Main scale") {
                    app.tick(&Input {navigation_steps:scale as i32-app.slint_selected() as i32,..Default::default()});
                    apply_scale_visual(&ui,app.slint_scale_info());
                    assert!(ui.get_scale_visible(),"{} scale view missing",name);
                    let (rows,selected,above,below)=app.slint_windowed_rows(10);
                    ui.set_row_names(Rc::new(slint::VecModel::from(rows.iter().map(|r|r.0.clone().into()).collect::<Vec<slint::SharedString>>())).into());
                    ui.set_row_values(Rc::new(slint::VecModel::from(rows.iter().map(|r|r.1.clone().into()).collect::<Vec<slint::SharedString>>())).into());
                    ui.set_selected_row(selected as i32);ui.set_more_above(above);ui.set_more_below(below);
                    save_extra_frame(&window,directory,&format!("{slug}-scale")); break;
                }
                if name=="Pam's Workout" {
                    if let Some(q)=rows.iter().position(|r|r.0.trim()=="Quantizer") {
                        app.tick(&Input{navigation_steps:q as i32-app.slint_selected() as i32,..Default::default()});
                        app.tick(&Input{knob2:1,..Default::default()});continue;
                    }
                }
                let closed=rows.iter().position(|r|r.2 && (r.0.trim().starts_with('>') || r.0.trim().starts_with('▸')));
                if let Some(group)=closed {app.tick(&Input{navigation_steps:group as i32-app.slint_selected() as i32,..Default::default()});app.tick(&Input{knob1_press:true,..Default::default()});} else {break;}
            }
            apply_scale_visual(&ui,None);
        }
        if ["Vector Filter","Scope","Swarm"].contains(&name) {
            if name=="Swarm" {app.tick(&Input{navigation_steps: -(app.slint_selected() as i32),..Default::default()});}
            apply_scale_visual(&ui,None);
            for frame in 0..12 {
                if name=="Vector Filter" {app.slint_pointer_pick(0.15+frame as f32*0.065,0.5+(frame as f32*0.5).sin()*0.35);app.slint_pointer_pick(1000.+frame as f32/12.,0.);}
                for block in 0..8 {
                    if name=="Scope" {*tone.lock().unwrap()=(0..512).map(|n|(0..8).map(|band|{let hz=180.+band as f32*980.+(frame as f32*0.23+band as f32).sin()*260.;let env=0.3+0.7*(frame as f32*0.4+band as f32).sin().abs();(std::f32::consts::TAU*hz*(n+(frame*8+block)*512) as f32/48000.).sin()*0.06*env}).sum::<f32>()).collect();}
                    let mut input=Input::default();if name=="Swarm" {for i in 0..8 {input.grid[i]=i%3!=frame%3;}}app.tick(&input);
                    if let Some(p)=processor.as_mut(){p.process(&mut buffer,2,48000.);}
                    if name=="Scope" {let _=app.slint_extra();}
                }
                let(rows,selected,above,below)=app.slint_windowed_rows(10);
                ui.set_row_names(Rc::new(slint::VecModel::from(rows.iter().map(|r|slint::SharedString::from(r.0.as_str())).collect::<Vec<_>>())).into());
                ui.set_row_values(Rc::new(slint::VecModel::from(rows.iter().map(|r|slint::SharedString::from(r.1.as_str())).collect::<Vec<_>>())).into());
                ui.set_selected_row(selected as i32);ui.set_more_above(above);ui.set_more_below(below);apply_instrument_visual(&ui,app.slint_extra());
                save_extra_frame(&window,directory,&format!("{slug}-motion-{frame:02}"));
            }
        }
        if name=="Settings" {
            for (section,slug) in [(0,"settings-output"),(1,"settings-input"),(2,"settings-controls"),(3,"settings-appearance")] {
                app.slint_pointer_pick(1000.+section as f32,0.);
                if section==2 {app.tick(&Input{navigation_steps:1,..Default::default()});}
                let (rows,selected,above,below)=app.slint_windowed_rows(10);
                ui.set_row_names(Rc::new(slint::VecModel::from(rows.iter().map(|r|slint::SharedString::from(r.0.as_str())).collect::<Vec<_>>())).into());
                ui.set_row_values(Rc::new(slint::VecModel::from(rows.iter().map(|r|slint::SharedString::from(r.1.as_str())).collect::<Vec<_>>())).into());
                ui.set_row_is_group(Rc::new(slint::VecModel::from(rows.iter().map(|r|r.2).collect::<Vec<_>>())).into());
                ui.set_selected_row(selected as i32);ui.set_more_above(above);ui.set_more_below(below);
                apply_instrument_visual(&ui,app.slint_extra());save_extra_frame(&window,directory,slug);
            }
        }
        app.on_exit();
    }
    verify_console_controls(&ui, &clock);
    ui.set_on_home(true);
    ui.set_pad_lock_available(false);
    ui.set_transport_action("".into());
    ui.set_live_bg(slint::Color::from_rgb_u8(18,27,27));ui.set_live_ink(slint::Color::from_rgb_u8(241,240,230));ui.set_live_accent(slint::Color::from_rgb_u8(183,214,197));
    let names:Vec<String>=manifests.iter().map(|m|m.name.clone()).collect();
    launcher::install_catalog(manifests.iter().map(|m|(m.name.clone(),m.category.clone(),m.description.clone())));
    let mut browser=launcher::Launcher::default();
    for (category,slug) in [(0,"home"),(1,"home-instruments"),(6,"home-ai"),(7,"home-kids"),(launcher::RECENT,"home-recent-empty")] {
        browser.category=category;
        let ids=browser.indices(&names);let visible:Vec<_>=ids.iter().copied().take(6).collect();
        ui.set_home_names(Rc::new(slint::VecModel::from(visible.iter().map(|i|slint::SharedString::from(names[*i].as_str())).collect::<Vec<_>>())).into());
        ui.set_home_ids(Rc::new(slint::VecModel::from(visible.iter().map(|i|*i as i32).collect::<Vec<_>>())).into());
        ui.set_home_running(Rc::new(slint::VecModel::from(vec![false;visible.len()])).into());
        ui.set_home_category(category as i32);ui.set_home_count(ids.len() as i32);ui.set_home_total(names.len() as i32);
        let name=visible.first().map(|i|names[*i].as_str()).unwrap_or("");
        ui.set_home_title(name.into());ui.set_home_description(if name.is_empty(){"Open an app to add it to your recent list.".to_string()}else{launcher::description(name)}.into());
        ui.set_home_family(if name.is_empty(){"WELCOME"}else{launcher::CATEGORIES[launcher::category(name)]}.into());
        ui.set_home_selected(0); ui.set_home_more_above(false); ui.set_home_more_below(ids.len()>6);
        save_extra_frame(&window,directory,slug);
    }
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

fn isolated_registry() -> (Registry, Arc<ModBus>) {registry_with_audio(Arc::new(AudioBus::new()))}
fn registry_with_audio(audio_bus:Arc<AudioBus>) -> (Registry, Arc<ModBus>) {
    let sensitivity = Arc::new(AtomicF32::new(0.1));
    let nav = Arc::new(AtomicF32::new(3.0));
    let preview_modbus = Arc::new(ModBus::new());
    let registry = Registry::new(registry::standard_context(
        Arc::new(AtomicF32::new(1000.0)),
        Arc::new(audio_devices::AudioDeviceState::new("Offline preview".into())),
        sensitivity, nav, Arc::clone(&preview_modbus), audio_bus,
        Arc::new(AtomicF32::new(1.0)), Arc::new(MixerBus::new()),
        Arc::new(prism::PrismCcTargets::new()),
        Arc::new(std::sync::atomic::AtomicBool::new(false)), Arc::new(midi_map::MidiMap::new()),
        Arc::new(theme::ThemeColor::new(theme::ACCENT_DEFAULT_HUE, theme::ACCENT_DEFAULT_SAT, theme::ACCENT_DEFAULT_VAL)),
        Arc::new(theme::ThemeColor::new(theme::BG_DEFAULT_HUE, theme::BG_DEFAULT_SAT, theme::BG_DEFAULT_VAL)),
        Arc::new(note_bus::NoteBus::new()),
    ));
    (registry, preview_modbus)
}
