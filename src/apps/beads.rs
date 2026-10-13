//! Mutable Instruments Beads, running the module's own firmware DSP.
//!
//! Emilie Gillet published the Beads source under the MIT licence on
//! 12 Oct 2026 (pichenettes/eurorack, commit d8f4739). Everything that
//! shapes the sound here is that code, unmodified, compiled by build.rs
//! through vendor/bridge/beads_bridge.cc: the granular player and its 32
//! grains, the delay controller ("Beads as a delay"), the four recording
//! media and the sample rates they run at, wow and flutter, the reverb, the
//! limiters, the input AGC, the attenurandomizer and PITCH knob laws, and
//! the wavetable synth Beads falls back to after 10 seconds with nothing
//! patched. See the bridge's own top comment for the few things it had to
//! provide itself (the codec's sample-rate conversion, mainly).
//!
//! How the panel maps onto Portamax:
//!
//! - **IN L / IN R** are two Source rows. With only one picked, the other
//!   is normalled to it and Beads records in mono, with twice the buffer,
//!   exactly as on the module. With neither, the synth takes over after the
//!   module's 10 seconds.
//! - **The 12 knobs** are raw pot positions, run through the firmware's own
//!   pot laws (so PITCH snaps to 0/1/3/4/7/12/24 semitones and the
//!   attenurandomizers have their centre detent).
//! - **QUALITY** (the module's media button) cycles the four media, **FREEZE**
//!   is a toggle, and **the pads are the SEED button**: tap for a grain or a
//!   manual tempo, hold 1.5 s to lock it back into free-running mode, the
//!   firmware's own rules (ui.cc).
//! - **Modulation** adds onto the knobs, the way it does across this sim, so
//!   the attenurandomizers behave as on a module with no CV patched (their
//!   internal-randomness half). DENSITY's modulation enters as its CV.
//!   `Beads: Seed Gate` and `Beads: Freeze Gate` are the two gate inputs; the
//!   seed input counts as patched while it has gone high in the last 4 s.
//! - The output is true stereo (grains are panned); the audio bus gets the
//!   mono sum, like Elements.
//!
//! Not here: the ASSIGN button's CV routing (the modulation bus replaces
//! it), the manual-gain chord on the hardware (it's an Input gain row), the
//! gate output mode, calibration and the buffer backup to flash.

use crate::app::play_kit::{KitConfig, Knob, Layer, PlayHost, PlayKit, Routes, Throw};
use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16};
use crate::util::{accelerate, AtomicF32};
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Arc as ArcShape, Circle, Line, PrimitiveStyle, PrimitiveStyleBuilder, Rectangle};
use embedded_graphics::text::Text;
use std::ffi::c_void;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// --- The engine (vendor/bridge/beads_bridge.cc) -------------------------

/// Pot indices, in the firmware's `PotsAdcChannel` order.
const POT_FEEDBACK: usize = 0;
const POT_DRY_WET: usize = 1;
const POT_REVERB: usize = 2;
const POT_TIME_AMT: usize = 3;
const POT_SIZE_AMT: usize = 4;
const POT_SHAPE_AMT: usize = 5;
const POT_PITCH_AMT: usize = 6;
const POT_TIME: usize = 7;
const POT_DENSITY: usize = 8;
const POT_PITCH: usize = 9;
const POT_SIZE: usize = 10;
const POT_SHAPE: usize = 11;
const NUM_POTS: usize = 12;

const MAX_GRAINS: usize = 32;
const OVERVIEW: usize = 256;
const SCOPE: usize = 256;

/// Mirrors `BeadsControls` in the bridge (checked by a test).
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Controls {
    pots: [f32; NUM_POTS],
    time_offset: f32,
    size_offset: f32,
    shape_offset: f32,
    pitch_offset: f32,
    density_cv: f32,
    mix_offset: [f32; 3],
    media: i32,
    gain: i32,
    has_left: i32,
    has_right: i32,
    seed_button: i32,
    seed_patched: i32,
    seed_gate: i32,
    freeze_button: i32,
    freeze_gate: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GrainView {
    /// 0 = at the record head, 1 = the far end of the buffer (synth mode:
    /// the grain's wavetable phase).
    pub age: f32,
    pub env: f32,
    /// 0 = left, 1 = right.
    pub pan: f32,
    /// Playback speed, negative when reversed (synth mode: Hz).
    pub speed: f32,
}

/// Mirrors `BeadsView` in the bridge: what the engine is doing right now,
/// read straight out of it once per audio block.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct View {
    /// 0 granular, 1 delay, 2 synth, 3 codec restarting.
    pub mode: i32,
    pub media: i32,
    pub mono: i32,
    pub frozen: i32,
    pub codec_rate: f32,
    pub buffer_seconds: f32,
    pub recorded: f32,
    pub overwrite_reserve: f32,
    pub vu: f32,
    pub agc_adjusting: i32,
    pub synth_countdown: f32,
    pub seed_led: i32,
    pub num_grains: i32,
    pub grains: [GrainView; MAX_GRAINS],
    /// Peak level of the recording buffer by age, 0 = newest.
    pub overview: [f32; OVERVIEW],
    /// The latest output (every 4th sample), oldest first.
    pub scope: [f32; SCOPE],
}

impl Default for View {
    fn default() -> Self {
        View {
            mode: 0,
            media: 3,
            mono: 0,
            frozen: 0,
            codec_rate: 48000.0,
            buffer_seconds: 4.0,
            recorded: 0.0,
            overwrite_reserve: 1.0,
            vu: 0.0,
            agc_adjusting: 0,
            synth_countdown: -1.0,
            seed_led: 0,
            num_grains: 0,
            grains: [GrainView::default(); MAX_GRAINS],
            overview: [0.0; OVERVIEW],
            scope: [0.0; SCOPE],
        }
    }
}

impl View {
    fn grains(&self) -> &[GrainView] {
        &self.grains[..(self.num_grains.max(0) as usize).min(MAX_GRAINS)]
    }
}

unsafe extern "C" {
    #[allow(dead_code)] // the layout test
    fn beads_controls_size() -> c_int;
    #[allow(dead_code)] // the layout test
    fn beads_view_size() -> c_int;
    fn beads_create(host_rate: f32) -> *mut c_void;
    fn beads_destroy(handle: *mut c_void);
    #[allow(clippy::too_many_arguments)]
    fn beads_render(handle: *mut c_void, controls: *const Controls, in_l: *const f32, in_r: *const f32, out_l: *mut f32, out_r: *mut f32, n: c_int, host_rate: f32);
    fn beads_view(handle: *mut c_void, view: *mut View);
}

/// Owns one engine (about a megabyte: the module's 768 KB of recording RAM
/// plus its reverb buffer).
struct Engine {
    handle: *mut c_void,
}

// Only ever used from one thread at a time: built on the UI thread, then
// moved into the processor and touched only by the audio callback.
unsafe impl Send for Engine {}

impl Engine {
    fn new(host_rate: f32) -> Option<Engine> {
        let handle = unsafe { beads_create(host_rate) };
        (!handle.is_null()).then_some(Engine { handle })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { beads_destroy(self.handle) };
    }
}

// --- What the knobs mean (display only; the engine applies the real laws)

/// The four recording media, in the firmware's order (`Processor::Media`),
/// with the colour the module's QUALITY LED shows for each (ui.cc's
/// `media_palette_`, brightened for a screen).
const MEDIA: [(&str, &str, Rgb); 4] = [
    ("Dirty tape", "DIRTY TAPE", (236, 52, 104)),
    ("Clean tape", "CLEAN TAPE", (255, 158, 44)),
    ("Vintage digital", "VINTAGE DIGITAL", (40, 232, 190)),
    ("Digital", "DIGITAL", (206, 210, 232)),
];

/// cv_reader.h's LAW_DETUNE, for the PITCH value shown.
fn pitch_semitones(raw: f32) -> f32 {
    let detune = 2.0 * raw.clamp(0.0, 1.0) - 1.0;
    let sign = if detune < 0.0 { -1.0 } else { 1.0 };
    let d = detune * sign * 6.0;
    let integral = (d as usize).min(6);
    let fractional = ((d - integral as f32 - 0.333_333_33) * 1.5).max(0.0);
    const INTERVALS: [f32; 8] = [0.0, 1.0, 3.0, 4.0, 7.0, 12.0, 24.0, 24.0];
    let t = fractional.clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    (INTERVALS[integral] + (INTERVALS[integral + 1] - INTERVALS[integral]) * smooth) * sign
}

