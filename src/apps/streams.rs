//! Streams: the real Mutable Instruments Streams (vendor/eurorack/streams, MIT,
//! (c) Emilie Gillet), running its own C++ through vendor/bridge/streams_bridge.cc.
//!
//! Streams is a dual dynamics gate: two channels, each an audio input and an
//! excite input, with a processor that turns the excite into a control signal
//! for the channel's VCA and low-pass filter. The processor is one of six
//! functions: an envelope, a vactrol (the soft, slightly lagging response of the
//! opto-couplers that low-pass gates are made of), an envelope follower, a
//! compressor, a filter controller and a Lorenz generator. Each has two knobs,
//! SHAPE and RESPONSE, and an alternate mode.
//!
//! The module's VCA and filter are analogue circuits; the digital part, which
//! is what runs here, only decides what they do. The channel's gain drives a
//! digital VCA on its audio, and both its gain and its filter-frequency control
//! voltages can be patched to any app's modulation input, so they can drive
//! anything. Pick each channel's **Audio** and **Excite** sources from other
//! apps' outputs; with no excite source, the pads are the excite input: the
//! lower two rows fire A and the upper two fire B, as a gate.

use super::mi_kit::{Controls, Keys, MiApp, Module, NoteQueue, RateBridge, Spec};
use crate::app::play_kit::{KitConfig, Layer, Routes};
use crate::audio::AudioProcessor;
use crate::audio_bus::{cycle_source, AudioBus, NO_SOURCE};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

unsafe extern "C" {
    fn streams_create() -> *mut c_void;
    fn streams_destroy(h: *mut c_void);
    fn streams_set(h: *mut c_void, ch: c_int, function: c_int, alternate: c_int, p0: u16, p1: u16);
    fn streams_process(h: *mut c_void, ch: c_int, audio: *const i16, excite: *const i16, gain: *mut u16, frequency: *mut u16, n: c_int);
}

/// Streams samples at 31.09 kHz (streams/drivers/adc.cc).
const STREAMS_RATE: f32 = 31_090.0;
const BLOCK: usize = 8;

const FUNCTIONS: [&str; 6] = ["Envelope", "Vactrol", "Follower", "Compressor", "Filter controller", "Lorenz"];
const OFF_ON: [&str; 2] = ["Off", "On"];

static SPECS: [Spec; 8] = [
    Spec::switch("A Function", &FUNCTIONS, 1),
    Spec::knob("A Shape", 0.5),
    Spec::knob("A Response", 0.5),
    Spec::switch("A Alternate", &OFF_ON, 0),
    Spec::switch("B Function", &FUNCTIONS, 0),
    Spec::knob("B Shape", 0.5),
    Spec::knob("B Response", 0.5),
    Spec::switch("B Alternate", &OFF_ON, 0),
];

const OUTS: [&str; 4] = ["Gain A", "Freq A", "Gain B", "Freq B"];
const SOURCE_ROWS: [&str; 4] = ["Audio A", "Excite A", "Audio B", "Excite B"];

pub struct Streams {
    controls: Arc<Controls>,
    modbus: Arc<ModBus>,
    audio_bus: Arc<AudioBus>,
    outs: Arc<super::mi_kit::CvOuts>,
    /// Audio A, Excite A, Audio B, Excite B: audio-bus indices.
    sources: Arc<[AtomicUsize; 4]>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    levels: Arc<[AtomicF32; 2]>,
    gates: Arc<[AtomicBool; 2]>,
}

impl Streams {
    fn new(modbus: Arc<ModBus>, audio_bus: Arc<AudioBus>, mixer_bus: &MixerBus) -> Streams {
        let (mix_level, ext_mix_level) = mixer_bus.register("Streams", &modbus);
        Streams {
            controls: Arc::new(Controls::new(&SPECS, "Streams", &modbus)),
            outs: Arc::new(super::mi_kit::CvOuts::new(&OUTS)),
            modbus,
            bus_out: audio_bus.register("Streams"),
            audio_bus,
            sources: Arc::new(std::array::from_fn(|_| AtomicUsize::new(NO_SOURCE))),
            mix_level,
            ext_mix_level,
            levels: Arc::new(std::array::from_fn(|_| AtomicF32::new(0.0))),
            gates: Arc::new(std::array::from_fn(|_| AtomicBool::new(false))),
        }
    }
}

