//! Peaks: the real Mutable Instruments Peaks (vendor/eurorack/peaks, MIT, (c)
//! Emilie Gillet), running its own C++ through vendor/bridge/peaks_bridge.cc.
//!
//! Peaks is two identical processors, each one of twelve functions: a
//! multistage envelope, an LFO, a tap-tempo LFO, bass, snare and FM drums, a
//! hi-hat (the snare with both its tone and snappy pots all the way up, as on
//! the module), a pulse shaper, a pulse randomizer, a bouncing ball, a mini
//! sequencer and a number station. Each has four pots, shown here as the
//! module's four knobs (what each means depends on the function).
//!
//! The pads are the two gate inputs: the lower two rows fire processor A, the
//! upper two fire B, and any note from another app does the same (notes below
//! the middle fire A). The drums and the number station are audio. The
//! envelopes, LFOs and the rest are modulation: each processor's output can be
//! patched to any app's modulation input.

use super::mi_kit::{octaves, CvOuts, Controls, Keys, MiApp, Module, NoteQueue, RateBridge, Spec};
use crate::app::play_kit::{KitConfig, Layer, Routes};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::ffi::c_void;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

unsafe extern "C" {
    fn peaks_create() -> *mut c_void;
    fn peaks_destroy(h: *mut c_void);
    fn peaks_set(h: *mut c_void, ch: c_int, function: c_int, p0: u16, p1: u16, p2: u16, p3: u16);
    fn peaks_function(h: *mut c_void, ch: c_int) -> c_int;
    fn peaks_render(h: *mut c_void, ch: c_int, gate: c_int, out: *mut i16, n: c_int);
}

/// Peaks' fixed rate.
const PEAKS_RATE: f32 = 48_000.0;
const BLOCK: usize = 8;

const FUNCTIONS: [&str; 12] = [
    "Envelope", "LFO", "Tap LFO", "Bass drum", "Snare drum", "Hi-hat", "FM drum", "Pulse shaper", "Pulse randomizer", "Bouncing ball", "Mini sequencer",
    "Number station",
];
/// Whether a function's output is sound (else a control voltage).
const AUDIO: [bool; 12] = [false, false, false, true, true, true, true, false, false, false, false, true];

const C_FUNC_A: usize = 0;
const C_FUNC_B: usize = 5;

static SPECS: [Spec; 10] = [
    Spec::switch("A Function", &FUNCTIONS, 3),
    Spec::knob("A Pot 1", 0.4),
    Spec::knob("A Pot 2", 0.5),
    Spec::knob("A Pot 3", 0.5),
    Spec::knob("A Pot 4", 0.5),
    Spec::switch("B Function", &FUNCTIONS, 0),
    Spec::knob("B Pot 1", 0.4),
    Spec::knob("B Pot 2", 0.5),
    Spec::knob("B Pot 3", 0.5),
    Spec::knob("B Pot 4", 0.5),
];

const OUTS: [&str; 2] = ["Out A", "Out B"];

pub struct Peaks {
    controls: Arc<Controls>,
    modbus: Arc<ModBus>,
    outs: Arc<CvOuts>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    levels: Arc<[AtomicF32; 2]>,
    gates: Arc<[AtomicBool; 2]>,
}

impl Peaks {
    fn new(modbus: Arc<ModBus>, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Peaks {
        let (mix_level, ext_mix_level) = mixer_bus.register("Peaks", &modbus);
        Peaks {
            controls: Arc::new(Controls::new(&SPECS, "Peaks", &modbus)),
            outs: Arc::new(CvOuts::new(&OUTS)),
            modbus,
            bus_out: audio_bus.register("Peaks"),
            mix_level,
            ext_mix_level,
            levels: Arc::new(std::array::from_fn(|_| AtomicF32::new(0.0))),
            gates: Arc::new(std::array::from_fn(|_| AtomicBool::new(false))),
        }
    }
}

impl Module for Peaks {
    const NAME: &'static str = "Peaks";
    // Black faceplate, white legends, a green LED.
    const PALETTE: [Rgb565; 4] = [Rgb565::new(1, 3, 2), Rgb565::new(29, 58, 29), Rgb565::new(6, 54, 12), Rgb565::new(13, 28, 15)];

