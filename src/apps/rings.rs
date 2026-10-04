//! Rings: the real Mutable Instruments Rings resonator (vendor/eurorack/
//! rings, MIT, (c) Emilie Gillet), running its own C++ DSP through
//! vendor/bridge/rings_bridge.cc.
//!
//! Rings is a resonator: modal bodies, sympathetic and inharmonic strings,
//! a two-operator FM voice, and the hidden "Disastrous Peace" string
//! synth. Each note strums it -- with its own internal exciter, or with
//! another app's audio when one is picked as the Exciter input, the way
//! patching a drum into the module's IN works. Up to four voices ring at
//! once; each new note goes to the next voice, as on the module.
//!
//! The panel maps one to one: STRUCTURE, BRIGHTNESS, DAMPING, POSITION,
//! the resonator model, polyphony, and FM (the FREQUENCY attenuverter's
//! job). The pads play chromatically from C3; any keyboard or app on the
//! note bus plays it too.

use super::mi_kit::{octaves, semitones, Controls, MiApp, Module, NoteQueue, RateBridge, Spec};
use crate::app::play_kit::{KitConfig, Layer, Routes};
use crate::audio::AudioProcessor;
use crate::audio_bus::{AudioBus, NO_SOURCE};
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::ffi::c_void;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

unsafe extern "C" {
    fn rings_block_size() -> c_int;
    fn rings_create() -> *mut c_void;
    fn rings_destroy(h: *mut c_void);
    #[allow(clippy::too_many_arguments)]
    fn rings_render(
        h: *mut c_void,
        model: c_int,
        fx: c_int,
        polyphony: c_int,
        structure: f32,
        brightness: f32,
        damping: f32,
        position: f32,
        note: f32,
        fm: f32,
        strum: c_int,
        internal_exciter: c_int,
        input: *const f32,
        out: *mut f32,
        aux: *mut f32,
        n: c_int,
    );
}

/// Rings' fixed rate (rings/dsp/dsp.h).
const RINGS_RATE: f32 = 48_000.0;

const MODELS: [&str; 7] = ["Modal", "Sympathetic", "String", "FM Voice", "Symp. Quant", "String+Verb", "Dis. Peace"];
const POLY: [&str; 3] = ["1", "2", "4"];
const FX: [&str; 6] = ["Formant", "Chorus", "Reverb", "Formant 2", "Ensemble", "Reverb 2"];

const C_STRUCTURE: usize = 0;
const C_BRIGHTNESS: usize = 1;
const C_DAMPING: usize = 2;
const C_POSITION: usize = 3;
const C_MODEL: usize = 4;
const C_POLY: usize = 5;
const C_OCTAVE: usize = 6;
const C_FM: usize = 7;
const C_FX: usize = 8;

static SPECS: [Spec; 9] = [
    Spec::knob("Structure", 0.3),
    Spec::knob("Brightness", 0.5),
    Spec::knob("Damping", 0.6),
    Spec::knob("Position", 0.4),
    Spec::switch("Model", &MODELS, 0),
    Spec::switch("Polyphony", &POLY, 2),
    Spec { modulated: false, ..Spec::range("Octave", -3.0, 3.0, 0.0, octaves) },
    Spec::range("FM", -12.0, 12.0, 0.0, semitones),
    Spec::switch("Peace FX", &FX, 1),
];

pub struct Rings {
    controls: Arc<Controls>,
    audio_bus: Arc<AudioBus>,
    /// The Exciter input: an audio-bus index, or NO_SOURCE for the
    /// internal exciter.
    source: Arc<AtomicUsize>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    level: Arc<AtomicF32>,
}

impl Rings {
    fn new(modbus: &ModBus, audio_bus: Arc<AudioBus>, mixer_bus: &MixerBus) -> Rings {
        let (mix_level, ext_mix_level) = mixer_bus.register("Rings", modbus);
        Rings {
            controls: Arc::new(Controls::new(&SPECS, "Rings", modbus)),
            bus_out: audio_bus.register("Rings"),
            audio_bus,
            source: Arc::new(AtomicUsize::new(NO_SOURCE)),
            mix_level,
            ext_mix_level,
            level: Arc::new(AtomicF32::new(0.0)),
        }
    }
}

impl Module for Rings {
    const NAME: &'static str = "Rings";
    // Brass and ivory, after the module's faceplate.
    const PALETTE: [Rgb565; 4] = [Rgb565::new(4, 6, 3), Rgb565::new(30, 56, 22), Rgb565::new(28, 44, 6), Rgb565::new(17, 30, 12)];

    fn controls(&self) -> &Controls {
        &self.controls
    }

