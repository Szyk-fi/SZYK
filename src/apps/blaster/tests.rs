use super::*;
use crate::app::MidiKeys;

const FS: f32 = 48_000.0;

fn app() -> BlasterApp {
    BlasterApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
}

fn keys(notes: &[u8]) -> Input {
    let mut k = MidiKeys::default();
    for &n in notes {
        k.0[n as usize] = 100;
    }
    Input { midi_keys: k, ..Default::default() }
}

/// Renders `seconds` of audio, returning the left channel.
fn run(p: &mut Box<dyn AudioProcessor>, seconds: f32) -> Vec<f32> {
    let blocks = (seconds * FS / 512.0).round() as usize;
    let mut all = Vec::with_capacity(blocks * 256);
    let mut buf = vec![0.0f32; 1024];
    for _ in 0..blocks {
        buf.fill(0.0);
        p.process(&mut buf, 2, FS);
        assert!(buf.iter().all(|s| s.is_finite()), "output must stay finite");
        all.extend(buf.chunks(2).map(|f| f[0]));
    }
    all
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, s| m.max(s.abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Frequency from zero crossings over a window, for a clean tone.
fn freq(x: &[f32]) -> f32 {
    let mut crossings = 0;
    for w in x.windows(2) {
        if w[0] <= 0.0 && w[1] > 0.0 {
            crossings += 1;
        }
    }
    crossings as f32 * FS / x.len() as f32
}

/// A plain sine charge and a plain sine blast, with nothing else going on.
fn clean(a: &BlasterApp) {
    let p = &a.sh.params;
    p.reset_all();
    p.set(P::ChgWave, 3.0);
    p.set(P::BlastWave, 3.0);
    p.set(P::Body, 0.0);
    p.set(P::Noise, 0.0);
    p.set(P::Punch, 0.0);
    p.set(P::Drive, 0.0);
    p.set(P::KeyFollow, 0.0);
    p.set(P::VelSens, 0.0);
    p.set(P::PowerVolume, 0.0);
}

#[test]
fn idle_is_silent() {
    let mut a = app();
    let mut p = a.audio_processor().unwrap();
    assert!(peak(&run(&mut p, 0.3)) < 1e-4);
}

#[test]
fn holding_a_key_charges_with_a_tone_that_climbs() {
    let mut a = app();
    clean(&a);
    a.sh.params.set(P::ChirpDepth, 0.0);
    a.sh.params.set(P::Climb, 2.0);
    a.sh.params.set(P::ChargeTime, 2.0);
    a.sh.params.set(P::ChgPitch, 300.0);
    a.sh.params.set(P::ChgVol, 1.0);
    a.sh.params.set(P::FullMode, 0.0);
    let mut p = a.audio_processor().unwrap();
    a.tick(&keys(&[60]));
    let first = run(&mut p, 0.4);
    run(&mut p, 0.9);
    let late = run(&mut p, 0.4);
    assert!(peak(&first) > 0.02, "it makes a sound while held");
    // pitch(t) = 300 * 2^(2 t / 2): 0.2 s in is ~345 Hz, 1.5 s in is ~850 Hz
    let (f0, f1) = (freq(&first[..9600]), freq(&late[..9600]));
    assert!(f1 > f0 * 1.9, "the charge climbs: {f0:.0} Hz then {f1:.0} Hz");
    assert!(a.sh.charge.get() > 0.5, "and the meter follows");
}

#[test]
fn the_charge_chirps_faster_as_it_fills() {
    // count the restarts of the chirp by counting the downward pitch jumps
    let mut a = app();
    clean(&a);
    let p = &a.sh.params;
    p.set(P::ChargeTime, 2.0);
    p.set(P::ChirpRate0, 4.0);
    p.set(P::ChirpRate1, 20.0);
    p.set(P::ChirpDepth, 1.5);
    p.set(P::Climb, 0.0);
    p.set(P::FullMode, 0.0);
    p.set(P::ChgVol, 1.0);
    let mut proc = a.audio_processor().unwrap();
    a.tick(&keys(&[60]));
    let early = run(&mut proc, 0.5);
    run(&mut proc, 0.8);
    let late = run(&mut proc, 0.5);
    // Each restart of the chirp is a sudden jump down in pitch, which shows as one
    // period much longer than the one before it; count those.
    let restarts = |x: &[f32]| {
        let mut ups = Vec::new();
        for (i, w) in x.windows(2).enumerate() {
            if w[0] <= 0.0 && w[1] > 0.0 {
                ups.push(i);
            }
        }
        let periods: Vec<usize> = ups.windows(2).map(|w| w[1] - w[0]).collect();
        periods.windows(2).filter(|w| w[1] as f32 > w[0] as f32 * 1.6).count()
    };
    let dips = restarts;
    assert!(dips(&late) > dips(&early) + 2, "more restarts late than early: {} vs {}", dips(&early), dips(&late));
}

#[test]
fn letting_go_fires_a_blast_that_falls_and_dies_away() {
    let mut a = app();
    clean(&a);
    let p = &a.sh.params;
    p.set(P::BlastStart, 2000.0);
    p.set(P::BlastEnd, 200.0);
    p.set(P::SweepTime, 0.3);
    p.set(P::Length, 0.5);
    p.set(P::Attack, 0.001);
    p.set(P::PowerLength, 0.0);
    p.set(P::PowerPitch, 0.0);
    p.set(P::ChargeTime, 0.3);
    let mut proc = a.audio_processor().unwrap();
    a.tick(&keys(&[60]));
    run(&mut proc, 0.5);
    a.tick(&keys(&[]));
    let blast = run(&mut proc, 1.5);
    assert!(peak(&blast[..4800]) > 0.1, "there is a blast right after release");
    let (early, late) = (freq(&blast[200..1600]), freq(&blast[9600..11000]));
    assert!(early > late * 2.5, "the pitch falls: {early:.0} Hz then {late:.0} Hz");
    assert!(peak(&blast[blast.len() - 4800..]) < 0.005, "and it dies away");
    assert_eq!(a.sh.active.load(Ordering::Relaxed), 0, "the voice is free again");
}

#[test]
fn a_bigger_charge_fires_a_bigger_blast_and_a_tap_a_small_one() {
    let shot = |hold: f32| {
        let mut a = app();
        clean(&a);
        a.sh.params.set(P::ChargeTime, 1.0);
        a.sh.params.set(P::Length, 0.2);
        a.sh.params.set(P::PowerLength, 3.0);
        a.sh.params.set(P::PowerVolume, 0.6);
        let mut proc = a.audio_processor().unwrap();
        a.tick(&keys(&[60]));
        run(&mut proc, hold);
        a.tick(&keys(&[]));
        let blast = run(&mut proc, 3.0);
        let audible = blast.iter().rposition(|s| s.abs() > 0.01).unwrap_or(0) as f32 / FS;
        (peak(&blast), audible)
    };
    let (tap_peak, tap_len) = shot(0.05);
    let (mid_peak, mid_len) = shot(0.5);
    let (full_peak, full_len) = shot(1.2);
    assert!(tap_peak < mid_peak && mid_peak < full_peak, "louder with charge: {tap_peak:.2} {mid_peak:.2} {full_peak:.2}");
    assert!(tap_len < mid_len && mid_len < full_len, "longer with charge: {tap_len:.2} {mid_len:.2} {full_len:.2}");
    assert!(tap_peak > 0.02, "a tap still fires something");
}

#[test]
fn fire_at_full_shoots_without_a_release() {
    let mut a = app();
    clean(&a);
    a.sh.params.set(P::ChargeTime, 0.3);
    a.sh.params.set(P::AutoFire, 1.0);
    let mut proc = a.audio_processor().unwrap();
    a.tick(&keys(&[60]));
    run(&mut proc, 0.6);
    assert!(a.sh.fired.load(Ordering::Relaxed) >= 1, "it fired by itself");
    assert_eq!(a.sh.charge.get(), 0.0, "and is no longer charging");
    let after = run(&mut proc, 0.2);
    assert!(peak(&after) > 0.05, "the blast is sounding while the key is still down");
}

#[test]
fn the_key_sets_the_pitch_unless_key_follow_is_off() {
    let blast_pitch = |note: u8, follow: f32| {
        let mut a = app();
        clean(&a);
        let p = &a.sh.params;
        p.set(P::KeyFollow, follow);
        p.set(P::BlastStart, 800.0);
        p.set(P::BlastEnd, 800.0);
        p.set(P::SweepTime, 0.05);
        p.set(P::Length, 0.4);
        p.set(P::PowerPitch, 0.0);
        p.set(P::ChargeTime, 0.3);
        let mut proc = a.audio_processor().unwrap();
        a.tick(&keys(&[note]));
        run(&mut proc, 0.1);
        a.tick(&keys(&[]));
        let b = run(&mut proc, 0.3);
        freq(&b[2400..9600])
    };
    let (low, high) = (blast_pitch(60, 1.0), blast_pitch(72, 1.0));
    assert!((high / low - 2.0).abs() < 0.1, "an octave up: {low:.0} {high:.0}");
    let (a, b) = (blast_pitch(60, 0.0), blast_pitch(72, 0.0));
    assert!((a / b - 1.0).abs() < 0.05, "key follow off: {a:.0} {b:.0}");
}

#[test]
fn several_keys_charge_and_fire_independently() {
    let mut a = app();
    clean(&a);
    a.sh.params.set(P::ChargeTime, 1.0);
    let mut proc = a.audio_processor().unwrap();
    a.tick(&keys(&[60, 64, 67]));
    run(&mut proc, 0.3);
    assert_eq!(a.sh.active.load(Ordering::Relaxed), 3);
    a.tick(&keys(&[60, 64]));
    run(&mut proc, 0.05);
    a.tick(&keys(&[]));
    run(&mut proc, 4.0);
    assert_eq!(a.sh.active.load(Ordering::Relaxed), 0, "all of them fired and finished");
    assert!(a.sh.fired.load(Ordering::Relaxed) >= 3);
}

#[test]
fn more_notes_than_voices_never_breaks() {
    let mut a = app();
    let mut proc = a.audio_processor().unwrap();
    let all: Vec<u8> = (48..64).collect();
    a.tick(&keys(&all));
    run(&mut proc, 0.5);
    a.tick(&keys(&[]));
    run(&mut proc, 1.0);
}

#[test]
fn every_character_charges_and_fires_something_that_ends() {
    let mut a = app();
    let mut proc = a.audio_processor().unwrap();
    for i in 0..a.presets.len() {
        a.load_preset(i);
        let name = a.presets[i].name;
        a.tick(&keys(&[60]));
        let charge = run(&mut proc, 0.6);
        assert!(peak(&charge) > 0.01, "{name}: no charge sound ({})", peak(&charge));
        a.tick(&keys(&[]));
        let blast = run(&mut proc, 0.4);
        assert!(peak(&blast) > 0.05, "{name}: no blast ({})", peak(&blast));
        let tail = run(&mut proc, 9.0);
        assert!(peak(&tail[tail.len() - 4800..]) < 0.002, "{name}: never ends ({})", peak(&tail[tail.len() - 4800..]));
        assert!(peak(&blast) <= 1.0 && peak(&charge) <= 1.0, "{name}: clips");
    }
}

#[test]
fn characters_are_a_similar_loudness() {
    let mut a = app();
    let mut proc = a.audio_processor().unwrap();
    let mut quiet = Vec::new();
    let mut loud = Vec::new();
    for i in 0..a.presets.len() {
        a.load_preset(i);
        a.tick(&keys(&[60]));
        run(&mut proc, 1.5);
        a.tick(&keys(&[]));
        let blast = run(&mut proc, 1.0);
        let p = peak(&blast);
        if p < 0.12 {
            quiet.push(format!("{} {p:.2}", a.presets[i].name));
        }
        if p > 0.98 {
            loud.push(format!("{} {p:.2}", a.presets[i].name));
        }
        run(&mut proc, 6.0);
    }
    assert!(quiet.is_empty(), "too quiet: {quiet:?}");
    assert!(loud.len() <= 6, "hot: {loud:?}");
}

#[test]
fn characters_have_unique_names_and_valid_values() {
    let presets = presets::factory();
    assert!(presets.len() >= 24);
    let mut names: Vec<_> = presets.iter().map(|p| p.name).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), presets.len());
    for p in &presets {
        for &(i, v) in &p.values {
            assert!(i < COUNT, "{}", p.name);
            let clamped = store::clamp_value(&DEFS[i], v);
            assert!((clamped - v).abs() <= 1e-3 * v.abs().max(1.0), "{}: {} = {v} is outside its range (clamps to {clamped})", p.name, DEFS[i].name);
        }
    }
}

