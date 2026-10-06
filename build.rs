// Compiles Mutable Instruments' DSP (vendor/eurorack, MIT-licensed)
// plus our small FFI bridges (vendor/bridge) into static libs linked
// into the Rust binary: Plaits and Clouds from the lists below, every
// other module from its `vendor/bridge/<name>.sources` list. Both file lists and the
// `-DTEST` flag come straight from each project's own
// vendor/eurorack/<module>/test/makefile -- that's the eurorack
// project's own standalone/desktop build of this code, not a guess on
// our part. Plaits drops two files from its list (packet_decoder.cc,
// user_data_receiver.cc) since they're for firmware-update-over-audio
// and pull in a package we haven't vendored, and we're not using it.

fn main() {
    generate_app_modules();
    apply_vendor_patches();
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

    compile_listed_bridges(eurorack, bridge);
    compile_airwindows(bridge);
    compile_o_c_firmwares();
}

/// The O&C-family firmwares (stock Ornaments & Crimes, Hemisphere Suite,
/// Phazerville Suite...), each built as a shared library the host loads a fresh
/// copy of per module -- the firmwares keep their state in globals, so a copy
/// per module is the isolation. Each variant's own source is compiled as published
/// (patched files listed in its PATCHES.md) against the shared host stand-in for the
/// Teensy and the module (vendor/o_c/host); `sketch.cpp` is an Arduino sketch's
/// .ino files in one translation unit. The libraries land in OUT_DIR/ocfw and
/// the Rust side finds them through `PORTAMAX_OCFW_DIR`.
fn compile_o_c_firmwares() {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=vendor/o_c");
    for v in O_C_VARIANTS {
        println!("cargo:rerun-if-changed=vendor/{}", v.dir);
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ocfw");
    std::fs::create_dir_all(&out).unwrap();
    println!("cargo:rustc-env=PORTAMAX_OCFW_DIR={}", out.display());
    fn newest(dir: &std::path::Path) -> u128 {
        let mut t = 0;
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            t = t.max(if p.is_dir() { newest(&p) } else { e.metadata().and_then(|m| m.modified()).ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0) });
        }
        t
    }
    let ext = if cfg!(target_os = "macos") { "dylib" } else { "so" };
    let shared_host = root.join("vendor/o_c/host");
    for v in O_C_VARIANTS {
        let base = root.join("vendor").join(v.dir);
        if !base.is_dir() {
            continue;
        }
        let lib = out.join(format!("libocfw_{}.{ext}", v.id));
        let stamp = out.join(format!("{}.stamp", v.id));
        let fingerprint = format!("{}-{}", newest(&base), newest(&shared_host));
        if lib.exists() && std::fs::read_to_string(&stamp).ok().as_deref() == Some(fingerprint.as_str()) {
            continue;
        }
        let mut build = cc::Build::new();
        build.cpp(true).std(v.cxx_std).warnings(false).flag("-w").flag("-Wno-c++11-narrowing").flag("-fno-rtti"); // the Teensy build has no RTTI
        build.define("F_CPU", Some("120000000")).define("typeof", Some("__typeof__"));
        for inc in std::iter::once(shared_host.display().to_string()).chain(v.includes.iter().map(|d| base.join(d).display().to_string())) {
            build.include(inc);
        }
        build.file(shared_host.join("oc_host_core.cpp"));
        for f in v.sources {
            build.file(base.join(f));
        }
        for (k, val) in v.defines {
            build.define(k, *val);
        }
        let objects = build.compile_intermediates();
        let mut link = build.get_compiler().to_command();
        link.arg(if cfg!(target_os = "macos") { "-dynamiclib" } else { "-shared" }).arg("-o").arg(&lib);
        for o in &objects {
            link.arg(o);
        }
        if cfg!(target_os = "macos") {
            link.arg("-lc++");
        } else {
            link.arg("-lstdc++").arg("-lpthread");
        }
        let status = link.status().expect("run the linker");
        assert!(status.success(), "linking {} failed", lib.display());
        std::fs::write(&stamp, fingerprint).unwrap();
    }
}

struct OcVariant {
    id: &'static str,
    dir: &'static str,
    includes: &'static [&'static str],
    sources: &'static [&'static str],
    defines: &'static [(&'static str, Option<&'static str>)],
    cxx_std: &'static str,
}

