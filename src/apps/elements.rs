//! Elements: the real Mutable Instruments Elements modal voice
//! (vendor/eurorack/elements, MIT, (c) Emilie Gillet), running its own C++
//! DSP through vendor/bridge/elements_bridge.cc.
//!
//! Elements is a whole physical-modelling voice: three exciters -- a bow,
//! a breath (blow) and a mallet (strike) -- feeding a resonator (modal
//! body, string, or a set of sympathetic strings) and a reverb. The
//! panel is the module's: the exciter levels and timbres, the envelope
//! CONTOUR, GEOMETRY, BRIGHTNESS, DAMPING, POSITION and SPACE (whose top
//! end freezes the reverb). The fourth model is the module's hidden
//! "Ominous" drone voice.
//!
//! The pads play it chromatically from C3; the gate stays open while any
//! note is held (bowed and blown sounds sustain), the newest held note
//! sounds, and a new note re-strikes. Another app's audio can excite the
//! resonator through the Input row, as with the module's STRIKE input.

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
    fn elements_block_size() -> c_int;
    fn elements_create(seed: u32) -> *mut c_void;
    fn elements_destroy(h: *mut c_void);
    #[allow(clippy::too_many_arguments)]
    fn elements_render(
        h: *mut c_void,
        panel: *const f32,
        model: c_int,
        gate: c_int,
        note: f32,
        modulation: f32,
        strength: f32,
        blow_in: *const f32,
        strike_in: *const f32,
        main_out: *mut f32,
        aux_out: *mut f32,
        n: c_int,
    );
}

/// Elements' fixed rate (elements/dsp/dsp.h).
const ELEMENTS_RATE: f32 = 32_000.0;
/// The panel controls the bridge takes, in elements::Patch order.
const PANEL: usize = 14;

const MODELS: [&str; 4] = ["Modal", "String", "Strings", "Ominous"];

const C_CONTOUR: usize = 0;
const C_BOW: usize = 1;
const C_BLOW: usize = 3;
const C_STRIKE: usize = 6;
const C_MALLET: usize = 7;
const C_GEOMETRY: usize = 9;
const C_BRIGHTNESS: usize = 10;
const C_DAMPING: usize = 11;
const C_POSITION: usize = 12;
const C_SPACE: usize = 13;
const C_MODEL: usize = 14;
const C_OCTAVE: usize = 15;
const C_FM: usize = 16;

fn space(v: f32) -> String {
    // Part::Process freezes the reverb from 1.75 up (elements/dsp/part.cc).
    if v >= 1.75 { "freeze".into() } else { format!("{:.0}%", v / 2.0 * 100.0) }
}

/// Defaults are Part::Init's (elements/dsp/part.cc), so it opens as the
/// module does: a struck modal body.
static SPECS: [Spec; 17] = [
    Spec::knob("Contour", 1.0),
    Spec::knob("Bow", 0.0),
    Spec::knob("Bow Timbre", 0.5),
    Spec::knob("Blow", 0.0),
    Spec::knob("Flow", 0.5),
    Spec::knob("Blow Timbre", 0.5),
    Spec::knob("Strike", 0.8),
    Spec::knob("Mallet", 0.5),
    Spec::knob("Strike Timbre", 0.5),
    Spec::knob("Geometry", 0.2),
    Spec::knob("Brightness", 0.5),
    Spec::knob("Damping", 0.25),
    Spec::knob("Position", 0.3),
    Spec::range("Space", 0.0, 2.0, 0.5, space),
    Spec::switch("Model", &MODELS, 0),
    Spec { modulated: false, ..Spec::range("Octave", -3.0, 3.0, 0.0, octaves) },
    Spec::range("FM", -12.0, 12.0, 0.0, semitones),
];

pub struct Elements {
    controls: Arc<Controls>,
    audio_bus: Arc<AudioBus>,
    source: Arc<AtomicUsize>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    level: Arc<AtomicF32>,
}

impl Elements {
    fn new(modbus: &ModBus, audio_bus: Arc<AudioBus>, mixer_bus: &MixerBus) -> Elements {
        let (mix_level, ext_mix_level) = mixer_bus.register("Elements", modbus);
        Elements {
            controls: Arc::new(Controls::new(&SPECS, "Elements", modbus)),
            bus_out: audio_bus.register("Elements"),
            audio_bus,
            source: Arc::new(AtomicUsize::new(NO_SOURCE)),
            mix_level,
            ext_mix_level,
            level: Arc::new(AtomicF32::new(0.0)),
        }
    }
}