#[test]
fn randomize_and_mutate_stay_in_range() {
    let params = Params::new();
    let mut rng = Rng::new(5);
    for _ in 0..100 {
        presets::randomize(&params, &mut rng);
        presets::mutate(&params, &mut rng);
        for i in 0..COUNT {
            let v = params.at(i);
            assert!(v.is_finite());
            assert_eq!(store::clamp_value(&DEFS[i], v), v, "{}", DEFS[i].name);
        }
    }
}

#[test]
fn random_blasts_always_make_sound_and_end() {
    let mut a = app();
    let mut proc = a.audio_processor().unwrap();
    let mut rng = Rng::new(21);
    for n in 0..20 {
        presets::randomize(&a.sh.params, &mut rng);
        a.tick(&keys(&[60]));
        let c = run(&mut proc, 0.5);
        a.tick(&keys(&[]));
        let b = run(&mut proc, 0.4);
        assert!(peak(&c).max(peak(&b)) > 0.01, "random blast {n} is silent");
        let tail = run(&mut proc, 9.0);
        assert!(peak(&tail[tail.len() - 4800..]) < 0.01, "random blast {n} never ends");
    }
}

#[test]
fn the_menu_lists_characters_and_groups() {
    let mut a = app();
    let rows = a.menu_rows();
    assert_eq!(rows[0].0, "Character");
    assert!(rows.iter().any(|r| r.2));
    let n = rows.len();
    a.expanded = [true; GROUPS.len()];
    assert!(a.menu_rows().len() > n + 40);
}

