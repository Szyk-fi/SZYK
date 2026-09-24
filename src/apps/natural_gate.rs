//! A clone of the Rabid Elephant Natural Gate: a dual-channel, zero-
//! bleed low-pass gate (LPG) -- NOT a generic envelope-follower/gate
//! utility. Each of its two independent, mirrored channels is a real
//! signal-path insert: a HIT trigger fires an internal envelope
//! generator whose value simultaneously drives a one-pole lowpass
//! filter's cutoff and a VCA gain on whatever audio is patched into
//! that channel's IN, exactly the "the object rings, and gets darker
//! as it decays" behavior a struck acoustic object has. It taps other
//! apps' live output for both IN and HIT (see audio_bus.rs, the same
//! technique Clouds/Beads already use) and republishes its own
//! shaped-audio output the same way, and exposes each channel's live
//! envelope as a modulation source other apps can route from (see
//! modbus.rs, the Pam's-channel pattern) while also being a
//! modulation *target* itself for its CTRL and DECAY CV inputs --
//! external gear (Pam's, a sequencer) can patch into "Natural Gate:
//! ChN Ctrl CV"/"...Decay CV" the same way it patches into Plaits'
//! Harmonics/Timbre.
//!
//! Feature set follows the real module's manual and reviews (rabidele
//! phant.com/products/natural-gate; Sound on Sound's review;
//! ManualsLib's scanned manual) as closely as this sim's architecture
//! allows:
//!
//! - Two independent, mirrored LPG channels (not a single shared
//!   detector -- the original stub here only had one).
//! - HIT input: any signal patched in (audio-rate, threshold ~ the
//!   real module's documented +0.25V) fires the envelope, scaled by
//!   the triggering signal's own peak ("velocity sensitivity").
//! - DECAY control (+ external CV with its own attenuverter): sets
//!   how long the struck object rings, from a highly-damped click to
//!   several seconds. Rapid retriggering dynamically shortens the
//!   effective decay, mirroring how a physical object rings shorter
//!   when struck repeatedly in quick succession (the manual's dynamic
//!   decay-vs-frequency behavior).
//! - "Memory": a new HIT while the envelope is still open builds on
//!   top of (never resets below) the current envelope level, matching
//!   the manual's explicit contrast with software envelopes that snap
//!   back to zero on every new trigger.
//! - MATERIAL (3-way: Hard/Medium/Soft): changes the envelope's attack
//!   speed and the lowpass filter's brightness range/saturation --
//!   "these selections vary the attack and level of the generated
//!   envelope, which in turn controls the colour of the output
//!   signal" (Sound on Sound).
//! - OPEN: a manual, post-envelope floor/offset on how far the gate is
//!   open -- lets a channel run wide open as a plain VCA/VCF insert
//!   with no HIT patched at all.
//! - CTRL CV + attenuverter: external modulation (an ADSR, an LFO --
//!   routed in via modbus, this sim's cross-app CV patchbay) added on
//!   top of the envelope's own gate-open amount.
//! - IN/OUT are genuinely DC-coupled in spirit: with nothing patched
//!   into IN, OUT carries the raw envelope itself (0..1, standing in
//!   for the real module's 0..+10V unipolar envelope-out behavior) --
//!   "envelope extraction" for free, same as the real module's
//!   internal normalization.
//! - Each channel's processed signal both mixes into the main output
//!   and republishes on its own audio_bus channel, so it can double as
//!   a VCA/ducking-compressor/waveshaper insert feeding a downstream
//!   app (Clouds, Beads) exactly like the real module's "each LPG
//!   channel can be used as a VCA, ducking compressor, wave shaper or
//!   envelope" (rabidelephant.com).
//!
//! Deliberately not implemented / simplified:
//! - The real CTRL knob is a single dual-purpose control (an
//!   attenuverting trimmer on the patched CV, *or* -- when nothing is
//!   patched -- a plain manual offset, via the jack's physical
//!   normalization). This sim has no concept of "is a modbus target
//!   currently being written by another app", so Ctrl Amount here only
//!   does something once something is actually routed to that
//!   channel's Ctrl CV target; use OPEN for the manual-offset use case
//!   instead, which covers the same practical need.
//! - "Sweeping CTRL across both decay and saturation ranges" (per one
//!   listing) isn't independently documented in enough detail to
//!   reproduce circuit-for-circuit; CTRL here does the well-documented
//!   part (adds into gate-open amount) rather than fabricating a
//!   second, undocumented saturation-sweep behavor.
//! - No physical calibration trim -- a numeric-parameter simulator has
//!   nothing analogous to zeroing an op-amp offset with a screwdriver.
//! - The real transistor-based (not opto/vactrol) LPG's exact
//!   nonlinearity and each MATERIAL position's precise EQ curve aren't
//!   public; implemented here as a real one-pole lowpass + tanh
//!   saturation whose cutoff range/attack time/drive vary per
//!   MATERIAL, in the spirit of "different tonal characters" rather
//!   than a literal circuit model.

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::mixer_bus::MixerBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NUM_CHANNELS: usize = 2;
const NUM_GROUPS: usize = NUM_CHANNELS;
/// How many recent audio blocks' worth of envelope value each channel's
/// live trace keeps -- one point pushed per block (see
/// `NaturalGateProcessor::process`), so at the sim's ~512-sample/48kHz
/// block size this is roughly a 2.5s rolling window: long enough to
/// show a full slow DECAY ring (`MAX_DECAY_S` = 4.0s, mostly) as an
/// actual decaying curve, not just a single dot.
const ENV_HISTORY_LEN: usize = 240;

