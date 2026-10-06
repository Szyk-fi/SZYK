use super::*;

fn app() -> HydraApp {
    HydraApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
}

fn run(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> f32 {
    let mut peak = 0.0f32;
    let mut buf = vec![0.0f32; 512];
    for _ in 0..blocks {
        buf.fill(0.0);
        p.process(&mut buf, 2, 48_000.0);
        peak = peak.max(buf.iter().fold(0.0, |m, s| m.max(s.abs())));
        assert!(buf.iter().all(|s| s.is_finite()), "output must stay finite");
    }
    peak
}

fn pad(i: usize) -> Input {
    Input { grid: std::array::from_fn(|n| n == i), ..Default::default() }
}

#[test]
fn idle_is_silent() {
    let mut a = app();
    let mut p = a.audio_processor().unwrap();
    assert!(run(&mut p, 10) < 1e-4);
}

#[test]
fn a_held_pad_sounds_and_release_goes_quiet() {
    let mut a = app();
    let mut p = a.audio_processor().unwrap();
    a.tick(&pad(0));
    assert!(run(&mut p, 20) > 0.01, "holding a pad must make sound");
    a.tick(&Input::default());
    // generous: the opening preset may have a long release and a reverb tail
    a.sh.params.set(P::AmpR, 0.05);
    a.sh.params.set(P::Rev_Mix, 0.0);
    a.sh.params.set(P::Dly_Mix, 0.0);
    run(&mut p, 400);
    assert!(run(&mut p, 10) < 0.005, "released note must decay to silence");
}

#[test]
fn every_factory_preset_plays_finite_audio() {
    let mut a = app();
    let mut p = a.audio_processor().unwrap();
    for i in 0..a.presets.len() {
        a.load_preset(i);
        a.tick(&pad(5));
        let peak = run(&mut p, 12);
        assert!(peak > 0.0005 || a.presets[i].name == "Init", "preset {} is silent", a.presets[i].name);
        a.tick(&Input::default());
        run(&mut p, 3);
    }
}

#[test]
fn menu_lists_groups_and_expands() {
    let mut a = app();
    let collapsed = a.menu_rows().len();
    assert!(a.menu_rows().iter().any(|r| r.2), "groups are headed rows");
    a.expanded = [true; GROUPS.len()];
    assert!(a.menu_rows().len() > collapsed + 100);
}

#[test]
fn instrument_settings_expose_the_hero_controls() {
    let a = app();
    let names: Vec<String> = a.instrument_settings().into_iter().map(|s| s.label).collect();
    for want in ["Preset", "Cutoff", "Resonance"] {
        assert!(names.iter().any(|n| n == want), "missing {want}");
    }
}

#[test]
fn moments_round_trip_the_whole_patch() {
    let mut a = app();
    a.load_preset(3);
    let snap = a.kit_snapshot();
    let before = a.sh.params.snapshot();
    a.load_preset(5);
    assert_ne!(a.sh.params.snapshot()[..], before[..]);
    a.kit_recall(&snap);
    assert_eq!(a.sh.params.snapshot()[..], before[..]);
}

#[test]
fn arpeggiator_steps_through_held_notes() {
    let mut a = app();
    a.sh.params.set(P::Arp_On, 1.0);
    let mut p = a.audio_processor().unwrap();
    a.tick(&pad(0));
    assert!(run(&mut p, 30) > 0.005);
}

#[test]
fn mono_mode_never_exceeds_one_voice() {
    let mut a = app();
    a.sh.params.set(P::Mode, 1.0);
    let mut p = a.audio_processor().unwrap();
    let both = Input { grid: std::array::from_fn(|n| n < 3), ..Default::default() };
    a.tick(&both);
    run(&mut p, 10);
    assert!(a.sh.active.load(Ordering::Relaxed) <= 1);
}

#[test]
fn many_notes_steal_voices_without_blowing_up() {
    let mut a = app();
    let mut p = a.audio_processor().unwrap();
    let all = Input { grid: [true; 16], ..Default::default() };
    for _ in 0..4 {
        a.tick(&all);
        run(&mut p, 4);
        a.tick(&Input::default());
        run(&mut p, 2);
    }
}

#[test]
fn control_voltage_inputs_exist() {
    let mods = Arc::new(ModBus::new());
    let _a = HydraApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), mods.clone(), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()));
    assert!(mods.names().iter().any(|n| n == "Hydra: Cutoff"));
}