#[test]
fn the_play_controls_include_the_character_browser_and_the_charge_dials() {
    let a = app();
    let labels: Vec<String> = a.instrument_settings().into_iter().map(|s| s.label).collect();
    for want in ["Charge time", "Blast length", "Character", "Level", "Blast start", "Noise"] {
        assert!(labels.iter().any(|l| l == want), "missing {want}: {labels:?}");
    }
    assert_eq!(labels.len(), CONTROLS);
}

#[test]
fn moments_round_trip_the_whole_blast() {
    let mut a = app();
    a.load_preset(3);
    let snap = a.kit_snapshot();
    let before = a.sh.params.snapshot();
    a.load_preset(8);
    assert_ne!(a.sh.params.snapshot()[..], before[..]);
    a.kit_recall(&snap);
    assert_eq!(a.sh.params.snapshot()[..], before[..]);
    assert_eq!(a.preset, 3);
}

#[test]
fn the_pad_charges_and_fires_through_the_app() {
    let mut a = app();
    let mut proc = a.audio_processor().unwrap();
    let pad = Input { grid: std::array::from_fn(|n| n == 12), ..Default::default() };
    a.tick(&pad);
    assert!(peak(&run(&mut proc, 0.5)) > 0.01, "holding a pad charges");
    assert!(a.sh.charge.get() > 0.1);
    a.tick(&Input::default());
    assert!(peak(&run(&mut proc, 0.3)) > 0.05, "letting go fires");
    assert!(a.sh.fired.load(Ordering::Relaxed) >= 1);
}