    fn controls(&self) -> &Controls {
        &self.controls
    }

    fn kit_config(&self) -> KitConfig {
        KitConfig {
            app_id: "peaks",
            layers: vec![Layer::Native(0, "GATES"), Layer::Controls, Layer::Moments],
            hero: vec![[1, 2], [3, 4], [6, 7], [8, 9], [C_FUNC_A, C_FUNC_B]],
            browse: Some(C_FUNC_A),
            routes: Routes { stick_x: Some(1), stick_y: Some(2), hand_l: Some(6), hand_r: Some(7) },
            throws: Vec::new(),
            midi_to_pads: false,
            own_expression: false,
        }
    }

    fn extra_rows(&self) -> Vec<(String, String)> {
        let mut rows = Vec::new();
        for (i, name) in OUTS.iter().enumerate() {
            rows.push((format!("{name} App"), self.outs.app_label(&self.modbus, i)));
            rows.push((format!("{name} Input"), self.outs.input_label(&self.modbus, i)));
        }
        rows
    }

    fn edit_extra(&mut self, i: usize, delta: i32) {
        if i % 2 == 0 {
            self.outs.step_app(&self.modbus, i / 2, delta.signum());
        } else {
            self.outs.step_input(&self.modbus, i / 2, delta.signum());
        }
    }

    fn reset_extra(&mut self, i: usize) {
        self.outs.clear(i / 2);
    }

    fn processor(&mut self, notes: Arc<NoteQueue>) -> Box<dyn AudioProcessor> {
        Box::new(PeaksProcessor {
            handle: unsafe { peaks_create() },
            controls: Arc::clone(&self.controls),
            modbus: Arc::clone(&self.modbus),
            outs: Arc::clone(&self.outs),
            notes,
            bus_out: Arc::clone(&self.bus_out),
            mix_level: Arc::clone(&self.mix_level),
            ext_mix_level: Arc::clone(&self.ext_mix_level),
            levels: Arc::clone(&self.levels),
            gates: Arc::clone(&self.gates),
            held: Vec::with_capacity(16),
            bridge: RateBridge::new(PEAKS_RATE, BLOCK),
            stereo: Vec::new(),
            last_cv: [0.0; 2],
        })
    }

    fn update(&mut self, keys: &Keys) {
        // Held notes below the middle fire A, the rest B.
        let a = keys.held().iter().any(|&n| n < 56);
        let b = keys.held().iter().any(|&n| n >= 56);
        self.gates[0].store(a, Ordering::Relaxed);
        self.gates[1].store(b, Ordering::Relaxed);
    }

    fn line(&self, _keys: &Keys) -> String {
        format!("A: {}   B: {}", FUNCTIONS[self.controls.choice(C_FUNC_A).min(11)], FUNCTIONS[self.controls.choice(C_FUNC_B).min(11)])
    }

    fn meters(&self) -> Vec<(String, f32)> {
        OUTS.iter().zip(self.levels.iter()).map(|(n, v)| (n.to_string(), v.get())).collect()
    }

    fn needs_background_audio(&self) -> bool {
        self.outs.any()
    }

    fn title(&self) -> String {
        format!("{} / {}", FUNCTIONS[self.controls.choice(C_FUNC_A).min(11)], FUNCTIONS[self.controls.choice(C_FUNC_B).min(11)])
    }

    fn hint(&self) -> &'static str {
        "pads: gates (lower A, upper B)   L/R: dial   U/D: function A   R1: menu (outputs)"
    }
}

struct PeaksProcessor {
    handle: *mut c_void,
    controls: Arc<Controls>,
    modbus: Arc<ModBus>,
    outs: Arc<CvOuts>,
    notes: Arc<NoteQueue>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    levels: Arc<[AtomicF32; 2]>,
    gates: Arc<[AtomicBool; 2]>,
    held: Vec<u8>,
    bridge: RateBridge,
    stereo: Vec<[f32; 2]>,
    last_cv: [f32; 2],
}

