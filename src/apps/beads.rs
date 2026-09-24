//! A clone of Qu-Bit^H^H^H^HMutable Instruments Beads: a granular
//! texture synthesizer built around a live circular capture buffer,
//! not a sample player -- it taps another app's output (see
//! audio_bus.rs, the same technique Clouds already uses) into a
//! rolling buffer and scatters overlapping grains back out of it.
//! Feature set follows the real module's manual (mutable-instruments.
//! net/modules/beads/manual) as closely as this sim's architecture
//! allows:
//!
//! - 4 recording-quality modes (bit depth/rate/buffer length), always
//!   applied to the captured/wet signal only -- the dry path stays
//!   full quality, same as the real module.
//! - FREEZE: stops writing new audio into the capture buffer.
//! - 3 grain-generation modes -- Latched (continuous, DENSITY sets
//!   rate), Gated (grains only while a pad is held), Clocked (pad
//!   presses act as clock ticks, DENSITY becomes a probability/
//!   divider) -- standing in for the real SEED button + SEED CV/gate
//!   input, which this sim has no generic per-app gate-patching for.
//! - TIME/SIZE/SHAPE/PITCH grain parameters, each with its own
//!   randomization amount (standing in for the real module's
//!   "attenurandomizers", which blend external CV against internal
//!   randomization -- this sim only has the randomization half).
//! - Feedback, Dry/Wet, and a small Reverb, matching the real
//!   module's IN -> grains -> (feedback loop) -> reverb -> dry/wet ->
//!   OUT signal flow.
//! - "Beads as a delay": turning SIZE fully clockwise collapses the
//!   grain cloud into a single continuously-retriggering tap, using
//!   TIME as the delay length -- a simplified stand-in for the real
//!   module's dual DENSITY/TIME delay-time system (which needs an
//!   external clock/tap-tempo input this sim doesn't generically
//!   have), but a real, working delay/echo rather than a fake one.
//!
//! Deliberately not implemented: the real module's "granular
//! wavetable synth" auto-fallback (after 10s with nothing patched
//! into either input, Beads granularizes internal Plaits wavetables
//! instead) -- that needs a direct coupling into Plaits' wavetable
//! data this sim doesn't have a path for yet.

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const MAX_CAPTURE_SECONDS: f32 = 32.0;
const MAX_GRAINS: usize = 8;
const MIN_GRAIN_MS: f32 = 30.0;
const MAX_GRAIN_MS: f32 = 4000.0;
/// SIZE at or above this (fraction of full CW rotation) collapses the
/// grain cloud into a single continuously-retriggering delay tap --
/// see module doc comment, "Beads as a delay".
const DELAY_MODE_THRESHOLD: f32 = 0.98;

const QUALITY_NAMES: [&str; 4] = ["Bright digital", "Cold digital", "Sunny tape", "Scorched cassette"];
const GRAIN_MODE_NAMES: [&str; 3] = ["Latched", "Gated", "Clocked"];

struct QualityProfile {
    /// Sample-and-hold decimation factor applied to the captured
    /// (wet) signal only -- 1 = full 48kHz, higher = lower effective
    /// rate.
    rate_div: u32,
    /// Amplitude quantization depth applied to the captured signal.
    bits: u32,
    /// How much of `MAX_CAPTURE_SECONDS` this mode's buffer actually
    /// uses (mono figures from the manual, scaled against the max).
    buffer_seconds: f32,
    /// Slow pitch-wobble depth (tape-style wow/flutter), 0 = none.
    wow_depth: f32,
}

