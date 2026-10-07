use super::*;
use crate::app::MidiKeys;

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

/// A keyboard with these notes down.
fn keys(notes: &[u8]) -> Input {
    let mut k = MidiKeys::default();
    for &n in notes {
        k.0[n as usize] = 100;
    }
    Input { midi_keys: k, ..Default::default() }
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
fn menu_lists_the_preset_area_first_then_groups_and_expands() {
    let mut a = app();
    let rows = a.menu_rows();
    assert_eq!(rows[0].0, "Folder");
    assert_eq!(rows[1].0, "Preset");
    assert!(rows.iter().any(|r| r.2), "groups are headed rows");
    let collapsed = rows.len();
    a.expanded = [true; GROUPS.len()];
    assert!(a.menu_rows().len() > collapsed + 100);
}

#[test]
fn the_folder_row_browses_folders_and_the_preset_row_stays_inside_one() {
    let mut a = app();
    a.folder = 2; // Init is 1 (Favorites is 0 and empty), Bass is 2
    a.step_folder(0);
    a.step_folder(1);
    let folder = a.folder_name();
    let first = a.cur;
    assert_eq!(a.lib.presets[a.cur].category, folder, "stepping into a folder loads its first sound");
    for _ in 0..40 {
        a.step_preset(1);
        assert_eq!(a.lib.presets[a.cur].category, folder, "the preset knob never leaves the folder");
    }
    let n = a.folder_list().len();
    a.load_preset(first);
    for _ in 0..n {
        a.step_preset(1);
    }
    assert_eq!(a.cur, first, "it wraps around the folder");
}

#[test]
fn browsing_lists_the_folders_presets_and_pressing_one_loads_it() {
    let mut a = app();
    a.browse_open = true;
    let listed: Vec<Row> = a.rows().into_iter().filter(|r| matches!(r, Row::Listed(_))).collect();
    assert_eq!(listed.len(), a.folder_list().len());
    let Row::Listed(i) = listed[listed.len() - 1] else { unreachable!() };
    a.row_edit(Row::Listed(i), 0, true);
    assert_eq!(a.cur, i);
}

#[test]
fn instrument_settings_expose_the_play_controls() {
    let a = app();
    let names: Vec<String> = a.instrument_settings().into_iter().map(|s| s.label).collect();
    for want in ["Preset", "Folder", "Cutoff", "Resonance", "Level"] {
        assert!(names.iter().any(|n| n == want), "missing {want}: {names:?}");
    }
    assert_eq!(names.len(), CONTROLS);
}

#[test]
fn the_four_macro_dials_carry_the_sounds_own_macro_names() {
    let mut a = app();
    let i = a.lib.presets.iter().position(|p| p.name == "Sub Bass").unwrap();
    a.load_preset(i);
    let names: Vec<String> = (12..16).map(|c| a.kit_label(c)).collect();
    assert_eq!(names, vec!["Cutoff", "Drive", "Sub", "Space"]);
}

#[test]
fn a_macro_moves_what_the_sound_wired_it_to() {
    let mut a = app();
    let i = a.lib.presets.iter().position(|p| p.name == "Sub Bass").unwrap();
    a.load_preset(i);
    let mut p = a.audio_processor().unwrap();
    let base = a.sh.params.get(P::F1_Cut);
    let at = |a: &HydraApp| engine_cutoff(a);
    assert!((at(&a) - base).abs() < 1.0, "a macro at rest changes nothing");
    a.sh.params.set(P::Mac1, 1.0);
    assert!(at(&a) > base * 2.0, "macro 1 (Cutoff) opens the filter: {} -> {}", base, at(&a));
    a.tick(&pad(0));
    assert!(run(&mut p, 10) > 0.005);
}

/// The cutoff the engine would render with, macros included.
fn engine_cutoff(a: &HydraApp) -> f32 {
    let eng = Engine::new(Arc::clone(&a.sh));
    eng.snapshot()[P::F1_Cut as usize]
}

#[test]
fn the_page_selector_swaps_six_controls_and_keeps_the_pinned_ones() {
    let mut a = app();
    let pinned: Vec<String> = [0, 1, 2, 3, 10, 11, 12, 13, 14, 15].iter().map(|&c| a.kit_label(c)).collect();
    let osc: Vec<String> = (4..10).map(|c| a.kit_label(c)).collect();
    a.kit_edit(C_BANK, 1);
    let filter: Vec<String> = (4..10).map(|c| a.kit_label(c)).collect();
    assert_ne!(osc, filter, "page 2 shows different controls");
    assert!(filter.contains(&"Ladder cutoff".to_string()));
    let still: Vec<String> = [0, 1, 2, 3, 10, 11, 12, 13, 14, 15].iter().map(|&c| a.kit_label(c)).collect();
    assert_eq!(pinned, still);
    for _ in 0..5 {
        a.kit_edit(C_BANK, 1);
    }
    assert_eq!(a.bank, 0, "six pages, and it wraps");
}

#[test]
fn a_page_control_turns_the_parameter_it_is_labelled_with() {
    let mut a = app();
    a.bank = 4; // FX
    let delay = (4..10).find(|&c| a.kit_label(c) == "Delay").unwrap();
    let before = a.sh.params.get(P::Dly_Mix);
    a.kit_set_norm(delay, 0.8);
    assert!(a.sh.params.get(P::Dly_Mix) > before);
    assert!((a.kit_norm(delay).unwrap() - 0.8).abs() < 0.02);
}

#[test]
fn moments_round_trip_the_whole_patch_and_the_macro_names() {
    let mut a = app();
    let i = a.lib.presets.iter().position(|p| p.name == "Warm Pad").unwrap();
    a.load_preset(i);
    let snap = a.kit_snapshot();
    let before = a.sh.params.snapshot();
    let names = a.macros.clone();
    let j = a.lib.presets.iter().position(|p| p.name == "Sub Bass").unwrap();
    a.load_preset(j);
    assert_ne!(a.sh.params.snapshot()[..], before[..]);
    a.kit_recall(&snap);
    assert_eq!(a.sh.params.snapshot()[..], before[..]);
    assert_eq!(a.macros, names);
    assert_eq!(a.cur, i);
}

#[test]
fn morph_slides_between_two_sounds_and_lands_on_the_second() {
    let mut a = app();
    let i = a.lib.presets.iter().position(|p| p.name == "Warm Pad").unwrap();
    let j = a.lib.presets.iter().position(|p| p.name == "Dark Pad").unwrap();
    a.load_preset(i);
    let from = a.sh.params.get(P::F1_Cut);
    let to = presets::resolve(&a.lib.presets[j])[P::F1_Cut as usize];
    assert!(from != to);
    a.start_morph(j);
    a.set_morph(0.0);
    assert!((a.sh.params.get(P::F1_Cut) - from).abs() < 1.0);
    a.set_morph(0.5);
    let mid = a.sh.params.get(P::F1_Cut);
    assert!(mid > from.min(to) && mid < from.max(to), "halfway sits between: {from} {mid} {to}");
    // a cutoff is heard in octaves, so halfway is the geometric middle
    assert!((mid - (from * to).sqrt()).abs() / mid < 0.05, "{mid} vs {}", (from * to).sqrt());
    a.set_morph(1.0);
    assert!((a.sh.params.get(P::F1_Cut) - to).abs() < 1.0);
    assert_eq!(a.macros, a.lib.presets[j].macros, "the macros become the target's");
}

#[test]
fn the_morph_dial_with_no_target_heads_for_the_next_sound_in_the_folder() {
    let mut a = app();
    let next = {
        let list = a.folder_list();
        let p = list.iter().position(|&i| i == a.cur).unwrap();
        list[(p + 1) % list.len()]
    };
    a.set_morph(0.4);
    assert_eq!(a.morph.as_ref().unwrap().target, next);
    a.kit_reset(a.bank_control_index_of_morph());
    assert!(a.morph.is_none(), "resetting the dial puts the sound back");
}

impl HydraApp {
    fn bank_control_index_of_morph(&mut self) -> usize {
        self.bank = 3;
        (4..10).find(|&c| self.kit_label(c) == "Morph").unwrap()
    }
}

#[test]
fn loading_a_preset_ends_a_morph() {
    let mut a = app();
    a.set_morph(0.3);
    assert!(a.morph.is_some());
    let cur = a.cur;
    a.load_preset(cur);
    assert!(a.morph.is_none());
    assert_eq!(a.morph_amt, 0.0);
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
fn a_chord_turns_one_key_into_several_voices_and_letting_go_stops_them_all() {
    let mut a = app();
    a.sh.params.set(P::Chord, 3.0); // major
    a.sh.params.set(P::AmpR, 0.02);
    let mut p = a.audio_processor().unwrap();
    a.tick(&keys(&[60]));
    run(&mut p, 8);
    assert_eq!(a.sh.active.load(Ordering::Relaxed), 3, "root, third and fifth");
    a.tick(&keys(&[]));
    run(&mut p, 200);
    assert_eq!(a.sh.active.load(Ordering::Relaxed), 0, "all three let go");
}

#[test]
fn changing_the_chord_while_a_key_is_down_does_not_leave_notes_hanging() {
    let mut a = app();
    a.sh.params.set(P::Chord, 7.0); // maj7: four notes
    a.sh.params.set(P::AmpR, 0.02);
    let mut p = a.audio_processor().unwrap();
    a.tick(&keys(&[60]));
    run(&mut p, 8);
    a.sh.params.set(P::Chord, 0.0);
    a.tick(&keys(&[]));
    run(&mut p, 200);
    assert_eq!(a.sh.active.load(Ordering::Relaxed), 0, "it stopped the notes it started, not the ones the new setting would make");
}

#[test]
fn overlapping_chords_share_a_note_until_both_keys_are_up() {
    let mut a = app();
    a.sh.params.set(P::Chord, 3.0);
    a.sh.params.set(P::AmpR, 0.02);
    let mut p = a.audio_processor().unwrap();
    // C major has G (67) and E major has... use C (60) and G (67): both contain G? 60: C E G; 67: G B D
    a.tick(&keys(&[60, 67]));
    run(&mut p, 8);
    a.tick(&keys(&[60]));
    run(&mut p, 8);
    // 67 let go, but the G inside C major's chord must keep sounding
    let sounding = a.sh.active.load(Ordering::Relaxed);
    assert_eq!(sounding, 3, "C major is still whole");
}

#[test]
fn a_scale_chord_builds_triads_from_the_pad_scale() {
    let mut a = app();
    a.sh.params.set(P::Scale, 1.0); // major
    a.sh.params.set(P::Root, 0.0);
    a.sh.params.set(P::Chord, CHORD_SCALE_FIRST as f32);
    let mut out = [0u8; 6];
    let n = Engine::expand(60, &a.sh.params.snapshot(), &mut out);
    assert_eq!(&out[..n], &[60, 64, 67], "I is C major");
    let n = Engine::expand(62, &a.sh.params.snapshot(), &mut out);
    assert_eq!(&out[..n], &[62, 65, 69], "ii is D minor");
    let n = Engine::expand(71, &a.sh.params.snapshot(), &mut out);
    assert_eq!(&out[..n], &[71, 74, 77], "vii is diminished");
    a.sh.params.set(P::Chord, (CHORD_SCALE_FIRST + 1) as f32);
    let n = Engine::expand(60, &a.sh.params.snapshot(), &mut out);
    assert_eq!(&out[..n], &[60, 64, 67, 71], "scale seventh: Cmaj7");
}

#[test]
fn hold_keeps_notes_sounding_until_a_new_hand_arrives() {
    let mut a = app();
    a.sh.params.set(P::Hold, 1.0);
    a.sh.params.set(P::AmpR, 0.02);
    let mut p = a.audio_processor().unwrap();
    a.tick(&keys(&[60, 64]));
    run(&mut p, 8);
    a.tick(&keys(&[]));
    run(&mut p, 100);
    assert_eq!(a.sh.active.load(Ordering::Relaxed), 2, "still ringing with the hands off");
    a.tick(&keys(&[67]));
    run(&mut p, 100);
    assert_eq!(a.sh.active.load(Ordering::Relaxed), 1, "a fresh touch replaces the held notes");
    a.sh.params.set(P::Hold, 0.0);
    run(&mut p, 100);
    a.tick(&keys(&[]));
    run(&mut p, 300);
    assert_eq!(a.sh.active.load(Ordering::Relaxed), 0, "hold off lets everything go");
}

#[test]
fn turning_hold_off_releases_what_it_was_keeping() {
    let mut a = app();
    a.sh.params.set(P::Hold, 1.0);
    a.sh.params.set(P::AmpR, 0.02);
    let mut p = a.audio_processor().unwrap();
    a.tick(&keys(&[60]));
    run(&mut p, 8);
    a.tick(&keys(&[]));
    run(&mut p, 50);
    assert_eq!(a.sh.active.load(Ordering::Relaxed), 1);
    a.sh.params.set(P::Hold, 0.0);
    run(&mut p, 300);
    assert_eq!(a.sh.active.load(Ordering::Relaxed), 0);
}

#[test]
fn pad_layouts_play_the_scale_every_semitone_or_fourths() {
    let mut a = app();
    a.sh.params.set(P::Scale, 1.0);
    a.sh.params.set(P::Root, 0.0);
    let bottom_left = 12; // physical pad 12 is rank 0
    assert_eq!(a.pad_note(bottom_left), 48);
    assert_eq!(a.pad_note(13), 50, "major scale: a whole tone up");
    a.sh.params.set(P::PadMode, 1.0);
    assert_eq!(a.pad_note(13), 49, "chromatic: a semitone");
    a.sh.params.set(P::PadMode, 2.0);
    assert_eq!(a.pad_note(13), 49);
    assert_eq!(a.pad_note(8), 53, "fourths: the row above is a fourth higher");
    assert!(a.pad_is_root(bottom_left));
}

#[test]
fn control_voltage_inputs_exist() {
    let mods = Arc::new(ModBus::new());
    let _a = HydraApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), mods.clone(), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()));
    assert!(mods.names().iter().any(|n| n == "Hydra: Cutoff"));
}

