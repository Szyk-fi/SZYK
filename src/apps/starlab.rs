//! A clone of Strymon StarLab: a Eurorack "time-warped reverberator"
//! built around a 4-comb/2-allpass Schroeder-style reverb tank, plus a
//! monophonic Karplus-Strong string-synthesis voice sharing the same
//! DELAY/KARPLUS section. Feature set follows the real module's
//! manual and quick-start guide (strymon.net/manuals/StarLab_*) as
//! closely as this sim's architecture allows. Note: despite this
//! sim's task briefing calling it a guitar pedal, the real StarLab is
//! a Eurorack module -- it taps another app's output the same way
//! Beads/Clouds do (see audio_bus.rs).
//!
//! - Three TEXTURE reverb structures (Sparse/Dense/Diffuse), each a
//!   real change to comb/allpass topology, not just a label: Sparse
//!   uses fewer combs plus a sample-and-hold decimator for a
//!   granular/staccato character; Dense is the fast 4-comb/1-allpass
//!   plate; Diffuse runs the allpass chain twice over a slew-smoothed
//!   input for a slow-building wash.
//! - SIZE/PITCH: a single knob that both rescales the comb/allpass
//!   delay lengths (0.5x..2x, "size") and pitch-shifts the
//!   regenerating tail (+-1 octave, "pitch") via a real dual-tap
//!   crossfaded delay-line pitch shifter -- the same building block
//!   used for SHIMMER below, since the real module's "variable
//!   processing rate" and a delay-based pitch shifter are the same
//!   underlying trick.
//! - INFINITE: freezes new audio from entering the reverb tank while
//!   the existing regenerating content keeps cycling -- a real
//!   freeze, not a mute.
//! - HARMONICS: SHIMMER (amount + interval, regenerative-in-tank or
//!   input-side, via the pitch shifter above) and GLIMMER (a real
//!   harmonic exciter: isolates a high- or low-band and adds back its
//!   soft-clipped residual).
//! - FILTER: LOW DAMP/HIGH DAMP shape the comb feedback path's
//!   frequency content; engaging LOW PASS repurposes HIGH DAMP into a
//!   real 24dB/oct resonant state-variable lowpass at the output,
//!   with its own resonance control.
//! - DELAY/KARPLUS: a shared FEEDBACK/DELAY-TUNE section that's
//!   either a real feedback delay line (with pad-tap tempo, standing
//!   in for the real module's TAP/TRIG clock input) or a genuine
//!   Karplus-Strong plucked/bowed string (pad press = pluck, pad held
//!   = bow), matching the real module's "monophonic string
//!   synthesizer" -- last-pressed pad wins the note, reusing this
//!   sim's existing pad-rank/note mapping as a stand-in for 1V/oct
//!   SIZE/PITCH CV tuning.
//! - LFO: full SPEED/SHAPE(6 waveforms incl. random S&H and an
//!   input-tracking envelope)/DEPTH modulating one of DELAY, PITCH,
//!   or FILTER, matching the real module's LFO section.
//! - Independent DRY and WET level controls (the real module's own
//!   split, replacing a single dry/wet crossfade), plus CLEAR (an
//!   instant reverb-buffer flush).
//!
//! Deliberately not implemented: the four onboard FAVORITE presets
//! and preset spillover (this sim has no generic per-app preset
//! storage); the real per-parameter CV inputs and their individual
//! voltage-offset behaviors (this sim has no generic per-app CV
//! patching beyond the pad grid, same limitation Beads documents);
//! the "hold ECHO ON + turn DELAY/TUNE" and "hold REGEN/HIGH
//! BAND/LOW PASS + turn knob" secondary-function gestures -- each of
//! those secondary functions (Karplus mode select, shimmer interval,
//! glimmer band, lowpass resonance) is instead given its own regular
//! menu leaf, the same workaround Beads uses for its
//! attenurandomizers; the 2-second-hold PITCH QUANTIZE scale-lock
//! mode; the front-panel LED level/mode indicators; and the analog
//! dry-path/ADC-DAC spec bullets, which describe hardware, not DSP.

