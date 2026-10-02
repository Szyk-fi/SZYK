//! Squeeze: a compressor, inspired by the idea of Polyend's Press (a
//! dedicated compressor for a performance rig). Original design, not a
//! model of Press's circuit.
//!
//! A feed-forward compressor: the detector follows the input (or another
//! app's output, for ducking -- pick Pulsar Kick as the Sidechain and
//! anything pumps to the kick), through a high-pass so lows don't pump
//! it, into a soft-knee (6 dB) gain computer. Gain reduction attacks and
//! releases in the dB domain, like a VCA compressor's control voltage.
//! Then makeup gain and a parallel (New York) mix.
//!
//! Pads: the play view opens on Throws (SLAM, DUCK, PUMP, ...), then
//! Controls and Moments.

use crate::{
    app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes, Throw},
    app::{App, Input},
    audio::AudioProcessor,
    audio_bus::{cycle_source, AudioBus, NO_SOURCE},
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    paramlist::ParamList,
    spleen_fonts::SPLEEN_6X12,
    util::AtomicF32,
};
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, text::Text};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "Squeeze";

// Own palette: warm amber VU on charcoal.
const BG: Rgb565 = Rgb565::new(3, 6, 3);
const INK: Rgb565 = Rgb565::new(30, 52, 18);
const ACCENT: Rgb565 = Rgb565::new(31, 38, 4);
const DIM: Rgb565 = Rgb565::new(16, 30, 10);
const FAINT: Rgb565 = Rgb565::new(5, 10, 4);

/// Knee width, dB.
const KNEE_DB: f32 = 6.0;

/// The controls, in play-view order. Ranges are the knobs' ends.
const CONTROLS: [(&str, f32, f32, f32); 7] = [
    // name, min, max, default
    ("Threshold", -48.0, 0.0, -18.0),
    ("Ratio", 1.0, 20.0, 4.0),
    ("Attack", 0.1, 100.0, 10.0),
    ("Release", 20.0, 1000.0, 150.0),
    ("Makeup", 0.0, 24.0, 6.0),
    ("Mix", 0.0, 1.0, 1.0),
    ("SC High-pass", 20.0, 400.0, 90.0),
];
const C_THRESH: usize = 0;
const C_RATIO: usize = 1;
const C_ATTACK: usize = 2;
const C_RELEASE: usize = 3;
const C_MAKEUP: usize = 4;
const C_MIX: usize = 5;
const C_HPF: usize = 6;
/// Play-view controls past the seven above.
const C_ON: usize = 7;
const N_CONTROLS: usize = 8;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "squeeze",
        layers: vec![Layer::Throws, Layer::Controls, Layer::Moments],
        hero: vec![[C_THRESH, C_RATIO], [C_ATTACK, C_RELEASE], [C_MAKEUP, C_MIX]],
        browse: Some(C_RATIO),
        // Stick down pulls the threshold into the signal (more squash);
        // the left hand rides the makeup, the right the parallel blend.
        routes: Routes { stick_x: Some(C_RELEASE), stick_y: Some(C_THRESH), hand_l: Some(C_MAKEUP), hand_r: Some(C_MIX) },
        throws: vec![
            Throw { control: C_THRESH, to: 0.0, label: "SLAM" },
            Throw { control: C_RATIO, to: 1.0, label: "LIMIT" },
            Throw { control: C_RELEASE, to: 0.05, label: "PUMP" },
            Throw { control: C_RELEASE, to: 1.0, label: "SMOOTH" },
            Throw { control: C_ATTACK, to: 1.0, label: "PUNCH" },
            Throw { control: C_MIX, to: 0.5, label: "NYC" },
            Throw { control: C_MIX, to: 0.0, label: "DRY" },
            Throw { control: C_ON, to: 0.0, label: "BYPASS" },
        ],
        midi_to_pads: false,
        own_expression: false,
    }
}

