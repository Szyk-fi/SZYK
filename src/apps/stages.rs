//! Stages: the real Mutable Instruments Stages segment generator
//! (vendor/eurorack/stages, MIT, (c) Emilie Gillet), running its own C++
//! through vendor/bridge/stages_bridge.cc.
//!
//! Stages is six segment channels. Each has a *type* (ramp, step, hold or
//! alt), a *loop* flag, a slider and a pot, and what a slider and pot mean
//! depends on the type -- that is the module's whole idea, and this keeps
//! it: the panel here is the real one, 1-6 segments in a group with
//! type, loop, slider and pot each. One segment is a single function (LFO,
//! oscillator, decay envelope, pulse and gate generators, sample and hold,
//! portamento, delay); more are a multi-segment envelope; a ramp followed
//! only by steps is the step sequencer.
//!
//! The PRESET switch loads a sensible group (an ADSR, an LFO...) onto the
//! panel; every setting stays editable afterwards, and PRESET Custom leaves
//! them alone. The pads are the gate input: a held pad is gate high, so
//! envelopes fire and sustain as on the module, and F3 starts and stops
//! it. The group's value and phase can be patched to any app's modulation
//! input, and the oscillator presets are audio.

use super::mi_kit::{CvOuts, Controls, Keys, MiApp, Module, NoteQueue, RateBridge, Spec};
use crate::app::play_kit::{KitConfig, Layer, Routes};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::ffi::c_void;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

unsafe extern "C" {
    fn stages_block_size() -> c_int;
    fn stages_sample_rate() -> f32;
    fn stages_create() -> *mut c_void;
    fn stages_destroy(h: *mut c_void);
    fn stages_configure(h: *mut c_void, has_trigger: c_int, types: *const c_int, loops: *const c_int, n: c_int);
    fn stages_set_segment(h: *mut c_void, index: c_int, primary: f32, secondary: f32);
    fn stages_process(h: *mut c_void, gate: c_int, value: *mut f32, phase: *mut f32, segment: *mut c_int, n: c_int) -> c_int;
}

const MAX_SEGMENTS: usize = 6;
const TYPES: [&str; 4] = ["Ramp", "Step", "Hold", "Alt"];
const LOOPS: [&str; 2] = ["Off", "On"];
const SEGMENT_COUNTS: [&str; 6] = ["1", "2", "3", "4", "5", "6"];
const TRIGGER: [&str; 2] = ["Off", "On"];

/// A loadable group: what the module's buttons and sliders would be set to.
struct Preset {
    name: &'static str,
    trigger: bool,
    /// Per segment: (type 0-3, loop, slider, pot).
    segments: &'static [(u8, bool, f32, f32)],
    /// What the sliders and pots mean in this group, for the status line.
    help: &'static str,
}

const RAMP: u8 = 0;
const STEP: u8 = 1;
const HOLD: u8 = 2;
const ALT: u8 = 3;