impl Module for Streams {
    const NAME: &'static str = "Streams";
    // Black faceplate, white legends, a warm orange bargraph.
    const PALETTE: [Rgb565; 4] = [Rgb565::new(2, 3, 3), Rgb565::new(29, 58, 28), Rgb565::new(31, 34, 3), Rgb565::new(14, 28, 14)];

    fn controls(&self) -> &Controls {
        &self.controls
    }

    fn kit_config(&self) -> KitConfig {
        KitConfig {
            app_id: "streams",
            layers: vec![Layer::Native(0, "GATES"), Layer::Controls, Layer::Moments],
            hero: vec![[1, 2], [5, 6], [0, 4], [3, 7]],
            browse: Some(0),
            routes: Routes { stick_x: Some(1), stick_y: Some(2), hand_l: Some(5), hand_r: Some(6) },
            throws: Vec::new(),
            midi_to_pads: false,
            own_expression: false,
        }
    }

    fn plays_notes(&self) -> bool {
        true
    }

    fn extra_rows(&self) -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> = SOURCE_ROWS
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let s = self.sources[i].load(Ordering::Relaxed);
                (n.to_string(), if s == NO_SOURCE { if i % 2 == 1 { "pads (gate)".into() } else { "none".into() } } else { self.audio_bus.source_name(s) })
            })
            .collect();
        for (i, name) in OUTS.iter().enumerate() {
            rows.push((format!("{name} App"), self.outs.app_label(&self.modbus, i)));
            rows.push((format!("{name} Input"), self.outs.input_label(&self.modbus, i)));
        }
        rows
    }

    fn edit_extra(&mut self, i: usize, delta: i32) {
        if i < 4 {
            let cur = self.sources[i].load(Ordering::Relaxed);
            self.sources[i].store(cycle_source(cur, delta.signum(), self.audio_bus.len()), Ordering::Relaxed);
        } else if (i - 4) % 2 == 0 {
            self.outs.step_app(&self.modbus, (i - 4) / 2, delta.signum());
        } else {
            self.outs.step_input(&self.modbus, (i - 4) / 2, delta.signum());
        }
    }

    fn reset_extra(&mut self, i: usize) {
        if i < 4 {
            self.sources[i].store(NO_SOURCE, Ordering::Relaxed);
        } else {
            self.outs.clear((i - 4) / 2);
        }
    }

    fn update(&mut self, keys: &Keys) {
        let a = keys.held().iter().any(|&n| n < 56);
        let b = keys.held().iter().any(|&n| n >= 56);
        self.gates[0].store(a, Ordering::Relaxed);
        self.gates[1].store(b, Ordering::Relaxed);
    }

    fn processor(&mut self, notes: Arc<NoteQueue>) -> Box<dyn AudioProcessor> {
        Box::new(StreamsProcessor {
            handle: unsafe { streams_create() },
            controls: Arc::clone(&self.controls),
            audio_bus: Arc::clone(&self.audio_bus),
            modbus: Arc::clone(&self.modbus),
            outs: Arc::clone(&self.outs),
            sources: Arc::clone(&self.sources),
            notes,
            bus_out: Arc::clone(&self.bus_out),
            mix_level: Arc::clone(&self.mix_level),
            ext_mix_level: Arc::clone(&self.ext_mix_level),
            levels: Arc::clone(&self.levels),
            gates: Arc::clone(&self.gates),
            audio_bridges: [RateBridge::new(STREAMS_RATE, BLOCK), RateBridge::new(STREAMS_RATE, BLOCK)],
            excite_bridges: [RateBridge::new(STREAMS_RATE, BLOCK), RateBridge::new(STREAMS_RATE, BLOCK)],
            excite_blocks: [VecDeque::with_capacity(64), VecDeque::with_capacity(64)],
            inputs: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            outputs: [Vec::new(), Vec::new()],
            sink: Vec::new(),
            last: [[0.0; 2]; 2],
        })
    }

    fn line(&self, _keys: &Keys) -> String {
        format!("A: {}   B: {}", FUNCTIONS[self.controls.choice(0).min(5)], FUNCTIONS[self.controls.choice(4).min(5)])
    }

    fn meters(&self) -> Vec<(String, f32)> {
        vec![("A".into(), self.levels[0].get()), ("B".into(), self.levels[1].get())]
    }

    fn needs_background_audio(&self) -> bool {
        self.outs.any() || self.sources.iter().any(|s| s.load(Ordering::Relaxed) != NO_SOURCE)
    }

    fn title(&self) -> String {
        format!("{} / {}", FUNCTIONS[self.controls.choice(0).min(5)], FUNCTIONS[self.controls.choice(4).min(5)])
    }

    fn hint(&self) -> &'static str {
        "pads: excite gates (lower A, upper B)   L/R: dial   U/D: function A   R1: menu (sources, outputs)"
    }
}