use crate::app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes, Throw};
use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::util::{accelerate, note_name, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NUM_COMBS: usize = 4;
const NUM_ALLPASS: usize = 2;
/// Prime-ish, mutually-coprime-ish delay lengths (in ms at 48kHz) --
/// the classic Schroeder-reverb trick to avoid the comb filters'
/// resonances lining up into an audible metallic ring.
const COMB_DELAYS_MS: [f32; NUM_COMBS] = [29.7, 37.1, 41.1, 43.7];
const ALLPASS_DELAYS_MS: [f32; NUM_ALLPASS] = [5.0, 1.7];
/// Headroom multiplier on top of SIZE's 2x maximum, so the comb/
/// allpass buffers are always big enough for the scaled delay length.
const SIZE_HEADROOM: f32 = 2.3;
const SHIMMER_BUF_SECONDS: f32 = 0.09;
const SIZE_PITCH_BUF_SECONDS: f32 = 0.09;
const DELAY_MAX_MS: f32 = 1550.0;
/// Karplus-Strong tuning range: 4 octaves up from this base frequency.
const KARPLUS_BASE_HZ: f32 = 55.0;
const KARPLUS_MIN_HZ: f32 = 40.0;
const PLUCK_SAMPLES: u32 = 160; // ~3.3ms noise burst at 48kHz

const TEXTURE_NAMES: [&str; 3] = ["Sparse", "Dense", "Diffuse"];
const LFO_TARGET_NAMES: [&str; 3] = ["Delay", "Pitch", "Filter"];
const LFO_SHAPE_NAMES: [&str; 6] = ["Triangle", "Square", "Ramp", "Saw", "Random", "Envelope"];

/// Real, live-data-driven visualization state for the Slint panel --
/// no fabricated animation: every field mirrors DSP state already
/// computed once per audio block by `StarlabProcessor::process` (see
/// `Params::waveform_snapshot`/`lfo_phase`/`lfo_value`/`tank_energy`).
pub(crate) struct StarlabVisual {
    /// A real oscilloscope of this block's processed (dry+wet mixed)
    /// output, as connected line-segment geometry (see
    /// `crate::app::polyline_segments`) -- same technique as Beads'
    /// grain-cloud waveform and Black Hole's oscilloscope.
    pub waveform: crate::app::CurveSegments,
    /// Which TEXTURE structure is active, for a badge.
    pub texture_name: String,
    /// A fixed index (0..3) for UI color-coding, mirroring Black
    /// Hole's `category_index`.
    pub texture_index: usize,
    /// INFINITE freeze indicator.
    pub infinite: bool,
    /// Whether the shared DELAY/KARPLUS section is currently a
    /// Karplus-Strong string voice (vs. a plain feedback delay).
    pub karplus_mode: bool,
    /// The real LFO's phase (0..1, one full cycle) at the end of this
    /// block -- drives a small moving indicator (e.g. an orbiting
    /// dot) reflecting the actual modulation source rather than a
    /// fabricated animation.
    pub lfo_phase: f32,
    /// The real LFO's output value (post-DEPTH, -1..1) at the end of
    /// this block.
    pub lfo_value: f32,
    /// A smoothed 0..1 read of the real energy currently
    /// regenerating in the reverb tank (an EMA of the comb section's
    /// output level, computed once per block) -- genuinely reflects
    /// DECAY/INFINITE/CLEAR/TEXTURE: rises while the tank is fed and
    /// its decaying content is loud, falls toward 0 after CLEAR or
    /// with DECAY low.
    pub tank_energy: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Source,
    Texture,
    InputGain,
    SizePitch,
    Decay,
    Infinite,
    LowDamp,
    HighDamp,
    LowPassMode,
    LowPassResonance,
    ShimmerAmount,
    ShimmerInterval,
    ShimmerRegen,
    GlimmerAmount,
    GlimmerHighBand,
    KarplusMode,
    Octave,
    Feedback,
    EchoOn,
    DelayTune,
    LfoTarget,
    LfoSpeed,
    LfoShape,
    LfoDepth,
    Dry,
    Wet,
    Clear,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 6;

struct Params {
    source: AtomicUsize,
    texture: AtomicU32,
    input_gain: AtomicF32,
    size_pitch: AtomicF32, // 0..1, 0.5 = neutral
    decay: AtomicF32,
    infinite: AtomicBool,
    low_damp: AtomicF32,
    high_damp: AtomicF32,
    low_pass_mode: AtomicBool,
    low_pass_resonance: AtomicF32,
    shimmer_amount: AtomicF32,
    shimmer_interval: AtomicF32, // 0..1, 0.5 = neutral (0 semitones)
    shimmer_regen: AtomicBool,
    glimmer_amount: AtomicF32,
    glimmer_high_band: AtomicBool,
    karplus_mode: AtomicBool,
    /// Transposes every pad's note by this many octaves -- same
    /// pattern/range as Plaits' own Octave.
    octave: AtomicI32,
    feedback: AtomicF32,
    echo_on: AtomicBool,
    delay_tune: AtomicF32,
    lfo_target: AtomicU32,
    lfo_speed: AtomicF32,
    lfo_shape: AtomicU32,
    lfo_depth: AtomicF32,
    dry: AtomicF32,
    wet: AtomicF32,
    /// Incremented every time CLEAR is triggered; the audio thread
    /// diffs against its own last-seen value to detect the edge.
    clear_trigger: AtomicU32,
    /// Set by the audio thread when a pad-tap interval was learned in
    /// Delay mode (stands in for the real module's TAP/TRIG clock
    /// input); cleared whenever DELAY/TUNE is turned by hand.
    tap_active: AtomicBool,
    tap_ms: AtomicF32,
    held: Mutex<[bool; 16]>,
    /// A downsampled snapshot of this block's real processed (dry+wet
    /// mixed) output, refreshed once per audio block, for the Slint
    /// oscilloscope -- real audio, not fabricated (same technique
    /// Black Hole uses for its own oscilloscope).
    waveform_snapshot: Mutex<Vec<f32>>,
    /// The real LFO's phase (0..1) at the end of the most recent
    /// block -- mirrors `StarlabProcessor::lfo_phase`.
    lfo_phase: AtomicF32,
    /// The real LFO's output value (post-DEPTH, -1..1) at the end of
    /// the most recent block.
    lfo_value: AtomicF32,
    /// A smoothed 0..1 read of the real energy regenerating in the
    /// reverb tank -- see `StarlabVisual::tank_energy`.
    tank_energy: AtomicF32,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Starlab", modbus);
        Self {
            source: AtomicUsize::new(crate::audio_bus::NO_SOURCE),
            texture: AtomicU32::new(1), // Dense
            input_gain: AtomicF32::new(0.5),
            size_pitch: AtomicF32::new(0.5),
            decay: AtomicF32::new(0.7),
            infinite: AtomicBool::new(false),
            low_damp: AtomicF32::new(0.0),
            high_damp: AtomicF32::new(0.3),
            low_pass_mode: AtomicBool::new(false),
            low_pass_resonance: AtomicF32::new(0.3),
            shimmer_amount: AtomicF32::new(0.0),
            shimmer_interval: AtomicF32::new(1.0), // +1 octave, matches real default full-CW-ish shimmer character
            shimmer_regen: AtomicBool::new(true),
            glimmer_amount: AtomicF32::new(0.0),
            glimmer_high_band: AtomicBool::new(true),
            karplus_mode: AtomicBool::new(false),
            octave: AtomicI32::new(0),
            feedback: AtomicF32::new(0.3),
            echo_on: AtomicBool::new(false),
            delay_tune: AtomicF32::new(0.3),
            lfo_target: AtomicU32::new(0),
            lfo_speed: AtomicF32::new(0.3),
            lfo_shape: AtomicU32::new(0),
            lfo_depth: AtomicF32::new(0.0),
            dry: AtomicF32::new(0.6),
            wet: AtomicF32::new(0.6),
            clear_trigger: AtomicU32::new(0),
            tap_active: AtomicBool::new(false),
            tap_ms: AtomicF32::new(300.0),
            held: Mutex::new([false; 16]),
            waveform_snapshot: Mutex::new(Vec::new()),
            lfo_phase: AtomicF32::new(0.0),
            lfo_value: AtomicF32::new(0.0),
            tank_energy: AtomicF32::new(0.0),
            bus_out: audio_bus.register("Starlab"),
            mix_level,
            ext_mix_level,
        }
    }
}

fn pad_rank(physical_index: i32) -> i32 {
    let row = physical_index / 4;
    let col = physical_index % 4;
    (3 - row) * 4 + col
}

fn note_for(rank: i32, octave: i32) -> i32 {
    (48 + rank + octave * 12).clamp(0, 127) // C3 up, chromatic -- a pad -- doesn't need scales
}

pub struct StarlabApp {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
    /// The shared play view (play_kit.rs).
    kit: PlayKit,
}

/// The play view's controls, most important first: the tank's decay and
/// SIZE/PITCH warp on the first knob pair (the two the real module is
/// played with), then the wet level and shimmer, the shared DELAY/TUNE +
/// FEEDBACK section, tone, LFO, and the switches on the upper pads.
const CONTROLS: [(Selection, &str); 16] = [
    (Selection::Decay, "Decay"),
    (Selection::SizePitch, "Size/Pitch"),
    (Selection::Wet, "Wet"),
    (Selection::ShimmerAmount, "Shimmer"),
    (Selection::DelayTune, "Delay/Tune"),
    (Selection::Feedback, "Feedback"),
    (Selection::HighDamp, "High Damp"),
    (Selection::GlimmerAmount, "Glimmer"),
    (Selection::LfoDepth, "LFO Depth"),
    (Selection::LfoSpeed, "LFO Speed"),
    (Selection::Dry, "Dry"),
    (Selection::EchoOn, "Echo On"),
    (Selection::Texture, "Texture"),
    (Selection::Infinite, "Infinite"),
    (Selection::KarplusMode, "Delay/Karplus"),
    (Selection::Octave, "Octave"),
];
const C_DECAY: usize = 0;
const C_SIZE_PITCH: usize = 1;
const C_SHIMMER: usize = 3;
const C_FEEDBACK: usize = 5;
const C_HIGH_DAMP: usize = 6;
const C_GLIMMER: usize = 7;
const C_DRY: usize = 10;
const C_TEXTURE: usize = 12;
const C_INFINITE: usize = 13;
/// Octave range the play view's dial and moments use; the menu's own
/// Octave row is unbounded (note_for clamps to MIDI range), so this only
/// bounds the dial's position.
const OCTAVE_SPAN: i32 = 4;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "starlab",
        // The pads keep doing what they always did (pluck/bow the string
        // in Karplus mode, tap the delay time in Delay mode); Throws
        // because Starlab is first of all an effect on another app.
        layers: vec![Layer::Native(0, "PLUCK"), Layer::Controls, Layer::Moments, Layer::Throws],
        hero: vec![[C_DECAY, C_SIZE_PITCH], [2, C_SHIMMER], [4, C_FEEDBACK], [C_HIGH_DAMP, C_GLIMMER]],
        // TEXTURE changes the tank's actual topology, so it's the choice
        // worth flicking through while playing.
        browse: Some(C_TEXTURE),
        // Stick X warps SIZE/PITCH (the module's signature tape-warp
        // move), Y pours in shimmer; hands stretch the tail and add
        // glimmer sparkle.
        routes: Routes { stick_x: Some(C_SIZE_PITCH), stick_y: Some(C_SHIMMER), hand_l: Some(C_DECAY), hand_r: Some(C_GLIMMER) },
        throws: vec![
            Throw { control: C_INFINITE, to: 1.0, label: "FREEZE" },
            Throw { control: C_DECAY, to: 1.0, label: "DECAY MAX" },
            Throw { control: C_SHIMMER, to: 1.0, label: "SHIMMER" },
            // SIZE/PITCH: full CW = 2x size, -1 oct; full CCW = 0.5x, +1 oct.
            Throw { control: C_SIZE_PITCH, to: 1.0, label: "OCT DOWN" },
            Throw { control: C_SIZE_PITCH, to: 0.0, label: "OCT UP" },
            Throw { control: C_FEEDBACK, to: 1.0, label: "FEEDBACK" },
            Throw { control: C_HIGH_DAMP, to: 1.0, label: "DARK" },
            Throw { control: C_DRY, to: 0.0, label: "WET ONLY" },
        ],
        midi_to_pads: true,
        own_expression: false,
    }
}

// --- Starlab's own palette: flat solid colors, not a
// device-wide theme -- Deep indigo night sky with a pale starlight-blue accent -- a reverb tail fading into starlight. ---

const STARLAB_BG: Rgb565 = Rgb565::new(1, 3, 4);
const STARLAB_TITLE: Rgb565 = Rgb565::new(26, 57, 31);
const STARLAB_ACCENT: Rgb565 = Rgb565::new(17, 47, 31);
const STARLAB_DIM: Rgb565 = Rgb565::new(9, 19, 13);
/// Unlit pad cells and dial tracks on the play column.
const STARLAB_FAINT: Rgb565 = Rgb565::new(3, 7, 9);