const O_C_VARIANTS: &[OcVariant] = &[OcVariant {
    id: "stock",
    dir: "o_c",
    includes: &["host", "fw", "fw/src/drivers", "fw/extern"],
    sources: &[
        "sketch.cpp", "host/oc_host_fw.cpp", "fw/OC_autotune.cpp", "fw/OC_bitmaps.cpp", "fw/OC_chords.cpp", "fw/OC_debug.cpp", "fw/OC_digital_inputs.cpp", "fw/OC_input_map.cpp",
        "fw/OC_menus.cpp", "fw/OC_patterns.cpp", "fw/OC_scales.cpp", "fw/OC_strings.cpp", "fw/OC_ui.cpp", "fw/bjorklund.cpp", "fw/braids_quantizer.cpp", "fw/frames_poly_lfo.cpp",
        "fw/frames_resources.cpp", "fw/peaks_bytebeat.cpp", "fw/peaks_multistage_envelope.cpp", "fw/peaks_resources.cpp", "fw/streams_lorenz_generator.cpp",
        "fw/streams_resources.cpp", "fw/src/drivers/weegfx.cpp", "fw/src/util/util_misc.cpp",
    ],
    defines: &[],
    cxx_std: "c++14",
},
OcVariant {
    id: "hemi",
    dir: "o_c_hemi",
    includes: &["host", "fw", "fw/src/drivers", "fw/extern", "."],
    sources: &[
        "sketch.cpp", "host/oc_host_fw.cpp", "fw/OC_autotune.cpp", "fw/OC_bitmaps.cpp", "fw/OC_debug.cpp", "fw/OC_digital_inputs.cpp", "fw/OC_input_map.cpp",
        "fw/OC_menus.cpp", "fw/OC_patterns.cpp", "fw/OC_scales.cpp", "fw/OC_strings.cpp", "fw/OC_ui.cpp", "fw/bjorklund.cpp", "fw/braids_quantizer.cpp",
        "fw/peaks_bytebeat.cpp", "fw/peaks_multistage_envelope.cpp", "fw/peaks_resources.cpp", "fw/streams_lorenz_generator.cpp",
        "fw/streams_resources.cpp", "fw/src/drivers/weegfx.cpp", "fw/src/util/util_misc.cpp",
    ],
    defines: &[],
    cxx_std: "c++14",
},
OcVariant {
    id: "phaz",
    dir: "o_c_phaz",
    includes: &["host", "fw", "fw/src/drivers", "fw/extern", "fw/src"],
    sources: &["host/oc_host_fw.cpp", "fw/HSIOFrame.cpp", "fw/HSUtils.cpp", "fw/HemisphereApplet.cpp", "fw/Main.cpp", "fw/OC_apps.cpp", "fw/OC_autotune.cpp", "fw/OC_bitmaps.cpp", "fw/OC_calibration.cpp", "fw/OC_chords.cpp", "fw/OC_core.cpp", "fw/OC_debug.cpp", "fw/OC_digital_inputs.cpp", "fw/OC_gpio.cpp", "fw/OC_input_map.cpp", "fw/OC_menus.cpp", "fw/OC_patterns.cpp", "fw/OC_scales.cpp", "fw/OC_strings.cpp", "fw/OC_ui.cpp", "fw/bjorklund.cpp", "fw/braids_quantizer.cpp", "fw/frames_poly_lfo.cpp", "fw/frames_resources.cpp", "fw/peaks_bytebeat.cpp", "fw/peaks_multistage_envelope.cpp", "fw/peaks_resources.cpp", "fw/src/drivers/weegfx.cpp", "fw/src/util/util_misc.cpp", "fw/streams_lorenz_generator.cpp", "fw/streams_resources.cpp", "fw/tideslite.cpp"],
    cxx_std: "c++17",
    defines: &[("USB_MIDI", None), ("ENABLE_APP_CALIBR8OR", None), ("ENABLE_APP_SCENES", None), ("ENABLE_APP_PONG", None), ("ENABLE_APP_PIQUED", None), ("PEWPEWPEW", None)],
}];