    fn kit_config(&self) -> KitConfig {
        KitConfig {
            app_id: "rings",
            layers: vec![Layer::Native(0, "KEYS"), Layer::Controls, Layer::Moments],
            hero: vec![[C_STRUCTURE, C_BRIGHTNESS], [C_DAMPING, C_POSITION], [C_MODEL, C_POLY], [C_OCTAVE, C_FM]],
            browse: Some(C_MODEL),
            // Stick: brightness and position, the two you play most;
            // hands: damping and structure.
            routes: Routes { stick_x: Some(C_BRIGHTNESS), stick_y: Some(C_POSITION), hand_l: Some(C_DAMPING), hand_r: Some(C_STRUCTURE) },
            throws: Vec::new(),
            midi_to_pads: false,
            own_expression: false,
        }
    }

    fn octave(&self) -> i32 {
        self.controls.raw(C_OCTAVE).round() as i32
    }

    fn extra_rows(&self) -> Vec<(String, String)> {
        vec![("Exciter".into(), match self.source.load(Ordering::Relaxed) {
            NO_SOURCE => "internal".into(),
            i => self.audio_bus.source_name(i),
        })]
    }

    fn edit_extra(&mut self, _i: usize, delta: i32) {
        let cur = self.source.load(Ordering::Relaxed);
        self.source.store(crate::audio_bus::cycle_source(cur, delta.signum(), self.audio_bus.len()), Ordering::Relaxed);
    }

    fn reset_extra(&mut self, _i: usize) {
        self.source.store(NO_SOURCE, Ordering::Relaxed);
    }

    fn processor(&mut self, notes: Arc<NoteQueue>) -> Box<dyn AudioProcessor> {
        let block = unsafe { rings_block_size() } as usize;
        Box::new(RingsProcessor {
            handle: unsafe { rings_create() },
            controls: Arc::clone(&self.controls),
            notes,
            audio_bus: Arc::clone(&self.audio_bus),
            source: Arc::clone(&self.source),
            bus_out: Arc::clone(&self.bus_out),
            mix_level: Arc::clone(&self.mix_level),
            ext_mix_level: Arc::clone(&self.ext_mix_level),
            level: Arc::clone(&self.level),
            bridge: RateBridge::new(RINGS_RATE, block),
            note: 48.0,
            pending: Vec::with_capacity(16),
            input: Vec::new(),
            stereo: Vec::new(),
        })
    }

    fn meters(&self) -> Vec<(String, f32)> {
        let c = &self.controls;
        vec![
            ("structure".into(), c.get(C_STRUCTURE)),
            ("brightness".into(), c.get(C_BRIGHTNESS)),
            ("damping".into(), c.get(C_DAMPING)),
            ("position".into(), c.get(C_POSITION)),
            ("output".into(), self.level.get().sqrt().min(1.0)),
        ]
    }

    fn title(&self) -> String {
        MODELS[self.controls.choice(C_MODEL).min(MODELS.len() - 1)].to_string()
    }

    fn hint(&self) -> &'static str {
        "pads: strum   L/R: dial (SELECT: next)   U/D: model   F2: pads   R1: menu"
    }
}

struct RingsProcessor {
    handle: *mut c_void,
    controls: Arc<Controls>,
    notes: Arc<NoteQueue>,
    audio_bus: Arc<AudioBus>,
    source: Arc<AtomicUsize>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    level: Arc<AtomicF32>,
    bridge: RateBridge,
    note: f32,
    /// Note-ons waiting for a block of their own: Rings takes one strum
    /// per block, so a chord is strummed over a few half-milliseconds.
    pending: Vec<u8>,
    input: Vec<f32>,
    stereo: Vec<[f32; 2]>,
}

// The handle is only ever touched from the audio thread.
unsafe impl Send for RingsProcessor {}

impl Drop for RingsProcessor {
    fn drop(&mut self) {
        unsafe { rings_destroy(self.handle) };
    }
}