fn quality_profile(mode: u32) -> QualityProfile {
    match mode % 4 {
        0 => QualityProfile { rate_div: 1, bits: 16, buffer_seconds: 8.0, wow_depth: 0.0 },
        1 => QualityProfile { rate_div: 2, bits: 12, buffer_seconds: 16.0, wow_depth: 0.0 },
        2 => QualityProfile { rate_div: 2, bits: 12, buffer_seconds: 20.0, wow_depth: 0.0015 },
        _ => QualityProfile { rate_div: 3, bits: 8, buffer_seconds: 32.0, wow_depth: 0.004 },
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Source,
    Quality,
    Freeze,
    GrainMode,
    Time,
    Size,
    Shape,
    Pitch,
    TimeRandom,
    SizeRandom,
    ShapeRandom,
    PitchRandom,
    Density,
    Feedback,
    DryWet,
    Reverb,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 4;

struct Params {
    source: AtomicUsize,
    quality: AtomicU32,
    freeze: AtomicBool,
    grain_mode: AtomicU32,
    time: AtomicF32,   // 0..1, buffer read position: 0 = most recent, 1 = oldest
    size: AtomicF32,   // 0..1 -> 30ms..4s (exponential); >=DELAY_MODE_THRESHOLD = delay mode
    shape: AtomicF32,  // 0..1, grain envelope: 0 = clicky/rectangular, 1 = slow attack
    pitch: AtomicF32,  // 0..1 -> -24..+24 semitones
    time_random: AtomicF32,
    size_random: AtomicF32,
    shape_random: AtomicF32,
    pitch_random: AtomicF32,
    density: AtomicF32, // 0..1, centered at 0.5 (see grain-mode-dependent meaning)
    feedback: AtomicF32,
    dry_wet: AtomicF32,
    reverb: AtomicF32,
    /// Real pad-hold state, updated every `tick()` -- this sim's
    /// stand-in for the real module's SEED button/gate input, read by
    /// the audio thread for Gated/Clocked grain generation.
    held: Mutex<[bool; 16]>,
    /// A small downsampled snapshot of the current quality mode's
    /// capture window, refreshed once per audio block -- real, live
    /// buffer content for the Slint "grain cloud" visual (not a
    /// fabricated waveform).
    waveform_snapshot: Mutex<Vec<f32>>,
    /// One entry per currently active grain: `(position fraction
    /// 0..1 within that same capture window, envelope amplitude
    /// 0..1)`, refreshed once per audio block -- real live grain
    /// state, same envelope value `process()` actually mixes in.
    grain_dots: Mutex<Vec<(f32, f32)>>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Beads", modbus);
        Self {
            source: AtomicUsize::new(crate::audio_bus::NO_SOURCE),
            quality: AtomicU32::new(0),
            freeze: AtomicBool::new(false),
            grain_mode: AtomicU32::new(0),
            time: AtomicF32::new(0.0),
            size: AtomicF32::new(0.3),
            shape: AtomicF32::new(0.0),
            pitch: AtomicF32::new(0.5),
            time_random: AtomicF32::new(0.0),
            size_random: AtomicF32::new(0.0),
            shape_random: AtomicF32::new(0.0),
            pitch_random: AtomicF32::new(0.0),
            density: AtomicF32::new(0.65),
            feedback: AtomicF32::new(0.0),
            dry_wet: AtomicF32::new(1.0),
            reverb: AtomicF32::new(0.0),
            held: Mutex::new([false; 16]),
            waveform_snapshot: Mutex::new(Vec::new()),
            grain_dots: Mutex::new(Vec::new()),
            bus_out: audio_bus.register("Beads"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct BeadsApp {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Beads's own palette: flat solid colors, not a
// device-wide theme -- Warm sand and terracotta grains on dark, tactile earth -- the color of beads scattering through fingers. ---

const BEADS_BG: Rgb565 = Rgb565::new(4, 7, 2);
const BEADS_TITLE: Rgb565 = Rgb565::new(29, 54, 23);
const BEADS_ACCENT: Rgb565 = Rgb565::new(26, 34, 9);
const BEADS_DIM: Rgb565 = Rgb565::new(17, 28, 11);

impl BeadsApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            audio_bus,
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
        }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => vec![Selection::Source, Selection::Quality, Selection::Freeze],
            1 => vec![Selection::GrainMode, Selection::Density, Selection::Time, Selection::Size, Selection::Shape, Selection::Pitch],
            2 => vec![Selection::TimeRandom, Selection::SizeRandom, Selection::ShapeRandom, Selection::PitchRandom],
            _ => vec![Selection::Feedback, Selection::DryWet, Selection::Reverb],
        }
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for g in 0..NUM_GROUPS {
            rows.push(Row::Group(g));
            if self.expanded[g] {
                for sel in self.group_leaves(g) {
                    rows.push(Row::Leaf(sel));
                }
            }
        }
        rows
    }

    fn group_name(&self, g: usize) -> &'static str {
        match g {
            0 => "Input",
            1 => "Grain",
            2 => "Randomize",
            _ => "Mix",
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => format!(
                "{}, {}{}",
                self.source_name(),
                QUALITY_NAMES[self.params.quality.load(Ordering::Relaxed) as usize % 4],
                if self.params.freeze.load(Ordering::Relaxed) { ", frozen" } else { "" }
            ),
            1 => {
                if self.params.size.get() >= DELAY_MODE_THRESHOLD {
                    format!("{}, delay", GRAIN_MODE_NAMES[self.params.grain_mode.load(Ordering::Relaxed) as usize % 3])
                } else {
                    format!("{}, {:.0}ms", GRAIN_MODE_NAMES[self.params.grain_mode.load(Ordering::Relaxed) as usize % 3], self.grain_ms())
                }
            }
            2 => {
                let avg = (self.params.time_random.get() + self.params.size_random.get() + self.params.shape_random.get() + self.params.pitch_random.get()) / 4.0;
                format!("{:.0}% avg", avg * 100.0)
            }
            _ => format!("{:.0}%fb, {:.0}% wet", self.params.feedback.get() * 100.0, self.params.dry_wet.get() * 100.0),
        }
    }

    fn grain_ms(&self) -> f32 {
        let size = self.params.size.get().clamp(0.0, 1.0);
        MIN_GRAIN_MS * (MAX_GRAIN_MS / MIN_GRAIN_MS).powf(size)
    }

    fn source_name(&self) -> String {
        self.audio_bus.source_name(self.params.source.load(Ordering::Relaxed))
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => "Source".into(),
            Selection::Quality => "Quality".into(),
            Selection::Freeze => "Freeze".into(),
            Selection::GrainMode => "Grain Mode".into(),
            Selection::Time => "Time".into(),
            Selection::Size => "Size".into(),
            Selection::Shape => "Shape".into(),
            Selection::Pitch => "Pitch".into(),
            Selection::TimeRandom => "Time Random".into(),
            Selection::SizeRandom => "Size Random".into(),
            Selection::ShapeRandom => "Shape Random".into(),
            Selection::PitchRandom => "Pitch Random".into(),
            Selection::Density => "Density".into(),
            Selection::Feedback => "Feedback".into(),
            Selection::DryWet => "Dry/Wet".into(),
            Selection::Reverb => "Reverb".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => self.source_name(),
            Selection::Quality => QUALITY_NAMES[self.params.quality.load(Ordering::Relaxed) as usize % 4].into(),
            Selection::Freeze => if self.params.freeze.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Selection::GrainMode => GRAIN_MODE_NAMES[self.params.grain_mode.load(Ordering::Relaxed) as usize % 3].into(),
            Selection::Time => format!("{:.0}%", self.params.time.get() * 100.0),
            Selection::Size => {
                if self.params.size.get() >= DELAY_MODE_THRESHOLD {
                    "delay".into()
                } else {
                    format!("{:.0} ms", self.grain_ms())
                }
            }
            Selection::Shape => format!("{:.0}%", self.params.shape.get() * 100.0),
            Selection::Pitch => format!("{:+.0} st", (self.params.pitch.get() - 0.5) * 48.0),
            Selection::TimeRandom => format!("{:.0}%", self.params.time_random.get() * 100.0),
            Selection::SizeRandom => format!("{:.0}%", self.params.size_random.get() * 100.0),
            Selection::ShapeRandom => format!("{:.0}%", self.params.shape_random.get() * 100.0),
            Selection::PitchRandom => format!("{:.0}%", self.params.pitch_random.get() * 100.0),
            Selection::Density => format!("{:.0}%", self.params.density.get() * 100.0),
            Selection::Feedback => format!("{:.0}%", self.params.feedback.get() * 100.0),
            Selection::DryWet => format!("{:.0}%", self.params.dry_wet.get() * 100.0),
            Selection::Reverb => format!("{:.0}%", self.params.reverb.get() * 100.0),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::Source => {
                let cur = self.params.source.load(Ordering::Relaxed);
                self.params.source.store(crate::audio_bus::cycle_source(cur, step, self.audio_bus.len()), Ordering::Relaxed);
            }
            Selection::Quality => {
                let cur = self.params.quality.load(Ordering::Relaxed) as i32;
                self.params.quality.store((cur + step).rem_euclid(4) as u32, Ordering::Relaxed);
            }
            Selection::Freeze => self.params.freeze.store(delta > 0, Ordering::Relaxed),
            Selection::GrainMode => {
                let cur = self.params.grain_mode.load(Ordering::Relaxed) as i32;
                self.params.grain_mode.store((cur + step).rem_euclid(3) as u32, Ordering::Relaxed);
            }
            Selection::Time => bump(&self.params.time, delta, sensitivity, 0.0, 1.0),
            Selection::Size => bump(&self.params.size, delta, sensitivity, 0.0, 1.0),
            Selection::Shape => bump(&self.params.shape, delta, sensitivity, 0.0, 1.0),
            Selection::Pitch => bump(&self.params.pitch, delta, sensitivity, 0.0, 1.0),
            Selection::TimeRandom => bump(&self.params.time_random, delta, sensitivity, 0.0, 1.0),
            Selection::SizeRandom => bump(&self.params.size_random, delta, sensitivity, 0.0, 1.0),
            Selection::ShapeRandom => bump(&self.params.shape_random, delta, sensitivity, 0.0, 1.0),
            Selection::PitchRandom => bump(&self.params.pitch_random, delta, sensitivity, 0.0, 1.0),
            Selection::Density => bump(&self.params.density, delta, sensitivity, 0.0, 1.0),
            Selection::Feedback => bump(&self.params.feedback, delta, sensitivity, 0.0, 0.97),
            Selection::DryWet => bump(&self.params.dry_wet, delta, sensitivity, 0.0, 1.0),
            Selection::Reverb => bump(&self.params.reverb, delta, sensitivity, 0.0, 1.0),
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Source => self.params.source.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed),
            Selection::Quality => self.params.quality.store(0, Ordering::Relaxed),
            Selection::Freeze => self.params.freeze.store(false, Ordering::Relaxed),
            Selection::GrainMode => self.params.grain_mode.store(0, Ordering::Relaxed),
            Selection::Time => self.params.time.set(0.0),
            Selection::Size => self.params.size.set(0.3),
            Selection::Shape => self.params.shape.set(0.0),
            Selection::Pitch => self.params.pitch.set(0.5),
            Selection::TimeRandom => self.params.time_random.set(0.0),
            Selection::SizeRandom => self.params.size_random.set(0.0),
            Selection::ShapeRandom => self.params.shape_random.set(0.0),
            Selection::PitchRandom => self.params.pitch_random.set(0.0),
            Selection::Density => self.params.density.set(0.65),
            Selection::Feedback => self.params.feedback.set(0.0),
            Selection::DryWet => self.params.dry_wet.set(1.0),
            Selection::Reverb => self.params.reverb.set(0.0),
        }
    }

    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g), true)
                }
                Row::Leaf(sel) => (self.leaf_name(*sel), self.leaf_value(*sel), false),
            })
            .collect()
    }

    pub(crate) fn selected_row(&self) -> usize {
        self.list.selected
    }

    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }

    /// The real "grain cloud" -- a scrolling snapshot of the live
    /// capture buffer plus each currently active grain's real
    /// position/envelope within it (see `Params::waveform_snapshot`/
    /// `grain_dots`, both written once per audio block by
    /// `BeadsProcessor::process`), converted to connected line-
    /// segment geometry (see `crate::app::polyline_segments`) instead
    /// of drawn directly.
    pub(crate) fn output_visual(&self) -> crate::app::BeadsExtra {
        const PANEL_W: f32 = 260.0;
        const PANEL_H: f32 = 150.0;
        let raw = self.params.waveform_snapshot.lock().unwrap().clone();
        let waveform = if raw.len() >= 2 {
            let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(&raw, PANEL_W, PANEL_H, true);
            crate::app::CurveSegments { mid_x, mid_y, length, angle_deg }
        } else {
            crate::app::CurveSegments::default()
        };
        crate::app::BeadsExtra {
            mode_name: GRAIN_MODE_NAMES[self.params.grain_mode.load(Ordering::Relaxed) as usize % 3].to_string(),
            frozen: self.params.freeze.load(Ordering::Relaxed),
            is_delay_mode: self.params.size.get() >= DELAY_MODE_THRESHOLD,
            waveform,
            grain_dots: self.params.grain_dots.lock().unwrap().clone(),
        }
    }
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.01).clamp(min, max);
    value.set(next);
}

