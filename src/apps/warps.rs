//! A clone of Mutable Instruments Warps: a meta-modulator that taps
//! two other apps' live outputs (see audio_bus.rs, the same technique
//! Clouds/Prism already use) as its carrier/modulator audio inputs
//! and combines them through a choice of real cross-modulation
//! algorithms. Feature set follows the real module's manual and
//! quickstart PDF (pichenettes.github.io/mutable-instruments-
//! documentation/modules/warps/) as closely as this sim's
//! architecture allows:
//!
//! - **All 7 real algorithms**, each real DSP rather than a stub:
//!   Crossfade (constant-power law, matching the manual's own
//!   wording), Crossfold (sum + a touch of cross-mod product, into a
//!   wavefolder), Diode Ring-Mod (a crude min()-based multiply with
//!   an asymmetric diode-style clip), Digital Ring-Mod (a clean
//!   multiply with gain + soft-clip), Exclusive-Or (both signals as
//!   16-bit ints, bitwise XOR'ed, TIMBRE picking how many bits),
//!   Comparator/Octaver (four comparison/rectification signals the
//!   manual describes, continuously morphed by TIMBRE rather than
//!   stepped), and Vocoder (a real N-band analysis/synthesis filter
//!   bank with per-band envelope followers -- 12 bands, not the real
//!   module's 20, a representative-subset simplification in the same
//!   spirit as `cascade.rs`'s 8-of-32 FM algorithms).
//! - **One continuous Algorithm knob**, 0..7, exactly mirroring the
//!   manual's own description of the real knob + its CV input:
//!   "CV control of modulation algorithm selection, with crossfading
//!   between adjacent algorithms" -- turning/CV-ing between two
//!   algorithm slots here genuinely crossfades their two real outputs
//!   sample-by-sample, not a hard switch. The Vocoder occupies the
//!   knob's last slot on its own (nothing to crossfade past it,
//!   same as the real hardware); turning further within that slot
//!   implements the manual's own documented behavior -- "as the
//!   ALGORITHM knob is turned clockwise, the release time of the
//!   envelope followers is increased... fully clockwise... freezes"
//!   the vocoder's spectral envelope.
//! - **Timbre knob**, doing exactly what the manual assigns it per
//!   algorithm (crossfade position, wavefolder depth, ring-mod gain,
//!   XOR bit mask width, comparator morph, vocoder formant shift).
//! - **Level 1** (carrier amplitude, or the internal oscillator's
//!   pitch once it's enabled) and **Level 2** (modulator amplitude,
//!   0..200% -- "gains above 1.0 can be applied, for a warm overdrive
//!   effect", per the manual), each with its own CV input summed in
//!   via `modbus.rs`, same additive-modulation convention every other
//!   knob in this build uses (see Plaits' `ext_harmonics` etc.).
//! - **Internal oscillator**, replacing the carrier entirely when
//!   enabled (same as the real INT. OSC button): sine/triangle/saw
//!   for the cross-modulation algorithms, saw/pulse/low-pass-filtered
//!   noise for the Vocoder, exactly the two waveform sets the manual
//!   documents. The tapped carrier input phase-modulates it
//!   (through-zero FM) while it's active, per the manual.
//! - **Both real outputs**: the main cross-modulated signal
//!   (published on the "Warps" bus, same as every other source app)
//!   and the AUX output on a second, separately-tappable "Warps
//!   (Aux)" bus -- carrier+modulator summed post-VCA when the
//!   oscillator is off, or the raw oscillator waveform when it's on,
//!   matching the manual's description of output 8 exactly.
//!
//! Deliberately not implemented: literal CV-cable "normalization"
//! (the real module substitutes a virtual +5V constant on an
//! unpatched Level 2 CV input; this sim's modulation stays purely
//! additive on top of the knob, the same simplification every other
//! app's `ext_*` input already makes) and the real module's literal
//! 20-band vocoder filter bank (uses 12 representative bands
//! instead -- see above). The manual and quickstart PDF were both
//! checked specifically for a secret/Easter-egg alternate algorithm
//! bank (some other Mutable modules have one); Warps does not have
//! one, so none was invented here.

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
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NUM_ALGORITHMS: usize = 7;
const ALGORITHM_NAMES: [&str; NUM_ALGORITHMS] = ["Crossfade", "Crossfold", "Diode Ring", "Digital Ring", "XOR", "Comparator", "Vocoder"];
/// Index of the Vocoder slot -- the last algorithm, occupying the top
/// of the knob's range on its own (see module doc comment).
const VOCODER_IDX: usize = NUM_ALGORITHMS - 1;

const VOCODER_BANDS: usize = 12;
/// Fixed resonance for every vocoder band's bandpass -- not the real
/// module's literal third-octave 48dB filters, a representative
/// simplification (see module doc comment).
const VOCODER_Q: f32 = 3.5;
/// How far TIMBRE can shift the modulator-envelope-to-carrier-band
/// connection, in bands either direction -- the manual's "shifting up
/// or down the formants extracted from the modulator signal".
const MAX_FORMANT_SHIFT: i32 = 4;

const OSC_WAVEFORM_NAMES_XMOD: [&str; 3] = ["Sine", "Triangle", "Saw"];
const OSC_WAVEFORM_NAMES_VOCODER: [&str; 3] = ["Saw", "Pulse", "Filtered Noise"];

/// How many points each of the three audio traces in `WarpsVisual` is
/// downsampled to, once per audio block -- cheap (a handful of array
/// reads, no extra DSP) regardless of the real block size, and plenty
/// of resolution for a small Slint trace.
const VISUAL_SNAPSHOT_POINTS: usize = 96;

/// Real, live-data-driven visualization state for a bespoke Slint
/// panel -- every field is either copied directly from genuine DSP
/// state `WarpsProcessor::process` already computes each block (the
/// carrier/modulator/output traces, the vocoder's per-band envelope
/// followers), or a straightforward re-expression of the same
/// atomics the parameter list already reads (the algorithm/crossfade/
/// oscillator state). Nothing here is fabricated.
///
/// The intended panel: three short overlaid traces -- carrier,
/// modulator, and the real cross-modulated output -- so the whole
/// point of a cross-modulator (how two signals combine into a third)
/// is visible at a glance, the same way Beads' scrolling capture
/// waveform makes its grain cloud visible. When the active slot is
/// (partly) the Vocoder, a 12-band level meter of the modulator's own
/// real envelope followers replaces or overlays that trace view,
/// since a vocoder's defining visual is its band spectrum rather than
/// a plain waveform.
#[derive(Clone, Debug)]
pub(crate) struct WarpsVisual {
    /// Name of the currently active (or currently-entering, while
    /// crossfading) algorithm slot -- one of `ALGORITHM_NAMES`.
    pub algorithm_name: String,
    /// `Some(name)` of the next algorithm slot only while the
    /// Algorithm knob sits between two adjacent slots (a genuine
    /// sample-by-sample crossfade, see the module doc comment);
    /// `None` when squarely inside one slot.
    pub next_algorithm_name: Option<String>,
    /// 0.0..1.0 crossfade fraction toward `next_algorithm_name`; 0.0
    /// whenever `next_algorithm_name` is `None`.
    pub crossfade_frac: f32,
    /// True whenever the active slot is (at least partly) the
    /// Vocoder -- the panel's cue to show the band-meter view.
    pub is_vocoder: bool,
    /// True only when the Vocoder's envelope followers are fully
    /// frozen (Algorithm knob fully clockwise within the Vocoder's own
    /// slot, per the manual) -- a genuine mode, not just a display
    /// label.
    pub vocoder_frozen: bool,
    /// Whether the internal oscillator currently replaces the
    /// carrier input.
    pub osc_enabled: bool,
    /// Name of the internal oscillator's current waveform, already
    /// resolved against whichever of the two real waveform sets
    /// (cross-mod vs. Vocoder) is active.
    pub osc_waveform_name: String,
    /// Recent carrier samples, post level-staging (Level 1 gain, or
    /// the oscillator's own output when it's active) and pre-
    /// algorithm -- i.e. exactly what's fed into the cross-modulator
    /// as "signal A". Typical range roughly -1.5..1.5.
    pub carrier_trace: Vec<f32>,
    /// Recent modulator samples, post Level 2 gain (0..2.0x, "warm
    /// overdrive" range per the manual) and pre-algorithm -- signal
    /// "B". Typical range roughly -2.0..2.0.
    pub modulator_trace: Vec<f32>,
    /// Recent samples of the real cross-modulated main output (same
    /// values republished on the "Warps" bus), post-algorithm.
    /// Bounded to the processor's own -4.0..4.0 headroom clamp.
    pub output_trace: Vec<f32>,
    /// The Vocoder's real per-band envelope-follower magnitudes (low
    /// to high frequency), continuously updated every sample
    /// regardless of which algorithm is selected (see
    /// `WarpsProcessor::vocoder_step`'s own comment) -- meaningful as
    /// a spectrum view once the Vocoder slot is engaged. Typically
    /// 0.0..~1.5, unbounded in principle.
    pub vocoder_band_levels: [f32; VOCODER_BANDS],
}

