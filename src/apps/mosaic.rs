//! Mosaic: four effects in series, each switched and shaped by its own
//! step sequence. Inspired by the idea of Polyend's Mess (a multi-effect
//! whose effects are sequenced); the effects, sequencer and layout here
//! are Portamax's own.
//!
//! Each slot has an effect, two parameters (Amount and Tone, meaning
//! whatever that effect needs), a wet Mix, and a pattern: up to 16 steps
//! at its own length and clock division (so slots can run polymetric),
//! with a Chance that a lit step actually fires. A lit step engages the
//! effect for that step; a step can also lock its own Amount (hold the
//! pad and turn knob 2). A slot with no lit steps is simply always on,
//! and so is everything while the transport is stopped -- then Mosaic is
//! a plain four-slot multi-effect.
//!
//! Pads (STEPS layer): the focused slot's 16 steps. Tap = step on/off;
//! hold + knob 2 = lock Amount on that step; hold + knob 2 press = clear
//! the lock. D-pad up/down focuses the next slot.
//!
//! Patterns are saved to the SD card (`saves/mosaic/patterns.json`).

use crate::{
    app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes, Throw},
    app::{App, Input},
    audio::AudioProcessor,
    audio_bus::{cycle_source, AudioBus, NO_SOURCE},
    display::FrameBuffer,
    led_output::PadColor,
    mixer_bus::MixerBus,
    modbus::ModBus,
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_6X12, SPLEEN_8X16},
    util::AtomicF32,
};
use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};
use std::f32::consts::{PI, TAU};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "Mosaic";
const SLOTS: usize = 4;
const STEPS: usize = 16;

// Own palette: tiles of teal and rust on slate.
const BG: Rgb565 = Rgb565::new(3, 7, 7);
const INK: Rgb565 = Rgb565::new(24, 54, 26);
const ACCENT: Rgb565 = Rgb565::new(28, 30, 8);
const DIM: Rgb565 = Rgb565::new(10, 26, 14);
const FAINT: Rgb565 = Rgb565::new(5, 12, 9);
const TILE_ON: Rgb565 = Rgb565::new(6, 44, 26);

/// The effects, with what Amount and Tone mean for each.
pub const EFFECTS: [(&str, &str, &str); 12] = [
    ("Off", "-", "-"),
    ("Low-pass", "Cutoff", "Resonance"),
    ("High-pass", "Cutoff", "Resonance"),
    ("Delay", "Time", "Feedback"),
    ("Repeat", "Slice", "Decay"),
    ("Reverse", "Length", "Fade"),
    ("Crush", "Bits", "Rate"),
    ("Drive", "Gain", "Tone"),
    ("Gate", "Rate", "Width"),
    ("Ring", "Frequency", "Shape"),
    ("Pitch", "Semitones", "Grain"),
    ("Reverb", "Size", "Damping"),
];
const FX_OFF: usize = 0;
const FX_LP: usize = 1;
const FX_HP: usize = 2;
const FX_DELAY: usize = 3;
const FX_REPEAT: usize = 4;
const FX_REVERSE: usize = 5;
const FX_CRUSH: usize = 6;
const FX_DRIVE: usize = 7;
const FX_GATE: usize = 8;
const FX_RING: usize = 9;
const FX_PITCH: usize = 10;
const FX_REVERB: usize = 11;

/// Step divisions, in beats.
pub const DIVS: [(&str, f32); 8] = [("1/32", 0.125), ("1/16T", 1.0 / 6.0), ("1/16", 0.25), ("1/8T", 1.0 / 3.0), ("1/8", 0.5), ("1/4", 1.0), ("1/2", 2.0), ("1 bar", 4.0)];
const DIV_DEFAULT: usize = 2;
/// Delay times on offer, in beats (1/16, 1/8, dotted 1/8, 1/4, dotted 1/4, 1/2).
const DELAY_BEATS: [f32; 6] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0];
/// Repeat / reverse / gate slices, in beats.
const SLICE_BEATS: [f32; 5] = [0.125, 0.25, 0.5, 1.0, 2.0];
/// Seconds of audio each slot keeps (delay, repeat, reverse, pitch).
const RING_SECONDS: f32 = 2.5;

struct SlotShared {
    fx: AtomicUsize,
    amount: AtomicF32,
    tone: AtomicF32,
    mix: AtomicF32,
    length: AtomicUsize,
    div: AtomicUsize,
    chance: AtomicF32,
    steps: [AtomicBool; STEPS],
    /// Amount locked on a step; NaN = none.
    locks: [AtomicF32; STEPS],
    cv: Arc<AtomicF32>,
    // telemetry
    pos: AtomicUsize,
    engaged: AtomicBool,
}

impl SlotShared {
    fn new(fx: usize, cv: Arc<AtomicF32>) -> Self {
        Self {
            fx: AtomicUsize::new(fx),
            amount: AtomicF32::new(0.5),
            tone: AtomicF32::new(0.4),
            mix: AtomicF32::new(1.0),
            length: AtomicUsize::new(STEPS),
            div: AtomicUsize::new(DIV_DEFAULT),
            chance: AtomicF32::new(1.0),
            steps: std::array::from_fn(|_| AtomicBool::new(false)),
            locks: std::array::from_fn(|_| AtomicF32::new(f32::NAN)),
            cv,
            pos: AtomicUsize::new(0),
            engaged: AtomicBool::new(true),
        }
    }
    fn any_steps(&self) -> bool {
        self.steps.iter().any(|s| s.load(Ordering::Relaxed))
    }
}

struct Shared {
    source: AtomicUsize,
    bpm: AtomicF32,
    running: AtomicBool,
    focus: AtomicUsize,
    slots: [SlotShared; SLOTS],
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    /// Bumped on every pattern change, for autosave.
    dirty: AtomicU32,
}

// ------------------------------------------------------------ controls

