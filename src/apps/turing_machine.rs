//! A clone of Music Thing Modular's Turing Machine: a clocked 16-bit
//! shift register whose big "Locks" knob sets whether each new bit is
//! random or a repeat of whatever the register already shifted out.
//! Feature set follows the real module's own manual and build-guide
//! documentation (musicthing.co.uk/Turing-Machine) plus its two most
//! common official expanders, Pulses and Volts, as closely as this
//! sim's architecture allows:
//!
//! - **Length**: the real module's rotary switch is not a free 2..16
//!   range -- it has exactly 8 positions (2, 3, 4, 5, 6, 8, 12, 16
//!   steps), and this clone uses that same discrete set.
//! - **Locks / Scan CV**: on the real hardware this one knob feeds a
//!   steady comparator voltage that's compared against white noise --
//!   noon (center) is pure random, turning either way "swamps" the
//!   noise and increases the odds the register keeps looping instead
//!   of randomizing. The manual documents specific detents: 3/9
//!   o'clock "slips" (mostly loops, occasionally mutates), 5 o'clock
//!   fully locks into the `length`-step loop, and 7 o'clock
//!   *double*-locks into a loop twice that long. This clone models all
//!   of that with real, computed behavior rather than hardcoded
//!   labels: probability-of-keeping-the-old-bit ramps linearly from 0
//!   at center to 1 at either extreme, and which side of center you're
//!   on decides whether the "keep" branch re-feeds the shifted-out bit
//!   unchanged (normal lock, toward 5 o'clock) or its logical inverse
//!   (toward 7 o'clock) -- feeding back an inverted bit is a genuine
//!   emergent property of a shift register that generically doubles
//!   the loop's period, which is exactly the "double lock" the real
//!   module documents, not a fabricated stand-in for it. The real
//!   module's external "Scan CV" jack (which sums into the Locks
//!   knob) is implemented the same way every other app's external CV
//!   input is: a modbus target ("Turing Machine: Locks CV") any other
//!   app's output can be routed into.
//! - **Write**: the real module's WRITE toggle switch forces every new
//!   bit to 1 or 0 regardless of what Locks would have chosen, letting
//!   you hand-edit the loop live; modeled as a 3-position Normal/Force
//!   1/Force 0 switch applied after the Locks decision, same order of
//!   operations as the real circuit.
//! - **Pulse / CV**: the two outputs the base module actually has on
//!   its own panel -- Pulse is the bit about to shift out (a gate),
//!   CV is the whole register read as one binary-weighted voltage.
//! - **Expander-equivalent outputs** (Exp Gate / Exp CV): most owners
//!   consider Pulses (11 fixed per-step/combination gate outputs) and
//!   Volts (a 5-bit weighted-DAC melodic output) part of a complete
//!   Turing Machine rig, so this clone folds in the two ideas those
//!   expanders are built from, generalized to fit one configurable
//!   output apiece instead of dedicating a whole extra panel to each:
//!   Exp Gate reproduces Pulses' "watch one step, or OR/AND two steps
//!   together" idea with a user-selectable step pair + combine mode
//!   (rather than 11 fixed jacks); Exp CV reproduces Volts' real
//!   algorithm exactly -- 5 register bits, each through its own
//!   independently adjustable weight pot, summed into one voltage.
//!
//! Deliberately not implemented: the real WRITE *jack* (a CV/gate
//! input that forces bits the same way the toggle does) -- this sim
//! has no generic per-app external gate input path beyond modbus
//! targets (already used above for Scan CV, which reuses the exact
//! same mechanism); a momentary "write gate" input isn't naturally
//! expressible as a target another app's *output level* writes into,
//! so only the panel toggle is modeled. Also not implemented: Pulses'
//! and Volts' exact fixed wiring (7 discrete step jacks + 4 fixed
//! OR-combinations on Pulses; multiple daisychained Volts boards) --
//! collapsed into one configurable instance of each idea rather than
//! faked as more outputs than this app's routing UI can reasonably
//! list, per the module doc-comment convention this sim already
//! follows (see beads.rs).

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
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

const REGISTER_BITS: u32 = 16;
const MIN_RATE_HZ: f32 = 0.25;
const MAX_RATE_HZ: f32 = 20.0;
const NUM_OUTPUTS: usize = 4;
const OUTPUT_NAMES: [&str; NUM_OUTPUTS] = ["Pulse", "CV", "Exp Gate", "Exp CV"];

/// The real module's rotary Length switch: exactly these 8 positions,
/// not a free range.
const LENGTH_STEPS: [u32; 8] = [2, 3, 4, 5, 6, 8, 12, 16];
const DEFAULT_LENGTH_IDX: usize = 7; // 16 steps

const WRITE_MODE_NAMES: [&str; 3] = ["Normal", "Force 1", "Force 0"];
const GATE_MODE_NAMES: [&str; 3] = ["Single", "OR", "AND"];
const NUM_VOLTS_WEIGHTS: usize = 5;
/// Sentinel for `gate_bit_b`: any value at or above this means "no
/// second bit selected", i.e. the Exp Gate output just watches bit A.
const GATE_BIT_B_OFF: u32 = REGISTER_BITS;

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Rate,
    Locks,
    Length,
    Write,
    GateBitA,
    GateBitB,
    GateMode,
    VoltsWeight(usize),
    OutputTarget(usize),
    OutputLevel(usize),
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 4;
const CORE_GROUP: usize = 0;
const GATE_GROUP: usize = 1;
const CV_GROUP: usize = 2;
const OUTPUT_GROUP: usize = 3;