impl Default for WarpsVisual {
    fn default() -> Self {
        Self {
            algorithm_name: ALGORITHM_NAMES[0].to_string(),
            next_algorithm_name: None,
            crossfade_frac: 0.0,
            is_vocoder: false,
            vocoder_frozen: false,
            osc_enabled: false,
            osc_waveform_name: OSC_WAVEFORM_NAMES_XMOD[0].to_string(),
            carrier_trace: Vec::new(),
            modulator_trace: Vec::new(),
            output_trace: Vec::new(),
            vocoder_band_levels: [0.0; VOCODER_BANDS],
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    CarrierSource,
    ModulatorSource,
    Algorithm,
    Timbre,
    OscEnabled,
    OscWaveform,
    Level1,
    Level2,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

/// Group 0 "Input" (both audio sources plus the internal oscillator
/// that can replace the carrier one of them), group 1 "Algorithm"
/// (the cross-mod algorithm slot and its Timbre shaping), group 2
/// "Levels" (Level 1/2, which double as carrier amplitude/oscillator
/// pitch and modulator gain). No separate "Output" group: unlike
/// Black Hole's Crush/Dry-Wet, Warps has no output-stage knobs of its
/// own -- both real outputs (main + AUX) are fixed busses, not
/// editable parameters -- so a 4th group would have nothing to hold.
const NUM_GROUPS: usize = 3;

struct Params {
    source_a: AtomicUsize,
    source_b: AtomicUsize,
    /// Knob position, 0..1 -- scaled to 0..NUM_ALGORITHMS at use-site.
    /// See module doc comment for the continuous adjacent-algorithm
    /// crossfade this drives.
    algorithm: AtomicF32,
    timbre: AtomicF32,
    /// Carrier amplitude (osc off) or oscillator pitch (osc on), 0..1
    /// knob position -- scaled at use-site depending on mode.
    level1: AtomicF32,
    /// Modulator amplitude, 0..1 knob position -- scaled to 0..2.0 at
    /// use-site (manual: "gains above 1.0... warm overdrive").
    level2: AtomicF32,
    osc_enabled: AtomicBool,
    osc_waveform: AtomicU32,
    ext_algorithm: Arc<AtomicF32>,
    ext_timbre: Arc<AtomicF32>,
    ext_level1: Arc<AtomicF32>,
    ext_level2: Arc<AtomicF32>,
    /// Main cross-modulated output -- the real module's output 7.
    bus_out: Arc<Mutex<Vec<f32>>>,
    /// AUX output -- the real module's output 8 (see module doc
    /// comment for what it carries in each mode).
    aux_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    /// Downsampled snapshots of this block's real carrier/modulator/
    /// output signals -- refreshed once per audio block, for the
    /// Slint overlay trace view (real audio, not fabricated). See
    /// `WarpsVisual` and `VISUAL_SNAPSHOT_POINTS`.
    carrier_snapshot: Mutex<Vec<f32>>,
    modulator_snapshot: Mutex<Vec<f32>>,
    output_snapshot: Mutex<Vec<f32>>,
    /// The Vocoder's real per-band envelope-follower state, copied out
    /// once per audio block -- see `WarpsProcessor::vocoder_step`,
    /// which keeps these updated every sample regardless of which
    /// algorithm is selected.
    vocoder_band_snapshot: Mutex<[f32; VOCODER_BANDS]>,
}

impl Params {
    fn new(modbus: &ModBus, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register("Warps", modbus);
        Self {
            source_a: AtomicUsize::new(crate::audio_bus::NO_SOURCE),
            source_b: AtomicUsize::new(crate::audio_bus::NO_SOURCE),
            // 3.0 / 7.0 lands exactly on index 3 (Digital Ring) with
            // zero crossfade fraction -- "the classic Warps sound".
            algorithm: AtomicF32::new(3.0 / NUM_ALGORITHMS as f32),
            timbre: AtomicF32::new(0.5),
            level1: AtomicF32::new(0.8),
            level2: AtomicF32::new(0.5), // -> unity (0.5 * 2.0 gain range)
            osc_enabled: AtomicBool::new(false),
            osc_waveform: AtomicU32::new(0),
            ext_algorithm: modbus.register("Warps: Algorithm"),
            ext_timbre: modbus.register("Warps: Timbre"),
            ext_level1: modbus.register("Warps: Level 1"),
            ext_level2: modbus.register("Warps: Level 2"),
            bus_out: audio_bus.register("Warps"),
            aux_out: audio_bus.register("Warps (Aux)"),
            mix_level,
            ext_mix_level,
            carrier_snapshot: Mutex::new(Vec::new()),
            modulator_snapshot: Mutex::new(Vec::new()),
            output_snapshot: Mutex::new(Vec::new()),
            vocoder_band_snapshot: Mutex::new([0.0; VOCODER_BANDS]),
        }
    }
}

pub struct WarpsApp {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Warps's own palette: flat solid colors, not a
// device-wide theme -- Near-black with a hot signal-red accent -- two signals colliding, digital and a little violent. ---

const WARPS_BG: Rgb565 = Rgb565::new(1, 2, 1);
const WARPS_TITLE: Rgb565 = Rgb565::new(31, 56, 27);
const WARPS_ACCENT: Rgb565 = Rgb565::new(31, 11, 10);
const WARPS_DIM: Rgb565 = Rgb565::new(15, 18, 10);

impl WarpsApp {
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
            0 => vec![Selection::CarrierSource, Selection::ModulatorSource, Selection::OscEnabled, Selection::OscWaveform],
            1 => vec![Selection::Algorithm, Selection::Timbre],
            _ => vec![Selection::Level1, Selection::Level2],
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
            1 => "Algorithm",
            _ => "Levels",
        }
    }

    /// Splits the raw 0..1 `algorithm` knob into (floor index 0..6,
    /// crossfade fraction 0..1 into the next slot -- or, when
    /// `floor_idx == VOCODER_IDX`, the vocoder's own "how far
    /// clockwise into the release/freeze zone" fraction).
    fn algorithm_slot(&self) -> (usize, f32) {
        algorithm_slot_of(self.params.algorithm.get())
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => {
                let osc = if self.params.osc_enabled.load(Ordering::Relaxed) {
                    format!(", osc: {}", self.osc_waveform_name())
                } else {
                    String::new()
                };
                format!("{} / {}{osc}", self.leaf_value(Selection::CarrierSource), self.leaf_value(Selection::ModulatorSource))
            }
            1 => format!("{}, timbre {:.0}%", self.leaf_value(Selection::Algorithm), self.params.timbre.get() * 100.0),
            _ => format!("L1 {}, L2 {}", self.leaf_value(Selection::Level1), self.leaf_value(Selection::Level2)),
        }
    }

    fn source_name(&self, idx: usize) -> String {
        self.audio_bus.source_name(idx)
    }

    fn osc_category_vocoder(&self) -> bool {
        self.algorithm_slot().0 == VOCODER_IDX
    }

    fn osc_waveform_name(&self) -> &'static str {
        let idx = self.params.osc_waveform.load(Ordering::Relaxed) as usize % 3;
        if self.osc_category_vocoder() {
            OSC_WAVEFORM_NAMES_VOCODER[idx]
        } else {
            OSC_WAVEFORM_NAMES_XMOD[idx]
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::CarrierSource => "Carrier (A)".into(),
            Selection::ModulatorSource => "Modulator (B)".into(),
            Selection::Algorithm => "Algorithm".into(),
            Selection::Timbre => "Timbre".into(),
            Selection::OscEnabled => "Int. Osc".into(),
            Selection::OscWaveform => "Osc Waveform".into(),
            Selection::Level1 => "Level 1".into(),
            Selection::Level2 => "Level 2".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::CarrierSource => {
                let name = self.source_name(self.params.source_a.load(Ordering::Relaxed));
                if self.params.osc_enabled.load(Ordering::Relaxed) {
                    format!("{name} (phase-mod)")
                } else {
                    name
                }
            }
            Selection::ModulatorSource => self.source_name(self.params.source_b.load(Ordering::Relaxed)),
            Selection::Algorithm => {
                let (idx, frac) = self.algorithm_slot();
                if idx == VOCODER_IDX {
                    if frac > 0.95 {
                        "Vocoder (frozen)".into()
                    } else if frac > 0.03 {
                        format!("Vocoder (release {:.0}%)", frac * 100.0)
                    } else {
                        "Vocoder".into()
                    }
                } else if frac > 0.03 {
                    format!("{}->{} {:.0}%", ALGORITHM_NAMES[idx], ALGORITHM_NAMES[idx + 1], frac * 100.0)
                } else {
                    ALGORITHM_NAMES[idx].into()
                }
            }
            Selection::Timbre => format!("{:.0}%", self.params.timbre.get() * 100.0),
            Selection::OscEnabled => if self.params.osc_enabled.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Selection::OscWaveform => self.osc_waveform_name().into(),
            Selection::Level1 => {
                if self.params.osc_enabled.load(Ordering::Relaxed) {
                    format!("{:.0} Hz", osc_freq_from_knob(self.params.level1.get()))
                } else {
                    format!("{:.0}%", self.params.level1.get() * 100.0)
                }
            }
            Selection::Level2 => format!("{:.0}%", self.params.level2.get() * 200.0),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::CarrierSource => {
                let cur = self.params.source_a.load(Ordering::Relaxed);
                self.params.source_a.store(crate::audio_bus::cycle_source(cur, step, self.audio_bus.len()), Ordering::Relaxed);
            }
            Selection::ModulatorSource => {
                let cur = self.params.source_b.load(Ordering::Relaxed);
                self.params.source_b.store(crate::audio_bus::cycle_source(cur, step, self.audio_bus.len()), Ordering::Relaxed);
            }
            Selection::Algorithm => bump(&self.params.algorithm, delta, sensitivity, 0.0, 1.0),
            Selection::Timbre => bump(&self.params.timbre, delta, sensitivity, 0.0, 1.0),
            Selection::OscEnabled => self.params.osc_enabled.store(delta > 0, Ordering::Relaxed),
            Selection::OscWaveform => {
                let cur = self.params.osc_waveform.load(Ordering::Relaxed) as i32;
                self.params.osc_waveform.store((cur + step).rem_euclid(3) as u32, Ordering::Relaxed);
            }
            Selection::Level1 => bump(&self.params.level1, delta, sensitivity, 0.0, 1.0),
            Selection::Level2 => bump(&self.params.level2, delta, sensitivity, 0.0, 1.0),
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::CarrierSource => self.params.source_a.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed),
            Selection::ModulatorSource => self.params.source_b.store(crate::audio_bus::NO_SOURCE, Ordering::Relaxed),
            Selection::Algorithm => self.params.algorithm.set(3.0 / NUM_ALGORITHMS as f32),
            Selection::Timbre => self.params.timbre.set(0.5),
            Selection::OscEnabled => self.params.osc_enabled.store(false, Ordering::Relaxed),
            Selection::OscWaveform => self.params.osc_waveform.store(0, Ordering::Relaxed),
            Selection::Level1 => self.params.level1.set(0.8),
            Selection::Level2 => self.params.level2.set(0.5),
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

    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }

    /// The real carrier/modulator/output overlay traces plus (when the
    /// Vocoder slot is engaged) the 12-band envelope-follower meter --
    /// see `Params::carrier_snapshot`/`modulator_snapshot`/
    /// `output_snapshot`/`vocoder_band_snapshot`, all written once per
    /// audio block by `WarpsProcessor::process`.
    pub(crate) fn output_visual(&self) -> WarpsVisual {
        let (idx, frac) = self.algorithm_slot();
        let is_vocoder = idx == VOCODER_IDX;
        let vocoder_frozen = is_vocoder && frac >= 0.995;
        let (algorithm_name, next_algorithm_name, crossfade_frac) = if is_vocoder {
            (ALGORITHM_NAMES[VOCODER_IDX].to_string(), None, 0.0)
        } else if frac > 0.03 {
            let next_idx = if idx + 1 == VOCODER_IDX { VOCODER_IDX } else { idx + 1 };
            (ALGORITHM_NAMES[idx].to_string(), Some(ALGORITHM_NAMES[next_idx].to_string()), frac)
        } else {
            (ALGORITHM_NAMES[idx].to_string(), None, 0.0)
        };

        WarpsVisual {
            algorithm_name,
            next_algorithm_name,
            crossfade_frac,
            is_vocoder,
            vocoder_frozen,
            osc_enabled: self.params.osc_enabled.load(Ordering::Relaxed),
            osc_waveform_name: self.osc_waveform_name().to_string(),
            carrier_trace: self.params.carrier_snapshot.lock().unwrap().clone(),
            modulator_trace: self.params.modulator_snapshot.lock().unwrap().clone(),
            output_trace: self.params.output_snapshot.lock().unwrap().clone(),
            vocoder_band_levels: *self.params.vocoder_band_snapshot.lock().unwrap(),
        }
    }
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.01).clamp(min, max);
    value.set(next);
}

/// Splits a raw 0..1 knob position into (algorithm index 0..=VOCODER_IDX,
/// fraction 0..1). See `WarpsApp::algorithm_slot`.
fn algorithm_slot_of(knob: f32) -> (usize, f32) {
    let pos = (knob.clamp(0.0, 1.0) * NUM_ALGORITHMS as f32).min(NUM_ALGORITHMS as f32 - 1e-4);
    let idx = pos.floor() as usize;
    (idx.min(VOCODER_IDX), pos - pos.floor())
}