const CONTROLS: [&str; 10] = ["Amount", "Tone", "Mix", "Chance", "Slot", "Effect", "Length", "Division", "Tempo", "Run"];
const C_AMOUNT: usize = 0;
const C_TONE: usize = 1;
const C_MIX: usize = 2;
const C_CHANCE: usize = 3;
const C_SLOT: usize = 4;
const C_EFFECT: usize = 5;
const C_LENGTH: usize = 6;
const C_DIV: usize = 7;
const C_TEMPO: usize = 8;
const C_RUN: usize = 9;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "mosaic",
        layers: vec![Layer::Native(0, "STEPS"), Layer::Throws, Layer::Controls, Layer::Moments],
        hero: vec![[C_AMOUNT, C_TONE], [C_MIX, C_CHANCE], [C_SLOT, C_EFFECT], [C_LENGTH, C_DIV]],
        browse: Some(C_SLOT),
        routes: Routes { stick_x: Some(C_AMOUNT), stick_y: Some(C_TONE), hand_l: Some(C_MIX), hand_r: Some(C_CHANCE) },
        throws: vec![
            Throw { control: C_AMOUNT, to: 1.0, label: "MAX" },
            Throw { control: C_AMOUNT, to: 0.0, label: "MIN" },
            Throw { control: C_TONE, to: 1.0, label: "TONE+" },
            Throw { control: C_MIX, to: 0.0, label: "DRY" },
            Throw { control: C_CHANCE, to: 0.0, label: "NEVER" },
            Throw { control: C_CHANCE, to: 0.5, label: "MAYBE" },
            Throw { control: C_DIV, to: 0.0, label: "ROLL" },
            Throw { control: C_DIV, to: 1.0, label: "SLOW" },
        ],
        midi_to_pads: false,
        own_expression: false,
    }
}

/// Menu rows: Source, Tempo, Run, Slot, then the focused slot's eight.
const ROWS: usize = 12;

pub struct MosaicApp {
    p: Arc<Shared>,
    bus: Arc<AudioBus>,
    own: usize,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
    prev_grid: [bool; 16],
    /// A pad that was turned while held doesn't toggle on release.
    turned: [bool; 16],
    save_path: Option<std::path::PathBuf>,
    saved: u32,
    seen: u32,
    quiet: u32,
}