impl Module for Elements {
    const NAME: &'static str = "Elements";
    // Deep blue and pale gold.
    const PALETTE: [Rgb565; 4] = [Rgb565::new(2, 4, 8), Rgb565::new(28, 56, 26), Rgb565::new(30, 46, 8), Rgb565::new(12, 24, 18)];

    fn controls(&self) -> &Controls {
        &self.controls
    }

    fn kit_config(&self) -> KitConfig {
        KitConfig {
            app_id: "elements",
            layers: vec![Layer::Native(0, "KEYS"), Layer::Controls, Layer::Moments],
            hero: vec![[C_GEOMETRY, C_BRIGHTNESS], [C_DAMPING, C_POSITION], [C_STRIKE, C_BOW], [C_BLOW, C_SPACE], [C_MALLET, C_CONTOUR]],
            browse: Some(C_MODEL),
            // Stick: brightness and position; hands: bow and breath, so
            // a hand in the beam bows or blows the held note.
            routes: Routes { stick_x: Some(C_BRIGHTNESS), stick_y: Some(C_POSITION), hand_l: Some(C_BOW), hand_r: Some(C_BLOW) },
            throws: Vec::new(),
            midi_to_pads: false,
            own_expression: false,
        }
    }

    fn octave(&self) -> i32 {
        self.controls.raw(C_OCTAVE).round() as i32
    }

    fn extra_rows(&self) -> Vec<(String, String)> {
        vec![("Input".into(), match self.source.load(Ordering::Relaxed) {
            NO_SOURCE => "none".into(),
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
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1);
        Box::new(ElementsProcessor {
            handle: unsafe { elements_create(seed) },
            controls: Arc::clone(&self.controls),
            notes,
            audio_bus: Arc::clone(&self.audio_bus),
            source: Arc::clone(&self.source),
            bus_out: Arc::clone(&self.bus_out),
            mix_level: Arc::clone(&self.mix_level),
            ext_mix_level: Arc::clone(&self.ext_mix_level),
            level: Arc::clone(&self.level),
            bridge: RateBridge::new(ELEMENTS_RATE, unsafe { elements_block_size() } as usize),
            held: Vec::with_capacity(16),
            note: 48.0,
            strength: 0.8,
            retrigger: false,
            input: Vec::new(),
            stereo: Vec::new(),
        })
    }

    fn meters(&self) -> Vec<(String, f32)> {
        let c = &self.controls;
        vec![
            ("bow".into(), c.get(C_BOW)),
            ("blow".into(), c.get(C_BLOW)),
            ("strike".into(), c.get(C_STRIKE)),
            ("geometry".into(), c.get(C_GEOMETRY)),
            ("damping".into(), c.get(C_DAMPING)),
            ("output".into(), self.level.get().sqrt().min(1.0)),
        ]
    }

    fn title(&self) -> String {
        MODELS[self.controls.choice(C_MODEL).min(MODELS.len() - 1)].to_string()
    }

    fn hint(&self) -> &'static str {
        "pads: play (hold to bow/blow)   L/R: dial   U/D: model   F2: pads   R1: menu"
    }
}

struct ElementsProcessor {
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
    /// Held notes, oldest first: the newest one sounds.
    held: Vec<u8>,
    note: f32,
    strength: f32,
    /// A new note while the gate is already open: close it for one block
    /// so the exciters' envelope starts again, as a fresh gate would.
    retrigger: bool,
    input: Vec<f32>,
    stereo: Vec<[f32; 2]>,
}

// The handle is only ever touched from the audio thread.
unsafe impl Send for ElementsProcessor {}

impl Drop for ElementsProcessor {
    fn drop(&mut self) {
        unsafe { elements_destroy(self.handle) };
    }
}

impl AudioProcessor for ElementsProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        while let Some(e) = self.notes.pop() {
            let was_open = !self.held.is_empty();
            self.held.retain(|&n| n != e.note);
            if e.velocity > 0 && self.held.len() < 16 {
                self.held.push(e.note);
                self.strength = e.velocity as f32 / 127.0;
                self.retrigger |= was_open;
            }
        }
        if let Some(&n) = self.held.last() {
            self.note = n as f32;
        }