struct StreamsProcessor {
    handle: *mut c_void,
    controls: Arc<Controls>,
    audio_bus: Arc<AudioBus>,
    modbus: Arc<ModBus>,
    outs: Arc<super::mi_kit::CvOuts>,
    sources: Arc<[AtomicUsize; 4]>,
    notes: Arc<NoteQueue>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    levels: Arc<[AtomicF32; 2]>,
    gates: Arc<[AtomicBool; 2]>,
    audio_bridges: [RateBridge; 2],
    excite_bridges: [RateBridge; 2],
    /// Excite blocks the excite bridge produced, for the audio bridge's render to consume.
    excite_blocks: [VecDeque<[f32; BLOCK]>; 2],
    inputs: [Vec<f32>; 4],
    outputs: [Vec<[f32; 2]>; 2],
    sink: Vec<[f32; 2]>,
    /// The last gain and frequency of each channel, 0..1.
    last: [[f32; 2]; 2],
}

// The handle is only ever touched from the audio thread.
unsafe impl Send for StreamsProcessor {}

impl Drop for StreamsProcessor {
    fn drop(&mut self) {
        unsafe { streams_destroy(self.handle) };
    }
}

fn copy_source(bus: &AudioBus, idx: usize, frames: usize, into: &mut Vec<f32>) {
    into.clear();
    into.resize(frames, 0.0);
    if idx == NO_SOURCE {
        return;
    }
    if let Some(b) = bus.get(idx) {
        if let Ok(b) = b.try_lock() {
            for (d, s) in into.iter_mut().zip(b.iter()) {
                *d = if s.is_finite() { *s } else { 0.0 };
            }
        }
    }
}