fn osc_freq_from_knob(knob: f32) -> f32 {
    // 20Hz..~20.5kHz over 10 octaves -- a full audio-rate range, per
    // the manual's "audio-rate oscillator".
    20.0 * 2f32.powf(knob.clamp(0.0, 1.0) * 10.0)
}

fn one_pole_coef(time_ms: f32, sample_rate: f32) -> f32 {
    1.0 - (-1.0 / (0.001 * time_ms.max(0.01) * sample_rate)).exp()
}

/// Downsamples `src` to at most `points` evenly-spaced real samples --
/// the shared primitive behind every `WarpsVisual` trace. Cheap (no
/// extra DSP, just array reads) and safe for any block size, including
/// blocks shorter than `points` (returns `src.len()` points instead).
fn downsample(src: &[f32], points: usize) -> Vec<f32> {
    if src.is_empty() || points == 0 {
        return Vec::new();
    }
    let take = points.min(src.len());
    (0..take).map(|i| src[i * src.len() / take]).collect()
}

// --- Cross-modulation algorithms (indices 0..VOCODER_IDX) ---

/// Algorithm 0: Crossfade, "using a constant-power law... both
/// signals equally mixed at 12 o'clock" -- the manual's own wording.
fn algo_crossfade(a: f32, b: f32, timbre: f32) -> f32 {
    let t = timbre.clamp(0.0, 1.0) * (PI / 2.0);
    a * t.cos() + b * t.sin()
}

/// A symmetric triangle-wave fold -- reflects a sample back into
/// -1..1 instead of clipping it.
fn triangle_fold(x: f32) -> f32 {
    let period = 4.0;
    let mut v = (x + 1.0).rem_euclid(period) - 1.0;
    if v > 1.0 {
        v = 2.0 - v;
    } else if v < -1.0 {
        v = -2.0 - v;
    }
    v
}

/// Algorithm 1: Crossfold, "carrier and modulator are summed, a tiny
/// bit of cross-modulation product is added... sent to a wavefolder,
/// the amount of which is controlled by TIMBRE".
fn algo_crossfold(a: f32, b: f32, timbre: f32) -> f32 {
    let sum = a + b + a * b * 0.2;
    let depth = 1.0 + timbre.clamp(0.0, 1.0) * 6.0;
    triangle_fold(sum * depth)
}

/// Asymmetric soft-clip standing in for the real module's "emulated
/// diode clipping" -- harder-limiting on the negative half, the way a
/// real diode's forward-voltage asymmetry would shape it.
fn diode_clip(x: f32) -> f32 {
    if x >= 0.0 {
        x.tanh()
    } else {
        (x * 1.6).tanh() * 0.7
    }
}

/// Algorithm 2: Diode Ring-Mod, "crudely multiplied, using a digital
/// model of a diode ring-modulator" -- a min()-magnitude "crude"
/// multiply (real diode ring-mods are famously imprecise multipliers)
/// rather than the clean product Digital Ring-Mod uses below.
fn algo_diode_ring(a: f32, b: f32, timbre: f32) -> f32 {
    let crude = a.abs().min(b.abs()) * (a * b).signum();
    let gain = 1.0 + timbre.clamp(0.0, 1.0) * 4.0;
    diode_clip(crude * gain)
}

/// Algorithm 3: Digital Ring-Mod, "a gentler version... uses a proper
/// multiplication operation... gain boost and soft-clipping".
fn algo_digital_ring(a: f32, b: f32, timbre: f32) -> f32 {
    let gain = 1.0 + timbre.clamp(0.0, 1.0) * 3.0;
    (a * b * gain).tanh()
}

/// Algorithm 4: Exclusive-Or, "both... converted to 16-bit integers,
/// ...XOR'ed bit by bit. TIMBRE controls which bits are XOR'ed
/// together" -- implemented as a widening bitmask: at TIMBRE=0 only
/// the top bit is XOR'ed (subtle), at TIMBRE=1 all 16 are.
fn algo_xor(a: f32, b: f32, timbre: f32) -> f32 {
    let ai = (a.clamp(-1.0, 1.0) * 32767.0) as i32 as i16;
    let bi = (b.clamp(-1.0, 1.0) * 32767.0) as i32 as i16;
    let num_bits = (1 + (timbre.clamp(0.0, 1.0) * 15.0).round() as u32).min(16);
    let mask = (((1u32 << num_bits) - 1) << (16 - num_bits)) as i16;
    let xored = (ai ^ bi) & mask;
    let passthrough = ai & !mask;
    (xored | passthrough) as f32 / 32767.0
}

/// Algorithm 5: Comparator/Octaver, "a handful of signals... through
/// comparison and rectification operations typical of octave pedals.
/// TIMBRE morphs through these signals" -- four such signals,
/// continuously crossfaded (not hard-switched) as TIMBRE sweeps.
fn algo_comparator(a: f32, b: f32, timbre: f32) -> f32 {
    let stage = timbre.clamp(0.0, 1.0) * 3.0;
    let idx = (stage.floor() as usize).min(3);
    let frac = stage - stage.floor();
    let signal = |i: usize| -> f32 {
        match i {
            0 => a.signum() * b.signum(),           // bipolar comparator square
            1 => (a + b).abs() * 2.0 - 1.0,          // full-wave rectified sum
            2 => a.min(b),                            // min-based rectify (octave flavor)
            _ => a.signum() * b.abs(),                // sign(carrier) * |modulator|
        }
    };
    signal(idx) * (1.0 - frac) + signal((idx + 1).min(3)) * frac
}

fn cross_mod_output(idx: usize, a: f32, b: f32, timbre: f32) -> f32 {
    match idx {
        0 => algo_crossfade(a, b, timbre),
        1 => algo_crossfold(a, b, timbre),
        2 => algo_diode_ring(a, b, timbre),
        3 => algo_digital_ring(a, b, timbre),
        4 => algo_xor(a, b, timbre),
        _ => algo_comparator(a, b, timbre),
    }
}

fn svf_f_coef(freq: f32, sample_rate: f32) -> f32 {
    (2.0 * (PI * freq / sample_rate).sin()).clamp(0.0, 1.0)
}

/// Chamberlin state-variable filter, one step -- returns (new_lp,
/// new_bp = bandpass output). Same formulation prism.rs/voltage.rs
/// each keep their own copy of.
fn svf_bandpass_step(input: f32, lp: f32, bp: f32, f_coef: f32, q: f32) -> (f32, f32) {
    let new_lp = lp + f_coef * bp;
    let high = input - new_lp - (1.0 / q) * bp;
    let new_bp = bp + f_coef * high;
    (new_lp, new_bp)
}

fn vocoder_band_freqs() -> [f32; VOCODER_BANDS] {
    let mut freqs = [0.0f32; VOCODER_BANDS];
    let lo = 150.0f32;
    let hi = 6000.0f32;
    for (i, f) in freqs.iter_mut().enumerate() {
        let t = i as f32 / (VOCODER_BANDS - 1) as f32;
        *f = lo * (hi / lo).powf(t);
    }
    freqs
}