impl StarlabApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        Self {
            params: Arc::new(Params::new(&modbus, &audio_bus, &mixer_bus)),
            audio_bus,
            sensitivity,
            nav_speed,
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
            kit: PlayKit::new(kit_config(), !cfg!(test)),
        }
    }

    fn knob(&self, i: usize) -> Knob<'_> {
        let p = &self.params;
        // Ranges are exactly what `edit` clamps each one to.
        match CONTROLS[i % 16].0 {
            Selection::Decay => Knob::F(&p.decay, 0.0, 0.97),
            Selection::SizePitch => Knob::F(&p.size_pitch, 0.0, 1.0),
            Selection::Wet => Knob::F(&p.wet, 0.0, 1.0),
            Selection::ShimmerAmount => Knob::F(&p.shimmer_amount, 0.0, 1.0),
            Selection::DelayTune => Knob::F(&p.delay_tune, 0.0, 1.0),
            Selection::Feedback => Knob::F(&p.feedback, 0.0, 0.98),
            Selection::HighDamp => Knob::F(&p.high_damp, 0.0, 1.0),
            Selection::GlimmerAmount => Knob::F(&p.glimmer_amount, 0.0, 1.0),
            Selection::LfoDepth => Knob::F(&p.lfo_depth, 0.0, 1.0),
            Selection::LfoSpeed => Knob::F(&p.lfo_speed, 0.0, 1.0),
            Selection::Dry => Knob::F(&p.dry, 0.0, 1.0),
            Selection::EchoOn => Knob::B(&p.echo_on),
            Selection::Texture => Knob::U(&p.texture, 3),
            Selection::Infinite => Knob::B(&p.infinite),
            Selection::KarplusMode => Knob::B(&p.karplus_mode),
            Selection::Octave => Knob::I(&p.octave, -OCTAVE_SPAN, OCTAVE_SPAN),
            _ => Knob::None,
        }
    }

    fn karplus(&self) -> bool {
        self.params.karplus_mode.load(Ordering::Relaxed)
    }

    fn pad_note(&self, pad: usize) -> i32 {
        note_for(pad_rank(pad as i32), self.params.octave.load(Ordering::Relaxed))
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => vec![Selection::Source, Selection::InputGain],
            1 => vec![Selection::Texture, Selection::SizePitch, Selection::Decay, Selection::Infinite, Selection::Clear],
            2 => vec![Selection::ShimmerAmount, Selection::ShimmerInterval, Selection::ShimmerRegen, Selection::GlimmerAmount, Selection::GlimmerHighBand],
            3 => vec![Selection::LowDamp, Selection::HighDamp, Selection::LowPassMode, Selection::LowPassResonance],
            4 => vec![
                Selection::KarplusMode,
                Selection::Octave,
                Selection::Feedback,
                Selection::EchoOn,
                Selection::DelayTune,
                Selection::LfoTarget,
                Selection::LfoSpeed,
                Selection::LfoShape,
                Selection::LfoDepth,
            ],
            _ => vec![Selection::Dry, Selection::Wet],
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
            1 => "Reverb",
            2 => "Harmonics",
            3 => "Filter",
            4 => "Delay/String",
            _ => "Output",
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => format!("{}, {:.0}% gain", self.source_name(), self.params.input_gain.get() * 100.0),
            1 => format!(
                "{}, decay {:.0}%{}",
                TEXTURE_NAMES[self.params.texture.load(Ordering::Relaxed) as usize % 3],
                self.params.decay.get() * 100.0,
                if self.params.infinite.load(Ordering::Relaxed) { ", infinite" } else { "" }
            ),
            2 => format!("{:.0}% shim, {:.0}% glim", self.params.shimmer_amount.get() * 100.0, self.params.glimmer_amount.get() * 100.0),
            3 => format!(
                "{:.0}%lo/{:.0}%hi{}",
                self.params.low_damp.get() * 100.0,
                self.params.high_damp.get() * 100.0,
                if self.params.low_pass_mode.load(Ordering::Relaxed) { ", LP" } else { "" }
            ),
            4 => format!(
                "{}, LFO {:.2}Hz->{}",
                if self.params.karplus_mode.load(Ordering::Relaxed) { "Karplus" } else { "Delay" },
                lfo_speed_hz(self.params.lfo_speed.get()),
                LFO_TARGET_NAMES[self.params.lfo_target.load(Ordering::Relaxed) as usize % 3]
            ),
            _ => format!("{:.0}% dry, {:.0}% wet", self.params.dry.get() * 100.0, self.params.wet.get() * 100.0),
        }
    }

    fn source_name(&self) -> String {
        self.audio_bus.source_name(self.params.source.load(Ordering::Relaxed))
    }

    fn shimmer_interval_semitones(&self) -> f32 {
        shimmer_interval_semitones(self.params.shimmer_interval.get())
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => "Source".into(),
            Selection::Texture => "Texture".into(),
            Selection::InputGain => "Input Gain".into(),
            Selection::SizePitch => "Size/Pitch".into(),
            Selection::Decay => "Decay".into(),
            Selection::Infinite => "Infinite".into(),
            Selection::LowDamp => "Low Damp".into(),
            Selection::HighDamp => "High Damp".into(),
            Selection::LowPassMode => "Low Pass".into(),
            Selection::LowPassResonance => "LP Resonance".into(),
            Selection::ShimmerAmount => "Shimmer".into(),
            Selection::ShimmerInterval => "Shimmer Interval".into(),
            Selection::ShimmerRegen => "Shimmer Regen".into(),
            Selection::GlimmerAmount => "Glimmer".into(),
            Selection::GlimmerHighBand => "Glimmer Band".into(),
            Selection::KarplusMode => "Delay/Karplus".into(),
            Selection::Octave => "Octave".into(),
            Selection::Feedback => "Feedback".into(),
            Selection::EchoOn => "Echo On".into(),
            Selection::DelayTune => "Delay/Tune".into(),
            Selection::LfoTarget => "LFO Target".into(),
            Selection::LfoSpeed => "LFO Speed".into(),
            Selection::LfoShape => "LFO Shape".into(),
            Selection::LfoDepth => "LFO Depth".into(),
            Selection::Dry => "Dry".into(),
            Selection::Wet => "Wet".into(),
            Selection::Clear => "Clear".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Source => self.source_name(),
            Selection::Texture => TEXTURE_NAMES[self.params.texture.load(Ordering::Relaxed) as usize % 3].into(),
            Selection::InputGain => format!("{:.0}%", self.params.input_gain.get() * 100.0),
            Selection::SizePitch => {
                let bipolar = (self.params.size_pitch.get().clamp(0.0, 1.0) - 0.5) * 2.0;
                format!("{:.0}%sz {:+.1}oct", 2f32.powf(bipolar) * 100.0, -bipolar)
            }
            Selection::Decay => format!("{:.0}%", self.params.decay.get() * 100.0),
            Selection::Infinite => if self.params.infinite.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Selection::LowDamp => format!("{:.0}%", self.params.low_damp.get() * 100.0),
            Selection::HighDamp => format!("{:.0}%", self.params.high_damp.get() * 100.0),
            Selection::LowPassMode => if self.params.low_pass_mode.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Selection::LowPassResonance => format!("{:.0}%", self.params.low_pass_resonance.get() * 100.0),
            Selection::ShimmerAmount => format!("{:.0}%", self.params.shimmer_amount.get() * 100.0),
            Selection::ShimmerInterval => format!("{:+.1} st", self.shimmer_interval_semitones()),
            Selection::ShimmerRegen => if self.params.shimmer_regen.load(Ordering::Relaxed) { "regen".into() } else { "input".into() },
            Selection::GlimmerAmount => format!("{:.0}%", self.params.glimmer_amount.get() * 100.0),
            Selection::GlimmerHighBand => if self.params.glimmer_high_band.load(Ordering::Relaxed) { "high".into() } else { "low".into() },
            Selection::KarplusMode => if self.params.karplus_mode.load(Ordering::Relaxed) { "Karplus".into() } else { "Delay".into() },
            Selection::Octave => format!("{:+}", self.params.octave.load(Ordering::Relaxed)),
            Selection::Feedback => format!("{:.0}%", self.params.feedback.get() * 100.0),
            Selection::EchoOn => if self.params.echo_on.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Selection::DelayTune => {
                if self.params.karplus_mode.load(Ordering::Relaxed) {
                    let hz = KARPLUS_BASE_HZ * 2f32.powf(self.params.delay_tune.get().clamp(0.0, 1.0) * 4.0);
                    format!("{:.0} Hz", hz)
                } else if self.params.tap_active.load(Ordering::Relaxed) {
                    format!("{:.0} ms (tap)", self.params.tap_ms.get())
                } else {
                    let ms = 1.0 + self.params.delay_tune.get().clamp(0.0, 1.0).powi(2) * (DELAY_MAX_MS - 1.0) * 0.968;
                    format!("{:.0} ms", ms)
                }
            }
            Selection::LfoTarget => LFO_TARGET_NAMES[self.params.lfo_target.load(Ordering::Relaxed) as usize % 3].into(),
            Selection::LfoSpeed => {
                let hz = lfo_speed_hz(self.params.lfo_speed.get());
                format!("{:.2} Hz", hz)
            }
            Selection::LfoShape => LFO_SHAPE_NAMES[self.params.lfo_shape.load(Ordering::Relaxed) as usize % 6].into(),
            Selection::LfoDepth => format!("{:.0}%", self.params.lfo_depth.get() * 100.0),
            Selection::Dry => format!("{:.0}%", self.params.dry.get() * 100.0),
            Selection::Wet => format!("{:.0}%", self.params.wet.get() * 100.0),
            Selection::Clear => format!("pulse:{}", self.params.clear_trigger.load(Ordering::Relaxed)),
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
            Selection::Texture => {
                let cur = self.params.texture.load(Ordering::Relaxed) as i32;
                self.params.texture.store((cur + step).rem_euclid(3) as u32, Ordering::Relaxed);
            }
            Selection::InputGain => bump(&self.params.input_gain, delta, sensitivity, 0.0, 1.0),
            Selection::SizePitch => bump(&self.params.size_pitch, delta, sensitivity, 0.0, 1.0),
            Selection::Decay => bump(&self.params.decay, delta, sensitivity, 0.0, 0.97),
            Selection::Infinite => self.params.infinite.store(delta > 0, Ordering::Relaxed),
            Selection::LowDamp => bump(&self.params.low_damp, delta, sensitivity, 0.0, 1.0),
            Selection::HighDamp => bump(&self.params.high_damp, delta, sensitivity, 0.0, 1.0),
            Selection::LowPassMode => self.params.low_pass_mode.store(delta > 0, Ordering::Relaxed),
            Selection::LowPassResonance => bump(&self.params.low_pass_resonance, delta, sensitivity, 0.0, 0.95),
            Selection::ShimmerAmount => bump(&self.params.shimmer_amount, delta, sensitivity, 0.0, 1.0),
            Selection::ShimmerInterval => bump(&self.params.shimmer_interval, delta, sensitivity, 0.0, 1.0),
            Selection::ShimmerRegen => self.params.shimmer_regen.store(delta > 0, Ordering::Relaxed),
            Selection::GlimmerAmount => bump(&self.params.glimmer_amount, delta, sensitivity, 0.0, 1.0),
            Selection::GlimmerHighBand => self.params.glimmer_high_band.store(delta > 0, Ordering::Relaxed),
            Selection::KarplusMode => self.params.karplus_mode.store(delta > 0, Ordering::Relaxed),
            Selection::Octave => {
                let cur = self.params.octave.load(Ordering::Relaxed);
                self.params.octave.store(cur + step, Ordering::Relaxed);
            }
            Selection::Feedback => bump(&self.params.feedback, delta, sensitivity, 0.0, 0.98),
            Selection::EchoOn => self.params.echo_on.store(delta > 0, Ordering::Relaxed),
            Selection::DelayTune => {
                bump(&self.params.delay_tune, delta, sensitivity, 0.0, 1.0);
                self.params.tap_active.store(false, Ordering::Relaxed);
            }
            Selection::LfoTarget => {
                let cur = self.params.lfo_target.load(Ordering::Relaxed) as i32;
                self.params.lfo_target.store((cur + step).rem_euclid(3) as u32, Ordering::Relaxed);
            }
            Selection::LfoSpeed => bump(&self.params.lfo_speed, delta, sensitivity, 0.0, 1.0),
            Selection::LfoShape => {
                let cur = self.params.lfo_shape.load(Ordering::Relaxed) as i32;
                self.params.lfo_shape.store((cur + step).rem_euclid(6) as u32, Ordering::Relaxed);
            }
            Selection::LfoDepth => bump(&self.params.lfo_depth, delta, sensitivity, 0.0, 1.0),
            Selection::Dry => bump(&self.params.dry, delta, sensitivity, 0.0, 1.0),
            Selection::Wet => bump(&self.params.wet, delta, sensitivity, 0.0, 1.0),
            Selection::Clear => {
                self.params.clear_trigger.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Source => self.params.source.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed),
            Selection::Texture => self.params.texture.store(1, Ordering::Relaxed),
            Selection::InputGain => self.params.input_gain.set(0.5),
            Selection::SizePitch => self.params.size_pitch.set(0.5),
            Selection::Decay => self.params.decay.set(0.7),
            Selection::Infinite => self.params.infinite.store(false, Ordering::Relaxed),
            Selection::LowDamp => self.params.low_damp.set(0.0),
            Selection::HighDamp => self.params.high_damp.set(0.3),
            Selection::LowPassMode => self.params.low_pass_mode.store(false, Ordering::Relaxed),
            Selection::LowPassResonance => self.params.low_pass_resonance.set(0.3),
            Selection::ShimmerAmount => self.params.shimmer_amount.set(0.0),
            Selection::ShimmerInterval => self.params.shimmer_interval.set(1.0),
            Selection::ShimmerRegen => self.params.shimmer_regen.store(true, Ordering::Relaxed),
            Selection::GlimmerAmount => self.params.glimmer_amount.set(0.0),
            Selection::GlimmerHighBand => self.params.glimmer_high_band.store(true, Ordering::Relaxed),
            Selection::KarplusMode => self.params.karplus_mode.store(false, Ordering::Relaxed),
            Selection::Octave => self.params.octave.store(0, Ordering::Relaxed),
            Selection::Feedback => self.params.feedback.set(0.3),
            Selection::EchoOn => self.params.echo_on.store(false, Ordering::Relaxed),
            Selection::DelayTune => {
                self.params.delay_tune.set(0.3);
                self.params.tap_active.store(false, Ordering::Relaxed);
            }
            Selection::LfoTarget => self.params.lfo_target.store(0, Ordering::Relaxed),
            Selection::LfoSpeed => self.params.lfo_speed.set(0.3),
            Selection::LfoShape => self.params.lfo_shape.store(0, Ordering::Relaxed),
            Selection::LfoDepth => self.params.lfo_depth.set(0.0),
            Selection::Dry => self.params.dry.set(0.6),
            Selection::Wet => self.params.wet.set(0.6),
            Selection::Clear => {}
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

    /// The real oscilloscope + LFO indicator + reverb-tank energy
    /// meter -- see `Params::waveform_snapshot`/`lfo_phase`/
    /// `lfo_value`/`tank_energy` (all written once per audio block by
    /// `StarlabProcessor::process`), converted to connected
    /// line-segment geometry (see `crate::app::polyline_segments`)
    /// instead of drawn directly.
    pub(crate) fn output_visual(&self) -> StarlabVisual {
        const PANEL_W: f32 = 260.0;
        const PANEL_H: f32 = 110.0;
        let raw = self.params.waveform_snapshot.lock().unwrap().clone();
        let waveform = if raw.len() >= 2 {
            let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(&raw, PANEL_W, PANEL_H, true);
            crate::app::CurveSegments { mid_x, mid_y, length, angle_deg }
        } else {
            crate::app::CurveSegments::default()
        };
        let texture = self.params.texture.load(Ordering::Relaxed) as usize % 3;
        StarlabVisual {
            waveform,
            texture_name: TEXTURE_NAMES[texture].to_string(),
            texture_index: texture,
            infinite: self.params.infinite.load(Ordering::Relaxed),
            karplus_mode: self.params.karplus_mode.load(Ordering::Relaxed),
            lfo_phase: self.params.lfo_phase.get(),
            lfo_value: self.params.lfo_value.get(),
            tank_energy: self.params.tank_energy.get(),
        }
    }
}