        self.input.clear();
        self.input.resize(frames, 0.0);
        let source = self.source.load(Ordering::Relaxed);
        if source != NO_SOURCE {
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
        let panel: [f32; PANEL] = std::array::from_fn(|i| c.get(i));
        let model = c.choice(C_MODEL) as c_int;
        let fm = c.get(C_FM);
        let handle = self.handle;
        let block = (unsafe { elements_block_size() } as usize).min(64);
        let gate_open = !self.held.is_empty();
        let (note, strength) = (self.note, self.strength);
        let retrigger = &mut self.retrigger;
        let silence = [0.0f32; 64];
        self.bridge.process(&self.input, &mut self.stereo, sample_rate, |inp, o| {
            let gate = gate_open && !*retrigger;
            *retrigger = false;
            let mut main = [0.0f32; 64];
            let mut aux = [0.0f32; 64];
            unsafe {
                elements_render(handle, panel.as_ptr(), model, gate as c_int, note, fm, strength, silence.as_ptr(), inp.as_ptr(), main.as_mut_ptr(), aux.as_mut_ptr(), block as c_int);
            }
            // The firmware sends `aux` to the left jack and `main` to the
            // right (elements/dsp/part.cc's mixdown).
            for k in 0..block {
                o[k] = [aux[k], main[k]];
            }
        });

        let mix = (self.mix_level.get() + self.ext_mix_level.get()).clamp(0.0, 2.0);
        if let Ok(mut bus) = self.bus_out.try_lock() {
            bus.clear();
            bus.extend(self.stereo.iter().map(|f| (f[0] + f[1]) * 0.5));
        }
        let mut energy = 0.0;
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

#[allow(dead_code)] // tests name it
pub type ElementsApp = MiApp<Elements>;

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    let modbus: Arc<ModBus> = ctx.get();
    let mixer: Arc<MixerBus> = ctx.get();
    Box::new(MiApp::new(Elements::new(&modbus, ctx.get(), &mixer), ctx.named("sensitivity"), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Input};

    fn app() -> ElementsApp {
        let modbus = ModBus::new();
        MiApp::new(Elements::new(&modbus, Arc::new(AudioBus::new()), &MixerBus::new()), Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(1.0)))
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

    fn hold(a: &mut ElementsApp, pad: usize, down: bool) {
        a.tick(&Input { grid: std::array::from_fn(|i| i == pad && down), ..Default::default() });
    }

    #[test]
    fn a_struck_note_rings_on_every_model() {
        for model in 0..MODELS.len() {
            let mut a = app();
            a.m.controls.set(C_MODEL, model as f32);
            let mut p = a.audio_processor().unwrap();
            hold(&mut a, 12, true);
            let (energy, peak) = render(&mut p, 40);
            assert!(energy > 0.05, "{}: sounds ({energy})", MODELS[model]);
            assert!(peak < 2.0, "{}: bounded ({peak})", MODELS[model]);
        }
    }

    #[test]
    fn a_bowed_note_sustains_while_held_and_stops_after() {
        let mut a = app();
        a.m.controls.set(C_STRIKE, 0.0);
        a.m.controls.set(C_BOW, 1.0);
        a.m.controls.set(C_SPACE, 0.0);
        let mut p = a.audio_processor().unwrap();
        hold(&mut a, 12, true);
        render(&mut p, 40);
        let (late, _) = render(&mut p, 20);
        assert!(late > 0.01, "the bow keeps it sounding ({late})");
        hold(&mut a, 12, false);
        render(&mut p, 200);
        let (after, _) = render(&mut p, 20);
        assert!(after < late * 0.05, "and lets go when released ({after} vs {late})");
    }

    #[test]
    fn its_knobs_are_mod_inputs() {
        let modbus = ModBus::new();
        let _e = Elements::new(&modbus, Arc::new(AudioBus::new()), &MixerBus::new());
        let c = Controls::new(&SPECS, "Elements", &ModBus::new());
        for name in c.mod_input_names("Elements") {
            assert!(modbus.index_of(&name).is_some(), "{name}");
        }
        assert!(modbus.index_of("Mixer: Elements Level").is_some());
    }
}
