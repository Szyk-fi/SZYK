//! Thin safe wrapper around the real Mutable Instruments Clouds
//! granular processor (vendor/eurorack, compiled by build.rs via the
//! bridge in vendor/bridge/clouds_bridge.cc).
//!
//! Clouds processes internally at a fixed 32kHz stereo
//! (`clouds::kMaxBlockSize`-chunked) -- see vendor/eurorack/clouds/
//! dsp/frame.h. Unlike Plaits (a pure generator, needing a persistent
//! streaming resampler across blocks), Clouds is block-in/block-out:
//! each `render` call resamples that block's mono input down to
//! 32kHz, runs it through the real engine, and resamples the stereo
//! result back up to the device rate, folding L/R to mono to match
//! every other app in this sim. Resampling restarts at each block's
//! own t=0 rather than carrying a phase across calls -- correct here
//! (unlike Plaits) because a block's input/output genuinely start
//! fresh at that block's time offset; there's no free-running
//! generator phase to keep continuous.

use std::ffi::c_void;
use std::os::raw::c_int;

const CLOUDS_SAMPLE_RATE: f32 = 32000.0;

unsafe extern "C" {
    fn clouds_processor_create() -> *mut c_void;
    fn clouds_processor_destroy(handle: *mut c_void);
    #[allow(clippy::too_many_arguments)]
    fn clouds_processor_render(
        handle: *mut c_void,
        playback_mode: c_int,
        position: f32,
        size: f32,
        pitch: f32,
        density: f32,
        texture: f32,
        dry_wet: f32,
        stereo_spread: f32,
        feedback: f32,
        reverb: f32,
        freeze: c_int,
        trigger: c_int,
        in_interleaved: *const f32,
        out_interleaved: *mut f32,
        num_frames: c_int,
    );
}

/// Mirrors clouds::Parameters, reduced to what this app drives (the
/// same knobs exposed on the real module's panel).
pub struct CloudsParams {
    pub playback_mode: i32, // 0=Granular, 1=Stretch, 2=Looping Delay, 3=Spectral
    pub position: f32,      // 0..1
    pub size: f32,          // 0..1
    pub pitch: f32,         // semitones, -48..48 (real hardware's own range)
    pub density: f32,       // 0..1
    pub texture: f32,       // 0..1
    pub dry_wet: f32,       // 0..1
    pub stereo_spread: f32, // 0..1
    pub feedback: f32,      // 0..1
    pub reverb: f32,        // 0..1
    pub freeze: bool,
    pub trigger: bool,
}

pub struct Granulator {
    handle: *mut c_void,
    in_interleaved: Vec<f32>,
    out_interleaved: Vec<f32>,
}

// The handle is a heap pointer with no shared mutable state outside
// what we control here; only ever touched from the audio callback
// thread.
unsafe impl Send for Granulator {}

impl Granulator {
    pub fn new() -> Self {
        let handle = unsafe { clouds_processor_create() };
        Self { handle, in_interleaved: Vec::new(), out_interleaved: Vec::new() }
    }

    /// `input` and `output` are mono at `device_rate`, same length.
    pub fn render(&mut self, input: &[f32], output: &mut [f32], device_rate: f32, params: &CloudsParams) {
        let frames = input.len();
        let clouds_frames = ((frames as f32) * CLOUDS_SAMPLE_RATE / device_rate).round().max(1.0) as usize;

        self.in_interleaved.clear();
        self.in_interleaved.resize(clouds_frames * 2, 0.0);
        let in_step = device_rate / CLOUDS_SAMPLE_RATE; // device-rate samples per clouds-rate sample
        for i in 0..clouds_frames {
            let src_pos = i as f32 * in_step;
            let i0 = src_pos as usize;
            let frac = src_pos - i0 as f32;
            let s0 = input.get(i0).copied().unwrap_or(0.0);
            let s1 = input.get(i0 + 1).copied().unwrap_or(s0);
            let s = s0 + (s1 - s0) * frac;
            self.in_interleaved[i * 2] = s;
            self.in_interleaved[i * 2 + 1] = s;
        }

        self.out_interleaved.clear();
        self.out_interleaved.resize(clouds_frames * 2, 0.0);

        unsafe {
            clouds_processor_render(
                self.handle,
                params.playback_mode,
                params.position,
                params.size,
                params.pitch,
                params.density,
                params.texture,
                params.dry_wet,
                params.stereo_spread,
                params.feedback,
                params.reverb,
                params.freeze as c_int,
                params.trigger as c_int,
                self.in_interleaved.as_ptr(),
                self.out_interleaved.as_mut_ptr(),
                clouds_frames as c_int,
            );
        }

        let out_step = CLOUDS_SAMPLE_RATE / device_rate; // clouds-rate samples per device-rate sample
        let last = clouds_frames.saturating_sub(1);
        for (j, out) in output.iter_mut().enumerate() {
            let src_pos = j as f32 * out_step;
            let i0 = (src_pos as usize).min(last);
            let i1 = (i0 + 1).min(last);
            let frac = (src_pos - i0 as f32).clamp(0.0, 1.0);
            let l0 = self.out_interleaved[i0 * 2];
            let r0 = self.out_interleaved[i0 * 2 + 1];
            let l1 = self.out_interleaved[i1 * 2];
            let r1 = self.out_interleaved[i1 * 2 + 1];
            let l = l0 + (l1 - l0) * frac;
            let r = r0 + (r1 - r0) * frac;
            *out = (l + r) * 0.5;
        }
    }
}

impl Drop for Granulator {
    fn drop(&mut self) {
        unsafe { clouds_processor_destroy(self.handle) };
    }
}