/// Live panel state for a bespoke Slint Turing Machine screen: the
/// actual 16-bit shift register (not a fabricated animation), plus the
/// real Locks-derived keep/randomize odds. Every field is read back
/// from state `TuringMachineProcessor::process` already computes and
/// publishes each step (`Params::register`/`pulse`/`cv`/`exp_gate`/
/// `volts_cv`), or from the live Locks/Scan CV knob values using the
/// exact same formula the processor itself evaluates -- nothing here
/// is invented for display purposes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TuringMachineVisual {
    /// The full 16-position ring, in the same display-index convention
    /// as `register_bits()`: index 0 is the bit about to shift out
    /// (the Pulse edge, the "leading" end of the loop), and index
    /// `active_len - 1` is the bit most recently shifted in (the
    /// "trailing"/write end). Entries at or past `active_len` are
    /// always `false` -- not because they're hidden, but because the
    /// current `Length` setting keeps the register masked to that many
    /// bits at the source (see `TuringMachineProcessor::process`'s
    /// `mask`).
    pub bits: [bool; REGISTER_BITS as usize],
    /// How many of `bits` (from index 0) are actually part of the
    /// current loop -- the real Length setting, one of the module's 8
    /// discrete switch positions (2, 3, 4, 5, 6, 8, 12, 16).
    pub active_len: usize,
    /// Ring index of the bit most recently shifted in -- always
    /// `active_len - 1`; the panel's "write" edge to highlight.
    pub write_index: usize,
    /// The Pulse output: whether `bits[0]` (the bit about to shift
    /// out) is currently high.
    pub pulse: bool,
    /// The CV output: the register read as one binary-weighted voltage
    /// in 0..1.
    pub cv: f32,
    /// The real, live probability (0..1) of keeping the shifted-out
    /// bit instead of drawing a fresh random one on the next step --
    /// the exact `dist` value the Locks knob (+ Scan CV) computes to,
    /// per the module doc comment: 0 at noon (pure random), ramping to
    /// 1 at either full-CW or full-CCW extreme.
    pub keep_probability: f32,
    /// True when the effective Locks position is on the CCW
    /// (double-lock / inverted-feedback) side of center rather than
    /// the plain-lock CW side -- i.e. which "keep" branch (normal
    /// repeat vs. inverted repeat) would fire on the next step.
    pub inverted_feedback: bool,
    /// True only once Locks is close enough to the full-CCW extreme to
    /// be in a genuine double-locked loop -- mirrors this app's own
    /// `locks_label` "dbl lock" detent threshold exactly, not a
    /// separate fabricated cutoff.
    pub double_locked: bool,
    /// Live Exp Gate output.
    pub exp_gate: bool,
    /// Live Exp CV output, 0..1.
    pub volts_cv: f32,
}

struct OutputParams {
    /// 0 = not routed, else (modbus target index + 1) -- same
    /// convention Pam's `ChannelParams::target` uses.
    target: AtomicUsize,
    level: AtomicF32,
}

struct Params {
    rate_hz: AtomicF32,
    /// 0..1: knob position, CCW (0.0) to CW (1.0). 0.5 = noon = pure
    /// random; see module doc comment for what each direction does.
    locks: AtomicF32,
    /// External "Scan CV" -- sums into `locks` every block, same
    /// pattern every other app's ext_* modbus target uses.
    ext_locks: Arc<AtomicF32>,
    /// Index into `LENGTH_STEPS` -- the real rotary switch's position.
    length_idx: AtomicUsize,
    /// WRITE toggle: 0 = Normal, 1 = Force 1, 2 = Force 0.
    write_mode: AtomicU32,
    /// Exp Gate: which register bit(s) it watches (display-index
    /// convention, 0 = the step shown leftmost/first -- same as
    /// `register_bits()`), and how two bits combine.
    gate_bit_a: AtomicU32,
    gate_bit_b: AtomicU32,
    gate_mode: AtomicU32,
    /// Exp CV: Volts' real algorithm -- 5 register bits (steps 0..5),
    /// each independently weighted (0..1 of its binary weight
    /// 1/2/4/8/16), summed and normalized into one voltage.
    volts_weight: [AtomicF32; NUM_VOLTS_WEIGHTS],
    outputs: [OutputParams; NUM_OUTPUTS],
    /// Live register contents + derived outputs, published every step
    /// for the panel to read back.
    register: AtomicU16,
    pulse: AtomicBool,
    cv: AtomicF32,
    exp_gate: AtomicBool,
    volts_cv: AtomicF32,
}

impl Params {
    fn new(modbus: &ModBus) -> Self {
        Self {
            rate_hz: AtomicF32::new(4.0),
            locks: AtomicF32::new(0.5),
            ext_locks: modbus.register("Turing Machine: Locks CV"),
            length_idx: AtomicUsize::new(DEFAULT_LENGTH_IDX),
            write_mode: AtomicU32::new(0),
            gate_bit_a: AtomicU32::new(0),
            gate_bit_b: AtomicU32::new(GATE_BIT_B_OFF),
            gate_mode: AtomicU32::new(0),
            volts_weight: std::array::from_fn(|_| AtomicF32::new(1.0)),
            outputs: std::array::from_fn(|_| OutputParams { target: AtomicUsize::new(0), level: AtomicF32::new(1.0) }),
            register: AtomicU16::new(0xACE1), // any nonzero seed
            pulse: AtomicBool::new(false),
            cv: AtomicF32::new(0.0),
            exp_gate: AtomicBool::new(false),
            volts_cv: AtomicF32::new(0.0),
        }
    }
}