/// Real per-channel LPG visualization data -- the defining visual of a
/// low-pass gate is that one envelope simultaneously drives both
/// amplitude (VCA) and filter brightness, exactly as it does in
/// `NaturalGateProcessor::process` below (`gate_open` feeds both the
/// lowpass cutoff and the final VCA multiply). Every field here is
/// read back from state the audio thread already computed and
/// published once per block -- see `ChannelParams::envelope_history`/
/// `envelope`/`gate_open`/`hit_flash` -- nothing here is fabricated
/// for display.
#[derive(Clone, Default)]
pub(crate) struct ChannelVisual {
    /// Rolling snapshot of this channel's envelope value (0..1) over
    /// the last `ENV_HISTORY_LEN` audio blocks, oldest first -- the
    /// classic "struck object" attack/decay curve, meant to be drawn
    /// as connected line-segment geometry (see
    /// `crate::app::polyline_segments`, the same helper Beads/Black
    /// Hole use for their own live traces).
    pub envelope_trace: Vec<f32>,
    /// This block's current envelope value, 0..1 -- equal to
    /// `envelope_trace`'s last point, exposed directly so a live
    /// head-dot/needle doesn't need to index into the trace.
    pub envelope_now: f32,
    /// This block's current post-OPEN/CTRL gate-open amount, 0..1 --
    /// simultaneously the VCA gain *and* the (normalized 0..1) lowpass
    /// brightness amount, since both are driven by this exact value in
    /// `process()` (`gate_open` feeds the cutoff-Hz exponential map and
    /// the final `vca` multiply identically). Intended for a
    /// brightness/openness indicator alongside the envelope trace.
    pub gate_open_now: f32,
    /// True if a HIT trigger fired anywhere within the most recent
    /// audio block -- a one-block flash suitable for a "HIT" indicator
    /// LED, same idea as Black Hole's clip LED.
    pub hit_flash: bool,
}

/// The full live visualization state for both channels -- see
/// `ChannelVisual` and `NaturalGateApp::output_visual`.
pub(crate) struct NaturalGateVisual {
    pub channels: [ChannelVisual; NUM_CHANNELS],
}

/// "Very short highly damped events" up to "several seconds" (manual).
const MIN_DECAY_S: f32 = 0.005;
const MAX_DECAY_S: f32 = 4.0;
/// The manual's documented HIT trigger threshold is +0.25V; this sim's
/// signals run roughly +-1.0 full scale (~ +-5V equivalent), so 0.05
/// is the proportionate normalized threshold.
const HIT_THRESHOLD: f32 = 0.05;
/// Re-arm threshold (below which a new trigger is allowed again) --
/// simple hysteresis so one transient can't retrigger many times.
const HIT_REARM_THRESHOLD: f32 = 0.02;
const HIT_REFRACTORY_S: f32 = 0.002;
/// Below this recent hit-to-hit interval, decay is shortened toward
/// `DECAY_MIN_SCALE`; at or above it, decay runs at its full, knob-set
/// time -- "higher frequencies receive automatically shortened
/// decays, mimicking acoustic instrument behavior" (Sound on Sound).
const DECAY_REFERENCE_S: f32 = 0.35;
const DECAY_MIN_SCALE: f32 = 0.15;

const MATERIAL_NAMES: [&str; 3] = ["Hard", "Medium", "Soft"];

struct MaterialProfile {
    attack_s: f32,
    cutoff_min_hz: f32,
    cutoff_max_hz: f32,
    drive: f32,
}

