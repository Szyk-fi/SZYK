//! Independent mono AudioBus filter. UI coordinates map to normalized parameters;
//! DSP uses a smoothed topology-preserving state-variable filter.
use crate::{
    app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes, Throw},
    app::{App, Input, SlintExtra, VectorFilterExtra},
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
const MODES: [&str; 3] = ["Low pass", "Band pass", "High pass"];
/// X (cutoff), Y (resonance), Z (drive) on a fresh start -- shared by
/// the constructor and the play view's reset.
const DEFAULT_XYZ: [f32; 3] = [0.65, 0.2, 0.1];

/// The play view's controls: the cube's three axes first, then wet,
/// the filter type and the on/bypass switch. Source stays in the menu.
const CONTROLS: [&str; 6] = ["Cutoff", "Resonance", "Drive", "Wet", "Filter type", "Filter on"];
const C_CUTOFF: usize = 0;
const C_RESONANCE: usize = 1;
const C_DRIVE: usize = 2;
const C_WET: usize = 3;
const C_MODE: usize = 4;
const C_ENABLED: usize = 5;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "vector_filter",
        // The filter has never read the pads, so there's no native layer
        // to keep: an effect opens on Throws.
        layers: vec![Layer::Throws, Layer::Controls, Layer::Moments],
        hero: vec![[C_CUTOFF, C_RESONANCE], [C_DRIVE, C_WET]],
        browse: Some(C_MODE),
        // The stick *is* the cube's X/Y face: cutoff across, resonance up.
        // The left hand pushes Z (drive into the filter); the right
        // sweeps the cutoff open, theremin-style, on top of the stick.
        routes: Routes { stick_x: Some(C_CUTOFF), stick_y: Some(C_RESONANCE), hand_l: Some(C_DRIVE), hand_r: Some(C_CUTOFF) },
        throws: vec![
            Throw { control: C_CUTOFF, to: 1.0, label: "OPEN" },
            Throw { control: C_CUTOFF, to: 0.0, label: "CLOSE" },
            Throw { control: C_RESONANCE, to: 1.0, label: "SCREAM" },
            Throw { control: C_DRIVE, to: 1.0, label: "DRIVE" },
            Throw { control: C_WET, to: 0.0, label: "DRY" },
            Throw { control: C_MODE, to: 0.5, label: "BAND" },
            Throw { control: C_MODE, to: 1.0, label: "HIGH" },
            Throw { control: C_ENABLED, to: 0.0, label: "BYPASS" },
        ],
        midi_to_pads: false,
        own_expression: false,
    }
}