// -------------------------------------------------- the factory library

/// What is played to judge a sound's loudness: low for bass, a chord to
/// arpeggiate for arps, middle C for everything else.
fn probe_notes(category: &str) -> Vec<u8> {
    match category {
        "Bass" => vec![36],
        "Arp" => vec![60, 64, 67],
        _ => vec![60],
    }
}

/// How loud a sound plays: the geometric mean of its RMS and its peak over
/// a second and a half held note, so a sustained pad and a short pluck are
/// judged alike.
fn loudness(a: &mut HydraApp, p: &mut Box<dyn AudioProcessor>, index: usize) -> f32 {
    a.load_preset(index);
    a.tick(&keys(&probe_notes(&a.lib.presets[index].category)));
    let mut buf = vec![0.0f32; 512];
    let (mut sum, mut peak, mut n) = (0.0f64, 0.0f32, 0usize);
    for block in 0..140 {
        buf.fill(0.0);
        p.process(&mut buf, 2, 48_000.0);
        assert!(buf.iter().all(|s| s.is_finite()), "{}: not finite", a.lib.presets[index].name);
        if block >= 4 {
            for s in &buf {
                sum += (*s as f64) * (*s as f64);
                peak = peak.max(s.abs());
                n += 1;
            }
        }
    }
    a.tick(&keys(&[]));
    a.sh.panic.store(true, Ordering::Relaxed);
    for _ in 0..4 {
        buf.fill(0.0);
        p.process(&mut buf, 2, 48_000.0);
    }
    let rms = (sum / n.max(1) as f64).sqrt() as f32;
    (rms * peak).sqrt()
}

