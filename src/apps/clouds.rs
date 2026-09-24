//! The real Mutable Instruments Clouds granular processor (see
//! src/clouds_ffi.rs and vendor/eurorack/clouds), not an approximation
//! -- same treatment as Plaits. Unlike Plaits (a voice that generates
//! sound from a note), Clouds is a *processor*: it granulates an
//! incoming audio signal. Since this sim has no microphone input, that
//! signal comes from the shared AudioBus (see audio_bus.rs) instead --
//! an **input mixer** blends together the live output of any other app
//! that publishes to the bus (Plaits, Sequencer, Bloom), each with its
//! own level, before feeding the result into the granular engine.
//! Those mixer levels are themselves modulation targets (see
//! modbus.rs), same as every other continuous knob here.
//!
//! Real Clouds' 4 playback modes (Granular/Stretch/Looping Delay/
//! Spectral), Position/Size/Pitch/Density/Texture/Dry-Wet/Spread/
//! Feedback/Reverb, and Freeze/Trigger are all here and all real --
//! `granular.overlap`/`window_shape` aren't exposed because the real
//! engine derives them itself from Density/Texture (not a
//! simplification on our part, that's how the actual hardware panel
//! works too: those aren't separate knobs there either).

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::clouds_ffi::{CloudsParams, Granulator};
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Fixed capacity for input slots -- storage only; how many are
/// actually shown/used follows `AudioBus::len()` live (see
/// `group_leaves`), the same way Pam's/Bloom query `ModBus` live
/// rather than snapshotting a count at construction (apps register
/// into these buses in an order this app can't control, and more can
/// be dropped into `apps/` later). This just needs to comfortably
/// exceed however many audio-producing apps ever actually exist --
/// it was 8 and silently dropped any app registered after the 8th
/// (alphabetically, that already meant losing Tape and Voltage) from
/// this app's input list. Bumped with real headroom rather than to
/// exactly today's count, so the next few apps someone adds don't
/// quietly hit the same ceiling again.
const MAX_INPUTS: usize = 128;
const PLAYBACK_MODE_NAMES: [&str; 4] = ["Granular", "Stretch", "Looping Delay", "Spectral"];
const MONITOR_LEN: usize = 150;

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * (max - min) * 0.05).clamp(min, max);
    value.set(next);
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    InputLevel(usize),
    PlaybackMode,
    Position,
    Size,
    Pitch,
    Density,
    Texture,
    DryWet,
    Spread,
    Feedback,
    Reverb,
    Freeze,
    Trigger,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 4;

fn group_name(g: usize) -> &'static str {
    match g {
        0 => "Mixer",
        1 => "Grain",
        2 => "Output",
        // Not just "Mode" -- it also holds Freeze and Trigger, both
        // playback-transport actions rather than settings, so a name
        // that covers all three reads less confusingly than one that
        // only describes the first leaf.
        _ => "Playback",
    }
}