#[test]
fn the_control_inputs_exist() {
    let mods = Arc::new(ModBus::new());
    let _a = BlasterApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), mods.clone(), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()));
    for n in ["Blaster: Pitch", "Blaster: Charge Time", "Blaster: Blast Length"] {
        assert!(mods.names().iter().any(|m| m == n), "{n}");
    }
}

#[test]
fn it_draws() {
    let mut a = app();
    let mut fb = FrameBuffer::new();
    a.draw(&mut fb);
    a.kit.menu = true;
    a.draw(&mut fb);
}

/// The fundamental of a harmonic-rich tone, by autocorrelation (zero crossings
/// would count the upper harmonics).
fn pitch_ac(x: &[f32]) -> f32 {
    let n = x.len().min(8192);
    let (lo, hi) = ((FS / 1000.0) as usize, (FS / 100.0) as usize);
    let corr: Vec<f32> = (0..=hi).map(|lag| if lag < lo { 0.0 } else { (0..n - lag).map(|i| x[i] * x[i + lag]).sum() }).collect();
    let best = corr.iter().cloned().fold(0.0f32, f32::max);
    for lag in lo.max(1)..hi {
        if corr[lag] > best * 0.9 && corr[lag] >= corr[lag - 1] && corr[lag] > corr[lag + 1] {
            let (a, b, c) = (corr[lag - 1], corr[lag], corr[lag + 1]);
            let off = if (a - 2.0 * b + c).abs() > 1e-12 { 0.5 * (a - c) / (a - 2.0 * b + c) } else { 0.0 };
            return FS / (lag as f32 + off);
        }
    }
    0.0
}