impl MosaicApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let own = bus.index_of(APP_NAME).unwrap_or(NO_SOURCE);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let defaults = [FX_LP, FX_DELAY, FX_OFF, FX_OFF];
        let slots = std::array::from_fn(|i| SlotShared::new(defaults[i], mods.register(format!("{APP_NAME}: Slot {} Amount", i + 1))));
        let mut app = Self {
            p: Arc::new(Shared {
                source: AtomicUsize::new(NO_SOURCE),
                bpm: AtomicF32::new(120.0),
                running: AtomicBool::new(true),
                focus: AtomicUsize::new(0),
                slots,
                mix_level,
                ext_mix_level,
                output,
                dirty: AtomicU32::new(0),
            }),
            bus,
            own,
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            prev_grid: [false; 16],
            turned: [false; 16],
            save_path: (!cfg!(test)).then(|| std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves/mosaic/patterns.json"))),
            saved: 0,
            seen: 0,
            quiet: 0,
        };
        app.load();
        app
    }

    fn focus(&self) -> usize {
        self.p.focus.load(Ordering::Relaxed).min(SLOTS - 1)
    }

    fn slot(&self) -> &SlotShared {
        &self.p.slots[self.focus()]
    }

    fn touch(&self) {
        self.p.dirty.fetch_add(1, Ordering::Relaxed);
    }

    fn value(&self, c: usize) -> String {
        let s = self.slot();
        let fx = s.fx.load(Ordering::Relaxed).min(EFFECTS.len() - 1);
        match c {
            C_AMOUNT => amount_text(fx, s.amount.get(), self.p.bpm.get()),
            C_TONE => format!("{:.0}%", s.tone.get() * 100.0),
            C_MIX => format!("{:.0}%", s.mix.get() * 100.0),
            C_CHANCE => format!("{:.0}%", s.chance.get() * 100.0),
            C_SLOT => format!("{} · {}", self.focus() + 1, EFFECTS[fx].0),
            C_EFFECT => EFFECTS[fx].0.into(),
            C_LENGTH => format!("{} steps", s.length.load(Ordering::Relaxed)),
            C_DIV => DIVS[s.div.load(Ordering::Relaxed).min(DIVS.len() - 1)].0.into(),
            C_TEMPO => format!("{:.0} BPM", self.p.bpm.get()),
            _ => (if self.p.running.load(Ordering::Relaxed) { "running" } else { "stopped" }).into(),
        }
    }

    fn label(&self, c: usize) -> String {
        let fx = self.slot().fx.load(Ordering::Relaxed).min(EFFECTS.len() - 1);
        match c {
            C_AMOUNT if fx != FX_OFF => EFFECTS[fx].1.into(),
            C_TONE if fx != FX_OFF => EFFECTS[fx].2.into(),
            _ => CONTROLS[c].into(),
        }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        let mut r = vec![
            ("Source".into(), self.bus.source_name(self.p.source.load(Ordering::Relaxed)), false),
            ("Tempo".into(), self.value(C_TEMPO), false),
            ("Run".into(), self.value(C_RUN), false),
            ("Slot".into(), self.value(C_SLOT), false),
        ];
        for c in [C_EFFECT, C_AMOUNT, C_TONE, C_MIX, C_LENGTH, C_DIV, C_CHANCE] {
            r.push((format!("  {}", self.label(c)), self.value(c), false));
        }
        r.push(("  >> Clear steps".into(), format!("{} lit", self.slot().steps.iter().filter(|s| s.load(Ordering::Relaxed)).count()), false));
        r
    }

    fn edit(&mut self, c: usize, d: i32) {
        if d == 0 {
            return;
        }
        let sens = self.sensitivity.get().max(0.01) * 10.0;
        let s = &self.p.slots[self.focus()];
        let nudge = |a: &AtomicF32, lo: f32, hi: f32| a.set((a.get() + d as f32 * 0.01 * sens * (hi - lo)).clamp(lo, hi));
        let step = |a: &AtomicUsize, n: usize| a.store((a.load(Ordering::Relaxed) as i32 + d.signum()).rem_euclid(n as i32) as usize, Ordering::Relaxed);
        match c {
            C_AMOUNT => nudge(&s.amount, 0.0, 1.0),
            C_TONE => nudge(&s.tone, 0.0, 1.0),
            C_MIX => nudge(&s.mix, 0.0, 1.0),
            C_CHANCE => nudge(&s.chance, 0.0, 1.0),
            C_SLOT => step(&self.p.focus, SLOTS),
            C_EFFECT => step(&s.fx, EFFECTS.len()),
            C_LENGTH => s.length.store((s.length.load(Ordering::Relaxed) as i32 + d.signum()).clamp(1, STEPS as i32) as usize, Ordering::Relaxed),
            C_DIV => s.div.store((s.div.load(Ordering::Relaxed) as i32 + d.signum()).clamp(0, DIVS.len() as i32 - 1) as usize, Ordering::Relaxed),
            C_TEMPO => self.p.bpm.set((self.p.bpm.get() + d as f32).clamp(40.0, 240.0)),
            C_RUN => self.p.running.store(d > 0, Ordering::Relaxed),
            _ => {}
        }
        if matches!(c, C_EFFECT | C_LENGTH | C_DIV) {
            self.touch();
        }
    }

    fn edit_row(&mut self, row: usize, d: i32) {
        match row {
            0 if d != 0 => {
                let mut s = cycle_source(self.p.source.load(Ordering::Relaxed), d.signum(), self.bus.len());
                if s == self.own {
                    s = cycle_source(s, d.signum(), self.bus.len());
                }
                self.p.source.store(s, Ordering::Relaxed);
            }
            1 => self.edit(C_TEMPO, d),
            2 => self.edit(C_RUN, d),
            3 => self.edit(C_SLOT, d),
            4 => self.edit(C_EFFECT, d),
            5 => self.edit(C_AMOUNT, d),
            6 => self.edit(C_TONE, d),
            7 => self.edit(C_MIX, d),
            8 => self.edit(C_LENGTH, d),
            9 => self.edit(C_DIV, d),
            10 => self.edit(C_CHANCE, d),
            _ => {}
        }
    }

    fn clear_steps(&mut self) {
        let s = self.slot();
        for i in 0..STEPS {
            s.steps[i].store(false, Ordering::Relaxed);
            s.locks[i].set(f32::NAN);
        }
        self.touch();
    }

    /// The STEPS layer: tap toggles, hold + knob 2 locks Amount.
    fn handle_steps(&mut self, grid: &[bool; 16], knob2: i32, knob2_press: bool) {
        let s = &self.p.slots[self.focus()];
        let mut changed = false;
        for i in 0..STEPS {
            if grid[i] && !self.prev_grid[i] {
                self.turned[i] = false;
            }
            if grid[i] && knob2 != 0 {
                let base = if s.locks[i].get().is_nan() { s.amount.get() } else { s.locks[i].get() };
                s.locks[i].set((base + knob2 as f32 * 0.02).clamp(0.0, 1.0));
                s.steps[i].store(true, Ordering::Relaxed);
                self.turned[i] = true;
                changed = true;
            }
            if grid[i] && knob2_press {
                s.locks[i].set(f32::NAN);
                self.turned[i] = true;
                changed = true;
            }
            if !grid[i] && self.prev_grid[i] && !self.turned[i] {
                s.steps[i].fetch_xor(true, Ordering::Relaxed);
                changed = true;
            }
        }
        self.prev_grid = *grid;
        if changed {
            self.touch();
        }
    }

    fn knob(&self, c: usize) -> Knob<'_> {
        let s = self.slot();
        match c {
            C_AMOUNT => Knob::F(&s.amount, 0.0, 1.0),
            C_TONE => Knob::F(&s.tone, 0.0, 1.0),
            C_MIX => Knob::F(&s.mix, 0.0, 1.0),
            C_CHANCE => Knob::F(&s.chance, 0.0, 1.0),
            C_TEMPO => Knob::F(&self.p.bpm, 40.0, 240.0),
            C_RUN => Knob::B(&self.p.running),
            _ => Knob::None,
        }
    }

    // ---------------------------------------------------------- save

    fn autosave(&mut self) {
        let Some(path) = self.save_path.clone() else { return };
        let dirty = self.p.dirty.load(Ordering::Relaxed);
        if dirty == self.saved {
            return;
        }
        if dirty != self.seen {
            self.seen = dirty;
            self.quiet = 0;
            return;
        }
        self.quiet += 1;
        if self.quiet < 30 {
            return;
        }
        self.saved = dirty;
        let slots: Vec<serde_json::Value> = self
            .p
            .slots
            .iter()
            .map(|s| {
                serde_json::json!({
                    "effect": EFFECTS[s.fx.load(Ordering::Relaxed).min(EFFECTS.len() - 1)].0,
                    "length": s.length.load(Ordering::Relaxed),
                    "division": DIVS[s.div.load(Ordering::Relaxed).min(DIVS.len() - 1)].0,
                    "steps": s.steps.iter().map(|b| b.load(Ordering::Relaxed)).collect::<Vec<_>>(),
                    "locks": s.locks.iter().map(|l| { let v = l.get(); if v.is_nan() { serde_json::Value::Null } else { serde_json::json!(v) } }).collect::<Vec<_>>(),
                })
            })
            .collect();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, serde_json::to_string_pretty(&serde_json::json!({ "slots": slots })).unwrap_or_default()) {
            eprintln!("Mosaic: couldn't save patterns: {e}");
        }
    }

    fn load(&mut self) {
        let Some(path) = &self.save_path else { return };
        let Ok(text) = std::fs::read_to_string(path) else { return };
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else { return };
        for (s, v) in self.p.slots.iter().zip(doc["slots"].as_array().into_iter().flatten()) {
            if let Some(i) = EFFECTS.iter().position(|e| Some(e.0) == v["effect"].as_str()) {
                s.fx.store(i, Ordering::Relaxed);
            }
            if let Some(l) = v["length"].as_u64() {
                s.length.store((l as usize).clamp(1, STEPS), Ordering::Relaxed);
            }
            if let Some(i) = DIVS.iter().position(|d| Some(d.0) == v["division"].as_str()) {
                s.div.store(i, Ordering::Relaxed);
            }
            for (a, b) in s.steps.iter().zip(v["steps"].as_array().into_iter().flatten()) {
                a.store(b.as_bool().unwrap_or(false), Ordering::Relaxed);
            }
            for (a, b) in s.locks.iter().zip(v["locks"].as_array().into_iter().flatten()) {
                a.set(b.as_f64().map_or(f32::NAN, |x| (x as f32).clamp(0.0, 1.0)));
            }
        }
    }
}