struct Shared {
    values: [AtomicF32; 7],
    cv: [Arc<AtomicF32>; 3],
    enabled: AtomicBool,
    source: AtomicUsize,
    sidechain: AtomicUsize,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    /// Telemetry: gain reduction (dB, >= 0), input and output peak.
    gr_db: AtomicF32,
    in_peak: AtomicF32,
    out_peak: AtomicF32,
}

impl Shared {
    fn value(&self, i: usize) -> f32 {
        let (_, lo, hi, _) = CONTROLS[i];
        let cv = match i {
            C_THRESH => self.cv[0].get() * 48.0,
            C_RATIO => self.cv[1].get() * 19.0,
            C_MIX => self.cv[2].get(),
            _ => 0.0,
        };
        (self.values[i].get() + cv).clamp(lo, hi)
    }
}

pub struct SqueezeApp {
    p: Arc<Shared>,
    bus: Arc<AudioBus>,
    own: usize,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
}

/// Menu rows: Source, Sidechain, then the seven controls.
const ROWS: usize = 9;

impl SqueezeApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let own = bus.index_of(APP_NAME).unwrap_or(NO_SOURCE);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        Self {
            p: Arc::new(Shared {
                values: std::array::from_fn(|i| AtomicF32::new(CONTROLS[i].3)),
                cv: [mods.register(format!("{APP_NAME}: Threshold")), mods.register(format!("{APP_NAME}: Ratio")), mods.register(format!("{APP_NAME}: Mix"))],
                enabled: AtomicBool::new(true),
                source: AtomicUsize::new(NO_SOURCE),
                sidechain: AtomicUsize::new(NO_SOURCE),
                mix_level,
                ext_mix_level,
                output,
                gr_db: AtomicF32::new(0.0),
                in_peak: AtomicF32::new(0.0),
                out_peak: AtomicF32::new(0.0),
            }),
            bus,
            own,
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
        }
    }

    fn fmt(i: usize, v: f32) -> String {
        match i {
            C_THRESH => format!("{v:.1} dB"),
            C_RATIO => {
                if v >= 19.5 {
                    "limit".into()
                } else {
                    format!("{v:.1}:1")
                }
            }
            C_ATTACK => format!("{v:.1} ms"),
            C_RELEASE => format!("{v:.0} ms"),
            C_MAKEUP => format!("+{v:.1} dB"),
            C_MIX => format!("{:.0}% wet", v * 100.0),
            _ => format!("{v:.0} Hz"),
        }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        let mut r = vec![
            ("Source".to_string(), self.bus.source_name(self.p.source.load(Ordering::Relaxed)), false),
            (
                "Sidechain".to_string(),
                match self.p.sidechain.load(Ordering::Relaxed) {
                    NO_SOURCE => "own input".to_string(),
                    i => self.bus.source_name(i),
                },
                false,
            ),
        ];
        for (i, c) in CONTROLS.iter().enumerate() {
            r.push((c.0.to_string(), Self::fmt(i, self.p.value(i)), false));
        }
        r
    }

    fn step_source(&self, cur: usize, d: i32) -> usize {
        let mut s = cycle_source(cur, d.signum(), self.bus.len());
        if s == self.own {
            s = cycle_source(s, d.signum(), self.bus.len());
        }
        s
    }

    fn edit_row(&mut self, row: usize, d: i32) {
        if d == 0 {
            return;
        }
        match row {
            0 => self.p.source.store(self.step_source(self.p.source.load(Ordering::Relaxed), d), Ordering::Relaxed),
            1 => self.p.sidechain.store(self.step_source(self.p.sidechain.load(Ordering::Relaxed), d), Ordering::Relaxed),
            r if r < ROWS => self.edit_control(r - 2, d),
            _ => {}
        }
    }

    fn edit_control(&mut self, i: usize, d: i32) {
        let (_, lo, hi, _) = CONTROLS[i];
        let a = &self.p.values[i];
        let sens = self.sensitivity.get().max(0.01) * 10.0;
        // Attack/release/HPF move in proportion (log-feel); the rest linearly.
        let next = match i {
            C_ATTACK | C_RELEASE | C_HPF => a.get() * (1.0 + 0.04 * sens).powi(d),
            _ => a.get() + d as f32 * (hi - lo) * 0.01 * sens,
        };
        a.set(next.clamp(lo, hi));
    }

    fn knob(&self, i: usize) -> Knob<'_> {
        match i {
            C_ON => Knob::B(&self.p.enabled),
            i if i < CONTROLS.len() => Knob::F(&self.p.values[i], CONTROLS[i].1, CONTROLS[i].2),
            _ => Knob::None,
        }
    }
}