impl AudioProcessor for StreamsProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        buffer.iter_mut().for_each(|s| *s = 0.0);
        let frames = buffer.len() / channels;
        while self.notes.pop().is_some() {}
        let c = &self.controls;
        let u16_of = |v: f32| (v.clamp(0.0, 1.0) * 65535.0) as u16;
        for ch in 0..2usize {
            let base = ch * 4;
            unsafe { streams_set(self.handle, ch as c_int, c.choice(base).min(5) as c_int, c.choice(base + 3) as c_int, u16_of(c.get(base + 1)), u16_of(c.get(base + 2))) };
        }
        // Inputs: audio A, excite A, audio B, excite B (sources 0..3 as in the rows).
        for i in 0..4 {
            copy_source(&self.audio_bus, self.sources[i].load(Ordering::Relaxed), frames, &mut self.inputs[i]);
        }
        let handle = self.handle;
        for ch in 0..2usize {
            let gate = self.gates[ch].load(Ordering::Relaxed);
            let excite_source = self.sources[ch * 2 + 1].load(Ordering::Relaxed) != NO_SOURCE;
            // The excite path goes through its own bridge, so its blocks line up
            // with the audio bridge's: same rate, block and latency.
            self.excite_blocks[ch].clear();
            let blocks = &mut self.excite_blocks[ch];
            self.sink.clear();
            self.sink.resize(frames, [0.0; 2]);
            self.excite_bridges[ch].process(&self.inputs[ch * 2 + 1], &mut self.sink, sample_rate, |inp, _| {
                let mut block = [0.0f32; BLOCK];
                block[..inp.len().min(BLOCK)].copy_from_slice(&inp[..inp.len().min(BLOCK)]);
                if blocks.len() < 256 {
                    blocks.push_back(block);
                }
            });
            self.outputs[ch].clear();
            self.outputs[ch].resize(frames, [0.0; 2]);
            let last = &mut self.last[ch];
            let blocks = &mut self.excite_blocks[ch];
            self.audio_bridges[ch].process(&self.inputs[ch * 2], &mut self.outputs[ch], sample_rate, |inp, o| {
                let mut audio = [0i16; BLOCK];
                let mut excite = [0i16; BLOCK];
                let ex = blocks.pop_front().unwrap_or([0.0; BLOCK]);
                for k in 0..BLOCK.min(inp.len()) {
                    audio[k] = (inp[k].clamp(-1.0, 1.0) * 32767.0) as i16;
                    let e = if excite_source { ex[k] } else { 0.0 } + if gate { 1.0 } else { 0.0 };
                    excite[k] = (e.clamp(-1.0, 1.0) * 32767.0) as i16;
                }
                let (mut gain, mut freq) = ([0u16; BLOCK], [0u16; BLOCK]);
                unsafe { streams_process(handle, ch as c_int, audio.as_ptr(), excite.as_ptr(), gain.as_mut_ptr(), freq.as_mut_ptr(), BLOCK as c_int) };
                for k in 0..BLOCK.min(o.len()) {
                    let g = gain[k] as f32 / 65535.0;
                    let v = inp[k] * g;
                    o[k] = [v, v];
                }
                last[0] = gain[BLOCK - 1] as f32 / 65535.0;
                last[1] = freq[BLOCK - 1] as f32 / 65535.0;
            });
        }
        for ch in 0..2 {
            self.levels[ch].set(self.last[ch][0]);
            self.outs.send(&self.modbus, ch * 2, self.last[ch][0]);
            self.outs.send(&self.modbus, ch * 2 + 1, self.last[ch][1]);
        }
        let mix = (self.mix_level.get() + self.ext_mix_level.get()).clamp(0.0, 2.0) * 0.5;
        let have_audio = self.sources[0].load(Ordering::Relaxed) != NO_SOURCE || self.sources[2].load(Ordering::Relaxed) != NO_SOURCE;
        if let Ok(mut bus) = self.bus_out.try_lock() {
            bus.clear();
            bus.extend((0..frames).map(|i| if have_audio { (self.outputs[0][i][0] + self.outputs[1][i][0]) * mix } else { 0.0 }));
        }
        for (i, frame) in buffer.chunks_mut(channels).enumerate() {
            frame.fill((self.outputs[0][i][0] + self.outputs[1][i][0]) * mix);
        }
    }
}