// The handle is only ever touched from the audio thread.
unsafe impl Send for PeaksProcessor {}

impl Drop for PeaksProcessor {
    fn drop(&mut self) {
        unsafe { peaks_destroy(self.handle) };
    }
}

impl AudioProcessor for PeaksProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        buffer.iter_mut().for_each(|s| *s = 0.0);
        let frames = buffer.len() / channels;
        // The keys' gates come from `Module::update` on the UI thread; the queue is
        // drained here only so it never fills.
        while self.notes.pop().is_some() {}
        let _ = &self.held;
        let c = &self.controls;
        let u16_of = |v: f32| (v.clamp(0.0, 1.0) * 65535.0) as u16;
        let functions = [c.choice(C_FUNC_A).min(11), c.choice(C_FUNC_B).min(11)];
        unsafe {
            peaks_set(self.handle, 0, functions[0] as c_int, u16_of(c.get(1)), u16_of(c.get(2)), u16_of(c.get(3)), u16_of(c.get(4)));
            peaks_set(self.handle, 1, functions[1] as c_int, u16_of(c.get(6)), u16_of(c.get(7)), u16_of(c.get(8)), u16_of(c.get(9)));
        }
        // The snare and hi-hat trade places by their pots, as on the module.
        let running = [unsafe { peaks_function(self.handle, 0) } as usize, unsafe { peaks_function(self.handle, 1) } as usize];
        let gates = [self.gates[0].load(Ordering::Relaxed), self.gates[1].load(Ordering::Relaxed)];
        let handle = self.handle;
        let last_cv = &mut self.last_cv;
        self.stereo.clear();
        self.stereo.resize(frames, [0.0; 2]);
        self.bridge.process(&[], &mut self.stereo, sample_rate, |_, o| {
            let mut mix = [0.0f32; BLOCK];
            for ch in 0..2usize {
                let mut raw = [0i16; BLOCK];
                unsafe { peaks_render(handle, ch as c_int, gates[ch] as c_int, raw.as_mut_ptr(), BLOCK as c_int) };
                let audio = AUDIO[running[ch].min(11)];
                for k in 0..BLOCK {
                    let v = raw[k] as f32 / 32768.0;
                    if audio {
                        mix[k] += v * 0.5;
                    }
                }
                last_cv[ch] = if audio { 0.0 } else { raw[BLOCK - 1] as f32 / 32768.0 };
            }
            for (k, frame) in o.iter_mut().enumerate().take(BLOCK) {
                *frame = [mix[k], mix[k]];
            }
        });
        for ch in 0..2 {
            self.levels[ch].set(self.last_cv[ch].abs().min(1.0).max(self.stereo.last().map_or(0.0, |s| s[0].abs()).min(1.0)));
            self.outs.send(&self.modbus, ch, self.last_cv[ch]);
        }
        let mix = (self.mix_level.get() + self.ext_mix_level.get()).clamp(0.0, 2.0);
        if let Ok(mut bus) = self.bus_out.try_lock() {
            bus.clear();
            bus.extend(self.stereo.iter().map(|f| f[0] * mix));
        }
        for (frame, s) in buffer.chunks_mut(channels).zip(self.stereo.iter()) {
            frame.fill(s[0] * mix);
        }
    }
}