/// What Amount means, in the effect's own units.
fn amount_text(fx: usize, a: f32, bpm: f32) -> String {
    match fx {
        FX_LP | FX_HP => format!("{:.0} Hz", cutoff_hz(a)),
        FX_DELAY => {
            let names = ["1/16", "1/8", "1/8.", "1/4", "1/4.", "1/2"];
            let i = pick(a, DELAY_BEATS.len());
            format!("{} ({:.0} ms)", names[i], DELAY_BEATS[i] * 60_000.0 / bpm)
        }
        FX_REPEAT | FX_REVERSE | FX_GATE => ["1/32", "1/16", "1/8", "1/4", "1/2"][pick(a, SLICE_BEATS.len())].into(),
        FX_CRUSH => format!("{:.0} bits", crush_bits(a)),
        FX_DRIVE => format!("x{:.0}", 1.0 + a * 30.0),
        FX_RING => format!("{:.0} Hz", ring_hz(a)),
        FX_PITCH => format!("{:+} st", pitch_st(a)),
        _ => format!("{:.0}%", a * 100.0),
    }
}

fn pick(a: f32, n: usize) -> usize {
    (a.clamp(0.0, 1.0) * (n - 1) as f32).round() as usize
}
fn cutoff_hz(a: f32) -> f32 {
    40.0 * 2f32.powf(a.clamp(0.0, 1.0) * 9.0)
}
fn crush_bits(a: f32) -> f32 {
    (16.0 - a.clamp(0.0, 1.0) * 14.0).round()
}
fn ring_hz(a: f32) -> f32 {
    30.0 * 2f32.powf(a.clamp(0.0, 1.0) * 8.0)
}
fn pitch_st(a: f32) -> i32 {
    (a.clamp(0.0, 1.0) * 24.0).round() as i32 - 12
}

impl PlayHost for MosaicApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS.len()
    }
    fn kit_label(&self, i: usize) -> String {
        self.label(i % CONTROLS.len())
    }
    fn kit_value(&self, i: usize) -> String {
        self.value(i % CONTROLS.len())
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        let s = self.slot();
        let u = |v: usize, n: usize| v.min(n - 1) as f32 / (n - 1).max(1) as f32;
        Some(match i {
            C_SLOT => u(self.focus(), SLOTS),
            C_EFFECT => u(s.fx.load(Ordering::Relaxed), EFFECTS.len()),
            C_LENGTH => u(s.length.load(Ordering::Relaxed) - 1, STEPS),
            C_DIV => u(s.div.load(Ordering::Relaxed), DIVS.len()),
            _ => return self.knob(i).norm(),
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        matches!(i, C_SLOT | C_EFFECT | C_LENGTH | C_DIV | C_RUN)
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit(i, delta);
    }
    fn kit_reset(&mut self, i: usize) {
        let s = &self.p.slots[self.focus()];
        match i {
            C_AMOUNT => s.amount.set(0.5),
            C_TONE => s.tone.set(0.4),
            C_MIX => s.mix.set(1.0),
            C_CHANCE => s.chance.set(1.0),
            C_LENGTH => s.length.store(STEPS, Ordering::Relaxed),
            C_DIV => s.div.store(DIV_DEFAULT, Ordering::Relaxed),
            C_TEMPO => self.p.bpm.set(120.0),
            C_RUN => self.p.running.store(true, Ordering::Relaxed),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let s = &self.p.slots[self.focus()];
        let pickn = |n: usize| pick(v, n);
        match i {
            C_SLOT => self.p.focus.store(pickn(SLOTS), Ordering::Relaxed),
            C_EFFECT => s.fx.store(pickn(EFFECTS.len()), Ordering::Relaxed),
            C_LENGTH => s.length.store(pickn(STEPS) + 1, Ordering::Relaxed),
            C_DIV => s.div.store(pickn(DIVS.len()), Ordering::Relaxed),
            _ => self.knob(i).set(v),
        }
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        let s = self.slot();
        let l = s.locks[pad].get();
        if l.is_nan() { format!("{}", pad + 1) } else { format!("{}·{:.0}", pad + 1, l * 100.0) }
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, _held: bool) -> PadColor {
        let s = self.slot();
        let len = s.length.load(Ordering::Relaxed);
        if pad >= len {
            return PadColor::Off;
        }
        let running = self.p.running.load(Ordering::Relaxed);
        if running && s.pos.load(Ordering::Relaxed) == pad {
            return PadColor::Yellow;
        }
        if s.steps[pad].load(Ordering::Relaxed) {
            if s.locks[pad].get().is_nan() { PadColor::Green } else { PadColor::Blue }
        } else {
            PadColor::Off
        }
    }
    fn kit_line(&self) -> String {
        if self.p.source.load(Ordering::Relaxed) == NO_SOURCE {
            return "R1: pick a Source".into();
        }
        let tiles: String = self
            .p
            .slots
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let fx = EFFECTS[s.fx.load(Ordering::Relaxed).min(EFFECTS.len() - 1)].0;
                let on = s.engaged.load(Ordering::Relaxed) && s.fx.load(Ordering::Relaxed) != FX_OFF;
                format!("{}{}{} ", if i == self.focus() { ">" } else { "" }, if on { "*" } else { "" }, fx)
            })
            .collect();
        tiles.trim_end().to_string()
    }
}