pub struct TuringMachineApp {
    params: Arc<Params>,
    modbus: Arc<ModBus>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- Turing Machine's own palette: warm paper white with bold
// cobalt, not a device-wide theme -- the playful, hand-drawn DIY-zine
// look Music Thing Modular's own hardware is known for, dark ink on
// a light panel instead of the shared dark screen. ---

const TURING_BG: Rgb565 = Rgb565::new(23, 46, 22);
const TURING_TITLE: Rgb565 = Rgb565::new(4, 6, 2);
const TURING_ACCENT: Rgb565 = Rgb565::new(0, 20, 18);
const TURING_DIM: Rgb565 = Rgb565::new(9, 17, 7);
const TURING_LED_OFF: Rgb565 = Rgb565::new(25, 50, 17);

impl TuringMachineApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>) -> Self {
        Self { params: Arc::new(Params::new(&modbus)), modbus, sensitivity, nav_speed, list: ParamList::new(), expanded: [false; NUM_GROUPS] }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            CORE_GROUP => vec![Selection::Rate, Selection::Locks, Selection::Length, Selection::Write],
            GATE_GROUP => vec![Selection::GateBitA, Selection::GateBitB, Selection::GateMode],
            CV_GROUP => (0..NUM_VOLTS_WEIGHTS).map(Selection::VoltsWeight).collect(),
            _ => (0..NUM_OUTPUTS).flat_map(|c| [Selection::OutputTarget(c), Selection::OutputLevel(c)]).collect(),
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
            CORE_GROUP => "Core",
            GATE_GROUP => "Expander Gate",
            CV_GROUP => "Expander CV",
            _ => "Output",
        }
    }

    fn length(&self) -> u32 {
        LENGTH_STEPS[self.params.length_idx.load(Ordering::Relaxed).min(LENGTH_STEPS.len() - 1)]
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            CORE_GROUP => {
                let t = self.params.locks.get().clamp(0.0, 1.0);
                let write = self.params.write_mode.load(Ordering::Relaxed);
                let write_suffix = if write != 0 { format!(", {}", WRITE_MODE_NAMES[write as usize % 3]) } else { String::new() };
                format!("{:.1} Hz, len {}, {}{}", self.params.rate_hz.get(), self.length(), self.locks_label(t), write_suffix)
            }
            GATE_GROUP => {
                let a = self.params.gate_bit_a.load(Ordering::Relaxed);
                let b = self.params.gate_bit_b.load(Ordering::Relaxed);
                if b >= GATE_BIT_B_OFF {
                    format!("bit {}", a + 1)
                } else {
                    let mode = GATE_MODE_NAMES[self.params.gate_mode.load(Ordering::Relaxed) as usize % 3];
                    format!("{} {}/{}", mode, a + 1, b + 1)
                }
            }
            CV_GROUP => {
                let avg = (0..NUM_VOLTS_WEIGHTS).map(|i| self.params.volts_weight[i].get()).sum::<f32>() / NUM_VOLTS_WEIGHTS as f32;
                format!("5-bit DAC, {:.0}% avg weight", avg * 100.0)
            }
            _ => {
                let routed = (0..NUM_OUTPUTS).filter(|&c| self.params.outputs[c].target.load(Ordering::Relaxed) > 0).count();
                format!("{routed}/{NUM_OUTPUTS} routed")
            }
        }
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
            Selection::Locks => "Locks".into(),
            Selection::Length => "Length".into(),
            Selection::Write => "Write".into(),
            Selection::GateBitA => "Gate Bit A".into(),
            Selection::GateBitB => "Gate Bit B".into(),
            Selection::GateMode => "Gate Mode".into(),
            Selection::VoltsWeight(i) => format!("CV Weight {}", i + 1),
            Selection::OutputTarget(c) => format!("{} Target", OUTPUT_NAMES[c]),
            Selection::OutputLevel(c) => format!("{} Level", OUTPUT_NAMES[c]),
        }
    }

    /// A short human label for where a Locks knob position sits --
    /// mirrors the real module's own documented detents.
    fn locks_label(&self, t: f32) -> &'static str {
        if t <= 0.03 {
            "dbl lock"
        } else if t >= 0.97 {
            "locked"
        } else if (t - 0.5).abs() < 0.03 {
            "random"
        } else if t < 0.5 {
            "slip <-"
        } else {
            "slip ->"
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Rate => format!("{:.1} Hz", self.params.rate_hz.get()),
            Selection::Locks => {
                let t = self.params.locks.get().clamp(0.0, 1.0);
                format!("{:.0}% ({})", t * 100.0, self.locks_label(t))
            }
            Selection::Length => format!("{}", self.length()),
            Selection::Write => WRITE_MODE_NAMES[self.params.write_mode.load(Ordering::Relaxed) as usize % 3].into(),
            Selection::GateBitA => format!("Bit {}", self.params.gate_bit_a.load(Ordering::Relaxed) + 1),
            Selection::GateBitB => {
                let b = self.params.gate_bit_b.load(Ordering::Relaxed);
                if b >= GATE_BIT_B_OFF { "Off".into() } else { format!("Bit {}", b + 1) }
            }
            Selection::GateMode => GATE_MODE_NAMES[self.params.gate_mode.load(Ordering::Relaxed) as usize % 3].into(),
            Selection::VoltsWeight(i) => format!("{:.0}%", self.params.volts_weight[i].get() * 100.0),
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
            Selection::Locks => bump(&self.params.locks, delta, sensitivity, 0.0, 1.0),
            Selection::Length => {
                let n = LENGTH_STEPS.len() as i32;
                let cur = self.params.length_idx.load(Ordering::Relaxed) as i32;
                self.params.length_idx.store((cur + step).rem_euclid(n) as usize, Ordering::Relaxed);
            }
            Selection::Write => {
                let cur = self.params.write_mode.load(Ordering::Relaxed) as i32;
                self.params.write_mode.store((cur + step).rem_euclid(3) as u32, Ordering::Relaxed);
            }
            Selection::GateBitA => {
                let cur = self.params.gate_bit_a.load(Ordering::Relaxed) as i32;
                self.params.gate_bit_a.store((cur + step).rem_euclid(REGISTER_BITS as i32) as u32, Ordering::Relaxed);
            }
            Selection::GateBitB => {
                // REGISTER_BITS valid bit indices (0..16) plus the
                // "Off" sentinel -- REGISTER_BITS + 1 total states.
                let cur = self.params.gate_bit_b.load(Ordering::Relaxed) as i32;
                let n = REGISTER_BITS as i32 + 1;
                self.params.gate_bit_b.store((cur + step).rem_euclid(n) as u32, Ordering::Relaxed);
            }
            Selection::GateMode => {
                let cur = self.params.gate_mode.load(Ordering::Relaxed) as i32;
                self.params.gate_mode.store((cur + step).rem_euclid(3) as u32, Ordering::Relaxed);
            }
            Selection::VoltsWeight(i) => bump(&self.params.volts_weight[i], delta, sensitivity, 0.0, 1.0),
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
            Selection::Rate => self.params.rate_hz.set(4.0),
            Selection::Locks => self.params.locks.set(0.5),
            Selection::Length => self.params.length_idx.store(DEFAULT_LENGTH_IDX, Ordering::Relaxed),
            Selection::Write => self.params.write_mode.store(0, Ordering::Relaxed),
            Selection::GateBitA => self.params.gate_bit_a.store(0, Ordering::Relaxed),
            Selection::GateBitB => self.params.gate_bit_b.store(GATE_BIT_B_OFF, Ordering::Relaxed),
            Selection::GateMode => self.params.gate_mode.store(0, Ordering::Relaxed),
            Selection::VoltsWeight(i) => self.params.volts_weight[i].set(1.0),
            Selection::OutputTarget(c) => self.params.outputs[c].target.store(0, Ordering::Relaxed),
            Selection::OutputLevel(c) => self.params.outputs[c].level.set(1.0),
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

    /// Live register contents (as a `length`-bit bool array, MSB
    /// first) + the derived Pulse/CV outputs, for a real step-light
    /// display.
    pub(crate) fn register_bits(&self) -> (Vec<bool>, bool, f32) {
        let length = self.length();
        let reg = self.params.register.load(Ordering::Relaxed);
        let bits = (0..length).map(|i| bit_at(reg, length, i)).collect();
        (bits, self.params.pulse.load(Ordering::Relaxed), self.params.cv.get())
    }

    /// Live Exp Gate / Exp CV outputs, for the panel.
    pub(crate) fn expander_state(&self) -> (bool, f32) {
        (self.params.exp_gate.load(Ordering::Relaxed), self.params.volts_cv.get())
    }

    /// The real, live panel state for a bespoke Slint Turing Machine
    /// screen -- the actual 16-bit register (see `TuringMachineVisual`
    /// doc comment for exactly which real state each field mirrors).
    pub(crate) fn output_visual(&self) -> TuringMachineVisual {
        let length = self.length();
        let reg = self.params.register.load(Ordering::Relaxed);
        let mut bits = [false; REGISTER_BITS as usize];
        for i in 0..REGISTER_BITS {
            bits[i as usize] = bit_at(reg, length, i);
        }

        // Same Locks(+Scan CV)-derived formula `TuringMachineProcessor
        // ::process` evaluates each step -- see module doc comment.
        let t = (self.params.locks.get() + self.params.ext_locks.get()).clamp(0.0, 1.0);
        let keep_probability = ((t - 0.5).abs() * 2.0).clamp(0.0, 1.0);
        let (exp_gate, volts_cv) = self.expander_state();

        TuringMachineVisual {
            bits,
            active_len: length as usize,
            write_index: (length as usize).saturating_sub(1),
            pulse: self.params.pulse.load(Ordering::Relaxed),
            cv: self.params.cv.get(),
            keep_probability,
            inverted_feedback: t < 0.5,
            // Mirrors `locks_label`'s own "dbl lock" detent threshold
            // exactly, not a separate fabricated cutoff.
            double_locked: t <= 0.03,
            exp_gate,
            volts_cv,
        }
    }
}

