//! Braids: the real Mutable Instruments Braids macro oscillator
//! (vendor/eurorack/braids, MIT, (c) Emilie Gillet), running its own C++
//! DSP through vendor/bridge/braids_bridge.cc.
//!
//! Braids is one voice with 47 oscillator models behind two knobs: analog
//! style saws, squares and sub/sync/triple stacks, filtered and vocal
//! shapes (Z-filters, VOSIM, vowels), FM, physical models (plucked, bowed,
//! blown, bells, drums), wavetables, noises and the digital modulation
//! modes. TIMBRE and COLOR mean something different in each. An AD
//! envelope can move timbre, colour, pitch and the VCA, and the output can
//! be bit- and rate-reduced, as on the module.
//!
//! Each note strikes it; it is monophonic, last note wins, like the
//! module's single trigger input. With the envelope's VCA off it drones.
//!
//! Left out, deliberately: the firmware's quantizer (the pads already pick
//! notes), and each unit's "signature" waveshaper and pitch drift, which
//! exist to make two real modules differ from one another.

use super::mi_kit::{octaves, semitones, Controls, MiApp, Module, NoteQueue, RateBridge, Spec};
use crate::app::play_kit::{KitConfig, Layer, Routes};
use crate::audio::AudioProcessor;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::ffi::c_void;
use std::os::raw::c_int;
use std::sync::{Arc, Mutex};

unsafe extern "C" {
    fn braids_block_size() -> c_int;
    fn braids_create() -> *mut c_void;
    fn braids_destroy(h: *mut c_void);
    #[allow(clippy::too_many_arguments)]
    fn braids_render(
        h: *mut c_void,
        shape: c_int,
        pitch: c_int,
        timbre: f32,
        color: f32,
        strike: c_int,
        ad_attack: c_int,
        ad_decay: c_int,
        ad_timbre: c_int,
        ad_color: c_int,
        ad_fm: c_int,
        ad_vca: c_int,
        resolution: c_int,
        rate: c_int,
        out: *mut f32,
    );
}

/// Braids' fixed rate.
const BRAIDS_RATE: f32 = 96_000.0;

/// In the firmware's order (braids/settings.h's MacroOscillatorShape).
const SHAPES: [&str; 47] = [
    "Csaw", "Morph", "Saw/Square", "Sine/Triangle", "Buzz", "Square Sub", "Saw Sub", "Square Sync", "Saw Sync", "Triple Saw",
    "Triple Square", "Triple Triangle", "Triple Sine", "Triple Ring Mod", "Saw Swarm", "Saw Comb", "Toy", "Z LP Filter", "Z Peak Filter",
    "Z BP Filter", "Z HP Filter", "VOSIM", "Vowel", "Vowel FOF", "Harmonics", "FM", "Feedback FM", "Chaotic FM", "Plucked", "Bowed",
    "Blown", "Fluted", "Struck Bell", "Struck Drum", "Kick", "Cymbal", "Snare", "Wavetables", "Wave Map", "Wave Line", "Wave x4",
    "Filtered Noise", "Twin Peaks", "Clocked Noise", "Granular Cloud", "Particle Noise", "Digital Mod",
];
const OFF_ON: [&str; 2] = ["Off", "On"];
const BITS: [&str; 7] = ["2 bit", "3 bit", "4 bit", "6 bit", "8 bit", "12 bit", "16 bit"];
const RATES: [&str; 7] = ["4 kHz", "8 kHz", "16 kHz", "24 kHz", "32 kHz", "48 kHz", "96 kHz"];

const C_SHAPE: usize = 0;
const C_TIMBRE: usize = 1;
const C_COLOR: usize = 2;
const C_FM: usize = 3;
const C_OCTAVE: usize = 4;
const C_ATTACK: usize = 5;
const C_DECAY: usize = 6;
const C_ENV_TIMBRE: usize = 7;
const C_ENV_COLOR: usize = 8;
const C_ENV_FM: usize = 9;
const C_ENV_VCA: usize = 10;
const C_BITS: usize = 11;
const C_RATE: usize = 12;

static SPECS: [Spec; 13] = [
    Spec::switch("Shape", &SHAPES, 0),
    Spec::knob("Timbre", 0.5),
    Spec::knob("Color", 0.5),
    Spec::range("FM", -12.0, 12.0, 0.0, semitones),
    Spec { modulated: false, ..Spec::range("Octave", -3.0, 3.0, 0.0, octaves) },
    Spec::knob("Attack", 0.0),
    Spec::knob("Decay", 0.45),
    Spec::knob("Env > Timbre", 0.0),
    Spec::knob("Env > Color", 0.0),
    Spec::knob("Env > Pitch", 0.0),
    Spec::switch("Env > VCA", &OFF_ON, 1),
    Spec::switch("Bits", &BITS, 6),
    Spec::switch("Sample rate", &RATES, 6),
];

pub struct Braids {
    controls: Arc<Controls>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    level: Arc<AtomicF32>,
    bus_out: Arc<Mutex<Vec<f32>>>,
}