/// cv_reader.h's LAW_ATTENUVERTER: -1..1 with a dead zone at the centre.
fn attenurandomizer(raw: f32) -> f32 {
    let v = raw.clamp(0.0, 1.0) - 0.5;
    (2.0 / 0.47) * v * (v.abs() - 0.03).max(0.0)
}

/// granular_player.h's ComputeGrainDuration, in seconds (max 4 s).
fn grain_seconds(size: f32) -> (f32, bool) {
    let d = size.clamp(0.0, 1.0);
    let reverse = d < 1.0 / 3.0;
    let clamped = if reverse { d * 3.0 * (2.0 - d * 3.0) } else { 1.0 - 1.5 * (d - 1.0 / 3.0) };
    let shortness = (1.025 * clamped).min(1.0);
    (2f32.powf(shortness * -108.0 / 12.0) * 4.0, reverse)
}

/// trigger_generator.cc: what DENSITY does with no seed cable.
fn density_text(density: f32) -> String {
    const MAX_HZ: f32 = 130.812_78;
    if density < 0.48 {
        format!("{} regular", hz(MAX_HZ * 2f32.powf(-density * 16.0)))
    } else if density > 0.52 {
        format!("{} random", hz(0.5 * MAX_HZ * 2f32.powf(-(1.0 - density) * 16.0)))
    } else {
        "seed only".into()
    }
}

fn hz(f: f32) -> String {
    if f >= 10.0 {
        format!("{f:.0} Hz")
    } else if f >= 1.0 {
        format!("{f:.1} Hz")
    } else {
        format!("{:.1} s", 1.0 / f.max(1e-3))
    }
}

fn shape_name(shape: f32) -> &'static str {
    // grain_envelope.h's table: square, decay, bell, reverse.
    match shape {
        s if s < 0.2 => "square",
        s if s < 0.45 => "decay",
        s if s < 0.8 => "bell",
        _ => "reverse",
    }
}

/// gain_control.cc's manual gain, in dB (0 = automatic).
fn gain_text(gain: u32) -> String {
    if gain == 0 {
        return "auto".into();
    }
    let g = (gain.min(255) as f32 / 255.0).powi(4);
    format!("{:+.1} dB", 20.0 * (1.0 + 39.0 * g).log10())
}

// --- Parameters shared with the audio thread -----------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Selection {
    SourceL,
    SourceR,
    Quality,
    Gain,
    Freeze,
    Time,
    Size,
    Shape,
    Pitch,
    Density,
    TimeAmt,
    SizeAmt,
    ShapeAmt,
    PitchAmt,
    Feedback,
    DryWet,
    Reverb,
}

impl Selection {
    fn pot(self) -> Option<usize> {
        Some(match self {
            Selection::Time => POT_TIME,
            Selection::Size => POT_SIZE,
            Selection::Shape => POT_SHAPE,
            Selection::Pitch => POT_PITCH,
            Selection::Density => POT_DENSITY,
            Selection::TimeAmt => POT_TIME_AMT,
            Selection::SizeAmt => POT_SIZE_AMT,
            Selection::ShapeAmt => POT_SHAPE_AMT,
            Selection::PitchAmt => POT_PITCH_AMT,
            Selection::Feedback => POT_FEEDBACK,
            Selection::DryWet => POT_DRY_WET,
            Selection::Reverb => POT_REVERB,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Selection::SourceL => "In L",
            Selection::SourceR => "In R",
            Selection::Quality => "Quality",
            Selection::Gain => "Input gain",
            Selection::Freeze => "Freeze",
            Selection::Time => "Time",
            Selection::Size => "Size",
            Selection::Shape => "Shape",
            Selection::Pitch => "Pitch",
            Selection::Density => "Density",
            Selection::TimeAmt => "Time random",
            Selection::SizeAmt => "Size random",
            Selection::ShapeAmt => "Shape random",
            Selection::PitchAmt => "Pitch random",
            Selection::Feedback => "Feedback",
            Selection::DryWet => "Dry/Wet",
            Selection::Reverb => "Reverb",
        }
    }
}

/// Where the knobs start: a cloud of medium grains a moment back in the
/// buffer, mostly wet, a little reverb. The attenurandomizers sit on their
/// centre detent, as on a module fresh out of the box.
const DEFAULT_POTS: [f32; NUM_POTS] = [
    0.0,  // feedback
    0.65, // dry/wet
    0.3,  // reverb
    0.5, 0.5, 0.5, 0.5, // amounts
    0.12, // time
    0.8,  // density
    0.5,  // pitch
    0.62, // size
    0.5,  // shape
];
const DEFAULT_MEDIA: u32 = 3;

#[derive(Clone, Copy, PartialEq, Debug)]
enum SeedState {
    Off = 0,
    On = 1,
    Locked = 2,
}

/// The SEED button's rules from ui.cc: it starts locked (grains run free at
/// DENSITY); a press unlocks it and is a tap; holding it 1.5 s locks it again.
#[derive(Clone, Copy, Debug)]
struct SeedButton {
    locked: bool,
    on: bool,
    held: bool,
    ignore_release: bool,
    held_for: Duration,
}

const SEED_LONG_PRESS: Duration = Duration::from_millis(1500);

impl SeedButton {
    fn new() -> SeedButton {
        SeedButton { locked: true, on: false, held: false, ignore_release: false, held_for: Duration::ZERO }
    }

    /// `down`: any SEED pad held; `dt`: time since the last update.
    fn update(&mut self, down: bool, dt: Duration) {
        if down && !self.held {
            self.held = true;
            self.ignore_release = false;
            self.held_for = Duration::ZERO;
            self.locked = false;
            self.on = true;
        } else if down {
            self.held_for += dt;
            if !self.ignore_release && self.held_for >= SEED_LONG_PRESS {
                self.locked = true;
                self.on = false;
                self.ignore_release = true;
            }
        } else if self.held {
            self.held = false;
            self.on = false;
        }
    }

    fn state(&self) -> SeedState {
        if self.locked {
            SeedState::Locked
        } else if self.on {
            SeedState::On
        } else {
            SeedState::Off
        }
    }
}

struct Params {
    source_l: AtomicUsize,
    source_r: AtomicUsize,
    pots: [AtomicF32; NUM_POTS],
    media: AtomicU32,
    gain: AtomicU32,
    freeze: AtomicBool,
    seed: AtomicU32,
    ext_time: Arc<AtomicF32>,
    ext_size: Arc<AtomicF32>,
    ext_shape: Arc<AtomicF32>,
    ext_pitch: Arc<AtomicF32>,
    ext_density: Arc<AtomicF32>,
    ext_feedback: Arc<AtomicF32>,
    ext_dry_wet: Arc<AtomicF32>,
    ext_reverb: Arc<AtomicF32>,
    ext_seed: Arc<AtomicF32>,
    ext_freeze: Arc<AtomicF32>,
    /// The engine's state, copied out once per audio block.
    view: Mutex<View>,
    /// The last block's output peak, for the meter.
    out_peak: AtomicF32,
    /// Audio frames since the seed gate was last high (saturating).
    seed_quiet_frames: AtomicU64,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Beads", modbus);
        Self {
            source_l: AtomicUsize::new(crate::audio_bus::NO_SOURCE),
            source_r: AtomicUsize::new(crate::audio_bus::NO_SOURCE),
            pots: std::array::from_fn(|i| AtomicF32::new(DEFAULT_POTS[i])),
            media: AtomicU32::new(DEFAULT_MEDIA),
            gain: AtomicU32::new(0),
            freeze: AtomicBool::new(false),
            seed: AtomicU32::new(SeedState::Locked as u32),
            ext_time: modbus.register("Beads: Time"),
            ext_size: modbus.register("Beads: Size"),
            ext_shape: modbus.register("Beads: Shape"),
            ext_pitch: modbus.register("Beads: Pitch"),
            ext_density: modbus.register("Beads: Density"),
            ext_feedback: modbus.register("Beads: Feedback"),
            ext_dry_wet: modbus.register("Beads: Dry/Wet"),
            ext_reverb: modbus.register("Beads: Reverb"),
            ext_seed: modbus.register("Beads: Seed Gate"),
            ext_freeze: modbus.register("Beads: Freeze Gate"),
            view: Mutex::new(View::default()),
            out_peak: AtomicF32::new(0.0),
            seed_quiet_frames: AtomicU64::new(u64::MAX),
            bus_out: audio_bus.register("Beads"),
            mix_level,
            ext_mix_level,
        }
    }

    fn pot(&self, i: usize) -> f32 {
        self.pots[i].get()
    }