struct Params {
    input_levels: [AtomicF32; MAX_INPUTS],
    ext_input_level: [Arc<AtomicF32>; MAX_INPUTS],
    playback_mode: AtomicU32,
    position: AtomicF32,
    size: AtomicF32,
    pitch: AtomicF32,
    density: AtomicF32,
    texture: AtomicF32,
    dry_wet: AtomicF32,
    stereo_spread: AtomicF32,
    feedback: AtomicF32,
    reverb: AtomicF32,
    freeze: AtomicBool,
    /// Edge-triggered: the UI sets this true, the audio thread reads
    /// and clears it back to false after firing one grain.
    trigger: AtomicBool,
    ext_position: Arc<AtomicF32>,
    ext_size: Arc<AtomicF32>,
    ext_pitch: Arc<AtomicF32>,
    ext_density: Arc<AtomicF32>,
    ext_texture: Arc<AtomicF32>,
    ext_dry_wet: Arc<AtomicF32>,
    ext_feedback: Arc<AtomicF32>,
    ext_reverb: Arc<AtomicF32>,
    monitor_history: Mutex<VecDeque<f32>>,
    /// This app's rendered mono output, republished every block for
    /// another app (Prism) to tap -- see audio_bus.rs. Clouds didn't
    /// publish this before; it produces real audible output just
    /// like every other source app, so it should be tappable/mixable
    /// the same way.
    bus_out: Arc<Mutex<Vec<f32>>>,
    /// This app's channel fader in the Mixer app, plus its own
    /// modulation input -- see mixer_bus.rs.
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Clouds", modbus);
        Self {
            input_levels: std::array::from_fn(|_| AtomicF32::new(0.0)),
            ext_input_level: std::array::from_fn(|i| modbus.register(format!("Clouds: Input {} Level", i + 1))),
            playback_mode: AtomicU32::new(0),
            position: AtomicF32::new(0.5),
            size: AtomicF32::new(0.5),
            pitch: AtomicF32::new(0.0),
            density: AtomicF32::new(0.5),
            texture: AtomicF32::new(0.5),
            dry_wet: AtomicF32::new(1.0),
            stereo_spread: AtomicF32::new(0.0),
            feedback: AtomicF32::new(0.0),
            reverb: AtomicF32::new(0.0),
            freeze: AtomicBool::new(false),
            trigger: AtomicBool::new(false),
            ext_position: modbus.register("Clouds: Position"),
            ext_size: modbus.register("Clouds: Size"),
            ext_pitch: modbus.register("Clouds: Pitch"),
            ext_density: modbus.register("Clouds: Density"),
            ext_texture: modbus.register("Clouds: Texture"),
            ext_dry_wet: modbus.register("Clouds: Dry/Wet"),
            ext_feedback: modbus.register("Clouds: Feedback"),
            ext_reverb: modbus.register("Clouds: Reverb"),
            monitor_history: Mutex::new(VecDeque::with_capacity(MONITOR_LEN)),
            bus_out: audio_bus.register("Clouds"),
            mix_level,
            ext_mix_level,
        }
    }
}