/// "Custom" is index 0 and changes nothing.
const PRESETS: [Preset; 15] = [
    Preset { name: "Custom", trigger: true, segments: &[], help: "edit any slider, pot, type or loop" },
    Preset {
        name: "ADSR",
        trigger: true,
        // Attack (time, peak level), decay (time, shape), sustain hold (level), release (time, shape).
        segments: &[(RAMP, false, 0.15, 1.0), (RAMP, false, 0.3, 0.5), (HOLD, true, 0.6, 0.5), (RAMP, false, 0.35, 0.5)],
        help: "S1 attack time/peak  S2 decay time/shape  S3 sustain level  S4 release time/shape",
    },
    Preset {
        name: "AR",
        trigger: true,
        segments: &[(RAMP, false, 0.15, 0.5), (HOLD, true, 1.0, 0.5), (RAMP, false, 0.35, 0.5)],
        help: "S1 attack time/shape  S3 release time/shape",
    },
    Preset { name: "Decay", trigger: true, segments: &[(RAMP, false, 0.45, 0.4)], help: "slider time, pot shape; fires on the gate" },
    Preset { name: "Free LFO", trigger: false, segments: &[(RAMP, true, 0.5, 0.5)], help: "slider rate, pot shape" },
    Preset { name: "Tap LFO", trigger: true, segments: &[(RAMP, true, 0.5, 0.5)], help: "tap the gate; slider/pot shape and multiply" },
    Preset { name: "Oscillator", trigger: false, segments: &[(ALT, true, 0.5, 0.5)], help: "slider pitch, pot shape (audio)" },
    Preset { name: "PLL Osc", trigger: true, segments: &[(ALT, true, 0.5, 0.5)], help: "locks to the gate's rate (audio)" },
    Preset { name: "Pulse", trigger: true, segments: &[(HOLD, false, 1.0, 0.4)], help: "slider level, pot length" },
    Preset { name: "Gates", trigger: true, segments: &[(HOLD, true, 1.0, 0.4)], help: "slider level, pot length; repeats while gated" },
    Preset { name: "Sample & Hold", trigger: true, segments: &[(STEP, false, 0.5, 0.0)], help: "slider level; holds on each gate" },
    Preset { name: "Portamento", trigger: false, segments: &[(STEP, false, 0.5, 0.4)], help: "slider level, pot glide time" },
    Preset { name: "Delay", trigger: false, segments: &[(HOLD, false, 0.5, 0.4)], help: "pot delay time" },
    Preset {
        name: "Sequencer",
        trigger: true,
        // A leading ramp is the direction/clock function; the steps follow.
        segments: &[(RAMP, false, 0.5, 0.5), (STEP, false, 0.2, 0.0), (STEP, false, 0.5, 0.0), (STEP, false, 0.35, 0.0), (STEP, false, 0.8, 0.0)],
        help: "each gate steps S2..S5; their sliders are the levels",
    },
    Preset {
        name: "Two-step",
        trigger: true,
        segments: &[(HOLD, false, 0.2, 0.3), (HOLD, false, 0.9, 0.5)],
        help: "S1 level and time, then S2 level and time",
    },
];

const C_PRESET: usize = 0;
const C_SEGMENTS: usize = 1;
const C_TRIGGER: usize = 2;
/// Segment `i`'s controls start here: type, loop, slider, pot.
const fn seg(i: usize) -> usize {
    3 + i * 4
}

macro_rules! segment_specs {
    ($($n:literal),*) => {
        [
            Spec::switch("Preset", &PRESET_NAMES_STATIC, 1),
            Spec::switch("Segments", &SEGMENT_COUNTS, 3),
            Spec::switch("Gate trigger", &TRIGGER, 1),
            $(
                Spec::switch(concat!("S", $n, " Type"), &TYPES, 0),
                Spec::switch(concat!("S", $n, " Loop"), &LOOPS, 0),
                Spec::knob(concat!("S", $n, " Slider"), 0.5),
                Spec::knob(concat!("S", $n, " Pot"), 0.5),
            )*
        ]
    };
}

const PRESET_NAMES_STATIC: [&str; 15] = [
    "Custom", "ADSR", "AR", "Decay", "Free LFO", "Tap LFO", "Oscillator", "PLL Osc", "Pulse", "Gates", "Sample & Hold", "Portamento", "Delay",
    "Sequencer", "Two-step",
];

static SPECS: [Spec; 27] = segment_specs!("1", "2", "3", "4", "5", "6");

const OUTS: [&str; 2] = ["Value", "Phase"];

pub struct Stages {
    controls: Arc<Controls>,
    modbus: Arc<ModBus>,
    outs: Arc<CvOuts>,
    running: Arc<AtomicBool>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    value: Arc<AtomicF32>,
    gate: Arc<AtomicBool>,
    /// The preset last applied, so a change is applied once.
    applied: AtomicUsize,
    active_segment: Arc<AtomicUsize>,
}

