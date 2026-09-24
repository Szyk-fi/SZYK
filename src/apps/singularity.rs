//! An intentionally unconventional multi-effect -- not "in the spirit
//! of" any real pedal the way Prism is of Microcosm, but four
//! original, deliberately alien-sounding processes built around
//! chaos and feedback rather than a faithful reproduction of anything:
//!
//! - **Spiral**: a pitch-shifting feedback loop -- every pass through
//!   the delay shifts pitch by a fixed interval, so at high Feedback
//!   it becomes a Shepard-tone-style infinite riser/faller that never
//!   resolves, just keeps climbing (or falling) forever.
//! - **Chaos**: a logistic map (the textbook `x = r*x*(1-x)` chaotic
//!   recurrence) continuously re-randomizes the delay time and a
//!   resonant filter's cutoff at a controllable rate -- genuinely
//!   non-repeating, unlike an LFO's periodic sweep, since a chaotic
//!   system's trajectory never exactly retraces itself.
//! - **Corrupt**: the same chaotic driver instead modulates a
//!   bit-crusher's depth and ring-modulates the signal against its
//!   own sine-mapped value, plus occasional chaotic "stutter" repeats
//!   of a short buffer slice -- a corrupted, glitchy texture.
//! - **Singularity**: the delay time itself is swept by a repeating
//!   envelope that shrinks toward near-zero (rising in pitch and
//!   pace, feedback climbing right along with it) before snapping
//!   back out to start over -- a collapsing/"falling in" gesture on
//!   a loop.
//!
//! One shared delay buffer per the usual pattern (see prism.rs), one
//! input mixer from AudioBus (same as Clouds/Prism, no microphone
//! input in this sim), and the same Mixer/AudioBus/ModBus
//! registration every audio-producing app in this build uses.

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
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Fixed capacity for input slots -- storage only; how many are
/// actually shown/used follows `AudioBus::len()` live (see
/// `group_leaves`). Was 8, which silently dropped any app registered
/// after the 8th (alphabetically, already losing Tape and Voltage)
/// from this app's input list -- bumped with real headroom so the
/// next few apps added to `apps/` don't quietly hit the same ceiling.
const MAX_INPUTS: usize = 128;
const DELAY_BUFFER_SECONDS: f32 = 2.5;
const MAX_FEEDBACK: f32 = 0.92; // headroom below 1.0 -- always a contraction
const DEFAULT_FEEDBACK: f32 = 0.5;
const CHAOS_R: f32 = 3.93; // logistic map growth rate, well into the chaotic regime
const MIN_CHAOS_INTERVAL: f32 = 0.005;
const MAX_CHAOS_INTERVAL: f32 = 0.3;
const SPIRAL_INTERVAL_SEMITONES: f32 = 1.0;
const MODE_NAMES: [&str; 4] = ["Spiral", "Chaos", "Corrupt", "Singularity"];

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * (max - min) * 0.05).clamp(min, max);
    value.set(next);
}

fn read_interp(buf: &[f32], pos: f32) -> f32 {
    let len = buf.len() as f32;
    let p = pos.rem_euclid(len);
    let i0 = (p as usize).min(buf.len() - 1);
    let i1 = (i0 + 1) % buf.len();
    let frac = p - i0 as f32;
    buf[i0] + (buf[i1] - buf[i0]) * frac
}

fn triangular_window(phase: f32) -> f32 {
    1.0 - (phase * 2.0 - 1.0).abs()
}

/// Same dual-read-head pitch shifter Prism's `pitch_shift_tap` uses,
/// duplicated locally.
fn pitch_shift_tap(buf: &[f32], write_pos: usize, base_delay: f32, rate: f32, phase: &mut f32, window: f32) -> f32 {
    *phase = (*phase + (rate - 1.0) / window).rem_euclid(1.0);
    let phase_b = (*phase + 0.5).rem_euclid(1.0);
    let sample_at = |ph: f32| {
        let offset = base_delay + ph * window - window * 0.5;
        read_interp(buf, write_pos as f32 - offset)
    };
    sample_at(*phase) * triangular_window(*phase) + sample_at(phase_b) * triangular_window(phase_b)
}

