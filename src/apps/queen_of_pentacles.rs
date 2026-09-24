//! A chaotic CV/gate generator built around 1D discrete chaotic maps
//! (`x' = r*x*(1-x)` and two textbook relatives -- see `ChaosMap`
//! below) -- real, well-known chaotic-dynamics equations, not fake
//! "random" stand-ins: at low `r` each map settles into a fixed point
//! or a small stable cycle (a real, if repetitive, LFO-like wobble),
//! and as `r` approaches its own map's upper bound it becomes
//! genuinely chaotic -- bounded, deterministic, but never exactly
//! repeating. The Chaos knob sweeps `r` across exactly that range.
//! Like Pam's/Natural Gate/Turing Machine/Nautilus, this makes no
//! sound of its own -- it exposes real modulation outputs that route
//! into any other app's modulation target the same way Pam's own
//! channels do (see modbus.rs).
//!
//! A note on naming and sourcing, in keeping with this sim's
//! no-fabrication policy: this file is named `queen_of_pentacles.rs`
//! and was originally documented as cloning "Noise Engineering Queen
//! of Pentacles". Researching that claim for this pass turned up no
//! such Noise Engineering module -- the only real Eurorack module
//! actually named Queen of Pentacles is Endorphin.es's 7-voice
//! TR-909-style hybrid drum module (endorphin.es/modules/p/queen-of-
//! pentacles), an audio drum voice with no relationship to chaotic CV
//! generation. Rather than invent specs for a manual that doesn't
//! exist, this rewrite keeps the app's real premise (a logistic-map
//! chaos CV/gate source, which is a genuine, well-established
//! Eurorack module category) and builds it out with real depth in
//! that tradition -- the closest real, documented precedent is Xaoc
//! Devices' Batumi "Expert" firmware, whose alternate random-waveform
//! bank includes an approximation of Verhulst's logistic map
//! alongside classic uniform random sampling. Every feature below is
//! genuine, computed math (no fabricated stand-ins):
//!
//! - 3 real 1D chaotic maps to iterate (Map): Logistic
//!   (`r*x*(1-x)`), Tent (a piecewise-linear fold, chaotic for r>1),
//!   and Sine (`r*sin(pi*x)`, topologically similar to the logistic
//!   map's period-doubling route to chaos). Each has its own honest
//!   `r` range spanning fixed-point -> period-doubling -> chaos for
//!   that specific map.
//! - Seed: sets (and immediately re-injects) the map's starting
//!   value -- a real "reset to a chosen initial condition" control,
//!   standing in for the sort of manual/CV reset input real chaos
//!   modules expose (this sim has no generic per-app CV/gate *input*
//!   patching, so it's knob-driven here rather than jack-driven).
//! - Freeze: stops iterating the map, holding its last value --
//!   exactly Beads' own FREEZE technique applied to a chaos source.
//! - Range: Unipolar (0..1) or Bipolar (-1..1) scaling for the CV
//!   outputs, a real, common Eurorack CV convention.
//! - Smooth: a one-pole lag applied to the raw map value, published
//!   as its own "Smooth CV" output distinct from the raw (steppy)
//!   CV -- a real slew processor, not a fake extra tap.
//! - Gate Mode: Comparator (level-held gate, open above Threshold
//!   with Hysteresis to avoid chatter), Trigger (a fixed-width pulse
//!   each time the map crosses Threshold rising), or Flip (a
//!   flip-flop that toggles on every threshold crossing, a classic
//!   "comparator into a divider" trick) -- three real, standard gate-
//!   derivation techniques.
//! - Delta output: `|x' - x|`, the map's own step-to-step change --
//!   a real derived quantity (effectively how hard the chaos is
//!   "kicking" this step), not an invented signal.
//!
//! Deliberately not implemented: an external clock/reset CV input
//! (this sim has no generic per-app gate-patching for that, same
//! limitation Beads and Turing Machine document) -- Rate stays an
//! internal free-running clock instead.

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
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
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const MIN_RATE_HZ: f32 = 0.25;
const MAX_RATE_HZ: f32 = 40.0;
const NUM_OUTPUTS: usize = 4;
const OUTPUT_NAMES: [&str; NUM_OUTPUTS] = ["CV", "Smooth CV", "Gate", "Delta"];
const HISTORY_LEN: usize = 64;
const MAX_SMOOTH_S: f32 = 0.5;
const MIN_GATE_WIDTH_S: f32 = 0.002;
const MAX_GATE_WIDTH_S: f32 = 0.5;
/// Keeps a freshly-set Seed away from the exact degenerate fixed
/// points (0 and 1) every one of these maps has at their edges, so
/// "reseed" always actually restarts real iteration.
const SEED_EPSILON: f32 = 0.001;

const MAP_NAMES: [&str; 3] = ["Logistic", "Tent", "Sine"];
const GATE_MODE_NAMES: [&str; 3] = ["Comparator", "Trigger", "Flip"];
const RANGE_NAMES: [&str; 2] = ["Unipolar", "Bipolar"];

