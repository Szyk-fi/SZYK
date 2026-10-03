//! Marbles: the real Mutable Instruments Marbles random sampler
//! (vendor/eurorack/marbles, MIT, (c) Emilie Gillet), running its own C++
//! t- and x/y-generators through vendor/bridge/marbles_bridge.cc.
//!
//! Marbles makes no sound itself: it makes random rhythms (the t section's
//! three gates) and random voltages (the x section's three outputs plus
//! y), shaped by distributions you control -- BIAS, SPREAD, STEPS (which
//! quantizes to one of six scales) and DEJA VU (how much it repeats
//! itself, over a loop of LENGTH steps).
//!
//! Here the three x voltages become notes: X1 sounds on each T1 gate, X2
//! on T2, X3 on T3, sent over the note bus to whichever instrument the
//! Plays row picks. Each output can also be patched to any app's
//! modulation input. F3 starts and stops it; it opens stopped.

use super::mi_kit::{octaves, CvOuts, Controls, MiApp, Module, NoteQueue, Spec};
use crate::app::play_kit::{KitConfig, Layer, Routes};
use crate::audio::AudioProcessor;
use crate::modbus::ModBus;
use crate::note_bus::{NoteBus, NoteOut, NoteRoute};
use crate::util::AtomicF32;
use embedded_graphics::pixelcolor::Rgb565;
use std::ffi::c_void;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

unsafe extern "C" {
    fn marbles_block_size() -> c_int;
    fn marbles_sample_rate() -> f32;
    fn marbles_create(seed: u32) -> *mut c_void;
    fn marbles_destroy(h: *mut c_void);
    fn marbles_render(h: *mut c_void, knobs: *const f32, modes: *const c_int, gates: *mut c_int, cv: *mut f32);
}

const T_MODELS: [&str; 7] = ["Coin Toss", "Clusters", "Drums", "Independent", "Divider", "Three States", "Markov"];
const T_RANGES: [&str; 3] = ["1/4x", "1x", "4x"];
const X_RANGES: [&str; 3] = ["Narrow", "Positive", "Full"];
const SCALES: [&str; 6] = ["Major", "Minor", "Pentatonic", "Pelog", "Bhairav", "Shri"];
const DEJA_VU: [&str; 3] = ["Off", "On", "Locked"];
const X_MODES: [&str; 3] = ["Identical", "Bump", "Tilt"];

const C_RATE: usize = 0;
const C_T_BIAS: usize = 1;
const C_JITTER: usize = 2;
const C_DEJA_VU: usize = 3;
const C_LENGTH: usize = 4;
const C_SPREAD: usize = 5;
const C_X_BIAS: usize = 6;
const C_STEPS: usize = 7;
const KNOBS: usize = 10;
const C_T_MODEL: usize = 10;
const C_T_RANGE: usize = 11;
const C_X_RANGE: usize = 12;
const C_SCALE: usize = 13;
const C_DV_MODE: usize = 14;
const C_X_MODE: usize = 15;
const C_OCTAVE: usize = 16;

fn rate(v: f32) -> String {
    // The t-generator's internal clock: 2 Hz at noon, +-60 semitones
    // (TGenerator::Process, `one_hertz_ * SemitonesToRatio(rate_)`, at 1x).
    format!("{:.2} Hz", 2.0 * 2f32.powf((v - 0.5) * 120.0 / 12.0))
}

static SPECS: [Spec; 17] = [
    Spec::range("Rate", 0.0, 1.0, 0.5, rate),
    Spec::knob("T Bias", 0.5),
    Spec::knob("Jitter", 0.0),
    Spec::knob("Deja Vu", 0.5),
    Spec::knob("Length", 0.5),
    Spec::knob("Spread", 0.5),
    Spec::knob("X Bias", 0.5),
    Spec::knob("Steps", 0.6),
    Spec::knob("Gate Length", 0.5),
    Spec::knob("Y Divider", 0.5),
    Spec::switch("T Model", &T_MODELS, 0),
    Spec::switch("T Range", &T_RANGES, 1),
    Spec::switch("X Range", &X_RANGES, 0),
    Spec::switch("Scale", &SCALES, 0),
    Spec::switch("Deja Vu Mode", &DEJA_VU, 0),
    Spec::switch("X Mode", &X_MODES, 0),
    Spec { modulated: false, ..Spec::range("Octave", -3.0, 3.0, 0.0, octaves) },
];

