//! A simple polyphonic synth: the 4x4 grid is a 16-note keyboard (held,
//! not toggled — press and hold to sustain, release to stop), knob1
//! edits cutoff, knob2 edits volume, and knob2 press cycles the
//! waveform (used to be the 4 top buttons, but F1-F4 are global OS
//! quick-nav now -- see os.rs -- so this app never sees them).
//! No microphone input — this app is its own sound source.

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::util::AtomicF32;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const BASE_FREQ: f32 = 220.0; // A3 -- grid key 0
const MIN_CUTOFF: f32 = 100.0;
const MAX_CUTOFF: f32 = 8000.0;
const DEFAULT_CUTOFF: f32 = 1000.0;
const DEFAULT_VOLUME: f32 = 0.8;
const ATTACK_SECONDS: f32 = 0.008; // fade in/out on note on/off, avoids clicks

#[derive(Clone, Copy)]
enum Waveform {
    Sine,
    Square,
    Saw,
    Triangle,
}

impl Waveform {
    fn from_index(i: u32) -> Self {
        match i % 4 {
            0 => Waveform::Sine,
            1 => Waveform::Square,
            2 => Waveform::Saw,
            _ => Waveform::Triangle,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Waveform::Sine => "Sine",
            Waveform::Square => "Square",
            Waveform::Saw => "Saw",
            Waveform::Triangle => "Triangle",
        }
    }

    fn sample(&self, phase: f32) -> f32 {
        match self {
            Waveform::Sine => (phase * TAU).sin(),
            Waveform::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Saw => 2.0 * phase - 1.0,
            Waveform::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
        }
    }
}

pub struct SynthApp {
    waveform: Arc<AtomicU32>,
    cutoff: Arc<AtomicF32>,
    volume: Arc<AtomicF32>,
    held: Arc<Mutex<[bool; 16]>>,
    sensitivity: Arc<AtomicF32>,
}

// --- Synth's own palette: warm analog orange on charcoal, not a
// device-wide theme -- the classic subtractive-synth panel look, for
// the sim's most foundational, no-frills voice. ---

const SYNTH_BG: Rgb565 = Rgb565::new(2, 5, 3);
const SYNTH_TITLE: Rgb565 = Rgb565::new(29, 56, 26);
const SYNTH_ACCENT: Rgb565 = Rgb565::new(31, 35, 7);
const SYNTH_DIM: Rgb565 = Rgb565::new(15, 27, 11);

impl SynthApp {
    pub fn new(cutoff: Arc<AtomicF32>, sensitivity: Arc<AtomicF32>) -> Self {
        Self {
            waveform: Arc::new(AtomicU32::new(0)),
            cutoff,
            volume: Arc::new(AtomicF32::new(DEFAULT_VOLUME)),
            held: Arc::new(Mutex::new([false; 16])),
            sensitivity,
        }
    }
}