    fn media(&self) -> usize {
        self.media.load(Ordering::Relaxed) as usize % 4
    }

    fn has_input(&self) -> bool {
        self.source_l.load(Ordering::Relaxed) != crate::audio_bus::NO_SOURCE || self.source_r.load(Ordering::Relaxed) != crate::audio_bus::NO_SOURCE
    }
}

// --- The app -------------------------------------------------------------

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const GROUPS: [(&str, &[Selection]); 4] = [
    ("Inputs", &[Selection::SourceL, Selection::SourceR, Selection::Quality, Selection::Gain, Selection::Freeze]),
    ("Grains", &[Selection::Density, Selection::Time, Selection::Size, Selection::Shape, Selection::Pitch]),
    ("Randomize", &[Selection::TimeAmt, Selection::SizeAmt, Selection::ShapeAmt, Selection::PitchAmt]),
    ("Mix", &[Selection::Feedback, Selection::DryWet, Selection::Reverb]),
];

/// The play view's controls, most important first: where in the buffer and
/// how long, then how many and how high, the envelope and the loop, the mix,
/// the four attenurandomizers, and the two buttons.
const CONTROLS: [Selection; 14] = [
    Selection::Time,
    Selection::Size,
    Selection::Density,
    Selection::Pitch,
    Selection::Shape,
    Selection::Feedback,
    Selection::DryWet,
    Selection::Reverb,
    Selection::TimeAmt,
    Selection::SizeAmt,
    Selection::PitchAmt,
    Selection::ShapeAmt,
    Selection::Freeze,
    Selection::Quality,
];
const C_FREEZE: usize = 12;
const C_QUALITY: usize = 13;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "beads",
        // The pads are the module's SEED button.
        layers: vec![Layer::Native(0, "SEED"), Layer::Throws, Layer::Controls, Layer::Moments],
        hero: vec![[0, 1], [2, 3], [4, 5], [6, 7]],
        // The QUALITY button is the one a player reaches for most.
        browse: Some(C_QUALITY),
        // Stick: scrub the buffer on X, grain size on Y (pushed to the top
        // it tips into delay mode). Hands: density, and reverb.
        routes: Routes { stick_x: Some(0), stick_y: Some(1), hand_l: Some(2), hand_r: Some(7) },
        throws: vec![
            Throw { control: C_FREEZE, to: 1.0, label: "FREEZE" },
            // SIZE fully clockwise is "Beads as a delay".
            Throw { control: 1, to: 1.0, label: "DELAY" },
            // Below a third of its travel, SIZE plays grains backwards.
            Throw { control: 1, to: 0.22, label: "REVERSE" },
            Throw { control: 8, to: 1.0, label: "SCATTER" },
            Throw { control: 10, to: 1.0, label: "SPRAY" },
            Throw { control: 5, to: 0.85, label: "FEEDBACK" },
            // PITCH's law snaps to +12 at 11/12 of its travel.
            Throw { control: 3, to: 11.0 / 12.0, label: "OCT UP" },
            Throw { control: 3, to: 1.0 / 12.0, label: "OCT DN" },
        ],
        midi_to_pads: true,
        own_expression: false,
    }
}

pub struct BeadsApp {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; GROUPS.len()],
    kit: PlayKit,
    seed: SeedButton,
    last_tick: Option<Instant>,
    /// Frames drawn, for the SEED LED's slow pulse while locked.
    frame: u32,
    /// Output meter, decaying between frames.
    meter: f32,
}

impl BeadsApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            audio_bus,
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; GROUPS.len()],
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            seed: SeedButton::new(),
            last_tick: None,
            frame: 0,
            meter: 0.0,
        }
    }

    fn view(&self) -> View {
        *self.params.view.lock().unwrap()
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (g, (_, leaves)) in GROUPS.iter().enumerate() {
            rows.push(Row::Group(g));
            if self.expanded[g] {
                rows.extend(leaves.iter().map(|s| Row::Leaf(*s)));
            }
        }
        rows
    }

    fn source_name(&self, idx: usize) -> String {
        self.audio_bus.source_name(idx)
    }

    fn group_summary(&self, g: usize) -> String {
        let p = &self.params;
        match g {
            0 => {
                let l = p.source_l.load(Ordering::Relaxed);
                let r = p.source_r.load(Ordering::Relaxed);
                let input = match (l != crate::audio_bus::NO_SOURCE, r != crate::audio_bus::NO_SOURCE) {
                    (false, false) => "no input".to_string(),
                    (true, false) => format!("{} (mono)", self.source_name(l)),
                    (false, true) => format!("{} (mono)", self.source_name(r)),
                    (true, true) => format!("{} + {}", self.source_name(l), self.source_name(r)),
                };
                format!("{input}, {}{}", MEDIA[p.media()].0, if p.freeze.load(Ordering::Relaxed) { ", frozen" } else { "" })
            }
            1 => format!("{}, {}", self.leaf_value(Selection::Density), self.leaf_value(Selection::Size)),
            2 => {
                let on = [POT_TIME_AMT, POT_SIZE_AMT, POT_SHAPE_AMT, POT_PITCH_AMT].iter().filter(|&&i| attenurandomizer(p.pot(i)) != 0.0).count();
                if on == 0 { "all off".into() } else { format!("{on} of 4 on") }
            }
            _ => format!("{:.0}% wet, {:.0}% fb", p.pot(POT_DRY_WET) * 100.0, p.pot(POT_FEEDBACK) * 100.0),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        let p = &self.params;
        match sel {
            Selection::SourceL => self.source_name(p.source_l.load(Ordering::Relaxed)),
            Selection::SourceR => {
                let r = p.source_r.load(Ordering::Relaxed);
                if r == crate::audio_bus::NO_SOURCE { "normalled to L".into() } else { self.source_name(r) }
            }
            Selection::Quality => MEDIA[p.media()].0.into(),
            Selection::Gain => gain_text(p.gain.load(Ordering::Relaxed)),
            Selection::Freeze => if p.freeze.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Selection::Time => {
                let seconds = self.view().buffer_seconds * p.pot(POT_TIME);
                format!("{seconds:.1} s back")
            }
            Selection::Size => {
                let size = p.pot(POT_SIZE);
                if size > 0.99 {
                    return "delay".into();
                }
                let (s, reverse) = grain_seconds(size);
                let len = if s >= 1.0 { format!("{s:.1} s") } else { format!("{:.0} ms", s * 1000.0) };
                if reverse { format!("{len} rev") } else { len }
            }
            Selection::Shape => shape_name(p.pot(POT_SHAPE)).into(),
            Selection::Pitch => format!("{:+.0} st", pitch_semitones(p.pot(POT_PITCH))),
            Selection::Density => density_text(p.pot(POT_DENSITY)),
            Selection::TimeAmt | Selection::SizeAmt | Selection::ShapeAmt | Selection::PitchAmt => {
                let a = attenurandomizer(p.pot(sel.pot().unwrap()));
                if a == 0.0 { "off".into() } else { format!("{:+.0}%", a * 100.0) }
            }
            Selection::Feedback | Selection::DryWet | Selection::Reverb => format!("{:.0}%", p.pot(sel.pot().unwrap()) * 100.0),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let p = &self.params;
        let step = delta.signum();
        match sel {
            Selection::SourceL => {
                let cur = p.source_l.load(Ordering::Relaxed);
                p.source_l.store(crate::audio_bus::cycle_source(cur, step, self.audio_bus.len()), Ordering::Relaxed);
            }
            Selection::SourceR => {
                let cur = p.source_r.load(Ordering::Relaxed);
                p.source_r.store(crate::audio_bus::cycle_source(cur, step, self.audio_bus.len()), Ordering::Relaxed);
            }
            Selection::Quality => {
                let cur = p.media.load(Ordering::Relaxed) as i32;
                p.media.store((cur + step).rem_euclid(4) as u32, Ordering::Relaxed);
            }
            Selection::Gain => {
                let cur = p.gain.load(Ordering::Relaxed) as i32;
                let next = (cur + accelerate(delta).round() as i32).clamp(0, 255);
                p.gain.store(next as u32, Ordering::Relaxed);
            }
            Selection::Freeze => p.freeze.store(delta > 0, Ordering::Relaxed),
            _ => {
                let pot = &p.pots[sel.pot().unwrap()];
                pot.set((pot.get() + accelerate(delta) * self.sensitivity.get() * 0.01).clamp(0.0, 1.0));
            }
        }
    }

    fn reset(&mut self, sel: Selection) {
        let p = &self.params;
        match sel {
            Selection::SourceL => p.source_l.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed),
            Selection::SourceR => p.source_r.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed),
            Selection::Quality => p.media.store(DEFAULT_MEDIA, Ordering::Relaxed),
            Selection::Gain => p.gain.store(0, Ordering::Relaxed),
            Selection::Freeze => p.freeze.store(false, Ordering::Relaxed),
            _ => {
                let i = sel.pot().unwrap();
                p.pots[i].set(DEFAULT_POTS[i]);
            }
        }
    }

    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", GROUPS[*g].0), self.group_summary(*g), true)
                }
                Row::Leaf(sel) => (sel.name().to_string(), self.leaf_value(*sel), false),
            })
            .collect()
    }

    #[allow(dead_code)] // the Slint screens
    pub(crate) fn selected_row(&self) -> usize {
        self.list.selected
    }

    #[allow(dead_code)] // the Slint screens
    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }

    /// For the Slint screen: the recording buffer as a line (oldest on the
    /// left) and each grain's place in it.
    #[allow(dead_code)] // the Slint screens
    pub(crate) fn output_visual(&self) -> crate::app::BeadsExtra {
        const PANEL_W: f32 = 260.0;
        const PANEL_H: f32 = 150.0;
        let v = self.view();
        let raw: Vec<f32> = (0..120).map(|i| v.overview[OVERVIEW - 1 - i * OVERVIEW / 120]).collect();
        let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(&raw, PANEL_W, PANEL_H, true);
        crate::app::BeadsExtra {
            mode_name: mode_name(&v).to_string(),
            frozen: v.frozen != 0 || self.params.freeze.load(Ordering::Relaxed),
            is_delay_mode: v.mode == 1,
            waveform: crate::app::CurveSegments { mid_x, mid_y, length, angle_deg },
            grain_dots: v.grains().iter().map(|g| (1.0 - g.age, g.env)).collect(),
        }
    }

    fn knob(&self, i: usize) -> Knob<'_> {
        let p = &self.params;
        match CONTROLS[i % CONTROLS.len()] {
            Selection::Freeze => Knob::B(&p.freeze),
            Selection::Quality => Knob::U(&p.media, 4),
            sel => match sel.pot() {
                Some(pot) => Knob::F(&p.pots[pot], 0.0, 1.0),
                None => Knob::None,
            },
        }
    }
}

