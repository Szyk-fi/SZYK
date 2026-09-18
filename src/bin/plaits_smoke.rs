//! Standalone stress test for the Plaits FFI bridge, run directly
//! (`cargo run --bin plaits_smoke`) so a crash's stack trace isn't
//! tangled up with the audio thread or the window event loop. Not part
//! of the app -- just a debugging tool while chasing a reported crash.

use std::ffi::c_void;
use std::os::raw::c_int;

// Same source file main.rs uses (via `mod plaits_ffi;`), included directly
// rather than through a separate lib target -- a lib+bin split caused the
// static lib's link instructions to not reach the main binary; this stays
// a single self-contained crate, same as main.rs already is.
#[path = "../plaits_ffi.rs"]
mod plaits_ffi;

unsafe extern "C" {
    fn plaits_voice_create() -> *mut c_void;
    fn plaits_voice_destroy(handle: *mut c_void);
    fn plaits_voice_render(
        handle: *mut c_void,
        engine: c_int,
        note: f32,
        harmonics: f32,
        timbre: f32,
        morph: f32,
        decay: f32,
        lpg_colour: f32,
        trigger: f32,
        level: f32,
        out: *mut f32,
        size: c_int,
    );
}

fn render_block(handle: *mut c_void, engine: i32, note: f32, trigger: f32, out: &mut [f32]) {
    unsafe {
        plaits_voice_render(
            handle,
            engine,
            note,
            0.5,
            0.5,
            0.5,
            0.5,
            0.5,
            trigger,
            1.0,
            out.as_mut_ptr(),
            out.len() as c_int,
        );
    }
}

fn stage1_raw_bridge_stress() {
    println!("=== stage 1: raw bridge stress (already passed once) ===");
    let handle = unsafe { plaits_voice_create() };
    let mut buf = vec![0.0f32; 32];
    for engine in 0..24 {
        for _ in 0..50 {
            render_block(handle, engine, 48.0, 1.0, &mut buf);
        }
    }
    unsafe { plaits_voice_destroy(handle) };
    println!("stage 1 ok");
}

fn stage2_repeated_create_destroy() {
    println!("=== stage 2: repeated create/destroy (simulates entering/exiting the app) ===");
    let mut buf = vec![0.0f32; 32];
    for i in 0..200 {
        let handle = unsafe { plaits_voice_create() };
        render_block(handle, 8, 48.0, 1.0, &mut buf);
        render_block(handle, 8, 48.0, 1.0, &mut buf);
        unsafe { plaits_voice_destroy(handle) };
        if i % 20 == 0 {
            println!("  cycle {i} ok");
        }
    }
    println!("stage 2 ok");
}

fn stage3_realistic_block_sizes_via_wrapper() {
    println!("=== stage 3: PlaitsVoice wrapper (resampler) with realistic block sizes ===");
    use crate::plaits_ffi::{PlaitsParams, PlaitsVoice};

    for block_size in [64usize, 256, 512, 1024, 4096] {
        println!("  block_size={block_size}");
        let mut voice = PlaitsVoice::new();
        let mut out = vec![0.0f32; block_size];
        let params = PlaitsParams {
            engine: 8,
            note: 48.0,
            harmonics: 0.5,
            timbre: 0.5,
            morph: 0.5,
            decay: 0.5,
            lpg_colour: 0.5,
            trigger: true,
        };
        for i in 0..100 {
            // Try a few different device rates, including ones that
            // don't divide 48000 evenly.
            let rate = match i % 3 {
                0 => 44100.0,
                1 => 48000.0,
                _ => 96000.0,
            };
            voice.render(&mut out, rate, &params);
        }
        println!("  block_size={block_size} ok, last sample={}", out[0]);
    }
    println!("stage 3 ok");
}

fn stage4_repeated_wrapper_create_destroy() {
    println!("=== stage 4: repeated PlaitsVoice wrapper create/destroy at realistic sizes ===");
    use crate::plaits_ffi::{PlaitsParams, PlaitsVoice};

    let mut out = vec![0.0f32; 512];
    let params = PlaitsParams {
        engine: 8,
        note: 48.0,
        harmonics: 0.5,
        timbre: 0.5,
        morph: 0.5,
        decay: 0.5,
        lpg_colour: 0.5,
        trigger: true,
    };
    for i in 0..200 {
        let mut voice = PlaitsVoice::new();
        voice.render(&mut out, 44100.0, &params);
        voice.render(&mut out, 44100.0, &params);
        drop(voice);
        if i % 20 == 0 {
            println!("  cycle {i} ok");
        }
    }
    println!("stage 4 ok");
}

fn main() {
    stage1_raw_bridge_stress();
    stage2_repeated_create_destroy();
    stage3_realistic_block_sizes_via_wrapper();
    stage4_repeated_wrapper_create_destroy();
    println!("ALL STAGES PASSED");
}
