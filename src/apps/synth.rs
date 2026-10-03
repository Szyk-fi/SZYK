//! A simple polyphonic synth: the 4x4 grid is a 16-note keyboard (held,
//! not toggled — press and hold to sustain, release to stop), knob1
//! edits cutoff, knob2 edits volume, and knob2 press cycles the
//! waveform (used to be the 4 top buttons, but F1-F4 are global OS
//! quick-nav now -- see os.rs -- so this app never sees them).
//! No microphone input — this app is its own sound source.
//!
//! It opens on the shared play view (play_kit.rs): the same knobs turn
//! cutoff/volume, the D-pad steps the waveform, F2 cycles the pads through
//! KEYS, Controls and Moments. R1 hands the knobs back to the original
//! direct behaviour above (Synth has no menu list of its own).

use crate::app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes};
use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::util::{note_name, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const BASE_FREQ: f32 = 220.0; // A3 -- grid key 0
/// MIDI note of grid key 0 (A3 = 220 Hz), so a keyboard key lands on
/// the pad that sounds its pitch.
const BASE_NOTE: i32 = 57;
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
    /// The shared play view (play_kit.rs).
    kit: PlayKit,
}

/// The play view's controls, most important first. Synth only has three,
/// so the Controls layer is mostly dark -- inventing more would be a lie
/// about what this voice can do.
const CONTROLS: [&str; 3] = ["Cutoff", "Volume", "Waveform"];
const C_CUTOFF: usize = 0;
const C_VOLUME: usize = 1;
const C_WAVEFORM: usize = 2;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "synth",
        layers: vec![Layer::Native(0, "KEYS"), Layer::Controls, Layer::Moments],
        hero: vec![[C_CUTOFF, C_VOLUME]],
        // The waveform is the only choice this synth has, so it's what
        // the D-pad steps.
        browse: Some(C_WAVEFORM),
        // Stick X sweeps the filter (the one move a one-pole lowpass
        // rewards); Y and the right hand swell the volume like an
        // expression pedal; the left hand opens the filter too.
        routes: Routes { stick_x: Some(C_CUTOFF), stick_y: Some(C_VOLUME), hand_l: Some(C_CUTOFF), hand_r: Some(C_VOLUME) },
        throws: Vec::new(),
        midi_to_pads: true,
        own_expression: false,
    }
}

// --- Synth's own palette: warm analog orange on charcoal, not a
// device-wide theme -- the classic subtractive-synth panel look, for
// the sim's most foundational, no-frills voice. ---

const SYNTH_BG: Rgb565 = Rgb565::new(2, 5, 3);
const SYNTH_TITLE: Rgb565 = Rgb565::new(29, 56, 26);
const SYNTH_ACCENT: Rgb565 = Rgb565::new(31, 35, 7);
const SYNTH_DIM: Rgb565 = Rgb565::new(15, 27, 11);
/// Unlit pad cells and dial tracks on the play column.
const SYNTH_FAINT: Rgb565 = Rgb565::new(5, 11, 5);

impl SynthApp {
    pub fn new(cutoff: Arc<AtomicF32>, sensitivity: Arc<AtomicF32>) -> Self {
        Self {
            waveform: Arc::new(AtomicU32::new(0)),
            cutoff,
            volume: Arc::new(AtomicF32::new(DEFAULT_VOLUME)),
            held: Arc::new(Mutex::new([false; 16])),
            sensitivity,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
        }
    }

    fn edit_cutoff(&self, delta: i32) {
        // Multiplicative, so each detent is the same musical interval
        // across the whole 100 Hz - 8 kHz range.
        let next = (self.cutoff.get() * 1.15f32.powf(delta as f32 * self.sensitivity.get())).clamp(MIN_CUTOFF, MAX_CUTOFF);
        self.cutoff.set(next);
    }

    fn edit_volume(&self, delta: i32) {
        let next = (self.volume.get() + delta as f32 * self.sensitivity.get() * 0.05).clamp(0.0, 1.0);
        self.volume.set(next);
    }

    fn waveform(&self) -> Waveform {
        Waveform::from_index(self.waveform.load(Ordering::Relaxed))
    }
}