const OUTS: [&str; 7] = ["T1", "T2", "T3", "X1", "X2", "X3", "Y"];

pub struct Marbles {
    controls: Arc<Controls>,
    modbus: Arc<ModBus>,
    outs: Arc<CvOuts>,
    route: NoteRoute,
    notes: Option<NoteOut>,
    running: Arc<AtomicBool>,
    /// The outputs' latest values, for the screen (0..1).
    levels: Arc<[AtomicF32; 7]>,
}

impl Marbles {
    fn new(modbus: Arc<ModBus>, bus: Option<Arc<NoteBus>>) -> Marbles {
        let (route, notes) = NoteRoute::new(bus, "Marbles", "marbles", false);
        Marbles {
            controls: Arc::new(Controls::new(&SPECS, "Marbles", &modbus)),
            modbus,
            outs: Arc::new(CvOuts::new(&OUTS)),
            route,
            notes: Some(notes),
            running: Arc::new(AtomicBool::new(false)),
            levels: Arc::new(std::array::from_fn(|_| AtomicF32::new(0.0))),
        }
    }
}

impl Module for Marbles {
    const NAME: &'static str = "Marbles";
    // Green felt and white marble.
    const PALETTE: [Rgb565; 4] = [Rgb565::new(2, 8, 4), Rgb565::new(28, 60, 28), Rgb565::new(8, 52, 20), Rgb565::new(10, 28, 14)];

    fn controls(&self) -> &Controls {
        &self.controls
    }

    fn kit_config(&self) -> KitConfig {
        KitConfig {
            app_id: "marbles",
            layers: vec![Layer::Controls, Layer::Moments],
            hero: vec![[C_RATE, C_SPREAD], [C_DEJA_VU, C_STEPS], [C_T_BIAS, C_X_BIAS], [C_JITTER, C_LENGTH]],
            browse: Some(C_T_MODEL),
            // Stick: how random the notes are (spread) and where they sit
            // (bias); hands: deja vu and jitter.
            routes: Routes { stick_x: Some(C_SPREAD), stick_y: Some(C_X_BIAS), hand_l: Some(C_DEJA_VU), hand_r: Some(C_JITTER) },
            throws: Vec::new(),
            midi_to_pads: false,
            own_expression: false,
        }
    }

    fn plays_notes(&self) -> bool {
        false
    }

    fn extra_rows(&self) -> Vec<(String, String)> {
        let mut rows = vec![("Plays".into(), self.route.label())];
        for (i, name) in OUTS.iter().enumerate() {
            rows.push((format!("{name} App"), self.outs.app_label(&self.modbus, i)));
            rows.push((format!("{name} Input"), self.outs.input_label(&self.modbus, i)));
        }
        rows
    }

    fn edit_extra(&mut self, i: usize, delta: i32) {
        match i {
            0 => self.route.step(delta.signum()),
            i => {
                let out = (i - 1) / 2;
                if (i - 1) % 2 == 0 {
                    self.outs.step_app(&self.modbus, out, delta.signum());
                } else {
                    self.outs.step_input(&self.modbus, out, delta.signum());
                }
            }
        }
    }

    fn reset_extra(&mut self, i: usize) {
        match i {
            0 => self.route.reset(),
            i => self.outs.clear((i - 1) / 2),
        }
    }