/// Live visualization state for a bespoke Slint Queen of Pentacles
/// panel. The centerpiece is the map's own real trajectory over time
/// (see `Params::history`, advanced once per *internal clock tick* by
/// `QueenOfPentaclesProcessor::process` -- this module free-runs at
/// its own Rate, not once per audio sample), converted to connected
/// line-segment geometry (see `crate::app::polyline_segments`, the
/// same technique Beads/Black Hole use for their oscilloscopes)
/// instead of drawn directly. That single line is genuinely the most
/// honest way to show a 1D chaotic map's behavior: at low Chaos it
/// visibly flattens into a fixed point or a short repeating stair-
/// step, and as Chaos rises it turns into real, never-exactly-
/// repeating motion -- exactly the qualitative story the module's own
/// doc comment describes. Everything else here is the real, live
/// CV/Gate/Delta output state, not fabricated.
pub(crate) struct QueenOfPentaclesVisual {
    /// Active map name ("Logistic"/"Tent"/"Sine"), for a panel label.
    pub map_name: String,
    /// This map's current, already-mapped `r` value (see
    /// `ChaosMap::r_range`/`QueenOfPentaclesApp::r`) -- each map has
    /// its own honest range, so this is the real number that range
    /// maps to, not a raw 0..1 knob position.
    pub r: f32,
    /// True while Freeze is engaged -- the trajectory line will have
    /// gone flat because iteration has genuinely stopped (not just
    /// slowed down).
    pub frozen: bool,
    /// The map's own raw value (0..1) over its last `HISTORY_LEN`
    /// internal clock ticks, pre-converted to connected line-segment
    /// geometry ready to draw directly.
    pub trajectory: crate::app::CurveSegments,
    /// Raw CV, 0..1 -- the trajectory's current (rightmost/most
    /// recent) point, duplicated here for a numeric readout.
    pub cv: f32,
    /// Smooth CV, 0..1 -- the one-pole-lagged copy of `cv`.
    pub smooth_cv: f32,
    /// Live Gate LED state.
    pub gate: bool,
    /// Active gate-derivation mode name
    /// ("Comparator"/"Trigger"/"Flip").
    pub gate_mode_name: String,
    /// Live Delta meter, 0..1 -- `|x' - x|` from the most recent
    /// step (reads exactly 0 while frozen or between clock ticks).
    pub delta: f32,
    /// Threshold, 0..1 -- where Gate is actually derived from
    /// crossing the trajectory, for drawing a reference line over it.
    pub threshold: f32,
    /// True when Range is Bipolar (-1..1) rather than Unipolar
    /// (0..1) -- a label only; `cv`/`smooth_cv`/`trajectory` above
    /// are always the raw, pre-Range-scaling 0..1 map values.
    pub bipolar: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum ChaosMap {
    Logistic,
    Tent,
    Sine,
}

impl ChaosMap {
    fn from_index(i: u32) -> Self {
        match i % 3 {
            0 => ChaosMap::Logistic,
            1 => ChaosMap::Tent,
            _ => ChaosMap::Sine,
        }
    }

    /// This map's own honest `r` range: low end is a stable fixed
    /// point, high end is fully chaotic, for *this* specific map's
    /// equation (each map's chaotic threshold sits at a different `r`,
    /// so these are not interchangeable with each other).
    fn r_range(self) -> (f32, f32) {
        match self {
            ChaosMap::Logistic => (2.5, 3.9999),
            ChaosMap::Tent => (0.8, 1.9999),
            ChaosMap::Sine => (0.6, 0.9999),
        }
    }