impl App for MosaicApp {
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
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        self.kit.led_overlay(self)
    }
    fn tick(&mut self, input: &Input) {
        // Holding a step pad on STEPS borrows knob 2 (and its press) for
        // that step's lock, so the kit doesn't also turn its hero control.
        let on_steps = !self.kit.menu && matches!(self.kit.layer(), Layer::Native(0, _));
        let holding = on_steps && input.grid.iter().any(|&g| g);
        let mut kin = *input;
        if holding {
            kin.knob2 = 0;
            kin.knob2_press = false;
        }
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, &kin);
        self.kit = play;
        if step.native.is_some() || on_steps {
            self.handle_steps(&step.input.grid, if holding { input.knob2 } else { 0 }, holding && input.knob2_press);
        } else {
            self.prev_grid = [false; 16];
        }
        let input = &step.input;
        self.list.navigate_input(input, ROWS, self.nav.get() as i32);
        self.edit_row(self.list.selected, input.knob2);
        if input.knob2_press && self.list.selected == ROWS - 1 {
            self.clear_steps();
        }
        self.autosave();
    }
    fn on_exit(&mut self) {
        self.prev_grid = [false; 16];
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if !self.kit.menu {
            if let Some(col) = self.play_column() {
                let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
                kit::draw::column(f, &col, 16, 40, 350, 285, pal);
            }
        } else {
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw(f, 16, 44, 24, r.len(), &r);
        }
        // the four slots' patterns, one row each
        let title = MonoTextStyle::new(&SPLEEN_8X16, INK);
        let small = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        let running = self.p.running.load(Ordering::Relaxed);
        for (si, s) in self.p.slots.iter().enumerate() {
            let y = 48 + si as i32 * 64;
            let fx = s.fx.load(Ordering::Relaxed).min(EFFECTS.len() - 1);
            let style = if si == self.focus() { title } else { MonoTextStyle::new(&SPLEEN_8X16, DIM) };
            Text::new(&format!("{} {}", si + 1, EFFECTS[fx].0), Point::new(392, y), style).draw(f).ok();
            let len = s.length.load(Ordering::Relaxed);
            for i in 0..STEPS {
                let x = 392 + i as i32 * 14;
                let lit = s.steps[i].load(Ordering::Relaxed);
                let c = if i >= len { BG } else if running && s.pos.load(Ordering::Relaxed) == i { ACCENT } else if lit { TILE_ON } else { FAINT };
                Rectangle::new(Point::new(x, y + 8), Size::new(12, 18)).into_styled(PrimitiveStyle::with_fill(c)).draw(f).ok();
                let l = s.locks[i].get();
                if !l.is_nan() && i < len {
                    let h = (l * 16.0) as u32;
                    Rectangle::new(Point::new(x + 4, y + 25 - h as i32), Size::new(4, h.max(1))).into_styled(PrimitiveStyle::with_fill(INK)).draw(f).ok();
                }
            }
            if !s.any_steps() && fx != FX_OFF {
                Text::new("always on", Point::new(392, y + 40), small).draw(f).ok();
            }
        }
        Text::new("pads: steps (hold+knob2 = lock)  D-pad: slot  F2: pads  R1: menu", Point::new(16, 340), small).draw(f).ok();
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let running = self.p.running.load(Ordering::Relaxed);
        // per slot: [name, 16 steps] then [settings, 16 blanks]
        let mut cells: Vec<String> = Vec::new();
        for (si, s) in self.p.slots.iter().enumerate() {
            let fx = s.fx.load(Ordering::Relaxed).min(EFFECTS.len() - 1);
            let len = s.length.load(Ordering::Relaxed);
            let on = fx != FX_OFF && s.engaged.load(Ordering::Relaxed);
            cells.push(format!("{}{} {}", si + 1, if on { "*" } else { " " }, EFFECTS[fx].0));
            for i in 0..STEPS {
                cells.push(
                    if i >= len {
                        String::new()
                    } else if running && s.pos.load(Ordering::Relaxed) == i {
                        "▶".into()
                    } else if s.steps[i].load(Ordering::Relaxed) {
                        if s.locks[i].get().is_nan() { "■".into() } else { "◆".into() }
                    } else {
                        "·".into()
                    },
                );
            }
            cells.push(format!(
                "   {} · {} · {}",
                DIVS[s.div.load(Ordering::Relaxed).min(DIVS.len() - 1)].0,
                if s.any_steps() { format!("{:.0}%", s.chance.get() * 100.0) } else { "always on".into() },
                amount_text(fx, s.amount.get(), self.p.bpm.get())
            ));
            cells.extend(std::iter::repeat_n(String::new(), STEPS));
        }
        let mut col_x = vec![0.0f32];
        col_x.extend((0..STEPS).map(|i| 78.0 + i as f32 * 12.5));
        crate::app::SlintExtra::Grid(crate::app::GridExtra {
            caption: format!("STEP FX / {:.0} BPM", self.p.bpm.get()),
            title: format!("Slot {} · {}", self.focus() + 1, EFFECTS[self.slot().fx.load(Ordering::Relaxed).min(EFFECTS.len() - 1)].0),
            cells,
            col_x,
            highlight: (self.focus() * 2) as i32,
            footer: "■ step  ◆ locked  ▶ playing  * engaged\nHold a step + knob 2 to lock its Amount.".into(),
            meter: -1.0,
        })
    }
    fn running(&self) -> Option<bool> {
        Some(self.p.running.load(Ordering::Relaxed))
    }
    fn toggle_running(&mut self) {
        self.p.running.fetch_xor(true, Ordering::Relaxed);
    }
    fn needs_background_audio(&self) -> bool {
        self.p.source.load(Ordering::Relaxed) != NO_SOURCE
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if self.taken {
            return None;
        }
        self.taken = true;
        Some(Box::new(Processor::new(Arc::clone(&self.p), Arc::clone(&self.bus), 48_000.0)))
    }
}

// ------------------------------------------------------------------ DSP

/// A small Schroeder reverb (four combs, two all-passes).
struct Reverb {
    combs: [Vec<f32>; 4],
    comb_pos: [usize; 4],
    comb_lp: [f32; 4],
    aps: [Vec<f32>; 2],
    ap_pos: [usize; 2],
}