    fn processor(&mut self, _notes: Arc<NoteQueue>) -> Box<dyn AudioProcessor> {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() | 1).unwrap_or(1);
        let (rate, block) = unsafe { (marbles_sample_rate(), marbles_block_size() as usize) };
        Box::new(MarblesProcessor {
            handle: unsafe { marbles_create(seed) },
            controls: Arc::clone(&self.controls),
            modbus: Arc::clone(&self.modbus),
            outs: Arc::clone(&self.outs),
            notes: self.notes.take().unwrap_or_else(NoteOut::detached),
            running: Arc::clone(&self.running),
            levels: Arc::clone(&self.levels),
            engine_rate: rate,
            block,
            owed: 0.0,
            gates: [false; 3],
            cv: [0.0; 4],
            sounding: [None; 3],
        })
    }

    fn line(&self, _keys: &super::mi_kit::Keys) -> String {
        if !self.running.load(Ordering::Relaxed) {
            return "stopped (F3 runs)".into();
        }
        format!("plays {}", self.route.label())
    }

    fn meters(&self) -> Vec<(String, f32)> {
        OUTS.iter().zip(self.levels.iter()).map(|(n, v)| (n.to_string(), v.get())).collect()
    }

    fn needs_background_audio(&self) -> bool {
        self.running.load(Ordering::Relaxed) && (self.route.external() || self.outs.any())
    }

    fn running(&self) -> Option<bool> {
        Some(self.running.load(Ordering::Relaxed))
    }

    fn toggle_running(&mut self) {
        self.running.fetch_xor(true, Ordering::Relaxed);
    }

    fn title(&self) -> String {
        T_MODELS[self.controls.choice(C_T_MODEL).min(T_MODELS.len() - 1)].to_string()
    }

    fn hint(&self) -> &'static str {
        "F3: run   L/R: dial (SELECT: next)   U/D: t model   R1: menu (Plays, outputs)"
    }
}

struct MarblesProcessor {
    handle: *mut c_void,
    controls: Arc<Controls>,
    modbus: Arc<ModBus>,
    outs: Arc<CvOuts>,
    notes: NoteOut,
    running: Arc<AtomicBool>,
    levels: Arc<[AtomicF32; 7]>,
    engine_rate: f32,
    block: usize,
    /// Engine samples owed: the device's time not yet rendered.
    owed: f32,
    gates: [bool; 3],
    cv: [f32; 4],
    /// The note each of T1..T3 is holding.
    sounding: [Option<u8>; 3],
}

// The handle is only ever touched from the audio thread.
unsafe impl Send for MarblesProcessor {}

impl Drop for MarblesProcessor {
    fn drop(&mut self) {
        unsafe { marbles_destroy(self.handle) };
    }
}

impl MarblesProcessor {
    /// Volts to 0..1 for the screen and the mod bus, by the x range.
    fn unit(volts: f32, range: usize) -> f32 {
        match range {
            0 => volts / 2.0,
            1 => volts / 5.0,
            _ => (volts + 5.0) / 10.0,
        }
        .clamp(0.0, 1.0)
    }
}

impl AudioProcessor for MarblesProcessor {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: f32) {
        // Marbles makes no sound of its own.
        buffer.iter_mut().for_each(|s| *s = 0.0);
        let frames = buffer.len() / channels;
        if !self.running.load(Ordering::Relaxed) {
            if self.sounding.iter().any(Option::is_some) {
                self.notes.all_off();
                self.sounding = [None; 3];
            }
            self.gates = [false; 3];
            return;
        }
        let c = &self.controls;
        let knobs: [f32; KNOBS] = std::array::from_fn(|i| c.get(i));
        let modes: [c_int; 6] = [C_T_MODEL, C_T_RANGE, C_X_RANGE, C_SCALE, C_DV_MODE, C_X_MODE].map(|i| c.choice(i) as c_int);
        let octave = c.raw(C_OCTAVE).round() as f32;
        let x_range = modes[2] as usize;