const TARGET: f32 = 0.16;

/// Writes `presets/levels.rs`: each factory sound's Level, found by playing
/// it and nudging until it is as loud as the rest. Run with
/// `HYDRA_WRITE_LEVELS=1 cargo test --bin portamax-sim calibrate_levels`.
#[test]
fn calibrate_levels() {
    if std::env::var("HYDRA_WRITE_LEVELS").is_err() {
        return;
    }
    let mut a = app();
    let mut p = a.audio_processor().unwrap();
    let mut rows = Vec::new();
    for i in 0..a.lib.presets.len() {
        let name = a.lib.presets[i].name.clone();
        let mut level = 0.7f32;
        for _ in 0..4 {
            a.lib.presets[i].values.retain(|&(k, _)| k != P::Level as usize);
            a.lib.presets[i].values.push((P::Level as usize, level));
            let l = loudness(&mut a, &mut p, i);
            if l < 1e-4 {
                break;
            }
            level = (level * (TARGET / l)).clamp(0.08, 1.0);
        }
        rows.push((name, (level * 100.0).round() / 100.0));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut text = String::from("//! Factory loudness, written by the `calibrate_levels` test:\n//! `HYDRA_WRITE_LEVELS=1 cargo test --bin portamax-sim calibrate_levels`.\n\npub static LEVELS: &[(&str, f32)] = &[\n");
    for (n, l) in &rows {
        text.push_str(&format!("    ({n:?}, {l}),\n"));
    }
    text.push_str("];\n");
    std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/src/apps/hydra/presets/levels.rs"), text).unwrap();
}

#[test]
fn every_factory_preset_plays_finite_audio_at_a_similar_loudness() {
    let mut a = app();
    let mut p = a.audio_processor().unwrap();
    let mut quiet = Vec::new();
    let mut loud = Vec::new();
    for i in 0..a.lib.presets.len() {
        let name = a.lib.presets[i].name.clone();
        let l = loudness(&mut a, &mut p, i);
        if name == "Init" {
            continue;
        }
        assert!(l > 0.005, "{name} is silent ({l})");
        if l < 0.07 {
            quiet.push(format!("{name} {l:.3}"));
        }
        if l > 0.4 {
            loud.push(format!("{name} {l:.3}"));
        }
    }
    assert!(quiet.len() <= 6, "too many quiet sounds: {quiet:?}");
    assert!(loud.is_empty(), "too loud: {loud:?}");
}