impl PlayHost for SynthApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS.len()
    }
    fn kit_label(&self, i: usize) -> String {
        CONTROLS.get(i).copied().unwrap_or("").to_string()
    }
    fn kit_value(&self, i: usize) -> String {
        match i {
            C_CUTOFF => format!("{:.0} Hz", self.cutoff.get()),
            C_VOLUME => format!("{:.0}%", self.volume.get() * 100.0),
            C_WAVEFORM => self.waveform().name().to_string(),
            _ => String::new(),
        }
    }
    /// Cutoff's position is logarithmic, matching how the knob already
    /// moves it, so the stick sweeps it evenly by ear rather than spending
    /// most of its travel above 4 kHz.
    fn kit_norm(&self, i: usize) -> Option<f32> {
        Some(match i {
            C_CUTOFF => ((self.cutoff.get() / MIN_CUTOFF).ln() / (MAX_CUTOFF / MIN_CUTOFF).ln()).clamp(0.0, 1.0),
            C_VOLUME => self.volume.get().clamp(0.0, 1.0),
            C_WAVEFORM => (self.waveform.load(Ordering::Relaxed) % 4) as f32 / 3.0,
            _ => return None,
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        i == C_WAVEFORM
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match i {
            C_CUTOFF => self.edit_cutoff(delta),
            C_VOLUME => self.edit_volume(delta),
            C_WAVEFORM => {
                let next = (self.waveform.load(Ordering::Relaxed) as i32 + delta).rem_euclid(4) as u32;
                self.waveform.store(next, Ordering::Relaxed);
            }
            _ => {}
        }
    }
    fn kit_reset(&mut self, i: usize) {
        match i {
            C_CUTOFF => self.cutoff.set(DEFAULT_CUTOFF),
            C_VOLUME => self.volume.set(DEFAULT_VOLUME),
            C_WAVEFORM => self.waveform.store(0, Ordering::Relaxed),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i {
            C_CUTOFF => self.cutoff.set((MIN_CUTOFF * (MAX_CUTOFF / MIN_CUTOFF).powf(v)).clamp(MIN_CUTOFF, MAX_CUTOFF)),
            C_VOLUME => self.volume.set(v),
            C_WAVEFORM => self.waveform.store((v * 3.0).round() as u32, Ordering::Relaxed),
            _ => {}
        }
    }
    /// Pad i sounds BASE_NOTE + i (physical index, as the voices always
    /// have); keys outside that 16-semitone window fold in by octaves.
    fn kit_midi_pad(&self, note: u8) -> Option<usize> {
        let d = note as i32 - BASE_NOTE;
        let pad = if (0..16).contains(&d) { d } else { d.rem_euclid(12) };
        Some(pad as usize)
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        note_name(BASE_NOTE + pad as i32)
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, held: bool) -> crate::led_output::PadColor {
        use crate::led_output::PadColor;
        if held {
            PadColor::Green
        } else if (BASE_NOTE + pad as i32).rem_euclid(12) == 9 {
            // The A's: the grid starts on A3, so these mark the octaves.
            PadColor::Blue
        } else {
            PadColor::Off
        }
    }
    fn kit_line(&self) -> String {
        let held = *self.held.lock().unwrap();
        let notes: Vec<String> = (0..16).filter(|&i| held[i]).take(4).map(|i| note_name(BASE_NOTE + i as i32)).collect();
        if notes.is_empty() { "-".into() } else { notes.join(" ") }
    }
}

impl App for SynthApp {
    fn supports_pad_lock(&self) -> bool { true }
    fn play_surface(&self) -> bool { true }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (!self.kit.menu).then(|| self.kit.column(self))
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }
    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
    }
    fn grid_led_overlay(&self) -> [crate::led_output::PadColor; 16] {
        self.kit.led_overlay(self)
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        vec![
            ("Waveform".into(), Waveform::from_index(self.waveform.load(Ordering::Relaxed)).name().into(), false),
            ("Cutoff".into(), format!("{:.0} Hz", self.cutoff.get()), false),
            ("Volume".into(), format!("{:.0}%", self.volume.get() * 100.0), false),
            ("Held notes".into(), self.held.lock().unwrap().iter().filter(|&&held| held).count().to_string(), false),
        ]
    }

    fn tick(&mut self, input: &Input) {
        // The play view takes the knobs and D-pad first; with R1's "menu"
        // (Synth has no list) they pass straight through to the original
        // knob behaviour below. Pads reach the voices only on KEYS -- on
        // a kit layer the kit hands us an empty grid, so nothing sounds.
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let input = &step.input;
        *self.held.lock().unwrap() = input.grid;

        if input.knob1 != 0 {
            self.edit_cutoff(input.knob1);
        }
        if input.knob2 != 0 {
            self.edit_volume(input.knob2);
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

        // Synth never had a menu list, so the play column takes the
        // empty right half of the screen and the readouts stay put.
        if let Some(col) = self.play_column() {
            let pal = kit::draw::Palette { bg: SYNTH_BG, ink: SYNTH_TITLE, accent: SYNTH_ACCENT, dim: SYNTH_DIM, faint: SYNTH_FAINT };
            kit::draw::column(fb, &col, 320, 40, 300, 280, pal);
        }

        let hint = if self.kit.menu {
            "Grid: hold to play. Knob1 press: reset cutoff."
        } else {
            "L/R: cutoff/volume  U/D: wave  F2: pads  R1: knobs direct"
        };
        Text::new(hint, Point::new(20, 340), dim).draw(fb).ok();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::play_kit::rank_pad;

    fn app() -> SynthApp {
        SynthApp::new(Arc::new(AtomicF32::new(DEFAULT_CUTOFF)), Arc::new(AtomicF32::new(1.0)))
    }

    fn render(app: &mut SynthApp) -> f32 {
        let mut proc = app.audio_processor().unwrap();
        let mut buf = vec![0.0f32; 2 * 4096];
        proc.process(&mut buf, 2, 48_000.0);
        buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32
    }

    #[test]
    fn opens_playable_and_knob1_turns_cutoff() {
        let mut a = app();
        assert!(a.play_column().is_some(), "play view first");
        let c = a.cutoff.get();
        a.tick(&Input { knob1: 3, ..Default::default() });
        assert!(a.cutoff.get() > c, "knob 1 is cutoff on the play view");
        a.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(a.waveform.load(Ordering::Relaxed), 1, "D-pad up = next waveform");
    }

    #[test]
    fn keys_layer_still_plays_and_kit_layers_keep_the_pads_silent() {
        let mut a = app();
        a.tick(&Input { grid: std::array::from_fn(|i| i == 5), ..Default::default() });
        assert!(a.held.lock().unwrap()[5], "KEYS layer holds the pad's voice");
        assert!(render(&mut a) > 1e-5, "a held pad is audible");
        a.toggle_grid_mode(); // Controls
        a.tick(&Input { grid: std::array::from_fn(|i| i == rank_pad(1)), ..Default::default() });
        assert!(!a.held.lock().unwrap().iter().any(|h| *h), "Controls layer pads don't play notes");
        // A MIDI key plays the pad with its pitch, back on KEYS.
        a.toggle_grid_mode();
        a.toggle_grid_mode();
        let mut keys = crate::app::MidiKeys::default();
        keys.0[(BASE_NOTE + 7) as usize] = 100;
        a.tick(&Input { midi_keys: keys, ..Default::default() });
        assert!(a.held.lock().unwrap()[7]);
    }

    #[test]
    fn r1_gives_the_knobs_back_their_original_jobs() {
        let mut a = app();
        a.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(a.play_column().is_none(), "R1 leaves the play view");
        a.tick(&Input { knob2_press: true, ..Default::default() });
        assert_eq!(a.waveform.load(Ordering::Relaxed), 1, "knob 2 press still cycles the waveform");
    }

    #[test]
    fn the_stick_sweeps_cutoff_and_returns() {
        let mut a = app();
        let c = a.cutoff.get();
        a.tick(&Input { stick: [1.0, 0.0], ..Default::default() });
        assert!(a.cutoff.get() > c * 2.0, "full right opens the filter well up");
        a.tick(&Input::default());
        assert!((a.cutoff.get() - c).abs() < 0.5, "back to the knob: {}", a.cutoff.get());
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(SynthApp::new(ctx.named("cutoff"), ctx.named("sensitivity")))
}