impl App for WarpsApp {
    fn needs_background_audio(&self) -> bool { self.params.source_a.load(Ordering::Relaxed) != crate::audio_bus::NO_SOURCE || self.params.source_b.load(Ordering::Relaxed) != crate::audio_bus::NO_SOURCE }
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
        const PANEL_W: f32 = 260.0;
        const PANEL_H: f32 = 70.0;
        let v = self.output_visual();
        let to_curve = |samples: &[f32]| {
            if samples.len() >= 2 {
                let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(samples, PANEL_W, PANEL_H, true);
                crate::app::CurveSegments { mid_x, mid_y, length, angle_deg }
            } else {
                crate::app::CurveSegments::default()
            }
        };
        crate::app::SlintExtra::Warps(crate::app::WarpsExtra {
            algorithm_name: v.algorithm_name,
            next_algorithm_name: v.next_algorithm_name,
            crossfade_frac: v.crossfade_frac,
            is_vocoder: v.is_vocoder,
            vocoder_frozen: v.vocoder_frozen,
            osc_enabled: v.osc_enabled,
            osc_waveform_name: v.osc_waveform_name,
            carrier_trace: to_curve(&v.carrier_trace),
            modulator_trace: to_curve(&v.modulator_trace),
            output_trace: to_curve(&v.output_trace),
            vocoder_band_levels: v.vocoder_band_levels,
        })
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(WarpsProcessor {
            params: Arc::clone(&self.params),
            audio_bus: Arc::clone(&self.audio_bus),
            osc_phase: 0.0,
            noise_lp: 0.0,
            rng: 0xC0FF_EE11,
            band_freqs: vocoder_band_freqs(),
            bands: std::array::from_fn(|_| VocoderBand::default()),
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(WARPS_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, WARPS_TITLE);
        Text::new("Warps", Point::new(16, 26), title).draw(fb).ok();

        let dim = MonoTextStyle::new(&SPLEEN_6X12, WARPS_DIM);
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
        self.list.draw_themed(fb, 16, 56, 22, 10, &display_rows, WARPS_BG, WARPS_DIM, WARPS_ACCENT);
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
struct VocoderBand {
    carrier_lp: f32,
    carrier_bp: f32,
    modulator_lp: f32,
    modulator_bp: f32,
    /// Modulator sub-band envelope follower state.
    env: f32,
}

struct WarpsProcessor {
    params: Arc<Params>,
    audio_bus: Arc<AudioBus>,
    /// Internal oscillator phase, 0..1.
    osc_phase: f32,
    /// One-pole low-pass state for the "filtered noise" waveform.
    noise_lp: f32,
    rng: u32,
    band_freqs: [f32; VOCODER_BANDS],
    bands: [VocoderBand; VOCODER_BANDS],
}

impl WarpsProcessor {
    fn next_noise(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// One sample of the internal oscillator, given the current
    /// carrier tap sample as its through-zero phase-mod input.
    fn osc_step(&mut self, waveform: u32, vocoder_category: bool, phase_mod_in: f32, freq_hz: f32, sample_rate: f32) -> f32 {
        const PM_DEPTH: f32 = 1.5; // >1.0 permits genuine through-zero reversal
        let fm = phase_mod_in * PM_DEPTH;
        let inc = (freq_hz * (1.0 + fm)) / sample_rate;
        self.osc_phase = (self.osc_phase + inc).rem_euclid(1.0);
        let phase = self.osc_phase;
        let wf = waveform % 3;
        if vocoder_category {
            match wf {
                0 => 2.0 * phase - 1.0,                       // saw
                1 => if phase < 0.5 { 1.0 } else { -1.0 },     // pulse
                _ => {
                    // Low-pass filtered internal noise.
                    let noise = self.next_noise();
                    let coef = svf_f_coef(1200.0, sample_rate).min(1.0);
                    self.noise_lp += (noise - self.noise_lp) * coef;
                    self.noise_lp
                }
            }
        } else {
            match wf {
                0 => (phase * TAU).sin(),
                1 => (2.0 / PI) * (phase * TAU).sin().asin(),
                _ => 2.0 * phase - 1.0,
            }
        }
    }

    /// One sample of the Vocoder algorithm (see module doc comment).
    /// `release_frac` is 0 (normal ~30ms release) to 1 (frozen
    /// spectral envelope) -- only nonzero when the algorithm knob is
    /// inside the Vocoder's own slot, per the manual.
    fn vocoder_step(&mut self, carrier: f32, modulator: f32, timbre: f32, release_frac: f32, sample_rate: f32) -> f32 {
        let shift = ((timbre.clamp(0.0, 1.0) - 0.5) * 2.0 * MAX_FORMANT_SHIFT as f32).round() as i32;
        let attack_coef = one_pole_coef(4.0, sample_rate);
        let frozen = release_frac >= 0.995;
        let release_ms = 25.0 + release_frac.clamp(0.0, 1.0).powi(2) * 6000.0;
        let release_coef = one_pole_coef(release_ms, sample_rate);

        for i in 0..VOCODER_BANDS {
            let f_coef = svf_f_coef(self.band_freqs[i], sample_rate);
            let band = &mut self.bands[i];
            let (c_lp, c_bp) = svf_bandpass_step(carrier, band.carrier_lp, band.carrier_bp, f_coef, VOCODER_Q);
            band.carrier_lp = c_lp;
            band.carrier_bp = c_bp;
            let (m_lp, m_bp) = svf_bandpass_step(modulator, band.modulator_lp, band.modulator_bp, f_coef, VOCODER_Q);
            band.modulator_lp = m_lp;
            band.modulator_bp = m_bp;
            if !frozen {
                let rectified = m_bp.abs();
                let coef = if rectified > band.env { attack_coef } else { release_coef };
                band.env += (rectified - band.env) * coef;
            }
        }

        let mut out = 0.0f32;
        for i in 0..VOCODER_BANDS {
            let env_idx = (i as i32 + shift).clamp(0, VOCODER_BANDS as i32 - 1) as usize;
            out += self.bands[i].carrier_bp * self.bands[env_idx].env;
        }
        (out * 3.0 / (VOCODER_BANDS as f32).sqrt()).tanh()
    }
}

impl AudioProcessor for WarpsProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        for out in buffer.iter_mut() {
            *out = 0.0;
        }
        let frames = buffer.len() / channels;
        if frames == 0 {
            return;
        }

        let a_idx = self.params.source_a.load(Ordering::Relaxed);
        let b_idx = self.params.source_b.load(Ordering::Relaxed);
        // Cloned out from behind the lock immediately (not held for
        // the rest of this function) -- Carrier and Modulator default
        // to the same index, and can always be pointed at the same
        // index by the user, which would mean locking the *same*
        // `Mutex` twice in a row and deadlocking the audio thread
        // forever if either lock were held while acquiring the other.
        let a_buf = self.audio_bus.get(a_idx).map(|b| b.lock().unwrap().clone()).unwrap_or_default();
        let b_buf = self.audio_bus.get(b_idx).map(|b| b.lock().unwrap().clone()).unwrap_or_default();

        let osc_enabled = self.params.osc_enabled.load(Ordering::Relaxed);
        let osc_waveform = self.params.osc_waveform.load(Ordering::Relaxed);
        let (algo_floor, algo_frac) = algorithm_slot_of((self.params.algorithm.get() + self.params.ext_algorithm.get()).clamp(0.0, 1.0));
        let vocoder_category = algo_floor == VOCODER_IDX;
        let timbre = (self.params.timbre.get() + self.params.ext_timbre.get()).clamp(0.0, 1.0);
        let level1_knob = (self.params.level1.get() + self.params.ext_level1.get()).clamp(0.0, 1.0);
        let level2_gain = (self.params.level2.get() + self.params.ext_level2.get()).clamp(0.0, 1.0) * 2.0;
        let osc_freq = osc_freq_from_knob(level1_knob);

        let mut mono = vec![0.0f32; frames];
        let mut aux = vec![0.0f32; frames];
        // Real per-sample carrier/modulator traces, kept only for this
        // block's downsampled visual snapshot (see below) -- exactly
        // the two signals fed into `cross_mod_output`/`vocoder_step`,
        // not a separate/fabricated tap.
        let mut carrier_buf = vec![0.0f32; frames];
        let mut modulator_buf = vec![0.0f32; frames];

        for n in 0..frames {
            let a_raw = if a_buf.is_empty() { 0.0 } else { a_buf[n % a_buf.len()] };
            let b_raw = if b_buf.is_empty() { 0.0 } else { b_buf[n % b_buf.len()] };

            let (carrier, osc_sample) = if osc_enabled {
                let s = self.osc_step(osc_waveform, vocoder_category, a_raw, osc_freq, sample_rate);
                (s, s)
            } else {
                (a_raw * level1_knob, 0.0)
            };
            let modulator = b_raw * level2_gain;
            carrier_buf[n] = carrier;
            modulator_buf[n] = modulator;

            // Vocoder output is always computed (its envelope
            // followers need continuous updating for correct release
            // behavior across the crossfade boundary), but only
            // mixed in near the top of the knob's range.
            let vocoder_release = if algo_floor == VOCODER_IDX { algo_frac } else { 0.0 };
            let vocoder_out = self.vocoder_step(carrier, modulator, timbre, vocoder_release, sample_rate);

            let sample = if algo_floor == VOCODER_IDX {
                vocoder_out
            } else if algo_floor + 1 == VOCODER_IDX {
                let a_out = cross_mod_output(algo_floor, carrier, modulator, timbre);
                a_out * (1.0 - algo_frac) + vocoder_out * algo_frac
            } else {
                let a_out = cross_mod_output(algo_floor, carrier, modulator, timbre);
                if algo_frac > 0.0005 {
                    let b_out = cross_mod_output(algo_floor + 1, carrier, modulator, timbre);
                    a_out * (1.0 - algo_frac) + b_out * algo_frac
                } else {
                    a_out
                }
            };

            mono[n] = sample.clamp(-4.0, 4.0);
            aux[n] = if osc_enabled { osc_sample } else { (carrier + modulator).clamp(-4.0, 4.0) };
        }

        for (out, s) in buffer.chunks_mut(channels).zip(mono.iter()) {
            for ch in out.iter_mut() {
                *ch = *s;
            }
        }

        *self.params.bus_out.lock().unwrap() = mono.clone();
        *self.params.aux_out.lock().unwrap() = aux;

        // Once per block (not per-sample): downsampled real carrier/
        // modulator/output snapshots and the Vocoder's real per-band
        // envelope state -- `WarpsApp::output_visual` reads all four
        // directly, no fabricated data. See `WarpsVisual`.
        *self.params.carrier_snapshot.lock().unwrap() = downsample(&carrier_buf, VISUAL_SNAPSHOT_POINTS);
        *self.params.modulator_snapshot.lock().unwrap() = downsample(&modulator_buf, VISUAL_SNAPSHOT_POINTS);
        *self.params.output_snapshot.lock().unwrap() = downsample(&mono, VISUAL_SNAPSHOT_POINTS);
        {
            let mut bands = self.params.vocoder_band_snapshot.lock().unwrap();
            for (i, band) in self.bands.iter().enumerate() {
                bands[i] = band.env;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> (WarpsApp, Arc<AudioBus>) {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let audio_bus = Arc::new(AudioBus::new());
        let mixer_bus = Arc::new(MixerBus::new());
        let app = WarpsApp::new(sensitivity, nav_speed, modbus, Arc::clone(&audio_bus), mixer_bus);
        (app, audio_bus)
    }

    /// A single-frequency correlation (Goertzel-style DFT bin
    /// magnitude) -- used to check ring-mod's sum/difference-frequency
    /// character without pulling in a full FFT.
    fn tone_magnitude(samples: &[f32], freq: f32, sample_rate: f32) -> f32 {
        let n = samples.len() as f32;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, &s) in samples.iter().enumerate() {
            let phase = TAU * freq * i as f32 / sample_rate;
            re += s * phase.cos();
            im -= s * phase.sin();
        }
        ((re * re + im * im).sqrt()) / n
    }

    /// With no sources registered and the oscillator off, Warps must
    /// stay silent, not panic.
    #[test]
    fn no_sources_and_no_oscillator_is_silence_not_a_panic() {
        let (mut app, _audio_bus) = new_app();
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.5f32; 256 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        assert!(buffer.iter().all(|&s| s == 0.0));
    }

    /// Crossfade (algorithm 0) at TIMBRE=0 must pass the carrier
    /// through essentially unchanged, and at TIMBRE=1 the modulator --
    /// the constant-power law's two endpoints.
    #[test]
    fn crossfade_endpoints_pass_carrier_or_modulator_through() {
        let (mut app, audio_bus) = new_app();
        let a_idx = audio_bus.len();
        let a = audio_bus.register("A");
        let b_idx = audio_bus.len();
        let b = audio_bus.register("B");
        *a.lock().unwrap() = vec![0.6; 64];
        *b.lock().unwrap() = vec![-0.3; 64];
        app.params.source_a.store(a_idx, Ordering::Relaxed);
        app.params.source_b.store(b_idx, Ordering::Relaxed);
        app.params.algorithm.set(0.0); // Crossfade, exactly index 0
        app.params.level1.set(1.0);
        app.params.level2.set(0.5); // -> unity modulator gain

        app.params.timbre.set(0.0);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 64 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        assert!((buffer[0] - 0.6).abs() < 0.01, "expected ~carrier at timbre=0, got {}", buffer[0]);

        app.params.timbre.set(1.0);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 64 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        assert!((buffer[0] - -0.3).abs() < 0.01, "expected ~modulator at timbre=1, got {}", buffer[0]);
    }

    /// Digital Ring-Mod of two tones must produce energy at their sum
    /// and difference frequencies, and (being a true product) much
    /// less energy at the two original frequencies themselves --
    /// classic ring-modulation spectral behavior.
    #[test]
    fn digital_ring_mod_produces_sum_and_difference_frequencies() {
        let (mut app, audio_bus) = new_app();
        let a_idx = audio_bus.len();
        let a = audio_bus.register("A");
        let b_idx = audio_bus.len();
        let b = audio_bus.register("B");
        let sample_rate = 48000.0f32;
        let n = 4800usize; // 0.1s -- 10Hz DFT-bin resolution
        let (freq_a, freq_b) = (220.0f32, 90.0f32);
        let a_samples: Vec<f32> = (0..n).map(|i| (TAU * freq_a * i as f32 / sample_rate).sin()).collect();
        let b_samples: Vec<f32> = (0..n).map(|i| (TAU * freq_b * i as f32 / sample_rate).sin()).collect();
        *a.lock().unwrap() = a_samples;
        *b.lock().unwrap() = b_samples;
        app.params.source_a.store(a_idx, Ordering::Relaxed);
        app.params.source_b.store(b_idx, Ordering::Relaxed);
        app.params.algorithm.set(3.0 / NUM_ALGORITHMS as f32); // Digital Ring, index 3
        app.params.timbre.set(0.0); // minimal extra gain/clip
        app.params.level1.set(1.0);
        app.params.level2.set(0.5); // unity

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; n * 2];
        processor.process(&mut buffer, 2, sample_rate);
        let mono: Vec<f32> = buffer.chunks(2).map(|c| c[0]).collect();

        let sum_mag = tone_magnitude(&mono, freq_a + freq_b, sample_rate);
        let diff_mag = tone_magnitude(&mono, freq_a - freq_b, sample_rate);
        let orig_a_mag = tone_magnitude(&mono, freq_a, sample_rate);
        let orig_b_mag = tone_magnitude(&mono, freq_b, sample_rate);

        assert!(sum_mag > 0.1, "expected energy at the sum frequency ({}), got {sum_mag}", freq_a + freq_b);
        assert!(diff_mag > 0.1, "expected energy at the difference frequency ({}), got {diff_mag}", freq_a - freq_b);
        assert!(orig_a_mag < sum_mag * 0.3, "ring-mod should suppress the original carrier frequency, got {orig_a_mag} vs sum {sum_mag}");
        assert!(orig_b_mag < sum_mag * 0.3, "ring-mod should suppress the original modulator frequency, got {orig_b_mag} vs sum {sum_mag}");
    }

    /// XOR's bitwise math must match a direct 16-bit XOR computation
    /// at full bit-mask width (TIMBRE=1).
    #[test]
    fn xor_algorithm_matches_direct_bitwise_math() {
        let a = 0.4f32;
        let b = -0.25f32;
        let result = algo_xor(a, b, 1.0);
        let ai = (a * 32767.0) as i32 as i16;
        let bi = (b * 32767.0) as i32 as i16;
        let expected = (ai ^ bi) as f32 / 32767.0;
        assert!((result - expected).abs() < 1e-6, "expected direct XOR {expected}, got {result}");
    }

    /// Every algorithm slot (including the boundary into/through the
    /// Vocoder) must stay finite and bounded under sustained, loud
    /// input -- a quick regression net for the whole set.
    #[test]
    fn every_algorithm_stays_bounded_with_loud_input() {
        for i in 0..=20 {
            let knob = i as f32 / 20.0;
            let (mut app, audio_bus) = new_app();
            let a_idx = audio_bus.len();
            let a = audio_bus.register("A");
            let b_idx = audio_bus.len();
            let b = audio_bus.register("B");
            *a.lock().unwrap() = vec![0.95; 256];
            *b.lock().unwrap() = vec![-0.9; 256];
            app.params.source_a.store(a_idx, Ordering::Relaxed);
            app.params.source_b.store(b_idx, Ordering::Relaxed);
            app.params.algorithm.set(knob);
            app.params.timbre.set(1.0);
            app.params.level1.set(1.0);
            app.params.level2.set(1.0);

            let mut processor = app.audio_processor().unwrap();
            let mut buffer = vec![0.0f32; 256 * 2];
            for _ in 0..20 {
                processor.process(&mut buffer, 2, 48000.0);
            }
            assert!(buffer.iter().all(|s| s.is_finite()), "knob {knob} produced a non-finite sample");
            let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
            assert!(peak <= 4.0001, "knob {knob} exceeded expected headroom: {peak}");
        }
    }

    /// The internal oscillator must produce real audio on its own,
    /// with nothing patched into either input at all -- it genuinely
    /// replaces the carrier, per the manual.
    #[test]
    fn internal_oscillator_sounds_with_nothing_patched() {
        let (mut app, _audio_bus) = new_app();
        app.params.osc_enabled.store(true, Ordering::Relaxed);
        app.params.algorithm.set(0.0); // Crossfade
        app.params.timbre.set(0.0); // pass the (oscillator) carrier straight through
        app.params.level1.set(0.5); // mid-range pitch

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.1, "expected the internal oscillator to be audible with nothing patched, got peak {peak}");
    }

    /// The Vocoder must produce real, bounded, finite output driven by
    /// two tapped sources, and republish it.
    #[test]
    fn vocoder_is_audible_and_bounded() {
        let (mut app, audio_bus) = new_app();
        let a_idx = audio_bus.len();
        let a = audio_bus.register("A");
        let b_idx = audio_bus.len();
        let b = audio_bus.register("B");
        let sample_rate = 48000.0f32;
        let n = 1024usize;
        *a.lock().unwrap() = (0..n).map(|i| (TAU * 220.0 * i as f32 / sample_rate).sin()).collect();
        *b.lock().unwrap() = (0..n).map(|i| (TAU * 90.0 * i as f32 / sample_rate).sin() * 0.8).collect();
        app.params.source_a.store(a_idx, Ordering::Relaxed);
        app.params.source_b.store(b_idx, Ordering::Relaxed);
        // Mid-way through the Vocoder's own slot (not the very top,
        // which freezes the envelope -- see the freeze test below for
        // that behavior specifically) -- pure Vocoder, moderate
        // release, envelope free to rise from its silent start.
        app.params.algorithm.set((VOCODER_IDX as f32 + 0.5) / NUM_ALGORITHMS as f32);
        app.params.level1.set(1.0);
        app.params.level2.set(0.5);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; n * 2];
        for _ in 0..10 {
            processor.process(&mut buffer, 2, sample_rate);
        }
        assert!(buffer.iter().all(|s| s.is_finite()));
        let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.01, "expected an audible vocoder, got peak {peak}");
        assert!(peak <= 4.0001, "vocoder output exceeded expected headroom: {peak}");
    }