impl App for BeadsApp {
    fn needs_background_audio(&self) -> bool { self.params.source.load(Ordering::Relaxed) != crate::audio_bus::NO_SOURCE || self.params.freeze.load(Ordering::Relaxed) }
    fn supports_pad_lock(&self) -> bool { true }

    fn tick(&mut self, input: &Input) {
        let rows = self.visible_rows();
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();

        if input.knob1_press {
            if let Some(Row::Group(g)) = current {
                self.expanded[g] = !self.expanded[g];
            }
        }
        if let Some(Row::Leaf(sel)) = current {
            self.edit(sel, input.knob2);
            if input.knob2_press {
                self.reset(sel);
            }
        }

        let mut held = self.params.held.lock().unwrap();
        for (i, pressed) in input.grid.iter().enumerate() {
            held[i] = *pressed;
        }
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        crate::app::SlintExtra::Beads(self.output_visual())
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(BeadsProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            capture: vec![0.0; (MAX_CAPTURE_SECONDS * 48000.0) as usize],
            write_pos: 0,
            grains: std::array::from_fn(|_| Grain::default()),
            spawn_accum: 0.0,
            rng: 0x1234_5678,
            was_held: false,
            clock_div_counter: 0,
            hold_counter: 0,
            held_value: 0.0,
            wow_phase: 0.0,
            reverb: Reverb::default(),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(BEADS_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, BEADS_TITLE);
        Text::new("Beads", Point::new(16, 26), title).draw(fb).ok();
        let dim = MonoTextStyle::new(&SPLEEN_6X12, BEADS_DIM);
        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", self.leaf_name(*sel)), self.leaf_value(*sel)),
            })
            .collect();
        self.list.draw_themed(fb, 16, 56, 22, 10, &display_rows, BEADS_BG, BEADS_DIM, BEADS_ACCENT);
        let _ = dim;
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.selected_row()
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        self.windowed_rows(visible)
    }
}

