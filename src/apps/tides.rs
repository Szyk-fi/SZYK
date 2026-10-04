//! Tides: the real Mutable Instruments Tides (2018) tidal modulator
//! (vendor/eurorack/tides2, MIT, (c) Emilie Gillet), running its own C++
//! poly slope generator through vendor/bridge/tides_bridge.cc.
//!
//! Tides makes four related slopes at once: envelopes (AD, AR) or a
//! looping LFO/oscillator, shaped by SHAPE, SLOPE and SMOOTHNESS, with
//! SHIFT spreading the four outputs apart -- as gates, amplitudes,
//! phases, or frequency ratios depending on the output mode. Slow and
//! medium ranges are modulation; the audio range is an oscillator.
//!
//! The pads are its V/OCT and TRIG inputs: a pad opens the gate (firing
//! the AD/AR envelopes) and transposes it from C3, so in the audio range
//! they play it as a voice. Each output can be patched to any app's
//! modulation input (looping slopes swing both ways around the knob,
//! envelopes push one way), and output 1 is its audio in the audio
//! range. F3 starts and stops it; it opens stopped.

use super::mi_kit::{semitones, CvOuts, Controls, Keys, MiApp, Module, NoteQueue, Spec};
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
    fn tides_max_block() -> c_int;
    fn tides_create() -> *mut c_void;
    fn tides_destroy(h: *mut c_void);
    #[allow(clippy::too_many_arguments)]
    fn tides_render(
        h: *mut c_void,
        ramp_mode: c_int,
        output_mode: c_int,
        range: c_int,
        frequency: f32,
        slope: f32,
        shape: f32,
        smoothness: f32,
        shift: f32,
        gate: c_int,
        out: *mut f32,
        n: c_int,
    );
}

const RAMP_MODES: [&str; 3] = ["AD", "Looping", "AR"];
const OUTPUT_MODES: [&str; 4] = ["Gates", "Amplitudes", "Phases", "Ratios"];
const RANGES: [&str; 3] = ["Slow", "Medium", "Audio"];
/// Each range's root frequency, as tides2/tides.cc's kRoot.
const ROOT_HZ: [f32; 3] = [0.125, 2.0, 130.81];

const C_FREQUENCY: usize = 0;
const C_SHAPE: usize = 1;
const C_SLOPE: usize = 2;
const C_SMOOTHNESS: usize = 3;
const C_SHIFT: usize = 4;
const C_RAMP: usize = 5;
const C_OUTPUT: usize = 6;
const C_RANGE: usize = 7;

static SPECS: [Spec; 8] = [
    // The FREQUENCY knob's own span (tides2/cv_reader.cc: 96 * pot - 48).
    Spec::range("Frequency", -48.0, 48.0, 0.0, semitones),
    Spec::knob("Shape", 0.5),
    Spec::knob("Slope", 0.5),
    Spec::knob("Smoothness", 0.5),
    Spec::knob("Shift", 0.6),
    Spec::switch("Ramp", &RAMP_MODES, 1),
    Spec::switch("Outputs", &OUTPUT_MODES, 2),
    Spec::switch("Range", &RANGES, 1),
];

const OUTS: [&str; 4] = ["Out 1", "Out 2", "Out 3", "Out 4"];

pub struct Tides {
    controls: Arc<Controls>,
    modbus: Arc<ModBus>,
    outs: Arc<CvOuts>,
    running: Arc<AtomicBool>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    levels: Arc<[AtomicF32; 4]>,
}

impl Tides {
    fn new(modbus: Arc<ModBus>, audio_bus: &AudioBus, mixer_bus: &MixerBus) -> Tides {
        let (mix_level, ext_mix_level) = mixer_bus.register("Tides", &modbus);
        Tides {
            controls: Arc::new(Controls::new(&SPECS, "Tides", &modbus)),
            outs: Arc::new(CvOuts::new(&OUTS)),
            modbus,
            running: Arc::new(AtomicBool::new(false)),
            bus_out: audio_bus.register("Tides"),
            mix_level,
            ext_mix_level,
            levels: Arc::new(std::array::from_fn(|_| AtomicF32::new(0.0))),
        }
    }
}

impl Module for Tides {
    const NAME: &'static str = "Tides";
    // Sea blue and foam.
    const PALETTE: [Rgb565; 4] = [Rgb565::new(1, 6, 9), Rgb565::new(24, 58, 30), Rgb565::new(6, 46, 28), Rgb565::new(8, 26, 18)];

