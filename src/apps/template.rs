//! A minimal, real, working app -- copy this file as the starting point
//! for a new one. It is NOT wired into `registry.rs` or any
//! `apps/<id>/manifest.toml`, so it never appears in the actual running
//! app list; its own test below is what keeps it honest (compiled and
//! exercised by `cargo test`) without it being a real, selectable app.
//! See `docs/ADDING_AN_APP.md` for exactly what changes turn a copy of
//! this file into a real, installed app.
//!
//! What this app actually does, deliberately kept tiny: pad 0 held
//! plays a single sine tone; knob1 changes its pitch; knob2 changes its
//! volume. That's the whole thing -- real audio, real controls, real
//! screen, nothing hidden or stubbed out. Every piece here is something
//! a real app needs; extend from here rather than starting over.
//!
//! `#![allow(dead_code)]` below is deliberate and specific to this
//! file being a reference, not a shipped app: everything here is real
//! and genuinely exercised by its own test (`cargo test`), just never
//! constructed by `registry.rs`'s ordinary `cargo build`. Delete the
//! allow once you register your copy for real -- a genuinely unused
//! warning in a real app means something's actually wrong.
#![allow(dead_code)]

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::util::AtomicF32;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const MIN_FREQ: f32 = 55.0;
const MAX_FREQ: f32 = 880.0;
const DEFAULT_FREQ: f32 = 220.0;
const DEFAULT_VOLUME: f32 = 0.5;
const ATTACK_SECONDS: f32 = 0.008; // fade in/out on note on/off, avoids clicks -- see SynthApp for the real reason (a hard on/off step is an audible click)

/// The app's own real, shared state. `Arc<AtomicF32>` (see `util.rs`)
/// is this project's standard way to share a plain float between the
/// UI thread (`tick`/`draw`, called from the main loop) and the audio
/// thread (`AudioProcessor::process`, called from the real-time audio
/// callback) without a lock on the hot path -- reach for a `Mutex`
/// only when you need something richer than one number (see `held`
/// below, or `SynthApp`'s per-voice array).
pub struct TemplateApp {
    freq: Arc<AtomicF32>,
    volume: Arc<AtomicF32>,
    held: Arc<AtomicBool>,
}

// This app's own palette -- every app picks its own colors rather than
// sharing one device-wide theme (see SynthApp's own version of this
// comment for the reasoning).
const BG: Rgb565 = Rgb565::new(2, 4, 6);
const TITLE: Rgb565 = Rgb565::new(10, 25, 31);
const DIM: Rgb565 = Rgb565::new(8, 16, 20);

impl TemplateApp {
    /// The constructor's argument list is exactly what `registry.rs`
    /// will need to pass in -- this one needs nothing shared with any
    /// other app, so it takes nothing. An app that reads/writes a
    /// value other apps also touch (Settings' sensitivity, a global
    /// filter cutoff) takes that as an `Arc<...>` parameter instead,
    /// same as `SynthApp::new`'s `cutoff` -- ask before assuming you
    /// need one; most new apps don't.
    pub fn new() -> Self {
        Self { freq: Arc::new(AtomicF32::new(DEFAULT_FREQ)), volume: Arc::new(AtomicF32::new(DEFAULT_VOLUME)), held: Arc::new(AtomicBool::new(false)) }
    }
}