#[allow(dead_code)] // tests name it
pub type StreamsApp = MiApp<Streams>;

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    let mixer: Arc<MixerBus> = ctx.get();
    Box::new(MiApp::new(Streams::new(ctx.get(), ctx.get(), &mixer), ctx.named("sensitivity"), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Input};

    fn fixture() -> (StreamsApp, Arc<ModBus>, Arc<Mutex<Vec<f32>>>, Arc<Mutex<Vec<f32>>>) {
        let modbus = Arc::new(ModBus::new());
        let bus = Arc::new(AudioBus::new());
        let tone = bus.register("Tone");
        let sidechain = bus.register("Kick");
        let a = MiApp::new(Streams::new(Arc::clone(&modbus), bus, &MixerBus::new()), Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(1.0)));
        (a, modbus, tone, sidechain)
    }

    fn feed(p: &mut Box<dyn AudioProcessor>, tone: &Arc<Mutex<Vec<f32>>>, amp: f32, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for b in 0..blocks {
            *tone.lock().unwrap() = (0..256).map(|n| (std::f32::consts::TAU * 220.0 * (n + b * 256) as f32 / 48_000.0).sin() * amp).collect();
            let mut out = vec![0.0f32; 512];
            p.process(&mut out, 2, 48_000.0);
            assert!(out.iter().all(|v| v.is_finite()));
            all.extend(out.iter().step_by(2));
        }
        all
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn an_envelope_opens_the_vca_on_a_gate_and_closes_it_after() {
        let (mut a, _, tone, _) = fixture();
        a.m.controls.set(0, 0.0); // Envelope
        a.m.controls.set(1, 0.1); // a short decay
        a.m.sources[0].store(0, Ordering::Relaxed); // audio A: Tone
        let mut p = a.audio_processor().unwrap();
        let shut = feed(&mut p, &tone, 0.5, 40);
        assert!(rms(&shut[shut.len() - 4800..]) < 0.01, "closed with no excite: {}", rms(&shut[shut.len() - 4800..]));
        a.tick(&Input { grid: std::array::from_fn(|i| i == 12), ..Default::default() });
        let open = feed(&mut p, &tone, 0.5, 60);
        assert!(rms(&open) > 0.03, "a gate opens it ({})", rms(&open));
        a.tick(&Input::default());
        let after = feed(&mut p, &tone, 0.5, 600);
        assert!(rms(&after[after.len() - 4800..]) < rms(&open) * 0.5, "and it closes again");
    }

    #[test]
    fn a_follower_opens_with_the_excite_signal_and_its_gain_can_be_patched() {
        let (mut a, modbus, tone, kick) = fixture();
        let probe = modbus.register("Probe: In");
        a.m.controls.set(0, 2.0); // Follower
        a.m.sources[0].store(0, Ordering::Relaxed); // audio A: Tone
        a.m.sources[1].store(1, Ordering::Relaxed); // excite A: Kick
        let mut guard = 0;
        while a.m.outs.app_label(&modbus, 0) != "Probe" && guard < 20 {
            a.m.edit_extra(4, 1);
            guard += 1;
        }
        let mut p = a.audio_processor().unwrap();
        let mut buf = vec![0.0f32; 512];
        *kick.lock().unwrap() = vec![0.0; 256];
        let quiet = feed(&mut p, &tone, 0.5, 80);
        let quiet_gain = probe.get();
        *kick.lock().unwrap() = (0..256).map(|n| (std::f32::consts::TAU * 150.0 * n as f32 / 48_000.0).sin() * 0.9).collect();
        for _ in 0..80 {
            *tone.lock().unwrap() = vec![0.3; 256];
            p.process(&mut buf, 2, 48_000.0);
        }
        assert!(probe.get() > quiet_gain + 0.05, "a loud excite raises the patched gain: {quiet_gain} -> {}", probe.get());
        let _ = quiet;
    }

    #[test]
    fn every_function_runs_and_stays_bounded() {
        for func in 0..FUNCTIONS.len() {
            let (mut a, _, tone, _) = fixture();
            a.m.controls.set(0, func as f32);
            a.m.controls.set(4, func as f32);
            a.m.sources[0].store(0, Ordering::Relaxed);
            a.m.sources[2].store(0, Ordering::Relaxed);
            let mut p = a.audio_processor().unwrap();
            a.tick(&Input { grid: std::array::from_fn(|i| i == 12 || i == 0), ..Default::default() });
            let out = feed(&mut p, &tone, 0.7, 60);
            assert!(out.iter().all(|v| v.abs() <= 2.0), "{}: bounded", FUNCTIONS[func]);
        }
    }

    #[test]
    fn its_knobs_are_mod_inputs() {
        let (_a, modbus, _, _) = fixture();
        for name in ["Streams: A Shape", "Streams: B Response", "Mixer: Streams Level"] {
            assert!(modbus.index_of(name).is_some(), "{name}");
        }
    }
}