impl Braids {
    fn new(modbus: &ModBus, audio_bus: &crate::audio_bus::AudioBus, mixer_bus: &MixerBus) -> Braids {
        let (mix_level, ext_mix_level) = mixer_bus.register("Braids", modbus);
        Braids {
            controls: Arc::new(Controls::new(&SPECS, "Braids", modbus)),
            mix_level,
            ext_mix_level,
            level: Arc::new(AtomicF32::new(0.0)),
            bus_out: audio_bus.register("Braids"),
        }
    }
}

impl Module for Braids {
    const NAME: &'static str = "Braids";
    // Black faceplate, white legends, a warm red accent.
    const PALETTE: [Rgb565; 4] = [Rgb565::new(2, 3, 3), Rgb565::new(30, 58, 28), Rgb565::new(30, 24, 8), Rgb565::new(15, 28, 14)];

    fn controls(&self) -> &Controls {
        &self.controls
    }

    fn kit_config(&self) -> KitConfig {
        KitConfig {
            app_id: "braids",
            layers: vec![Layer::Native(0, "KEYS"), Layer::Controls, Layer::Moments],
            hero: vec![[C_TIMBRE, C_COLOR], [C_SHAPE, C_FM], [C_ATTACK, C_DECAY], [C_ENV_TIMBRE, C_ENV_COLOR]],
            browse: Some(C_SHAPE),
            // Timbre and colour are what you play on a Braids; the hands bend
            // pitch and shape the decay.
            routes: Routes { stick_x: Some(C_TIMBRE), stick_y: Some(C_COLOR), hand_l: Some(C_FM), hand_r: Some(C_DECAY) },
            throws: Vec::new(),
            midi_to_pads: false,
            own_expression: false,
        }
    }

    fn octave(&self) -> i32 {
        self.controls.raw(C_OCTAVE).round() as i32
    }

    fn processor(&mut self, notes: Arc<NoteQueue>) -> Box<dyn AudioProcessor> {
        let block = unsafe { braids_block_size() } as usize;
        Box::new(BraidsProcessor {
            handle: unsafe { braids_create() },
            controls: Arc::clone(&self.controls),
            notes,
            mix_level: Arc::clone(&self.mix_level),
            ext_mix_level: Arc::clone(&self.ext_mix_level),
            level: Arc::clone(&self.level),
            bus_out: Arc::clone(&self.bus_out),
            bridge: RateBridge::new(BRAIDS_RATE, block),
            note: 48.0,
            strike: false,
            stereo: Vec::new(),
        })
    }

    fn meters(&self) -> Vec<(String, f32)> {
        let c = &self.controls;
        vec![("timbre".into(), c.get(C_TIMBRE)), ("color".into(), c.get(C_COLOR)), ("output".into(), self.level.get().sqrt().min(1.0))]
    }

    fn title(&self) -> String {
        SHAPES[self.controls.choice(C_SHAPE).min(SHAPES.len() - 1)].to_string()
    }

    fn hint(&self) -> &'static str {
        "pads: notes   L/R: dial (SELECT: next)   U/D: shape   F2: pads   R1: menu"
    }
}

struct BraidsProcessor {
    handle: *mut c_void,
    controls: Arc<Controls>,
    notes: Arc<NoteQueue>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    level: Arc<AtomicF32>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    bridge: RateBridge,
    /// Last note struck (MIDI), kept so the pitch holds after release.
    note: f32,
    strike: bool,
    stereo: Vec<[f32; 2]>,
}

// The handle is only ever touched from the audio thread.
unsafe impl Send for BraidsProcessor {}

impl Drop for BraidsProcessor {
    fn drop(&mut self) {
        unsafe { braids_destroy(self.handle) };
    }
}

impl AudioProcessor for BraidsProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        let frames = buffer.len() / channels;
        while let Some(e) = self.notes.pop() {
            if e.velocity > 0 {
                self.note = e.note as f32;
                self.strike = true;
            }
        }
        self.stereo.clear();
        self.stereo.resize(frames, [0.0; 2]);
        let c = &self.controls;
        let shape = c.choice(C_SHAPE) as c_int;
        let (timbre, color) = (c.get(C_TIMBRE), c.get(C_COLOR));
        let pitch_semitones = self.note + c.raw(C_OCTAVE).round() * 12.0 + c.get(C_FM);
        let pitch = (pitch_semitones * 128.0).round().clamp(0.0, 16383.0) as c_int;
        // The firmware's 0..15 envelope amounts.
        let amount = |i: usize| (c.get(i) * 15.0).round() as c_int;
        let (attack, decay, e_timbre, e_color, e_fm) = (amount(C_ATTACK), amount(C_DECAY), amount(C_ENV_TIMBRE), amount(C_ENV_COLOR), amount(C_ENV_FM));
        let vca = c.choice(C_ENV_VCA) as c_int;
        let (bits, rate) = (c.choice(C_BITS) as c_int, c.choice(C_RATE) as c_int);
        let handle = self.handle;
        let strike = &mut self.strike;
        self.bridge.process(&[], &mut self.stereo, sample_rate, |_, o| {
            let mut block = [0.0f32; 24];
            unsafe { braids_render(handle, shape, pitch, timbre, color, *strike as c_int, attack, decay, e_timbre, e_color, e_fm, vca, bits, rate, block.as_mut_ptr()) };
            *strike = false;
            for (k, s) in block.iter().enumerate().take(o.len()) {
                o[k] = [*s, *s];
            }
        });

        let mix = (self.mix_level.get() + self.ext_mix_level.get()).clamp(0.0, 2.0);
        let mut energy = 0.0;
        if let Ok(mut bus) = self.bus_out.try_lock() {
            bus.clear();
            bus.extend(self.stereo.iter().map(|f| f[0]));
        }
        for (frame, s) in buffer.chunks_mut(channels).zip(self.stereo.iter()) {
            energy += s[0] * s[0];
            frame.fill(s[0] * mix);
        }
        self.level.set(energy / frames.max(1) as f32);
    }
}

