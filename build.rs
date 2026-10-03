// Compiles Mutable Instruments' Plaits and Clouds DSP (vendor/eurorack,
// MIT-licensed) plus our small FFI bridges (vendor/bridge) into two
// static libs linked into the Rust binary. Both file lists and the
// `-DTEST` flag come straight from each project's own
// vendor/eurorack/<module>/test/makefile -- that's the eurorack
// project's own standalone/desktop build of this code, not a guess on
// our part. Plaits drops two files from its list (packet_decoder.cc,
// user_data_receiver.cc) since they're for firmware-update-over-audio
// and pull in a package we haven't vendored, and we're not using it.

fn main() {
    generate_app_modules();
    let eurorack = "vendor/eurorack";
    let bridge = "vendor/bridge";


    let plaits_sources: &[&str] = &[
        "plaits/dsp/fm/algorithms.cc",
        "plaits/dsp/engine/additive_engine.cc",
        "plaits/dsp/engine/bass_drum_engine.cc",
        "plaits/dsp/engine2/chiptune_engine.cc",
        "plaits/dsp/chords/chord_bank.cc",
        "plaits/dsp/engine/chord_engine.cc",
        "plaits/dsp/fm/dx_units.cc",
        "plaits/dsp/engine/fm_engine.cc",
        "plaits/dsp/engine/grain_engine.cc",
        "plaits/dsp/engine/hi_hat_engine.cc",
        "plaits/dsp/speech/lpc_speech_synth.cc",
        "plaits/dsp/speech/lpc_speech_synth_controller.cc",
        "plaits/dsp/speech/lpc_speech_synth_phonemes.cc",
        "plaits/dsp/speech/lpc_speech_synth_words.cc",
        "plaits/dsp/engine/modal_engine.cc",
        "plaits/dsp/physical_modelling/modal_voice.cc",
        "plaits/dsp/speech/naive_speech_synth.cc",
        "plaits/dsp/engine/noise_engine.cc",
        "plaits/dsp/engine/particle_engine.cc",
        "plaits/dsp/engine2/phase_distortion_engine.cc",
        "stmlib/utils/random.cc",
        "plaits/dsp/physical_modelling/resonator.cc",
        "plaits/resources.cc",
        "plaits/dsp/speech/sam_speech_synth.cc",
        "plaits/dsp/engine2/six_op_engine.cc",
        "plaits/dsp/engine/snare_drum_engine.cc",
        "plaits/dsp/engine/speech_engine.cc",
        "plaits/dsp/physical_modelling/string.cc",
        "plaits/dsp/engine/string_engine.cc",
        "plaits/dsp/engine2/string_machine_engine.cc",
        "plaits/dsp/physical_modelling/string_voice.cc",
        "plaits/dsp/engine/swarm_engine.cc",
        "stmlib/dsp/units.cc",
        "plaits/dsp/engine/virtual_analog_engine.cc",
        "plaits/dsp/engine2/virtual_analog_vcf_engine.cc",
        "plaits/dsp/voice.cc",
        "plaits/dsp/engine/waveshaping_engine.cc",
        "plaits/dsp/engine/wavetable_engine.cc",
        "plaits/dsp/engine2/wave_terrain_engine.cc",
    ];

    let mut files: Vec<String> = plaits_sources.iter().map(|s| format!("{eurorack}/{s}")).collect();
    files.push(format!("{bridge}/plaits_bridge.cc"));
    compile_cached("plaits_bridge", eurorack, &files);

    // clouds/test/makefile's CC_FILES, minus clouds_test.cc itself (our
    // own bridge replaces it). The fx/ classes (reverb, pitch_shifter,
    // diffuser) are header-only templates, so nothing to compile there.
    let clouds_sources: &[&str] = &[
        "stmlib/dsp/atan.cc",
        "clouds/dsp/correlator.cc",
        "clouds/dsp/granular_processor.cc",
        "clouds/dsp/mu_law.cc",
        "stmlib/utils/random.cc",
        "clouds/resources.cc",
        "clouds/dsp/pvoc/frame_transformation.cc",
        "clouds/dsp/pvoc/phase_vocoder.cc",
        "clouds/dsp/pvoc/stft.cc",
        "stmlib/dsp/units.cc",
    ];

    let mut files: Vec<String> = clouds_sources.iter().map(|s| format!("{eurorack}/{s}")).collect();
    files.push(format!("{bridge}/clouds_bridge.cc"));
    compile_cached("clouds_bridge", eurorack, &files);
}