/// Levels of harmonics 1..=12 of a steady tone near `f0`, in dB re the first.
fn harmonics(x: &[f32], f0: f32) -> Vec<f32> {
    let n = x.len();
    let win: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
    let level = |f: f32| {
        // a direct DFT at the bin nearest f, over a few neighbours (no FFT needed for twelve lines)
        let mut best = 0.0f32;
        for df in [-3.0f32, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0] {
            let w = std::f32::consts::TAU * (f + df) / FS;
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, (s, h)) in x.iter().zip(&win).enumerate() {
                re += s * h * (w * i as f32).cos();
                im += s * h * (w * i as f32).sin();
            }
            best = best.max((re * re + im * im).sqrt());
        }
        best
    };
    let h1 = level(f0);
    (1..=12).map(|k| 20.0 * (level(f0 * k as f32) / h1).log10()).collect()
}

#[test]
fn the_reed_wave_has_the_spectrum_measured_from_a_recording() {
    let mut a = app();
    clean(&a);
    let p = &a.sh.params;
    p.set(P::ChgWave, 7.0);
    p.set(P::ChgPitch, 500.0);
    p.set(P::ChirpCount, 0.0);
    p.set(P::ChirpDepth, 0.0);
    p.set(P::Climb, 0.0);
    p.set(P::ChgVol, 1.0);
    p.set(P::ChargeTime, 0.1);
    p.set(P::FullMode, 1.0);
    p.set(P::FlutterSemis, 1.0);
    p.set(P::FlutterRate, 2.0);
    // at full charge with Hold at top = 0, flutter would move the pitch; use a mode that holds still
    p.set(P::FullMode, 2.0);
    p.set(P::RippleDepth, 0.0);
    let mut proc = a.audio_processor().unwrap();
    a.tick(&keys(&[60]));
    run(&mut proc, 0.3);
    let tone = run(&mut proc, 0.5);
    let measured = harmonics(&tone, 500.0);
    for (k, (got, want)) in measured.iter().zip(voice::REED_DB.iter()).enumerate().take(12) {
        assert!((got - want).abs() < if k < 8 { 3.5 } else { 5.0 }, "harmonic {}: {got:.1} dB, the recording has {want:.1}", k + 1);
    }
    assert!(measured[4] < -15.0 && measured[6] < -15.0, "the 5th and 7th are nearly absent, unlike a square wave");
    assert!(measured[1] > -3.0, "and the 2nd is strong, unlike a square wave: {:.1}", measured[1]);
}

#[test]
fn counted_chirps_end_at_the_top_exactly_when_the_charge_is_full() {
    let mut a = app();
    clean(&a);
    let p = &a.sh.params;
    p.set(P::ChargeTime, 1.0);
    p.set(P::ChgPitch, 200.0);
    p.set(P::Climb, 0.0);
    p.set(P::ChirpCount, 1.0);
    p.set(P::ChirpDepth, 1.0);
    p.set(P::ChirpCurve, 1.0);
    p.set(P::HoldTop, 1.0);
    p.set(P::FullMode, 2.0);
    p.set(P::RippleDepth, 0.0);
    p.set(P::ChgVol, 1.0);
    let mut proc = a.audio_processor().unwrap();
    a.tick(&keys(&[60]));
    let start = run(&mut proc, 0.1);
    run(&mut proc, 0.78);
    let before_full = run(&mut proc, 0.1);
    run(&mut proc, 0.1);
    let held = run(&mut proc, 0.3);
    let (f_start, f_end, f_held) = (freq(&start), freq(&before_full), freq(&held));
    assert!((185.0..225.0).contains(&f_start), "it starts at the base pitch: {f_start:.0}");
    assert!(f_end > 340.0 && f_end < 400.0, "the ramp is nearly at the top just before full: {f_end:.0}");
    assert!((f_held - 400.0).abs() < 8.0, "and the held tone sits at the top, one octave up: {f_held:.0}");
}