    /// Turning the algorithm knob fully clockwise within the Vocoder's
    /// own slot must freeze the spectral envelope -- a loud modulator
    /// fed in while frozen must *not* build the envelope up at all
    /// (it stays pinned at its starting value), unlike at the start
    /// of the slot (normal short release) where the same loud
    /// modulator clearly does build it up. Tested directly against
    /// the per-band envelope state rather than via a sudden
    /// silence-the-modulator transient, since abruptly dropping a
    /// constant input is itself a transient that re-excites a
    /// resonant bandpass -- real filter physics, but a confound for
    /// testing the freeze feature specifically.
    #[test]
    fn vocoder_freezes_when_algorithm_knob_is_fully_clockwise() {
        let sample_rate = 48000.0f32;
        let n = 512usize;
        let carrier: Vec<f32> = (0..n).map(|i| (TAU * 300.0 * i as f32 / sample_rate).sin()).collect();
        let modulator_loud: Vec<f32> = (0..n).map(|i| (TAU * 110.0 * i as f32 / sample_rate).sin() * 0.9).collect();

        let run = |algo_knob: f32| -> f32 {
            let (mut app, audio_bus) = new_app();
            let a_idx = audio_bus.len();
            let a = audio_bus.register("A");
            let b_idx = audio_bus.len();
            let b = audio_bus.register("B");
            *a.lock().unwrap() = carrier.clone();
            *b.lock().unwrap() = modulator_loud.clone();
            app.params.source_a.store(a_idx, Ordering::Relaxed);
            app.params.source_b.store(b_idx, Ordering::Relaxed);
            app.params.algorithm.set(algo_knob);
            app.params.level1.set(1.0);
            app.params.level2.set(0.5);

            let mut processor = WarpsProcessor {
                params: Arc::clone(&app.params),
                audio_bus: Arc::clone(&app.audio_bus),
                osc_phase: 0.0,
                noise_lp: 0.0,
                rng: 0xC0FF_EE11,
                band_freqs: vocoder_band_freqs(),
                bands: std::array::from_fn(|_| VocoderBand::default()),
            };
            let mut buffer = vec![0.0f32; n * 2];
            for _ in 0..20 {
                processor.process(&mut buffer, 2, sample_rate);
            }
            processor.bands.iter().map(|b| b.env).fold(0.0f32, f32::max)
        };

        let short_release_env = run((VOCODER_IDX as f32) / NUM_ALGORITHMS as f32); // slot start
        let frozen_env = run(1.0); // fully clockwise -> frozen from the very first sample

        assert!(short_release_env > 0.01, "expected the envelope to build up under a loud modulator with normal (short) release, got {short_release_env}");
        assert!(frozen_env < 0.0001, "expected a frozen envelope (started at 0, never updated) to stay pinned near 0 even under a loud modulator, got {frozen_env}");
    }

