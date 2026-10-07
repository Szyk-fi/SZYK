use super::*;

#[test]
fn the_library_is_big_and_organized_into_folders() {
    let presets = factory();
    assert!(presets.len() >= 200, "a real library: {}", presets.len());
    for c in CATEGORIES {
        let n = presets.iter().filter(|p| p.category == c).count();
        assert!(n >= if c == "Init" { 1 } else { 15 }, "{c} has {n}");
    }
    for p in &presets {
        assert!(CATEGORIES.contains(&p.category.as_str()), "{} is in a folder that is not listed: {}", p.name, p.category);
    }
}

#[test]
fn names_are_unique_and_every_value_is_a_real_parameter() {
    let presets = factory();
    let mut names: Vec<_> = presets.iter().map(|p| p.name.as_str()).collect();
    names.sort();
    let before = names.len();
    names.dedup();
    assert_eq!(names.len(), before, "names are unique across the whole library");
    for p in &presets {
        for &(i, v) in &p.values {
            assert!(i < COUNT, "{}: index {i}", p.name);
            assert!(v.is_finite(), "{}: {}", p.name, DEFS[i].name);
        }
    }
}

#[test]
fn every_value_a_preset_sets_is_inside_the_range_it_asked_for() {
    // A typo like a cutoff of 90000 or a wave of 7 would be quietly clamped
    // by the table; that hides a sound that is not what its author meant.
    for p in factory() {
        for &(i, v) in &p.values {
            let clamped = super::super::store::clamp_value(&DEFS[i], v);
            assert!((clamped - v).abs() <= 1e-3 * v.abs().max(1.0), "{}: {} = {v} is outside its range (clamps to {clamped})", p.name, DEFS[i].name);
        }
    }
}

#[test]
fn every_preset_names_four_macros_that_do_something() {
    for p in factory() {
        let r = resolve(&p);
        for n in 0..4 {
            assert!(!p.macros[n].is_empty(), "{}: macro {} has no name", p.name, n + 1);
            let base = P::Mac1_DA as usize + n * 4;
            let (a, b) = (r[base] as usize, r[base + 2] as usize);
            assert!(a > 0 || b > 0, "{}: macro {} ({}) moves nothing", p.name, n + 1, p.macros[n]);
            assert!(r[base + 1] != 0.0 || r[base + 3] != 0.0, "{}: macro {} ({}) has no amount", p.name, n + 1, p.macros[n]);
        }
    }
}

#[test]
fn a_macro_does_not_move_a_parameter_that_is_already_at_its_limit() {
    // "Bright" on a filter that is already wide open would be a dead dial.
    let mut dead = Vec::new();
    for p in factory() {
        let r = resolve(&p);
        'macros: for n in 0..4 {
            let base = P::Mac1_DA as usize + n * 4;
            let mut live = false;
            for slot in 0..2 {
                let (d, amt) = (r[base + slot * 2] as usize, r[base + slot * 2 + 1]);
                if let Some(dest) = macro_dest(d) {
                    let at = to_norm(&DEFS[dest as usize], r[dest as usize]);
                    if (amt > 0.0 && at < 0.98) || (amt < 0.0 && at > 0.02) {
                        live = true;
                    }
                }
            }
            if !live {
                dead.push(format!("{} / {}", p.name, p.macros[n]));
                continue 'macros;
            }
        }
    }
    assert!(dead.len() <= factory().len() / 12, "too many dead macros ({}): {:?}", dead.len(), dead);
}

#[test]
fn applying_a_preset_resets_what_it_does_not_mention() {
    let params = Params::new();
    let presets = factory();
    let lead = presets.iter().find(|p| p.name == "Super Saw").unwrap();
    apply(&params, lead);
    assert_eq!(params.get(P::Unison), 7.0);
    apply(&params, &presets[0]);
    assert_eq!(params.get(P::Unison), 1.0, "Init puts everything back");
    assert_eq!(params.get(P::F1_Cut), DEFS[P::F1_Cut as usize].default);
}

#[test]
fn randomize_and_mutate_stay_in_range_and_keep_a_sounding_patch() {
    let params = Params::new();
    let mut rng = Rng::new(99);
    for _ in 0..50 {
        randomize(&params, &mut rng);
        assert!(params.get(P::A_Level) >= 0.3, "an oscillator is on");
        assert!(params.get(P::F1_Cut) >= 100.0, "the filter is not closed");
        assert!(params.get(P::AmpS) >= 0.4);
        for i in 0..COUNT {
            let v = params.at(i);
            assert!(v.is_finite());
            assert_eq!(super::super::store::clamp_value(&DEFS[i], v), v, "{}", DEFS[i].name);
        }
        mutate(&params, &mut rng);
    }
}