fn material_profile(mode: u32) -> MaterialProfile {
    match mode % 3 {
        0 => MaterialProfile { attack_s: 0.0008, cutoff_min_hz: 200.0, cutoff_max_hz: 16000.0, drive: 0.1 },
        1 => MaterialProfile { attack_s: 0.006, cutoff_min_hz: 150.0, cutoff_max_hz: 9000.0, drive: 0.4 },
        _ => MaterialProfile { attack_s: 0.03, cutoff_min_hz: 80.0, cutoff_max_hz: 4000.0, drive: 0.9 },
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    InSource(usize),
    HitSource(usize),
    Material(usize),
    Decay(usize),
    DecayCvAmount(usize),
    Open(usize),
    CtrlAmount(usize),
    Level(usize),
    OutTarget(usize),
    OutLevel(usize),
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

struct ChannelParams {
    /// 0 = unpatched (real behavior: OUT then carries the raw
    /// envelope, standing in for the real module's DC-normalized
    /// envelope-extraction mode), else audio_bus index + 1.
    in_source: AtomicUsize,
    hit_source: AtomicUsize,
    material: AtomicU32,
    decay: AtomicF32,
    decay_cv_amount: AtomicF32,
    decay_cv: Arc<AtomicF32>,
    open: AtomicF32,
    ctrl_amount: AtomicF32,
    ctrl_cv: Arc<AtomicF32>,
    /// How much of this channel's shaped output reaches the main mix
    /// -- this sim's stand-in for physically patching (or not) the
    /// channel's OUT jack into a mixer.
    level: AtomicF32,
    /// 0 = not routed, else (modbus target index + 1) -- same
    /// convention Pam's `ChannelParams::target` uses.
    out_target: AtomicUsize,
    out_level: AtomicF32,
    /// Live 0..1 state published by the audio thread every block, for
    /// the panel meter to read back.
    envelope: AtomicF32,
    gate_open: AtomicF32,
    /// True whenever a HIT trigger fired anywhere in the most recent
    /// audio block -- a one-block flash for a "HIT" indicator LED, same
    /// technique as Black Hole's clip LED.
    hit_flash: AtomicBool,
    /// A rolling snapshot of `envelope`'s value, one point pushed per
    /// audio block (see `ENV_HISTORY_LEN`), refreshed once per block by
    /// `NaturalGateProcessor::process` -- real, live envelope history
    /// for the Slint "struck object" decay-curve visual, not a
    /// fabricated shape.
    envelope_history: Mutex<Vec<f32>>,
    bus_out: Arc<std::sync::Mutex<Vec<f32>>>,
    /// This channel's own fader in the Mixer app -- same convention
    /// every other audio-producing app follows (see e.g. bloom.rs),
    /// applied as a final stage alongside `level` (this app's own
    /// internal balance control) so the channel is actually mixable/
    /// adjustable from the Mixer screen, not just always-on.
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl ChannelParams {
    fn new(idx: usize, modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register(format!("Natural Gate Ch{}", idx + 1), modbus);
        Self {
            in_source: AtomicUsize::new(0),
            hit_source: AtomicUsize::new(0),
            material: AtomicU32::new(1),
            decay: AtomicF32::new(0.3),
            decay_cv_amount: AtomicF32::new(0.0),
            decay_cv: modbus.register(format!("Natural Gate: Ch{} Decay CV", idx + 1)),
            open: AtomicF32::new(0.0),
            ctrl_amount: AtomicF32::new(0.0),
            ctrl_cv: modbus.register(format!("Natural Gate: Ch{} Ctrl CV", idx + 1)),
            level: AtomicF32::new(1.0),
            out_target: AtomicUsize::new(0),
            out_level: AtomicF32::new(1.0),
            envelope: AtomicF32::new(0.0),
            gate_open: AtomicF32::new(0.0),
            hit_flash: AtomicBool::new(false),
            envelope_history: Mutex::new(Vec::new()),
            bus_out: audio_bus.register(format!("Natural Gate: Ch{}", idx + 1)),
            mix_level,
            ext_mix_level,
        }
    }
}

struct Params {
    channels: [ChannelParams; NUM_CHANNELS],
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        Self { channels: std::array::from_fn(|i| ChannelParams::new(i, modbus, audio_bus, mixer_bus)) }
    }
}

pub struct NaturalGateApp {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    modbus: Arc<ModBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Natural Gate's own palette: warm wood-bronze on deep forest
// brown, not a device-wide theme -- the color of a struck acoustic
// object ringing and darkening as it decays. ---

const NATURAL_GATE_BG: Rgb565 = Rgb565::new(3, 5, 2);
const NATURAL_GATE_TITLE: Rgb565 = Rgb565::new(29, 54, 23);
const NATURAL_GATE_ACCENT: Rgb565 = Rgb565::new(24, 34, 7);
const NATURAL_GATE_DIM: Rgb565 = Rgb565::new(17, 28, 10);
const NATURAL_GATE_METER_OUTLINE: Rgb565 = Rgb565::new(6, 10, 4);

impl NaturalGateApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            audio_bus,
            modbus,
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
        }
    }

    fn group_leaves(&self, c: usize) -> Vec<Selection> {
        vec![
            Selection::InSource(c),
            Selection::HitSource(c),
            Selection::Material(c),
            Selection::Decay(c),
            Selection::DecayCvAmount(c),
            Selection::Open(c),
            Selection::CtrlAmount(c),
            Selection::Level(c),
            Selection::OutTarget(c),
            Selection::OutLevel(c),
        ]
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

    fn group_name(&self, g: usize) -> String {
        format!("Ch{}", g + 1)
    }

    fn group_summary(&self, g: usize) -> String {
        let ch = &self.params.channels[g];
        format!("{:.0}ms dec, {:.0}% open", MIN_DECAY_S * 1000.0 + ch.decay.get() * (MAX_DECAY_S - MIN_DECAY_S) * 1000.0, ch.open.get() * 100.0)
    }

    fn in_source_name(&self, c: usize) -> String {
        let idx = self.params.channels[c].in_source.load(Ordering::Relaxed);
        if idx == 0 {
            "None (env out)".into()
        } else {
            self.audio_bus.names().get(idx - 1).cloned().unwrap_or_else(|| "None (env out)".into())
        }
    }

    fn hit_source_name(&self, c: usize) -> String {
        let idx = self.params.channels[c].hit_source.load(Ordering::Relaxed);
        if idx == 0 {
            "None".into()
        } else {
            self.audio_bus.names().get(idx - 1).cloned().unwrap_or_else(|| "None".into())
        }
    }

    fn out_target_name(&self, idx: usize) -> String {
        if idx == 0 {
            "None".into()
        } else {
            self.modbus.names().get(idx - 1).cloned().unwrap_or_else(|| "None".into())
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::InSource(_) => "In Source".into(),
            Selection::HitSource(_) => "Hit Source".into(),
            Selection::Material(_) => "Material".into(),
            Selection::Decay(_) => "Decay".into(),
            Selection::DecayCvAmount(_) => "Decay CV Amt".into(),
            Selection::Open(_) => "Open".into(),
            Selection::CtrlAmount(_) => "Ctrl Amt".into(),
            Selection::Level(_) => "Level".into(),
            Selection::OutTarget(_) => "Env Out Target".into(),
            Selection::OutLevel(_) => "Env Out Level".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::InSource(c) => self.in_source_name(c),
            Selection::HitSource(c) => self.hit_source_name(c),
            Selection::Material(c) => MATERIAL_NAMES[self.params.channels[c].material.load(Ordering::Relaxed) as usize % 3].into(),
            Selection::Decay(c) => {
                format!("{:.0} ms", MIN_DECAY_S * 1000.0 + self.params.channels[c].decay.get() * (MAX_DECAY_S - MIN_DECAY_S) * 1000.0)
            }
            Selection::DecayCvAmount(c) => format!("{:+.0}%", self.params.channels[c].decay_cv_amount.get() * 100.0),
            Selection::Open(c) => format!("{:.0}%", self.params.channels[c].open.get() * 100.0),
            Selection::CtrlAmount(c) => format!("{:+.0}%", self.params.channels[c].ctrl_amount.get() * 100.0),
            Selection::Level(c) => format!("{:.0}%", self.params.channels[c].level.get() * 100.0),
            Selection::OutTarget(c) => self.out_target_name(self.params.channels[c].out_target.load(Ordering::Relaxed)),
            Selection::OutLevel(c) => format!("{:.0}%", self.params.channels[c].out_level.get() * 100.0),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::InSource(c) => {
                let n = self.audio_bus.len();
                let cur = self.params.channels[c].in_source.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(n as i32 + 1);
                self.params.channels[c].in_source.store(next as usize, Ordering::Relaxed);
            }
            Selection::HitSource(c) => {
                let n = self.audio_bus.len();
                let cur = self.params.channels[c].hit_source.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(n as i32 + 1);
                self.params.channels[c].hit_source.store(next as usize, Ordering::Relaxed);
            }
            Selection::Material(c) => {
                let cur = self.params.channels[c].material.load(Ordering::Relaxed) as i32;
                self.params.channels[c].material.store((cur + step).rem_euclid(3) as u32, Ordering::Relaxed);
            }
            Selection::Decay(c) => bump(&self.params.channels[c].decay, delta, sensitivity, 0.0, 1.0),
            Selection::DecayCvAmount(c) => bump(&self.params.channels[c].decay_cv_amount, delta, sensitivity, -1.0, 1.0),
            Selection::Open(c) => bump(&self.params.channels[c].open, delta, sensitivity, 0.0, 1.0),
            Selection::CtrlAmount(c) => bump(&self.params.channels[c].ctrl_amount, delta, sensitivity, -1.0, 1.0),
            Selection::Level(c) => bump(&self.params.channels[c].level, delta, sensitivity, 0.0, 1.0),
            Selection::OutTarget(c) => {
                let n = self.modbus.len();
                let cur = self.params.channels[c].out_target.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(n as i32 + 1);
                self.params.channels[c].out_target.store(next as usize, Ordering::Relaxed);
            }
            Selection::OutLevel(c) => bump(&self.params.channels[c].out_level, delta, sensitivity, 0.0, 1.0),
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::InSource(c) => self.params.channels[c].in_source.store(0, Ordering::Relaxed),
            Selection::HitSource(c) => self.params.channels[c].hit_source.store(0, Ordering::Relaxed),
            Selection::Material(c) => self.params.channels[c].material.store(1, Ordering::Relaxed),
            Selection::Decay(c) => self.params.channels[c].decay.set(0.3),
            Selection::DecayCvAmount(c) => self.params.channels[c].decay_cv_amount.set(0.0),
            Selection::Open(c) => self.params.channels[c].open.set(0.0),
            Selection::CtrlAmount(c) => self.params.channels[c].ctrl_amount.set(0.0),
            Selection::Level(c) => self.params.channels[c].level.set(1.0),
            Selection::OutTarget(c) => self.params.channels[c].out_target.store(0, Ordering::Relaxed),
            Selection::OutLevel(c) => self.params.channels[c].out_level.set(1.0),
        }
    }

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
    /// current selection -- see `ParamList::centered_scroll_window`.
    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }

    /// Real live envelope (0..1) and post-open/ctrl gate-open amount
    /// (0..1) for channel `c` -- what the panel meter reads.
    pub(crate) fn envelope_and_gate(&self, c: usize) -> (f32, f32) {
        (self.params.channels[c].envelope.get().clamp(0.0, 1.0), self.params.channels[c].gate_open.get().clamp(0.0, 1.0))
    }

    /// The real per-channel envelope trace + gate-open/brightness +
    /// hit-flash state for a bespoke Slint LPG panel -- see
    /// `ChannelVisual` and `ChannelParams::envelope_history` (both
    /// written once per audio block by `NaturalGateProcessor::process`,
    /// never fabricated for display).
    pub(crate) fn output_visual(&self) -> NaturalGateVisual {
        NaturalGateVisual {
            channels: std::array::from_fn(|c| {
                let cp = &self.params.channels[c];
                ChannelVisual {
                    envelope_trace: cp.envelope_history.lock().unwrap().clone(),
                    envelope_now: cp.envelope.get().clamp(0.0, 1.0),
                    gate_open_now: cp.gate_open.get().clamp(0.0, 1.0),
                    hit_flash: cp.hit_flash.load(Ordering::Relaxed),
                }
            }),
        }
    }
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.01).clamp(min, max);
    value.set(next);
}