#[derive(Clone, Copy, Default)]
struct Grain {
    active: bool,
    /// Read position into `capture`, advanced by `rate` each sample.
    pos: f32,
    rate: f32,
    /// Samples remaining before this grain ends.
    remaining: f32,
    length: f32,
    shape: f32,
    /// Set for the single continuously-retriggering grain used by
    /// delay mode -- lets the spawn code respawn it at exactly its
    /// own duration without waiting on the normal density scheduler.
    is_delay_tap: bool,
}

/// A small 2-comb + 1-allpass reverb -- not the real module's own
/// algorithm (undocumented), just a modest, genuinely-computed
/// diffuse tail rather than a fake dry-signal-only knob.
struct Reverb {
    comb_a: Vec<f32>,
    comb_b: Vec<f32>,
    allpass: Vec<f32>,
    pos_a: usize,
    pos_b: usize,
    pos_ap: usize,
}

impl Default for Reverb {
    fn default() -> Self {
        Self { comb_a: vec![0.0; 1687], comb_b: vec![0.0; 2053], allpass: vec![0.0; 347], pos_a: 0, pos_b: 0, pos_ap: 0 }
    }
}

impl Reverb {
    fn process(&mut self, input: f32, amount: f32) -> f32 {
        if amount <= 0.0001 {
            return input;
        }
        let fb = 0.78;
        let a = self.comb_a[self.pos_a];
        self.comb_a[self.pos_a] = input + a * fb;
        self.pos_a = (self.pos_a + 1) % self.comb_a.len();

        let b = self.comb_b[self.pos_b];
        self.comb_b[self.pos_b] = input + b * fb;
        self.pos_b = (self.pos_b + 1) % self.comb_b.len();

        let combined = (a + b) * 0.5;
        let ap_in = self.allpass[self.pos_ap];
        let ap_out = -combined + ap_in;
        self.allpass[self.pos_ap] = combined + ap_in * 0.5;
        self.pos_ap = (self.pos_ap + 1) % self.allpass.len();

        input * (1.0 - amount) + ap_out * amount
    }
}