#[allow(dead_code)] // tests name it
pub type BraidsApp = MiApp<Braids>;

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    let modbus: Arc<ModBus> = ctx.get();
    let mixer: Arc<MixerBus> = ctx.get();
    let bus: Arc<crate::audio_bus::AudioBus> = ctx.get();
    Box::new(MiApp::new(Braids::new(&modbus, &bus, &mixer), ctx.named("sensitivity"), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Input};

    fn app() -> BraidsApp {
        let modbus = ModBus::new();
        MiApp::new(Braids::new(&modbus, &crate::audio_bus::AudioBus::new(), &MixerBus::new()), Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(1.0)))
    }

    fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        let mut buf = vec![0.0f32; 512 * 2];
        for _ in 0..blocks {
            p.process(&mut buf, 2, 48_000.0);
            for v in &buf {
                assert!(v.is_finite());
            }
            all.extend(buf.iter().step_by(2));
        }
        all
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    /// Fundamental by autocorrelation (a shape's harmonics can cross zero many times).
    fn hz(x: &[f32]) -> f32 {
        let n = 4096.min(x.len() / 2);
        let (mut best, mut best_lag) = (f32::MIN, 0);
        for lag in 24..480 {
            let r: f32 = (0..n).map(|i| x[i] * x[i + lag]).sum();
            if r > best {
                best = r;
                best_lag = lag;
            }
        }
        48_000.0 / best_lag as f32
    }

    #[test]
    fn every_shape_sounds_when_struck_and_stays_bounded() {
        for shape in 0..SHAPES.len() {
            let mut a = app();
            a.m.controls.set(C_SHAPE, shape as f32);
            let mut p = a.audio_processor().unwrap();
            let before = rms(&render(&mut p, 5));
            assert!(before < 1e-4, "{}: quiet before a note ({before})", SHAPES[shape]);
            a.tick(&Input { grid: std::array::from_fn(|i| i == 12), ..Default::default() });
            let out = render(&mut p, 30);
            assert!(rms(&out) > 0.005, "{}: a pad strikes it ({})", SHAPES[shape], rms(&out));
            assert!(out.iter().all(|v| v.abs() <= 1.01), "{}: bounded", SHAPES[shape]);
        }
    }

    #[test]
    fn notes_play_at_their_pitch() {
        let mut a = app();
        a.m.controls.set(C_SHAPE, 0.0);
        a.m.controls.set(C_ATTACK, 0.0);
        a.m.controls.set(C_ENV_VCA, 0.0); // drone: no envelope to shape the level
        let mut p = a.audio_processor().unwrap();
        let mut input = Input::default();
        input.midi_keys.0[69] = 100;
        a.tick(&input);
        render(&mut p, 10);
        let out = render(&mut p, 80);
        let f = hz(&out);
        assert!((f - 440.0).abs() < 10.0, "A4 plays at 440 Hz, got {f}");
    }

    #[test]
    fn the_envelope_closes_the_vca_and_a_new_note_reopens_it() {
        let mut a = app();
        a.m.controls.set(C_DECAY, 0.1);
        let mut p = a.audio_processor().unwrap();
        a.tick(&Input { grid: std::array::from_fn(|i| i == 12), ..Default::default() });
        let hit = render(&mut p, 4);
        let tail = render(&mut p, 120);
        assert!(rms(&hit) > 0.01);
        assert!(rms(&tail[tail.len() - 4800..]) < rms(&hit) * 0.1, "the envelope has closed the VCA");
    }

    #[test]
    fn its_knobs_are_mod_inputs() {
        let modbus = ModBus::new();
        let _b = Braids::new(&modbus, &crate::audio_bus::AudioBus::new(), &MixerBus::new());
        for name in ["Braids: Timbre", "Braids: Color", "Braids: FM", "Mixer: Braids Level"] {
            assert!(modbus.index_of(name).is_some(), "{name}");
        }
    }
}
