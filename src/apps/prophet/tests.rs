use super::*;
use crate::app::MidiKeys;
fn app() -> ProphetApp {
    ProphetApp::with_dirs(
        Arc::new(ModBus::new()),
        Arc::new(AudioBus::new()),
        Arc::new(MixerBus::new()),
        Path::new("/tmp/no-rev2-import"),
        Path::new("/tmp/no-rev2-saves"),
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
fn render(p: &mut dyn AudioProcessor, time: f32, sr: f32) -> Vec<f32> {
    let n = (time * sr) as usize * 2;
    let mut out = Vec::with_capacity(n);
    let mut b = [0.; 512];
    while out.len() < n {
        p.process(&mut b, 2, sr);
        out.extend_from_slice(&b);
    }
    out.truncate(n);
    out
}
fn energy(x: &[f32]) -> f32 {
    x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32
}
fn play(patch: Patch, notes: &[(usize, u8)], time: f32) -> Vec<f32> {
    let mut a = app();
    a.state.patch = patch;
    a.tick(&input(notes));
    let mut p = a.audio_processor().unwrap();
    render(p.as_mut(), time, 48000.)
}
#[test]
fn edit_buffer_and_program_banks_preserve_all_bytes() {
    let mut p = Patch::default();
    for i in 0..patch::RAW_LEN {
        p.data[i] = (i.wrapping_mul(73) % 256) as u8;
    }
    for location in [None, Some((0, 0)), Some((7, 127))] {
        let bytes = p.export(location).unwrap();
        assert_eq!(bytes.len(), if location.is_some() { 2346 } else { 2344 });
        let q = patch::import(&bytes).unwrap()[0];
        assert_eq!(&q.data[..patch::RAW_LEN], &p.data[..patch::RAW_LEN]);
        assert_eq!(q.location, location);
        assert_eq!(q.export(location).unwrap(), bytes);
    }
}
#[test]
fn independent_packed_vector_uses_lsb_first_msb_mask() {
    let p = Patch::default();
    let mut bytes = p.export(None).unwrap();
    bytes[4..12].copy_from_slice(&[0x55, 1, 2, 3, 4, 5, 6, 7]);
    let p = patch::import(&bytes).unwrap()[0];
    assert_eq!(&p.data[..7], &[129, 2, 131, 4, 133, 6, 135]);
}
#[test]
fn banks_are_atomic_and_realtime_interleaving_is_accepted() {
    let p = Patch::default();
    let good = p.export(Some((3, 120))).unwrap();
    let mut bank = good.clone();
    bank.extend(&good);
    assert_eq!(patch::import(&bank).unwrap().len(), 2);
    bank.pop();
    assert!(patch::import(&bank).is_err());
    let mut interrupted = Vec::new();
    for &b in &good {
        interrupted.push(b);
        interrupted.push(0xf8);
    }
    assert_eq!(patch::import(&interrupted).unwrap()[0].data, p.data);
}
#[test]
fn malformed_wrong_model_and_short_dumps_are_rejected() {
    let p = Patch::default().export(None).unwrap();
    for n in [0, 1, 2, 3, 100, 2343] {
        assert!(patch::import(&p[..n]).is_err());
    }
    for (i, v) in [(1, 0x43), (2, 0x23), (3, 5), (20, 0x80), (2340, 0x7f)] {
        let mut bad = p.clone();
        bad[i] = v;
        assert!(patch::import(&bad).is_err(), "{i}");
    }
}
#[test]
fn reserved_bytes_and_b_layer_names_survive() {
    let mut p = Patch::default();
    p.set_name(0, "Test A");
    p.set_name(1, "Test B");
    p.data[121] = 0x8d;
    p.data[1024 + 121] = 0xf1;
    p.data[2045] = 219;
    let q = patch::import(&p.export(None).unwrap()).unwrap()[0];
    assert_eq!(q.name(0), "Test A");
    assert_eq!(q.name(1), "Test B");
    assert_eq!(q.data[121], 0x8d);
    assert_eq!(q.data[1145], 0xf1);
    assert_eq!(q.data[2045], 219);
    assert_eq!(&q.data[2046..], &[0, 0]);
}
#[test]
fn nrpn_uses_a_different_layout_to_sysex() {
    assert_eq!(spec::nrpn_offset(15), Some(22));
    assert_eq!(spec::nrpn_offset(2063), Some(1046));
    assert_eq!(spec::nrpn_offset(0), Some(0));
    assert_eq!(spec::nrpn_offset(5), Some(1));
    assert_eq!(spec::nrpn_offset(276), Some(256));
    assert_eq!(spec::nrpn_offset(3091), Some(2047));
    assert_eq!(spec::nrpn_offset(27), None);
    assert_eq!(spec::nrpn_offset(2211), None);
    assert_eq!(spec::nrpn_offset(2219), None);
}
#[test]
fn pads_and_midi_play_and_release() {
    let mut a = app();
    let mut p = a.audio_processor().unwrap();
    let idle = render(p.as_mut(), 0.03, 48000.);
    assert!(energy(&idle) < 1e-12);
    let mut i = Input::default();
    i.grid[12] = true;
    a.tick(&i);
    assert!(energy(&render(p.as_mut(), 0.3, 48000.)) > 1e-5);
    a.tick(&Input::default());
    let tail = render(p.as_mut(), 2., 48000.);
    assert!(energy(&tail[tail.len() - 4096..]) < 1e-10);
    a.tick(&input(&[(69, 110)]));
    assert!(energy(&render(p.as_mut(), 0.1, 44100.)) > 1e-5);
    a.on_exit();
    let tail = render(p.as_mut(), 2., 44100.);
    assert!(energy(&tail[tail.len() - 4096..]) < 1e-10);
}
#[test]
fn every_starting_point_plays_finite_audio_at_device_rates() {
    for sr in [8000., 32000., 44100., 48000., 96000., 192000.] {
        for index in 0..16 {
            let mut a = app();
            a.load(index);
            a.state.playing = index == 13;
            a.tick(&input(&[(60, 100), (64, 100), (67, 100)]));
            let mut p = a.audio_processor().unwrap();
            let audio = render(p.as_mut(), 0.3, sr);
            assert!(
                audio.iter().all(|v| v.is_finite() && v.abs() <= 1.),
                "preset {index} {sr}"
            );
            assert!(energy(&audio) > 1e-10, "silent {index} {sr}");
        }
    }
}
#[test]
fn split_and_stack_route_b_instead_of_discarding_it() {
    let mut p = Patch::default();
    p.data[4] = 0;
    p.data[5] = 0;
    p.data[231] = 2;
    p.data[232] = 60;
    assert!(energy(&play(p, &[(48, 100)], 0.2)) < 1e-10);
    assert!(energy(&play(p, &[(72, 100)], 0.2)) > 1e-5);
    p.data[231] = 1;
    assert!(energy(&play(p, &[(48, 100)], 0.2)) > 1e-5);
    p.data[231] = 0;
    assert!(energy(&play(p, &[(72, 100)], 0.2)) < 1e-10);
}
#[test]
fn filter_poles_shape_mod_sub_and_sync_change_the_sound() {
    let base = Patch::default();
    let reference = play(base, &[(60, 100)], 0.2);
    for (i, v) in [(26, 0), (6, 15), (15, 100), (17, 1), (4, 4), (21, 100)] {
        let mut p = base;
        p.data[i] = v;
        if i == 17 {
            p.data[1] = 36;
        }
        let sound = play(p, &[(60, 100)], 0.2);
        let difference: Vec<_> = sound.iter().zip(&reference).map(|(a, b)| a - b).collect();
        assert!(energy(&difference) > 1e-8, "parameter {i}");
    }
}
#[test]
fn each_lfo_and_mod_slot_has_an_audible_path() {
    let base = Patch::default();
    let ref_audio = play(base, &[(60, 100)], 0.3);
    for l in 0..4 {
        let mut p = base;
        p.data[53 + l] = 75;
        p.data[61 + l] = 30;
        p.data[65 + l] = 10;
        let audio = play(p, &[(60, 100)], 0.3);
        let d: Vec<_> = audio.iter().zip(&ref_audio).map(|(a, b)| a - b).collect();
        assert!(energy(&d) > 1e-8);
    }
    for slot in 0..8 {
        let mut p = base;
        p.data[77 + slot] = 21;
        p.data[85 + slot] = 155;
        p.data[93 + slot] = 1;
        let audio = play(p, &[(60, 100)], 0.3);
        let d: Vec<_> = audio.iter().zip(&ref_audio).map(|(a, b)| a - b).collect();
        assert!(energy(&d) > 1e-8);
    }
}
#[test]
fn auxiliary_envelope_destination_and_loop_change_audio() {
    let mut p = Patch::default();
    let ref_audio = play(p, &[(60, 100)], 0.5);
    p.data[30] = 1;
    p.data[34] = 190;
    p.data[43] = 5;
    p.data[46] = 40;
    p.data[49] = 0;
    p.data[52] = 40;
    p.data[31] = 1;
    let audio = play(p, &[(60, 100)], 0.5);
    let d: Vec<_> = audio.iter().zip(&ref_audio).map(|(a, b)| a - b).collect();
    assert!(energy(&d) > 1e-7);
}
#[test]
fn all_effects_have_finite_distinct_output() {
    let base = Patch::default();
    let reference = play(base, &[(60, 100)], 0.5);
    for kind in 1..=13 {
        let mut p = base;
        p.data[115] = kind;
        p.data[116] = 1;
        p.data[117] = 100;
        p.data[118] = 40;
        p.data[119] = 80;
        let audio = play(p, &[(60, 100)], 0.5);
        assert!(audio.iter().all(|v| v.is_finite() && v.abs() <= 1.));
        let d: Vec<_> = audio.iter().zip(&reference).map(|(a, b)| a - b).collect();
        assert!(energy(&d) > 1e-8, "FX {kind}");
    }
}
#[test]
fn poly_sequencer_plays_without_held_notes_and_stops() {
    let mut a = app();
    a.load(13);
    a.toggle_running();
    let mut p = a.audio_processor().unwrap();
    let audio = render(p.as_mut(), 1., 48000.);
    assert!(energy(&audio) > 1e-5);
    a.toggle_running();
    let tail = render(p.as_mut(), 2., 48000.);
    assert!(energy(&tail[tail.len() - 4096..]) < 1e-10);
}
#[test]
fn arp_and_gated_sequence_change_held_chords() {
    let base = Patch::default();
    let reference = play(base, &[(60, 100), (64, 100), (67, 100)], 0.5);
    for gated in [false, true] {
        let mut p = base;
        if gated {
            p.data[139] = 0;
            p.data[131] = 6;
            p.data[111] = 3;
            for i in 0..16 {
                p.data[140 + i] = (i * 7) as u8;
            }
        } else {
            p.data[136] = 1;
            p.data[131] = 6;
        }
        let audio = play(p, &[(60, 100), (64, 100), (67, 100)], 0.5);
        let d: Vec<_> = audio.iter().zip(&reference).map(|(a, b)| a - b).collect();
        assert!(energy(&d) > 1e-8);
    }
}
#[test]
fn hold_transpose_and_negative_pad_transpose_work() {
    let mut a = app();
    a.state.hold = true;
    a.tick(&input(&[(60, 100)]));
    a.tick(&Input::default());
    assert_eq!(a.state.notes[60], 100);
    a.state.hold = false;
    a.held = [0; 128];
    a.transpose = -12;
    let mut i = Input::default();
    i.grid[12] = true;
    a.tick(&i);
    assert!(a.state.notes.iter().any(|v| *v > 0));
    a.tick(&Input::default());
    assert_eq!(a.state.notes, [0; 128]);
}
#[test]
fn step_recording_writes_six_note_velocities() {
    let mut a = app();
    a.record = true;
    a.tick(&input(&[
        (60, 100),
        (64, 90),
        (67, 80),
        (72, 70),
        (76, 60),
        (79, 50),
    ]));
    for t in 0..6 {
        assert_eq!(
            a.state.patch.data[256 + t * 128],
            [60, 64, 67, 72, 76, 79][t]
        );
        assert_eq!(
            a.state.patch.data[320 + t * 128],
            [228, 218, 208, 198, 188, 178][t]
        );
    }
    assert_eq!(a.record_step, 1);
}
#[test]
fn file_bank_scan_and_save_reload_are_real() {
    let dir = std::env::temp_dir().join(format!("rev2-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut p = Patch::default();
    p.set_name(1, "Imported B");
    let mut bank = p.export(Some((2, 127))).unwrap();
    bank.extend(p.export(None).unwrap());
    std::fs::write(dir.join("bank.syx"), bank).unwrap();
    let mut a = ProphetApp::with_dirs(
        Arc::new(ModBus::new()),
        Arc::new(AudioBus::new()),
        Arc::new(MixerBus::new()),
        &dir,
        &dir.join("save"),
    );
    assert_eq!(a.programs.len(), 18);
    a.load(16);
    assert_eq!(a.state.patch.name(1), "Imported B");
    a.state.patch.data[1024 + 22] = 91;
    a.slot = 511;
    a.save().unwrap();
    let data = std::fs::read(dir.join("save/U4-128.syx")).unwrap();
    let saved = patch::import(&data).unwrap()[0];
    assert_eq!(saved.data[1046], 91);
    assert_eq!(saved.location, Some((3, 127)));
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn layer_copy_swap_and_compare_retain_sequences() {
    let mut a = app();
    a.state.patch.data[256] = 71;
    a.copy_layer(false, true);
    assert_eq!(a.state.patch.data[1280], 71);
    a.state.patch.data[0] = 36;
    a.copy_layer(false, false);
    assert_eq!(a.state.patch.data[1024], 36);
    a.state.patch.data[1024] = 48;
    a.copy_layer(true, false);
    assert_eq!(a.state.patch.data[0], 48);
    let edited = a.state.patch;
    a.compare();
    assert_eq!(a.state.patch, a.baseline);
    a.compare();
    assert_eq!(a.state.patch, edited);
}
#[test]
fn busy_locks_and_large_blocks_never_require_allocation_or_waiting() {
    let mut a = app();
    a.tick(&input(&[(60, 100)]));
    let mut p = a.audio_processor().unwrap();
    render(p.as_mut(), 0.1, 48000.);
    let state = a.shared.state.lock().unwrap();
    let bus = a.shared.output.lock().unwrap();
    let mut b = [0.; 512];
    p.process(&mut b, 2, 48000.);
    assert!(energy(&b) > 0.);
    drop(state);
    drop(bus);
    let cap = a.shared.output.lock().unwrap().capacity();
    let mut b = vec![0.; cap * 4];
    p.process(&mut b, 2, 48000.);
    assert_eq!(a.shared.output.lock().unwrap().capacity(), cap);
    for sr in [f32::NAN, 0., 1.] {
        p.process(&mut b, 2, sr);
        assert!(b.iter().all(|v| *v == 0.));
    }
    p.process(&mut b, 0, 48000.);
}
#[test]
fn panel_controls_have_usable_labels_and_layer_specific_offsets() {
    let mut a = app();
    for section in 0..16 {
        a.section = section;
        for layer in 0..2 {
            a.layer = layer;
            let rows = a.rows();
            assert!(!rows.is_empty());
            assert!(rows.iter().all(|r| !r.name.is_empty()));
            for row in rows {
                if let Some(i) = row.offset {
                    assert!(i < 2048);
                }
            }
            let mut f = FrameBuffer::new();
            a.draw(&mut f);
            assert!(f.buffer().iter().any(|v| *v != 0));
        }
    }
}
#[test]
fn parameter_limits_cover_hardware_endpoints_and_sequence_sentinels() {
    let mut p = Patch::default();
    for layer in [0, 1024] {
        for (offset, max) in [
            (117, 127),
            (118, 255),
            (119, 127),
            (124, 16),
            (140, 127),
            (156, 127),
            (172, 127),
            (188, 127),
            (53, 150),
        ] {
            assert!(p.set(layer + offset, max).is_ok());
            if max < 255 {
                assert!(p.set(layer + offset, max + 1).is_err());
            }
        }
        assert!(p.set(layer + 130, 30).is_ok());
        assert!(p.set(layer + 130, 29).is_err());
    }
    assert_eq!(spec::LAYER_MODES, &["A", "Stack A/B", "Split A/B"]);
    assert_eq!(spec::text(140, 126), "Reset");
    assert_eq!(spec::text(140, 127), "Rest");
    assert_eq!(spec::text(320, 127), "Reset");
    assert_eq!(spec::text(320, 128), "Rest");
    assert_eq!(spec::text(320, 255), "Velocity 127");
}
#[test]
fn slint_controls_edit_real_parameters_page_layers_and_release_keys() {
    let mut a = app();
    for section in 0..16 {
        a.slint_pointer_pick(1000. + section as f32, 0.);
        let SlintExtra::Rev2(view) = a.slint_extra() else {
            panic!("dedicated panel missing")
        };
        assert_eq!(view.section, section);
        assert!(!view.labels.is_empty());
        assert_eq!(view.labels.len(), view.values.len());
        assert_eq!(view.labels.len(), view.norms.len());
        assert!(view.labels.len() <= 4);
        for page in 0..view.pages {
            let SlintExtra::Rev2(v) = a.slint_extra() else {
                unreachable!()
            };
            assert_eq!(v.page, page);
            assert!(v.selected < v.labels.len());
            a.slint_pointer_pick(1400., 1.);
        }
    }
    a.slint_pointer_pick(1001., 0.);
    let before = a.state.patch.data[22];
    a.slint_pointer_pick(1100., 1.);
    assert_eq!(a.state.patch.data[22], before + 1);
    a.slint_pointer_pick(1205., 0.);
    let before_b = a.state.patch.data[1046];
    a.slint_pointer_pick(1100., -1.);
    assert_eq!(a.state.patch.data[1046], before_b - 1);
    a.slint_pointer_pick(1300., 1.);
    assert!(a.state.notes[48] > 0);
    a.slint_pointer_pick(1300., 0.);
    assert_eq!(a.state.notes[48], 0);
    for section in 10..=13 {
        a.slint_pointer_pick(1000. + section as f32, 0.);
        for page in 0..4 {
            assert_eq!(a.seq_page, page);
            assert!(a
                .rows()
                .last()
                .unwrap()
                .name
                .ends_with(&format!("{:02}", page * 16 + 16)));
            a.slint_pointer_pick(1207., 0.);
        }
    }
    for track in 0..6 {
        assert_eq!(a.track, track);
        assert!(a.rows()[0]
            .name
            .starts_with(&format!("Track {}", track + 1)));
        a.slint_pointer_pick(1208., 0.);
    }
}
#[test]
fn sequence_nrpn_uses_the_same_velocity_encoding_as_sysex() {
    let mut a = app();
    for message in [[0xb0, 99, 2], [0xb0, 98, 84], [0xb0, 6, 1], [0xb0, 38, 127]] {
        a.receive(&message);
    }
    assert_eq!(a.state.patch.data[320], 255);
    assert_eq!(spec::text(320, a.state.patch.data[320]), "Velocity 127");
    a.receive(&[0xb0, 38, 0]);
    assert_eq!(a.state.patch.data[320], 128);
    assert_eq!(spec::text(320, a.state.patch.data[320]), "Rest");
}
#[test]
fn multi_mode_routes_channels_without_applying_the_split_point() {
    let mut a = app();
    a.state.multi = true;
    a.state.patch.data[231] = 2; // Split must not override Multi's channel routing.
    a.state.patch.data[28] = 0;
    a.state.midi_notes[1][48] = 100; // Below split, on layer B's MIDI channel.
    a.publish();
    let mut p = a.audio_processor().unwrap();
    assert!(energy(&render(p.as_mut(), 0.2, 48000.)) > 1e-5);
}
#[test]
fn alternate_pan_modulates_width_and_fixed_pan_moves_the_program() {
    let mut p = Patch::default();
    p.data[77] = 21;
    p.data[85] = 254;
    p.data[93] = 14;
    for mode in [0, 1] {
        p.data[209] = mode;
        let audio = play(p, &[(60, 100), (64, 100)], 0.2);
        let left = energy(&audio.iter().step_by(2).copied().collect::<Vec<_>>());
        let right = energy(&audio.iter().skip(1).step_by(2).copied().collect::<Vec<_>>());
        assert!(right > 1e-5);
        if mode == 0 {
            assert!(left > 1e-5);
        } else {
            assert!(left < 1e-10);
        }
    }
}
#[test]
#[ignore = "external Edisyn init dump: REV2_REFERENCE_SYX=/path/file.syx"]
fn reference_dump_imports_roundtrips_and_plays() {
    let path = std::env::var("REV2_REFERENCE_SYX").unwrap();
    let bytes = std::fs::read(path).unwrap();
    let patches = patch::import(&bytes).unwrap();
    assert!(!patches.is_empty());
    let p = patches[0];
    assert_eq!(p.export(p.location).unwrap(), bytes);
    assert_eq!(p.name(0), "Basic Program A");
    assert_eq!(p.name(1), "Basic Program B");
    assert!(energy(&play(p, &[(60, 100)], 0.5)) > 1e-6);
}
#[test]
fn direct_midi_nrpn_cc_sustain_and_channel_filter_are_connected() {
    let mut a = app();
    for msg in [[0xb0, 99, 16], [0xb0, 98, 15], [0xb0, 6, 0], [0xb0, 38, 83]] {
        a.receive(&msg);
    }
    assert_eq!(a.state.patch.data[1046], 83);
    a.receive(&[0xb0, 102, 75]);
    assert_eq!(a.state.patch.data[22], 75);
    a.receive(&[0xb0, 1, 100]);
    assert!(a.state.wheel > 0.7);
    a.receive(&[0x90, 60, 100]);
    a.receive(&[0xb0, 64, 127]);
    a.receive(&[0x80, 60, 0]);
    assert_eq!(a.state.midi_notes[0][60], 100);
    a.receive(&[0xb0, 64, 0]);
    assert_eq!(a.state.midi_notes[0][60], 0);
    a.midi_channel = 1;
    a.receive(&[0x91, 62, 100]);
    assert_eq!(a.state.midi_notes[0][62], 0);
    a.state.multi = true;
    a.receive(&[0x91, 62, 100]);
    assert_eq!(a.state.midi_notes[1][62], 100);
    let mut p = a.audio_processor().unwrap();
    assert!(energy(&render(p.as_mut(), 0.2, 48000.)) > 1e-5);
}
#[test]
fn midi_increment_decrement_and_null_selection_work() {
    let mut d = midi::Decoder::default();
    assert_eq!(d.message(&[0xb0, 99, 0]), None);
    d.message(&[0xb0, 98, 15]);
    assert_eq!(
        d.message(&[0xb0, 38, 50]),
        Some(midi::Event::Parameter(15, 50))
    );
    assert_eq!(
        d.message(&[0xb0, 96, 0]),
        Some(midi::Event::Parameter(15, 51))
    );
    assert_eq!(
        d.message(&[0xb0, 97, 0]),
        Some(midi::Event::Parameter(15, 50))
    );
    d.message(&[0xb0, 101, 127]);
    d.message(&[0xb0, 100, 127]);
    assert_eq!(
        d.message(&[0xb0, 38, 80]),
        Some(midi::Event::Control(0, 38, 80))
    );
}
#[test]
fn all_alternative_tunings_are_finite_and_selected_tunings_change_pitch() {
    for i in 0..17 {
        assert!(tuning::table(i).iter().all(|v| v.is_finite()));
    }
    let equal = tuning::table(0);
    assert!((equal[69] - 69.).abs() < 1e-5);
    assert!((tuning::table(4)[70] - 69.5).abs() < 1e-4);
    assert!((tuning::table(15)[67] - 67.).abs() < 0.01);
    let mut a = app();
    a.state.tuning = tuning::table(4);
    a.tick(&input(&[(72, 100)]));
    let mut p = a.audio_processor().unwrap();
    let audio = render(p.as_mut(), 0.2, 48000.);
    let base = play(Patch::default(), &[(72, 100)], 0.2);
    let difference: Vec<_> = audio.iter().zip(base).map(|(a, b)| a - b).collect();
    assert!(energy(&difference) > 1e-5);
}
#[test]
fn external_patch_via_live_midi_uses_both_layers() {
    let mut a = app();
    let mut p = Patch::default();
    p.set_name(0, "Live");
    p.set_name(1, "B Live");
    p.data[1046] = 83;
    a.receive(&p.export(None).unwrap());
    assert_eq!(a.state.patch.name(0), "Live");
    assert_eq!(a.state.patch.name(1), "B Live");
    assert_eq!(a.state.patch.data[1046], 83);
}
#[test]
#[ignore = "temporary visual/audio review: REV2_REVIEW_DIR=/absolute/path"]
fn export_review_frames_and_audio() {
    let dir = PathBuf::from(std::env::var("REV2_REVIEW_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut a = app();
    for section in 0..16 {
        a.section = section;
        a.layer = section % 2;
        let mut frame = FrameBuffer::new();
        a.draw(&mut frame);
        let rgb: Vec<u8> = frame
            .buffer()
            .iter()
            .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8])
            .collect();
        let file = std::fs::File::create(dir.join(format!("rev2-{section:02}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, 640, 360);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgb)
            .unwrap();
    }
    let mut wav = hound::WavWriter::create(
        dir.join("Rev2-Demo.wav"),
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for index in [0, 1, 3, 7, 11, 13] {
        a.load(index);
        a.state.playing = index == 13;
        a.tick(&input(&[(60, 100), (64, 95), (67, 90)]));
        let mut p = a.audio_processor().unwrap();
        for x in render(p.as_mut(), 1.5, 48000.) {
            wav.write_sample((x * 32767.) as i16).unwrap();
        }
        a.tick(&Input::default());
        a.state.playing = false;
        a.publish();
        for x in render(p.as_mut(), 0.5, 48000.) {
            wav.write_sample((x * 32767.) as i16).unwrap();
        }
    }
    wav.finalize().unwrap();
}