/// Compiles one C++ static lib with the eurorack test-build flags, but only
/// when its sources (or CXXFLAGS) changed since the last build. The build
/// script also reruns whenever an app file changes (see
/// `generate_app_modules`), and recompiling the DSP libs every time would
/// cost a minute per edit.
fn compile_cached(lib: &str, include: &str, files: &[String]) {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-env-changed=CXXFLAGS");
    let mut fingerprint = std::env::var("CXXFLAGS").unwrap_or_default();
    for f in files {
        println!("cargo:rerun-if-changed={f}");
        let t = std::fs::metadata(f).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
        fingerprint.push_str(&format!("\n{f}:{t}"));
    }
    let stamp = out.join(format!("{lib}.stamp"));
    let archive = out.join(format!("lib{lib}.a"));
    if archive.exists() && std::fs::read_to_string(&stamp).ok().as_deref() == Some(fingerprint.as_str()) {
        let target = std::env::var("TARGET").unwrap_or_default();
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static={lib}");
        println!("cargo:rustc-link-lib={}", if target.contains("apple") || target.contains("freebsd") { "c++" } else { "stdc++" });
        return;
    }
    let mut build = cc::Build::new();
    build.cpp(true).std("c++14").define("TEST", None).include(include).warnings(false);
    for f in files {
        build.file(f);
    }
    build.compile(lib);
    std::fs::write(stamp, fingerprint).ok();
}

/// Finds every app module under src/apps and writes `apps_gen.rs` (included
/// by src/apps/mod.rs): a `pub mod` for each, plus `FACTORIES`, the table of
/// modules that export `pub fn create(&AppContext, &str)`. Adding an app is
/// dropping its file (or folder) into src/apps and a manifest into apps/ --
/// nothing central to edit. A file pulled into another module with
/// `#[path = ...]` or `include!` is that module's private part, not an app.
fn generate_app_modules() {
    use std::fmt::Write as _;
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dir = root.join("src/apps");
    println!("cargo:rerun-if-changed=src/apps");
    let mut modules: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("src/apps").flatten() {
        let path = entry.path();
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        if path.is_dir() {
            let m = path.join("mod.rs");
            if m.exists() {
                modules.push((name, m));
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") && name != "mod" {
            modules.push((name, path));
        }
    }
    // private parts of other modules
    let mut private = std::collections::HashSet::new();
    for (_, path) in &modules {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        for line in text.lines() {
            let l = line.trim();
            let quoted = |l: &str| l.split('"').nth(1).map(|f| f.trim_end_matches(".rs").to_string());
            if l.starts_with("#[path") || l.starts_with("include!(") {
                if let Some(f) = quoted(l) {
                    private.insert(f);
                }
            }
        }
    }
    modules.retain(|(name, _)| !private.contains(name));
    modules.sort();
    let mut out = String::from("// @generated by build.rs from the files in src/apps -- do not edit.\n");
    let mut factories = Vec::new();
    for (name, path) in &modules {
        println!("cargo:rerun-if-changed={}", path.display());
        writeln!(out, "#[path = {:?}]\npub mod {name};", path.display().to_string()).unwrap();
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if text.contains("pub fn create(ctx: &crate::app::AppContext") {
            factories.push(name.clone());
        }
    }
    out.push_str("\n/// Every module that can build an app, by module name. A manifest names\n/// one with `module = ...` (default: its id).\n#[allow(dead_code)]\npub const FACTORIES: &[(&str, crate::app::AppFactory)] = &[\n");
    for f in &factories {
        writeln!(out, "    ({f:?}, {f}::create),").unwrap();
    }
    out.push_str("];\n");
    let dest = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("apps_gen.rs");
    if std::fs::read_to_string(&dest).ok().as_deref() != Some(out.as_str()) {
        std::fs::write(dest, out).unwrap();
    }
}