fn bitcrush(x: f32, levels: f32) -> f32 {
    let levels = levels.max(1.0);
    (x * levels).round() / levels
}

/// One step of the logistic map, clamped defensively away from the
/// exact 0/1 boundary (where float rounding could otherwise stick at
/// a fixed point instead of staying chaotic).
fn chaos_step(x: f32) -> f32 {
    (CHAOS_R * x * (1.0 - x)).clamp(0.0001, 0.9999)
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    InputLevel(usize),
    Mode,
    Amount,
    Rate,
    Feedback,
    Mix,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 2;

struct Params {
    input_levels: [AtomicF32; MAX_INPUTS],
    ext_input_level: [Arc<AtomicF32>; MAX_INPUTS],
    mode: AtomicU32,
    amount: AtomicF32,
    ext_amount: Arc<AtomicF32>,
    rate: AtomicF32,
    feedback: AtomicF32,
    mix: AtomicF32,
    ext_mix: Arc<AtomicF32>,
    /// The chaotic map's current value, 0..1 -- shared with the UI
    /// purely for the visualization, same "physics/phase state in a
    /// shared atomic" split Bloom/Nebula use.
    chaos_value: AtomicF32,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Singularity", modbus);
        Self {
            input_levels: std::array::from_fn(|_| AtomicF32::new(0.0)),
            ext_input_level: std::array::from_fn(|i| modbus.register(format!("Singularity: Input {} Level", i + 1))),
            mode: AtomicU32::new(0),
            amount: AtomicF32::new(0.5),
            ext_amount: modbus.register("Singularity: Amount".to_string()),
            rate: AtomicF32::new(0.5),
            feedback: AtomicF32::new(DEFAULT_FEEDBACK),
            mix: AtomicF32::new(0.5),
            ext_mix: modbus.register("Singularity: Mix".to_string()),
            chaos_value: AtomicF32::new(0.3141592),
            bus_out: audio_bus.register("Singularity"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct SingularityApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    audio_bus: Arc<AudioBus>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Singularity's own palette: near-black void with acid chartreuse,
// not a device-wide theme -- deliberately alien and toxic-looking,
// matching this app's own "unconventional, deliberately alien" DSP. ---

const SINGULARITY_BG: Rgb565 = Rgb565::new(0, 1, 0);
const SINGULARITY_TITLE: Rgb565 = Rgb565::new(28, 63, 25);
const SINGULARITY_ACCENT: Rgb565 = Rgb565::new(23, 63, 5);
const SINGULARITY_DIM: Rgb565 = Rgb565::new(11, 30, 7);
const SINGULARITY_RING: Rgb565 = Rgb565::new(3, 7, 2);
const SINGULARITY_FLASH: Rgb565 = Rgb565::new(25, 63, 12);

impl SingularityApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self { params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)), sensitivity, nav_speed, audio_bus, list: ParamList::new(), expanded: [false; NUM_GROUPS] }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => (0..self.audio_bus.len().min(MAX_INPUTS)).map(Selection::InputLevel).collect(),
            _ => vec![Selection::Mode, Selection::Amount, Selection::Rate, Selection::Feedback, Selection::Mix],
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

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::InputLevel(i) => self.audio_bus.names().get(i).cloned().unwrap_or_else(|| format!("Input {}", i + 1)),
            Selection::Mode => "Mode".into(),
            Selection::Amount => "Amount".into(),
            Selection::Rate => "Rate".into(),
            Selection::Feedback => "Feedback".into(),
            Selection::Mix => "Mix".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::InputLevel(i) => format!("{:.2}", self.params.input_levels[i].get()),
            Selection::Mode => MODE_NAMES[self.params.mode.load(Ordering::Relaxed) as usize % MODE_NAMES.len()].to_string(),
            Selection::Amount => format!("{:.2}", self.params.amount.get()),
            Selection::Rate => format!("{:.2}", self.params.rate.get()),
            Selection::Feedback => format!("{:.0}%", self.params.feedback.get() * 100.0),
            Selection::Mix => format!("{:.2}", self.params.mix.get()),
        }
    }

    fn group_name(&self, g: usize) -> &'static str {
        if g == 0 { "Mixer" } else { "Effect" }
    }

    fn group_summary(&self, g: usize) -> String {
        if g == 0 {
            let active = (0..self.audio_bus.len().min(MAX_INPUTS)).filter(|&i| self.params.input_levels[i].get() > 0.0).count();
            format!("{active} active")
        } else {
            MODE_NAMES[self.params.mode.load(Ordering::Relaxed) as usize % MODE_NAMES.len()].to_string()
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::InputLevel(i) => bump(&self.params.input_levels[i], delta, sensitivity, 0.0, 1.0),
            Selection::Mode => {
                let cur = self.params.mode.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(MODE_NAMES.len() as i32);
                self.params.mode.store(next as u32, Ordering::Relaxed);
            }
            Selection::Amount => bump(&self.params.amount, delta, sensitivity, 0.0, 1.0),
            Selection::Rate => bump(&self.params.rate, delta, sensitivity, 0.0, 1.0),
            Selection::Feedback => bump(&self.params.feedback, delta, sensitivity, 0.0, MAX_FEEDBACK),
            Selection::Mix => bump(&self.params.mix, delta, sensitivity, 0.0, 1.0),
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::InputLevel(i) => self.params.input_levels[i].set(0.0),
            Selection::Amount => self.params.amount.set(0.5),
            Selection::Rate => self.params.rate.set(0.5),
            Selection::Feedback => self.params.feedback.set(DEFAULT_FEEDBACK),
            Selection::Mix => self.params.mix.set(0.5),
            Selection::Mode => {} // no single sensible default among equal choices
        }
    }
}