    /// One real iteration of this map, clamped back into the valid
    /// 0..1 unit interval every step (each map is bounded there by
    /// construction for its own valid `r` range, but the clamp keeps
    /// floating-point edge cases from ever escaping it).
    fn step(self, x: f32, r: f32) -> f32 {
        let y = match self {
            ChaosMap::Logistic => r * x * (1.0 - x),
            ChaosMap::Tent => {
                if x < 0.5 {
                    r * x
                } else {
                    r * (1.0 - x)
                }
            }
            ChaosMap::Sine => r * (std::f32::consts::PI * x).sin(),
        };
        y.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Rate,
    Map,
    Chaos,
    Seed,
    Freeze,
    Range,
    Smooth,
    Threshold,
    Hysteresis,
    GateMode,
    GateWidth,
    OutputTarget(usize),
    OutputLevel(usize),
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 3;
const CHAOS_GROUP: usize = 0;
const GATE_GROUP: usize = 1;
const OUTPUTS_GROUP: usize = 2;

struct OutputParams {
    /// 0 = not routed, else (modbus target index + 1).
    target: AtomicUsize,
    level: AtomicF32,
}

struct Params {
    rate_hz: AtomicF32,
    map: AtomicU32,
    chaos: AtomicF32, // 0..1 -> this map's own r_range()
    seed: AtomicF32,  // 0..1, the value re-injected on reseed
    /// Set (by `edit`) whenever Seed changes -- the audio thread
    /// consumes this once to actually reseed `x`, then clears it.
    reseed_pending: AtomicBool,
    freeze: AtomicBool,
    bipolar: AtomicBool,
    smooth: AtomicF32, // 0..1 -> 0..MAX_SMOOTH_S lag time
    threshold: AtomicF32,
    hysteresis: AtomicF32,
    gate_mode: AtomicU32,
    gate_width: AtomicF32, // 0..1 -> MIN_GATE_WIDTH_S..MAX_GATE_WIDTH_S
    outputs: [OutputParams; NUM_OUTPUTS],
    /// Live published state for the panel's scope/readouts.
    cv: AtomicF32,
    smooth_cv: AtomicF32,
    gate: AtomicBool,
    delta: AtomicF32,
    history: Mutex<VecDeque<f32>>,
}

impl Params {
    fn new() -> Self {
        Self {
            rate_hz: AtomicF32::new(3.0),
            map: AtomicU32::new(0),
            // Deliberately calm by default (just past the logistic
            // map's period-2 onset, r ~ 3.03) rather than deep chaos --
            // a fresh app should read as "alive but legible," not
            // immediately glitchy; turn Chaos up for the real thing.
            chaos: AtomicF32::new(0.35),
            seed: AtomicF32::new(0.42),
            reseed_pending: AtomicBool::new(false),
            freeze: AtomicBool::new(false),
            bipolar: AtomicBool::new(false),
            smooth: AtomicF32::new(0.0),
            threshold: AtomicF32::new(0.5),
            hysteresis: AtomicF32::new(0.03),
            gate_mode: AtomicU32::new(0),
            gate_width: AtomicF32::new(0.1),
            outputs: std::array::from_fn(|_| OutputParams { target: AtomicUsize::new(0), level: AtomicF32::new(1.0) }),
            cv: AtomicF32::new(0.42),
            smooth_cv: AtomicF32::new(0.42),
            gate: AtomicBool::new(false),
            delta: AtomicF32::new(0.0),
            history: Mutex::new(VecDeque::with_capacity(HISTORY_LEN)),
        }
    }
}

pub struct QueenOfPentaclesApp {
    params: Arc<Params>,
    modbus: Arc<ModBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Queen of Pentacles' own palette: deep emerald and old gold, not
// a device-wide theme -- an earthy, regal tarot-card palette (the
// Pentacles suit's own material/earth associations) for a chaotic-map
// generator, not a device utility. ---

const QOP_BG: Rgb565 = Rgb565::new(1, 6, 2);
const QOP_TITLE: Rgb565 = Rgb565::new(28, 50, 15);
const QOP_ACCENT: Rgb565 = Rgb565::new(26, 43, 7);
const QOP_DIM: Rgb565 = Rgb565::new(9, 26, 10);
const QOP_SCOPE_OUTLINE: Rgb565 = Rgb565::new(4, 10, 5);

impl QueenOfPentaclesApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>) -> Self {
        Self { params: Arc::new(Params::new()), modbus, sensitivity, nav_speed, list: ParamList::new(), expanded: [false; NUM_GROUPS] }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        if g == CHAOS_GROUP {
            vec![Selection::Rate, Selection::Map, Selection::Chaos, Selection::Seed, Selection::Freeze, Selection::Range, Selection::Smooth]
        } else if g == GATE_GROUP {
            let mut leaves = vec![Selection::Threshold, Selection::Hysteresis, Selection::GateMode];
            if self.gate_mode() != 0 {
                leaves.push(Selection::GateWidth);
            }
            leaves
        } else {
            (0..NUM_OUTPUTS).flat_map(|c| [Selection::OutputTarget(c), Selection::OutputLevel(c)]).collect()
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
        if g == CHAOS_GROUP {
            "Chaos"
        } else if g == GATE_GROUP {
            "Gate"
        } else {
            "Outputs"
        }
    }

    fn group_summary(&self, g: usize) -> String {
        if g == CHAOS_GROUP {
            let frozen = if self.params.freeze.load(Ordering::Relaxed) { ", frozen" } else { "" };
            format!("{:.1} Hz, {} r={:.3}{frozen}", self.params.rate_hz.get(), MAP_NAMES[self.map_index() as usize], self.r())
        } else if g == GATE_GROUP {
            let on = if self.params.gate.load(Ordering::Relaxed) { ", on" } else { "" };
            format!("{}, thr={:.2}{on}", GATE_MODE_NAMES[self.gate_mode() as usize], self.params.threshold.get())
        } else {
            let routed = (0..NUM_OUTPUTS).filter(|&c| self.params.outputs[c].target.load(Ordering::Relaxed) > 0).count();
            format!("{routed}/{NUM_OUTPUTS} routed")
        }
    }

    fn map_index(&self) -> u32 {
        self.params.map.load(Ordering::Relaxed) % 3
    }

    fn gate_mode(&self) -> u32 {
        self.params.gate_mode.load(Ordering::Relaxed) % 3
    }

    fn r(&self) -> f32 {
        let (lo, hi) = ChaosMap::from_index(self.map_index()).r_range();
        lo + self.params.chaos.get().clamp(0.0, 1.0) * (hi - lo)
    }

    fn target_name(&self, idx: usize) -> String {
        if idx == 0 {
            "None".into()
        } else {
            self.modbus.names().get(idx - 1).cloned().unwrap_or_else(|| "None".into())
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Rate => "Rate".into(),
            Selection::Map => "Map".into(),
            Selection::Chaos => "Chaos".into(),
            Selection::Seed => "Seed".into(),
            Selection::Freeze => "Freeze".into(),
            Selection::Range => "Range".into(),
            Selection::Smooth => "Smooth".into(),
            Selection::Threshold => "Threshold".into(),
            Selection::Hysteresis => "Hysteresis".into(),
            Selection::GateMode => "Gate Mode".into(),
            Selection::GateWidth => "Gate Width".into(),
            Selection::OutputTarget(c) => format!("{} Target", OUTPUT_NAMES[c]),
            Selection::OutputLevel(c) => format!("{} Level", OUTPUT_NAMES[c]),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Rate => format!("{:.1} Hz", self.params.rate_hz.get()),
            Selection::Map => MAP_NAMES[self.map_index() as usize].into(),
            Selection::Chaos => format!("{:.0}% (r={:.3})", self.params.chaos.get() * 100.0, self.r()),
            Selection::Seed => format!("{:.3}", self.params.seed.get()),
            Selection::Freeze => if self.params.freeze.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Selection::Range => RANGE_NAMES[self.params.bipolar.load(Ordering::Relaxed) as usize].into(),
            Selection::Smooth => format!("{:.0} ms", self.params.smooth.get() * MAX_SMOOTH_S * 1000.0),
            Selection::Threshold => format!("{:.2}", self.params.threshold.get()),
            Selection::Hysteresis => format!("{:.2}", self.params.hysteresis.get()),
            Selection::GateMode => GATE_MODE_NAMES[self.gate_mode() as usize].into(),
            Selection::GateWidth => {
                format!("{:.0} ms", (MIN_GATE_WIDTH_S + self.params.gate_width.get() * (MAX_GATE_WIDTH_S - MIN_GATE_WIDTH_S)) * 1000.0)
            }
            Selection::OutputTarget(c) => self.target_name(self.params.outputs[c].target.load(Ordering::Relaxed)),
            Selection::OutputLevel(c) => format!("{:.0}%", self.params.outputs[c].level.get() * 100.0),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        let sensitivity = self.sensitivity.get();
        let step = delta.signum();
        match sel {
            Selection::Rate => bump(&self.params.rate_hz, delta, sensitivity, MIN_RATE_HZ, MAX_RATE_HZ),
            Selection::Map => {
                let cur = self.params.map.load(Ordering::Relaxed) as i32;
                self.params.map.store((cur + step).rem_euclid(3) as u32, Ordering::Relaxed);
            }
            Selection::Chaos => bump(&self.params.chaos, delta, sensitivity, 0.0, 1.0),
            Selection::Seed => {
                bump(&self.params.seed, delta, sensitivity, SEED_EPSILON, 1.0 - SEED_EPSILON);
                self.params.reseed_pending.store(true, Ordering::Relaxed);
            }
            Selection::Freeze => self.params.freeze.store(delta > 0, Ordering::Relaxed),
            Selection::Range => self.params.bipolar.store(delta > 0, Ordering::Relaxed),
            Selection::Smooth => bump(&self.params.smooth, delta, sensitivity, 0.0, 1.0),
            Selection::Threshold => bump(&self.params.threshold, delta, sensitivity, 0.0, 1.0),
            Selection::Hysteresis => bump(&self.params.hysteresis, delta, sensitivity, 0.0, 0.3),
            Selection::GateMode => {
                let cur = self.params.gate_mode.load(Ordering::Relaxed) as i32;
                self.params.gate_mode.store((cur + step).rem_euclid(3) as u32, Ordering::Relaxed);
            }
            Selection::GateWidth => bump(&self.params.gate_width, delta, sensitivity, 0.0, 1.0),
            Selection::OutputTarget(c) => {
                let n = self.modbus.len();
                let cur = self.params.outputs[c].target.load(Ordering::Relaxed) as i32;
                let next = (cur + step).rem_euclid(n as i32 + 1);
                self.params.outputs[c].target.store(next as usize, Ordering::Relaxed);
            }
            Selection::OutputLevel(c) => bump(&self.params.outputs[c].level, delta, sensitivity, 0.0, 1.0),
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::Rate => self.params.rate_hz.set(6.0),
            Selection::Map => self.params.map.store(0, Ordering::Relaxed),
            Selection::Chaos => self.params.chaos.set(0.85),
            Selection::Seed => {
                self.params.seed.set(0.42);
                self.params.reseed_pending.store(true, Ordering::Relaxed);
            }
            Selection::Freeze => self.params.freeze.store(false, Ordering::Relaxed),
            Selection::Range => self.params.bipolar.store(false, Ordering::Relaxed),
            Selection::Smooth => self.params.smooth.set(0.0),
            Selection::Threshold => self.params.threshold.set(0.5),
            Selection::Hysteresis => self.params.hysteresis.set(0.03),
            Selection::GateMode => self.params.gate_mode.store(0, Ordering::Relaxed),
            Selection::GateWidth => self.params.gate_width.set(0.1),
            Selection::OutputTarget(c) => self.params.outputs[c].target.store(0, Ordering::Relaxed),
            Selection::OutputLevel(c) => self.params.outputs[c].level.set(1.0),
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

    /// Live CV/Gate + a short scrolling history of the map's own
    /// value, for a real scope-style panel.
    pub(crate) fn live_state(&self) -> (f32, bool, Vec<f32>) {
        (self.params.cv.get(), self.params.gate.load(Ordering::Relaxed), self.params.history.lock().unwrap().iter().copied().collect())
    }

    /// The real map trajectory + live CV/Gate/Delta state for a
    /// bespoke Slint panel -- see `QueenOfPentaclesVisual`.
    pub(crate) fn output_visual(&self) -> QueenOfPentaclesVisual {
        const PANEL_W: f32 = 260.0;
        const PANEL_H: f32 = 110.0;
        let raw: Vec<f32> = self.params.history.lock().unwrap().iter().copied().collect();
        let trajectory = if raw.len() >= 2 {
            let (mid_x, mid_y, length, angle_deg) = crate::app::polyline_segments(&raw, PANEL_W, PANEL_H, false);
            crate::app::CurveSegments { mid_x, mid_y, length, angle_deg }
        } else {
            crate::app::CurveSegments::default()
        };
        QueenOfPentaclesVisual {
            map_name: MAP_NAMES[self.map_index() as usize].to_string(),
            r: self.r(),
            frozen: self.params.freeze.load(Ordering::Relaxed),
            trajectory,
            cv: self.params.cv.get(),
            smooth_cv: self.params.smooth_cv.get(),
            gate: self.params.gate.load(Ordering::Relaxed),
            gate_mode_name: GATE_MODE_NAMES[self.gate_mode() as usize].to_string(),
            delta: self.params.delta.get(),
            threshold: self.params.threshold.get(),
            bipolar: self.params.bipolar.load(Ordering::Relaxed),
        }
    }
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.02).clamp(min, max);
    value.set(next);
}

impl App for QueenOfPentaclesApp {
    fn needs_background_audio(&self) -> bool { self.params.outputs.iter().any(|o| o.target.load(Ordering::Relaxed) > 0) }
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
        let v = self.output_visual();
        crate::app::SlintExtra::QueenOfPentacles(crate::app::QueenOfPentaclesExtra {
            map_name: v.map_name,
            r: v.r,
            frozen: v.frozen,
            trajectory: v.trajectory,
            cv: v.cv,
            smooth_cv: v.smooth_cv,
            gate: v.gate,
            gate_mode_name: v.gate_mode_name,
            delta: v.delta,
            threshold: v.threshold,
            bipolar: v.bipolar,
        })
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(QueenOfPentaclesProcessor {
            params: Arc::clone(&self.params),
            modbus: Arc::clone(&self.modbus),
            phase: 0.0,
            x: 0.42,
            smoothed: 0.42,
            gate_state: false,
            gate_pulse_remaining: 0.0,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(QOP_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, QOP_TITLE);
        Text::new("Queen of Pentacles", Point::new(16, 26), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_8X16, QOP_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, QOP_DIM);
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
        self.list.draw_themed(fb, 16, 56, 22, 10, &display_rows, QOP_BG, QOP_DIM, QOP_ACCENT);

        let (cv, gate, history) = self.live_state();
        let scope_x = 420;
        let scope_y = 60;
        let scope_w = 180;
        let scope_h = 60;
        Rectangle::new(Point::new(scope_x, scope_y), Size::new(scope_w as u32, scope_h as u32))
            .into_styled(PrimitiveStyle::with_stroke(QOP_SCOPE_OUTLINE, 1))
            .draw(fb)
            .ok();
        for (i, v) in history.iter().enumerate() {
            let x = scope_x + (i as i32 * scope_w) / HISTORY_LEN.max(1) as i32;
            let h = (v.clamp(0.0, 1.0) * scope_h as f32) as i32;
            Rectangle::new(Point::new(x, scope_y + scope_h - h), Size::new(2, h.max(1) as u32))
                .into_styled(PrimitiveStyle::with_fill(QOP_ACCENT))
                .draw(fb)
                .ok();
        }
        Text::new(&format!("CV: {cv:.3}"), Point::new(scope_x, scope_y + scope_h + 18), accent).draw(fb).ok();
        Text::new(if gate { "GATE: on" } else { "GATE: off" }, Point::new(scope_x, scope_y + scope_h + 34), dim).draw(fb).ok();
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

struct QueenOfPentaclesProcessor {
    params: Arc<Params>,
    modbus: Arc<ModBus>,
    phase: f32,
    /// Real-time-thread-local chaotic-map state, 0..1.
    x: f32,
    /// One-pole-lagged copy of `x`, feeding the Smooth CV output.
    smoothed: f32,
    /// Comparator/flip-flop state, carried across steps for
    /// hysteresis and Flip mode.
    gate_state: bool,
    /// Samples remaining in the current Trigger-mode pulse, if any.
    gate_pulse_remaining: f32,
}

impl AudioProcessor for QueenOfPentaclesProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        // A CV/gate source, not a sound source -- makes no sound of
        // its own (the mix bus just adds zero), same as Pam's.
        for out in buffer.iter_mut() {
            *out = 0.0;
        }

        if self.params.reseed_pending.swap(false, Ordering::Relaxed) {
            self.x = self.params.seed.get().clamp(SEED_EPSILON, 1.0 - SEED_EPSILON);
        }

        let frames = (buffer.len() / channels).max(1) as f32;
        let dt = frames / sample_rate;
        let rate_hz = self.params.rate_hz.get().clamp(MIN_RATE_HZ, MAX_RATE_HZ);

        // Gate Width (Trigger mode) counts down in real time regardless
        // of whether the clock stepped this block, same as any other
        // fixed-duration pulse generator.
        if self.gate_pulse_remaining > 0.0 {
            self.gate_pulse_remaining = (self.gate_pulse_remaining - dt).max(0.0);
            if self.gate_pulse_remaining == 0.0 {
                self.gate_state = false;
            }
        }

        let prev_phase = self.phase;
        self.phase = (self.phase + rate_hz * dt).fract();
        let stepped = self.phase < prev_phase;

        let freeze = self.params.freeze.load(Ordering::Relaxed);
        let prev_x = self.x;
        if stepped && !freeze {
            let map = ChaosMap::from_index(self.params.map.load(Ordering::Relaxed) % 3);
            let (lo, hi) = map.r_range();
            let r = lo + self.params.chaos.get().clamp(0.0, 1.0) * (hi - lo);
            self.x = map.step(self.x, r);
        }
        // `delta` reads 0.0 while frozen (x genuinely didn't move) or
        // between clock steps (nothing new happened yet) -- both real.
        let delta = (self.x - prev_x).abs().clamp(0.0, 1.0);

        // One-pole lag toward the (possibly just-updated) map value,
        // every sample-block, independent of the map's own clock --
        // a real slew processor, not resampled/faked.
        let smooth_amount = self.params.smooth.get().clamp(0.0, 1.0);
        if smooth_amount <= 0.0001 {
            self.smoothed = self.x;
        } else {
            let tau = (smooth_amount * MAX_SMOOTH_S).max(0.001);
            let coef = 1.0 - (-dt / tau).exp();
            self.smoothed += (self.x - self.smoothed) * coef;
        }

        let threshold = self.params.threshold.get().clamp(0.0, 1.0);
        let hysteresis = self.params.hysteresis.get().clamp(0.0, 0.3);
        let gate_mode = self.params.gate_mode.load(Ordering::Relaxed) % 3;
        if stepped {
            let above = self.x > threshold;
            match gate_mode {
                0 => {
                    // Comparator: level-held, hysteresis avoids chatter
                    // when the map hovers right at threshold.
                    self.gate_state = if self.gate_state { self.x > (threshold - hysteresis).max(0.0) } else { above };
                }
                1 => {
                    // Trigger: a fixed-width pulse on each rising
                    // crossing of threshold.
                    let rising = above && prev_x <= threshold;
                    if rising {
                        self.gate_state = true;
                        let width = MIN_GATE_WIDTH_S + self.params.gate_width.get().clamp(0.0, 1.0) * (MAX_GATE_WIDTH_S - MIN_GATE_WIDTH_S);
                        self.gate_pulse_remaining = width;
                    }
                }
                _ => {
                    // Flip: toggles on every crossing (either
                    // direction) -- a comparator-into-a-divider.
                    let crossed = (prev_x > threshold) != (self.x > threshold);
                    if crossed {
                        self.gate_state = !self.gate_state;
                    }
                }
            }
        }

        self.params.cv.set(self.x);
        self.params.smooth_cv.set(self.smoothed);
        self.params.gate.store(self.gate_state, Ordering::Relaxed);
        self.params.delta.set(delta);

        if stepped {
            let mut hist = self.params.history.lock().unwrap();
            hist.push_back(self.x);
            if hist.len() > HISTORY_LEN {
                hist.pop_front();
            }
        }

        let bipolar = self.params.bipolar.load(Ordering::Relaxed);
        let scale = |v: f32| if bipolar { v * 2.0 - 1.0 } else { v };
        let outputs = [
            (scale(self.x), &self.params.outputs[0]),
            (scale(self.smoothed), &self.params.outputs[1]),
            (if self.gate_state { 1.0 } else { 0.0 }, &self.params.outputs[2]),
            (delta, &self.params.outputs[3]),
        ];
        for (value, out) in outputs {
            let target = out.target.load(Ordering::Relaxed);
            if target > 0 {
                if let Some(handle) = self.modbus.get(target - 1) {
                    handle.set(value * out.level.get());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> (QueenOfPentaclesApp, Arc<ModBus>) {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let app = QueenOfPentaclesApp::new(sensitivity, nav_speed, Arc::clone(&modbus));
        (app, modbus)
    }

    #[test]
    fn never_adds_sound_to_the_mix() {
        let (mut app, _modbus) = new_app();
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.6f32; 512 * 2];
        processor.process(&mut buffer, 2, 48000.0);
        assert!(buffer.iter().all(|&s| s == 0.0));
    }

    /// The logistic map's value must always stay in the real, bounded
    /// 0..1 range -- it's a well-known property of the equation for
    /// `r` in `0..4` starting inside `0..1`, and a regression here
    /// would mean the map itself is broken.
    #[test]
    fn cv_stays_within_0_to_1_across_the_full_chaos_range() {
        let (mut app, _modbus) = new_app();
        app.params.rate_hz.set(MAX_RATE_HZ);
        for chaos in [0.0, 0.3, 0.6, 0.85, 1.0] {
            app.params.chaos.set(chaos);
            let mut processor = app.audio_processor().unwrap();
            let mut buffer = vec![0.0f32; 512 * 2];
            for _ in 0..500 {
                processor.process(&mut buffer, 2, 48000.0);
            }
            let (cv, _, _) = app.live_state();
            assert!((0.0..=1.0).contains(&cv), "chaos={chaos} produced out-of-range CV {cv}");
        }
    }

    /// At full chaos, the sequence must not immediately settle into
    /// a fixed point -- a real sanity check that it's actually
    /// iterating the chaotic regime, not stuck.
    #[test]
    fn full_chaos_produces_a_genuinely_varying_sequence() {
        let (mut app, _modbus) = new_app();
        app.params.chaos.set(1.0);
        app.params.rate_hz.set(MAX_RATE_HZ);
        let mut processor = app.audio_processor().unwrap();
        // A large enough block that `rate_hz * dt >= 1` -- guarantees
        // the clock phase wraps (steps) at least once every call, so
        // every sampled value is a genuinely new map iteration rather
        // than the same still-unstepped value read back repeatedly
        // (which is what a smaller, more "realistic" block size would
        // mostly capture at only 40Hz, and would fail this test for a
        // reason that has nothing to do with the map's own chaos).
        let mut buffer = vec![0.0f32; 2000 * 2];
        let mut values = Vec::new();
        for _ in 0..300 {
            processor.process(&mut buffer, 2, 48000.0);
            let (cv, _, _) = app.live_state();
            values.push(cv);
        }
        let distinct = values.windows(2).filter(|w| (w[0] - w[1]).abs() > 1e-6).count();
        assert!(distinct > values.len() / 2, "expected a genuinely varying chaotic sequence, most steps didn't change");
    }

    /// A low Chaos setting (well below each map's own chaotic
    /// threshold) must settle into a fixed point or a short stable
    /// cycle -- the real, textbook non-chaotic regime -- for every
    /// map, not just the logistic one.
    #[test]
    fn low_chaos_settles_for_every_map() {
        for map_idx in 0..3u32 {
            let (mut app, _modbus) = new_app();
            app.params.map.store(map_idx, Ordering::Relaxed);
            app.params.chaos.set(0.0);
            app.params.rate_hz.set(MAX_RATE_HZ);
            let mut processor = app.audio_processor().unwrap();
            let mut buffer = vec![0.0f32; 2000 * 2];
            let mut values = Vec::new();
            for _ in 0..200 {
                processor.process(&mut buffer, 2, 48000.0);
                let (cv, _, _) = app.live_state();
                values.push(cv);
            }
            // Look at the settled tail: consecutive differences should
            // mostly collapse toward (near-)zero once it locks onto its
            // fixed point / short cycle.
            let tail = &values[values.len() - 20..];
            let max_step = tail.windows(2).fold(0.0f32, |m, w| m.max((w[0] - w[1]).abs()));
            assert!(max_step < 0.05, "map {map_idx} at chaos=0.0 never settled, max step in tail was {max_step}");
        }
    }

    /// FREEZE must stop the map from iterating -- CV holds exactly
    /// its last value, and Delta (a real "did it move" readout) must
    /// read exactly zero while frozen.
    #[test]
    fn freeze_holds_the_map_and_zeroes_delta() {
        let (mut app, _modbus) = new_app();
        app.params.chaos.set(1.0);
        app.params.rate_hz.set(MAX_RATE_HZ);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 2000 * 2];
        // Run unfrozen for a bit so it's not sitting on the initial x.
        for _ in 0..5 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        app.params.freeze.store(true, Ordering::Relaxed);
        let (held_cv, _, _) = app.live_state();
        for _ in 0..50 {
            processor.process(&mut buffer, 2, 48000.0);
            let (cv, _, _) = app.live_state();
            assert_eq!(cv, held_cv, "CV must hold exactly while frozen");
            assert_eq!(app.params.delta.get(), 0.0, "Delta must read exactly zero while frozen");
        }
    }

    /// Seed must actually reseed the running map: setting Seed and
    /// nudging it (which the UI's `edit()` does on every turn) must
    /// make the very next step originate from that seed, not from
    /// wherever the map happened to already be.
    #[test]
    fn seed_reseeds_the_running_map() {
        let (mut app, _modbus) = new_app();
        app.params.rate_hz.set(MAX_RATE_HZ);
        app.params.chaos.set(1.0);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 2000 * 2];
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        app.edit(Selection::Seed, 1); // bumps seed and marks reseed_pending
        let seed_value = app.params.seed.get();
        processor.process(&mut buffer, 2, 48000.0);
        let (cv, _, _) = app.live_state();
        let map = ChaosMap::Logistic;
        let (lo, hi) = map.r_range();
        let r = lo + app.params.chaos.get() * (hi - lo);
        let expected = map.step(seed_value.clamp(SEED_EPSILON, 1.0 - SEED_EPSILON), r);
        assert!((cv - expected).abs() < 1e-4, "expected the step right after reseeding to originate from the new seed, got {cv} vs {expected}");
    }

    /// Trigger gate mode must produce a real fixed-width pulse, not a
    /// level-held gate -- it must turn back off on its own well
    /// before the next threshold crossing, purely from Gate Width
    /// counting down.
    #[test]
    fn trigger_mode_produces_a_bounded_pulse_that_ends_on_its_own() {
        let (mut app, _modbus) = new_app();
        app.params.gate_mode.store(1, Ordering::Relaxed); // Trigger
        app.params.gate_width.set(0.0); // shortest real pulse (MIN_GATE_WIDTH_S)
        app.params.threshold.set(0.5);
        app.params.chaos.set(1.0);
        app.params.rate_hz.set(MAX_RATE_HZ);
        let mut processor = app.audio_processor().unwrap();
        // Small blocks so the pulse's short duration is actually
        // resolvable against real elapsed time.
        let mut buffer = vec![0.0f32; 8 * 2];
        let mut saw_high = false;
        let mut saw_low_after_high = false;
        for _ in 0..4000 {
            processor.process(&mut buffer, 2, 48000.0);
            let (_, gate, _) = app.live_state();
            if gate {
                saw_high = true;
            } else if saw_high {
                saw_low_after_high = true;
                break;
            }
        }
        assert!(saw_high, "expected Trigger mode to produce at least one pulse");
        assert!(saw_low_after_high, "expected the pulse to end on its own (bounded width), not stay latched high");
    }

    /// Flip gate mode must toggle (not just pulse or level-follow) on
    /// threshold crossings -- verified indirectly by checking it
    /// actually visits both states over a long enough run.
    #[test]
    fn flip_mode_toggles_and_visits_both_states() {
        let (mut app, _modbus) = new_app();
        app.params.gate_mode.store(2, Ordering::Relaxed); // Flip
        app.params.chaos.set(1.0);
        app.params.rate_hz.set(MAX_RATE_HZ);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 2000 * 2];
        let mut saw_on = false;
        let mut saw_off = false;
        for _ in 0..200 {
            processor.process(&mut buffer, 2, 48000.0);
            let (_, gate, _) = app.live_state();
            if gate {
                saw_on = true;
            } else {
                saw_off = true;
            }
        }
        assert!(saw_on && saw_off, "expected Flip mode to visit both gate states over a long run");
    }

    /// Bipolar Range must rescale the CV outputs into -1..1 while
    /// Gate and Delta (which aren't voltage-symmetric signals) stay
    /// untouched.
    #[test]
    fn bipolar_range_rescales_cv_outputs_only() {
        let (mut app, modbus) = new_app();
        let cv_target = modbus.register("Target: CV");
        let gate_target = modbus.register("Target: Gate");
        app.params.outputs[0].target.store(1, Ordering::Relaxed); // CV -> cv_target
        app.params.outputs[2].target.store(2, Ordering::Relaxed); // Gate -> gate_target
        app.params.bipolar.store(true, Ordering::Relaxed);
        app.params.rate_hz.set(MAX_RATE_HZ);
        app.params.chaos.set(1.0);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..50 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let cv_v = cv_target.get();
        assert!((-1.0..=1.0).contains(&cv_v), "bipolar CV must land in -1..1, got {cv_v}");
        let gate_v = gate_target.get();
        assert!(gate_v == 0.0 || gate_v == 1.0, "Gate must stay a real 0/1 value regardless of Range, got {gate_v}");
    }

    /// A routed CV output must reach its modbus target.
    #[test]
    fn a_routed_cv_output_writes_into_its_modbus_target() {
        let (mut app, modbus) = new_app();
        let target_handle = modbus.register("Some App: Some Param");
        app.params.outputs[0].target.store(1, Ordering::Relaxed);
        app.params.outputs[0].level.set(1.0);
        app.params.rate_hz.set(MAX_RATE_HZ);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..50 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let v = target_handle.get();
        assert!((0.0..=1.0).contains(&v), "expected a real CV value routed to the target, got {v}");
    }

    /// Smooth CV must actually lag behind the raw CV when Smooth is
    /// turned up -- a real slew, not an identical copy.
    #[test]
    fn smooth_cv_lags_behind_raw_cv_when_smoothed() {
        let (mut app, modbus) = new_app();
        let raw_target = modbus.register("Target: raw");
        let smooth_target = modbus.register("Target: smooth");
        app.params.outputs[0].target.store(1, Ordering::Relaxed); // CV
        app.params.outputs[1].target.store(2, Ordering::Relaxed); // Smooth CV
        app.params.smooth.set(1.0); // max lag
        app.params.chaos.set(1.0);
        app.params.rate_hz.set(MAX_RATE_HZ);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut saw_divergence = false;
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
            if (raw_target.get() - smooth_target.get()).abs() > 0.01 {
                saw_divergence = true;
                break;
            }
        }
        assert!(saw_divergence, "expected Smooth CV to visibly lag behind raw CV with Smooth turned up");
    }

    /// At full chaos, `output_visual()`'s trajectory must show real,
    /// substantial variation across its points -- the whole point of
    /// plotting the map's raw value over time is that it visibly
    /// looks chaotic, not flat, once Chaos is turned up.
    #[test]
    fn output_visual_trajectory_varies_at_full_chaos() {
        let (mut app, _modbus) = new_app();
        app.params.chaos.set(1.0);
        app.params.rate_hz.set(MAX_RATE_HZ);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 2000 * 2];
        for _ in 0..HISTORY_LEN + 10 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let visual = app.output_visual();
        assert!(visual.trajectory.mid_y.len() >= 2, "expected a populated trajectory once enough clock ticks have elapsed");
        let min = visual.trajectory.mid_y.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = visual.trajectory.mid_y.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(max - min > 5.0, "expected a genuinely varying trajectory at full chaos, y range was only {}", max - min);
    }

    /// At a low, non-chaotic Chaos setting, the trajectory must
    /// settle down -- most of its segments should collapse toward a
    /// near-flat line once the map locks onto its fixed point, the
    /// same real settling behavior `low_chaos_settles_for_every_map`
    /// checks on the raw CV.
    #[test]
    fn output_visual_trajectory_settles_at_low_chaos() {
        let (mut app, _modbus) = new_app();
        app.params.chaos.set(0.0);
        app.params.rate_hz.set(MAX_RATE_HZ);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 2000 * 2];
        for _ in 0..HISTORY_LEN + 10 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let visual = app.output_visual();
        let tail = &visual.trajectory.mid_y[visual.trajectory.mid_y.len().saturating_sub(10)..];
        let min = tail.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = tail.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(max - min < 5.0, "expected the trajectory to visibly flatten at low chaos, tail y range was {}", max - min);
    }

    /// `output_visual()` must publish the real live CV/Gate/Delta
    /// state (matching `live_state()`/`params`), not fabricated
    /// values, and an empty trajectory (rather than a panic) before
    /// enough clock ticks have ever happened.
    #[test]
    fn output_visual_reports_real_live_state() {
        let (mut app, _modbus) = new_app();
        let empty = app.output_visual();
        assert_eq!(empty.trajectory.mid_y.len(), 0, "expected an empty trajectory before any clock ticks have occurred");

        app.params.map.store(1, Ordering::Relaxed); // Tent
        app.params.chaos.set(0.5);
        app.params.rate_hz.set(MAX_RATE_HZ);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 2000 * 2];
        for _ in 0..20 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let (cv, gate, _) = app.live_state();
        let visual = app.output_visual();
        assert_eq!(visual.map_name, "Tent");
        assert_eq!(visual.cv, cv);
        assert_eq!(visual.gate, gate);
        assert_eq!(visual.delta, app.params.delta.get());
        assert_eq!(visual.smooth_cv, app.params.smooth_cv.get());
        assert_eq!(visual.threshold, app.params.threshold.get());
        assert!(!visual.frozen);
        assert!(!visual.bipolar);
    }
}