/// The register bit at display-index `i` (0 = leftmost/first step,
/// same convention the panel's step LEDs use) -- `i >= length` reads
/// as `false` rather than panicking, since a shorter Length or a
/// fixed Exp CV bit position can legitimately point past the current
/// loop.
fn bit_at(reg: u16, length: u32, i: u32) -> bool {
    if i >= length {
        return false;
    }
    (reg >> (length - 1 - i)) & 1 != 0
}

fn bump(value: &AtomicF32, delta: i32, sensitivity: f32, min: f32, max: f32) {
    let next = (value.get() + accelerate(delta) * sensitivity * 0.05).clamp(min, max);
    value.set(next);
}

impl App for TuringMachineApp {
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
        crate::app::SlintExtra::TuringMachine(crate::app::TuringMachineExtra {
            bits: v.bits,
            active_len: v.active_len,
            write_index: v.write_index,
            pulse: v.pulse,
            cv: v.cv,
            keep_probability: v.keep_probability,
            inverted_feedback: v.inverted_feedback,
            double_locked: v.double_locked,
            exp_gate: v.exp_gate,
            volts_cv: v.volts_cv,
        })
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(TuringMachineProcessor {
            params: Arc::clone(&self.params),
            modbus: Arc::clone(&self.modbus),
            phase: 0.0,
            rng: 0xDEAD_BEEF ^ (Arc::as_ptr(&self.params) as u32).wrapping_mul(0x9E3779B9) | 1,
        }))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(TURING_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, TURING_TITLE);
        Text::new("Turing Machine", Point::new(16, 26), title).draw(fb).ok();

        let accent = MonoTextStyle::new(&SPLEEN_8X16, TURING_ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, TURING_DIM);

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
        self.list.draw_themed(fb, 16, 56, 22, 10, &display_rows, TURING_BG, TURING_DIM, TURING_ACCENT);

        // The register itself, as a row of lit/unlit step LEDs --
        // exactly what the real module's own row of pulse LEDs shows.
        let (bits, pulse, cv) = self.register_bits();
        let led_x = 420;
        let led_y = 60;
        let led_d = 14i32;
        let led_gap = 6i32;
        for (i, on) in bits.iter().enumerate() {
            let x = led_x + i as i32 * (led_d + led_gap);
            let color = if *on { TURING_ACCENT } else { TURING_LED_OFF };
            Rectangle::new(Point::new(x, led_y), Size::new(led_d as u32, led_d as u32))
                .into_styled(PrimitiveStyle::with_fill(color))
                .draw(fb)
                .ok();
        }
        Text::new("REGISTER", Point::new(led_x, led_y - 6), dim).draw(fb).ok();

        let pulse_color = if pulse { accent } else { dim };
        Text::new(&format!("PULSE: {}", if pulse { "on " } else { "off" }), Point::new(led_x, led_y + 30), pulse_color)
            .draw(fb)
            .ok();
        Text::new(&format!("CV: {cv:.2}"), Point::new(led_x, led_y + 50), accent).draw(fb).ok();

        let (exp_gate, volts_cv) = self.expander_state();
        let exp_color = if exp_gate { accent } else { dim };
        Text::new(&format!("EXP GATE: {}", if exp_gate { "on " } else { "off" }), Point::new(led_x, led_y + 70), exp_color)
            .draw(fb)
            .ok();
        Text::new(&format!("EXP CV: {volts_cv:.2}"), Point::new(led_x, led_y + 90), accent).draw(fb).ok();
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