#[test]
fn ripple_shimmers_the_held_tone_without_moving_its_pitch() {
    let mut a = app();
    clean(&a);
    let p = &a.sh.params;
    p.set(P::ChargeTime, 0.2);
    p.set(P::ChgPitch, 400.0);
    p.set(P::ChirpDepth, 0.0);
    p.set(P::Climb, 0.0);
    p.set(P::FullMode, 2.0);
    p.set(P::FlutterRate, 19.0);
    p.set(P::RippleDepth, 0.6);
    p.set(P::ChgVol, 1.0);
    let mut proc = a.audio_processor().unwrap();
    a.tick(&keys(&[60]));
    run(&mut proc, 0.5);
    let held = run(&mut proc, 1.0);
    let env: Vec<f32> = held.chunks(480).map(rms).collect();
    let (lo, hi) = (env.iter().cloned().fold(f32::MAX, f32::min), env.iter().cloned().fold(0.0f32, f32::max));
    assert!(lo < hi * 0.6, "the volume ripples: {lo:.3} to {hi:.3}");
    assert!((freq(&held) - 400.0).abs() < 8.0, "the pitch does not move: {:.0}", freq(&held));
}

#[test]
fn the_first_character_matches_the_recording_it_was_built_from() {
    let mut a = app();
    let mut proc = a.audio_processor().unwrap();
    a.load_preset(0);
    assert_eq!(a.presets[0].name, "X Charge Shot");
    a.tick(&keys(&[60]));
    let early = run(&mut proc, 0.6);
    run(&mut proc, 0.9);
    let held = run(&mut proc, 0.5);
    // the recording holds 502 Hz at full charge, and its charge starts near 155 Hz
    assert!((pitch_ac(&held) - 502.0).abs() < 12.0, "held tone {:.0} Hz", pitch_ac(&held));
    assert!(pitch_ac(&early[..4800]) < 190.0, "it starts low: {:.0} Hz", pitch_ac(&early[..4800]));
    // the release is a noise burst with no tone: it peaks within about 0.2 s, is gone by 0.7 s
    a.tick(&keys(&[]));
    let blast = run(&mut proc, 1.0);
    let at = blast.iter().position(|s| s.abs() == peak(&blast)).unwrap() as f32 / FS;
    assert!(at < 0.25, "the burst peaks quickly: {at:.2}s");
    assert!(peak(&blast[(0.75 * FS) as usize..]) < 0.003, "and is gone by 0.75 s");
    assert!(peak(&blast) > 4.0 * peak(&held), "and is much louder than the held charge ({:.3} vs {:.3})", peak(&blast), peak(&held));
}

/// Writes a WAV of each character (hold, release, tail) so a person can listen:
/// `BLASTER_WRITE_WAVS=/some/dir cargo test --bin portamax-sim render_demo_wavs`.
#[test]
fn render_demo_wavs() {
    let Ok(dir) = std::env::var("BLASTER_WRITE_WAVS") else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let mut a = app();
    let mut proc = a.audio_processor().unwrap();
    for i in 0..a.presets.len() {
        a.load_preset(i);
        let hold = a.sh.params.get(P::ChargeTime) * 1.15;
        let mut audio = run(&mut proc, 0.15);
        a.tick(&keys(&[60]));
        audio.extend(run(&mut proc, hold));
        a.tick(&keys(&[]));
        audio.extend(run(&mut proc, 3.0));
        run(&mut proc, 6.0);
        let mut bytes = Vec::new();
        let n = audio.len() as u32;
        bytes.extend(b"RIFF");
        bytes.extend((36 + n * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend((FS as u32).to_le_bytes());
        bytes.extend((FS as u32 * 2).to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((n * 2).to_le_bytes());
        for s in &audio {
            bytes.extend(((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        let name = format!("{:02}_{}.wav", i + 1, a.presets[i].name.replace(' ', "_"));
        std::fs::write(std::path::Path::new(&dir).join(name), bytes).unwrap();
    }
}