impl PlayHost for SqueezeApp {
    fn kit_control_count(&self) -> usize {
        N_CONTROLS
    }
    fn kit_label(&self, i: usize) -> String {
        if i == C_ON { "Comp on".into() } else { CONTROLS[i % CONTROLS.len()].0.into() }
    }
    fn kit_value(&self, i: usize) -> String {
        if i == C_ON {
            (if self.p.enabled.load(Ordering::Relaxed) { "on" } else { "bypassed" }).into()
        } else {
            Self::fmt(i, self.p.value(i))
        }
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        self.knob(i).norm()
    }
    fn kit_stepped(&self, i: usize) -> bool {
        self.knob(i).stepped()
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        if i == C_ON {
            if delta != 0 {
                self.p.enabled.store(delta > 0, Ordering::Relaxed);
            }
        } else {
            self.edit_control(i, delta);
        }
    }
    fn kit_reset(&mut self, i: usize) {
        if i == C_ON {
            self.p.enabled.store(true, Ordering::Relaxed);
        } else if i < CONTROLS.len() {
            self.p.values[i].set(CONTROLS[i].3);
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        self.knob(i).set(v);
    }
    fn kit_line(&self) -> String {
        if self.p.source.load(Ordering::Relaxed) == NO_SOURCE {
            "R1: pick a Source".into()
        } else if !self.p.enabled.load(Ordering::Relaxed) {
            "Bypassed".into()
        } else {
            format!("GR {:.1} dB", self.p.gr_db.get())
        }
    }
}

impl App for SqueezeApp {
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
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let input = &step.input;
        self.list.navigate_input(input, ROWS, self.nav.get() as i32);
        self.edit_row(self.list.selected, input.knob2);
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        let dim = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        if !self.kit.menu {
            if let Some(col) = self.play_column() {
                let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
                kit::draw::column(f, &col, 16, 40, 350, 285, pal);
            }
        } else {
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw(f, 16, 44, 24, r.len(), &r);
        }
        // gain-reduction meter on the right: 0..24 dB down from the top
        let gr = self.p.gr_db.get().clamp(0.0, 24.0);
        let h = (gr / 24.0 * 260.0) as u32;
        embedded_graphics::primitives::Rectangle::new(Point::new(420, 40), Size::new(40, 260))
            .into_styled(embedded_graphics::primitives::PrimitiveStyle::with_stroke(DIM, 1))
            .draw(f)
            .ok();
        embedded_graphics::primitives::Rectangle::new(Point::new(421, 41), Size::new(38, h))
            .into_styled(embedded_graphics::primitives::PrimitiveStyle::with_fill(ACCENT))
            .draw(f)
            .ok();
        Text::new(&format!("GR {gr:.1} dB"), Point::new(410, 318), MonoTextStyle::new(&SPLEEN_6X12, INK)).draw(f).ok();
        Text::new("knobs: threshold / ratio   F2: pads   F3: bypass   R1: menu", Point::new(16, 340), dim).draw(f).ok();
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_levels(&mut self, visible: usize) -> Vec<Option<f32>> {
        // the meter rides on the threshold row: gain reduction out of 24 dB
        let mut v = vec![None; visible];
        if let Some(x) = v.get_mut(2) {
            *x = Some((self.p.gr_db.get() / 24.0).clamp(0.0, 1.0));
        }
        v
    }
    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let bar = |v: f32| {
            let n = (v.clamp(0.0, 1.0) * 30.0).round() as usize;
            format!("{}{}", "|".repeat(n), "·".repeat(30 - n))
        };
        let gr = self.p.gr_db.get();
        let thresh = self.p.value(C_THRESH);
        crate::app::SlintExtra::Grid(crate::app::GridExtra {
            caption: "COMPRESSOR / GAIN REDUCTION".into(),
            title: if self.p.enabled.load(Ordering::Relaxed) { format!("GR {gr:.1} dB") } else { "Bypassed".into() },
            cells: {
                let src = self.bus.source_name(self.p.source.load(Ordering::Relaxed));
                let sc = match self.p.sidechain.load(Ordering::Relaxed) { NO_SOURCE => "own input".to_string(), i => self.bus.source_name(i) };
                [
                    ["IN".to_string(), bar(self.p.in_peak.get())],
                    ["GR".into(), bar(gr / 24.0)],
                    ["OUT".into(), bar(self.p.out_peak.get())],
                    [String::new(), String::new()],
                    ["threshold".into(), Self::fmt(C_THRESH, thresh)],
                    ["ratio".into(), Self::fmt(C_RATIO, self.p.value(C_RATIO))],
                    ["attack".into(), Self::fmt(C_ATTACK, self.p.value(C_ATTACK))],
                    ["release".into(), Self::fmt(C_RELEASE, self.p.value(C_RELEASE))],
                    ["makeup".into(), Self::fmt(C_MAKEUP, self.p.value(C_MAKEUP))],
                    ["mix".into(), Self::fmt(C_MIX, self.p.value(C_MIX))],
                    [String::new(), String::new()],
                    ["source".into(), src],
                    ["sidechain".into(), sc],
                ]
                .concat()
            },
            col_x: vec![0.0, 70.0],
            highlight: -1,
            footer: "Pick a Sidechain to duck under another app (e.g. Pulsar Kick).".into(),
            meter: (gr / 24.0).clamp(0.0, 1.0),
        })
    }
    fn transport_action(&self) -> Option<&'static str> {
        Some(if self.p.enabled.load(Ordering::Relaxed) { "BYPASS" } else { "ENABLE" })
    }
    fn toggle_running(&mut self) {
        self.p.enabled.fetch_xor(true, Ordering::Relaxed);
    }
    fn needs_background_audio(&self) -> bool {
        self.p.source.load(Ordering::Relaxed) != NO_SOURCE
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if self.taken {
            return None;
        }
        self.taken = true;
        Some(Box::new(Processor { p: Arc::clone(&self.p), bus: Arc::clone(&self.bus), input: Vec::with_capacity(4096), side: Vec::with_capacity(4096), env_db: -120.0, gr_db: 0.0, hp: [0.0; 2], bypass: 0.0 }))
    }
}