impl App for TemplateApp {
    /// Real menu rows for the connected Slint UI (see
    /// `examples/slint_home_live.rs`) -- optional (default is empty),
    /// but every real app should have this: it's the only thing the
    /// generic `ParamListColumn` screen has to show while this app
    /// hasn't earned its own bespoke Slint screen yet.
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![("Frequency".into(), format!("{:.0} Hz", self.freq.get()), false), ("Volume".into(), format!("{:.0}%", self.volume.get() * 100.0), false)]
    }

    /// Called once per frame regardless of whether the app is making
    /// sound -- this is where controls get read, never where audio
    /// gets rendered (that's `audio_processor`'s job, on a different
    /// thread with a real-time deadline).
    fn tick(&mut self, input: &Input) {
        self.held.store(input.grid[0], Ordering::Relaxed);

        if input.knob1 != 0 {
            let next = (self.freq.get() * 1.05f32.powf(input.knob1 as f32)).clamp(MIN_FREQ, MAX_FREQ);
            self.freq.set(next);
        }
        if input.knob2 != 0 {
            let next = (self.volume.get() + input.knob2 as f32 * 0.05).clamp(0.0, 1.0);
            self.volume.set(next);
        }
        if input.knob1_press {
            self.freq.set(DEFAULT_FREQ);
        }
    }

    /// Called once, lazily, the first time this app's processor is
    /// actually needed (see this method's own doc comment on the
    /// `App` trait for the dormant-until-woken mechanics) -- return
    /// `None` if this app makes no sound at all (a pure utility/editor
    /// app, e.g. MIDI Learn).
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(TemplateProcessor { freq: Arc::clone(&self.freq), volume: Arc::clone(&self.volume), held: Arc::clone(&self.held), phase: 0.0, amp: 0.0 }))
    }

    /// The `embedded_graphics`-based screen (`main.rs`'s own window,
    /// not the Slint preview) -- every app needs this even if
    /// `slint_rows` above covers the Slint side, since `main.rs` is
    /// still the primary way this whole simulator runs.
    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32)).into_styled(PrimitiveStyle::with_fill(BG)).draw(fb).ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, TITLE);
        Text::new("Template", Point::new(20, 30), title).draw(fb).ok();

        let dim = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        Text::new(&format!("Frequency {:.0} Hz (knob 1)", self.freq.get()), Point::new(20, 60), dim).draw(fb).ok();
        Text::new(&format!("Volume {:.2} (knob 2)", self.volume.get()), Point::new(20, 75), dim).draw(fb).ok();
        Text::new("Pad 0: hold to play. Knob1 press: reset frequency.", Point::new(20, 340), dim).draw(fb).ok();
    }
}

/// The real-time audio side -- owns nothing the UI side (`TemplateApp`)
/// also owns exclusively; every field here is an `Arc` clone shared
/// with it, read fresh every block. Never locks anything that could
/// block for long (see `AudioProcessor`'s own doc comment on why a
/// stall here is an audible glitch, not just a bug).
struct TemplateProcessor {
    freq: Arc<AtomicF32>,
    volume: Arc<AtomicF32>,
    held: Arc<AtomicBool>,
    phase: f32,
    amp: f32,
}

impl AudioProcessor for TemplateProcessor {
    /// `buffer` is interleaved by `channels` (e.g. `[L, R, L, R, ...]`
    /// for stereo) -- `chunks_mut(channels)` below is the standard way
    /// every app in this codebase walks it one frame at a time.
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let freq = self.freq.get();
        let volume = self.volume.get();
        let held = self.held.load(Ordering::Relaxed);
        let envelope_coef = 1.0 - (-1.0 / (sample_rate * ATTACK_SECONDS)).exp();

        for frame in buffer.chunks_mut(channels) {
            let target = if held { 1.0 } else { 0.0 };
            self.amp += (target - self.amp) * envelope_coef;
            let sample = (self.phase * TAU).sin() * self.amp * volume;
            self.phase = (self.phase + freq / sample_rate).fract();
            for out in frame.iter_mut() {
                *out = sample;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Not a correctness check on the sine math (that's simple enough
    /// not to need one) -- this is what keeps this file compiling and
    /// actually running as real code even though it's never registered
    /// as a real app, per this file's own module doc comment.
    #[test]
    fn a_held_pad_produces_real_nonzero_audio_and_a_released_one_goes_quiet() {
        let mut app = TemplateApp::new();
        let mut processor = app.audio_processor().expect("this app makes sound, so this must be Some");

        app.tick(&Input { grid: std::array::from_fn(|i| i == 0), ..Default::default() });
        let mut buffer = vec![0.0f32; 512];
        for _ in 0..20 {
            processor.process(&mut buffer, 1, 48_000.0);
        }
        assert!(buffer.iter().any(|&s| s != 0.0), "holding pad 0 must produce real, audible output");

        app.tick(&Input::default());
        for _ in 0..20 {
            processor.process(&mut buffer, 1, 48_000.0);
        }
        assert!(buffer.iter().all(|&s| s.abs() < 0.001), "releasing the pad must let the envelope decay back to silence");
    }
}