    fn controls(&self) -> &Controls {
        &self.controls
    }

    fn kit_config(&self) -> KitConfig {
        KitConfig {
            app_id: "tides",
            layers: vec![Layer::Native(0, "KEYS"), Layer::Controls, Layer::Moments],
            hero: vec![[C_FREQUENCY, C_SHAPE], [C_SLOPE, C_SMOOTHNESS], [C_SHIFT, C_RAMP], [C_OUTPUT, C_RANGE]],
            browse: Some(C_RAMP),
            routes: Routes { stick_x: Some(C_SHAPE), stick_y: Some(C_SLOPE), hand_l: Some(C_SMOOTHNESS), hand_r: Some(C_SHIFT) },
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
        Box::new(TidesProcessor {
            handle: unsafe { tides_create() },
            controls: Arc::clone(&self.controls),
            modbus: Arc::clone(&self.modbus),
            outs: Arc::clone(&self.outs),
            notes,
            running: Arc::clone(&self.running),
            bus_out: Arc::clone(&self.bus_out),
            mix_level: Arc::clone(&self.mix_level),
            ext_mix_level: Arc::clone(&self.ext_mix_level),
            levels: Arc::clone(&self.levels),
            held: Vec::with_capacity(16),
            note: 48,
            block: (unsafe { tides_max_block() } as usize).min(64),
            mono: Vec::new(),
        })
    }

    fn line(&self, keys: &Keys) -> String {
        if !self.running.load(Ordering::Relaxed) {
            return "stopped (F3 runs)".into();
        }
        match keys.held().last() {
            Some(&n) => crate::util::note_name(n as i32),
            None => "running".into(),
        }
    }

    fn meters(&self) -> Vec<(String, f32)> {
        OUTS.iter().zip(self.levels.iter()).map(|(n, v)| (n.to_string(), v.get())).collect()
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
        format!("{} · {}", RAMP_MODES[self.controls.choice(C_RAMP).min(2)], RANGES[self.controls.choice(C_RANGE).min(2)])
    }

    fn hint(&self) -> &'static str {
        "F3: run   pads: trig + pitch   L/R: dial   U/D: ramp mode   R1: menu (outputs)"
    }
}

struct TidesProcessor {
    handle: *mut c_void,
    controls: Arc<Controls>,
    modbus: Arc<ModBus>,
    outs: Arc<CvOuts>,
    notes: Arc<NoteQueue>,
    running: Arc<AtomicBool>,
    bus_out: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    levels: Arc<[AtomicF32; 4]>,
    held: Vec<u8>,
    note: u8,
    block: usize,
    mono: Vec<f32>,
}

// The handle is only ever touched from the audio thread.
unsafe impl Send for TidesProcessor {}

impl Drop for TidesProcessor {
    fn drop(&mut self) {
        unsafe { tides_destroy(self.handle) };
    }
}