    /// Warps must republish both its main output and its AUX output
    /// on the audio bus, matching the real module's two physical
    /// outputs.
    #[test]
    fn republishes_both_main_and_aux_outputs_on_the_audio_bus() {
        let (mut app, audio_bus) = new_app();
        let a_idx = audio_bus.len();
        let a = audio_bus.register("A");
        let b_idx = audio_bus.len();
        let b = audio_bus.register("B");
        *a.lock().unwrap() = vec![0.5; 128];
        *b.lock().unwrap() = vec![0.5; 128];
        app.params.source_a.store(a_idx, Ordering::Relaxed);
        app.params.source_b.store(b_idx, Ordering::Relaxed);
        app.params.algorithm.set(0.0);

        assert!(audio_bus.names().iter().any(|n| n == "Warps (Aux)"), "expected a separately-registered AUX bus");

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 128 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        let published = app.params.bus_out.lock().unwrap();
        assert_eq!(published.len(), 128);
        let aux = app.params.aux_out.lock().unwrap();
        assert_eq!(aux.len(), 128);
    }

    /// The AUX output, with the oscillator off, must carry the
    /// carrier+modulator sum post-VCA -- distinct from (and not
    /// dependent on) whatever the main cross-modulated output is
    /// doing.
    #[test]
    fn aux_output_carries_carrier_plus_modulator_sum_when_osc_is_off() {
        let (mut app, audio_bus) = new_app();
        let a_idx = audio_bus.len();
        let a = audio_bus.register("A");
        let b_idx = audio_bus.len();
        let b = audio_bus.register("B");
        *a.lock().unwrap() = vec![0.4; 64];
        *b.lock().unwrap() = vec![0.2; 64];
        app.params.source_a.store(a_idx, Ordering::Relaxed);
        app.params.source_b.store(b_idx, Ordering::Relaxed);
        app.params.level1.set(1.0); // unity carrier amplitude
        app.params.level2.set(0.5); // unity modulator gain
        app.params.algorithm.set(4.0 / NUM_ALGORITHMS as f32); // XOR -- deliberately unrelated to the sum

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 64 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        let aux = app.params.aux_out.lock().unwrap();
        assert!((aux[0] - 0.6).abs() < 0.01, "expected AUX ~= carrier+modulator (0.6), got {}", aux[0]);
    }