pub struct CloudsApp {
    params: Arc<Params>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    audio_bus: Arc<AudioBus>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Clouds' own palette: pale sky blue on a light ground, not a
// device-wide theme -- airy and drifting, the opposite of a dark
// instrument screen, matching this app's own name and character. ---

const CLOUDS_BG: Rgb565 = Rgb565::new(21, 47, 25);
const CLOUDS_TITLE: Rgb565 = Rgb565::new(3, 10, 7);
const CLOUDS_ACCENT: Rgb565 = Rgb565::new(5, 20, 16);
const CLOUDS_DIM: Rgb565 = Rgb565::new(7, 18, 10);
const CLOUDS_FROZEN: Rgb565 = Rgb565::new(9, 20, 20);
const CLOUDS_MIDLINE: Rgb565 = Rgb565::new(20, 40, 26);

impl CloudsApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            sensitivity,
            nav_speed,
            audio_bus,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
        }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => (0..self.audio_bus.len().min(MAX_INPUTS)).map(Selection::InputLevel).collect(),
            1 => vec![Selection::Position, Selection::Size, Selection::Pitch, Selection::Density, Selection::Texture],
            2 => vec![Selection::DryWet, Selection::Spread, Selection::Feedback, Selection::Reverb],
            _ => vec![Selection::PlaybackMode, Selection::Freeze, Selection::Trigger],
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
            Selection::PlaybackMode => "Mode".into(),
            Selection::Position => "Position".into(),
            Selection::Size => "Size".into(),
            Selection::Pitch => "Pitch".into(),
            Selection::Density => "Density".into(),
            Selection::Texture => "Texture".into(),
            Selection::DryWet => "Dry/Wet".into(),
            Selection::Spread => "Spread".into(),
            Selection::Feedback => "Feedback".into(),
            Selection::Reverb => "Reverb".into(),
            Selection::Freeze => "Freeze".into(),
            Selection::Trigger => "Trigger".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::InputLevel(i) => format!("{:.2}", self.params.input_levels[i].get()),
            Selection::PlaybackMode => {
                let idx = self.params.playback_mode.load(Ordering::Relaxed) as usize % PLAYBACK_MODE_NAMES.len();
                PLAYBACK_MODE_NAMES[idx].to_string()
            }
            Selection::Position => format!("{:.2}", self.params.position.get()),
            Selection::Size => format!("{:.2}", self.params.size.get()),
            Selection::Pitch => format!("{:+.1} st", self.params.pitch.get()),
            Selection::Density => format!("{:.2}", self.params.density.get()),
            Selection::Texture => format!("{:.2}", self.params.texture.get()),
            Selection::DryWet => format!("{:.2}", self.params.dry_wet.get()),
            Selection::Spread => format!("{:.2}", self.params.stereo_spread.get()),
            Selection::Feedback => format!("{:.2}", self.params.feedback.get()),
            Selection::Reverb => format!("{:.2}", self.params.reverb.get()),
            Selection::Freeze => {
                if self.params.freeze.load(Ordering::Relaxed) { "frozen".into() } else { "off".into() }
            }
            Selection::Trigger => "press knob2".into(),
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => {
                let active =
                    (0..self.audio_bus.len().min(MAX_INPUTS)).filter(|&i| self.params.input_levels[i].get() > 0.0).count();
                format!("{active} active")
            }
            1 => format!("pos {:.2}", self.params.position.get()),
            2 => format!("wet {:.2}", self.params.dry_wet.get()),
            _ => {
                let idx = self.params.playback_mode.load(Ordering::Relaxed) as usize % PLAYBACK_MODE_NAMES.len();
                PLAYBACK_MODE_NAMES[idx].to_string()
            }
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
            Selection::PlaybackMode => {
                let cur = self.params.playback_mode.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(PLAYBACK_MODE_NAMES.len() as i32);
                self.params.playback_mode.store(next as u32, Ordering::Relaxed);
            }
            Selection::Position => bump(&self.params.position, delta, sensitivity, 0.0, 1.0),
            Selection::Size => bump(&self.params.size, delta, sensitivity, 0.0, 1.0),
            Selection::Pitch => bump(&self.params.pitch, delta, sensitivity, -48.0, 48.0),
            Selection::Density => bump(&self.params.density, delta, sensitivity, 0.0, 1.0),
            Selection::Texture => bump(&self.params.texture, delta, sensitivity, 0.0, 1.0),
            Selection::DryWet => bump(&self.params.dry_wet, delta, sensitivity, 0.0, 1.0),
            Selection::Spread => bump(&self.params.stereo_spread, delta, sensitivity, 0.0, 1.0),
            Selection::Feedback => bump(&self.params.feedback, delta, sensitivity, 0.0, 1.0),
            Selection::Reverb => bump(&self.params.reverb, delta, sensitivity, 0.0, 1.0),
            Selection::Freeze => self.params.freeze.store(delta > 0, Ordering::Relaxed),
            Selection::Trigger => {} // action only fires on press -- see reset()
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::InputLevel(i) => self.params.input_levels[i].set(0.0),
            Selection::Position => self.params.position.set(0.5),
            Selection::Size => self.params.size.set(0.5),
            Selection::Pitch => self.params.pitch.set(0.0),
            Selection::Density => self.params.density.set(0.5),
            Selection::Texture => self.params.texture.set(0.5),
            Selection::DryWet => self.params.dry_wet.set(1.0),
            Selection::Spread => self.params.stereo_spread.set(0.0),
            Selection::Feedback => self.params.feedback.set(0.0),
            Selection::Reverb => self.params.reverb.set(0.0),
            Selection::Freeze => self.params.freeze.store(false, Ordering::Relaxed),
            Selection::Trigger => self.params.trigger.store(true, Ordering::Relaxed),
            Selection::PlaybackMode => {} // no sensible single default
        }
    }
}