impl Stages {
    fn new(modbus: Arc<ModBus>, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Stages {
        debug_assert!(PRESET_NAMES_STATIC.iter().zip(PRESETS.iter()).all(|(a, p)| *a == p.name));
        let (mix_level, ext_mix_level) = mixer_bus.register("Stages", &modbus);
        let s = Stages {
            controls: Arc::new(Controls::new(&SPECS, "Stages", &modbus)),
            outs: Arc::new(CvOuts::new(&OUTS)),
            modbus,
            running: Arc::new(AtomicBool::new(false)),
            bus_out: audio_bus.register("Stages"),
            mix_level,
            ext_mix_level,
            value: Arc::new(AtomicF32::new(0.0)),
            gate: Arc::new(AtomicBool::new(false)),
            applied: AtomicUsize::new(usize::MAX),
            active_segment: Arc::new(AtomicUsize::new(0)),
        };
        s.apply_preset(s.controls.choice(C_PRESET));
        s
    }

    /// Writes a preset's group onto the panel.
    fn apply_preset(&self, index: usize) {
        self.applied.store(index, Ordering::Relaxed);
        let p = &PRESETS[index.min(PRESETS.len() - 1)];
        if p.segments.is_empty() {
            return;
        }
        let c = &self.controls;
        c.set(C_SEGMENTS, (p.segments.len() - 1) as f32);
        c.set(C_TRIGGER, p.trigger as u8 as f32);
        for i in 0..MAX_SEGMENTS {
            let (t, l, s, q) = p.segments.get(i).copied().unwrap_or((RAMP, false, 0.5, 0.5));
            c.set(seg(i), t as f32);
            c.set(seg(i) + 1, l as u8 as f32);
            c.set(seg(i) + 2, s);
            c.set(seg(i) + 3, q);
        }
    }
}

impl Module for Stages {
    const NAME: &'static str = "Stages";
    // Warm grey faceplate with an amber trace.
    const PALETTE: [Rgb565; 4] = [Rgb565::new(4, 7, 6), Rgb565::new(28, 54, 26), Rgb565::new(31, 40, 6), Rgb565::new(15, 28, 14)];

    fn controls(&self) -> &Controls {
        &self.controls
    }

    fn kit_config(&self) -> KitConfig {
        KitConfig {
            app_id: "stages",
            layers: vec![Layer::Native(0, "GATE"), Layer::Controls, Layer::Moments],
            hero: vec![[seg(0) + 2, seg(0) + 3], [seg(1) + 2, seg(1) + 3], [seg(2) + 2, seg(2) + 3], [C_PRESET, C_SEGMENTS]],
            browse: Some(C_PRESET),
            routes: Routes { stick_x: Some(seg(0) + 2), stick_y: Some(seg(0) + 3), hand_l: Some(seg(1) + 2), hand_r: Some(seg(1) + 3) },
            throws: Vec::new(),
            midi_to_pads: false,
            own_expression: false,
        }
    }