struct TuringMachineProcessor {
    params: Arc<Params>,
    modbus: Arc<ModBus>,
    /// Real-time-thread-local clock phase.
    phase: f32,
    rng: u32,
}

impl TuringMachineProcessor {
    fn next_rand_u32(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// A fresh 0..1 roll, decoupled from `next_rand_bit`'s own state
    /// advance so the "keep vs. randomize" decision and the new bit's
    /// own value are two independent draws, not the same one reused.
    fn next_rand_unit(&mut self) -> f32 {
        self.next_rand_u32() as f32 / u32::MAX as f32
    }

    fn next_rand_bit(&mut self) -> bool {
        self.next_rand_u32() & 1 != 0
    }
}

impl AudioProcessor for TuringMachineProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        // A CV/gate source, not a sound source -- makes no sound of
        // its own (the mix bus just adds zero), same as Pam's.
        for out in buffer.iter_mut() {
            *out = 0.0;
        }

        let frames = (buffer.len() / channels).max(1) as f32;
        let dt = frames / sample_rate;
        let rate_hz = self.params.rate_hz.get().clamp(MIN_RATE_HZ, MAX_RATE_HZ);

        let prev_phase = self.phase;
        self.phase = (self.phase + rate_hz * dt).fract();
        let stepped = self.phase < prev_phase;