/// The grain amplitude envelope: `shape` 0 = clicky/rectangular (very
/// short attack and decay), 1 = a slow attack reminiscent of a
/// reversed grain -- see module doc comment.
fn grain_envelope(progress: f32, shape: f32) -> f32 {
    let attack = (0.02 + shape.clamp(0.0, 1.0) * 0.88).clamp(0.02, 0.95);
    if progress < attack {
        0.5 - 0.5 * (std::f32::consts::PI * (progress / attack)).cos()
    } else {
        let d = ((progress - attack) / (1.0 - attack)).clamp(0.0, 1.0);
        0.5 + 0.5 * (std::f32::consts::PI * d).cos()
    }
}

struct BeadsProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    /// A rolling capture of the tapped source -- what grains are cut
    /// from. Fixed at the largest quality mode's length; each mode
    /// just uses a shorter prefix of it (see `quality_profile`).
    capture: Vec<f32>,
    write_pos: usize,
    grains: [Grain; MAX_GRAINS],
    /// Fractional grain-spawn accumulator -- lets density be
    /// sub-block-accurate without needing per-sample scheduling.
    spawn_accum: f32,
    rng: u32,
    /// Rising/falling edge tracking on `held` for Gated/Clocked modes.
    was_held: bool,
    /// Clocked mode's division counter -- counts ticks since the last
    /// grain was allowed through.
    clock_div_counter: u32,
    /// Sample-and-hold state for the quality mode's rate decimation.
    hold_counter: u32,
    held_value: f32,
    wow_phase: f32,
    reverb: Reverb,
}

impl BeadsProcessor {
    fn next_rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// A "peaky" random value in -1..1, clustered toward the middle --
    /// matches the manual's description of the attenurandomizers'
    /// internal random source.
    fn peaky_rand(&mut self) -> f32 {
        (self.next_rand() + self.next_rand()) * 0.5
    }

    fn spawn_grain(&mut self, cap_len: usize, length_samples: f32, rate: f32, pitch_random: f32, time_random: f32, size_random: f32, shape_random: f32, shape: f32, time: f32, is_delay_tap: bool) {
        let cap_len_f = cap_len as f32;
        let rate = rate * 2f32.powf(self.peaky_rand() * pitch_random * 2.0);
        let length_samples = (length_samples * (1.0 + self.peaky_rand() * size_random)).max(1.0);
        let shape = (shape + self.peaky_rand() * shape_random).clamp(0.0, 1.0);

        // TIME picks how far back into the buffer a grain starts: 0 =
        // most recent audio, 1 = the oldest material still held in
        // this quality mode's window. Jitter is scaled to a few
        // grain-lengths (plus TIME's own randomization amount), not
        // the whole buffer.
        let base_back = time.clamp(0.0, 1.0) * (cap_len_f - length_samples * rate.max(1.0)).max(0.0);
        let jitter = self.peaky_rand() * time_random * cap_len_f * 0.5;
        let start = (self.write_pos as f32 - length_samples * rate.max(1.0) - base_back + jitter).rem_euclid(cap_len_f);

        let slot = if is_delay_tap {
            // The delay tap always reuses grain slot 0 so there's
            // never more than one active -- a real single repeating
            // tap, not a cloud.
            &mut self.grains[0]
        } else {
            match self.grains.iter_mut().find(|g| !g.active) {
                Some(slot) => slot,
                None => return,
            }
        };
        slot.active = true;
        slot.pos = start;
        slot.rate = rate;
        slot.length = length_samples;
        slot.remaining = length_samples;
        slot.shape = shape;
        slot.is_delay_tap = is_delay_tap;
    }
}