impl App for SynthApp {
    fn supports_pad_lock(&self) -> bool { true }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![
            ("Waveform".into(), Waveform::from_index(self.waveform.load(Ordering::Relaxed)).name().into(), false),
            ("Cutoff".into(), format!("{:.0} Hz", self.cutoff.get()), false),
            ("Volume".into(), format!("{:.0}%", self.volume.get() * 100.0), false),
            ("Held notes".into(), self.held.lock().unwrap().iter().filter(|&&held| held).count().to_string(), false),
        ]
    }

    fn tick(&mut self, input: &Input) {
        *self.held.lock().unwrap() = input.grid;

        let sensitivity = self.sensitivity.get();
        if input.knob1 != 0 {
            let next = (self.cutoff.get() * 1.15f32.powf(input.knob1 as f32 * sensitivity))
                .clamp(MIN_CUTOFF, MAX_CUTOFF);
            self.cutoff.set(next);
        }
        if input.knob2 != 0 {
            let next = (self.volume.get() + input.knob2 as f32 * sensitivity * 0.05)
                .clamp(0.0, 1.0);
            self.volume.set(next);
        }
        // Pushing knob1 resets cutoff, standard behavior for a
        // clickable encoder; pushing knob2 cycles the waveform instead
        // of resetting volume -- this used to be the 4 top buttons, but
        // those are global OS quick-nav now (see os.rs).
        if input.knob1_press {
            self.cutoff.set(DEFAULT_CUTOFF);
        }
        if input.knob2_press {
            let next = (self.waveform.load(Ordering::Relaxed) + 1) % 4;
            self.waveform.store(next, Ordering::Relaxed);
        }
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(SynthProcessor {
            waveform: Arc::clone(&self.waveform),
            cutoff: Arc::clone(&self.cutoff),
            volume: Arc::clone(&self.volume),
            held: Arc::clone(&self.held),
            voices: [Voice::default(); 16],
            filters: Vec::new(),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(SYNTH_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, SYNTH_TITLE);
        Text::new("Synth", Point::new(20, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, SYNTH_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, SYNTH_DIM);

        let waveform = Waveform::from_index(self.waveform.load(Ordering::Relaxed));
        Text::new(
            &format!("Cutoff {:.0} Hz (knob 1)", self.cutoff.get()),
            Point::new(20, 60),
            dim,
        )
        .draw(fb)
        .ok();
        Text::new(
            &format!("Volume {:.2} (knob 2)", self.volume.get()),
            Point::new(20, 75),
            dim,
        )
        .draw(fb)
        .ok();

        Text::new(&format!("Waveform: {} (knob 2 press to cycle)", waveform.name()), Point::new(20, 100), accent)
            .draw(fb)
            .ok();

        Text::new(
            "Grid: hold to play. Knob1 press: reset cutoff.",
            Point::new(20, 340),
            dim,
        )
        .draw(fb)
        .ok();
    }
}

#[derive(Default, Clone, Copy)]
struct Voice {
    phase: f32,
    amp: f32,
}

/// Reused from the earlier Filter app: a one-pole lowpass on the synth's
/// mixed output, stateful per channel.
struct OnePoleLowpass {
    state: f32,
}

impl OnePoleLowpass {
    fn new() -> Self {
        Self { state: 0.0 }
    }

    fn process(&mut self, input: f32, cutoff_hz: f32, sample_rate: f32) -> f32 {
        let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz);
        let dt = 1.0 / sample_rate;
        let alpha = dt / (rc + dt);
        self.state += alpha * (input - self.state);
        self.state
    }
}

struct SynthProcessor {
    waveform: Arc<AtomicU32>,
    cutoff: Arc<AtomicF32>,
    volume: Arc<AtomicF32>,
    held: Arc<Mutex<[bool; 16]>>,
    voices: [Voice; 16],
    filters: Vec<OnePoleLowpass>,
}

impl AudioProcessor for SynthProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        if self.filters.len() != channels {
            self.filters = (0..channels).map(|_| OnePoleLowpass::new()).collect();
        }

        let waveform = Waveform::from_index(self.waveform.load(Ordering::Relaxed));
        let cutoff = self.cutoff.get();
        let volume = self.volume.get();
        let held = *self.held.lock().unwrap();
        let envelope_coef = 1.0 - (-1.0 / (sample_rate * ATTACK_SECONDS)).exp();

        for frame in buffer.chunks_mut(channels) {
            let mut mix = 0.0;
            for (i, voice) in self.voices.iter_mut().enumerate() {
                let target = if held[i] { 1.0 } else { 0.0 };
                voice.amp += (target - voice.amp) * envelope_coef;
                if voice.amp > 0.0005 {
                    let freq = BASE_FREQ * 2f32.powf(i as f32 / 12.0);
                    mix += waveform.sample(voice.phase) * voice.amp;
                    voice.phase = (voice.phase + freq / sample_rate).fract();
                }
            }
            mix *= volume * 0.3; // headroom -- unlikely all 16 keys are held at once

            for (ch, sample) in frame.iter_mut().enumerate() {
                *sample = self.filters[ch].process(mix, cutoff, sample_rate);
            }
        }
    }
}