impl AudioProcessor for TidesProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        buffer.iter_mut().for_each(|s| *s = 0.0);
        let frames = buffer.len() / channels;
        while let Some(e) = self.notes.pop() {
            self.held.retain(|&n| n != e.note);
            if e.velocity > 0 && self.held.len() < 16 {
                self.held.push(e.note);
            }
        }
        if let Some(&n) = self.held.last() {
            self.note = n;
        }
        if !self.running.load(Ordering::Relaxed) {
            if let Ok(mut bus) = self.bus_out.try_lock() {
                bus.clear();
                bus.resize(frames, 0.0);
            }
            return;
        }

        let c = &self.controls;
        let ramp = c.choice(C_RAMP) as c_int;
        let output = c.choice(C_OUTPUT) as c_int;
        let range = c.choice(C_RANGE).min(2);
        // V/OCT from the pads: the held note transposes from C3.
        let transpose = c.get(C_FREQUENCY) + (self.note as f32 - 48.0);
        let frequency = ROOT_HZ[range] * 2f32.powf(transpose / 12.0) / sample_rate;
        let gate = !self.held.is_empty();
        let looping = ramp == 1;

        self.mono.clear();
        self.mono.resize(frames, 0.0);
        let mut out = [0.0f32; 64 * 4];
        let mut last = [0.0f32; 4];
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(self.block);
            unsafe {
                tides_render(self.handle, ramp, output, (range == 2) as c_int, frequency, c.get(C_SLOPE), c.get(C_SHAPE), c.get(C_SMOOTHNESS), c.get(C_SHIFT), gate as c_int, out.as_mut_ptr(), n as c_int);
            }
            for k in 0..n {
                // The firmware's DACs swing +-5 V when looping, 0..8 V
                // otherwise (PolySlopeGenerator::Scale).
                self.mono[done + k] = if looping { out[k * 4] / 5.0 } else { out[k * 4] / 8.0 };
            }
            last.copy_from_slice(&out[(n - 1) * 4..n * 4]);
            done += n;
        }

        for i in 0..4 {
            let v = if looping { last[i] / 10.0 } else { last[i] / 8.0 };
            self.levels[i].set(if looping { v * 2.0 } else { v });
            self.outs.send(&self.modbus, i, v);
        }

        let audio = range == 2;
        if let Ok(mut bus) = self.bus_out.try_lock() {
            bus.clear();
            bus.extend(self.mono.iter().map(|s| if audio { *s } else { 0.0 }));
        }
        if audio {
            let mix = (self.mix_level.get() + self.ext_mix_level.get()).clamp(0.0, 2.0) * 0.5;
            for (frame, s) in buffer.chunks_mut(channels).zip(self.mono.iter()) {
                for o in frame.iter_mut() {
                    *o = s * mix;
                }
            }
        }
    }
}

#[allow(dead_code)] // tests name it
pub type TidesApp = MiApp<Tides>;

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    let audio_bus: Arc<AudioBus> = ctx.get();
    let mixer: Arc<MixerBus> = ctx.get();
    Box::new(MiApp::new(Tides::new(ctx.get(), &audio_bus, &mixer), ctx.named("sensitivity"), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Input};

    fn app() -> (TidesApp, Arc<ModBus>) {
        let modbus = Arc::new(ModBus::new());
        (MiApp::new(Tides::new(Arc::clone(&modbus), &AudioBus::new(), &MixerBus::new()), Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(1.0))), modbus)
    }

    #[test]
    fn it_opens_stopped_and_silent() {
        let (mut a, _) = app();
        assert_eq!(a.running(), Some(false));
        a.m.controls.set(C_RANGE, 2.0);
        let mut p = a.audio_processor().unwrap();
        let mut buf = vec![0.0f32; 1024];
        p.process(&mut buf, 2, 48_000.0);
        assert!(buf.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn the_lfo_sweeps_a_patched_input_both_ways() {
        let (mut a, modbus) = app();
        let target = modbus.register("Voltage: Filter Cutoff");
        for _ in 0..4 {
            if a.m.outs.app_label(&modbus, 0) != "Voltage" {
                a.m.edit_extra(0, 1);
            }
        }
        a.m.controls.set(C_FREQUENCY, 24.0); // 8 Hz in the medium range
        a.toggle_running();
        let mut p = a.audio_processor().unwrap();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut buf = vec![0.0f32; 256 * 2];
        for _ in 0..200 {
            p.process(&mut buf, 2, 48_000.0);
            lo = lo.min(target.get());
            hi = hi.max(target.get());
        }
        assert!(lo < -0.2 && hi > 0.2, "swings around the knob: {lo}..{hi}");
    }

    #[test]
    fn in_the_audio_range_a_pad_plays_its_pitch() {
        let (mut a, _) = app();
        a.m.controls.set(C_RANGE, 2.0);
        a.toggle_running();
        let mut p = a.audio_processor().unwrap();
        // The pad an octave up from the bottom-left (rank 12 = C4).
        let pad = crate::app::play_kit::rank_pad(12);
        a.tick(&Input { grid: std::array::from_fn(|i| i == pad), ..Default::default() });
        let mut buf = vec![0.0f32; 4800 * 2];
        p.process(&mut buf, 2, 48_000.0);
        p.process(&mut buf, 2, 48_000.0);
        let crossings = buf.chunks(2).collect::<Vec<_>>().windows(2).filter(|w| w[0][0] < 0.0 && w[1][0] >= 0.0).count();
        // C4 is 261.6 Hz: about 26 cycles in 0.1 s.
        assert!((23..=29).contains(&crossings), "{crossings} cycles");
    }
}