impl Reverb {
    fn new(sr: f32) -> Self {
        let s = sr / 44_100.0;
        let len = |n: f32| vec![0.0; (n * s) as usize + 1];
        Self {
            combs: [len(1557.0), len(1617.0), len(1491.0), len(1422.0)],
            comb_pos: [0; 4],
            comb_lp: [0.0; 4],
            aps: [len(225.0), len(556.0)],
            ap_pos: [0; 2],
        }
    }
    fn tick(&mut self, x: f32, size: f32, damp: f32) -> f32 {
        let fb = 0.7 + size * 0.27;
        let mut out = 0.0;
        for i in 0..4 {
            let b = &mut self.combs[i];
            let p = self.comb_pos[i];
            let y = b[p];
            self.comb_lp[i] = y * (1.0 - damp) + self.comb_lp[i] * damp;
            b[p] = x * 0.25 + self.comb_lp[i] * fb;
            self.comb_pos[i] = (p + 1) % b.len();
            out += y;
        }
        for i in 0..2 {
            let b = &mut self.aps[i];
            let p = self.ap_pos[i];
            let d = b[p];
            let y = -out * 0.5 + d;
            b[p] = out + d * 0.5;
            self.ap_pos[i] = (p + 1) % b.len();
            out = y;
        }
        out
    }
}

struct SlotDsp {
    ring: Vec<f32>,
    w: usize,
    /// Previous effect, to reset state on change.
    fx: usize,
    svf: [f32; 2],
    delay_fb: f32,
    /// Repeat/reverse: the slice caught when the step fired, how long it
    /// is, how far into it we are, and the repeat's decaying level.
    cap: Vec<f32>,
    cap_len: usize,
    cap_phase: usize,
    cap_gain: f32,
    /// Catch a new slice at the next sample.
    trigger: bool,
    crush_hold: f32,
    crush_count: f32,
    drive_lp: f32,
    gate_gain: f32,
    ring_phase: f32,
    pitch_phase: f32,
    reverb: Reverb,
    /// Engaged amount, smoothed (0 = bypassed).
    wet_gain: f32,
    engaged: bool,
    amount: f32,
    // step clock
    last_step: i64,
}

impl SlotDsp {
    fn new(sr: f32) -> Self {
        Self {
            ring: vec![0.0; (RING_SECONDS * sr) as usize],
            w: 0,
            fx: FX_OFF,
            svf: [0.0; 2],
            delay_fb: 0.0,
            cap: vec![0.0; (RING_SECONDS * sr) as usize / 2],
            cap_len: 0,
            cap_phase: 0,
            cap_gain: 1.0,
            trigger: true,
            crush_hold: 0.0,
            crush_count: 0.0,
            drive_lp: 0.0,
            gate_gain: 0.0,
            ring_phase: 0.0,
            pitch_phase: 0.0,
            reverb: Reverb::new(sr),
            wet_gain: 1.0,
            engaged: true,
            amount: 0.5,
            last_step: -1,
        }
    }

    fn read(&self, back: f32) -> f32 {
        let n = self.ring.len();
        let pos = self.w as f32 - back;
        let i = pos.floor();
        let f = pos - i;
        let a = (i as i64).rem_euclid(n as i64) as usize;
        let b = (a + 1) % n;
        self.ring[a] * (1.0 - f) + self.ring[b] * f
    }

    fn reset(&mut self) {
        self.svf = [0.0; 2];
        self.delay_fb = 0.0;
        self.drive_lp = 0.0;
        self.gate_gain = 0.0;
    }

    /// One sample through the effect; `beat_s` = seconds per beat.
    #[allow(clippy::too_many_arguments)]
    fn tick(&mut self, x: f32, fx: usize, a: f32, t: f32, sr: f32, beat_s: f32, beat_frac: f32, running: bool) -> f32 {
        let n = self.ring.len();
        let y = match fx {
            FX_LP | FX_HP => {
                let g = (PI * cutoff_hz(a).min(sr * 0.45) / sr).tan();
                let k = 2.0 - t * 1.85;
                let a1 = 1.0 / (1.0 + g * (g + k));
                let v1 = a1 * (self.svf[0] + g * (x - self.svf[1]));
                let v2 = self.svf[1] + g * v1;
                self.svf = [2.0 * v1 - self.svf[0], 2.0 * v2 - self.svf[1]];
                if fx == FX_LP { v2 } else { x - k * v1 - v2 }
            }
            FX_DELAY => {
                let d = (DELAY_BEATS[pick(a, DELAY_BEATS.len())] * beat_s * sr).min((n - 2) as f32);
                let echo = self.read(d);
                // the ring holds input + feedback for the delay
                self.ring[self.w] = x + echo * t * 0.9;
                self.w = (self.w + 1) % n;
                // the dry signal stays; Mix sets how loud the echoes are
                return x + echo * 0.8;
            }
            FX_REPEAT | FX_REVERSE => {
                let len = ((SLICE_BEATS[pick(a, SLICE_BEATS.len())] * beat_s * sr) as usize).clamp(16, self.cap.len());
                // Always on with the transport stopped: catch afresh every
                // two slices, so it follows the music instead of freezing.
                if !running && self.cap_phase >= 2 * self.cap_len.max(1) {
                    self.trigger = true;
                }
                if self.trigger {
                    self.trigger = false;
                    for i in 0..len {
                        self.cap[i] = self.ring[(self.w + n - len + i) % n];
                    }
                    self.cap_len = len;
                    self.cap_phase = 0;
                    self.cap_gain = 1.0;
                }
                self.ring[self.w] = x;
                self.w = (self.w + 1) % n;
                let l = self.cap_len.max(1);
                let p = self.cap_phase % l;
                self.cap_phase += 1;
                return if fx == FX_REPEAT {
                    if p == l - 1 {
                        self.cap_gain *= 1.0 - t * 0.5;
                    }
                    // tiny fades at the slice ends to avoid clicks
                    let edge = ((p.min(l - 1 - p)) as f32 / 32.0).min(1.0);
                    self.cap[p] * self.cap_gain * edge
                } else {
                    let fade = (p.min(l - 1 - p) as f32 / (l as f32 * (0.02 + t * 0.3))).min(1.0);
                    self.cap[l - 1 - p] * fade
                };
            }
            FX_CRUSH => {
                let step = 1.0 + t * 31.0;
                self.crush_count += 1.0;
                if self.crush_count >= step {
                    self.crush_count -= step;
                    let q = 2f32.powf(crush_bits(a) - 1.0);
                    self.crush_hold = (x * q).round() / q;
                }
                self.crush_hold
            }
            FX_DRIVE => {
                let d = (x * (1.0 + a * 30.0)).tanh();
                let c = 1.0 - (-TAU * (800.0 * 2f32.powf(t * 4.0)) / sr).exp();
                self.drive_lp += c * (d - self.drive_lp);
                self.drive_lp * 0.8
            }
            FX_GATE => {
                let period = SLICE_BEATS[pick(a, SLICE_BEATS.len())];
                let ph = (beat_frac / period).fract();
                let target = if ph < 0.1 + t * 0.8 { 1.0 } else { 0.0 };
                self.gate_gain += (target - self.gate_gain) * (1.0 - (-1.0 / (sr * 0.002)).exp());
                x * self.gate_gain
            }
            FX_RING => {
                self.ring_phase = (self.ring_phase + ring_hz(a) / sr).fract();
                let sine = (self.ring_phase * TAU).sin();
                let square = if self.ring_phase < 0.5 { 1.0 } else { -1.0 };
                x * (sine * (1.0 - t) + square * t * 0.7)
            }
            FX_PITCH => {
                // two crossfaded read heads sweeping through a grain window
                let ratio = 2f32.powf(pitch_st(a) as f32 / 12.0);
                let win = (0.02 + t * 0.1) * sr;
                self.pitch_phase = (self.pitch_phase + (1.0 - ratio) / win).rem_euclid(1.0);
                let p2 = (self.pitch_phase + 0.5).fract();
                let g1 = (self.pitch_phase * PI).sin();
                let g2 = (p2 * PI).sin();
                let out = self.read(self.pitch_phase * win + 1.0) * g1 + self.read(p2 * win + 1.0) * g2;
                self.ring[self.w] = x;
                self.w = (self.w + 1) % n;
                return out;
            }
            FX_REVERB => x + self.reverb.tick(x, a, t * 0.9) * 0.6,
            _ => x,
        };
        // effects that don't use the ring still feed it, so switching to
        // Repeat/Reverse has the recent past ready
        self.ring[self.w] = x;
        self.w = (self.w + 1) % n;
        y
    }

}