#[allow(dead_code)] // tests name it
pub type PeaksApp = MiApp<Peaks>;

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    let audio_bus: Arc<AudioBus> = ctx.get();
    let mixer: Arc<MixerBus> = ctx.get();
    let _ = octaves;
    Box::new(MiApp::new(Peaks::new(ctx.get(), &audio_bus, &mixer), ctx.named("sensitivity"), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Input};

    fn app() -> (PeaksApp, Arc<ModBus>) {
        let modbus = Arc::new(ModBus::new());
        (MiApp::new(Peaks::new(Arc::clone(&modbus), &AudioBus::new(), &MixerBus::new()), Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(1.0))), modbus)
    }

    fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        let mut buf = vec![0.0f32; 512 * 2];
        for _ in 0..blocks {
            p.process(&mut buf, 2, 48_000.0);
            assert!(buf.iter().all(|v| v.is_finite()));
            all.extend(buf.iter().step_by(2));
        }
        all
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn the_drums_are_silent_until_a_gate_and_then_ring_out() {
        for (func, name) in [(3, "Bass drum"), (4, "Snare drum"), (6, "FM drum")] {
            let (mut a, _) = app();
            a.m.controls.set(C_FUNC_A, func as f32);
            let mut p = a.audio_processor().unwrap();
            assert!(rms(&render(&mut p, 5)) < 1e-6, "{name}: quiet before a gate");
            // The lower rows are gate A.
            a.tick(&Input { grid: std::array::from_fn(|i| i == 12), ..Default::default() });
            let hit = render(&mut p, 12);
            assert!(rms(&hit) > 0.02, "{name}: a pad fires it ({})", rms(&hit));
            a.tick(&Input::default());
            let tail = render(&mut p, 120);
            assert!(rms(&tail[tail.len() - 4800..]) < rms(&hit) * 0.2, "{name}: it dies away");
        }
    }

    #[test]
    fn an_lfo_swings_a_patched_input_and_an_envelope_follows_its_gate() {
        let (mut a, modbus) = app();
        let probe = modbus.register("Probe: In");
        a.m.controls.set(C_FUNC_A, 1.0); // LFO
        a.m.controls.set(1, 0.9); // fast
        a.m.controls.set(C_FUNC_B, 0.0); // envelope
        a.m.controls.set(6, 0.05); // fast attack
        a.m.controls.set(7, 0.3);
        a.m.controls.set(8, 0.9); // high sustain
        a.m.controls.set(9, 0.2);
        let mut guard = 0;
        while a.m.outs.app_label(&modbus, 0) != "Probe" && guard < 20 {
            a.m.edit_extra(0, 1);
            guard += 1;
        }
        let mut p = a.audio_processor().unwrap();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut buf = vec![0.0f32; 512 * 2];
        for _ in 0..200 {
            p.process(&mut buf, 2, 48_000.0);
            lo = lo.min(probe.get());
            hi = hi.max(probe.get());
        }
        assert!(hi - lo > 0.5, "the LFO swings: {lo}..{hi}");
        // Envelope B on its own output: patch B and hold the upper pad.
        let probe_b = modbus.register("Probe B: In");
        while a.m.outs.app_label(&modbus, 1) != "Probe B" && guard < 40 {
            a.m.edit_extra(2, 1);
            guard += 1;
        }
        a.tick(&Input { grid: std::array::from_fn(|i| i == 0), ..Default::default() });
        for _ in 0..60 {
            p.process(&mut buf, 2, 48_000.0);
        }
        assert!(probe_b.get() > 0.3, "the envelope rises while the gate is held ({})", probe_b.get());
        a.tick(&Input::default());
        for _ in 0..200 {
            p.process(&mut buf, 2, 48_000.0);
        }
        assert!(probe_b.get() < 0.05, "and falls when it is released ({})", probe_b.get());
    }

    #[test]
    fn every_function_runs_and_stays_bounded() {
        for func in 0..FUNCTIONS.len() {
            let (mut a, _) = app();
            a.m.controls.set(C_FUNC_A, func as f32);
            a.m.controls.set(C_FUNC_B, func as f32);
            let mut p = a.audio_processor().unwrap();
            a.tick(&Input { grid: std::array::from_fn(|i| i == 12 || i == 0), ..Default::default() });
            let out = render(&mut p, 40);
            assert!(out.iter().all(|v| v.abs() <= 2.0), "{}: bounded", FUNCTIONS[func]);
        }
    }

    #[test]
    fn its_pots_are_mod_inputs() {
        let (_a, modbus) = app();
        for name in ["Peaks: A Pot 1", "Peaks: B Pot 4", "Mixer: Peaks Level"] {
            assert!(modbus.index_of(name).is_some(), "{name}");
        }
    }
}