// --- Vector Filter's own palette for the embedded-graphics play column:
// phosphor cyan on deep navy, a vector-display look. ---
const VF_BG: Rgb565 = Rgb565::new(1, 4, 8);
const VF_INK: Rgb565 = Rgb565::new(24, 56, 30);
const VF_ACCENT: Rgb565 = Rgb565::new(5, 50, 28);
const VF_DIM: Rgb565 = Rgb565::new(9, 24, 17);
const VF_FAINT: Rgb565 = Rgb565::new(3, 9, 13);
struct Shared {
    xyz: [AtomicF32; 3],
    cv: [Arc<AtomicF32>; 3],
    wet: AtomicF32,
    enabled: AtomicBool,
    mode: AtomicUsize,
    source: AtomicUsize,
    mix: Arc<AtomicF32>,
    ext: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    wave: Mutex<Vec<f32>>,
}
pub struct VectorFilterApp {
    p: Arc<Shared>,
    bus: Arc<AudioBus>,
    own: usize,
    list: ParamList,
    nav: Arc<AtomicF32>,
    taken: bool,
    /// The shared play view (play_kit.rs).
    kit: PlayKit,
}
impl VectorFilterApp {
    pub fn new(
        bus: Arc<AudioBus>,
        mods: Arc<ModBus>,
        mixer: Arc<MixerBus>,
        nav: Arc<AtomicF32>,
    ) -> Self {
        let output = bus.register("Vector Filter");
        let own=bus.index_of("Vector Filter").unwrap();
        let (mix, ext) = mixer.register("Vector Filter", &mods);
        Self {
            p: Arc::new(Shared {
                xyz: DEFAULT_XYZ.map(AtomicF32::new),
                cv: [
                    mods.register("Vector Filter: Cutoff"),
                    mods.register("Vector Filter: Resonance"),
                    mods.register("Vector Filter: Drive"),
                ],
                wet: AtomicF32::new(1.),
                enabled: AtomicBool::new(true),
                mode: AtomicUsize::new(0),
                source: AtomicUsize::new(NO_SOURCE),
                mix,
                ext,
                output,
                wave: Mutex::new(vec![0.; 64]),
            }),
            bus,
            own,
            list: ParamList::new(),
            nav,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
        }
    }
    fn xyz(&self) -> [f32; 3] {
        std::array::from_fn(|i| (self.p.xyz[i].get() + self.p.cv[i].get()).clamp(0., 1.))
    }
    fn rows(&self) -> Vec<(String, String, bool)> {
        let p = self.xyz();
        vec![
            (
                "Source".into(),
                self.bus.source_name(self.p.source.load(Ordering::Relaxed)),
                false,
            ),
            (
                "X · Cutoff".into(),
                format!("{:.0} Hz", 40. * 400f32.powf(p[0])),
                false,
            ),
            (
                "Y · Resonance".into(),
                format!("Q {:.2}", 0.5 + 7.5 * p[1]),
                false,
            ),
            ("Z · Drive".into(), format!("{:.1}×", 1. + 7. * p[2]), false),
            (
                "Wet".into(),
                format!("{:.0}%", self.p.wet.get() * 100.),
                false,
            ),
            (
                "Filter type".into(),
                MODES[self.p.mode.load(Ordering::Relaxed)].into(),
                false,
            ),
        ]
    }
    /// One menu row's knob-2 edit; the play view reuses it so both
    /// move each control in the same steps.
    fn edit_row(&mut self, row: usize, d: i32) {
        if d == 0 {
            return;
        }
        match row {
            0 => {
                let mut s = cycle_source(
                    self.p.source.load(Ordering::Relaxed),
                    d.signum(),
                    self.bus.len(),
                );
                if s == self.own {
                    s = cycle_source(s, d.signum(), self.bus.len());
                }
                self.p.source.store(s, Ordering::Relaxed);
            }
            1..=3 => {
                let a = &self.p.xyz[row - 1];
                a.set((a.get() + d as f32 * 0.02).clamp(0., 1.));
            }
            4 => self
                .p
                .wet
                .set((self.p.wet.get() + d as f32 * 0.02).clamp(0., 1.)),
            _ => self.p.mode.store(
                (self.p.mode.load(Ordering::Relaxed) as i32 + d).rem_euclid(3) as usize,
                Ordering::Relaxed,
            ),
        }
    }
    fn knob(&self, i: usize) -> Knob<'_> {
        match i {
            C_CUTOFF | C_RESONANCE | C_DRIVE => Knob::F(&self.p.xyz[i], 0., 1.),
            C_WET => Knob::F(&self.p.wet, 0., 1.),
            C_ENABLED => Knob::B(&self.p.enabled),
            // The mode is an AtomicUsize, which Knob has no variant for;
            // see kit_norm / kit_set_norm.
            _ => Knob::None,
        }
    }
}
impl PlayHost for VectorFilterApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS.len()
    }
    fn kit_label(&self, i: usize) -> String {
        CONTROLS[i % CONTROLS.len()].to_string()
    }
    fn kit_value(&self, i: usize) -> String {
        if i == C_ENABLED {
            (if self.p.enabled.load(Ordering::Relaxed) { "on" } else { "bypassed" }).into()
        } else {
            // rows() is Source then the same order as CONTROLS; it shows
            // knob + CV, i.e. what is actually heard.
            self.rows().get(i + 1).map(|r| r.1.clone()).unwrap_or_default()
        }
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        if i == C_MODE {
            Some((self.p.mode.load(Ordering::Relaxed) % MODES.len()) as f32 / (MODES.len() - 1) as f32)
        } else {
            self.knob(i).norm()
        }
    }
    fn kit_stepped(&self, i: usize) -> bool {
        i == C_MODE || self.knob(i).stepped()
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match i {
            // The menu has no row for the switch; F3 toggles it there.
            C_ENABLED if delta != 0 => self.p.enabled.store(delta > 0, Ordering::Relaxed),
            C_ENABLED => {}
            _ => self.edit_row(i + 1, delta),
        }
    }
    fn kit_reset(&mut self, i: usize) {
        match i {
            C_CUTOFF | C_RESONANCE | C_DRIVE => self.p.xyz[i].set(DEFAULT_XYZ[i]),
            C_WET => self.p.wet.set(1.),
            C_MODE => self.p.mode.store(0, Ordering::Relaxed),
            C_ENABLED => self.p.enabled.store(true, Ordering::Relaxed),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        if i == C_MODE {
            let idx = (v.clamp(0., 1.) * (MODES.len() - 1) as f32).round() as usize;
            self.p.mode.store(idx, Ordering::Relaxed);
        } else {
            self.knob(i).set(v);
        }
    }
    fn kit_line(&self) -> String {
        if self.p.source.load(Ordering::Relaxed) == NO_SOURCE {
            "R1: pick a Source".into()
        } else if !self.p.enabled.load(Ordering::Relaxed) {
            "Bypassed".into()
        } else {
            let p = self.xyz();
            format!("{:.0} Hz  Q {:.1}", 40. * 400f32.powf(p[0]), 0.5 + 7.5 * p[1])
        }
    }
}
impl App for VectorFilterApp {
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
    fn tick(&mut self, i: &Input) {
        // The play view takes the knobs and D-pad first; in the menu
        // they pass straight through to the list.
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, i);
        self.kit = play;
        let i = &step.input;
        self.list.navigate_input(i, 6, self.nav.get() as i32);
        self.edit_row(self.list.selected, i.knob2);
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        if !self.kit.menu {
            if let Some(col) = self.play_column() {
                let pal = kit::draw::Palette { bg: VF_BG, ink: VF_INK, accent: VF_ACCENT, dim: VF_DIM, faint: VF_FAINT };
                kit::draw::column(f, &col, 16, 40, 350, 285, pal);
                let dim = MonoTextStyle::new(&SPLEEN_6X12, VF_DIM);
                Text::new("knobs: cutoff / reso   D-pad: type   F2: pads   F3: bypass   R1: menu", Point::new(16, 340), dim).draw(f).ok();
            }
            return;
        }
        let r = self
            .rows()
            .into_iter()
            .map(|(a, b, _)| (a, b))
            .collect::<Vec<_>>();
        self.list.draw(f, 16, 44, 24, r.len(), &r);
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_pointer_pick(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        if x >= 1000. {
            self.p.xyz[2].set((x - 1000.).clamp(0., 1.));
        } else {
            self.p.xyz[0].set(x.clamp(0., 1.));
            self.p.xyz[1].set(y.clamp(0., 1.));
        }
    }
    fn slint_extra(&mut self) -> SlintExtra {
        SlintExtra::VectorFilter(VectorFilterExtra {
            xyz: self.xyz().to_vec(),
            wave: self.p.wave.lock().unwrap().clone(),
            source: self.bus.source_name(self.p.source.load(Ordering::Relaxed)),
            mode: MODES[self.p.mode.load(Ordering::Relaxed)].into(),
            enabled: self.p.enabled.load(Ordering::Relaxed),
        })
    }
    fn transport_action(&self) -> Option<&'static str> {
        Some(if self.p.enabled.load(Ordering::Relaxed) {
            "BYPASS"
        } else {
            "ENABLE"
        })
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
        Some(Box::new(Processor {
            p: self.p.clone(),
            bus: self.bus.clone(),
            input: Vec::with_capacity(4096),
            state: [0.; 2],
            g: 0.1,
            k: 1.,
            drive: 1.,
            wet: 1.,
        }))
    }
}
struct Processor {
    p: Arc<Shared>,
    bus: Arc<AudioBus>,
    input: Vec<f32>,
    state: [f32; 2],
    g: f32,
    k: f32,
    drive: f32,
    wet: f32,
}
impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.);
        if channels == 0 || !rate.is_finite() || rate < 1000. {
            return;
        }
        self.input.clear();
        if let Some(b) = self.bus.get(self.p.source.load(Ordering::Relaxed)) {
            if let Ok(b) = b.try_lock() {
                self.input.extend_from_slice(&b);
            }
        }
        let xyz: [f32; 3] =
            std::array::from_fn(|i| (self.p.xyz[i].get() + self.p.cv[i].get()).clamp(0., 1.));
        let tg = (std::f32::consts::PI * (40. * 400f32.powf(xyz[0])).min(rate * 0.45) / rate).tan();
        let tk = 1. / (0.5 + xyz[1] * 7.5);
        let td = 1. + 7. * xyz[2];
        let tw = if self.p.enabled.load(Ordering::Relaxed) {
            self.p.wet.get()
        } else {
            0.
        };
        let gain = (self.p.mix.get() + self.p.ext.get()).clamp(0., 1.);
        let mode = self.p.mode.load(Ordering::Relaxed);
        let smooth = 1. - (-1. / (rate * 0.008)).exp();
        for (n, frame) in out.chunks_mut(channels).enumerate() {
            let dry = self
                .input
                .get(n)
                .copied()
                .filter(|x| x.is_finite())
                .unwrap_or(0.)
                .clamp(-4., 4.);
            self.g += smooth * (tg - self.g);
            self.k += smooth * (tk - self.k);
            self.drive += smooth * (td - self.drive);
            self.wet += smooth * (tw - self.wet);
            let v0 = (dry * self.drive).tanh();
            let a1 = 1. / (1. + self.g * (self.g + self.k));
            let v1 = a1 * (self.state[0] + self.g * (v0 - self.state[1]));
            let v2 = self.state[1] + self.g * v1;
            self.state = [2. * v1 - self.state[0], 2. * v2 - self.state[1]];
            let filtered = match mode {
                1 => v1,
                2 => v0 - self.k * v1 - v2,
                _ => v2,
            };
            let value = (dry * (1. - self.wet) + filtered * self.wet).clamp(-1., 1.) * gain;
            frame.fill(value);
        }
        if let Ok(mut b) = self.p.output.try_lock() {
            b.clear();
            b.extend(out.chunks(channels).map(|f| f[0]));
        }
        if let Ok(mut w) = self.p.wave.try_lock() {
            for i in 0..w.len() {
                w[i] = out
                    .get((i * out.len() / w.len() / channels) * channels)
                    .copied()
                    .unwrap_or(0.);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (VectorFilterApp, Arc<Mutex<Vec<f32>>>) {
        let b = Arc::new(AudioBus::new());
        let input = b.register("fixture");
        (
            VectorFilterApp::new(
                b,
                Arc::new(ModBus::new()),
                Arc::new(MixerBus::new()),
                Arc::new(AtomicF32::new(1.)),
            ),
            input,
        )
    }
    #[test]
    fn cube_controls_are_bounded_and_unpatched_is_silent() {
        let (mut a, _) = fixture();
        a.slint_pointer_pick(2., -1.);
        assert_eq!(a.xyz()[..2], [1., 0.]);
        a.slint_pointer_pick(1000.7, 0.);
        assert!((a.xyz()[2] - 0.7).abs() < 0.001);
        let mut p = a.audio_processor().unwrap();
        let mut out = [1.; 512];
        p.process(&mut out, 2, 48000.);
        assert!(out.iter().all(|x| *x == 0.));
    }
    #[test]
    fn cutoff_changes_audio_and_extreme_controls_stay_finite() {
        let (mut a, input) = fixture();
        // Drives the menu's Source row with knob 2.
        a.kit.menu = true;
        a.tick(&Input {
            knob2: 1,
            ..Default::default()
        });
        let mut p = a.audio_processor().unwrap();
        let mut out = [0.; 512];
        let mut power = [0.; 2];
        for pass in 0..2 {
            a.slint_pointer_pick(pass as f32, 0.);
            for block in 0..100 {
                *input.lock().unwrap() = (0..256)
                    .map(|n| {
                        (std::f32::consts::TAU * 4000. * (n + block * 256) as f32 / 48000.).sin()
                            * 0.2
                    })
                    .collect();
                p.process(&mut out, 2, 48000.);
                assert!(out.iter().all(|v| v.is_finite() && v.abs() <= 1.));
            }
            power[pass] = out.iter().map(|v| v * v).sum::<f32>();
        }
        assert!(power[1] > power[0] * 10.);
        a.slint_pointer_pick(1., 1.);
        a.slint_pointer_pick(1001., 0.);
        for _ in 0..100 {
            p.process(&mut out, 2, 48000.);
            assert!(out.iter().all(|v| v.is_finite() && v.abs() <= 1.));
        }
    }
    fn pad(rank: usize) -> Input {
        Input { grid: std::array::from_fn(|k| k == kit::rank_pad(rank)), ..Default::default() }
    }
    #[test]
    fn opens_playable_knob1_is_cutoff_and_the_stick_is_the_xy_face() {
        let (mut a, _) = fixture();
        assert!(a.play_column().is_some(), "play view first");
        a.tick(&Input { knob1: 3, ..Default::default() });
        assert!((a.p.xyz[0].get() - 0.71).abs() < 1e-5, "knob 1 is cutoff, in the menu's 2% steps");
        a.tick(&Input { stick: [-1., 1.], ..Default::default() });
        assert!((a.p.xyz[0].get() - 0.21).abs() < 1e-5 && (a.p.xyz[1].get() - 0.7).abs() < 1e-5, "stick X/Y push cutoff/resonance");
        a.tick(&Input::default());
        assert!((a.p.xyz[0].get() - 0.71).abs() < 1e-5 && (a.p.xyz[1].get() - 0.2).abs() < 1e-5, "and let go back to the knobs");
        a.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(a.p.mode.load(Ordering::Relaxed), 1, "D-pad up steps the filter type");
        a.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(a.play_column().is_none(), "R1 opens the full menu");
    }
    #[test]
    fn throws_hold_and_put_back_exactly() {
        let (mut a, _) = fixture();
        a.tick(&pad(2));
        assert_eq!(a.p.xyz[1].get(), 1., "SCREAM");
        a.tick(&Input::default());
        assert!((a.p.xyz[1].get() - 0.2).abs() < 1e-5);
        a.tick(&pad(6));
        assert_eq!(a.p.mode.load(Ordering::Relaxed), 2, "HIGH holds high pass");
        a.tick(&Input::default());
        assert_eq!(a.p.mode.load(Ordering::Relaxed), 0);
        a.tick(&pad(7));
        assert!(!a.p.enabled.load(Ordering::Relaxed), "BYPASS");
        a.tick(&Input::default());
        assert!(a.p.enabled.load(Ordering::Relaxed));
    }
}