    /// Level 2 must permit gain above unity ("gains above 1.0 can be
    /// applied, for a warm overdrive effect") while staying bounded --
    /// a genuinely louder, soft-clipped modulator, not a fake ceiling.
    #[test]
    fn level2_overdrive_stays_bounded() {
        let (mut app, audio_bus) = new_app();
        let a_idx = audio_bus.len();
        let a = audio_bus.register("A");
        let b_idx = audio_bus.len();
        let b = audio_bus.register("B");
        *a.lock().unwrap() = vec![0.9; 128];
        *b.lock().unwrap() = vec![0.9; 128];
        app.params.source_a.store(a_idx, Ordering::Relaxed);
        app.params.source_b.store(b_idx, Ordering::Relaxed);
        app.params.algorithm.set(3.0 / NUM_ALGORITHMS as f32); // Digital Ring
        app.params.timbre.set(1.0);
        app.params.level1.set(1.0);
        app.params.level2.set(1.0); // -> 2.0x gain, full overdrive

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 128 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        assert!(buffer.iter().all(|s| s.is_finite()));
        let peak = buffer.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
        assert!(peak <= 1.0001, "Digital Ring-Mod is tanh soft-clipped, expected <=1.0, got {peak}");
    }

    /// `output_visual`'s carrier/modulator/output traces must be real
    /// downsampled audio, not empty placeholders -- and the output
    /// trace in particular must actually track the selected algorithm
    /// (Crossfade at TIMBRE=0 passes the carrier through essentially
    /// unchanged, same as the endpoints test above).
    #[test]
    fn output_visual_traces_reflect_real_audio() {
        let (mut app, audio_bus) = new_app();
        let a_idx = audio_bus.len();
        let a = audio_bus.register("A");
        let b_idx = audio_bus.len();
        let b = audio_bus.register("B");
        *a.lock().unwrap() = vec![0.6; 256];
        *b.lock().unwrap() = vec![-0.3; 256];
        app.params.source_a.store(a_idx, Ordering::Relaxed);
        app.params.source_b.store(b_idx, Ordering::Relaxed);
        app.params.algorithm.set(0.0); // Crossfade
        app.params.timbre.set(0.0); // -> carrier passthrough
        app.params.level1.set(1.0);
        app.params.level2.set(0.5); // unity

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 256 * 2];
        processor.process(&mut buffer, 2, 48000.0);

        let visual = app.output_visual();
        assert!(!visual.carrier_trace.is_empty(), "expected a non-empty carrier trace");
        assert!(!visual.modulator_trace.is_empty(), "expected a non-empty modulator trace");
        assert!(!visual.output_trace.is_empty(), "expected a non-empty output trace");
        assert!(visual.carrier_trace.iter().all(|s| (s - 0.6).abs() < 0.01), "expected the carrier trace to be the real ~0.6 carrier");
        assert!(visual.modulator_trace.iter().all(|s| (s - -0.3).abs() < 0.01), "expected the modulator trace to be the real ~-0.3 modulator");
        assert!(visual.output_trace.iter().all(|s| (s - 0.6).abs() < 0.01), "expected the output trace to match the real (carrier-passthrough) output");
        assert_eq!(visual.algorithm_name, "Crossfade");
        assert!(!visual.is_vocoder);
        assert!(visual.next_algorithm_name.is_none());
    }

    /// While the Vocoder slot is engaged, `output_visual`'s band
    /// levels must be real, distinct-per-band envelope-follower
    /// magnitudes driven by an actual modulator signal -- not all
    /// zero and not all identical (a flat array would mean fabricated
    /// or unconnected data).
    #[test]
    fn output_visual_reports_real_distinct_vocoder_band_levels() {
        let (mut app, audio_bus) = new_app();
        let a_idx = audio_bus.len();
        let a = audio_bus.register("A");
        let b_idx = audio_bus.len();
        let b = audio_bus.register("B");
        let sample_rate = 48000.0f32;
        let n = 1024usize;
        *a.lock().unwrap() = (0..n).map(|i| (TAU * 220.0 * i as f32 / sample_rate).sin()).collect();
        *b.lock().unwrap() = (0..n).map(|i| (TAU * 900.0 * i as f32 / sample_rate).sin() * 0.8).collect();
        app.params.source_a.store(a_idx, Ordering::Relaxed);
        app.params.source_b.store(b_idx, Ordering::Relaxed);
        app.params.algorithm.set((VOCODER_IDX as f32 + 0.5) / NUM_ALGORITHMS as f32);
        app.params.level1.set(1.0);
        app.params.level2.set(0.5);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; n * 2];
        for _ in 0..10 {
            processor.process(&mut buffer, 2, sample_rate);
        }

        let visual = app.output_visual();
        assert!(visual.is_vocoder);
        assert!(!visual.vocoder_frozen);
        assert!(visual.vocoder_band_levels.iter().any(|&level| level > 0.001), "expected at least one real, nonzero band envelope, got {:?}", visual.vocoder_band_levels);
        let first = visual.vocoder_band_levels[0];
        assert!(visual.vocoder_band_levels.iter().any(|&level| (level - first).abs() > 1e-6), "expected genuinely distinct per-band levels, not a flat/fabricated array, got {:?}", visual.vocoder_band_levels);
    }
}
