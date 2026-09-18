//! Thin safe wrapper around the real Mutable Instruments Plaits voice
//! (vendor/eurorack, compiled by build.rs into a static lib via the
//! bridge in vendor/bridge/plaits_bridge.cc). This is the boundary where
//! "real DSP, real C++" meets the rest of this otherwise pure-Rust sim.
//!
//! Plaits renders internally at a fixed 48kHz (`plaits::kSampleRate`) --
//! see plaits/dsp/dsp.h. Whatever the actual output device's rate is,
//! `PlaitsVoice::render` resamples (linear interpolation) from 48kHz to
//! it, rather than letting pitch drift with the device's sample rate.

use std::ffi::c_void;
use std::os::raw::c_int;

const PLAITS_SAMPLE_RATE: f32 = 48000.0;
/// How many 48kHz-rate samples to render per refill. Matches
/// plaits::kMaxBlockSize is not required here -- the bridge chunks
/// internally -- this is just a reasonable batch size to amortize the
/// FFI call overhead.
const REFILL_SIZE: usize = 32;

unsafe extern "C" {
    fn plaits_voice_create() -> *mut c_void;
    fn plaits_voice_destroy(handle: *mut c_void);
    #[allow(clippy::too_many_arguments)]
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

/// Parameters for one render call — mirrors plaits::Patch/Modulations,
/// reduced to what this app actually drives.
pub struct PlaitsParams {
    pub engine: i32,
    pub note: f32,
    pub harmonics: f32,
    pub timbre: f32,
    pub morph: f32,
    pub decay: f32,
    pub lpg_colour: f32,
    pub trigger: bool,
}

pub struct PlaitsVoice {
    handle: *mut c_void,
    src: Vec<f32>,
    /// Read position into `src`, in source-sample units (fractional).
    phase: f32,
}

// The handle is a heap pointer with no shared mutable state outside what
// we control here; only ever touched from the audio callback thread.
unsafe impl Send for PlaitsVoice {}

impl PlaitsVoice {
    pub fn new() -> Self {
        let handle = unsafe { plaits_voice_create() };
        Self {
            handle,
            src: Vec::new(),
            phase: 0.0,
        }
    }

    fn refill(&mut self, params: &PlaitsParams) {
        // Fixed-size, so this is a stack array, not a heap allocation --
        // `refill` runs per voice, roughly every REFILL_SIZE output
        // samples consumed, so up to 16x per block in Poly mode. A fresh
        // `Vec` here on every one of those calls was a real source of
        // audio-thread jitter.
        let mut chunk = [0.0f32; REFILL_SIZE];
        let trigger = if params.trigger { 8.0 } else { 0.0 };
        unsafe {
            plaits_voice_render(
                self.handle,
                params.engine,
                params.note,
                params.harmonics,
                params.timbre,
                params.morph,
                params.decay,
                params.lpg_colour,
                trigger,
                1.0,
                chunk.as_mut_ptr(),
                chunk.len() as c_int,
            );
        }
        self.src.extend_from_slice(&chunk);
    }

    /// Renders `out.len()` mono samples at `device_rate`, resampled from
    /// Plaits' internal 48kHz.
    pub fn render(&mut self, out: &mut [f32], device_rate: f32, params: &PlaitsParams) {
        let ratio = PLAITS_SAMPLE_RATE / device_rate;

        for sample in out.iter_mut() {
            while self.src.len() < 2 || (self.phase as usize + 1) >= self.src.len() {
                self.refill(params);
            }
            let i = self.phase as usize;
            let frac = self.phase - i as f32;
            *sample = self.src[i] + (self.src[i + 1] - self.src[i]) * frac;

            self.phase += ratio;
            // Trim consumed history so `src` doesn't grow forever.
            if self.phase >= (REFILL_SIZE as f32) {
                let drop = (self.phase as usize).saturating_sub(1).min(self.src.len().saturating_sub(2));
                self.src.drain(0..drop);
                self.phase -= drop as f32;
            }
        }
    }
}

impl Drop for PlaitsVoice {
    fn drop(&mut self) {
        unsafe { plaits_voice_destroy(self.handle) };
    }
}