/// Airwindows' ~500 effects (vendor/airwindows, MIT) as one static lib.
/// Each plugin is a VST2 class; vendor/airwindows/shim/audioeffectx.h
/// stands in for the SDK. Every effect defines the same global
/// `createEffectInstance`, so each gets its own generated translation unit
/// that renames it (`aw_make_<Name>`) and includes the effect's own .cpp
/// files; a generated registry lists them all for vendor/bridge/
/// airwindows_bridge.cc. Adding an effect is dropping its folder into
/// vendor/airwindows/src.
fn compile_airwindows(bridge: &str) {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = root.join("vendor/airwindows/src");
    println!("cargo:rerun-if-changed=vendor/airwindows");
    let Ok(entries) = std::fs::read_dir(&src) else { return };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !n.starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    names.sort();
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("airwindows");
    std::fs::create_dir_all(&out).unwrap();
    let write_if_changed = |path: &std::path::Path, text: &str| {
        if std::fs::read_to_string(path).ok().as_deref() != Some(text) {
            std::fs::write(path, text).unwrap();
        }
    };
    let mut files = Vec::new();
    // Only the generated wrappers are listed files; a change to the shim or
    // to any plugin source reaches the fingerprint through the registry's text.
    fn newest(dir: &std::path::Path) -> u128 {
        let mut t = 0;
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            t = t.max(if p.is_dir() { newest(&p) } else { e.metadata().and_then(|m| m.modified()).ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0) });
        }
        t
    }
    let stamp = newest(&root.join("vendor/airwindows"));
    let mut registry = format!("// sources {stamp}\n#include \"audioeffectx.h\"\nstruct AwEntry {{ const char* id; AudioEffect* (*make)(audioMasterCallback); }};\n");
    let mut table = String::new();
    let mut count = 0;
    for name in &names {
        let dir = src.join(name);
        let mut cpps: Vec<String> = std::fs::read_dir(&dir)
            .map(|d| d.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|f| f.ends_with(".cpp")).collect())
            .unwrap_or_default();
        // The effect's own file first: it declares the class the Proc file extends.
        cpps.sort_by_key(|f| (f != &format!("{name}.cpp"), f.clone()));
        if cpps.is_empty() || !cpps[0].starts_with(name.as_str()) {
            continue;
        }
        let mut tu = format!("#define createEffectInstance aw_make_{name}\n");
        for f in &cpps {
            tu.push_str(&format!("#include \"{}/{f}\"\n", dir.display()));
        }
        let path = out.join(format!("aw_{name}.cc"));
        write_if_changed(&path, &tu);
        files.push(path.display().to_string());
        registry.push_str(&format!("AudioEffect* aw_make_{name}(audioMasterCallback);\n"));
        table.push_str(&format!("  {{\"{name}\", aw_make_{name}}},\n"));
        count += 1;
    }
    registry.push_str(&format!("extern const AwEntry aw_registry[] = {{\n{table}}};\nextern const int aw_registry_len = {count};\n"));
    let reg = out.join("aw_registry.cc");
    write_if_changed(&reg, &registry);
    files.push(reg.display().to_string());
    files.push(format!("{bridge}/airwindows_bridge.cc"));
    compile_cached_with("airwindows_bridge", &[&format!("{}", root.join("vendor/airwindows/shim").display())], &[], &[], &files);
}

/// Every other bridge is self-describing: `vendor/bridge/<name>.sources`
/// lists its eurorack sources (one per line, `#` comments allowed) and
/// `vendor/bridge/<name>_bridge.cc` is its C ABI, compiled together into
/// `lib<name>_bridge.a`. Adding a Mutable Instruments module is dropping
/// those two files in -- nothing here to edit.
fn compile_listed_bridges(eurorack: &str, bridge: &str) {
    println!("cargo:rerun-if-changed={bridge}");
    let Ok(entries) = std::fs::read_dir(bridge) else { return };
    let mut lists: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "sources")).collect();
    lists.sort();
    for list in lists {
        println!("cargo:rerun-if-changed={}", list.display());
        let name = list.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
        let text = std::fs::read_to_string(&list).unwrap_or_default();
        let mut files: Vec<String> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| format!("{eurorack}/{l}"))
            .collect();
        files.push(format!("{bridge}/{name}_bridge.cc"));
        // Optional `<name>.includes`: extra include directories, relative to the repo root.
        let extra: Vec<String> = std::fs::read_to_string(list.with_extension("includes"))
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect();
        let mut includes: Vec<&str> = vec![eurorack];
        includes.extend(extra.iter().map(String::as_str));
        // Optional `<name>.defines`: one `NAME` or `NAME=VALUE` per line (replacing
        // the default -DTEST); a line starting with `-` is passed as a raw compiler flag.
        let text = std::fs::read_to_string(list.with_extension("defines")).ok();
        let (mut defines, mut flags): (Vec<(String, Option<String>)>, Vec<String>) = (Vec::new(), Vec::new());
        match &text {
            None => defines.push(("TEST".into(), None)),
            Some(t) => {
                for l in t.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
                    if l.starts_with('-') {
                        flags.push(l.to_string());
                    } else if let Some((k, v)) = l.split_once('=') {
                        defines.push((k.to_string(), Some(v.to_string())));
                    } else {
                        defines.push((l.to_string(), None));
                    }
                }
            }
        }
        let define_refs: Vec<(&str, Option<&str>)> = defines.iter().map(|(k, v)| (k.as_str(), v.as_deref())).collect();
        let flag_refs: Vec<&str> = flags.iter().map(String::as_str).collect();
        compile_cached_with(&format!("{name}_bridge"), &includes, &define_refs, &flag_refs, &files);
    }
}