struct Processor {
    p: Arc<Shared>,
    bus: Arc<AudioBus>,
    input: Vec<f32>,
    slots: Vec<SlotDsp>,
    beat: f64,
    sr: f32,
    rng: u32,
}

impl Processor {
    fn new(p: Arc<Shared>, bus: Arc<AudioBus>, sr: f32) -> Self {
        Self { p, bus, input: Vec::with_capacity(4096), slots: (0..SLOTS).map(|_| SlotDsp::new(sr)).collect(), beat: 0.0, sr, rng: 0x9e37_79b9 }
    }
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.0);
        if channels == 0 || !rate.is_finite() || rate < 1000.0 {
            return;
        }
        if (rate - self.sr).abs() > 1.0 {
            // buffers were sized for another rate: rebuild (rare: device change)
            self.sr = rate;
            self.slots = (0..SLOTS).map(|_| SlotDsp::new(rate)).collect();
        }
        self.input.clear();
        if let Some(b) = self.bus.get(self.p.source.load(Ordering::Relaxed)) {
            if let Ok(b) = b.try_lock() {
                self.input.extend_from_slice(&b);
            }
        }
        let bpm = self.p.bpm.get().clamp(40.0, 240.0);
        let beat_s = 60.0 / bpm;
        let running = self.p.running.load(Ordering::Relaxed);
        let level = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0);
        let smooth = 1.0 - (-1.0 / (rate * 0.005)).exp();
        let frames = out.len() / channels;
        let beat_inc = (bpm / 60.0 / rate) as f64;
        // per-slot settings for this block
        let mut cfg = [(0usize, 0.0f32, 0.0f32, 0.0f32, 1usize, 0.25f32, 1.0f32, false); SLOTS];
        for (i, s) in self.p.slots.iter().enumerate() {
            cfg[i] = (
                s.fx.load(Ordering::Relaxed).min(EFFECTS.len() - 1),
                s.amount.get(),
                s.tone.get(),
                s.mix.get(),
                s.length.load(Ordering::Relaxed).clamp(1, STEPS),
                DIVS[s.div.load(Ordering::Relaxed).min(DIVS.len() - 1)].1,
                s.chance.get(),
                s.any_steps(),
            );
        }
        for (n, frame) in out.chunks_mut(channels).enumerate().take(frames) {
            let mut x = self.input.get(n).copied().filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-4.0, 4.0);
            if running {
                self.beat += beat_inc;
            }
            for si in 0..SLOTS {
                let (fx, base, tone, mix, len, div, chance, any) = cfg[si];
                // step clock
                if running {
                    let st = (self.beat / div as f64).floor() as i64;
                    if st != self.slots[si].last_step {
                        self.slots[si].last_step = st;
                        let pos = st.rem_euclid(len as i64) as usize;
                        let shared = Arc::clone(&self.p);
                        let s = &shared.slots[si];
                        s.pos.store(pos, Ordering::Relaxed);
                        let lit = s.steps[pos].load(Ordering::Relaxed);
                        let lock = s.locks[pos].get();
                        let fires = !any || (lit && (chance >= 1.0 || self.rand() < chance));
                        self.slots[si].engaged = fires;
                        if fires {
                            self.slots[si].trigger = true;
                        }
                        self.slots[si].amount = if lit && !lock.is_nan() { lock } else { base };
                        s.engaged.store(fires, Ordering::Relaxed);
                    }
                } else {
                    self.slots[si].engaged = true;
                    self.slots[si].amount = base;
                    self.slots[si].last_step = -1;
                }
                let d = &mut self.slots[si];
                if fx != d.fx {
                    d.fx = fx;
                    d.reset();
                }
                let amount = (d.amount + self.p.slots[si].cv.get()).clamp(0.0, 1.0);
                let target = if d.engaged && fx != FX_OFF { mix } else { 0.0 };
                d.wet_gain += (target - d.wet_gain) * smooth;
                let beat_frac = self.beat as f32;
                let wet = d.tick(x, fx, amount, tone, rate, beat_s, beat_frac, running);
                x = x + (wet - x) * d.wet_gain;
            }
            let y = (x * level).tanh();
            frame.fill(y);
        }
        if let Ok(mut b) = self.p.output.try_lock() {
            b.clear();
            b.extend(out.chunks(channels).map(|f| f[0]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (MosaicApp, Arc<Mutex<Vec<f32>>>) {
        let bus = Arc::new(AudioBus::new());
        let src = bus.register("Synth");
        let mut a = MosaicApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), bus, Arc::new(MixerBus::new()));
        a.p.source.store(0, Ordering::Relaxed);
        (a, src)
    }

    fn run(p: &mut Box<dyn AudioProcessor>, src: &Arc<Mutex<Vec<f32>>>, blocks: usize, hz: f32) -> Vec<f32> {
        let mut all = Vec::new();
        for b in 0..blocks {
            *src.lock().unwrap() = (0..256).map(|n| (TAU * hz * (n + b * 256) as f32 / 48_000.0).sin() * 0.5).collect();
            let mut out = vec![0.0; 512];
            p.process(&mut out, 2, 48_000.0);
            all.extend(out.iter().step_by(2));
        }
        all
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn only(a: &MosaicApp, fx: usize) {
        for (i, s) in a.p.slots.iter().enumerate() {
            s.fx.store(if i == 0 { fx } else { FX_OFF }, Ordering::Relaxed);
        }
    }

    #[test]
    fn every_effect_is_finite_and_audible_at_extremes() {
        for fx in 1..EFFECTS.len() {
            for (amt, tone) in [(0.0, 0.0), (1.0, 1.0), (0.5, 0.5)] {
                let (a, src) = fixture();
                only(&a, fx);
                a.p.slots[0].amount.set(amt);
                a.p.slots[0].tone.set(tone);
                let mut a = a;
                let mut p = a.audio_processor().unwrap();
                let out = run(&mut p, &src, 450, 330.0);
                assert!(out.iter().all(|v| v.is_finite() && v.abs() <= 1.0), "{} {amt} {tone}", EFFECTS[fx].0);
                // (a high-pass at 20 kHz on a 330 Hz tone is rightly silent)
                if !(fx == FX_HP && amt == 1.0) {
                    assert!(rms(&out) > 0.003, "{} {amt} {tone} is silent", EFFECTS[fx].0);
                }
            }
        }
    }

    #[test]
    fn a_lit_step_engages_the_effect_and_an_unlit_one_bypasses_it() {
        let (mut a, src) = fixture();
        only(&a, FX_LP);
        let s = &a.p.slots[0];
        s.amount.set(0.0); // 40 Hz: kills a 2 kHz tone
        s.div.store(7, Ordering::Relaxed); // 1 bar per step: long, steady steps
        s.length.store(2, Ordering::Relaxed);
        s.steps[0].store(true, Ordering::Relaxed);
        let mut p = a.audio_processor().unwrap();
        // 120 BPM: a bar is 2 s = 375 blocks of 256
        let out = run(&mut p, &src, 750, 2000.0);
        let first = rms(&out[48_000..72_000]);
        let second = rms(&out[120_000..144_000]);
        assert!(first < 0.05, "step 1 lit: filtered ({first})");
        assert!(second > 0.3, "step 2 unlit: dry ({second})");
    }

    #[test]
    fn a_locked_step_uses_its_own_amount() {
        let (mut a, src) = fixture();
        only(&a, FX_LP);
        let s = &a.p.slots[0];
        s.amount.set(1.0); // wide open
        s.div.store(7, Ordering::Relaxed);
        s.length.store(1, Ordering::Relaxed);
        s.steps[0].store(true, Ordering::Relaxed);
        s.locks[0].set(0.0); // locked shut
        let mut p = a.audio_processor().unwrap();
        let out = run(&mut p, &src, 200, 2000.0);
        assert!(rms(&out[24_000..]) < 0.05, "the lock wins over the base amount");
    }

    #[test]
    fn pads_toggle_steps_and_hold_plus_knob_locks() {
        let (mut a, _) = fixture();
        assert!(a.play_column().is_some(), "opens on the play view");
        let pad = |i: usize| Input { grid: std::array::from_fn(|k| k == i), ..Default::default() };
        a.tick(&pad(3));
        a.tick(&Input::default());
        assert!(a.p.slots[0].steps[3].load(Ordering::Relaxed), "tap lights the step");
        let before = a.p.slots[0].amount.get();
        a.tick(&pad(5));
        a.tick(&Input { knob2: -5, ..pad(5) });
        a.tick(&Input::default());
        let s = &a.p.slots[0];
        assert!(s.steps[5].load(Ordering::Relaxed) && (s.locks[5].get() - (before - 0.1)).abs() < 1e-4, "hold + knob 2 locks the step");
        assert_eq!(s.amount.get(), before, "and doesn't turn the hero control");
        a.tick(&pad(5));
        a.tick(&Input { knob2_press: true, ..pad(5) });
        a.tick(&Input::default());
        assert!(a.p.slots[0].locks[5].get().is_nan() && a.p.slots[0].steps[5].load(Ordering::Relaxed), "knob 2 press clears the lock");
        a.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(a.focus(), 1, "D-pad up focuses the next slot");
    }

    #[test]
    fn repeat_loops_the_slice_it_caught() {
        let (mut a, src) = fixture();
        only(&a, FX_REPEAT);
        a.p.running.store(false, Ordering::Relaxed); // engaged throughout
        a.p.slots[0].tone.set(0.0);
        let mut p = a.audio_processor().unwrap();
        run(&mut p, &src, 200, 300.0);
        // the source goes silent; the repeat keeps sounding
        let mut tail = Vec::new();
        for _ in 0..30 {
            let mut out = vec![0.0; 512];
            *src.lock().unwrap() = vec![0.0; 256];
            p.process(&mut out, 2, 48_000.0);
            tail.extend(out.iter().step_by(2));
        }
        assert!(rms(&tail) > 0.05, "{}", rms(&tail));
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(MosaicApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}