struct Processor {
    p: Arc<Shared>,
    bus: Arc<AudioBus>,
    input: Vec<f32>,
    side: Vec<f32>,
    /// Detector level, dB.
    env_db: f32,
    /// Smoothed gain reduction, dB (>= 0).
    gr_db: f32,
    /// Sidechain high-pass state (x1, y1).
    hp: [f32; 2],
    /// 1 = fully bypassed (smoothed).
    bypass: f32,
}

/// Soft-knee static curve: dB of gain reduction for a level `over` dB
/// above threshold.
pub fn gain_reduction(level_db: f32, thresh_db: f32, ratio: f32) -> f32 {
    let over = level_db - thresh_db;
    let slope = 1.0 - 1.0 / ratio.max(1.0);
    if over <= -KNEE_DB / 2.0 {
        0.0
    } else if over >= KNEE_DB / 2.0 {
        over * slope
    } else {
        slope * (over + KNEE_DB / 2.0).powi(2) / (2.0 * KNEE_DB)
    }
}

fn copy_bus(bus: &AudioBus, idx: usize, into: &mut Vec<f32>) {
    into.clear();
    if idx == NO_SOURCE {
        return;
    }
    if let Some(b) = bus.get(idx) {
        if let Ok(b) = b.try_lock() {
            into.extend_from_slice(&b);
        }
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.0);
        if channels == 0 || !rate.is_finite() || rate < 1000.0 {
            return;
        }
        copy_bus(&self.bus, self.p.source.load(Ordering::Relaxed), &mut self.input);
        let sc_idx = self.p.sidechain.load(Ordering::Relaxed);
        copy_bus(&self.bus, sc_idx, &mut self.side);
        let external = sc_idx != NO_SOURCE;
        let thresh = self.p.value(C_THRESH);
        let ratio = self.p.value(C_RATIO);
        let ratio = if ratio >= 19.5 { 1000.0 } else { ratio };
        let att = (-1.0 / (rate * self.p.value(C_ATTACK) * 0.001)).exp();
        let rel = (-1.0 / (rate * self.p.value(C_RELEASE) * 0.001)).exp();
        let makeup = 10f32.powf(self.p.value(C_MAKEUP) / 20.0);
        let mix = self.p.value(C_MIX);
        let hp_c = (-std::f32::consts::TAU * self.p.value(C_HPF) / rate).exp();
        let level = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0);
        let target_bypass = if self.p.enabled.load(Ordering::Relaxed) { 0.0 } else { 1.0 };
        let bp_step = 1.0 / (rate * 0.01);
        // Peak detector: fast attack (~0.5 ms), release ~ 30 ms; the
        // musical timing is in the gain-reduction smoothing below.
        let det_att = (-1.0 / (rate * 0.0005)).exp();
        let det_rel = (-1.0 / (rate * 0.03)).exp();
        let (mut in_peak, mut out_peak, mut max_gr) = (0.0f32, 0.0f32, 0.0f32);
        for (n, frame) in out.chunks_mut(channels).enumerate() {
            let x = self.input.get(n).copied().filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-4.0, 4.0);
            let s = if external { self.side.get(n).copied().filter(|v| v.is_finite()).unwrap_or(0.0) } else { x };
            // sidechain high-pass
            let hp = hp_c * (self.hp[1] + s - self.hp[0]);
            self.hp = [s, hp];
            let lvl = 20.0 * hp.abs().max(1e-6).log10();
            let c = if lvl > self.env_db { det_att } else { det_rel };
            self.env_db = lvl + c * (self.env_db - lvl);
            let target = gain_reduction(self.env_db, thresh, ratio);
            let c = if target > self.gr_db { att } else { rel };
            self.gr_db = target + c * (self.gr_db - target);
            let wet = x * 10f32.powf(-self.gr_db / 20.0) * makeup;
            let comp = x * (1.0 - mix) + wet * mix;
            self.bypass += (target_bypass - self.bypass).clamp(-bp_step, bp_step);
            let y = (comp * (1.0 - self.bypass) + x * self.bypass).tanh() * level;
            in_peak = in_peak.max(x.abs());
            out_peak = out_peak.max(y.abs());
            max_gr = max_gr.max(self.gr_db);
            frame.fill(y);
        }
        self.p.gr_db.set(max_gr);
        self.p.in_peak.set(in_peak);
        self.p.out_peak.set(out_peak);
        if let Ok(mut b) = self.p.output.try_lock() {
            b.clear();
            b.extend(out.chunks(channels).map(|f| f[0]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (SqueezeApp, Arc<Mutex<Vec<f32>>>, Arc<Mutex<Vec<f32>>>) {
        let bus = Arc::new(AudioBus::new());
        let src = bus.register("Loud");
        let kick = bus.register("Kick");
        let a = SqueezeApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), bus, Arc::new(MixerBus::new()));
        (a, src, kick)
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn run(p: &mut Box<dyn AudioProcessor>, src: &Arc<Mutex<Vec<f32>>>, amp: f32, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for b in 0..blocks {
            *src.lock().unwrap() = (0..256).map(|n| (std::f32::consts::TAU * 220.0 * (n + b * 256) as f32 / 48_000.0).sin() * amp).collect();
            let mut out = vec![0.0; 512];
            p.process(&mut out, 2, 48_000.0);
            all.extend(out.iter().step_by(2));
        }
        all
    }

    #[test]
    fn the_static_curve_has_a_soft_knee_and_the_right_slope() {
        assert_eq!(gain_reduction(-30.0, -18.0, 4.0), 0.0, "well below threshold: untouched");
        assert!((gain_reduction(-6.0, -18.0, 4.0) - 9.0).abs() < 1e-4, "12 dB over at 4:1 -> 9 dB down");
        let at = gain_reduction(-18.0, -18.0, 4.0);
        assert!(at > 0.0 && at < 1.5, "inside the knee: gentle ({at})");
    }

    #[test]
    fn loud_input_is_turned_down_and_quiet_left_alone() {
        let (mut a, src, _) = fixture();
        a.p.source.store(0, Ordering::Relaxed);
        a.p.values[C_MAKEUP].set(0.0);
        let mut p = a.audio_processor().unwrap();
        let loud = run(&mut p, &src, 0.9, 200);
        let tail = &loud[loud.len() - 4800..];
        assert!(rms(tail) < 0.9 * 0.707 * 0.6, "loud sine is compressed ({})", rms(tail));
        assert!(a.p.gr_db.get() > 4.0);
        let quiet = run(&mut p, &src, 0.02, 400);
        let tail = &quiet[quiet.len() - 4800..];
        assert!((rms(tail) - 0.02 * 0.707).abs() < 0.003, "quiet passes through ({})", rms(tail));
    }

    #[test]
    fn an_external_sidechain_ducks_the_source() {
        let (mut a, src, kick) = fixture();
        a.p.source.store(0, Ordering::Relaxed);
        a.p.sidechain.store(1, Ordering::Relaxed);
        a.p.values[C_MAKEUP].set(0.0);
        a.p.values[C_THRESH].set(-30.0);
        a.p.values[C_RATIO].set(10.0);
        let mut p = a.audio_processor().unwrap();
        *kick.lock().unwrap() = vec![0.0; 256];
        let open = run(&mut p, &src, 0.1, 100);
        // a 150 Hz thump (DC would be removed by the sidechain high-pass)
        *kick.lock().unwrap() = (0..256).map(|n| (std::f32::consts::TAU * 150.0 * n as f32 / 48_000.0).sin() * 0.8).collect();
        let ducked = run(&mut p, &src, 0.1, 100);
        assert!(rms(&ducked[ducked.len() - 4800..]) < rms(&open[open.len() - 4800..]) * 0.5, "ducks under the kick");
    }

    #[test]
    fn bypass_and_unpatched_behave() {
        let (mut a, src, _) = fixture();
        let mut p = a.audio_processor().unwrap();
        assert!(run(&mut p, &src, 0.9, 10).iter().all(|v| *v == 0.0), "no source: silent");
        a.p.source.store(0, Ordering::Relaxed);
        a.toggle_running();
        let x = run(&mut p, &src, 0.5, 50);
        assert!((rms(&x[x.len() - 4800..]) - (0.5f32).tanh() * 0.68).abs() < 0.05, "bypassed: dry (soft-clipped)");
    }

    #[test]
    fn opens_on_the_play_view_with_threshold_on_knob_1() {
        let (mut a, _, _) = fixture();
        assert!(a.play_column().is_some());
        let before = a.p.values[C_THRESH].get();
        a.tick(&Input { knob1: 2, ..Default::default() });
        assert!(a.p.values[C_THRESH].get() > before);
        a.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(a.play_column().is_none(), "R1 opens the menu");
    }
}