impl PlayHost for StarlabApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS.len()
    }
    fn kit_label(&self, i: usize) -> String {
        CONTROLS[i % 16].1.to_string()
    }
    fn kit_value(&self, i: usize) -> String {
        self.leaf_value(CONTROLS[i % 16].0)
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        self.knob(i).norm()
    }
    fn kit_stepped(&self, i: usize) -> bool {
        self.knob(i).stepped()
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit(CONTROLS[i % 16].0, delta);
    }
    fn kit_reset(&mut self, i: usize) {
        self.reset(CONTROLS[i % 16].0);
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        self.knob(i).set(v);
        // Same rule as `edit`: moving DELAY/TUNE by hand (stick, hands or
        // a recalled moment) takes over from a tapped tempo, or the move
        // would be inaudible in Delay mode.
        if CONTROLS[i % 16].0 == Selection::DelayTune {
            self.params.tap_active.store(false, Ordering::Relaxed);
        }
    }
    /// A key plucks the pad with its pitch (Karplus mode); keys outside
    /// the pads' 16-semitone window fold in by octaves. In Delay mode any
    /// pad is a tap, so the same mapping is as good as any.
    fn kit_midi_pad(&self, note: u8) -> Option<usize> {
        let d = note as i32 - note_for(0, self.params.octave.load(Ordering::Relaxed));
        let rank = if (0..16).contains(&d) { d } else { d.rem_euclid(12) };
        Some(pad_rank(rank) as usize)
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        if self.karplus() { note_name(self.pad_note(pad)) } else { "TAP".into() }
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, held: bool) -> crate::led_output::PadColor {
        use crate::led_output::PadColor;
        if held {
            PadColor::Green
        } else if self.karplus() && self.pad_note(pad).rem_euclid(12) == 0 {
            PadColor::Blue
        } else if !self.karplus() && self.params.tap_active.load(Ordering::Relaxed) {
            // A learned tap tempo is in charge of the delay time.
            PadColor::Yellow
        } else {
            PadColor::Off
        }
    }
    fn kit_line(&self) -> String {
        let p = &self.params;
        let mode = if self.karplus() {
            let held = *p.held.lock().unwrap();
            match (0..16).find(|&i| held[i]) {
                Some(i) => format!("String {}", note_name(self.pad_note(i))),
                None => "String".to_string(),
            }
        } else {
            self.leaf_value(Selection::DelayTune)
        };
        let frozen = if p.infinite.load(Ordering::Relaxed) { "  FROZEN" } else { "" };
        format!("{mode}  tank {:.0}%{frozen}", p.tank_energy.get().clamp(0.0, 1.0) * 100.0)
    }
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.01).clamp(min, max);
    value.set(next);
}

/// Maps the SHIMMER INTERVAL knob (0..1, 0.5 = center) to a semitone
/// offset spanning -1..+1 octave, with the real module's two small
/// "detune" zones flanking center approximated by the curve simply
/// passing close to +-0.5 semitones near the middle rather than as
/// hard snap points.
fn shimmer_interval_semitones(v: f32) -> f32 {
    (v.clamp(0.0, 1.0) - 0.5) * 2.0 * 12.0
}