/// Compiles one C++ static lib with the eurorack test-build flags, but only
/// when its sources (or CXXFLAGS) changed since the last build. The build
/// script also reruns whenever an app file changes (see
/// `generate_app_modules`), and recompiling the DSP libs every time would
/// cost a minute per edit.
fn compile_cached(lib: &str, include: &str, files: &[String]) {
    compile_cached_with(lib, &[include], &[("TEST", None)], &[], files);
}

fn compile_cached_with(lib: &str, includes: &[&str], defines: &[(&str, Option<&str>)], flags: &[&str], files: &[String]) {
    // Plain C files (a vendored library's one .c file) can't go through the
    // C++ compiler; they get a C static lib of their own.
    let (c_files, cpp_files): (Vec<String>, Vec<String>) = files.iter().cloned().partition(|f| f.ends_with(".c"));
    if !c_files.is_empty() {
        compile_one(&format!("{lib}_c"), false, includes, defines, flags, &c_files);
    }
    compile_one(lib, true, includes, defines, flags, &cpp_files);
}

fn compile_one(lib: &str, cpp: bool, includes: &[&str], defines: &[(&str, Option<&str>)], flags: &[&str], files: &[String]) {
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
    build.cpp(cpp).warnings(false);
    if cpp {
        build.std("c++14");
        // The vendored sources lean on <cstdio>/<cstring> arriving through other
        // headers, which clang's libc++ does and GCC's libstdc++ no longer does,
        // so a Linux build fails with "printf was not declared". Pulling them in
        // everywhere costs nothing and needs no change to vendored files.
        if std::env::var("TARGET").is_ok_and(|t| t.contains("linux")) {
            for h in ["cstdio", "cstring", "cstdlib"] {
                build.flag("-include").flag(h);
            }
        }
    }
    for (k, v) in defines {
        build.define(k, *v);
    }
    for i in includes {
        build.include(i);
    }
    for f in flags {
        build.flag(f);
    }
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

/// Reapplies the fixes `vendor/PATCHES.md` lists for `vendor/eurorack`, which
/// is an upstream git submodule: a fix made inside it can't be committed here,
/// so a fresh checkout would otherwise build the unpatched source. Each patch
/// is a literal replacement that does nothing once applied (or if upstream
/// changed that code), so it is safe to run on every build.
fn apply_vendor_patches() {
    // tides2/ramp/ramp_extractor.cc: wrapping `expected_phase` below 1 never
    // terminates when `period` is 0 (it is infinite), which hung Stages' PLL
    // oscillator. fmodf gives the same value for every finite input.
    let path = "vendor/eurorack/tides2/ramp/ramp_extractor.cc";
    let old = "            while (expected_phase >= 1.0f) {\n              expected_phase -= 1.0f;\n            }\n";
    let new = "            expected_phase = std::isfinite(expected_phase) ? fmodf(expected_phase, 1.0f) : 0.0f;\n";
    if let Ok(text) = std::fs::read_to_string(path) {
        if text.contains(old) {
            let text = text.replace(old, new);
            let text = if text.contains("#include <cmath>") { text } else { text.replacen("#include <algorithm>", "#include <algorithm>\n#include <cmath>", 1) };
            std::fs::write(path, text).expect("patching vendor/eurorack");
        }
    }
}