impl AudioProcessor for RingsProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        while let Some(e) = self.notes.pop() {
            if e.velocity > 0 && self.pending.len() < 16 {
                self.pending.push(e.note);
            }
        }
        self.input.clear();
        self.input.resize(frames, 0.0);
        let source = self.source.load(Ordering::Relaxed);
        let external = source != NO_SOURCE;
        if external {
            if let Some(src) = self.audio_bus.get(source) {
                if let Ok(src) = src.try_lock() {
                    for (d, s) in self.input.iter_mut().zip(src.iter()) {
                        *d = *s;
                    }
                }
            }
        }
        self.stereo.clear();
        self.stereo.resize(frames, [0.0; 2]);

        let c = &self.controls;
        let model = c.choice(C_MODEL) as c_int;
        let fx = c.choice(C_FX) as c_int;
        let poly = [1, 2, 4][c.choice(C_POLY).min(2)];
        let (structure, brightness, damping, position, fm) = (c.get(C_STRUCTURE), c.get(C_BRIGHTNESS), c.get(C_DAMPING), c.get(C_POSITION), c.get(C_FM));
        let handle = self.handle;
        let block = self.bridge_block();
        let pending = &mut self.pending;
        let note = &mut self.note;
        self.bridge.process(&self.input, &mut self.stereo, sample_rate, |inp, o| {
            let mut strum = 0;
            if !pending.is_empty() {
                *note = pending.remove(0) as f32;
                strum = 1;
            }
            let mut l = [0.0f32; 64];
            let mut r = [0.0f32; 64];
            unsafe {
                rings_render(handle, model, fx, poly, structure, brightness, damping, position, *note, fm, strum, (!external) as c_int, inp.as_ptr(), l.as_mut_ptr(), r.as_mut_ptr(), block as c_int);
            }
            for k in 0..block {
                o[k] = [l[k], r[k]];
            }
        });

        let mix = (self.mix_level.get() + self.ext_mix_level.get()).clamp(0.0, 2.0);
        let mut energy = 0.0;
        if let Ok(mut bus) = self.bus_out.try_lock() {
            bus.clear();
            bus.extend(self.stereo.iter().map(|f| (f[0] + f[1]) * 0.5));
        }
        for (frame, s) in buffer.chunks_mut(channels).zip(self.stereo.iter()) {
            energy += s[0] * s[0];
            match frame {
                [a, b, ..] => {
                    *a = s[0] * mix;
                    *b = s[1] * mix;
                }
                [a] => *a = (s[0] + s[1]) * 0.5 * mix,
                [] => {}
            }
        }
        self.level.set(energy / frames.max(1) as f32);
    }
}

impl RingsProcessor {
    fn bridge_block(&self) -> usize {
        unsafe { rings_block_size() as usize }.min(64)
    }
}

#[allow(dead_code)] // tests name it
pub type RingsApp = MiApp<Rings>;

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    let modbus: Arc<ModBus> = ctx.get();
    let mixer: Arc<MixerBus> = ctx.get();
    Box::new(MiApp::new(Rings::new(&modbus, ctx.get(), &mixer), ctx.named("sensitivity"), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Input};

    fn app() -> RingsApp {
        let modbus = ModBus::new();
        MiApp::new(Rings::new(&modbus, Arc::new(AudioBus::new()), &MixerBus::new()), Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(1.0)))
    }

    fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> (f32, f32) {
        let mut buf = vec![0.0f32; 512 * 2];
        let (mut energy, mut peak) = (0.0, 0.0f32);
        for _ in 0..blocks {
            p.process(&mut buf, 2, 48_000.0);
            for v in &buf {
                assert!(v.is_finite());
                energy += v * v;
                peak = peak.max(v.abs());
            }
        }
        (energy, peak)
    }

    #[test]
    fn a_pad_strums_every_model_and_it_rings_out() {
        for model in 0..MODELS.len() {
            let mut a = app();
            a.m.controls.set(C_MODEL, model as f32);
            let mut p = a.audio_processor().unwrap();
            let (silence, _) = render(&mut p, 5);
            assert!(silence < 1e-6, "{}: quiet before a note ({silence})", MODELS[model]);
            a.tick(&Input { grid: std::array::from_fn(|i| i == 12), ..Default::default() });
            let (energy, peak) = render(&mut p, 40);
            assert!(energy > 0.05, "{}: a pad strums it ({energy})", MODELS[model]);
            assert!(peak < 4.0, "{}: bounded ({peak})", MODELS[model]);
        }
    }

    #[test]
    fn notes_from_a_keyboard_play_at_their_pitch() {
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        let mut input = Input::default();
        input.midi_keys.0[69] = 100;
        a.tick(&input);
        render(&mut p, 2);
        let rp = p.as_ref() as *const dyn AudioProcessor as *const RingsProcessor;
        assert_eq!(unsafe { (*rp).note }, 69.0);
    }

    #[test]
    fn its_knobs_are_mod_inputs() {
        let modbus = ModBus::new();
        let _r = Rings::new(&modbus, Arc::new(AudioBus::new()), &MixerBus::new());
        for name in ["Rings: Structure", "Rings: Brightness", "Rings: Damping", "Rings: Position", "Rings: FM", "Mixer: Rings Level"] {
            assert!(modbus.index_of(name).is_some(), "{name}");
        }
    }
}