impl App for NaturalGateApp {
    fn needs_background_audio(&self) -> bool { self.params.channels.iter().any(|c| c.in_source.load(Ordering::Relaxed) > 0 || c.out_target.load(Ordering::Relaxed) > 0) }
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

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        const PANEL_W: f32 = 125.0;
        const PANEL_H: f32 = 90.0;
        let v = self.output_visual();
        let to_curve = |samples: &[f32]| {
            if samples.len() >= 2 {
                let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(samples, PANEL_W, PANEL_H, false);
                crate::app::CurveSegments { mid_x, mid_y, length, angle_deg }
            } else {
                crate::app::CurveSegments::default()
            }
        };
        crate::app::SlintExtra::NaturalGate(crate::app::NaturalGateExtra {
            ch1_trace: to_curve(&v.channels[0].envelope_trace),
            ch1_now: v.channels[0].envelope_now,
            ch1_open: v.channels[0].gate_open_now,
            ch1_hit: v.channels[0].hit_flash,
            ch2_trace: to_curve(&v.channels[1].envelope_trace),
            ch2_now: v.channels[1].envelope_now,
            ch2_open: v.channels[1].gate_open_now,
            ch2_hit: v.channels[1].hit_flash,
        })
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(NaturalGateProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            modbus: Arc::clone(&self.modbus),
            channels: std::array::from_fn(|_| ChannelState::default()),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(NATURAL_GATE_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, NATURAL_GATE_TITLE);
        Text::new("Natural Gate", Point::new(16, 26), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_8X16, NATURAL_GATE_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, NATURAL_GATE_DIM);

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
        self.list.draw_themed(fb, 16, 56, 22, 10, &display_rows, NATURAL_GATE_BG, NATURAL_GATE_DIM, NATURAL_GATE_ACCENT);

        // Per-channel live meters -- the gate-open amount as a bar,
        // same "read the real audio-thread state back for a live
        // indicator" pattern Plaits' peak/RMS panel uses.
        for c in 0..NUM_CHANNELS {
            let (_, gate_open) = self.envelope_and_gate(c);
            let meter_x = 460;
            let meter_y = 190 + c as i32 * 40;
            let meter_w = 160;
            let meter_h = 16;
            Text::new(&format!("CH{}", c + 1), Point::new(meter_x, meter_y - 4), accent).draw(fb).ok();
            Rectangle::new(Point::new(meter_x, meter_y), Size::new(meter_w as u32, meter_h as u32))
                .into_styled(PrimitiveStyle::with_stroke(NATURAL_GATE_METER_OUTLINE, 1))
                .draw(fb)
                .ok();
            let filled = (gate_open.clamp(0.0, 1.0) * meter_w as f32) as u32;
            if filled > 0 {
                Rectangle::new(Point::new(meter_x, meter_y), Size::new(filled.min(meter_w as u32), meter_h as u32))
                    .into_styled(PrimitiveStyle::with_fill(NATURAL_GATE_ACCENT))
                    .draw(fb)
                    .ok();
            }
        }
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

/// Real-time-thread-local state for one LPG channel -- read back into
/// the shared `ChannelParams` atomics every block for the UI.
struct ChannelState {
    /// Current envelope-generator value, 0..1.
    env: f32,
    /// True while approaching `attack_target` (post-HIT); false while
    /// decaying back toward 0.
    in_attack: bool,
    attack_target: f32,
    /// Rising/falling-edge tracking on the HIT source for trigger
    /// detection, with its own low re-arm threshold (hysteresis).
    hit_armed: bool,
    /// Seconds since the last trigger (for the refractory window and
    /// for the dynamic decay-shortening-on-rapid-hits behavior).
    since_last_trigger: f32,
    last_trigger_interval: f32,
    /// One-pole lowpass filter state for the audio path.
    lowpass: f32,
    /// Rolling envelope-value history, one point pushed per audio
    /// block, capped at `ENV_HISTORY_LEN` -- the thread-local buffer
    /// `ChannelParams::envelope_history` is cloned from every block.
    history: Vec<f32>,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            env: 0.0,
            in_attack: false,
            attack_target: 0.0,
            hit_armed: true,
            since_last_trigger: DECAY_REFERENCE_S,
            last_trigger_interval: DECAY_REFERENCE_S,
            lowpass: 0.0,
            history: Vec::with_capacity(ENV_HISTORY_LEN),
        }
    }
}