impl CloudsApp {
    /// Real, windowed `(name, value, is_group)` rows -- mirrors this
    /// app's own `draw()` row-building, exposed for an alternate
    /// renderer (a live Slint screen) instead of drawn.
    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", group_name(*g)), self.group_summary(*g), true)
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

    /// The real granular output monitor -- same `Params.
    /// monitor_history` `draw()`'s own scrolling-waveform panel
    /// reads, resampled to `n` points and converted to connected
    /// line-segment geometry (see `crate::app::polyline_segments`)
    /// instead of drawn directly.
    pub(crate) fn output_visual(&self, n: usize) -> crate::app::CloudsExtra {
        let mode = self.params.playback_mode.load(Ordering::Relaxed) as usize % PLAYBACK_MODE_NAMES.len();
        let history = self.params.monitor_history.lock().unwrap();
        let raw: Vec<f32> = history.iter().copied().collect();
        drop(history);
        let waveform = if raw.len() >= 2 {
            let step = (raw.len() as f32 / n as f32).max(1.0);
            let resampled: Vec<f32> = (0..n).map(|i| raw[((i as f32 * step) as usize).min(raw.len() - 1)]).collect();
            let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(&resampled, 260.0, 190.0, true);
            crate::app::CurveSegments { mid_x, mid_y, length, angle_deg }
        } else {
            crate::app::CurveSegments::default()
        };
        crate::app::CloudsExtra {
            playback_mode: mode,
            playback_mode_name: PLAYBACK_MODE_NAMES[mode].to_string(),
            frozen: self.params.freeze.load(Ordering::Relaxed),
            waveform,
        }
    }
}

impl App for CloudsApp {
    fn needs_background_audio(&self) -> bool { self.params.input_levels.iter().any(|v| v.get() > 0.0) || self.params.ext_input_level.iter().any(|v| v.get() > 0.0) || self.params.freeze.load(Ordering::Relaxed) }
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
        crate::app::SlintExtra::Clouds(self.output_visual(60))
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
        // No grid usage -- Clouds processes whatever audio is routed
        // in, it isn't played directly.
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(CloudsProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            granulator: Granulator::new(),
            input_mono: Vec::new(),
            output_mono: Vec::new(),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(CLOUDS_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, CLOUDS_TITLE);
        Text::new("Clouds", Point::new(16, 30), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_6X12, CLOUDS_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, CLOUDS_DIM);

        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", group_name(*g)), self.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", self.leaf_name(*sel)), self.leaf_value(*sel)),
            })
            .collect();
        self.list.draw_themed(fb, 16, 44, 24, 10, &display_rows, CLOUDS_BG, CLOUDS_DIM, CLOUDS_ACCENT);

        // --- Right: live output monitor, same scrolling-waveform
        // technique as Plaits'/Pam's/Bloom's panels. ---
        let panel_x = 400;
        let panel_y = 44;
        let panel_w = 220;
        let panel_h = 250;
        let frozen = self.params.freeze.load(Ordering::Relaxed);
        let label = if frozen { "Output (frozen)" } else { "Output" };
        Text::new(label, Point::new(panel_x, panel_y - 4), accent).draw(fb).ok();

        let mid_y = panel_y + panel_h / 2;
        Line::new(Point::new(panel_x, mid_y), Point::new(panel_x + panel_w, mid_y))
            .into_styled(PrimitiveStyle::with_stroke(CLOUDS_MIDLINE, 1))
            .draw(fb)
            .ok();

        let history = self.params.monitor_history.lock().unwrap();
        if history.len() >= 2 {
            let step = panel_w as f32 / (history.len() - 1) as f32;
            let color = if frozen { CLOUDS_FROZEN } else { CLOUDS_ACCENT };
            let style = PrimitiveStyle::with_stroke(color, 1);
            for i in 0..history.len() - 1 {
                let v0 = history[i].clamp(-1.0, 1.0);
                let v1 = history[i + 1].clamp(-1.0, 1.0);
                let p0 = Point::new(panel_x + (i as f32 * step) as i32, mid_y - (v0 * panel_h as f32 / 2.0) as i32);
                let p1 =
                    Point::new(panel_x + ((i + 1) as f32 * step) as i32, mid_y - (v1 * panel_h as f32 / 2.0) as i32);
                Line::new(p0, p1).into_styled(style).draw(fb).ok();
            }
        }
        drop(history);

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(16, 337), dim).draw(fb).ok();
    }
}