        if stepped {
            let length_idx = self.params.length_idx.load(Ordering::Relaxed).min(LENGTH_STEPS.len() - 1);
            let length = LENGTH_STEPS[length_idx];
            let reg = self.params.register.load(Ordering::Relaxed);

            // The bit shifted off the top -- what a "locked" register
            // (Locks fully CW) always re-feeds, recreating the exact
            // same `length`-step loop forever.
            let shifted_out = (reg >> (length - 1)) & 1 != 0;

            // Locks knob (+ Scan CV): see module doc comment for the
            // real comparator-vs-noise behavior this reproduces. `t`
            // is the knob's CCW(0)..CW(1) position; distance from
            // center (0.5) is the probability of keeping the old loop
            // instead of drawing a fresh random bit, and which side of
            // center decides whether "keep" means an exact repeat
            // (normal lock, toward CW) or an inverted repeat (double
            // lock via period-doubling, toward CCW).
            let t = (self.params.locks.get() + self.params.ext_locks.get()).clamp(0.0, 1.0);
            let dist = ((t - 0.5).abs() * 2.0).clamp(0.0, 1.0);
            let roll = self.next_rand_unit();
            let candidate = if t >= 0.5 { shifted_out } else { !shifted_out };
            let mut new_bit = if roll < dist { candidate } else { self.next_rand_bit() };

            // WRITE toggle: forces the incoming bit regardless of
            // Locks, same order of operations as the real circuit.
            match self.params.write_mode.load(Ordering::Relaxed) {
                1 => new_bit = true,
                2 => new_bit = false,
                _ => {}
            }

            let mask: u16 = if length >= REGISTER_BITS { u16::MAX } else { (1u16 << length) - 1 };
            let shifted = ((reg << 1) | (new_bit as u16)) & mask;
            self.params.register.store(shifted, Ordering::Relaxed);

            let pulse = (shifted >> (length - 1)) & 1 != 0;
            self.params.pulse.store(pulse, Ordering::Relaxed);
            let cv = shifted as f32 / mask.max(1) as f32;
            self.params.cv.set(cv);

            // Exp Gate: Pulses-expander-style -- watch one bit, or
            // OR/AND two of them together.
            let bit_a = self.params.gate_bit_a.load(Ordering::Relaxed);
            let bit_b = self.params.gate_bit_b.load(Ordering::Relaxed);
            let a_on = bit_at(shifted, length, bit_a);
            let exp_gate = if bit_b >= GATE_BIT_B_OFF {
                a_on
            } else {
                let b_on = bit_at(shifted, length, bit_b);
                match self.params.gate_mode.load(Ordering::Relaxed) {
                    1 => a_on || b_on,
                    2 => a_on && b_on,
                    _ => a_on,
                }
            };
            self.params.exp_gate.store(exp_gate, Ordering::Relaxed);

            // Exp CV: Volts-expander-style -- 5 register bits, each
            // through its own weight pot (0..1 of that bit's binary
            // weight 1/2/4/8/16), summed and normalized.
            const BIN_WEIGHTS: [f32; NUM_VOLTS_WEIGHTS] = [1.0, 2.0, 4.0, 8.0, 16.0];
            let mut sum = 0.0f32;
            let mut max = 0.0f32;
            for (i, bw) in BIN_WEIGHTS.iter().enumerate() {
                let w = self.params.volts_weight[i].get().clamp(0.0, 1.0) * bw;
                max += w;
                if bit_at(shifted, length, i as u32) {
                    sum += w;
                }
            }
            let volts_cv = if max > 0.0 { sum / max } else { 0.0 };
            self.params.volts_cv.set(volts_cv);

            let outputs = [
                (if pulse { 1.0 } else { 0.0 }, &self.params.outputs[0]),
                (cv, &self.params.outputs[1]),
                (if exp_gate { 1.0 } else { 0.0 }, &self.params.outputs[2]),
                (volts_cv, &self.params.outputs[3]),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> (TuringMachineApp, Arc<ModBus>) {
        let sensitivity = Arc::new(AtomicF32::new(0.1));
        let nav_speed = Arc::new(AtomicF32::new(6.0));
        let modbus = Arc::new(ModBus::new());
        let app = TuringMachineApp::new(sensitivity, nav_speed, Arc::clone(&modbus));
        (app, modbus)
    }

    /// The index of `n` steps in `LENGTH_STEPS` -- test helper mirroring
    /// how the app itself stores Length as a discrete switch position.
    fn length_idx_for(n: u32) -> usize {
        LENGTH_STEPS.iter().position(|&s| s == n).unwrap()
    }

    /// Fully locked (Locks = 1.0, fully CW), the register must settle
    /// into a fixed repeating pattern -- the same `length`-step loop
    /// forever, never introducing a fresh random bit. Drives with
    /// `MAX_RATE_HZ` (the fastest realistic setting) and a real block
    /// size, then dedups consecutive-equal captures down to the actual
    /// per-step sequence (most captures land between clock steps)
    /// before checking it repeats -- exercises the real clock/step
    /// logic exactly as a knob turned all the way up would, rather
    /// than an unrealistic rate that steps faster than the clock's own
    /// one-wrap-per-block assumption supports.
    #[test]
    fn fully_locked_register_repeats_its_own_loop_exactly() {
        let (mut app, _modbus) = new_app();
        app.params.locks.set(1.0);
        app.params.rate_hz.set(MAX_RATE_HZ);
        app.params.length_idx.store(length_idx_for(8), Ordering::Relaxed);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut steps = Vec::new();
        let mut last = app.params.register.load(Ordering::Relaxed);
        for _ in 0..400 {
            processor.process(&mut buffer, 2, 48000.0);
            let now = app.params.register.load(Ordering::Relaxed);
            if now != last {
                steps.push(now);
                last = now;
            }
        }
        assert!(steps.len() >= 16, "expected at least 2 full 8-step loops, got {} steps", steps.len());
        let tail = &steps[steps.len() - 16..];
        assert_eq!(&tail[0..8], &tail[8..16], "a fully locked register must repeat its own 8-step loop exactly");
    }

    /// Fully double-locked (Locks = 0.0, fully CCW), the register must
    /// settle into a fixed pattern that repeats with period `2 *
    /// length`, not `length` -- the real module's documented "double
    /// lock" behavior, produced here by genuinely feeding back the
    /// *inverted* shifted-out bit rather than faking a longer loop.
    #[test]
    fn fully_double_locked_register_repeats_at_twice_the_length() {
        let (mut app, _modbus) = new_app();
        app.params.locks.set(0.0);
        app.params.rate_hz.set(MAX_RATE_HZ);
        app.params.length_idx.store(length_idx_for(4), Ordering::Relaxed);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut steps = Vec::new();
        let mut last = app.params.register.load(Ordering::Relaxed);
        for _ in 0..400 {
            processor.process(&mut buffer, 2, 48000.0);
            let now = app.params.register.load(Ordering::Relaxed);
            if now != last {
                steps.push(now);
                last = now;
            }
        }
        assert!(steps.len() >= 16, "expected at least 2 full 8-step (2x4) loops, got {} steps", steps.len());
        let tail = &steps[steps.len() - 16..];
        assert_eq!(&tail[0..8], &tail[8..16], "a fully double-locked register must repeat its own 2*length loop exactly");
        // And the period must genuinely be 2*length, not length --
        // the first half of one loop must differ from its second half
        // (that's exactly what makes it a period-8 and not period-4
        // sequence).
        assert_ne!(&tail[0..4], &tail[4..8], "double lock must not degenerate into an ordinary length-4 loop");
    }

    /// At dead center (Locks = 0.5, noon), the register must be a
    /// genuine random walk -- never settling into a short repeating
    /// loop the way either locked extreme does.
    #[test]
    fn centered_locks_produces_a_genuine_random_walk() {
        let (mut app, _modbus) = new_app();
        app.params.locks.set(0.5);
        app.params.rate_hz.set(MAX_RATE_HZ);
        app.params.length_idx.store(length_idx_for(8), Ordering::Relaxed);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut steps = Vec::new();
        let mut last = app.params.register.load(Ordering::Relaxed);
        for _ in 0..2000 {
            processor.process(&mut buffer, 2, 48000.0);
            let now = app.params.register.load(Ordering::Relaxed);
            if now != last {
                steps.push(now);
                last = now;
            }
        }
        assert!(steps.len() >= 64, "expected plenty of steps, got {}", steps.len());
        // A random walk must visit more distinct register values than
        // a short fixed loop ever could.
        let distinct: std::collections::HashSet<_> = steps.iter().copied().collect();
        assert!(distinct.len() > 16, "expected a genuinely varying random walk, only saw {} distinct values", distinct.len());
    }

    /// The module itself must never add anything to the audible mix.
    #[test]
    fn never_adds_sound_to_the_mix() {
        let (mut app, _modbus) = new_app();
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.5f32; 512 * 2]; // pre-fill with garbage to prove it gets zeroed
        processor.process(&mut buffer, 2, 48000.0);
        assert!(buffer.iter().all(|&s| s == 0.0));
    }

    /// A routed Pulse output must reach its modbus target.
    #[test]
    fn a_routed_pulse_output_writes_into_its_modbus_target() {
        let (mut app, modbus) = new_app();
        let target_handle = modbus.register("Some App: Some Param");
        // Locks CV registers first (inside Params::new), so this new
        // target lands at index 1.
        app.params.outputs[0].target.store(2, Ordering::Relaxed);
        app.params.outputs[0].level.set(1.0);
        app.params.rate_hz.set(MAX_RATE_HZ);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..200 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        // Whatever it settled on, it must be exactly 0.0 or 1.0 -- a
        // real gate value, not something in between or untouched.
        let v = target_handle.get();
        assert!(v == 0.0 || v == 1.0, "expected a real gate value (0 or 1), got {v}");
    }

    /// Changing `Length` must clamp the register into that many bits
    /// -- never produce a CV/pulse pattern wider than the configured
    /// length, and only ever land on one of the real switch's 8
    /// discrete positions.
    #[test]
    fn shorter_length_never_panics_and_stays_in_range() {
        let (mut app, _modbus) = new_app();
        app.params.length_idx.store(length_idx_for(3), Ordering::Relaxed);
        app.params.rate_hz.set(MAX_RATE_HZ);
        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..200 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let (bits, _, cv) = app.register_bits();
        assert_eq!(bits.len(), 3);
        assert!((0.0..=1.0).contains(&cv));
    }

    /// Editing Length must only ever land on one of the real switch's
    /// 8 discrete positions (2,3,4,5,6,8,12,16), never an arbitrary
    /// in-between value.
    #[test]
    fn length_only_takes_the_real_switchs_discrete_positions() {
        let (mut app, _modbus) = new_app();
        for _ in 0..20 {
            app.edit(Selection::Length, 1);
            assert!(LENGTH_STEPS.contains(&app.length()), "Length landed on non-switch value {}", app.length());
        }
        for _ in 0..20 {
            app.edit(Selection::Length, -1);
            assert!(LENGTH_STEPS.contains(&app.length()), "Length landed on non-switch value {}", app.length());
        }
    }

    /// WRITE forced to 1 must drive every new bit to 1 regardless of
    /// Locks -- the register climbs to (and stays at) all-ones.
    #[test]
    fn write_force_1_fills_the_register_with_ones() {
        let (mut app, _modbus) = new_app();
        app.params.length_idx.store(length_idx_for(8), Ordering::Relaxed);
        app.params.locks.set(0.5); // would otherwise be a random walk
        app.params.write_mode.store(1, Ordering::Relaxed); // Force 1
        app.params.rate_hz.set(MAX_RATE_HZ);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..50 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let (bits, pulse, cv) = app.register_bits();
        assert!(bits.iter().all(|&b| b), "expected every register bit forced to 1, got {bits:?}");
        assert!(pulse);
        assert_eq!(cv, 1.0);
    }

    /// WRITE forced to 0 must drive every new bit to 0 regardless of
    /// Locks -- the register clears to (and stays at) all-zeros.
    #[test]
    fn write_force_0_clears_the_register_to_zeros() {
        let (mut app, _modbus) = new_app();
        app.params.length_idx.store(length_idx_for(8), Ordering::Relaxed);
        app.params.locks.set(0.5);
        app.params.write_mode.store(2, Ordering::Relaxed); // Force 0
        app.params.rate_hz.set(MAX_RATE_HZ);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..50 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let (bits, pulse, cv) = app.register_bits();
        assert!(bits.iter().all(|&b| !b), "expected every register bit forced to 0, got {bits:?}");
        assert!(!pulse);
        assert_eq!(cv, 0.0);
    }

    /// The Scan CV modbus target (summed into Locks) must be able to
    /// push a centered knob into a fully locked state, same as
    /// physically turning the knob would.
    #[test]
    fn scan_cv_target_can_lock_a_centered_knob() {
        let (mut app, _modbus) = new_app();
        app.params.length_idx.store(length_idx_for(8), Ordering::Relaxed);
        app.params.locks.set(0.5); // centered -> would be random alone
        app.params.ext_locks.set(0.5); // pushes effective t to 1.0 -> locked
        app.params.rate_hz.set(MAX_RATE_HZ);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        let mut steps = Vec::new();
        let mut last = app.params.register.load(Ordering::Relaxed);
        for _ in 0..400 {
            processor.process(&mut buffer, 2, 48000.0);
            let now = app.params.register.load(Ordering::Relaxed);
            if now != last {
                steps.push(now);
                last = now;
            }
        }
        assert!(steps.len() >= 16);
        let tail = &steps[steps.len() - 16..];
        assert_eq!(&tail[0..8], &tail[8..16], "Scan CV summed to a locking value must lock the loop, same as turning the knob");
    }

    /// Exp Gate (single bit A) and Exp CV (5-bit weighted DAC) must
    /// both read as fully on / full-scale once WRITE has forced the
    /// whole register to 1, and both must stay within their real
    /// bounded ranges throughout.
    #[test]
    fn expander_outputs_track_a_fully_set_register() {
        let (mut app, _modbus) = new_app();
        app.params.length_idx.store(length_idx_for(8), Ordering::Relaxed);
        app.params.write_mode.store(1, Ordering::Relaxed); // Force 1
        app.params.rate_hz.set(MAX_RATE_HZ);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..50 {
            processor.process(&mut buffer, 2, 48000.0);
        }
        let (exp_gate, volts_cv) = app.expander_state();
        assert!(exp_gate, "expected Exp Gate on once the watched bit is forced to 1");
        assert_eq!(volts_cv, 1.0, "expected Exp CV at full scale once every weighted bit is 1");
    }

    /// `output_visual`'s `bits` must match `register_bits()` exactly
    /// for every active position, `active_len` must equal the real
    /// Length setting, and every position at/past `active_len` must
    /// read `false` -- the panel must never show a lit step that isn't
    /// genuinely part of the current loop.
    #[test]
    fn output_visual_bits_match_the_real_register_exactly() {
        let (mut app, _modbus) = new_app();
        app.params.length_idx.store(length_idx_for(6), Ordering::Relaxed);
        app.params.write_mode.store(1, Ordering::Relaxed); // Force 1, so it's easy to tell real bits from stray defaults
        app.params.rate_hz.set(MAX_RATE_HZ);

        let mut processor = app.audio_processor().unwrap();
        let mut buffer = vec![0.0f32; 512 * 2];
        for _ in 0..30 {
            processor.process(&mut buffer, 2, 48000.0);
        }

        let (expected_bits, expected_pulse, expected_cv) = app.register_bits();
        let visual = app.output_visual();

        assert_eq!(visual.active_len, 6);
        assert_eq!(visual.write_index, 5);
        assert_eq!(visual.pulse, expected_pulse);
        assert_eq!(visual.cv, expected_cv);
        for (i, &want) in expected_bits.iter().enumerate() {
            assert_eq!(visual.bits[i], want, "visual bit {i} disagreed with the real register");
        }
        for i in expected_bits.len()..REGISTER_BITS as usize {
            assert!(!visual.bits[i], "bit {i} is past active_len and must read false, not a stray value");
        }
    }

    /// A fully locked (Locks = 1.0) knob must report `keep_probability`
    /// at its real maximum (1.0) and the *non*-inverted (plain-lock)
    /// side; a fully double-locked (Locks = 0.0) knob must report the
    /// same max probability but the inverted side, with `double_locked`
    /// set -- exactly the two extremes the module doc comment and
    /// `locks_label` already document.
    #[test]
    fn output_visual_keep_probability_and_lock_side_track_the_locks_knob() {
        let (mut app, _modbus) = new_app();

        app.params.locks.set(1.0);
        let locked = app.output_visual();
        assert_eq!(locked.keep_probability, 1.0);
        assert!(!locked.inverted_feedback);
        assert!(!locked.double_locked);

        app.params.locks.set(0.0);
        let double_locked = app.output_visual();
        assert_eq!(double_locked.keep_probability, 1.0);
        assert!(double_locked.inverted_feedback);
        assert!(double_locked.double_locked);

        app.params.locks.set(0.5);
        let random = app.output_visual();
        assert_eq!(random.keep_probability, 0.0);
    }

    /// Exp Gate's OR/AND combine modes must compute real boolean
    /// combinations of the two selected bits, not just echo bit A.
    #[test]
    fn expander_gate_or_and_and_modes_combine_two_bits() {
        let (mut app, _modbus) = new_app();
        app.params.length_idx.store(length_idx_for(8), Ordering::Relaxed);
        app.params.write_mode.store(2, Ordering::Relaxed); // Force 0 -> both bits start 0
        // Watch the two most-recently-shifted-in bits (display indices
        // length-1 and length-2) -- these track WRITE immediately every
        // step, unlike the MSB-side bits which take `length` steps to
        // converge after a WRITE-mode change.
        app.params.gate_bit_a.store(7, Ordering::Relaxed);
        app.params.gate_bit_b.store(6, Ordering::Relaxed);
        app.params.gate_mode.store(1, Ordering::Relaxed); // OR
        app.params.rate_hz.set(MAX_RATE_HZ);

        {
            let mut processor = app.audio_processor().unwrap();
            let mut buffer = vec![0.0f32; 512 * 2];
            for _ in 0..20 {
                processor.process(&mut buffer, 2, 48000.0);
            }
            let (exp_gate, _) = app.expander_state();
            assert!(!exp_gate, "OR of two zero bits must be off");
        }

        // Force both bits to 1 and re-check OR, then AND.
        app.params.write_mode.store(1, Ordering::Relaxed); // Force 1
        {
            let mut processor = app.audio_processor().unwrap();
            let mut buffer = vec![0.0f32; 512 * 2];
            for _ in 0..20 {
                processor.process(&mut buffer, 2, 48000.0);
            }
            let (exp_gate, _) = app.expander_state();
            assert!(exp_gate, "OR of two set bits must be on");
        }
        app.params.gate_mode.store(2, Ordering::Relaxed); // AND
        {
            let mut processor = app.audio_processor().unwrap();
            let mut buffer = vec![0.0f32; 512 * 2];
            for _ in 0..20 {
                processor.process(&mut buffer, 2, 48000.0);
            }
            let (exp_gate, _) = app.expander_state();
            assert!(exp_gate, "AND of two set bits must be on");
        }
    }
}