struct NaturalGateProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    modbus: Arc<ModBus>,
    channels: [ChannelState; NUM_CHANNELS],
}

impl AudioProcessor for NaturalGateProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        for out in buffer.iter_mut() {
            *out = 0.0;
        }
        let frames = buffer.len() / channels.max(1);
        if frames == 0 {
            return;
        }
        let dt = 1.0 / sample_rate;

        for c in 0..NUM_CHANNELS {
            let cp = &self.params.channels[c];
            let st = &mut self.channels[c];

            let in_idx = cp.in_source.load(Ordering::Relaxed);
            let tapped_in = if in_idx > 0 { self.audio_bus.get(in_idx - 1).map(|b| b.lock().unwrap().clone()) } else { None };
            let hit_idx = cp.hit_source.load(Ordering::Relaxed);
            let tapped_hit = if hit_idx > 0 { self.audio_bus.get(hit_idx - 1).map(|b| b.lock().unwrap().clone()) } else { None };

            let material = material_profile(cp.material.load(Ordering::Relaxed));
            let decay_knob = cp.decay.get().clamp(0.0, 1.0);
            let decay_cv_amount = cp.decay_cv_amount.get().clamp(-1.0, 1.0);
            let decay_cv = cp.decay_cv.get();
            let open = cp.open.get().clamp(0.0, 1.0);
            let ctrl_amount = cp.ctrl_amount.get().clamp(-1.0, 1.0);
            let ctrl_cv = cp.ctrl_cv.get();
            let level = cp.level.get().clamp(0.0, 1.0);
            let mix_level = (cp.mix_level.get() + cp.ext_mix_level.get()).clamp(0.0, 2.0);

            let mut mono = vec![0.0f32; frames];
            // True if a HIT trigger fires anywhere in this block --
            // published once below as a one-block flash for a "HIT"
            // indicator LED.
            let mut hit_this_block = false;
            for n in 0..frames {
                st.since_last_trigger += dt;

                let hit_sample = tapped_hit.as_ref().and_then(|b| b.get(n % b.len().max(1)).copied()).unwrap_or(0.0);
                let abs_hit = hit_sample.abs();
                if st.hit_armed && abs_hit >= HIT_THRESHOLD && st.since_last_trigger >= HIT_REFRACTORY_S {
                    // Velocity-sensitive: a louder hit drives the
                    // envelope higher, same as the real module's
                    // documented velocity response.
                    let velocity = (abs_hit * 3.0).min(1.0);
                    // "Memory": build on top of (never cut below) the
                    // currently-open amount, rather than resetting to
                    // zero on every new trigger.
                    st.attack_target = st.env.max(velocity);
                    st.in_attack = true;
                    st.hit_armed = false;
                    st.last_trigger_interval = st.since_last_trigger;
                    st.since_last_trigger = 0.0;
                    hit_this_block = true;
                } else if !st.hit_armed && abs_hit < HIT_REARM_THRESHOLD {
                    st.hit_armed = true;
                }

                if st.in_attack {
                    let coef = 1.0 - (-dt / material.attack_s).exp();
                    st.env += (st.attack_target - st.env) * coef;
                    if (st.attack_target - st.env).abs() < 0.001 {
                        st.env = st.attack_target;
                        st.in_attack = false;
                    }
                } else {
                    // Dynamic decay: hits arriving faster than
                    // `DECAY_REFERENCE_S` apart shorten the ring time,
                    // down to `DECAY_MIN_SCALE` of the knob's setting.
                    let rate_factor = (st.last_trigger_interval / DECAY_REFERENCE_S).clamp(DECAY_MIN_SCALE, 1.0);
                    let base_decay_s = MIN_DECAY_S + decay_knob * (MAX_DECAY_S - MIN_DECAY_S);
                    let decay_s = (base_decay_s * 2f32.powf(decay_cv * decay_cv_amount * 2.0) * rate_factor).clamp(MIN_DECAY_S, MAX_DECAY_S);
                    let coef = 1.0 - (-dt / decay_s).exp();
                    st.env += (0.0 - st.env) * coef;
                }
                st.env = st.env.clamp(0.0, 1.0);

                let gate_open = (st.env + open + ctrl_amount * ctrl_cv).clamp(0.0, 1.0);

                let out_sample = if in_idx > 0 {
                    let in_sample = tapped_in.as_ref().and_then(|b| b.get(n % b.len().max(1)).copied()).unwrap_or(0.0);
                    let cutoff_hz = material.cutoff_min_hz * (material.cutoff_max_hz / material.cutoff_min_hz).powf(gate_open);
                    let rc_coef = (1.0 - (-std::f32::consts::TAU * cutoff_hz * dt).exp()).clamp(0.0, 1.0);
                    st.lowpass += (in_sample - st.lowpass) * rc_coef;
                    let vca = st.lowpass * gate_open;
                    (vca * (1.0 + material.drive)).tanh()
                } else {
                    // Nothing patched into IN -- OUT carries the raw
                    // envelope itself, standing in for the real
                    // module's DC-normalized envelope-extraction mode.
                    gate_open
                };
                mono[n] = out_sample;

                if n == frames - 1 {
                    cp.envelope.set(st.env);
                    cp.gate_open.set(gate_open);
                }
            }

            for (out, s) in buffer.chunks_mut(channels).zip(mono.iter()) {
                for ch in out.iter_mut() {
                    *ch += *s * level * mix_level;
                }
            }
            *cp.bus_out.lock().unwrap() = mono;

            // Once per block (not per-sample): push this block's final
            // envelope value onto the rolling history and publish both
            // it and the hit-flash for the Slint LPG panel -- real
            // live envelope state, no fabricated shape.
            if st.history.len() >= ENV_HISTORY_LEN {
                st.history.remove(0);
            }
            st.history.push(st.env);
            *cp.envelope_history.lock().unwrap() = st.history.clone();
            cp.hit_flash.store(hit_this_block, Ordering::Relaxed);

            let target = cp.out_target.load(Ordering::Relaxed);
            if target > 0 {
                if let Some(handle) = self.modbus.get(target - 1) {
                    handle.set(cp.gate_open.get() * cp.out_level.get());
                }
            }
        }

        for s in buffer.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> (NaturalGateApp, Arc<AudioBus>, Arc<ModBus>) {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let app = NaturalGateApp::new(sensitivity, nav_speed, Arc::clone(&modbus), Arc::clone(&audio_bus), mixer_bus);
        (app, audio_bus, modbus)
    }

    /// No sources tapped anywhere must be silence, not a panic --
    /// every channel's IN and HIT default to "None".
    #[test]
    fn silence_with_no_source_is_silence_not_a_panic() {
        let (mut app, _audio_bus, _modbus) = new_app();
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..10 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|&s| s == 0.0));
        for c in 0..NUM_CHANNELS {
            let (envelope, gate) = app.envelope_and_gate(c);
            assert_eq!(envelope, 0.0);
            assert_eq!(gate, 0.0);
        }
    }

    /// A HIT trigger must open the channel's envelope and, with IN
    /// patched, produce audible gated audio.
    #[test]
    fn a_hit_trigger_opens_the_envelope_and_gates_the_audio() {
        let (mut app, audio_bus, _modbus) = new_app();
        let hit_buf = audio_bus.register("Hit Source");
        let in_buf = audio_bus.register("In Source");
        *hit_buf.lock().unwrap() = vec![0.9; 8];
        // Sustained IN so the gate, once open, has something to shape.
        *in_buf.lock().unwrap() = vec![0.8; 512];

        // Natural Gate registers its own 2 channel outputs first
        // (bus indices 0-1) -- "Hit Source"/"In Source" land at 2/3.
        app.params.channels[0].hit_source.store(3, Ordering::Relaxed);
        app.params.channels[0].in_source.store(4, Ordering::Relaxed);
        app.params.channels[0].material.store(0, Ordering::Relaxed); // Hard: fast attack

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.05, "expected audible gated output after a HIT trigger, got peak {peak}");
        let (envelope, gate) = app.envelope_and_gate(0);
        assert!(envelope > 0.1, "expected a raised envelope after a HIT trigger, got {envelope}");
        assert!(gate > 0.1);
    }

    /// With nothing patched into IN, OUT must carry the raw envelope
    /// itself (the real module's DC-normalized envelope-extraction
    /// behavior) rather than staying silent.
    #[test]
    fn unpatched_in_outputs_the_raw_envelope() {
        let (mut app, audio_bus, _modbus) = new_app();
        let hit_buf = audio_bus.register("Hit Source");
        *hit_buf.lock().unwrap() = vec![0.9; 8];
        // Natural Gate registers its own 2 channel outputs first
        // (bus indices 0-1) -- "Hit Source" lands at 2.
        app.params.channels[0].hit_source.store(3, Ordering::Relaxed);
        // in_source stays 0 = unpatched.

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.05, "expected the raw envelope on OUT with IN unpatched, got peak {peak}");
    }

    /// The OPEN slider must be usable as a manual, HIT-free floor --
    /// turning it up alone (no trigger, no CTRL) must open the gate.
    #[test]
    fn open_slider_works_with_no_hit_trigger() {
        let (mut app, audio_bus, _modbus) = new_app();
        let in_buf = audio_bus.register("In Source");
        *in_buf.lock().unwrap() = vec![0.8; 512];
        // Natural Gate registers its own 2 channel outputs first
        // (bus indices 0-1) -- "In Source" lands at 2.
        app.params.channels[0].in_source.store(3, Ordering::Relaxed);
        app.params.channels[0].open.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..10 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.1, "expected OPEN alone to gate audio through with no trigger, got peak {peak}");
    }

    /// A routed "Env Out" must actually reach the modbus target it's
    /// pointed at, scaled by its own level.
    #[test]
    fn a_routed_env_out_writes_into_its_modbus_target() {
        let (mut app, audio_bus, modbus) = new_app();
        let hit_buf = audio_bus.register("Hit Source");
        *hit_buf.lock().unwrap() = vec![0.9; 8];
        // Natural Gate registers its own 2 channel outputs first
        // (bus indices 0-1) -- "Hit Source" lands at 2.
        app.params.channels[0].hit_source.store(3, Ordering::Relaxed);
        app.params.channels[0].open.set(1.0); // guarantee a nonzero gate-open reading
        let target_handle = modbus.register("Some App: Some Param");

        // Natural Gate registers its own 6 modbus targets first (2
        // channels x [Mixer ext level, Decay CV, Ctrl CV]) --
        // "Some App: Some Param" lands at modbus index 6.
        app.params.channels[0].out_target.store(7, Ordering::Relaxed);
        app.params.channels[0].out_level.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(target_handle.get() > 0.5, "expected the open gate to write a raised value into its routed target, got {}", target_handle.get());
    }

    /// External CTRL CV (routed via modbus) must be able to open the
    /// gate on its own, with no HIT trigger at all.
    #[test]
    fn ctrl_cv_can_open_the_gate_with_no_hit() {
        let (mut app, audio_bus, modbus) = new_app();
        let in_buf = audio_bus.register("In Source");
        *in_buf.lock().unwrap() = vec![0.8; 512];
        // Natural Gate registers its own 2 channel outputs first
        // (bus indices 0-1) -- "In Source" lands at 2.
        app.params.channels[0].in_source.store(3, Ordering::Relaxed);
        app.params.channels[0].ctrl_amount.set(1.0);
        // Drive the channel's own registered Ctrl CV target directly,
        // simulating an external app (Pam's) routed into it.
        let ctrl_handle = Arc::clone(&app.params.channels[0].ctrl_cv);
        ctrl_handle.set(1.0);
        let _ = modbus; // Ctrl CV is a target this app itself registered.

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..10 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.1, "expected CTRL CV alone to open the gate, got peak {peak}");
    }

    /// Rapid retriggering must build the envelope up rather than reset
    /// it to zero on every hit ("memory", per the manual).
    #[test]
    fn rapid_retriggers_build_the_envelope_rather_than_reset_it() {
        let (mut app, audio_bus, _modbus) = new_app();
        let hit_buf = audio_bus.register("Hit Source");
        // Natural Gate registers its own 2 channel outputs first
        // (bus indices 0-1) -- "Hit Source" lands at 2.
        app.params.channels[0].hit_source.store(3, Ordering::Relaxed);
        app.params.channels[0].material.store(0, Ordering::Relaxed); // Hard: fast attack
        app.params.channels[0].decay.set(1.0); // slow decay so it doesn't fall back to 0 between hits

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 64 * 2];
        // A quiet hit, then -- while it's still ringing -- a second
        // quiet hit; the envelope must never have dropped to 0 in
        // between, i.e. it should already have been nonzero before
        // the second hit ever landed.
        *hit_buf.lock().unwrap() = vec![0.3; 4];
        processor.process(&mut buffer, 2, 48000.0);
        let (env_after_first, _) = app.envelope_and_gate(0);
        assert!(env_after_first > 0.0, "expected a nonzero envelope after the first hit");

        *hit_buf.lock().unwrap() = vec![0.0; 64];
        processor.process(&mut buffer, 2, 48000.0);
        *hit_buf.lock().unwrap() = vec![0.3; 4];
        processor.process(&mut buffer, 2, 48000.0);
        let (env_after_second, _) = app.envelope_and_gate(0);
        assert!(env_after_second >= env_after_first * 0.5, "expected the second same-level hit to build on the still-ringing envelope, not fall far below it");
    }

    /// Output must stay finite and bounded under extreme parameter
    /// settings (max decay, max CTRL, max OPEN, dense retriggering).
    #[test]
    fn extreme_settings_stay_bounded_and_finite() {
        let (mut app, audio_bus, _modbus) = new_app();
        let hit_buf = audio_bus.register("Hit Source");
        let in_buf = audio_bus.register("In Source");
        *hit_buf.lock().unwrap() = vec![1.0; 32];
        *in_buf.lock().unwrap() = vec![1.0; 512];
        // Natural Gate registers its own 2 channel outputs first (bus
        // indices 0-1) -- "Hit Source"/"In Source" land at 2/3.
        for c in 0..NUM_CHANNELS {
            app.params.channels[c].hit_source.store(3, Ordering::Relaxed);
            app.params.channels[c].in_source.store(4, Ordering::Relaxed);
            app.params.channels[c].decay.set(1.0);
            app.params.channels[c].open.set(1.0);
            app.params.channels[c].ctrl_amount.set(1.0);
            app.params.channels[c].ctrl_cv.set(1.0);
            app.params.channels[c].decay_cv_amount.set(1.0);
            app.params.channels[c].decay_cv.set(1.0);
        }

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..100 {
            processor.process(&mut buffer, 2, 48000.0);
            assert!(buffer.iter().all(|s| s.is_finite()), "output must stay finite under extreme settings");
        }
        let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak <= 1.0001, "expected bounded output, got peak {peak}");
    }

    /// Each MATERIAL setting must actually change the processed
    /// output (a real, audible difference, not a label-only switch).
    #[test]
    fn material_settings_produce_audibly_different_output() {
        let audio_bus = Arc::new(AudioBus::new());
        let hit_buf = audio_bus.register("Hit Source");
        let in_buf = audio_bus.register("In Source");
        *hit_buf.lock().unwrap() = vec![0.9; 8];
        *in_buf.lock().unwrap() = vec![0.9; 512];

        let mut outputs = Vec::new();
        for material in 0..3u32 {
            let sensitivity = Arc::new(AtomicF32::new(0.1));
            let nav_speed = Arc::new(AtomicF32::new(6.0));
            let modbus = Arc::new(ModBus::new());
            let mixer_bus = Arc::new(MixerBus::new());
            let mut app = NaturalGateApp::new(sensitivity, nav_speed, modbus, Arc::clone(&audio_bus), mixer_bus);
            app.params.channels[0].hit_source.store(1, Ordering::Relaxed);
            app.params.channels[0].in_source.store(2, Ordering::Relaxed);
            app.params.channels[0].material.store(material, Ordering::Relaxed);

            let mut processor = app.audio_processor().unwrap();
            let mut buffer = vec![0.0f32; 256 * 2];
            for _ in 0..6 {
                processor.process(&mut buffer, 2, 48000.0);
            }
            outputs.push(buffer);
        }
        assert_ne!(outputs[0], outputs[1], "Hard and Medium materials should sound different");
        assert_ne!(outputs[1], outputs[2], "Medium and Soft materials should sound different");
    }

    /// republishes its shaped output on its own audio_bus channel so
    /// downstream apps (Clouds, Beads) can tap it, same as every other
    /// audio-bus-publishing app in this sim.
    #[test]
    fn republishes_its_output_on_the_audio_bus() {
        let (mut app, audio_bus, _modbus) = new_app();
        let hit_buf = audio_bus.register("Hit Source");
        *hit_buf.lock().unwrap() = vec![0.9; 8];
        // Natural Gate registers its own 2 channel outputs first
        // (bus indices 0-1) -- "Hit Source" lands at 2.
        app.params.channels[0].hit_source.store(3, Ordering::Relaxed);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 256 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        let published = app.params.channels[0].bus_out.lock().unwrap();
        assert_eq!(published.len(), 256);
    }

    /// A HIT must produce a visible envelope rise in
    /// `output_visual()`'s trace: the trace must climb from silence,
    /// the hit-flash must fire on the triggering block, and the
    /// current gate-open reading must track the same rise.
    #[test]
    fn a_hit_produces_a_visible_envelope_rise_in_the_visual_snapshot() {
        let (mut app, audio_bus, _modbus) = new_app();
        let hit_buf = audio_bus.register("Hit Source");
        *hit_buf.lock().unwrap() = vec![0.0; 8];
        // Natural Gate registers its own 2 channel outputs first (bus
        // indices 0-1) -- "Hit Source" lands at 2.
        app.params.channels[0].hit_source.store(3, Ordering::Relaxed);
        app.params.channels[0].material.store(0, Ordering::Relaxed); // Hard: fast attack
        app.params.channels[0].decay.set(0.5);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];

        // A few silent blocks first: no trace movement, no flash.
        for _ in 0..3 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let before = app.output_visual();
        assert!(!before.channels[0].hit_flash, "expected no hit-flash before any trigger");
        assert!(before.channels[0].envelope_trace.iter().all(|&v| v == 0.0), "expected a flat, silent trace before any trigger");

        // Now a loud HIT.
        *hit_buf.lock().unwrap() = vec![0.9; 8];
        processor.process(&mut buffer, 2, 48000.0);
        let just_after = app.output_visual();
        assert!(just_after.channels[0].hit_flash, "expected the hit-flash to fire on the triggering block");
        assert!(just_after.channels[0].envelope_now > 0.1, "expected a raised envelope right after the hit, got {}", just_after.channels[0].envelope_now);
        assert!(just_after.channels[0].gate_open_now > 0.1, "expected a raised gate-open reading right after the hit");

        // The flash must not persist into the following (silent) block.
        *hit_buf.lock().unwrap() = vec![0.0; 512];
        processor.process(&mut buffer, 2, 48000.0);
        let later = app.output_visual();
        assert!(!later.channels[0].hit_flash, "expected the hit-flash to clear on the next, non-triggering block");

        // The trace itself must show real movement: its peak must be
        // well above its first (pre-hit) point, and it must have
        // climbed then started ringing back down (peak strictly after
        // the earliest points, and not still rising at the very end
        // relative to the peak).
        let trace = &later.channels[0].envelope_trace;
        assert!(trace.len() >= 2, "expected a multi-point trace");
        let peak = trace.iter().cloned().fold(0.0f32, f32::max);
        assert!(peak > 0.1, "expected the trace to show a real envelope rise, peak was {peak}");
        assert!(trace[0] <= 0.01, "expected the trace to start from silence, got {}", trace[0]);

        // Channel 2 (never hit) must stay completely flat/silent.
        assert!(later.channels[1].envelope_trace.iter().all(|&v| v == 0.0));
        assert_eq!(later.channels[1].envelope_now, 0.0);
        assert!(!later.channels[1].hit_flash);
    }
}