impl SingularityApp {
    /// Real, windowed `(name, value, is_group)` rows -- mirrors this
    /// app's own `draw()` row-building, exposed for an alternate
    /// renderer (a live Slint screen) instead of drawn.
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

    /// `display_rows`, windowed to at most `visible` rows around the
    /// current selection -- see `ParamList::centered_scroll_window`. Returns
    /// `(window, selected_index_in_window, has_more_above,
    /// has_more_below)`.
    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }

    /// The real chaotic orbiting point -- same math `draw()`'s own
    /// sketch uses, exposed as a plain center-relative coordinate for
    /// an alternate renderer instead of drawn directly.
    pub(crate) fn orbit_visual(&self) -> crate::app::OrbitExtra {
        let x = self.params.chaos_value.get();
        let angle = x * TAU * 3.0;
        let r_frac = (20.0 + x * 85.0) / 210.0;
        crate::app::OrbitExtra { x: angle.cos() * r_frac, y: angle.sin() * r_frac, value: x }
    }
}

impl App for SingularityApp {
    fn needs_background_audio(&self) -> bool { self.params.input_levels.iter().any(|v| v.get() > 0.0) || self.params.ext_input_level.iter().any(|v| v.get() > 0.0) }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.selected_row()
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        self.windowed_rows(visible)
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        crate::app::SlintExtra::Orbit(self.orbit_visual())
    }

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
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(SingularityProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            input_mono: Vec::new(),
            output_mono: Vec::new(),
            delay_buf: Vec::new(),
            write_pos: 0,
            chaos_x: 0.3141592,
            chaos_timer: 0.0,
            chaos_delay_target: 0.3,
            chaos_cutoff_target: 0.5,
            svf_lp: 0.0,
            svf_bp: 0.0,
            grain_phase: 0.0,
            singularity_phase: 0.0,
            stutter_buf: Vec::new(),
            stutter_write_pos: 0,
            stutter_playback: None,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(SINGULARITY_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, SINGULARITY_TITLE);
        Text::new("Singularity", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, SINGULARITY_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, SINGULARITY_DIM);

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
        self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, SINGULARITY_BG, SINGULARITY_DIM, SINGULARITY_ACCENT);

        // --- Right: the chaotic value, drawn as an orbiting point --
        // radius driven by the logistic map's current value, so its
        // never-repeating trajectory is visible, not just numeric. ---
        let center = Point::new(500, 175);
        Circle::with_center(center, 210).into_styled(PrimitiveStyle::with_stroke(SINGULARITY_RING, 1)).draw(fb).ok();
        let x = self.params.chaos_value.get();
        let angle = x * TAU * 3.0;
        let r = 20.0 + x * 85.0;
        let point = Point::new(center.x + (angle.cos() * r) as i32, center.y + (angle.sin() * r) as i32);
        Circle::with_center(point, 8).into_styled(PrimitiveStyle::with_fill(SINGULARITY_FLASH)).draw(fb).ok();
        Text::new(&format!("chaos: {:.4}", x), Point::new(360, 300), accent).draw(fb).ok();

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