struct CloudsProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    granulator: Granulator,
    input_mono: Vec<f32>,
    output_mono: Vec<f32>,
}

impl AudioProcessor for CloudsProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        self.input_mono.clear();
        self.input_mono.resize(frames, 0.0);

        // Input mixer: sum every routed source at its own level --
        // each level additionally modulatable via modbus.rs, on top of
        // the knob value, same additive pattern as everywhere else.
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

        let params = CloudsParams {
            playback_mode: self.params.playback_mode.load(Ordering::Relaxed) as i32,
            position: (self.params.position.get() + self.params.ext_position.get()).clamp(0.0, 1.0),
            size: (self.params.size.get() + self.params.ext_size.get()).clamp(0.0, 1.0),
            pitch: (self.params.pitch.get() + self.params.ext_pitch.get()).clamp(-48.0, 48.0),
            density: (self.params.density.get() + self.params.ext_density.get()).clamp(0.0, 1.0),
            texture: (self.params.texture.get() + self.params.ext_texture.get()).clamp(0.0, 1.0),
            dry_wet: (self.params.dry_wet.get() + self.params.ext_dry_wet.get()).clamp(0.0, 1.0),
            stereo_spread: self.params.stereo_spread.get().clamp(0.0, 1.0),
            feedback: (self.params.feedback.get() + self.params.ext_feedback.get()).clamp(0.0, 1.0),
            reverb: (self.params.reverb.get() + self.params.ext_reverb.get()).clamp(0.0, 1.0),
            freeze: self.params.freeze.load(Ordering::Relaxed),
            trigger: self.params.trigger.swap(false, Ordering::Relaxed),
        };

        self.output_mono.clear();
        self.output_mono.resize(frames, 0.0);
        self.granulator.render(&self.input_mono, &mut self.output_mono, sample_rate, &params);

        {
            let mut hist = self.params.monitor_history.lock().unwrap();
            for s in &self.output_mono {
                hist.push_back(*s);
                if hist.len() > MONITOR_LEN {
                    hist.pop_front();
                }
            }
        }

        {
            let mut bus_out = self.params.bus_out.lock().unwrap();
            bus_out.clear();
            bus_out.extend_from_slice(&self.output_mono);
        }

        // The Mixer app's channel fader for this app -- applied only
        // to what reaches the device, not to `bus_out` above (see
        // plaits.rs for the same pattern).
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
    /// app's own input list, not just the first 8. This is exactly
    /// the bug that made Tape and Voltage (alphabetically the last
    /// two of the 11 real apps that register) invisible as inputs
    /// here, even though `AudioBus` itself already tracked them fine.
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
        let app = CloudsApp::new(sensitivity, nav_speed, modbus, audio_bus, mixer_bus);

        // 12 test sources + this app's own registration of itself = 13.
        let leaves = app.group_leaves(0);
        assert_eq!(leaves.len(), 13, "expected all 13 registered sources (12 test ones + itself) to be selectable inputs, got {}", leaves.len());
        assert!(matches!(leaves[12], Selection::InputLevel(12)), "the last source (past the old cap of 8) must still be reachable");
    }
}