/// Maps the LFO SPEED knob (0..1) to a rate in Hz, spanning the real
/// module's 15-second to 1/15-second (15Hz) period range.
fn lfo_speed_hz(v: f32) -> f32 {
    let min_hz: f32 = 1.0 / 15.0;
    let max_hz: f32 = 15.0;
    min_hz * (max_hz / min_hz).powf(v.clamp(0.0, 1.0))
}

impl App for StarlabApp {
    fn needs_background_audio(&self) -> bool { self.params.source.load(Ordering::Relaxed) != crate::audio_bus::NO_SOURCE }
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

    fn tick(&mut self, input: &Input) {
        // The play view takes the knobs and D-pad first; in the menu they
        // pass straight through. Pads reach the string/tap input only on
        // PLUCK (kit layers hand back an empty grid).
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

        let mut held = self.params.held.lock().unwrap();
        for (i, pressed) in input.grid.iter().enumerate() {
            held[i] = *pressed;
        }
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let v = self.output_visual();
        crate::app::SlintExtra::Starlab(crate::app::StarlabExtra {
            waveform: v.waveform,
            texture_name: v.texture_name,
            texture_index: v.texture_index,
            infinite: v.infinite,
            karplus_mode: v.karplus_mode,
            lfo_phase: v.lfo_phase,
            lfo_value: v.lfo_value,
            tank_energy: v.tank_energy,
        })
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let sample_rate = 48000.0f32;
        Some(Box::new(StarlabProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            combs: std::array::from_fn(|i| CombFilter::new(vec![0.0; (COMB_DELAYS_MS[i] * SIZE_HEADROOM * 0.001 * sample_rate) as usize + 8])),
            allpasses: std::array::from_fn(|i| AllpassFilter::new(vec![0.0; (ALLPASS_DELAYS_MS[i] * SIZE_HEADROOM * 0.001 * sample_rate) as usize + 8])),
            size_pitch_shifter: PitchShifter::new((SIZE_PITCH_BUF_SECONDS * sample_rate) as usize),
            shimmer_shifter: PitchShifter::new((SHIMMER_BUF_SECONDS * sample_rate) as usize),
            delay_buf: vec![0.0; (DELAY_MAX_MS * 0.001 * sample_rate) as usize + 8],
            delay_pos: 0,
            karplus_buf: vec![0.0; (sample_rate / KARPLUS_MIN_HZ) as usize + 8],
            karplus_pos: 0,
            karplus_lp: 0.0,
            karplus_dc: 0.0,
            karplus_excite_remaining: 0,
            karplus_note_semis: 0.0,
            prev_held: [false; 16],
            tap_accum_samples: 0.0,
            shimmer_feedback_carry: 0.0,
            sparse_hold_value: 0.0,
            sparse_hold_counter: 0,
            diffuse_smooth: 0.0,
            glimmer_lp: 0.0,
            output_dc: 0.0,
            svf1: Svf::default(),
            svf2: Svf::default(),
            lfo_phase: 0.0,
            lfo_rand_value: 0.0,
            lfo_env: 0.0,
            rng: 0xC0FF_EE01,
            last_clear_seen: 0,
            tank_energy_smoothed: 0.0,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(STARLAB_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, STARLAB_TITLE);
        Text::new("Starlab", Point::new(16, 26), title).draw(fb).ok();
        let dim = MonoTextStyle::new(&SPLEEN_6X12, STARLAB_DIM);
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
        if self.kit.menu {
            self.list.draw_themed(fb, 16, 56, 22, 10, &display_rows, STARLAB_BG, STARLAB_DIM, STARLAB_ACCENT);
        } else if let Some(col) = self.play_column() {
            let pal = kit::draw::Palette { bg: STARLAB_BG, ink: STARLAB_TITLE, accent: STARLAB_ACCENT, dim: STARLAB_DIM, faint: STARLAB_FAINT };
            kit::draw::column(fb, &col, 16, 44, 350, 290, pal);
            Text::new("knobs: dials   D-pad: texture   F2: pads   R1: menu", Point::new(16, 348), dim).draw(fb).ok();
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

/// A feedback comb filter with a one-pole lowpass (HIGH DAMP) and a
/// slow-tracking-based highpass (LOW DAMP) in the feedback path --
/// the classic Schroeder reverb building block, extended with a
/// variable active length so SIZE can rescale it without reallocating.
struct CombFilter {
    buffer: Vec<f32>,
    pos: usize,
    lp_state: f32,
    dc_track: f32,
}

impl CombFilter {
    fn new(buffer: Vec<f32>) -> Self {
        Self { buffer, pos: 0, lp_state: 0.0, dc_track: 0.0 }
    }

    fn process(&mut self, input: f32, feedback: f32, len: usize, high_damp: f32, low_damp: f32) -> f32 {
        let len = len.clamp(1, self.buffer.len());
        let idx = self.pos % len;
        let out = self.buffer[idx];
        self.lp_state = out * (1.0 - high_damp) + self.lp_state * high_damp;
        self.dc_track += (self.lp_state - self.dc_track) * 0.002;
        let damped = self.lp_state - self.dc_track * low_damp;
        self.buffer[idx] = input + damped * feedback;
        self.pos = (self.pos + 1) % len;
        out
    }

    fn clear(&mut self) {
        self.buffer.iter_mut().for_each(|s| *s = 0.0);
        self.lp_state = 0.0;
        self.dc_track = 0.0;
    }
}

/// A unity-gain allpass filter -- diffuses the comb filters' output
/// into a smoother tail without adding its own coloration.
struct AllpassFilter {
    buffer: Vec<f32>,
    pos: usize,
}

impl AllpassFilter {
    fn new(buffer: Vec<f32>) -> Self {
        Self { buffer, pos: 0 }
    }

    fn process(&mut self, input: f32, len: usize) -> f32 {
        const G: f32 = 0.5;
        let len = len.clamp(1, self.buffer.len());
        let idx = self.pos % len;
        let buffered = self.buffer[idx];
        let out = -input * G + buffered;
        self.buffer[idx] = input + buffered * G;
        self.pos = (self.pos + 1) % len;
        out
    }

    fn clear(&mut self) {
        self.buffer.iter_mut().for_each(|s| *s = 0.0);
    }
}

fn interp(buf: &[f32], pos: f32) -> f32 {
    let len = buf.len();
    let i0 = (pos as usize) % len;
    let i1 = (i0 + 1) % len;
    let frac = pos - pos.floor();
    buf[i0] * (1.0 - frac) + buf[i1] * frac
}

/// A real-time delay-line pitch shifter: two read taps, half a buffer
/// apart, each advancing at `rate` while the write pointer advances
/// at 1 sample/sample; a Hann-shaped crossfade between the taps
/// (weighted by each tap's distance from the write pointer) hides the
/// periodic wrap-around discontinuity. The classic "Doppler"/granular
/// delay-based pitch shifter, the same technique used by real
/// hardware shimmer reverbs -- not a fake frequency-domain stand-in.
struct PitchShifter {
    buffer: Vec<f32>,
    write_pos: usize,
    read_pos: f32,
}

impl PitchShifter {
    fn new(len: usize) -> Self {
        Self { buffer: vec![0.0; len.max(8)], write_pos: 0, read_pos: 0.0 }
    }

    fn process(&mut self, input: f32, rate: f32) -> f32 {
        let len_f = self.buffer.len() as f32;
        self.buffer[self.write_pos] = input;
        self.write_pos = (self.write_pos + 1) % self.buffer.len();

        self.read_pos = (self.read_pos + rate).rem_euclid(len_f);
        let tap_b = (self.read_pos + len_f * 0.5).rem_euclid(len_f);
        let sample_a = interp(&self.buffer, self.read_pos);
        let sample_b = interp(&self.buffer, tap_b);

        let dist_a = (self.write_pos as f32 - self.read_pos).rem_euclid(len_f) / len_f;
        let w_a = 0.5 - 0.5 * (dist_a * TAU).cos();
        sample_a * w_a + sample_b * (1.0 - w_a)
    }

    fn clear(&mut self) {
        self.buffer.iter_mut().for_each(|s| *s = 0.0);
    }
}

/// A Chamberlin state-variable filter -- used as a real, stable,
/// resonance-capable lowpass for the LOW PASS filter mode. Two
/// instances are cascaded in the processor for a ~24dB/octave slope.
#[derive(Default)]
struct Svf {
    low: f32,
    band: f32,
}

impl Svf {
    fn process(&mut self, input: f32, freq_norm: f32, q: f32) -> f32 {
        let f = freq_norm.clamp(0.0005, 0.45);
        let q_inv = (1.0 - q.clamp(0.0, 0.95) * 0.98).max(0.05);
        self.low += f * self.band;
        let high = input - self.low - q_inv * self.band;
        self.band += f * high;
        if !self.low.is_finite() {
            self.low = 0.0;
        }
        if !self.band.is_finite() {
            self.band = 0.0;
        }
        self.low
    }

    fn clear(&mut self) {
        self.low = 0.0;
        self.band = 0.0;
    }
}

struct StarlabProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    combs: [CombFilter; NUM_COMBS],
    allpasses: [AllpassFilter; NUM_ALLPASS],
    /// Pitch-shifts the regenerating tail for SIZE/PITCH's "pitch"
    /// half (the "size" half comes from rescaling comb/allpass
    /// lengths directly).
    size_pitch_shifter: PitchShifter,
    /// Pitch-shifts either the reverb input (input shimmer) or the
    /// regenerating tail (regen shimmer), per SHIMMER REGEN.
    shimmer_shifter: PitchShifter,
    delay_buf: Vec<f32>,
    delay_pos: usize,
    karplus_buf: Vec<f32>,
    karplus_pos: usize,
    karplus_lp: f32,
    karplus_dc: f32,
    karplus_excite_remaining: u32,
    karplus_note_semis: f32,
    /// Edge-detection state for the pad grid, this sim's stand-in for
    /// the real module's TAP/TRIG (delay mode) and IN GATE/TAP-TRIG
    /// (Karplus pluck/bow) inputs.
    prev_held: [bool; 16],
    tap_accum_samples: f32,
    shimmer_feedback_carry: f32,
    sparse_hold_value: f32,
    sparse_hold_counter: u32,
    diffuse_smooth: f32,
    glimmer_lp: f32,
    output_dc: f32,
    svf1: Svf,
    svf2: Svf,
    lfo_phase: f32,
    lfo_rand_value: f32,
    lfo_env: f32,
    rng: u32,
    last_clear_seen: u32,
    /// Slow EMA of the reverb tank's (comb-section) output level, the
    /// real basis for `StarlabVisual::tank_energy` -- see the doc
    /// comment on `Params::tank_energy`.
    tank_energy_smoothed: f32,
}

impl StarlabProcessor {
    fn next_rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    fn clear_all(&mut self) {
        for c in self.combs.iter_mut() {
            c.clear();
        }
        for a in self.allpasses.iter_mut() {
            a.clear();
        }
        self.size_pitch_shifter.clear();
        self.shimmer_shifter.clear();
        self.delay_buf.iter_mut().for_each(|s| *s = 0.0);
        self.karplus_buf.iter_mut().for_each(|s| *s = 0.0);
        self.karplus_lp = 0.0;
        self.karplus_dc = 0.0;
        self.shimmer_feedback_carry = 0.0;
        self.diffuse_smooth = 0.0;
        self.glimmer_lp = 0.0;
        self.output_dc = 0.0;
        self.svf1.clear();
        self.svf2.clear();
        self.tank_energy_smoothed = 0.0;
    }

    fn lfo_waveform(&mut self, shape: u32, input_level: f32, speed01: f32) -> f32 {
        match shape % 6 {
            0 => {
                // Triangle
                let t = self.lfo_phase * 2.0;
                if t < 1.0 {
                    -1.0 + 2.0 * t
                } else {
                    3.0 - 2.0 * t
                }
            }
            1 => if self.lfo_phase < 0.5 { 1.0 } else { -1.0 }, // Square
            2 => 2.0 * self.lfo_phase - 1.0,                    // Ramp (rising)
            3 => 1.0 - 2.0 * self.lfo_phase,                    // Saw (falling)
            4 => self.lfo_rand_value,                           // Random S&H
            _ => {
                // Envelope: tracks input level, decay rate set by speed.
                let decay = 0.9 + (1.0 - speed01.clamp(0.0, 1.0)) * 0.0999;
                let target = input_level.abs().min(1.0);
                if target > self.lfo_env {
                    self.lfo_env = target;
                } else {
                    self.lfo_env *= decay;
                }
                self.lfo_env
            }
        }
    }
}

impl AudioProcessor for StarlabProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        for out in buffer.iter_mut() {
            *out = 0.0;
        }

        let clear_now = self.params.clear_trigger.load(Ordering::Relaxed);
        if clear_now != self.last_clear_seen {
            self.clear_all();
            self.last_clear_seen = clear_now;
        }

        let source_idx = self.params.source.load(Ordering::Relaxed);
        let tapped = self.audio_bus.get(source_idx).map(|b| b.lock().unwrap().clone());

        let texture = self.params.texture.load(Ordering::Relaxed) % 3;
        let input_gain = 0.3 + self.params.input_gain.get().clamp(0.0, 1.0) * 3.7;
        let decay = self.params.decay.get().clamp(0.0, 0.97);
        let infinite = self.params.infinite.load(Ordering::Relaxed);
        let low_damp = self.params.low_damp.get().clamp(0.0, 1.0);
        let high_damp = self.params.high_damp.get().clamp(0.0, 1.0);
        let low_pass_mode = self.params.low_pass_mode.load(Ordering::Relaxed);
        let low_pass_resonance = self.params.low_pass_resonance.get().clamp(0.0, 0.95);
        let shimmer_amount = self.params.shimmer_amount.get().clamp(0.0, 1.0);
        let shimmer_rate = 2f32.powf(shimmer_interval_semitones(self.params.shimmer_interval.get()) / 12.0);
        let shimmer_regen = self.params.shimmer_regen.load(Ordering::Relaxed);
        let glimmer_amount = self.params.glimmer_amount.get().clamp(0.0, 1.0);
        let glimmer_high_band = self.params.glimmer_high_band.load(Ordering::Relaxed);
        let karplus_mode = self.params.karplus_mode.load(Ordering::Relaxed);
        let feedback = self.params.feedback.get().clamp(0.0, 0.98);
        let echo_on = self.params.echo_on.load(Ordering::Relaxed);
        let delay_tune = self.params.delay_tune.get().clamp(0.0, 1.0);
        let lfo_target = self.params.lfo_target.load(Ordering::Relaxed) % 3;
        let lfo_speed01 = self.params.lfo_speed.get().clamp(0.0, 1.0);
        let lfo_hz = lfo_speed_hz(lfo_speed01);
        let lfo_shape = self.params.lfo_shape.load(Ordering::Relaxed);
        let lfo_depth = self.params.lfo_depth.get().clamp(0.0, 1.0);
        let dry_level = self.params.dry.get().clamp(0.0, 1.0);
        let wet_level = self.params.wet.get().clamp(0.0, 1.0);
        let held = *self.params.held.lock().unwrap();

        let frames = buffer.len() / channels;
        let mut mono = vec![0.0f32; frames];

        // Real per-block state captured for the Slint visualization
        // (see `StarlabVisual`): the LFO's last computed value this
        // block, and the reverb tank's accumulated output level.
        let mut last_lfo_value = 0.0f32;
        let mut tank_level_accum = 0.0f32;

        // SIZE/PITCH -> comb/allpass length scaling (once per block, matching
        // Beads' quality-profile pattern of recomputing derived lengths per call).
        let bipolar_base = (self.params.size_pitch.get().clamp(0.0, 1.0) - 0.5) * 2.0;

        for n in 0..frames {
            let dry_raw = tapped.as_ref().and_then(|b| b.get(n % b.len().max(1)).copied()).unwrap_or(0.0);

            // --- Pad-grid edge detection: stands in for TAP/TRIG and IN GATE. ---
            let mut rising_pad: Option<usize> = None;
            for i in 0..16 {
                if held[i] && !self.prev_held[i] && rising_pad.is_none() {
                    rising_pad = Some(i);
                }
                self.prev_held[i] = held[i];
            }
            let any_held = held.iter().any(|&h| h);

            if let Some(i) = rising_pad {
                let octave = self.params.octave.load(Ordering::Relaxed);
                let note = note_for(pad_rank(i as i32), octave);
                self.karplus_note_semis = (note - 48) as f32; // offset from the pad-grid's own C3 base
                if karplus_mode {
                    self.karplus_excite_remaining = PLUCK_SAMPLES;
                } else {
                    let elapsed = self.tap_accum_samples;
                    let min_s = 0.05 * sample_rate;
                    let max_s = 3.0 * sample_rate;
                    if (min_s..=max_s).contains(&elapsed) {
                        self.params.tap_ms.set(elapsed / sample_rate * 1000.0);
                        self.params.tap_active.store(true, Ordering::Relaxed);
                    }
                    self.tap_accum_samples = 0.0;
                }
            }
            self.tap_accum_samples = (self.tap_accum_samples + 1.0).min(4.0 * sample_rate);

            // --- LFO ---
            let lfo_raw = self.lfo_waveform(lfo_shape, dry_raw, lfo_speed01);
            self.lfo_phase = (self.lfo_phase + lfo_hz / sample_rate).fract();
            if self.lfo_phase < lfo_hz / sample_rate {
                self.lfo_rand_value = self.next_rand();
            }
            let lfo_value = lfo_raw * lfo_depth;
            last_lfo_value = lfo_value;

            // --- DELAY/KARPLUS section ---
            let dk_signal = if karplus_mode {
                let mut freq = KARPLUS_BASE_HZ * 2f32.powf(delay_tune * 4.0) * 2f32.powf(self.karplus_note_semis / 12.0);
                if lfo_target == 0 {
                    freq *= 2f32.powf(lfo_value * 2.0 / 12.0);
                }
                let len = ((sample_rate / freq.max(1.0)).round() as usize).clamp(4, self.karplus_buf.len());
                let idx = self.karplus_pos % len;
                let out = self.karplus_buf[idx];
                self.karplus_lp = out * (1.0 - high_damp) + self.karplus_lp * high_damp;
                self.karplus_dc += (self.karplus_lp - self.karplus_dc) * 0.002;
                let filtered = self.karplus_lp - self.karplus_dc * low_damp;
                let decay_coeff = 0.9 + feedback * 0.0994;
                let mut new_val = filtered * decay_coeff;
                if self.karplus_excite_remaining > 0 {
                    new_val += self.next_rand();
                    self.karplus_excite_remaining -= 1;
                } else if any_held {
                    new_val += self.next_rand() * 0.15; // continuous "bow"
                }
                self.karplus_buf[idx] = new_val;
                self.karplus_pos = (self.karplus_pos + 1) % len;
                out
            } else {
                let mut ms = if self.params.tap_active.load(Ordering::Relaxed) {
                    self.params.tap_ms.get()
                } else {
                    1.0 + delay_tune.powi(2) * (DELAY_MAX_MS - 1.0) * 0.968
                };
                if lfo_target == 0 {
                    ms += lfo_value * if echo_on { 80.0 } else { 35.0 };
                }
                let delay_samples = ((ms.max(1.0) * 0.001 * sample_rate) as usize).clamp(1, self.delay_buf.len() - 1);
                let write_idx = self.delay_pos % self.delay_buf.len();
                let read_idx = (self.delay_pos + self.delay_buf.len() - delay_samples) % self.delay_buf.len();
                let out = self.delay_buf[read_idx];
                let gained = (dry_raw * input_gain).clamp(-4.0, 4.0).tanh();
                self.delay_buf[write_idx] = gained + out * feedback.min(0.95);
                self.delay_pos = (self.delay_pos + 1) % self.delay_buf.len();
                out
            };

            // --- Input gain (soft-clipping stage) + delay/karplus summed at first "+" node ---
            let gained = (dry_raw * input_gain).clamp(-6.0, 6.0).tanh();
            let mut reverb_input = gained + dk_signal;

            // --- Glimmer: harmonic exciter on a high- or low-band of the input path ---
            self.glimmer_lp += (reverb_input - self.glimmer_lp) * 0.15;
            let band = if glimmer_high_band { reverb_input - self.glimmer_lp } else { self.glimmer_lp };
            let excited = (band * 5.0).tanh() - band;
            reverb_input += excited * glimmer_amount * 2.0;

            // --- Input-side shimmer (REGEN off) ---
            if !shimmer_regen && shimmer_amount > 0.0001 {
                let shifted = self.shimmer_shifter.process(reverb_input, shimmer_rate);
                reverb_input += shifted * shimmer_amount;
            }

            // --- INFINITE: freeze new content from entering the tank ---
            let fresh_input = if infinite { 0.0 } else { reverb_input };
            let mut comb_input = fresh_input + self.shimmer_feedback_carry;

            // --- Texture-dependent pre-processing ---
            match texture {
                0 => {
                    // Sparse: sample-and-hold decimation for a granular/staccato feel.
                    if self.sparse_hold_counter == 0 {
                        self.sparse_hold_value = comb_input;
                    }
                    self.sparse_hold_counter = (self.sparse_hold_counter + 1) % 2;
                    comb_input = self.sparse_hold_value;
                }
                2 => {
                    // Diffuse: slew-smooth the input for a slow-building wash.
                    self.diffuse_smooth += (comb_input - self.diffuse_smooth) * 0.01;
                    comb_input = self.diffuse_smooth;
                }
                _ => {}
            }

            // --- SIZE/PITCH: length scaling + LFO pitch modulation ---
            let mut bipolar = bipolar_base;
            if lfo_target == 1 {
                bipolar += lfo_value * 0.6;
            }
            let bipolar_clamped = bipolar.clamp(-1.0, 1.0);
            let size_mult = 2f32.powf(bipolar_clamped);
            let pitch_rate = 2f32.powf(-bipolar.clamp(-1.6, 1.6));

            let active_combs = if texture == 0 { 2 } else { NUM_COMBS };
            let allpass_passes = if texture == 2 { 2 } else { 1 };
            let high_damp_for_combs = if low_pass_mode { 0.15 } else { high_damp };
            let mut high_damp_for_filter = high_damp;
            if lfo_target == 2 {
                let scale = if low_pass_mode { 0.5 } else { 0.25 };
                high_damp_for_filter = (high_damp_for_filter + lfo_value * scale).clamp(0.0, 1.0);
            }

            let mut comb_sum = 0.0f32;
            for i in 0..active_combs {
                let len = ((COMB_DELAYS_MS[i] * 0.001 * sample_rate * size_mult) as usize).clamp(16, self.combs[i].buffer.len());
                comb_sum += self.combs[i].process(comb_input, decay, len, high_damp_for_combs, low_damp);
            }
            comb_sum /= active_combs as f32;
            tank_level_accum += comb_sum.abs();

            let mut tail = comb_sum;
            for _ in 0..allpass_passes {
                for (j, ap) in self.allpasses.iter_mut().enumerate() {
                    let len = ((ALLPASS_DELAYS_MS[j] * 0.001 * sample_rate * size_mult) as usize).clamp(4, ap.buffer.len());
                    tail = ap.process(tail, len);
                }
            }

            let pitched_tail = self.size_pitch_shifter.process(tail, pitch_rate);

            // --- Regenerative shimmer (REGEN on): pitch-shift the tail, feed
            // back into next sample's comb input. ---
            self.shimmer_feedback_carry = if shimmer_regen && shimmer_amount > 0.0001 {
                self.shimmer_shifter.process(pitched_tail, shimmer_rate) * shimmer_amount
            } else {
                0.0
            };

            // --- Output filtering: LOW DAMP highpass, then optional 24dB/oct
            // resonant lowpass (LOW PASS mode repurposes HIGH DAMP as cutoff). ---
            let mut wet_out = pitched_tail;
            self.output_dc += (wet_out - self.output_dc) * 0.002;
            wet_out -= self.output_dc * low_damp;
            if low_pass_mode {
                let cutoff_hz = 8000.0 * (200.0f32 / 8000.0).powf(high_damp_for_filter);
                let freq_norm = cutoff_hz / sample_rate;
                wet_out = self.svf1.process(wet_out, freq_norm, low_pass_resonance);
                wet_out = self.svf2.process(wet_out, freq_norm, low_pass_resonance * 0.5);
            }

            let echo_contrib = if echo_on { dk_signal } else { 0.0 };
            let mixed = dry_raw * dry_level + wet_out * wet_level + echo_contrib * wet_level;
            mono[n] = mixed.tanh();
        }

        for (out, s) in buffer.chunks_mut(channels).zip(mono.iter()) {
            for ch in out.iter_mut() {
                *ch = *s;
            }
        }
        *self.params.bus_out.lock().unwrap() = mono.clone();

        // Once per block (not per-sample): a downsampled snapshot of
        // the real processed output for the Slint oscilloscope (same
        // technique Black Hole uses), plus the real LFO phase/value
        // and a smoothed read of the reverb tank's real energy level
        // -- see `StarlabVisual`, no fabricated data.
        const SNAPSHOT_POINTS: usize = 80;
        if mono.len() >= SNAPSHOT_POINTS {
            let step = mono.len() as f32 / SNAPSHOT_POINTS as f32;
            let snapshot: Vec<f32> = (0..SNAPSHOT_POINTS).map(|i| mono[((i as f32 * step) as usize).min(mono.len() - 1)]).collect();
            *self.params.waveform_snapshot.lock().unwrap() = snapshot;
        } else if !mono.is_empty() {
            *self.params.waveform_snapshot.lock().unwrap() = mono.clone();
        }

        self.params.lfo_phase.set(self.lfo_phase);
        self.params.lfo_value.set(last_lfo_value);

        let block_tank_level = tank_level_accum / frames.max(1) as f32;
        self.tank_energy_smoothed += (block_tank_level - self.tank_energy_smoothed) * 0.2;
        // Scale so a healthy sustained tail sits comfortably below
        // full-scale on the meter; the exact factor is cosmetic, the
        // underlying value is the tank's real, already-computed level.
        self.params.tank_energy.set((self.tank_energy_smoothed * 2.5).clamp(0.0, 1.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> (StarlabApp, Arc<AudioBus>) {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let app = StarlabApp::new(sensitivity, nav_speed, modbus, Arc::clone(&audio_bus), mixer_bus);
        (app, audio_bus)
    }

    /// A silent tapped source, no pads held -- must be silence, not a
    /// panic.
    #[test]
    fn silence_in_is_silence_out() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed); // Starlab registers its own bus first (index 0) -- "Source" lands at 1.
        *source.lock().unwrap() = vec![0.0; 512];
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|&s| s.abs() < 1e-6));
    }

    /// A loud tapped source with decay/wet up must produce an audible
    /// reverberant tail.
    #[test]
    fn a_loud_tapped_source_produces_an_audible_reverb_tail() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.8; 512];
        app.params.decay.set(0.8);
        app.params.dry.set(0.0);
        app.params.wet.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.01, "expected an audible reverb tail from a loud tapped source, got peak {peak}");
    }

    /// Karplus-Strong mode: a plucked string (pad rising edge) must
    /// produce audible output even with nothing tapped.
    #[test]
    fn a_karplus_pluck_produces_audible_output_with_no_input() {
        let (mut app, _audio_bus) = new_app();
        app.params.karplus_mode.store(true, Ordering::Relaxed);
        app.params.echo_on.store(true, Ordering::Relaxed);
        app.params.dry.set(0.0);
        app.params.wet.set(1.0);
        app.params.feedback.set(0.8);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        // No pad held yet -- must stay silent.
        processor.process(&mut buffer, 2, 48000.0);
        let peak_before = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert_eq!(peak_before, 0.0, "expected silence before any pluck, got peak {peak_before}");

        // Pluck: press then release a pad.
        *app.params.held.lock().unwrap() = std::array::from_fn(|i| i == 0);
        let mut peak = 0.0f32;
        for i in 0..30 {
            if i == 1 {
                *app.params.held.lock().unwrap() = [false; 16];
            }
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.01, "expected an audible plucked string, got peak {peak}");
    }

    /// Maximum decay/shimmer/feedback must never blow up -- stays
    /// finite and bounded over a long sustained render, across every
    /// texture.
    #[test]
    fn max_decay_shimmer_and_feedback_stays_bounded_and_finite() {
        for texture in 0..3u32 {
            let (mut app, audio_bus) = new_app();
            let source = audio_bus.register("Source");
            app.params.source.store(1, Ordering::Relaxed);
            *source.lock().unwrap() = vec![0.9; 512];
            app.params.texture.store(texture, Ordering::Relaxed);
            app.params.decay.set(0.97);
            app.params.shimmer_amount.set(1.0);
            app.params.glimmer_amount.set(1.0);
            app.params.feedback.set(0.98);
            app.params.echo_on.store(true, Ordering::Relaxed);
            app.params.low_pass_mode.store(true, Ordering::Relaxed);
            app.params.low_pass_resonance.set(0.95);
            app.params.lfo_depth.set(1.0);
            app.params.dry.set(1.0);
            app.params.wet.set(1.0);

            let mut processor = app.audio_processor().unwrap();
            let mut buffer = vec![0.0f32; 512 * 2];
            for _ in 0..200 {
                processor.process(&mut buffer, 2, 48000.0);
            }
            assert!(buffer.iter().all(|s| s.is_finite()), "texture {texture}: output must stay finite");
            let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
            assert!(peak <= 1.0001, "texture {texture}: expected soft-clipped output, got peak {peak}");
        }
    }

    /// INFINITE must stop new audio from entering the reverb tank --
    /// engaged immediately (before anything real was ever captured),
    /// a newly loud source must never reach the (wet) output.
    #[test]
    fn infinite_freezes_the_tank_before_any_input() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        app.params.dry.set(0.0);
        app.params.wet.set(1.0);
        app.params.decay.set(0.9);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];

        app.params.infinite.store(true, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.9; 512];
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        assert!(buffer.iter().all(|&s| s == 0.0), "expected silence while INFINITE with an empty tank, buffer was not all-zero");
    }

    /// Delay mode with ECHO ON must produce audible taps of a loud
    /// tapped source even with DECAY at minimum (a "delay only"
    /// experience, per the manual).
    #[test]
    fn delay_mode_with_echo_on_and_zero_decay_still_produces_taps() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.8; 512];
        app.params.decay.set(0.0);
        app.params.echo_on.store(true, Ordering::Relaxed);
        app.params.feedback.set(0.5);
        app.params.delay_tune.set(0.05);
        app.params.dry.set(0.0);
        app.params.wet.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut peak = 0.0f32;
        for _ in 0..60 {
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.01, "expected audible delay taps, got peak {peak}");
    }

    /// CLEAR must instantly flush the reverb tank -- a loud sustained
    /// tail, once cleared, must drop back toward silence (with the
    /// source then silenced) rather than continuing to ring.
    #[test]
    fn clear_flushes_the_reverb_tank() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.9; 512];
        app.params.decay.set(0.9);
        app.params.dry.set(0.0);
        app.params.wet.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let peak_before = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak_before > 0.01, "expected a built-up tail before clearing, got peak {peak_before}");

        *source.lock().unwrap() = vec![0.0; 512];
        app.params.clear_trigger.fetch_add(1, Ordering::Relaxed);
        processor.process(&mut buffer, 2, 48000.0);
        let peak_after = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak_after < peak_before, "expected CLEAR to reduce the tail, before {peak_before} after {peak_after}");
    }

    /// Starlab must republish its own output on the audio bus.
    #[test]
    fn republishes_its_output_on_the_audio_bus() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed); // Starlab registers its own bus first (index 0) -- "Source" lands at 1.
        *source.lock().unwrap() = vec![0.5; 256];

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 256 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        let published = app.params.bus_out.lock().unwrap();
        assert_eq!(published.len(), 256);
    }

    /// A loud tapped source with a modulating LFO must produce real
    /// oscilloscope geometry, a nonzero tank energy reading, and an
    /// LFO phase that stays in its real 0..1 range -- `output_visual`
    /// must reflect genuine per-block DSP state, not fabricated data.
    #[test]
    fn output_visual_reflects_real_processed_state() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.8; 512];
        app.params.decay.set(0.9);
        app.params.dry.set(0.0);
        app.params.wet.set(1.0);
        app.params.lfo_depth.set(1.0);
        app.params.lfo_speed.set(0.9);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..30 {
            processor.process(&mut buffer, 2, 48000.0);
        }

        let visual = app.output_visual();
        assert!(!visual.waveform.length.is_empty(), "expected real oscilloscope geometry from a loud tapped source");
        assert!(visual.tank_energy > 0.0, "expected nonzero tank energy after a loud sustained reverb tail, got {}", visual.tank_energy);
        assert!((0.0..1.0).contains(&visual.lfo_phase), "LFO phase must stay in its real 0..1 range, got {}", visual.lfo_phase);
    }

    /// CLEAR must flush the reverb tank's real energy, not just mute
    /// the output -- the tank_energy visual reading must drop after a
    /// CLEAR, mirroring `clear_flushes_the_reverb_tank` above.
    #[test]
    fn tank_energy_drops_after_clear() {
        let (mut app, audio_bus) = new_app();
        let source = audio_bus.register("Source");
        app.params.source.store(1, Ordering::Relaxed);
        *source.lock().unwrap() = vec![0.9; 512];
        app.params.decay.set(0.9);
        app.params.dry.set(0.0);
        app.params.wet.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..40 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let energy_before = app.output_visual().tank_energy;
        assert!(energy_before > 0.0, "expected built-up tank energy before clearing, got {energy_before}");

        *source.lock().unwrap() = vec![0.0; 512];
        app.params.clear_trigger.fetch_add(1, Ordering::Relaxed);
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let energy_after = app.output_visual().tank_energy;
        assert!(energy_after < energy_before, "expected CLEAR to reduce the tank energy reading, before {energy_before} after {energy_after}");
    }

    #[test]
    fn opens_playable_and_knob1_turns_decay() {
        let (mut app, _bus) = new_app();
        assert!(app.play_column().is_some(), "play view first");
        let decay = app.params.decay.get();
        app.tick(&Input { knob1: 5, ..Default::default() });
        assert!(app.params.decay.get() > decay, "knob 1 is Decay on the play view");
        app.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(app.params.texture.load(Ordering::Relaxed), 2, "D-pad up = next texture");
        app.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(app.play_column().is_none(), "R1 opens the full menu");
    }

    /// The PLUCK layer still feeds the pads to the string: a pad pressed
    /// through tick() reaches `held`, and a processor plucks from it.
    #[test]
    fn pluck_layer_still_plucks_the_string() {
        let (mut app, _bus) = new_app();
        app.params.karplus_mode.store(true, Ordering::Relaxed);
        app.params.echo_on.store(true, Ordering::Relaxed);
        app.params.dry.set(0.0);
        app.params.wet.set(1.0);
        app.params.feedback.set(0.8);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        app.tick(&Input { grid: std::array::from_fn(|i| i == 12), ..Default::default() });
        assert!(app.params.held.lock().unwrap()[12]);
        let mut peak = 0.0f32;
        for i in 0..30 {
            if i == 1 {
                app.tick(&Input::default());
            }
            processor.process(&mut buffer, 2, 48000.0);
            peak = peak.max(buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs())));
        }
        assert!(peak > 0.01, "a pad on PLUCK plucks the string, got peak {peak}");
    }

    #[test]
    fn throws_freeze_the_tank_and_let_go() {
        let (mut app, _bus) = new_app();
        app.toggle_grid_mode(); // Controls
        app.toggle_grid_mode(); // Moments
        app.toggle_grid_mode(); // Throws
        assert_eq!(app.grid_mode_label(), Some("THROWS"));
        let freeze = Input { grid: std::array::from_fn(|i| i == kit::rank_pad(0)), ..Default::default() };
        app.tick(&freeze);
        assert!(app.params.infinite.load(Ordering::Relaxed), "held FREEZE engages INFINITE");
        assert!(!app.params.held.lock().unwrap().iter().any(|h| *h), "throw pads never pluck");
        app.tick(&Input::default());
        assert!(!app.params.infinite.load(Ordering::Relaxed), "and lets go on release");
    }
}