    fn plays_notes(&self) -> bool {
        true
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

    fn update(&mut self, keys: &Keys) {
        self.gate.store(!keys.held().is_empty(), Ordering::Relaxed);
        let preset = self.controls.choice(C_PRESET);
        if preset != self.applied.load(Ordering::Relaxed) {
            self.apply_preset(preset);
        }
    }

    fn processor(&mut self, _notes: Arc<NoteQueue>) -> Box<dyn AudioProcessor> {
        let block = unsafe { stages_block_size() } as usize;
        Box::new(StagesProcessor {
            handle: unsafe { stages_create() },
            controls: Arc::clone(&self.controls),
            modbus: Arc::clone(&self.modbus),
            outs: Arc::clone(&self.outs),
            running: Arc::clone(&self.running),
            gate: Arc::clone(&self.gate),
            bus_out: Arc::clone(&self.bus_out),
            mix_level: Arc::clone(&self.mix_level),
            ext_mix_level: Arc::clone(&self.ext_mix_level),
            value: Arc::clone(&self.value),
            active_segment: Arc::clone(&self.active_segment),
            bridge: RateBridge::new(unsafe { stages_sample_rate() }, block),
            configured: None,
            phase: 0.0,
            stereo: Vec::new(),
        })
    }

    fn line(&self, _keys: &Keys) -> String {
        if !self.running.load(Ordering::Relaxed) {
            return "stopped (F3 runs)".into();
        }
        let p = &PRESETS[self.controls.choice(C_PRESET).min(PRESETS.len() - 1)];
        format!("{}: {}", p.name, p.help)
    }

    fn meters(&self) -> Vec<(String, f32)> {
        vec![("value".into(), self.value.get().clamp(0.0, 1.0)), ("segment".into(), self.active_segment.load(Ordering::Relaxed) as f32 / 6.0)]
    }

    fn needs_background_audio(&self) -> bool {
        self.running.load(Ordering::Relaxed) && self.outs.any()
    }

    fn running(&self) -> Option<bool> {
        Some(self.running.load(Ordering::Relaxed))
    }

    fn toggle_running(&mut self) {
        self.running.fetch_xor(true, Ordering::Relaxed);
    }

    fn title(&self) -> String {
        PRESETS[self.controls.choice(C_PRESET).min(PRESETS.len() - 1)].name.to_string()
    }

    fn hint(&self) -> &'static str {
        "F3: run   pads: gate   L/R: dial   U/D: preset   R1: menu (segments, outputs)"
    }
}

struct StagesProcessor {
    handle: *mut c_void,
    controls: Arc<Controls>,
    modbus: Arc<ModBus>,
    outs: Arc<CvOuts>,
    running: Arc<AtomicBool>,
    gate: Arc<AtomicBool>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    value: Arc<AtomicF32>,
    active_segment: Arc<AtomicUsize>,
    bridge: RateBridge,
    /// The group configuration last sent to the generator.
    configured: Option<(bool, usize, [c_int; MAX_SEGMENTS], [c_int; MAX_SEGMENTS])>,
    phase: f32,
    stereo: Vec<[f32; 2]>,
}

// The handle is only ever touched from the audio thread.
unsafe impl Send for StagesProcessor {}

impl Drop for StagesProcessor {
    fn drop(&mut self) {
        unsafe { stages_destroy(self.handle) };
    }
}

impl StagesProcessor {
    /// Sends the panel's group to the generator when its shape changed.
    fn configure(&mut self) -> bool {
        let c = &self.controls;
        let n = (c.choice(C_SEGMENTS) + 1).min(MAX_SEGMENTS);
        let trig = c.choice(C_TRIGGER) != 0;
        let (mut types, mut loops) = ([0 as c_int; MAX_SEGMENTS], [0 as c_int; MAX_SEGMENTS]);
        for i in 0..MAX_SEGMENTS {
            types[i] = c.choice(seg(i)) as c_int;
            loops[i] = c.choice(seg(i) + 1) as c_int;
        }
        let now = (trig, n, types, loops);
        if self.configured.as_ref() != Some(&now) {
            unsafe { stages_configure(self.handle, trig as c_int, types.as_ptr(), loops.as_ptr(), n as c_int) };
            self.configured = Some(now);
        }
        // Audio rate if a single alt segment: the oscillator presets.
        n == 1 && types[0] == ALT as c_int
    }
}

impl AudioProcessor for StagesProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        buffer.iter_mut().for_each(|s| *s = 0.0);
        let frames = buffer.len() / channels;
        if !self.running.load(Ordering::Relaxed) {
            if let Ok(mut bus) = self.bus_out.try_lock() {
                bus.clear();
                bus.resize(frames, 0.0);
            }
            return;
        }
        let audio = self.configure();
        let n = (self.controls.choice(C_SEGMENTS) + 1).min(MAX_SEGMENTS);
        for i in 0..n {
            unsafe { stages_set_segment(self.handle, i as c_int, self.controls.get(seg(i) + 2), self.controls.get(seg(i) + 3)) };
        }
        self.stereo.clear();
        self.stereo.resize(frames, [0.0; 2]);
        let gate = self.gate.load(Ordering::Relaxed) as c_int;
        let handle = self.handle;
        let (mut last_value, mut last_phase, mut last_segment) = (self.value.get(), self.phase, 0usize);
        self.bridge.process(&[], &mut self.stereo, sample_rate, |_, o| {
            let mut value = [0.0f32; 8];
            let mut phase = [0.0f32; 8];
            let mut segment = [0 as c_int; 8];
            let k = o.len().min(8);
            unsafe { stages_process(handle, gate, value.as_mut_ptr(), phase.as_mut_ptr(), segment.as_mut_ptr(), k as c_int) };
            for i in 0..k {
                // Audio-rate outputs swing around the middle of the 0..1 range.
                let v = if audio { (value[i] - 0.5) * 2.0 } else { value[i] };
                o[i] = [v, v];
            }
            last_value = value[k - 1];
            last_phase = phase[k - 1];
            last_segment = segment[k - 1].max(0) as usize;
        });
        self.value.set(last_value);
        self.phase = last_phase;
        self.active_segment.store(last_segment, Ordering::Relaxed);
        self.outs.send(&self.modbus, 0, last_value);
        self.outs.send(&self.modbus, 1, last_phase);