#[test]
fn mutate_changes_some_continuous_parameters_but_not_the_setup() {
    let params = Params::new();
    let mut rng = Rng::new(3);
    let before = params.snapshot();
    for _ in 0..5 {
        mutate(&params, &mut rng);
    }
    let after = params.snapshot();
    assert!(before.iter().zip(&after).any(|(a, b)| a != b), "something moved");
    for p in [P::Voices, P::Mode, P::Arp_On, P::Unison, P::A_Type, P::Level, P::Chord, P::Hold, P::Mac1_DA, P::Mac1_AA] {
        assert_eq!(before[p as usize], after[p as usize], "{:?} is left alone", p);
    }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hydra_test_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_saved_preset_lands_in_its_folder_and_comes_back_whole() {
    let dir = temp("save");
    let params = Params::new();
    params.set(P::F1_Cut, 1234.0);
    params.set(P::Chord, 5.0);
    let macros = ["Warm".to_string(), "Grit".to_string(), String::new(), "Air".to_string()];
    let path = save_user_in(&dir, "Bass", "My Wobble", &params.snapshot(), &macros).unwrap();
    assert!(path.starts_with(dir.join("Bass")), "{path:?}");
    let loaded = load_user_in(&dir);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].category, "Bass");
    assert_eq!(loaded[0].name, "My Wobble");
    assert!(loaded[0].user);
    assert_eq!(loaded[0].macros[1], "Grit");
    let back = Params::new();
    apply(&back, &loaded[0]);
    assert_eq!(back.get(P::F1_Cut), 1234.0);
    assert_eq!(back.get(P::Chord), 5.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_dropped_into_a_folder_shows_up_and_a_damaged_one_is_ignored() {
    let dir = temp("drop");
    std::fs::create_dir_all(dir.join("Pad")).unwrap();
    std::fs::write(dir.join("Pad").join("Hand Made.json"), r#"{"name":"Hand Made","values":[0.5]}"#).unwrap();
    std::fs::write(dir.join("Pad").join("Broken.json"), "not json").unwrap();
    std::fs::write(dir.join("Pad").join("notes.txt"), "ignore me").unwrap();
    let loaded = load_user_in(&dir);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "Hand Made");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn user_presets_sit_after_the_factory_ones_in_their_folder() {
    let mut mine = Preset::new("Mine", vec![]);
    mine.category = "Bass".into();
    mine.user = true;
    let lib = Library::with(factory(), vec![mine], HashSet::new());
    let bass = lib.in_folder("Bass");
    assert_eq!(lib.presets[*bass.last().unwrap()].name, "Mine");
    assert_eq!(lib.place("Bass", *bass.last().unwrap()), (bass.len(), bass.len()));
}

#[test]
fn folders_list_favorites_first_and_only_folders_with_sounds() {
    let lib = Library::with(factory(), vec![], HashSet::new());
    let f = lib.folders();
    assert_eq!(f[0], "Favorites");
    assert_eq!(f[1], "Init");
    assert!(f.contains(&"Wavetable".to_string()));
    assert!(lib.in_folder("Favorites").is_empty(), "nothing is a favorite yet");
}

#[test]
fn favorites_collect_into_their_own_folder() {
    let mut lib = Library::with(factory(), vec![], HashSet::new());
    let pad = lib.in_folder("Pad")[2];
    let bass = lib.in_folder("Bass")[0];
    // (toggling writes saves/hydra/favorites.json, which these tests must not touch)
    lib.favorites.insert(lib.key(pad));
    lib.favorites.insert(lib.key(bass));
    assert!(lib.is_favorite(pad));
    assert_eq!(lib.in_folder("Favorites"), vec![bass, pad]);
}

#[test]
fn a_user_preset_with_the_same_name_is_replaced_not_duplicated() {
    let mut lib = Library::with(factory(), vec![], HashSet::new());
    let mut a = Preset::new("User 1", vec![(P::Level as usize, 0.2)]);
    a.category = "Lead".into();
    a.user = true;
    let first = lib.add_user(a.clone());
    let n = lib.presets.len();
    a.values = vec![(P::Level as usize, 0.9)];
    let second = lib.add_user(a);
    assert_eq!(first, second);
    assert_eq!(lib.presets.len(), n);
    assert_eq!(lib.presets[second].values[0].1, 0.9);
}
