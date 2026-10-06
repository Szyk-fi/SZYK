use super::*;
use crate::app::MidiKeys;
fn app() -> ProphetApp {
    ProphetApp::with_dir(
        Arc::new(ModBus::new()),
        Arc::new(AudioBus::new()),
        Arc::new(MixerBus::new()),
        Path::new("/tmp/portamax-prophet-no-user-bank"),
    )
}
fn input(notes: &[(usize, u8)]) -> Input {
    let mut k = [0; 128];
    for &(n, v) in notes {
        k[n] = v;
    }
    Input {
        midi_keys: MidiKeys(k),
        ..Input::default()
    }
}
fn render(p: &mut dyn AudioProcessor, seconds: f32, sr: f32) -> Vec<f32> {
    let n = (seconds * sr) as usize;
    let mut result = Vec::with_capacity(n * 2);
    let mut block = [0.; 1024];
    while result.len() < n * 2 {
        p.process(&mut block, 2, sr);
        result.extend_from_slice(&block);
    }
    result.truncate(n * 2);
    result
}
fn energy(x: &[f32]) -> f32 {
    x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32
}
#[test]
fn pads_and_keyboard_produce_audio_and_release_to_silence() {
    let mut a = app();
    let mut p = a.audio_processor().unwrap();
    assert_eq!(energy(&render(p.as_mut(), 0.02, 48000.)), 0.);
    let mut i = Input::default();
    i.grid[12] = true;
    a.tick(&i);
    assert!(energy(&render(p.as_mut(), 0.4, 48000.)) > 0.00001);
    a.tick(&Input::default());
    let tail = render(p.as_mut(), 2., 48000.);
    assert!(energy(&tail[tail.len() - 4096..]) < 1e-9);
    a.tick(&input(&[(69, 110)]));
    assert!(energy(&render(p.as_mut(), 0.2, 44100.)) > 0.00001);
    a.on_exit();
    let tail = render(p.as_mut(), 2., 44100.);
    assert!(energy(&tail[tail.len() - 4096..]) < 1e-9);
}
#[test]
fn all_64_factory_presets_play_finite_audio_at_device_rates() {
    let mut a = app();
    assert_eq!(a.factory.len(), 64);
    let names: std::collections::HashSet<_> = a.factory.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names.len(), 64);
    for sr in [32000., 44100., 48000., 96000.] {
        for index in 0..64 {
            a.load(index);
            let mut p = a.audio_processor().unwrap();
            a.tick(&input(&[(60, 110), (64, 95), (67, 105)]));
            let audio = render(
                p.as_mut(),
                if a.factory[index].bank == "STRINGS" {
                    1.0
                } else {
                    0.35
                },
                sr,
            );
            assert!(
                audio.iter().all(|v| v.is_finite() && v.abs() <= 1.),
                "{} at {sr}",
                a.name()
            );
            assert!(energy(&audio) > 1e-9, "silent {} at {sr}", a.name());
            a.tick(&Input::default());
        }
    }
}
#[test]
fn polyphony_is_bounded_and_a_stolen_note_does_not_release_a_new_one() {
    let mut a = app();
    a.load(63);
    a.shared.controls.set(VOICES, 0.);
    a.shared.controls.set(AR, time_norm(0.02));
    let mut p = a.audio_processor().unwrap();
    a.tick(&input(&[(60, 100), (61, 100), (62, 100), (63, 100)]));
    render(p.as_mut(), 0.08, 48000.);
    a.tick(&input(&[
        (60, 100),
        (61, 100),
        (62, 100),
        (63, 100),
        (72, 110),
    ]));
    render(p.as_mut(), 0.08, 48000.);
    assert_eq!(a.shared.meters.iter().filter(|v| v.get() > 0.01).count(), 4);
    a.tick(&input(&[(61, 100), (62, 100), (63, 100), (72, 110)]));
    assert!(energy(&render(p.as_mut(), 0.12, 48000.)) > 0.0001);
    a.tick(&Input::default());
    let tail = render(p.as_mut(), 0.5, 48000.);
    assert!(energy(&tail[tail.len() - 4096..]) < 1e-10);
}
#[test]
fn mono_returns_to_the_previously_held_note() {
    let mut a = app();
    a.load(63);
    a.shared.controls.set(MODE, 1.);
    let mut p = a.audio_processor().unwrap();
    a.tick(&input(&[(60, 100)]));
    render(p.as_mut(), 0.1, 48000.);
    a.tick(&input(&[(60, 100), (67, 100)]));
    render(p.as_mut(), 0.1, 48000.);
    a.tick(&input(&[(60, 100)]));
    let returned = render(p.as_mut(), 0.15, 48000.);
    let mut reference = app();
    reference.load(63);
    reference.shared.controls.set(MODE, 1.);
    reference.tick(&input(&[(60, 100)]));
    let mut q = reference.audio_processor().unwrap();
    let ref_audio = render(q.as_mut(), 0.15, 48000.);
    // Correlate near C4 rather than comparing oscillator phase.
    let tone = |audio: &[f32], freq: f32| {
        let mut re = 0.;
        let mut im = 0.;
        let len = audio.len() / 2;
        for (i, f) in audio.chunks_exact(2).enumerate() {
            let phase = std::f32::consts::TAU * freq * i as f32 / 48000.;
            re += f[0] * phase.cos();
            im += f[0] * phase.sin();
        }
        (re * re + im * im).sqrt() / len as f32
    };
    assert!(tone(&returned, 261.626) > tone(&returned, 391.995) * 3.);
    assert!(tone(&ref_audio, 261.626) > 0.01);
}
#[test]
fn sync_polymod_pwm_and_filter_controls_change_the_sound() {
    fn sound(pairs: &[(usize, f32)]) -> Vec<f32> {
        let mut a = app();
        a.load(63);
        for &(i, v) in pairs {
            a.shared.controls.set(i, v);
        }
        a.tick(&input(&[(60, 100)]));
        let mut p = a.audio_processor().unwrap();
        render(p.as_mut(), 0.15, 48000.)
    }
    for (before, after) in [
        (vec![(A_TUNE, 7.)], vec![(A_TUNE, 7.), (SYNC, 1.)]),
        (vec![], vec![(POLY_B, 0.4), (P_FREQ, 1.)]),
        (
            vec![(A_SAW, 0.), (A_PULSE, 1.), (A_PW, 0.5)],
            vec![(A_SAW, 0.), (A_PULSE, 1.), (A_PW, 0.2)],
        ),
        (vec![], vec![(CUTOFF, cutoff_norm(300.)), (RES, 0.6)]),
    ] {
        let x = sound(&before);
        let y = sound(&after);
        let delta: Vec<_> = x.iter().zip(&y).map(|(x, y)| x - y).collect();
        assert!(energy(&delta) > 0.00001, "control must change real audio");
    }
}
#[test]
fn compare_restores_edits_and_saving_roundtrips_all_parameters() {
    let dir = std::env::temp_dir().join(format!("portamax-prophet-test-{}", std::process::id()));
    let make = || {
        ProphetApp::with_dir(
            Arc::new(ModBus::new()),
            Arc::new(AudioBus::new()),
            Arc::new(MixerBus::new()),
            &dir,
        )
    };
    let mut a = make();
    let factory_before = a.factory[0].values;
    a.shared.controls.set(CUTOFF, 0.28);
    a.shared.controls.set(POLY_B, 0.22);
    let edits = a.values();
    a.compare();
    assert_eq!(a.values(), a.baseline);
    a.compare();
    assert_eq!(a.values(), edits);
    a.user_slot = 7;
    a.save().unwrap();
    assert_eq!(a.factory[0].values, factory_before);
    let mut restored = make();
    restored.load(71);
    assert_eq!(restored.values(), edits);
    assert!(!restored.dirty());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn malformed_patches_are_rejected_without_partial_application() {
    let a = app();
    let mut p = StoredPatch::from_preset(&a.factory[0]);
    p.version = 2;
    assert!(p.preset().is_err());
    let mut p = StoredPatch::from_preset(&a.factory[0]);
    p.parameters.remove("A Saw");
    assert!(p.preset().is_err());
    let mut p = StoredPatch::from_preset(&a.factory[0]);
    p.parameters.insert("A Saw".into(), 0.5);
    assert!(p.preset().is_err());
    let mut p = StoredPatch::from_preset(&a.factory[0]);
    p.parameters.insert("Cutoff".into(), f32::NAN);
    assert!(p.preset().is_err());
}
#[test]
fn touch_keyboard_merges_with_midi_and_octave_changes_release_old_pads() {
    let mut a = app();
    a.tick(&input(&[(60, 100)]));
    a.slint_pointer_pick(30., 320.);
    assert_eq!(a.shared.keys.lock().unwrap().notes[48], 100);
    a.slint_pointer_pick(-1., -1.);
    assert_eq!(a.shared.keys.lock().unwrap().notes[60], 100);
    assert_eq!(a.shared.keys.lock().unwrap().notes[48], 0);
    let mut i = Input::default();
    i.grid[12] = true;
    a.tick(&i);
    assert!(a.shared.keys.lock().unwrap().notes[48] > 0);
    a.shared.controls.set(OCTAVE, 1.);
    a.tick(&i);
    assert_eq!(a.shared.keys.lock().unwrap().notes[48], 0);
    assert!(a.shared.keys.lock().unwrap().notes[60] > 0);
    a.view = 1;
    a.publish_keys();
    assert!(a.shared.keys.lock().unwrap().notes.iter().all(|&v| v == 0));
}
#[test]
fn mod_bus_and_mixer_change_output_and_audio_bus_is_published() {
    let mods = Arc::new(ModBus::new());
    let bus = Arc::new(AudioBus::new());
    let mixer = Arc::new(MixerBus::new());
    let mut a = ProphetApp::with_dir(
        mods.clone(),
        bus.clone(),
        mixer.clone(),
        Path::new("/tmp/portamax-prophet-no-user-bank"),
    );
    a.load(63);
    a.tick(&input(&[(60, 100)]));
    let mut p = a.audio_processor().unwrap();
    let loud = energy(&render(p.as_mut(), 0.1, 48000.));
    assert!(loud > 0.);
    let index = mods.index_of("Prophet: Cutoff").unwrap();
    mods.get(index).unwrap().set(-0.85);
    let dark = energy(&render(p.as_mut(), 0.2, 48000.));
    assert!(dark < loud * 0.7);
    let index = bus.index_of("Prophet").unwrap();
    assert!(bus
        .peek(index)
        .unwrap()
        .lock()
        .unwrap()
        .iter()
        .any(|x| x.abs() > 1e-6));
    mixer.level(0).unwrap().set(0.);
    assert_eq!(energy(&render(p.as_mut(), 0.1, 48000.)), 0.);
}
#[test]
fn ui_contentions_and_oversized_callbacks_do_not_block_or_allocate_output() {
    let mut a = app();
    let mut p = a.audio_processor().unwrap();
    a.tick(&input(&[(60, 100)]));
    render(p.as_mut(), 0.1, 48000.);
    let held = a.shared.keys.lock().unwrap();
    let bus = a.shared.output.lock().unwrap();
    let mut b = [0.; 1024];
    p.process(&mut b, 2, 48000.);
    assert!(energy(&b) > 0.);
    drop(held);
    drop(bus);
    let capacity = a.shared.output.lock().unwrap().capacity();
    let mut b = vec![0.; capacity * 4];
    p.process(&mut b, 2, 48000.);
    assert_eq!(a.shared.output.lock().unwrap().capacity(), capacity);
    p.process(&mut b, 0, 48000.);
    assert!(b.iter().all(|&v| v == 0.));
    p.process(&mut b, 2, f32::NAN);
    assert!(b.iter().all(|&v| v == 0.));
}
#[test]
fn pointer_drag_edits_a_dial_and_bank_selection_loads_a_sound() {
    let mut a = app();
    a.select_group(3);
    let before = a.shared.controls.norm(CUTOFF);
    a.slint_pointer_pick(99., 151.);
    a.slint_pointer_pick(99., 131.);
    a.slint_pointer_pick(-1., -1.);
    assert!(a.shared.controls.norm(CUTOFF) > before);
    a.view = 1;
    a.bank = 2;
    a.slint_pointer_pick(60., 145.);
    a.slint_pointer_pick(-1., -1.);
    assert_eq!(a.program, 16);
}
#[test]
fn screenshot_pixels_are_identical_to_slint_screen() {
    let mut a = app();
    let mut fb = FrameBuffer::new();
    a.draw(&mut fb);
    let SlintExtra::Screen(s) = a.slint_extra() else {
        panic!("fullscreen app must provide its picture");
    };
    assert_eq!(s.frame_rgba, crate::apps::kids_kit::frame_rgba(&fb));
}

#[test]
fn a_sequencer_can_play_the_unopened_app_through_the_real_registry() {
    use crate::{app::AppContext, note_bus::NoteBus, registry::Registry};
    let mods = Arc::new(ModBus::new());
    let bus = Arc::new(AudioBus::new());
    let notes = Arc::new(NoteBus::new());
    let mut ctx = AppContext::new();
    ctx.provide(mods.clone())
        .provide(bus.clone())
        .provide(notes.clone())
        .provide(Arc::new(MixerBus::new()));
    let manifests: Vec<_> =
        crate::manifest::discover(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/apps")))
            .into_iter()
            .filter(|m| m.id == "prophet")
            .collect();
    assert_eq!(manifests.len(), 1);
    let registry = Registry::new(ctx);
    let mut apps = registry.build(&manifests);
    let instrument = notes
        .instrument_index("Prophet")
        .expect("Prophet must appear in Plays before it opens");
    let out = bus.index_of("Prophet").unwrap();
    assert!(!bus.is_claimed(out));
    assert!(mods.index_of("Prophet: Poly-Mod B").is_some());
    let mut source = notes.register_source("test sequencer");
    source.set_route(instrument);
    source.note_on(60, 110);
    let mut processor = apps[0].1.audio_processor().unwrap();
    apps[0].1.background_tick();
    assert!(bus.is_claimed(out), "incoming notes must wake the lazy app");
    assert!(energy(&render(processor.as_mut(), 0.2, 48000.)) > 0.00001);
    source.note_off(60);
    apps[0].1.background_tick();
    let tail = render(processor.as_mut(), 2., 48000.);
    assert!(energy(&tail[tail.len() - 4096..]) < 1e-9);
    assert_eq!(mods.names(), manifests[0].mod_inputs);
}
#[test]
#[ignore = "Writes preview PNGs, preset JSON and an audio demo when PORTAMAX_PROPHET_EXPORT is set"]
fn export_previews_and_demo() {
    let dir = PathBuf::from(std::env::var("PORTAMAX_PROPHET_EXPORT").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut a = app();
    for (name, view, group, program) in [
        ("Prophet-Panel", 0, 3, 0),
        ("Prophet-Oscillators", 0, 1, 0),
        ("Prophet-Envelopes", 0, 4, 8),
        ("Prophet-Poly-Mod", 0, 5, 40),
        ("Prophet-Performance", 0, 7, 24),
        ("Prophet-Presets", 1, 3, 0),
    ] {
        a.load(program);
        a.view = view;
        a.select_group(group);
        let mut fb = FrameBuffer::new();
        a.draw(&mut fb);
        let path = dir.join(format!("{name}.png"));
        let mut encoder = png::Encoder::new(std::fs::File::create(path).unwrap(), 640, 360);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&crate::apps::kids_kit::frame_rgba(&fb))
            .unwrap();
    }
    std::fs::create_dir_all(dir.join("factory-presets")).unwrap();
    for (i, p) in a.factory.iter().enumerate() {
        let path = dir.join("factory-presets").join(format!(
            "{:02}-{}.json",
            i + 1,
            p.name.replace(' ', "-")
        ));
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&StoredPatch::from_preset(p)).unwrap(),
        )
        .unwrap();
    }
    let mut wav = hound::WavWriter::create(
        dir.join("Prophet-Preset-Demo.wav"),
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for program in [0, 8, 16, 25, 32, 40, 48, 63] {
        a.load(program);
        a.view = 0;
        let mut p = a.audio_processor().unwrap();
        for notes in [
            vec![(60, 100), (64, 90), (67, 100)],
            vec![(57, 100), (60, 95), (64, 100)],
        ] {
            a.tick(&input(&notes));
            for s in render(p.as_mut(), 1.4, 48000.) {
                wav.write_sample((s.clamp(-1., 1.) * 32767.) as i16)
                    .unwrap();
            }
            a.tick(&Input::default());
            for s in render(p.as_mut(), 0.6, 48000.) {
                wav.write_sample((s.clamp(-1., 1.) * 32767.) as i16)
                    .unwrap();
            }
        }
    }
    wav.finalize().unwrap();
}