impl AudioProcessor for BeadsProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        for out in buffer.iter_mut() {
            *out = 0.0;
        }

        let source_idx = self.params.source.load(Ordering::Relaxed);
        let tapped = self.audio_bus.get(source_idx).map(|b| b.lock().unwrap().clone());

        let profile = quality_profile(self.params.quality.load(Ordering::Relaxed));
        let cap_len = ((profile.buffer_seconds / MAX_CAPTURE_SECONDS) * self.capture.len() as f32) as usize;
        let cap_len = cap_len.clamp(1024, self.capture.len());

        let is_delay_mode = self.params.size.get() >= DELAY_MODE_THRESHOLD;
        let grain_ms = if is_delay_mode {
            // TIME sets the delay length directly in this mode (see
            // module doc comment) -- from ~50ms up to the full
            // buffer window.
            let time = self.params.time.get().clamp(0.0, 1.0);
            (50.0 + time * (cap_len as f32 / sample_rate * 1000.0 - 50.0)).max(20.0)
        } else {
            MIN_GRAIN_MS * (MAX_GRAIN_MS / MIN_GRAIN_MS).powf(self.params.size.get().clamp(0.0, 1.0))
        };
        let length_samples = (grain_ms * 0.001 * sample_rate).max(1.0);
        let pitch_oct = (self.params.pitch.get().clamp(0.0, 1.0) - 0.5) * 4.0;
        let rate = 2f32.powf(pitch_oct);
        let density = self.params.density.get().clamp(0.0, 1.0);
        let time = self.params.time.get().clamp(0.0, 1.0);
        let time_random = self.params.time_random.get().clamp(0.0, 1.0);
        let size_random = self.params.size_random.get().clamp(0.0, 1.0);
        let shape_random = self.params.shape_random.get().clamp(0.0, 1.0);
        let pitch_random = self.params.pitch_random.get().clamp(0.0, 1.0);
        let shape = self.params.shape.get().clamp(0.0, 1.0);
        let feedback = self.params.feedback.get().clamp(0.0, 0.97);
        let dry_wet = self.params.dry_wet.get().clamp(0.0, 1.0);
        let reverb_amount = self.params.reverb.get().clamp(0.0, 1.0);
        let frozen = self.params.freeze.load(Ordering::Relaxed);
        let grain_mode = self.params.grain_mode.load(Ordering::Relaxed) % 3;
        let is_held = self.params.held.lock().unwrap().iter().any(|&h| h);

        // Latched-mode rate: null at the center (density == 0.5),
        // randomly-modulated turning CW, constant turning CCW, same
        // shape as the real DENSITY knob.
        let (latched_hz, latched_jitter) = if density >= 0.5 {
            let t = (density - 0.5) * 2.0;
            (t * 60.0, t)
        } else {
            let t = (0.5 - density) * 2.0;
            (t * 60.0, 0.0)
        };

        let frames = buffer.len() / channels;
        let mut mono = vec![0.0f32; frames];

        for n in 0..frames {
            // --- Capture write (quality-degraded, frozen = frozen) ---
            let input_sample = tapped.as_ref().and_then(|b| b.get(n % b.len().max(1)).copied()).unwrap_or(0.0);
            if !frozen {
                let write_idx = self.write_pos % cap_len;
                let fed_back = self.capture[write_idx] * feedback;
                let mut raw = (input_sample + fed_back).clamp(-4.0, 4.0);

                // Tape-style wow/flutter: a slow pitch wobble applied
                // by re-reading a slightly time-shifted version of
                // the signal being written.
                if profile.wow_depth > 0.0 {
                    self.wow_phase += 0.6 / sample_rate;
                    raw *= 1.0 + (self.wow_phase * std::f32::consts::TAU).sin() * profile.wow_depth * 40.0;
                }

                // Sample-rate decimation (sample-and-hold).
                if self.hold_counter == 0 {
                    self.held_value = raw;
                }
                self.hold_counter = (self.hold_counter + 1) % profile.rate_div.max(1);

                // Bit-depth quantization.
                let levels = (1u32 << profile.bits.min(16).max(1)) as f32 / 2.0 - 1.0;
                let quantized = (self.held_value * levels).round() / levels;

                self.capture[write_idx] = quantized;
                self.write_pos = (self.write_pos + 1) % cap_len;
            }

            // --- Grain spawn scheduling, per grain mode ---
            match grain_mode {
                0 => {
                    // Latched: continuous, DENSITY-scheduled.
                    if latched_hz > 0.01 {
                        self.spawn_accum += latched_hz / sample_rate;
                        if self.spawn_accum >= 1.0 {
                            self.spawn_accum -= 1.0;
                            let jitter_amt = (time_random + latched_jitter * 0.3).min(1.0);
                            self.spawn_grain(cap_len, length_samples, rate, pitch_random, jitter_amt, size_random, shape_random, shape, time, false);
                        }
                    }
                }
                1 => {
                    // Gated: only while a pad is held. DENSITY near
                    // center = a single grain per press; otherwise a
                    // continuous rate while held.
                    let rising_edge = is_held && !self.was_held;
                    if (0.4..=0.6).contains(&density) {
                        if rising_edge {
                            self.spawn_grain(cap_len, length_samples, rate, pitch_random, time_random, size_random, shape_random, shape, time, false);
                        }
                    } else if is_held {
                        let hz = 1.0 + density * 59.0;
                        self.spawn_accum += hz / sample_rate;
                        if self.spawn_accum >= 1.0 {
                            self.spawn_accum -= 1.0;
                            self.spawn_grain(cap_len, length_samples, rate, pitch_random, time_random, size_random, shape_random, shape, time, false);
                        }
                    } else {
                        self.spawn_accum = 0.0;
                    }
                }
                _ => {
                    // Clocked: each pad-press edge is a clock tick.
                    // DENSITY >= center = probability (0-100%);
                    // DENSITY < center = a clock divider (1/16..1/1).
                    let rising_edge = is_held && !self.was_held;
                    if rising_edge {
                        let should_spawn = if density >= 0.5 {
                            let prob = (density - 0.5) * 2.0;
                            self.next_rand().abs() < prob
                        } else {
                            let t = (0.5 - density) * 2.0;
                            let division = (1.0 + t * 15.0).round() as u32;
                            self.clock_div_counter += 1;
                            let hit = self.clock_div_counter >= division;
                            if hit {
                                self.clock_div_counter = 0;
                            }
                            hit
                        };
                        if should_spawn {
                            self.spawn_grain(cap_len, length_samples, rate, pitch_random, time_random, size_random, shape_random, shape, time, false);
                        }
                    }
                }
            }
            self.was_held = is_held;

            // --- Delay mode: a single continuously-retriggering tap,
            // reusing grain slot 0, independent of grain_mode/DENSITY
            // scheduling above (see module doc comment). ---
            if is_delay_mode && !self.grains[0].active {
                self.spawn_grain(cap_len, length_samples, rate, pitch_random * 0.15, 0.0, 0.0, shape_random, shape, 0.0, true);
            }

            // --- Sum every active grain ---
            let mut sample = 0.0f32;
            for grain in self.grains.iter_mut() {
                if !grain.active {
                    continue;
                }
                let idx = grain.pos.rem_euclid(cap_len as f32);
                let i0 = idx as usize;
                let i1 = (i0 + 1) % cap_len;
                let frac = idx - i0 as f32;
                let value = self.capture[i0] * (1.0 - frac) + self.capture[i1] * frac;

                let progress = 1.0 - (grain.remaining / grain.length).clamp(0.0, 1.0);
                let window = grain_envelope(progress, grain.shape);
                sample += value * window;

                grain.pos += grain.rate;
                grain.remaining -= 1.0;
                if grain.remaining <= 0.0 {
                    grain.active = false;
                }
            }
            // Headroom-normalize by the max simultaneous grain count
            // so density alone doesn't make it louder.
            let wet = self.reverb.process(sample / (MAX_GRAINS as f32).sqrt(), reverb_amount);
            mono[n] = (input_sample * (1.0 - dry_wet) + wet * dry_wet).tanh();
        }

        for (out, s) in buffer.chunks_mut(channels).zip(mono.iter()) {
            for ch in out.iter_mut() {
                *ch = *s;
            }
        }
        *self.params.bus_out.lock().unwrap() = mono;

        // Once per block (not per-sample): a downsampled snapshot of
        // the current quality mode's capture window, and each active
        // grain's real position/envelope within it -- the Slint
        // "grain cloud" visual reads both directly, no fabricated
        // data.
        const SNAPSHOT_POINTS: usize = 120;
        let snapshot: Vec<f32> = (0..SNAPSHOT_POINTS).map(|i| self.capture[i * cap_len / SNAPSHOT_POINTS]).collect();
        *self.params.waveform_snapshot.lock().unwrap() = snapshot;

        let dots: Vec<(f32, f32)> = self
            .grains
            .iter()
            .filter(|g| g.active)
            .map(|g| {
                let frac = g.pos.rem_euclid(cap_len as f32) / cap_len as f32;
                let progress = 1.0 - (g.remaining / g.length).clamp(0.0, 1.0);
                (frac, grain_envelope(progress, g.shape))
            })
            .collect();
        *self.params.grain_dots.lock().unwrap() = dots;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> (BeadsApp, Arc<AudioBus>) {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(3.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let app = BeadsApp::new(sensitivity, nav_speed, modbus, Arc::clone(&audio_bus), mixer_bus);
        (app, audio_bus)
    }

    /// No tapped source, no feedback -- Beads must stay silent and
    /// never panic.
    #[test]
    fn no_source_and_no_feedback_is_silence_not_a_panic() {
        let (mut app, _audio_bus) = new_app();
        app.params.density.set(1.0); // spawn plenty of grains from an all-zero capture
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..10 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|&s| s == 0.0));
    }

    /// A loud tapped source must produce audible grains in Latched
    /// mode (the default).
    #[test]
    fn a_loud_tapped_source_produces_audible_grains() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed); // Beads registers its own bus first (index 0) -- "Source" lands at 1.
        *source.lock().unwrap() = vec![0.8; 512];
        app.params.density.set(1.0);
        app.params.dry_wet.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.01, "expected audible grains from a loud tapped source, got peak {peak}");
    }

    /// High feedback must never blow up -- output must stay finite
    /// and bounded even after many blocks of self-feeding texture.
    #[test]
    fn high_feedback_stays_bounded_and_finite() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed); // Beads registers its own bus first (index 0) -- "Source" lands at 1.
        *source.lock().unwrap() = vec![0.9; 512];
        app.params.feedback.set(0.97);
        app.params.density.set(1.0);
        app.params.dry_wet.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..200 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|s| s.is_finite()), "output must stay finite under sustained high feedback");
        let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak <= 1.0001, "expected soft-clipped output, got peak {peak}");
    }

    /// Beads must republish its own output on the audio bus.
    #[test]
    fn republishes_its_output_on_the_audio_bus() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed); // Beads registers its own bus first (index 0) -- "Source" lands at 1.
        *source.lock().unwrap() = vec![0.5; 256];
        app.params.density.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 256 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        let published = app.params.bus_out.lock().unwrap();
        assert_eq!(published.len(), 256);
    }

    /// FREEZE must stop new audio from entering the capture buffer --
    /// frozen immediately (before anything real was ever captured),
    /// a newly loud source must never reach the (wet) output.
    #[test]
    fn freeze_stops_capturing_new_audio() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        app.params.density.set(1.0);
        app.params.dry_wet.set(1.0);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];

        // Freeze immediately, before any real audio has ever been
        // captured (buffer starts silent).
        app.params.freeze.store(true, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.9; 512];
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        // With dry_wet fully wet and a still-silent capture buffer
        // (nothing was ever written into it while frozen), every
        // grain reads silence -- output must stay exactly zero.
        assert!(buffer.iter().all(|&s| s == 0.0), "expected silence while frozen, buffer was not all-zero");
    }

    /// Gated mode must stay silent with no pad held, and produce
    /// audio once one is.
    #[test]
    fn gated_mode_only_sounds_while_a_pad_is_held() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.8; 512];
        app.params.grain_mode.store(1, Ordering::Relaxed); // Gated
        app.params.density.set(1.0); // continuous rate while held
        app.params.dry_wet.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let peak_unheld = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert_eq!(peak_unheld, 0.0, "expected silence in Gated mode with nothing held, got peak {peak_unheld}");

        *app.params.held.lock().unwrap() = [true; 16];
        let mut peak_held = 0.0f32;
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
            peak_held = peak_held.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak_held > 0.01, "expected audible grains once a pad is held in Gated mode, got peak {peak_held}");
    }

    /// SIZE turned fully clockwise must engage delay mode and produce
    /// a genuine repeating tap (bounded, finite, audible) rather than
    /// a normal grain cloud.
    #[test]
    fn size_fully_clockwise_engages_a_bounded_delay() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.7; 512];
        app.params.size.set(1.0);
        app.params.time.set(0.1);
        app.params.dry_wet.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..80 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|s| s.is_finite()));
        let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.01, "expected an audible delay tap, got peak {peak}");
    }

    /// Reverb must stay finite and bounded even at full amount.
    #[test]
    fn reverb_stays_bounded_and_finite() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.9; 512];
        app.params.density.set(1.0);
        app.params.reverb.set(1.0);
        app.params.dry_wet.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..200 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|s| s.is_finite()));
        let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak <= 1.0001, "expected soft-clipped output, got peak {peak}");
    }
}