fn mode_name(v: &View) -> &'static str {
    match v.mode {
        1 => "Delay",
        2 => "Synth",
        3 => "Switching",
        _ => MEDIA[(v.media.clamp(0, 3)) as usize].0,
    }
}

impl PlayHost for BeadsApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS.len()
    }
    fn kit_label(&self, i: usize) -> String {
        let sel = CONTROLS[i % CONTROLS.len()];
        match sel {
            Selection::TimeAmt => "Time rnd".into(),
            Selection::SizeAmt => "Size rnd".into(),
            Selection::ShapeAmt => "Shape rnd".into(),
            Selection::PitchAmt => "Pitch rnd".into(),
            _ => sel.name().into(),
        }
    }
    fn kit_value(&self, i: usize) -> String {
        self.leaf_value(CONTROLS[i % CONTROLS.len()])
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        self.knob(i).norm()
    }
    fn kit_stepped(&self, i: usize) -> bool {
        self.knob(i).stepped()
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit(CONTROLS[i % CONTROLS.len()], delta);
    }
    fn kit_reset(&mut self, i: usize) {
        self.reset(CONTROLS[i % CONTROLS.len()]);
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        self.knob(i).set(v);
    }
    fn kit_pad_label(&self, _layer: u8, _pad: usize) -> String {
        "SEED".into()
    }
    fn kit_pad_color(&self, _layer: u8, _pad: usize, held: bool) -> crate::led_output::PadColor {
        use crate::led_output::PadColor;
        if held {
            PadColor::Green
        } else if self.seed.locked {
            PadColor::Blue
        } else {
            PadColor::Off
        }
    }
    fn kit_line(&self) -> String {
        if self.seed.locked {
            "Seed free-running. Tap a pad to seed by hand".into()
        } else {
            "Seeding by hand. Hold a pad 1.5 s to free-run".into()
        }
    }
}

impl App for BeadsApp {
    fn needs_background_audio(&self) -> bool {
        self.params.has_input() || self.params.freeze.load(Ordering::Relaxed)
    }
    fn supports_pad_lock(&self) -> bool {
        true
    }
    fn play_surface(&self) -> bool {
        true
    }
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

