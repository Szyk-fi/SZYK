use super::*;
fn app() -> BebotApp {
    BebotApp::new(
        Arc::new(AudioBus::new()),
        Arc::new(MixerBus::new()),
        Arc::new(ModBus::new()),
    )
}
fn render(p: &mut Box<dyn AudioProcessor>, seconds: f32) -> Vec<f32> {
    let mut all = Vec::new();
    let mut b = vec![0.; 512];
    for _ in 0..(seconds * 48000. / 256.).ceil() as usize {
        p.process(&mut b, 2, 48000.);
        all.extend_from_slice(&b);
    }
    all
}
fn rms(b: &[f32]) -> f32 {
    (b.iter().map(|x| x * x).sum::<f32>() / b.len() as f32).sqrt()
}
#[test]
fn drag_sings_changes_pitch_and_release_stops() {
    let mut a = app();
    a.state.patch.v[13] = 0.;
    a.state.patch.v[14] = 0.;
    let mut p = a.audio_processor().unwrap();
    a.slint_pointer_pick(100., 120.);
    let low = a.state.notes[0];
    assert!(rms(&render(&mut p, 0.25)) > 0.02);
    a.slint_pointer_pick(550., 120.);
    assert!(a.state.notes[0] > low + 12.);
    assert_eq!(a.state.gates[0], 1.);
    a.slint_pointer_pick(-1., -1.);
    let b = render(&mut p, 2.);
    assert!(rms(&b[b.len() - 2048..]) < 0.0001);
}
#[test]
fn toolbar_crossing_does_not_change_mode_or_preset_mid_drag() {
    let mut a = app();
    a.slint_pointer_pick(400., 140.);
    a.slint_pointer_pick(340., 10.);
    assert_eq!(a.state.patch.v[0], 0.);
    assert_eq!(a.preset, 0);
    assert!(a.pointer.is_some());
    a.slint_pointer_pick(-1., -1.);
    assert!(a.pointer.is_none());
}
#[test]
fn leaving_releases_mouse_and_keys() {
    let mut a = app();
    a.slint_pointer_pick(200., 150.);
    a.tick(&Input {
        grid: [true; 16],
        ..Default::default()
    });
    a.on_exit();
    assert!(a.state.gates.iter().all(|x| *x == 0.));
    assert!(a.pointer.is_none());
}
#[test]
fn snap_lands_in_selected_key() {
    let mut patch = Patch::default();
    patch.v[1] = 1.;
    patch.v[3] = 2.;
    for n in 24..110 {
        let snapped = dsp::snap(n as f32 + 0.3, patch) as i32;
        assert!([0, 2, 4, 5, 7, 9, 11].contains(&(snapped - 2).rem_euclid(12)));
    }
}
#[test]
fn every_factory_voice_and_chord_is_finite_audible_and_bounded() {
    for preset in 0..8 {
        let mut a = app();
        a.load_factory(preset);
        let mut p = a.audio_processor().unwrap();
        a.tick(&Input {
            grid: [true; 16],
            pad_pressure: [0.7; 16],
            ..Default::default()
        });
        let b = render(&mut p, 0.6);
        assert!(rms(&b) > 0.005, "preset {preset}");
        assert!(b.iter().all(|x| x.is_finite() && x.abs() <= 1.));
    }
}
#[test]
fn sine_pitch_is_440_hz_without_drift() {
    let mut a = app();
    a.state.patch.v[0] = 2.;
    a.state.patch.v[2] = 0.;
    a.state.patch.v[13] = 0.;
    a.state.patch.v[14] = 0.;
    a.state.patch.v[11] = 0.;
    a.state.notes[0] = 69.;
    a.state.y[0] = 1.;
    a.state.gates[0] = 1.;
    a.publish();
    let mut p = a.audio_processor().unwrap();
    let b = render(&mut p, 1.);
    let mono: Vec<f32> = b.chunks(2).map(|f| f[0]).collect();
    let crossings = mono[12000..]
        .windows(2)
        .filter(|s| s[0] <= 0. && s[1] > 0.)
        .count();
    let frequency = crossings as f32 * 48000. / (mono.len() - 12000) as f32;
    assert!((frequency - 440.).abs() < 2., "{frequency}");
}
#[test]
fn vertical_drag_changes_filter_brightness() {
    let mut a = app();
    a.state.patch.v[13] = 0.;
    a.state.patch.v[14] = 0.;
    a.state.patch.v[6] = 0.;
    let mut p = a.audio_processor().unwrap();
    a.slint_pointer_pick(300., 325.);
    let dull = render(&mut p, 0.4);
    a.slint_pointer_pick(300., 50.);
    let bright = render(&mut p, 0.4);
    let energy = |b: &[f32]| {
        b[b.len() / 2..]
            .windows(3)
            .map(|w| (w[2] - w[0]).powi(2))
            .sum::<f32>()
    };
    assert!(energy(&bright) > energy(&dull) * 3.);
}
#[test]
fn slint_frame_is_the_actual_fullscreen_picture() {
    let mut a = app();
    let SlintExtra::Screen(s) = a.slint_extra() else {
        panic!("missing Slint screen")
    };
    assert_eq!((s.width, s.height), (640, 360));
    assert_eq!(s.frame_rgba.len(), 640 * 360 * 4);
    let mut fb = FrameBuffer::new();
    a.draw(&mut fb);
    let SlintExtra::Screen(t) = kit::screen_extra(&fb) else {
        unreachable!()
    };
    assert_eq!(s.frame_rgba, t.frame_rgba);
}
#[test]
fn mouse_toolbar_opens_working_settings() {
    let mut a = app();
    a.slint_pointer_pick(580., 20.);
    assert!(a.menu);
    a.slint_pointer_pick(-1., -1.);
    let original = a.state.patch.v[0];
    a.slint_pointer_pick(250., 60.);
    assert_eq!(a.state.patch.v[0], original + 1.);
    assert_eq!(a.state.gates[0], 0.);
}
#[test]
fn invalid_pointer_cancels_and_outside_does_not_start() {
    let mut a = app();
    a.slint_pointer_pick(700., 200.);
    assert_eq!(a.state.gates[0], 0.);
    a.slint_pointer_pick(200., 100.);
    a.slint_pointer_pick(f32::NAN, 0.);
    assert_eq!(a.state.gates[0], 0.);
}
#[test]
fn chorus_and_echo_make_stereo_tails_that_decay() {
    let mut a = app();
    a.state.patch.v[13] = 0.8;
    a.state.patch.v[14] = 0.8;
    a.state.patch.v[15] = 0.04;
    a.state.patch.feedback = 0.35;
    a.slint_pointer_pick(350., 100.);
    let mut p = a.audio_processor().unwrap();
    let b = render(&mut p, 0.4);
    assert!(b.chunks(2).any(|f| (f[0] - f[1]).abs() > 0.001));
    a.slint_pointer_pick(-1., -1.);
    let b = render(&mut p, 3.);
    assert!(rms(&b[b.len() - 2048..]) < 0.001);
}
#[test]
fn render_actual_ui_for_review() {
    let Ok(dir) = std::env::var("BEBOT_PREVIEW_DIR") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    let mut a = app();
    for name in ["play", "settings"] {
        if name == "play" {
            a.slint_pointer_pick(490., 100.);
        } else {
            a.slint_pointer_pick(-1., -1.);
            a.menu = true;
        }
        let SlintExtra::Screen(s) = a.slint_extra() else {
            unreachable!()
        };
        let file =
            std::fs::File::create(std::path::Path::new(&dir).join(format!("bebot-{name}.png")))
                .unwrap();
        let mut encoder = png::Encoder::new(file, s.width, s.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&s.frame_rgba)
            .unwrap();
    }
}
#[test]
fn releasing_a_lower_key_does_not_retune_a_held_higher_key() {
    let mut a = app();
    let mut input = Input::default();
    input.midi_keys.0[60] = 100;
    input.midi_keys.0[72] = 100;
    a.tick(&input);
    let slot = a
        .state
        .notes
        .iter()
        .enumerate()
        .find(|(i, n)| *i > 0 && **n == 72.)
        .unwrap()
        .0;
    input.midi_keys.0[60] = 0;
    a.tick(&input);
    assert_eq!(a.state.notes[slot], 72.);
    assert!(a.state.gates[slot] > 0.);
}
