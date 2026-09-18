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
    let eurorack = "vendor/eurorack";
    let bridge = "vendor/bridge";

    println!("cargo:rerun-if-changed={bridge}/plaits_bridge.cc");
    println!("cargo:rerun-if-changed={bridge}/clouds_bridge.cc");

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

    let mut plaits_build = cc::Build::new();
    plaits_build
        .cpp(true)
        .std("c++14")
        .define("TEST", None)
        .include(eurorack)
        .warnings(false);

    for src in plaits_sources {
        plaits_build.file(format!("{eurorack}/{src}"));
    }
    plaits_build.file(format!("{bridge}/plaits_bridge.cc"));

    plaits_build.compile("plaits_bridge");

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

    let mut clouds_build = cc::Build::new();
    clouds_build
        .cpp(true)
        .std("c++14")
        .define("TEST", None)
        .include(eurorack)
        .warnings(false);

    for src in clouds_sources {
        clouds_build.file(format!("{eurorack}/{src}"));
    }
    clouds_build.file(format!("{bridge}/clouds_bridge.cc"));

    clouds_build.compile("clouds_bridge");
}