    fn tick(&mut self, input: &Input) {
        // The play view takes the knobs and D-pad first; in the menu they
        // pass straight through. The pads reach SEED only on that layer.
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let input = &step.input;
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

        let now = Instant::now();
        let dt = self.last_tick.map(|t| now.duration_since(t)).unwrap_or_default();
        self.last_tick = Some(now);
        self.seed.update(input.grid.iter().any(|g| *g), dt);
        self.params.seed.store(self.seed.state() as u32, Ordering::Relaxed);
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        crate::app::SlintExtra::Beads(self.output_visual())
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let engine = Engine::new(48000.0)?;
        Some(Box::new(BeadsProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            engine,
            in_l: Vec::new(),
            in_r: Vec::new(),
            out_l: Vec::new(),
            out_r: Vec::new(),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        self.frame = self.frame.wrapping_add(1);
        let v = self.view();
        self.meter = (self.meter * 0.85).max(self.params.out_peak.get());
        let accent = MEDIA[v.media.clamp(0, 3) as usize].2;
        let freeze = self.params.freeze.load(Ordering::Relaxed);
        fill(fb, 0, 0, WIDTH as i32, HEIGHT as i32, BG);
        self.draw_header(fb, &v, accent);
        if self.kit.menu {
            draw_buffer(fb, &v, freeze, 16, 44, 608, 74, accent, true);
            let rows: Vec<(String, String)> = self
                .visible_rows()
                .iter()
                .map(|row| match row {
                    Row::Group(g) => {
                        let arrow = if self.expanded[*g] { "v" } else { ">" };
                        (format!("{arrow} {}", GROUPS[*g].0), self.group_summary(*g))
                    }
                    Row::Leaf(sel) => (format!("    {}", sel.name()), self.leaf_value(*sel)),
                })
                .collect();
            self.list.draw_themed(fb, 16, 140, 22, 9, &rows, c(BG), c(mix(DIM, INK, 0.45)), c(accent));
        } else {
            draw_buffer(fb, &v, freeze, 16, 44, 608, 150, accent, false);
            self.draw_readout(fb, &v, accent, 226);
            if let Some(col) = self.play_column() {
                draw_controls(fb, &col, accent, 240);
            }
        }
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

// --- Drawing -------------------------------------------------------------

type Rgb = (u8, u8, u8);

const BG: Rgb = (12, 13, 17);
const PANEL: Rgb = (20, 22, 28);
const FAINT: Rgb = (40, 43, 53);
const DIM: Rgb = (118, 124, 140);
const INK: Rgb = (238, 234, 224);
const ICE: Rgb = (132, 214, 255);
const REC: Rgb = (255, 78, 70);

fn c(rgb: Rgb) -> Rgb565 {
    Rgb565::new(rgb.0 >> 3, rgb.1 >> 2, rgb.2 >> 3)
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

fn fill(fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, color: Rgb) {
    if w > 0 && h > 0 {
        Rectangle::new(Point::new(x, y), Size::new(w as u32, h as u32)).into_styled(PrimitiveStyle::with_fill(c(color))).draw(fb).ok();
    }
}

fn frame(fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, color: Rgb) {
    Rectangle::new(Point::new(x, y), Size::new(w.max(1) as u32, h.max(1) as u32)).into_styled(PrimitiveStyle::with_stroke(c(color), 1)).draw(fb).ok();
}

fn vline(fb: &mut FrameBuffer, x: i32, y0: i32, y1: i32, color: Rgb) {
    Line::new(Point::new(x, y0), Point::new(x, y1)).into_styled(PrimitiveStyle::with_stroke(c(color), 1)).draw(fb).ok();
}

fn text(fb: &mut FrameBuffer, s: &str, x: i32, y: i32, font: &MonoFont<'_>, color: Rgb) {
    Text::new(s, Point::new(x, y), MonoTextStyle::new(font, c(color))).draw(fb).ok();
}

fn text_width(s: &str, font: &MonoFont<'_>) -> i32 {
    s.chars().count() as i32 * font.character_size.width as i32
}

fn text_right(fb: &mut FrameBuffer, s: &str, right: i32, y: i32, font: &MonoFont<'_>, color: Rgb) {
    text(fb, s, right - text_width(s, font), y, font, color);
}

fn text_center(fb: &mut FrameBuffer, s: &str, cx: i32, y: i32, font: &MonoFont<'_>, color: Rgb) {
    text(fb, s, cx - text_width(s, font) / 2, y, font, color);
}

fn dot(fb: &mut FrameBuffer, cx: i32, cy: i32, d: u32, color: Rgb) {
    Circle::with_center(Point::new(cx, cy), d).into_styled(PrimitiveStyle::with_fill(c(color))).draw(fb).ok();
}

fn ring(fb: &mut FrameBuffer, cx: i32, cy: i32, d: u32, color: Rgb) {
    Circle::with_center(Point::new(cx, cy), d).into_styled(PrimitiveStyle::with_stroke(c(color), 1)).draw(fb).ok();
}

fn pill(fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, color: Rgb) {
    let r = h / 2;
    fill(fb, x + r, y, w - 2 * r, h, color);
    dot(fb, x + r, y + r, (2 * r) as u32 + 1, color);
    dot(fb, x + w - r - 1, y + r, (2 * r) as u32 + 1, color);
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

impl BeadsApp {
    fn draw_header(&self, fb: &mut FrameBuffer, v: &View, accent: Rgb) {
        text(fb, "Beads", 16, 30, &SPLEEN_16X32, INK);
        text(fb, "TEXTURE SYNTHESIZER", 102, 22, &SPLEEN_6X12, DIM);
        let sub = if v.mode == 2 {
            "synth mode".to_string()
        } else {
            format!("{:.0} kHz {} {:.0} s", v.codec_rate / 1000.0, if v.mono != 0 { "mono" } else { "stereo" }, v.buffer_seconds)
        };
        text(fb, &sub, 102, 34, &SPLEEN_6X12, mix(DIM, accent, 0.5));

        // The QUALITY chip, in the colour the module's LED shows, dimmed as
        // the buffer nears overwriting what was there (the LED does too).
        let name = MEDIA[v.media.clamp(0, 3) as usize].1;
        let w = text_width(name, &SPLEEN_8X16) + 22;
        let x = 344 - w / 2;
        pill(fb, x, 12, w, 20, mix(BG, accent, 0.25 + 0.75 * v.overwrite_reserve));
        text_center(fb, name, x + w / 2, 27, &SPLEEN_8X16, BG);

        // FREEZE and SEED, as the module's two lit buttons.
        let frozen = self.params.freeze.load(Ordering::Relaxed) || v.frozen != 0;
        let fx = 450;
        dot(fb, fx, 22, 11, if frozen { ICE } else { FAINT });
        text(fb, "FREEZE", fx + 10, 26, &SPLEEN_6X12, if frozen { ICE } else { DIM });
        let pulse = {
            let t = (self.frame % 90) as f32 / 90.0;
            0.35 + 0.65 * (1.0 - (2.0 * t - 1.0).abs())
        };
        let seed_on = self.seed.on || v.seed_led != 0;
        let seed_color = if seed_on { INK } else if self.seed.locked { mix(FAINT, INK, pulse) } else { FAINT };
        let sx = 518;
        dot(fb, sx, 22, 11, seed_color);
        text(fb, "SEED", sx + 10, 26, &SPLEEN_6X12, if seed_on { INK } else { DIM });

        // Input level: the module's single VU LED as a short bar, blinking
        // while the automatic gain is still learning the level.
        let vx = 572;
        text(fb, "IN", vx, 26, &SPLEEN_6X12, DIM);
        let segs = 6;
        for i in 0..segs {
            let lit = v.vu * segs as f32 > i as f32 + 0.5;
            let col = if v.agc_adjusting != 0 && (self.frame / 8) % 2 == 0 {
                FAINT
            } else if lit {
                if i == segs - 1 { REC } else { mix(accent, INK, 0.3) }
            } else {
                FAINT
            };
            fill(fb, vx + 16 + i * 6, 17, 4, 10, col);
        }
    }

    fn draw_readout(&self, fb: &mut FrameBuffer, v: &View, accent: Rgb, y: i32) {
        let p = &self.params;
        let items: Vec<(&str, String)> = if v.mode == 2 {
            vec![("SYNTH", "no input, so Beads plays its own wavetables".into())]
        } else {
            vec![
                ("GRAINS", v.num_grains.to_string()),
                ("DENSITY", self.leaf_value(Selection::Density)),
                ("SIZE", self.leaf_value(Selection::Size)),
                ("PITCH", self.leaf_value(Selection::Pitch)),
                ("SHAPE", shape_name(p.pot(POT_SHAPE)).into()),
            ]
        };
        let mut x = 16;
        for (label, value) in items {
            text(fb, label, x, y, &SPLEEN_6X12, DIM);
            let lx = x + text_width(label, &SPLEEN_6X12) + 6;
            text(fb, &value, lx, y, &SPLEEN_6X12, mix(INK, accent, 0.25));
            x = lx + text_width(&value, &SPLEEN_6X12) + 16;
        }
        let meter = self.meter.min(1.0);
        text_right(fb, "OUT", 578, y, &SPLEEN_6X12, DIM);
        fill(fb, 584, y - 7, 40, 4, FAINT);
        fill(fb, 584, y - 7, (40.0 * meter) as i32, 4, if meter > 0.97 { REC } else { accent });
    }
}

/// The recording buffer, newest audio at the right edge (the record head),
/// with every playing grain as a bead at its place in it: across by how far
/// back it reads, down by its pan, size and brightness by its envelope.
#[allow(clippy::too_many_arguments)]
fn draw_buffer(fb: &mut FrameBuffer, v: &View, freeze_button: bool, x: i32, y: i32, w: i32, h: i32, accent: Rgb, compact: bool) {
    fill(fb, x, y, w, h, PANEL);
    if v.mode == 2 {
        draw_synth(fb, v, x, y, w, h, accent, compact);
        return;
    }
    let frozen = v.frozen != 0 || freeze_button;
    let mid = y + h / 2;

    // Second marks, behind everything.
    let secs = v.buffer_seconds.max(0.1);
    let step = if secs <= 5.0 { 0.5 } else if secs <= 10.0 { 1.0 } else if secs <= 20.0 { 2.0 } else { 4.0 };
    let mut t = step;
    while t < secs - step * 0.25 {
        let gx = x + w - 1 - ((t / secs) * (w - 1) as f32) as i32;
        let mut gy = y + 2;
        while gy < y + h - 2 {
            fill(fb, gx, gy, 1, 2, FAINT);
            gy += 5;
        }
        if !compact {
            let label = if step < 1.0 { format!("-{t:.1}s") } else { format!("-{t:.0}s") };
            text_center(fb, &label, gx, y + h + 13, &SPLEEN_6X12, DIM);
        }
        t += step;
    }
    if !compact {
        text_right(fb, "now", x + w, y + h + 13, &SPLEEN_6X12, if frozen { ICE } else { REC });
    }

    // The buffer's contents.
    let wave = if frozen { mix(FAINT, ICE, 0.45) } else { mix(FAINT, accent, 0.4) };
    let wave_hot = if frozen { ICE } else { mix(accent, INK, 0.15) };
    let half = (h / 2 - 6) as f32;
    for px in 0..w {
        let age = 1.0 - px as f32 / (w - 1) as f32;
        let gx = x + px;
        if age > v.recorded + 1e-3 {
            if px % 4 == 0 {
                fill(fb, gx, mid, 1, 1, FAINT);
            }
            continue;
        }
        let k = ((age * (OVERVIEW - 1) as f32) as usize).min(OVERVIEW - 1);
        let peak = v.overview[k].clamp(0.0, 1.0);
        let a = (peak.powf(0.7) * half).max(1.0) as i32;
        vline(fb, gx, mid - a, mid + a, mix(wave, wave_hot, peak * 0.7));
    }

    // Grains, or in delay mode the delay controller's play heads.
    let grains = v.grains();
    if v.mode == 1 {
        for g in grains {
            let gx = x + w - 1 - (g.age * (w - 1) as f32) as i32;
            let col = mix(PANEL, INK, 0.3 + 0.7 * g.env);
            let reach = ((h / 2 - 4) as f32 * (0.25 + 0.75 * g.env)) as i32;
            vline(fb, gx, mid - reach, mid + reach, col);
            dot(fb, gx, mid - reach, 5, col);
            dot(fb, gx, mid + reach, 5, col);
        }
    } else {
        // Dimmest first, so the loudest beads sit on top.
        let mut order: Vec<&GrainView> = grains.iter().collect();
        order.sort_by(|a, b| a.env.total_cmp(&b.env));
        for g in order {
            let gx = x + w - 1 - (g.age * (w - 1) as f32) as i32;
            let gy = y + 9 + (g.pan.clamp(0.0, 1.0) * (h - 18) as f32) as i32;
            let size = if compact { 3.0 + 5.0 * g.env } else { 4.0 + 11.0 * g.env };
            dot(fb, gx, gy, (size * 2.3) as u32, mix(PANEL, accent, 0.22 * g.env + 0.06));
            if g.speed < 0.0 {
                ring(fb, gx, gy, size as u32 + 2, mix(accent, INK, g.env));
                ring(fb, gx, gy, size as u32, mix(accent, INK, g.env));
            } else {
                dot(fb, gx, gy, size as u32, mix(accent, INK, 0.15 + 0.6 * g.env));
            }
        }
    }

    // The record head.
    let head = if frozen { ICE } else { REC };
    fill(fb, x + w - 2, y, 2, h, head);
    if !compact {
        let label = if frozen { "FROZEN" } else if v.mode == 1 { "DELAY" } else { "REC" };
        let lw = text_width(label, &SPLEEN_6X12) + 10;
        fill(fb, x + w - 2 - lw, y, lw, 14, head);
        text(fb, label, x + w - lw + 3, y + 11, &SPLEEN_6X12, BG);
    }

    if v.mode == 3 {
        fill(fb, x + w / 2 - 90, mid - 12, 180, 24, PANEL);
        text_center(fb, "SWITCHING MEDIUM", x + w / 2, mid + 5, &SPLEEN_8X16, DIM);
    } else if v.synth_countdown >= 0.0 {
        let msg = format!("NO INPUT - SYNTH IN {:.0} S", v.synth_countdown.ceil());
        let bh = if compact { 26 } else { 44 };
        fill(fb, x + w / 2 - 140, mid - bh / 2, 280, bh, PANEL);
        frame(fb, x + w / 2 - 140, mid - bh / 2, 280, bh, FAINT);
        text_center(fb, &msg, x + w / 2, mid + if compact { 5 } else { -2 }, &SPLEEN_8X16, INK);
        if !compact {
            text_center(fb, "R1 menu, Inputs, In L picks a source", x + w / 2, mid + 14, &SPLEEN_6X12, DIM);
        }
    }
    frame(fb, x, y, w, h, FAINT);
}

/// Synth mode: no buffer, so each wavetable grain sits on a pitch axis
/// (log Hz) and rides its own phase up and down.
#[allow(clippy::too_many_arguments)]
fn draw_synth(fb: &mut FrameBuffer, v: &View, x: i32, y: i32, w: i32, h: i32, accent: Rgb, compact: bool) {
    let lo = 20f32.log2();
    let hi = 5000f32.log2();
    for f in [50.0f32, 100.0, 200.0, 500.0, 1000.0, 2000.0] {
        let gx = x + ((f.log2() - lo) / (hi - lo) * w as f32) as i32;
        let mut gy = y + 2;
        while gy < y + h - 2 {
            fill(fb, gx, gy, 1, 2, FAINT);
            gy += 5;
        }
        if !compact {
            let label = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f:.0}") };
            text_center(fb, &label, gx, y + h + 13, &SPLEEN_6X12, DIM);
        }
    }
    if !compact {
        text_right(fb, "Hz", x + w, y + h + 13, &SPLEEN_6X12, DIM);
    }
    // The output itself, as a trace across the panel.
    let mid = y + h / 2;
    let amp = (h / 2 - 6) as f32;
    let mut last: Option<Point> = None;
    for (i, s) in v.scope.iter().enumerate() {
        let pt = Point::new(x + 1 + (i as i32 * (w - 3)) / (SCOPE as i32 - 1), mid - (s.clamp(-1.0, 1.0) * amp) as i32);
        if let Some(prev) = last {
            Line::new(prev, pt).into_styled(PrimitiveStyle::with_stroke(c(mix(PANEL, accent, 0.55)), 1)).draw(fb).ok();
        }
        last = Some(pt);
    }
    for g in v.grains() {
        let f = g.speed.abs().max(20.0);
        let gx = x + ((f.log2() - lo) / (hi - lo) * w as f32).clamp(6.0, (w - 7) as f32) as i32;
        let gy = y + h / 2 - ((g.age * std::f32::consts::TAU).sin() * (h / 2 - 14) as f32) as i32;
        let size = if compact { 3.0 + 5.0 * g.env } else { 4.0 + 11.0 * g.env };
        dot(fb, gx, gy, (size * 2.3) as u32, mix(PANEL, accent, 0.25 * g.env + 0.06));
        dot(fb, gx, gy, size as u32, mix(accent, INK, 0.2 + 0.6 * g.env));
    }
    let label = "SYNTH";
    let lw = text_width(label, &SPLEEN_6X12) + 10;
    fill(fb, x + w - lw, y, lw, 14, accent);
    text(fb, label, x + w - lw + 5, y + 11, &SPLEEN_6X12, BG);
    frame(fb, x, y, w, h, FAINT);
}

/// The play view's lower half: the knob pair and its neighbours as dials,
/// and the pads.
fn draw_controls(fb: &mut FrameBuffer, col: &crate::app::PlayColumn, accent: Rgb, y: i32) {
    for (i, d) in col.dials.iter().enumerate().take(4) {
        let cx = 54 + i as i32 * 96;
        let cy = y + 36;
        let diameter = 56u32;
        let top_left = Point::new(cx - diameter as i32 / 2, cy - diameter as i32 / 2);
        let track = PrimitiveStyleBuilder::new().stroke_color(c(FAINT)).stroke_width(5).build();
        ArcShape::new(top_left, diameter, Angle::from_degrees(135.0), Angle::from_degrees(270.0)).into_styled(track).draw(fb).ok();
        let lit = if d.knob > 0 { accent } else { mix(DIM, accent, 0.3) };
        let sweep = 270.0 * d.norm.clamp(0.0, 1.0);
        if sweep > 0.5 {
            let value = PrimitiveStyleBuilder::new().stroke_color(c(lit)).stroke_width(5).build();
            ArcShape::new(top_left, diameter, Angle::from_degrees(135.0), Angle::from_degrees(sweep)).into_styled(value).draw(fb).ok();
        }
        // The pointer, riding the end of the arc.
        let a = (135.0f32 + sweep).to_radians();
        let r = diameter as f32 / 2.0 - 3.0;
        dot(fb, cx + (a.cos() * r).round() as i32, cy + (a.sin() * r).round() as i32, 9, INK);
        if d.knob > 0 {
            dot(fb, cx, cy, 17, lit);
            text_center(fb, &d.knob.to_string(), cx + 1, cy + 4, &SPLEEN_6X12, BG);
        }
        text_center(fb, &clip(&d.label.to_uppercase(), 15), cx, cy + 44, &SPLEEN_6X12, if d.knob > 0 { INK } else { DIM });
        text_center(fb, &clip(&d.value, 15), cx, cy + 57, &SPLEEN_6X12, mix(DIM, accent, 0.5));
    }

    let px = 428;
    let chip = format!("PADS: {}", col.layer);
    pill(fb, px, y - 3, text_width(&chip, &SPLEEN_6X12) + 16, 15, accent);
    text(fb, &chip, px + 8, y + 8, &SPLEEN_6X12, BG);
    text_right(fb, "R1 MENU", 624, y + 8, &SPLEEN_6X12, DIM);
    let cw = 46;
    let ch = 20;
    for i in 0..16usize {
        let cx = px + (i % 4) as i32 * (cw + 4);
        let cy = y + 17 + (i / 4) as i32 * (ch + 4);
        let st = col.pad_state.get(i).copied().unwrap_or(0);
        let (bg, ink) = match st {
            4 => (INK, BG),
            3 => (accent, BG),
            1 | 2 => (FAINT, INK),
            _ => (PANEL, DIM),
        };
        fill(fb, cx, cy, cw, ch, bg);
        let label = col.pad_labels.get(i).cloned().unwrap_or_default();
        text_center(fb, &clip(&label, 7), cx + cw / 2, cy + 14, &SPLEEN_6X12, ink);
    }

    let line = if !col.status.is_empty() { &col.status } else { &col.line };
    text(fb, &clip(line, 48), 16, y + 112, &SPLEEN_8X16, mix(INK, accent, 0.35));
}

// --- Audio ---------------------------------------------------------------

struct BeadsProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    engine: Engine,
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
}

/// How long the seed input counts as patched after its last high.
const SEED_PATCHED_SECONDS: f32 = 4.0;

/// Copies one picked source into `dest` (silence if none); true if one is
/// picked, i.e. a cable is in that jack.
fn read_source(audio_bus: &AudioBus, idx: usize, dest: &mut [f32]) -> bool {
    dest.fill(0.0);
    if idx == crate::audio_bus::NO_SOURCE {
        return false;
    }
    if let Some(buf) = audio_bus.get(idx) {
        if let Ok(b) = buf.lock() {
            if !b.is_empty() {
                for (n, d) in dest.iter_mut().enumerate() {
                    *d = b[n % b.len()];
                }
            }
        }
    }
    true
}

impl AudioProcessor for BeadsProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let channels = channels.max(1);
        let frames = buffer.len() / channels;
        for v in [&mut self.in_l, &mut self.in_r, &mut self.out_l, &mut self.out_r] {
            if v.len() < frames {
                v.resize(frames, 0.0);
            }
        }
        let p = &self.params;
        let has_left = read_source(&self.audio_bus, p.source_l.load(Ordering::Relaxed), &mut self.in_l[..frames]);
        let has_right = read_source(&self.audio_bus, p.source_r.load(Ordering::Relaxed), &mut self.in_r[..frames]);

        let seed_high = p.ext_seed.get() > 0.5;
        let quiet = if seed_high { 0 } else { p.seed_quiet_frames.load(Ordering::Relaxed).saturating_add(frames as u64) };
        p.seed_quiet_frames.store(quiet, Ordering::Relaxed);
        let seed_patched = (quiet as f32) < SEED_PATCHED_SECONDS * sample_rate;

        let controls = Controls {
            pots: std::array::from_fn(|i| p.pot(i)),
            time_offset: p.ext_time.get(),
            size_offset: p.ext_size.get(),
            shape_offset: p.ext_shape.get(),
            pitch_offset: p.ext_pitch.get(),
            density_cv: p.ext_density.get(),
            mix_offset: [p.ext_feedback.get(), p.ext_dry_wet.get(), p.ext_reverb.get()],
            media: p.media() as i32,
            gain: p.gain.load(Ordering::Relaxed).min(255) as i32,
            has_left: has_left as i32,
            has_right: has_right as i32,
            seed_button: p.seed.load(Ordering::Relaxed) as i32,
            seed_patched: seed_patched as i32,
            seed_gate: seed_high as i32,
            freeze_button: p.freeze.load(Ordering::Relaxed) as i32,
            freeze_gate: (p.ext_freeze.get() > 0.5) as i32,
        };
        // The bridge's GainControl does the normalling (an empty jack copies
        // the other), as the firmware's does.
        unsafe {
            beads_render(
                self.engine.handle,
                &controls,
                self.in_l.as_ptr(),
                self.in_r.as_ptr(),
                self.out_l.as_mut_ptr(),
                self.out_r.as_mut_ptr(),
                frames as c_int,
                sample_rate,
            );
        }

        let level = (p.mix_level.get() + p.ext_mix_level.get()).clamp(0.0, 2.0);
        let mut peak = 0.0f32;
        for (n, out) in buffer.chunks_mut(channels).enumerate() {
            let l = self.out_l[n];
            let r = self.out_r[n];
            peak = peak.max(l.abs()).max(r.abs());
            if channels >= 2 {
                out[0] = l * level;
                out[1] = r * level;
                for ch in out.iter_mut().skip(2) {
                    *ch = 0.0;
                }
            } else {
                out[0] = (l + r) * 0.5 * level;
            }
        }
        p.out_peak.set(peak);
        if let Ok(mut bus) = p.bus_out.lock() {
            bus.clear();
            bus.extend(self.out_l[..frames].iter().zip(&self.out_r[..frames]).map(|(l, r)| (l + r) * 0.5));
        }
        if let Ok(mut v) = p.view.try_lock() {
            unsafe { beads_view(self.engine.handle, &mut *v) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::play_kit as kit;

    fn new_app() -> (BeadsApp, Arc<AudioBus>) {
        let audio_bus = Arc::new(AudioBus::new());
        let app = BeadsApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::clone(&audio_bus), Arc::new(MixerBus::new()));
        (app, audio_bus)
    }

    /// A 220 Hz sine on the bus as "Source" (index 1: Beads registers its
    /// own output first), picked as IN L.
    fn with_sine(app: &BeadsApp, audio_bus: &AudioBus) -> Arc<Mutex<Vec<f32>>> {
        let source = audio_bus.register("Source");
        app.params.source_l.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = (0..480).map(|i| 0.5 * (i as f32 * std::f32::consts::TAU * 200.0 / 48000.0).sin()).collect();
        source
    }

    fn run(processor: &mut Box<dyn AudioProcessor>, seconds: f32) -> (Vec<f32>, f32) {
        let mut buffer = vec![0.0f32; 480 * 2];
        let mut peak = 0.0f32;
        for _ in 0..(seconds * 100.0) as usize {
            processor.process(&mut buffer, 2, 48000.0);
            peak = buffer.iter().fold(peak, |m, s| m.max(s.abs()));
        }
        (buffer, peak)
    }

    #[test]
    fn the_rust_and_c_layouts_agree() {
        assert_eq!(unsafe { beads_controls_size() } as usize, std::mem::size_of::<Controls>());
        assert_eq!(unsafe { beads_view_size() } as usize, std::mem::size_of::<View>());
    }

    #[test]
    fn a_source_comes_back_as_grains() {
        let (mut app, bus) = new_app();
        with_sine(&app, &bus);
        app.params.pots[POT_DRY_WET].set(1.0); // wet only: anything heard is grains
        app.params.pots[POT_TIME].set(0.05);
        let mut p = app.audio_processor().unwrap();
        let (_, peak) = run(&mut p, 2.0);
        assert!(peak > 0.05, "grains should be audible, peak {peak}");
        let v = app.view();
        assert_eq!(v.mode, 0);
        assert_eq!(v.mono, 1, "one source: mono, as with one cable in IN L");
        assert!((v.buffer_seconds - 8.0).abs() < 0.1, "digital mono holds 8 s, got {}", v.buffer_seconds);
        assert!(v.overview.iter().take(32).any(|&x| x > 0.1), "the newest part of the buffer holds the sine");
    }

    #[test]
    fn grains_are_panned_so_the_output_is_stereo() {
        let (mut app, bus) = new_app();
        with_sine(&app, &bus);
        app.params.pots[POT_DRY_WET].set(1.0);
        app.params.pots[POT_TIME].set(0.05);
        app.params.pots[POT_TIME_AMT].set(1.0); // randomness widens the pan spread
        app.params.pots[POT_DENSITY].set(0.95);
        let mut p = app.audio_processor().unwrap();
        let mut differs = false;
        let mut buffer = vec![0.0f32; 960];
        for _ in 0..200 {
            p.process(&mut buffer, 2, 48000.0);
            differs |= buffer.chunks(2).any(|f| (f[0] - f[1]).abs() > 0.01);
        }
        assert!(differs, "left and right should differ");
    }

    #[test]
    fn freezing_an_empty_buffer_keeps_the_wet_signal_silent() {
        let (mut app, bus) = new_app();
        with_sine(&app, &bus);
        app.params.freeze.store(true, Ordering::Relaxed);
        app.params.pots[POT_DRY_WET].set(1.0);
        app.params.pots[POT_REVERB].set(0.0);
        let mut p = app.audio_processor().unwrap();
        let (_, peak) = run(&mut p, 1.5);
        assert!(peak < 1e-3, "nothing was recorded, so nothing plays: {peak}");
        assert_eq!(app.view().frozen, 1);
    }

    #[test]
    fn size_fully_clockwise_is_the_delay() {
        let (mut app, bus) = new_app();
        with_sine(&app, &bus);
        app.params.pots[POT_SIZE].set(1.0);
        app.params.pots[POT_DRY_WET].set(1.0);
        let mut p = app.audio_processor().unwrap();
        let (buffer, peak) = run(&mut p, 3.0);
        assert!(buffer.iter().all(|s| s.is_finite()));
        assert!(peak > 0.05, "the delay should be audible: {peak}");
        assert_eq!(app.view().mode, 1, "delay mode after the 1.5 s transition");
    }

    #[test]
    fn the_tape_media_run_at_24_khz_with_longer_buffers() {
        let (mut app, bus) = new_app();
        with_sine(&app, &bus);
        app.params.media.store(0, Ordering::Relaxed);
        app.params.pots[POT_TIME].set(0.02);
        app.params.pots[POT_DRY_WET].set(1.0);
        let mut p = app.audio_processor().unwrap();
        let (_, peak) = run(&mut p, 2.0);
        let v = app.view();
        assert_eq!(v.codec_rate, 24000.0);
        assert!((v.buffer_seconds - 32.0).abs() < 0.1, "dirty tape, mono: 32 s, got {}", v.buffer_seconds);
        assert!(peak > 0.05, "and it plays through the resamplers: {peak}");
    }

    #[test]
    fn with_nothing_patched_the_synth_starts_after_ten_seconds() {
        let (mut app, _bus) = new_app();
        app.params.pots[POT_DENSITY].set(0.9);
        let mut p = app.audio_processor().unwrap();
        let (_, peak) = run(&mut p, 5.0);
        assert_eq!(peak, 0.0, "silent while counting down");
        assert!(app.view().synth_countdown > 4.0);
        let (_, peak) = run(&mut p, 7.0);
        assert_eq!(app.view().mode, 2);
        assert!(peak > 0.01, "the wavetable synth plays: {peak}");
    }

    #[test]
    fn high_feedback_stays_bounded_and_finite() {
        let (mut app, bus) = new_app();
        with_sine(&app, &bus);
        app.params.pots[POT_FEEDBACK].set(1.0);
        app.params.pots[POT_REVERB].set(1.0);
        app.params.pots[POT_DRY_WET].set(1.0);
        app.params.pots[POT_DENSITY].set(1.0);
        let mut p = app.audio_processor().unwrap();
        let (buffer, peak) = run(&mut p, 4.0);
        assert!(buffer.iter().all(|s| s.is_finite()));
        assert!(peak <= 1.0, "the firmware's limiter holds it: {peak}");
    }

    #[test]
    fn republishes_its_output_on_the_audio_bus() {
        let (mut app, bus) = new_app();
        with_sine(&app, &bus);
        let mut p = app.audio_processor().unwrap();
        run(&mut p, 0.5);
        assert_eq!(app.params.bus_out.lock().unwrap().len(), 480);
    }

    #[test]
    fn the_seed_button_follows_the_firmware() {
        let mut s = SeedButton::new();
        assert_eq!(s.state(), SeedState::Locked, "boots free-running");
        let frame = Duration::from_millis(16);
        s.update(true, frame);
        assert_eq!(s.state(), SeedState::On, "a press unlocks and taps");
        s.update(false, frame);
        assert_eq!(s.state(), SeedState::Off);
        s.update(true, frame);
        for _ in 0..100 {
            s.update(true, frame);
        }
        assert_eq!(s.state(), SeedState::Locked, "1.5 s held locks it again");
        s.update(false, frame);
        assert_eq!(s.state(), SeedState::Locked, "and letting go keeps it locked");
    }

    #[test]
    fn the_knob_laws_match_the_firmware() {
        assert_eq!(pitch_semitones(0.5), 0.0);
        assert!((pitch_semitones(11.0 / 12.0) - 12.0).abs() < 0.01);
        assert!((pitch_semitones(1.0 / 12.0) + 12.0).abs() < 0.01);
        assert_eq!(attenurandomizer(0.5), 0.0);
        assert_eq!(attenurandomizer(0.52), 0.0, "centre detent");
        assert!(attenurandomizer(1.0) > 0.9);
        let (s, reverse) = grain_seconds(0.2);
        assert!(reverse && s > 0.0);
    }

    #[test]
    fn opens_playable_and_the_pads_are_seed() {
        let (mut app, _bus) = new_app();
        assert!(app.play_column().is_some(), "play view first");
        let time = app.params.pot(POT_TIME);
        app.tick(&Input { knob1: 3, ..Default::default() });
        assert!(app.params.pot(POT_TIME) > time, "knob 1 is Time");
        app.tick(&Input { grid: std::array::from_fn(|i| i == 5), ..Default::default() });
        assert_eq!(app.params.seed.load(Ordering::Relaxed), SeedState::On as u32, "a pad is the SEED button");
        app.tick(&Input::default());
        app.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_ne!(app.params.media(), DEFAULT_MEDIA as usize, "D-pad up/down is QUALITY");
        app.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(app.play_column().is_none(), "R1 opens the full menu");
    }

    #[test]
    fn throws_freeze_and_spring_back_without_pressing_seed() {
        let (mut app, _bus) = new_app();
        app.toggle_grid_mode();
        assert_eq!(app.grid_mode_label(), Some("THROWS"));
        let freeze = kit::rank_pad(0);
        app.tick(&Input { grid: std::array::from_fn(|i| i == freeze), ..Default::default() });
        assert!(app.params.freeze.load(Ordering::Relaxed), "FREEZE held");
        assert_eq!(app.params.seed.load(Ordering::Relaxed), SeedState::Locked as u32, "kit layers never press SEED");
        app.tick(&Input::default());
        assert!(!app.params.freeze.load(Ordering::Relaxed));
    }

    /// Renders the play view, the menu and synth mode to PNGs in the folder
    /// named by PORTAMAX_BEADS_SHOT.
    #[test]
    #[ignore = "writes screenshots to the folder in PORTAMAX_BEADS_SHOT"]
    fn screenshot() {
        let Ok(dir) = std::env::var("PORTAMAX_BEADS_SHOT") else { return };
        let shoot = |app: &mut BeadsApp, name: &str| {
            let mut fb = FrameBuffer::new();
            app.draw(&mut fb);
            let file = std::fs::File::create(std::path::Path::new(&dir).join(name)).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), WIDTH as u32, HEIGHT as u32);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let data: Vec<u8> = fb.buffer().iter().flat_map(|px| [(px >> 16) as u8, (px >> 8) as u8, *px as u8]).collect();
            enc.write_header().unwrap().write_image_data(&data).unwrap();
        };
        let (mut app, bus) = new_app();
        let source = bus.register("Source");
        app.params.source_l.store(1, Ordering::Relaxed);
        app.params.pots[POT_TIME].set(0.3);
        app.params.pots[POT_TIME_AMT].set(0.85);
        app.params.pots[POT_PITCH_AMT].set(0.75);
        app.params.pots[POT_SIZE_AMT].set(0.8);
        app.params.pots[POT_DENSITY].set(0.95);
        app.params.pots[POT_SIZE].set(0.5);
        let mut p = app.audio_processor().unwrap();
        // A plucked phrase, so the buffer has shape.
        let mut phase = 0.0f32;
        let mut buffer = vec![0.0f32; 960];
        let mut play = |p: &mut Box<dyn AudioProcessor>, blocks: usize, buffer: &mut Vec<f32>, phase: &mut f32, start: usize| {
            for block in start..start + blocks {
                let note = [220.0, 330.0, 247.0, 392.0][(block / 25) % 4];
                let env_start = (block % 25) as f32 / 25.0;
                *source.lock().unwrap() = (0..480)
                    .map(|i| {
                        *phase += note / 48000.0;
                        let e = (-(env_start * 0.25 + i as f32 / 48000.0) * 16.0).exp();
                        0.7 * e * (*phase * std::f32::consts::TAU).sin()
                    })
                    .collect();
                p.process(buffer, 2, 48000.0);
            }
        };
        play(&mut p, 700, &mut buffer, &mut phase, 0);
        app.tick(&Input::default());
        shoot(&mut app, "beads_play.png");
        app.params.media.store(1, Ordering::Relaxed);
        play(&mut p, 900, &mut buffer, &mut phase, 700);
        app.params.freeze.store(true, Ordering::Relaxed);
        play(&mut p, 40, &mut buffer, &mut phase, 1600);
        shoot(&mut app, "beads_frozen_tape.png");
        app.params.freeze.store(false, Ordering::Relaxed);
        app.params.media.store(2, Ordering::Relaxed);
        app.params.pots[POT_SIZE].set(1.0);
        play(&mut p, 400, &mut buffer, &mut phase, 1640);
        shoot(&mut app, "beads_delay.png");
        app.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        app.expanded[0] = true;
        app.expanded[1] = true;
        shoot(&mut app, "beads_menu.png");
        app.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        app.params.source_l.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed);
        app.params.pots[POT_SIZE].set(0.6);
        app.params.media.store(0, Ordering::Relaxed);
        run(&mut p, 3.0);
        shoot(&mut app, "beads_countdown.png");
        run(&mut p, 9.0);
        shoot(&mut app, "beads_synth.png");
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(BeadsApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}