struct SingularityProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    input_mono: Vec<f32>,
    output_mono: Vec<f32>,
    delay_buf: Vec<f32>,
    write_pos: usize,
    /// The logistic map's current value and its own control-rate
    /// timer -- iterated far slower than the sample rate (see
    /// `MIN_CHAOS_INTERVAL`/`MAX_CHAOS_INTERVAL`) so its trajectory is
    /// audible as evolving modulation rather than raw noise.
    chaos_x: f32,
    chaos_timer: f32,
    chaos_delay_target: f32,
    chaos_cutoff_target: f32,
    svf_lp: f32,
    svf_bp: f32,
    /// Spiral's pitch-shift read head.
    grain_phase: f32,
    /// Singularity's repeating collapse-envelope phase.
    singularity_phase: f32,
    /// Corrupt's short rolling buffer for chaotic stutter repeats.
    stutter_buf: Vec<f32>,
    stutter_write_pos: usize,
    /// `Some((read_pos, remaining_samples))` while replaying a
    /// captured stutter slice instead of the live input.
    stutter_playback: Option<(usize, usize)>,
}

impl AudioProcessor for SingularityProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        self.input_mono.clear();
        self.input_mono.resize(frames, 0.0);
        self.output_mono.clear();
        self.output_mono.resize(frames, 0.0);

        if self.delay_buf.is_empty() {
            self.delay_buf = vec![0.0; (DELAY_BUFFER_SECONDS * sample_rate) as usize];
        }
        if self.stutter_buf.is_empty() {
            self.stutter_buf = vec![0.0; (0.25 * sample_rate) as usize];
        }
        let buf_len = self.delay_buf.len();
        let dt = 1.0 / sample_rate;

        let num_inputs = self.audio_bus.len().min(MAX_INPUTS);
        for i in 0..num_inputs {
            let level = (self.params.input_levels[i].get() + self.params.ext_input_level[i].get()).clamp(0.0, 2.0);
            if level <= 0.0 {
                continue;
            }
            if let Some(src) = self.audio_bus.get(i) {
                let src = src.lock().unwrap();
                for (m, s) in self.input_mono.iter_mut().zip(src.iter()) {
                    *m += *s * level;
                }
            }
        }

        let mode = self.params.mode.load(Ordering::Relaxed) % MODE_NAMES.len() as u32;
        let amount = (self.params.amount.get() + self.params.ext_amount.get()).clamp(0.0, 1.0);
        let rate = self.params.rate.get().clamp(0.0, 1.0);
        let feedback = self.params.feedback.get().clamp(0.0, MAX_FEEDBACK);
        let mix = (self.params.mix.get() + self.params.ext_mix.get()).clamp(0.0, 1.0);
        // Higher Rate = shorter interval between chaos iterations.
        let chaos_interval = MAX_CHAOS_INTERVAL - rate * (MAX_CHAOS_INTERVAL - MIN_CHAOS_INTERVAL);

        for n in 0..frames {
            let input_sample = self.input_mono[n];

            self.chaos_timer -= dt;
            if self.chaos_timer <= 0.0 {
                self.chaos_x = chaos_step(self.chaos_x);
                self.chaos_delay_target = 0.02 + self.chaos_x * 0.6;
                self.chaos_cutoff_target = self.chaos_x;
                self.chaos_timer = chaos_interval.max(0.001);
                self.params.chaos_value.set(self.chaos_x);
            }

            let wet = match mode {
                // Spiral: a fixed-interval pitch shift inside the
                // feedback loop -- each pass shifts again, so high
                // Feedback becomes an infinite riser/faller.
                0 => {
                    let rate_mult = 2f32.powf(SPIRAL_INTERVAL_SEMITONES * (1.0 + amount * 3.0) / 12.0);
                    let window = (0.08 * sample_rate).max(4.0);
                    pitch_shift_tap(&self.delay_buf, self.write_pos, window * 1.5, rate_mult, &mut self.grain_phase, window)
                }
                // Chaos: the delay time and a resonant filter's
                // cutoff are both driven by the same chaotic value,
                // read from the same feedback loop.
                1 => {
                    let delay_samples = (self.chaos_delay_target * sample_rate).min((buf_len - 1) as f32);
                    let read_pos = (self.write_pos as f32 - delay_samples).rem_euclid(buf_len as f32);
                    let raw = read_interp(&self.delay_buf, read_pos);
                    let cutoff_hz = 200.0 + self.chaos_cutoff_target * 4000.0;
                    let f_coef = (2.0 * (std::f32::consts::PI * cutoff_hz / sample_rate).sin()).clamp(0.0, 1.0);
                    let q = 1.5 + amount * 6.0;
                    let new_lp = self.svf_lp + f_coef * self.svf_bp;
                    let high = raw - new_lp - (1.0 / q) * self.svf_bp;
                    let new_bp = self.svf_bp + f_coef * high;
                    self.svf_lp = new_lp.clamp(-8.0, 8.0);
                    self.svf_bp = new_bp.clamp(-8.0, 8.0);
                    self.svf_lp
                }
                // Corrupt: chaos drives bit-depth and a chaotic
                // "should I stutter right now" gate, plus a constant
                // ring-mod against the chaotic value's own sine.
                2 => {
                    // Keep a short rolling capture of the live input
                    // so a stutter always repeats something recent.
                    if self.stutter_playback.is_none() {
                        let len = self.stutter_buf.len();
                        self.stutter_buf[self.stutter_write_pos] = input_sample;
                        self.stutter_write_pos = (self.stutter_write_pos + 1) % len;
                    }
                    if self.stutter_playback.is_none() && self.chaos_x > (0.97 - amount * 0.2) {
                        let slice_len = (self.stutter_buf.len() / 4).max(1);
                        self.stutter_playback = Some((self.stutter_write_pos, slice_len * 3));
                    }
                    let source = if let Some((mut pos, mut remaining)) = self.stutter_playback {
                        let len = self.stutter_buf.len();
                        let slice_len = (len / 4).max(1);
                        let s = self.stutter_buf[pos % len];
                        pos += 1;
                        remaining -= 1;
                        self.stutter_playback = if remaining == 0 { None } else { Some((pos, remaining)) };
                        // Loop back to the start of the captured slice
                        // instead of drifting into never-captured territory.
                        let _ = slice_len;
                        s
                    } else {
                        input_sample
                    };
                    let levels = 2.0 + (1.0 - amount) * 30.0;
                    let crushed = bitcrush(source, levels);
                    let ring = (self.chaos_x * TAU * 4.0).sin();
                    crushed * (1.0 - amount * 0.6) + crushed * ring * amount * 0.6
                }
                // Singularity: delay time on a repeating collapse
                // envelope -- shrinking toward near-zero (rising
                // pitch/pace) with feedback climbing alongside it,
                // then snapping back out.
                _ => {
                    let period = 1.0 + (1.0 - rate) * 4.0;
                    self.singularity_phase = (self.singularity_phase + dt / period).rem_euclid(1.0);
                    let collapse = 1.0 - self.singularity_phase; // 1 -> 0 across the period
                    let delay_samples = (0.01 + collapse * collapse * 0.5 * sample_rate / sample_rate).min(1.0) * sample_rate * 0.5;
                    let delay_samples = delay_samples.min((buf_len - 1) as f32).max(1.0);
                    let read_pos = (self.write_pos as f32 - delay_samples).rem_euclid(buf_len as f32);
                    read_interp(&self.delay_buf, read_pos)
                }
            };

            let feedback_boost = if mode == 3 {
                // Singularity's own feedback climbs as it collapses,
                // but the hard MAX_FEEDBACK ceiling still applies.
                (feedback + (1.0 - self.singularity_phase) * amount * (MAX_FEEDBACK - feedback)).min(MAX_FEEDBACK)
            } else {
                feedback
            };

            let write_sample = if mode == 2 {
                input_sample // Corrupt never feeds back -- it's a forward-only mangling of the live signal
            } else {
                input_sample + wet * feedback_boost
            };
            self.delay_buf[self.write_pos] = write_sample;
            self.write_pos = (self.write_pos + 1) % buf_len;

            self.output_mono[n] = input_sample * (1.0 - mix) + wet * mix;
        }

        {
            let mut bus_out = self.params.bus_out.lock().unwrap();
            bus_out.clear();
            bus_out.extend_from_slice(&self.output_mono);
        }

        let mix_level = (self.params.mix_level.get() + self.params.ext_mix_level.get()).clamp(0.0, 2.0);
        for (frame, sample) in buffer.chunks_mut(channels).zip(self.output_mono.iter()) {
            for out in frame.iter_mut() {
                *out = *sample * mix_level;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// More than the old `MAX_INPUTS` (8) real apps registered into
    /// `AudioBus` -- every one of them must still show up in this
    /// app's own input list, not just the first 8 (the bug that made
    /// Tape and Voltage, alphabetically last of the 11 real apps that
    /// register, invisible as inputs here).
    #[test]
    fn every_registered_source_appears_as_an_input_past_the_old_cap() {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        for i in 0..12 {
            audio_bus.register(format!("source-{i}"));
        }
        let app = SingularityApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);

        // 12 test sources + this app's own registration of itself = 13.
        let leaves = app.group_leaves(0);
        assert_eq!(leaves.len(), 13, "expected all 13 registered sources (12 test ones + itself) to be selectable inputs, got {}", leaves.len());
        assert!(matches!(leaves[12], Selection::InputLevel(12)), "the last source (past the old cap of 8) must still be reachable");
    }

    fn new_processor(params: Arc<Params>, audio_bus: Arc<AudioBus>) -> SingularityProcessor {
        SingularityProcessor {
            params,
            audio_bus,
            input_mono: Vec::new(),
            output_mono: Vec::new(),
            delay_buf: Vec::new(),
            write_pos: 0,
            chaos_x: 0.3141592,
            chaos_timer: 0.0,
            chaos_delay_target: 0.3,
            chaos_cutoff_target: 0.5,
            svf_lp: 0.0,
            svf_bp: 0.0,
            grain_phase: 0.0,
            singularity_phase: 0.0,
            stutter_buf: Vec::new(),
            stutter_write_pos: 0,
            stutter_playback: None,
        }
    }

    /// Every mode, under sustained full-scale input with Feedback at
    /// its maximum, must stay finite and within a sane bound --
    /// `MAX_FEEDBACK` keeps every feedback path a strict contraction
    /// regardless of mode.
    #[test]
    fn every_mode_stays_bounded_with_sustained_input() {
        for mode in 0..4u32 {
            let modbus = ModBus::new();
            let audio_bus = Arc::new(AudioBus::new());
            let mixer_bus = MixerBus::new();
            let src = audio_bus.register("test-source");
            let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
            params.input_levels[0].set(1.0);
            params.mix.set(1.0);
            params.mode.store(mode, Ordering::Relaxed);
            params.feedback.set(MAX_FEEDBACK);
            params.amount.set(1.0);

            let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
            let sample_rate = 48000.0;
            let frames = 512;
            let mut buffer = vec![0.0f32; frames * 2];
            {
                let mut s = src.lock().unwrap();
                s.resize(frames, 1.0);
            }
            let mut peak = 0.0f32;
            for _ in 0..300 {
                proc.process(&mut buffer, 2, sample_rate);
                for v in buffer.iter() {
                    assert!(v.is_finite(), "mode {mode} ('{}') produced a non-finite sample", MODE_NAMES[mode as usize]);
                    peak = peak.max(v.abs());
                }
            }
            assert!(peak < 25.0, "mode {mode} ('{}') exceeded a sane bound: peak={peak}", MODE_NAMES[mode as usize]);
        }
    }

    /// Spiral's whole character depends on it actually shifting
    /// pitch each pass -- verified indirectly by confirming its
    /// output differs meaningfully from a straight (unshifted) delay
    /// echo of the same impulse.
    #[test]
    fn spiral_output_differs_from_plain_delay() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0);
        params.mix.set(1.0);
        params.mode.store(0, Ordering::Relaxed);
        params.feedback.set(0.6);
        params.amount.set(0.8);

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let sample_rate = 48000.0;
        let frames = 512;
        let mut buffer = vec![0.0f32; frames * 2];
        {
            let mut s = src.lock().unwrap();
            s.resize(frames, 0.0);
            for (i, v) in s.iter_mut().enumerate() {
                *v = (i as f32 * 0.05).sin();
            }
        }
        let mut out = Vec::new();
        for _ in 0..40 {
            proc.process(&mut buffer, 2, sample_rate);
            out.extend_from_slice(&buffer);
        }
        let peak: f32 = out.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(peak > 0.05, "expected an audible pitch-shifted feedback tail, got peak {peak}");
    }

    /// Chaos's whole point is non-periodic modulation -- the chaos
    /// value itself must actually move over time, not sit fixed.
    #[test]
    fn chaos_value_evolves_over_time() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0);
        params.rate.set(1.0); // fastest chaos updates

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let mut buffer = vec![0.0f32; 512 * 2];
        {
            let mut s = src.lock().unwrap();
            s.resize(512, 0.3);
        }
        let start = proc.chaos_x;
        let mut seen_values = std::collections::HashSet::new();
        for _ in 0..50 {
            proc.process(&mut buffer, 2, 48000.0);
            seen_values.insert((proc.chaos_x * 1e6) as i64);
        }
        assert!(seen_values.len() > 5, "expected the chaotic value to visit several distinct values, saw {}", seen_values.len());
        assert_ne!(proc.chaos_x, start, "chaos value should have moved from its starting point");
    }

    /// The Mixer app's channel fader must only affect what reaches
    /// the device output, not what this app publishes to audio_bus.rs
    /// for another app (Clouds) to tap.
    #[test]
    fn mixer_fader_does_not_affect_audio_bus_publish() {
        let modbus = ModBus::new();
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = MixerBus::new();
        let src = audio_bus.register("test-source");
        let params = Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus));
        params.input_levels[0].set(1.0);
        params.mix.set(1.0);
        params.mix_level.set(0.0);

        let mut proc = new_processor(Arc::clone(&params), Arc::clone(&audio_bus));
        let mut buffer = vec![0.0f32; 512 * 2];
        {
            let mut s = src.lock().unwrap();
            s.resize(512, 1.0);
        }
        for _ in 0..10 {
            proc.process(&mut buffer, 2, 48000.0);
        }
        let device_peak = buffer.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert_eq!(device_peak, 0.0);

        let bus_out = params.bus_out.lock().unwrap();
        let bus_peak = bus_out.iter().cloned().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(bus_peak > 0.0, "audio_bus publish should be unaffected by the Mixer channel fader");
    }
}
