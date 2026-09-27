//! Independent mono AudioBus filter. UI coordinates map to normalized parameters;
//! DSP uses a smoothed topology-preserving state-variable filter.
use crate::{
    app::{App, Input, SlintExtra, VectorFilterExtra},
    audio::AudioProcessor,
    audio_bus::{cycle_source, AudioBus, NO_SOURCE},
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    paramlist::ParamList,
    util::AtomicF32,
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
const MODES: [&str; 3] = ["Low pass", "Band pass", "High pass"];
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
                xyz: [
                    AtomicF32::new(0.65),
                    AtomicF32::new(0.2),
                    AtomicF32::new(0.1),
                ],
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
}
impl App for VectorFilterApp {
    fn tick(&mut self, i: &Input) {
        self.list.navigate_input(i, 6, self.nav.get() as i32);
        let d = i.knob2;
        if d == 0 {
            return;
        }
        match self.list.selected {
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
                let a = &self.p.xyz[self.list.selected - 1];
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
    fn draw(&mut self, f: &mut FrameBuffer) {
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
}