        if let Ok(mut bus) = self.bus_out.try_lock() {
            bus.clear();
            bus.extend(self.stereo.iter().map(|s| if audio { s[0] } else { 0.0 }));
        }
        if audio {
            let mix = (self.mix_level.get() + self.ext_mix_level.get()).clamp(0.0, 2.0) * 0.5;
            for (frame, s) in buffer.chunks_mut(channels).zip(self.stereo.iter()) {
                frame.fill(s[0] * mix);
            }
        }
    }
}

#[allow(dead_code)] // tests name it
pub type StagesApp = MiApp<Stages>;

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    let audio_bus: Arc<AudioBus> = ctx.get();
    let mixer: Arc<MixerBus> = ctx.get();
    Box::new(MiApp::new(Stages::new(ctx.get(), &audio_bus, &mixer), ctx.named("sensitivity"), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Input};

    fn app() -> (StagesApp, Arc<ModBus>) {
        let modbus = Arc::new(ModBus::new());
        (MiApp::new(Stages::new(Arc::clone(&modbus), &AudioBus::new(), &MixerBus::new()), Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(1.0))), modbus)
    }

    /// Runs `seconds` of audio with the gate held for the first `hold` seconds
    /// and returns the generator's value after each 256-frame block.
    fn trace(a: &mut StagesApp, target: &Arc<AtomicF32>, seconds: f32, hold: f32) -> Vec<f32> {
        let mut p = a.audio_processor().unwrap();
        let mut out = Vec::new();
        let mut buf = vec![0.0f32; 256 * 2];
        let blocks = (seconds * 48_000.0 / 256.0) as usize;
        for b in 0..blocks {
            let t = b as f32 * 256.0 / 48_000.0;
            let pad = (t < hold).then_some(12);
            a.tick(&Input { grid: std::array::from_fn(|i| Some(i) == pad), ..Default::default() });
            p.process(&mut buf, 2, 48_000.0);
            out.push(target.get());
        }
        out
    }

    fn patch_value_to_probe(a: &mut StagesApp, modbus: &ModBus) {
        let mut guard = 0;
        while a.m.outs.app_label(modbus, 0) != "Probe" && guard < 20 {
            a.m.edit_extra(0, 1);
            guard += 1;
        }
    }

    fn set_preset(a: &mut StagesApp, name: &str) {
        let i = PRESETS.iter().position(|p| p.name == name).unwrap();
        a.m.controls.set(C_PRESET, i as f32);
        a.tick(&Input::default());
    }

    #[test]
    fn it_opens_stopped_and_silent() {
        let (mut a, _) = app();
        assert_eq!(a.running(), Some(false));
        let mut p = a.audio_processor().unwrap();
        let mut buf = vec![0.0f32; 1024];
        p.process(&mut buf, 2, 48_000.0);
        assert!(buf.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn an_adsr_rises_holds_at_its_sustain_and_releases_when_the_gate_falls() {
        let (mut a, modbus) = app();
        // Probe: In has to exist before patching to it.
        let probe = modbus.register("Probe: In");
        set_preset(&mut a, "ADSR");
        patch_value_to_probe(&mut a, &modbus);
        a.toggle_running();
        let v = trace(&mut a, &probe, 6.0, 3.0);
        let peak = v.iter().cloned().fold(0.0f32, f32::max);
        let at = |s: f32| v[(s * 48_000.0 / 256.0) as usize];
        assert!(peak > 0.9, "the attack reaches the peak ({peak})");
        assert!((at(2.9) - 0.6).abs() < 0.08, "sustains at the S3 level, got {}", at(2.9));
        assert!(at(5.9) < 0.05, "released to zero, got {}", at(5.9));
    }

    #[test]
    fn a_free_lfo_runs_with_no_gate_and_swings() {
        let (mut a, modbus) = app();
        let probe = modbus.register("Probe: In");
        set_preset(&mut a, "Free LFO");
        a.m.controls.set(seg(0) + 2, 0.9); // fast
        patch_value_to_probe(&mut a, &modbus);
        a.toggle_running();
        let v = trace(&mut a, &probe, 3.0, 0.0);
        let (lo, hi) = v.iter().fold((f32::MAX, f32::MIN), |(l, h), x| (l.min(*x), h.max(*x)));
        assert!(hi - lo > 0.5, "swings: {lo}..{hi}");
    }

    #[test]
    fn a_pulse_generator_fires_on_the_gate_and_ends_by_itself() {
        let (mut a, modbus) = app();
        let probe = modbus.register("Probe: In");
        set_preset(&mut a, "Pulse");
        a.m.controls.set(seg(0) + 3, 0.2); // a short pulse
        patch_value_to_probe(&mut a, &modbus);
        a.toggle_running();
        let v = trace(&mut a, &probe, 5.0, 4.0);
        let early = v[..40].iter().cloned().fold(0.0f32, f32::max);
        assert!(early > 0.8, "fires when the gate rises ({early})");
        assert!(v[v.len() - 5] < 0.05, "back low although the gate is still held ({})", v[v.len() - 5]);
    }

    #[test]
    fn the_oscillator_preset_is_audio() {
        let (mut a, _) = app();
        set_preset(&mut a, "Oscillator");
        a.m.controls.set(seg(0) + 2, 0.6);
        a.toggle_running();
        let mut p = a.audio_processor().unwrap();
        let mut buf = vec![0.0f32; 4800 * 2];
        p.process(&mut buf, 2, 48_000.0);
        p.process(&mut buf, 2, 48_000.0);
        let rms = (buf.iter().map(|v| v * v).sum::<f32>() / buf.len() as f32).sqrt();
        assert!(rms > 0.05 && buf.iter().all(|v| v.is_finite() && v.abs() < 2.0), "audio comes out ({rms})");
    }

    #[test]
    fn every_preset_runs_without_blowing_up() {
        for p in PRESETS.iter().skip(1) {
            let (mut a, modbus) = app();
            let probe = modbus.register("Probe: In");
            set_preset(&mut a, p.name);
            patch_value_to_probe(&mut a, &modbus);
            a.toggle_running();
            let v = trace(&mut a, &probe, 2.0, 1.0);
            assert!(v.iter().all(|x| x.is_finite() && x.abs() < 4.0), "{}: finite and bounded", p.name);
        }
    }

    #[test]
    fn its_sliders_are_mod_inputs() {
        let (_a, modbus) = app();
        for name in ["Stages: S1 Slider", "Stages: S6 Pot", "Mixer: Stages Level"] {
            assert!(modbus.index_of(name).is_some(), "{name}");
        }
    }
}