        // A control-rate generator needs no resampling: run the engine
        // for the same span of time as this device block, at its own
        // rate, and act on every one of its samples (31 us apart).
        self.owed += frames as f32 * self.engine_rate / sample_rate;
        let mut g = [0 as c_int; 15];
        let mut v = [0.0f32; 20];
        while self.owed >= self.block as f32 {
            self.owed -= self.block as f32;
            unsafe { marbles_render(self.handle, knobs.as_ptr(), modes.as_ptr(), g.as_mut_ptr(), v.as_mut_ptr()) };
            for f in 0..self.block.min(5) {
                let cv = [v[f * 4], v[f * 4 + 1], v[f * 4 + 2], v[f * 4 + 3]];
                for t in 0..3 {
                    let gate = g[f * 3 + t] != 0;
                    if gate && !self.gates[t] {
                        // X1 sounds on T1, X2 on T2, X3 on T3, at 1 V/oct
                        // from C3 (Narrow, the default range, spans two
                        // octaves up from there).
                        let note = (48.0 + (cv[t] + octave) * 12.0).round().clamp(0.0, 127.0) as u8;
                        if let Some(old) = self.sounding[t].take() {
                            self.notes.note_off(old);
                        }
                        self.notes.note_on(note, 100);
                        self.sounding[t] = Some(note);
                    } else if !gate && self.gates[t] {
                        if let Some(old) = self.sounding[t].take() {
                            self.notes.note_off(old);
                        }
                    }
                    self.gates[t] = gate;
                }
                self.cv = cv;
            }
        }

        let values = [
            self.gates[0] as u8 as f32,
            self.gates[1] as u8 as f32,
            self.gates[2] as u8 as f32,
            Self::unit(self.cv[0], x_range),
            Self::unit(self.cv[1], x_range),
            Self::unit(self.cv[2], x_range),
            ((self.cv[3] + 5.0) / 10.0).clamp(0.0, 1.0),
        ];
        for (i, v) in values.iter().enumerate() {
            self.levels[i].set(*v);
            self.outs.send(&self.modbus, i, *v);
        }
    }
}

#[allow(dead_code)] // tests name it
pub type MarblesApp = MiApp<Marbles>;

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(MiApp::new(Marbles::new(ctx.get(), ctx.try_get()), ctx.named("sensitivity"), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Input};

    fn app(bus: Option<Arc<NoteBus>>) -> (MarblesApp, Arc<ModBus>) {
        let modbus = Arc::new(ModBus::new());
        (MiApp::new(Marbles::new(Arc::clone(&modbus), bus), Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(1.0))), modbus)
    }

    fn run(p: &mut Box<dyn AudioProcessor>, blocks: usize) {
        let mut buf = vec![0.0f32; 512 * 2];
        for _ in 0..blocks {
            p.process(&mut buf, 2, 48_000.0);
            assert!(buf.iter().all(|s| *s == 0.0), "no sound of its own");
        }
    }

    #[test]
    fn it_opens_stopped_and_f3_runs_it() {
        let (mut a, _) = app(None);
        assert_eq!(a.running(), Some(false), "no autoplay");
        a.toggle_running();
        assert_eq!(a.running(), Some(true));
    }

    #[test]
    fn running_it_makes_gates_and_voltages_and_plays_notes_into_an_instrument() {
        let bus = Arc::new(NoteBus::new());
        bus.declare_instrument("voltage", "Voltage");
        let (mut a, modbus) = app(Some(Arc::clone(&bus)));
        let target = modbus.register("Voltage: Filter Cutoff");
        // Plays: step to the first instrument.
        a.m.edit_extra(0, 1);
        assert_eq!(a.m.route.label(), "Voltage");
        // X1 -> Voltage's cutoff.
        for _ in 0..4 {
            if a.m.outs.app_label(&modbus, 3) != "Voltage" {
                a.m.edit_extra(7, 1);
            }
        }
        assert_eq!(a.m.outs.input_label(&modbus, 3), "Filter Cutoff");
        a.toggle_running();
        let mut p = a.audio_processor().unwrap();
        let inbox = bus.inbox(bus.instrument_index("Voltage").unwrap()).unwrap();
        let mut heard = false;
        let mut gates = 0;
        let mut buf = vec![0.0f32; 512 * 2];
        for _ in 0..400 {
            p.process(&mut buf, 2, 48_000.0);
            heard |= inbox.any_held();
            gates += a.m.levels[0].get() as i32;
        }
        assert!(gates > 0, "T1 fires");
        assert!(heard, "notes reached the instrument");
        assert!(target.get() > 0.0, "X1 reached the patched input");
        a.tick(&Input::default());
        run(&mut p, 1);
    }
}
