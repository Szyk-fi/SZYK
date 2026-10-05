//! O&C: the Ornaments & Crimes firmware, running as the module's own code.
//!
//! Ornaments & Crimes is open firmware for a four-channel eurorack CV module
//! (a Teensy 3.2, four CV inputs, four trigger inputs, a four-channel 16-bit
//! DAC, a 128x64 OLED, two encoders with buttons and two more buttons). Its
//! thirteen apps -- quantizers, an analogue shift register, a sequencer,
//! envelope generators, LFOs, chord and Tonnetz sequencers, bouncing balls,
//! bytebeats, a Lorenz attractor -- are all in this one firmware, and this
//! runs it: vendor/o_c compiles the firmware's own source (a handful of files
//! patched, listed in vendor/o_c/PATCHES.md) against a host stand-in for the
//! Teensy and the module.
//!
//! The firmware's setup() and loop() run on a thread of their own, blocking
//! splash and settings screens included. Its two timer interrupts run on the
//! audio thread, at the 16.666 kHz and 1 kHz it asks for, so CV, triggers and
//! the quantizers keep sample time. Whatever the OLED shows is shown here, four
//! times larger.
//!
//! The panel, on the device's controls:
//!
//! - The **D-pad** is the two encoders: up/down turn the left one (move through
//!   the menu), left/right turn the right one (change the value); hold left or
//!   right to turn it faster.
//! - **SELECT** is the right encoder's button; holding it presses it long,
//!   which is how the firmware opens its app list.
//! - The **pads**, on the PANEL layer: the top row is the module's four
//!   buttons (UP, DOWN, and the left and right encoder buttons -- hold one for
//!   a second for a long press); the second row is its four trigger inputs,
//!   high while held. F2 turns to the Controls pads.
//! - **CV 1-4** take a patched mod-bus input ("O&C: CV 1"...) plus a knob of
//!   their own, so the stick and hands can play them. A trigger input also goes
//!   high from a mod-bus value ("O&C: TR 1"...) above one half.
//! - The four outputs can each be patched to any app's mod input, and a chosen
//!   output can play notes on another app, struck when a chosen trigger rises.
//!
//! The firmware's settings are kept in its EEPROM, saved in `saves/oc/`.
//!
//! Four modules run at once, O&C 1-4 (the apps `oc`, `oc2`, `oc3`, `oc4`). The
//! firmware keeps its state in globals, so build.rs compiles it four times over,
//! each in its own namespace, and each instance has its own screen, settings,
//! inputs ("O&C 2: CV 1"...) and outputs. A second app for the same instance is
//! silent.

use super::kids_kit;
use super::mi_kit::CvOuts;
use crate::{
    app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input, SlintExtra},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    note_bus::{NoteBus, NoteOut, NoteRoute},
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12},
    util::AtomicF32,
};
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}, text::Text};
use std::ffi::c_int;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};

/// How many modules can run at once (build.rs compiles the firmware this many times).
pub const INSTANCES: usize = 4;
const APP_NAMES: [&str; INSTANCES] = ["O&C", "O&C 2", "O&C 3", "O&C 4"];
const APP_IDS: [&str; INSTANCES] = ["oc", "oc2", "oc3", "oc4"];

#[path = "oc_firmware.rs"]
mod firmware;
use firmware::{Firmware, VARIANTS};

const EEPROM_SIZE: usize = 8192;

// Own palette: black glass, a cool white OLED.
const BG: Rgb565 = Rgb565::new(1, 2, 3);
const OLED: Rgb565 = Rgb565::new(24, 56, 31);
const OLED_OFF: Rgb565 = Rgb565::new(1, 3, 4);
const INK: Rgb565 = Rgb565::new(20, 44, 28);
const ACCENT: Rgb565 = Rgb565::new(31, 44, 4);
const DIM: Rgb565 = Rgb565::new(7, 16, 14);
const FAINT: Rgb565 = Rgb565::new(3, 7, 7);

/// The module's pins (OC_gpio.h, the non-flipped layout).
const PIN_TR: [c_int; 4] = [0, 1, 2, 3];
const PIN_BUTTON: [c_int; 4] = [5, 4, 23, 14]; // UP, DOWN, left encoder, right encoder
const PIN_ENC_RIGHT: [c_int; 2] = [16, 15];
const PIN_ENC_LEFT: [c_int; 2] = [22, 21];

const BUTTON_NAMES: [&str; 4] = ["UP", "DOWN", "L", "R"];
const NOTE_CHANNELS: [&str; 5] = ["Off", "A", "B", "C", "D"];
const NOTE_GATES: [&str; 5] = ["Off", "TR 1", "TR 2", "TR 3", "TR 4"];
const OUTS: [&str; 4] = ["A", "B", "C", "D"];

// Controls: the four CV knobs, then the notes setup.
const C_NOTE_CH: usize = 4;
const C_NOTE_GATE: usize = 5;
const C_PLAYS: usize = 6;
const N_CONTROLS: usize = 7;

fn kit_config(instance: usize) -> KitConfig {
    KitConfig {
        app_id: APP_IDS[instance],
        layers: vec![Layer::Native(0, "PANEL"), Layer::Controls, Layer::Moments],
        // The D-pad is the firmware's encoders, so there are no dials for it.
        hero: Vec::new(),
        browse: None,
        routes: Routes { stick_x: Some(0), stick_y: Some(1), hand_l: Some(2), hand_r: Some(3) },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: false,
    }
}

/// What the UI hands the audio thread, and what it hands back.
struct Shared {
    /// Detents waiting to be turned, per encoder (left, right), signed.
    detents: [AtomicI32; 2],
    /// Button pads held right now (UP, DOWN, L, R).
    held: [AtomicBool; 4],
    /// Extra milliseconds a button is held low for (a long press), per button.
    pulse_ms: [AtomicU32; 4],
    /// Trigger input pads held.
    gate_pads: [AtomicBool; 4],
    /// The same, from the keyboard while the module has a window of its own.
    key_held: [AtomicBool; 4],
    key_gates: [AtomicBool; 4],
    /// Whether the module's own window is open.
    window: AtomicBool,
    /// Manual CV knobs, volts (-5..5), and the mod-bus inputs that add to them.
    cv_knob: [AtomicF32; 4],
    cv_mod: [Arc<AtomicF32>; 4],
    gate_mod: [Arc<AtomicF32>; 4],
    note_channel: AtomicU32,
    note_gate: AtomicU32,
    /// Outputs in millivolts, as the DAC sets them.
    out_mv: [AtomicI32; 4],
    /// Inputs and gates as patched this millisecond (display).
    in_mv: [AtomicI32; 4],
    gates: [AtomicBool; 4],
    frames: AtomicU64,
    /// Whether this app's audio thread is the one driving the firmware.
    driving: AtomicBool,
    /// The running firmware, if any; both threads clone the Arc out and call it
    /// without holding the lock.
    fw: Mutex<Option<Arc<Firmware>>>,
    /// The chosen firmware, an index into `VARIANTS`.
    variant: AtomicUsize,
}

/// One app per firmware instance.
static FIRMWARE_CLAIMED: [AtomicBool; INSTANCES] = [AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false)];

pub struct OcApp {
    instance: usize,
    p: Arc<Shared>,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
    outs: Arc<CvOuts>,
    modbus: Arc<ModBus>,
    note_route: NoteRoute,
    note_out: Option<NoteOut>,
    frame: [u8; 1024],
    persist: bool,
    eeprom_saved: Vec<u8>,
    last_save: std::time::Instant,
    /// Firmwares taken off the audio thread, to be unloaded once nothing holds them.
    retiring: Vec<Arc<Firmware>>,
    /// What went wrong loading a firmware, for the screen.
    status: String,
    /// When this module's current firmware started (tests wait out its splash screen).
    started_at: Option<std::time::Instant>,
}

fn saves_dir() -> PathBuf {
    let root = std::env::var_os("PORTAMAX_SAVES_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves")));
    root.join("oc")
}

/// Which firmware module `instance` runs, remembered between sessions.
fn variant_file(instance: usize) -> PathBuf {
    saves_dir().join(if instance == 0 { "firmware.txt".to_string() } else { format!("firmware{}.txt", instance + 1) })
}

/// A firmware's settings: each variant keeps its own (their layouts differ). The
/// stock firmware's keep the names they have always had.
fn eeprom_file(instance: usize, variant: &str) -> PathBuf {
    let n = if instance == 0 { String::new() } else { (instance + 1).to_string() };
    saves_dir().join(if variant == "stock" { format!("eeprom{n}.bin") } else { format!("eeprom_{variant}{n}.bin") })
}

impl OcApp {
    /// Module `instance` (0..4): each is its own firmware, with its own settings,
    /// screen, inputs and outputs.
    pub fn new(instance: usize, sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, _bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        Self::build(instance, sensitivity, nav, mods, mixer, !cfg!(test))
    }

    fn build(instance: usize, sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, mixer: Arc<MixerBus>, persist: bool) -> Self {
        let instance = instance.min(INSTANCES - 1);
        let name = APP_NAMES[instance];
        // The module has no audio of its own, but its level is registered like
        // everyone's so the mixer can list it.
        let _ = mixer.register(name, &mods);
        let (route, out) = NoteRoute::new(None, name, APP_IDS[instance], false);
        let available = firmware::available();
        let remembered = persist.then(|| std::fs::read_to_string(variant_file(instance)).ok()).flatten().and_then(|t| VARIANTS.iter().position(|v| v.id == t.trim()));
        let variant = remembered.filter(|v| available.contains(v)).or_else(|| available.first().copied()).unwrap_or(0);
        let p = Arc::new(Shared {
            detents: [AtomicI32::new(0), AtomicI32::new(0)],
            held: std::array::from_fn(|_| AtomicBool::new(false)),
            pulse_ms: std::array::from_fn(|_| AtomicU32::new(0)),
            gate_pads: std::array::from_fn(|_| AtomicBool::new(false)),
            key_held: std::array::from_fn(|_| AtomicBool::new(false)),
            key_gates: std::array::from_fn(|_| AtomicBool::new(false)),
            window: AtomicBool::new(false),
            cv_knob: std::array::from_fn(|_| AtomicF32::new(0.0)),
            cv_mod: std::array::from_fn(|i| mods.register(format!("{name}: CV {}", i + 1))),
            gate_mod: std::array::from_fn(|i| mods.register(format!("{name}: TR {}", i + 1))),
            note_channel: AtomicU32::new(0),
            note_gate: AtomicU32::new(0),
            out_mv: std::array::from_fn(|_| AtomicI32::new(0)),
            in_mv: std::array::from_fn(|_| AtomicI32::new(0)),
            gates: std::array::from_fn(|_| AtomicBool::new(false)),
            frames: AtomicU64::new(0),
            driving: AtomicBool::new(false),
            fw: Mutex::new(None),
            variant: AtomicUsize::new(variant),
        });
        Self {
            instance,
            p,
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(instance), persist),
            outs: Arc::new(CvOuts::new(&OUTS)),
            modbus: mods,
            note_route: route,
            note_out: Some(out),
            frame: [0; 1024],
            persist,
            eeprom_saved: Vec::new(),
            last_save: std::time::Instant::now(),
            retiring: Vec::new(),
            status: String::new(),
            started_at: None,
        }
    }

    /// Lets the module play other apps (see note_bus.rs).
    pub fn with_notes(mut self, bus: Option<Arc<NoteBus>>) -> Self {
        let (route, out) = NoteRoute::new(bus, APP_NAMES[self.instance], APP_IDS[self.instance], false);
        self.note_route = route;
        self.note_out = Some(out);
        self
    }

    /// The running firmware, if any.
    fn fw(&self) -> Option<Arc<Firmware>> {
        self.p.fw.lock().ok().and_then(|g| g.clone())
    }

    /// The name of the firmware app on screen ("CopierMaschine"...).
    pub fn firmware_app(&self) -> String {
        self.fw().map(|f| f.app_name()).unwrap_or_default()
    }

    pub fn variant_id(&self) -> &'static str {
        VARIANTS[self.p.variant.load(Ordering::Relaxed).min(VARIANTS.len() - 1)].id
    }

    /// Loads the chosen firmware, hands it its saved settings and starts it.
    fn ensure_firmware(&mut self) {
        if self.fw().is_some() {
            return;
        }
        let id = self.variant_id();
        match Firmware::load(id) {
            Ok(fw) => {
                let path = eeprom_file(self.instance, id);
                self.eeprom_saved = Vec::new();
                if self.persist {
                    if let Some(d) = std::fs::read(&path).ok().filter(|d| d.len() == EEPROM_SIZE) {
                        // The firmware reads its settings as it boots, so they go in first.
                        fw.eeprom_write(&d);
                        self.eeprom_saved = d;
                    }
                }
                fw.start();
                self.started_at = Some(std::time::Instant::now());
                self.status.clear();
                if let Ok(mut slot) = self.p.fw.lock() {
                    *slot = Some(Arc::new(fw));
                }
            }
            Err(e) => self.status = e,
        }
    }

    /// Switches to firmware `index` (into `VARIANTS`): the old one is retired and
    /// the new one boots, as if the module had been reflashed.
    fn set_variant(&mut self, index: usize) {
        let available = firmware::available();
        if !available.contains(&index) || index == self.p.variant.load(Ordering::Relaxed) {
            return;
        }
        self.p.variant.store(index, Ordering::Relaxed);
        if self.persist {
            std::fs::create_dir_all(saves_dir()).ok();
            std::fs::write(variant_file(self.instance), VARIANTS[index].id).ok();
        }
        if let Some(old) = self.p.fw.lock().ok().and_then(|mut g| g.take()) {
            self.retiring.push(old);
        }
        if self.p.driving.load(Ordering::Relaxed) {
            self.ensure_firmware();
        }
    }

    fn step_variant(&mut self, delta: i32) {
        let available = firmware::available();
        if available.len() < 2 || delta == 0 {
            return;
        }
        let pos = available.iter().position(|&v| v == self.p.variant.load(Ordering::Relaxed)).unwrap_or(0) as i32;
        let next = available[(pos + delta.signum()).rem_euclid(available.len() as i32) as usize];
        self.set_variant(next);
    }

    fn text(&self, i: usize) -> (String, String) {
        match i {
            i if i < 4 => (format!("CV {}", i + 1), format!("{:+.2} V", self.p.cv_knob[i].get())),
            C_NOTE_CH => (
                "Notes from".into(),
                match self.p.note_channel.load(Ordering::Relaxed) as usize {
                    0 => "off".into(),
                    c => format!("output {}", NOTE_CHANNELS[c.min(4)]),
                },
            ),
            C_NOTE_GATE => ("Struck by".into(), NOTE_GATES[(self.p.note_gate.load(Ordering::Relaxed) as usize).min(4)].into()),
            _ => ("Plays".into(), self.note_route.label()),
        }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        let mut r: Vec<(String, String, bool)> = (0..N_CONTROLS)
            .map(|i| {
                let (n, v) = self.text(i);
                (n, v, false)
            })
            .collect();
        r.push(("Firmware".into(), VARIANTS[self.p.variant.load(Ordering::Relaxed).min(VARIANTS.len() - 1)].name.into(), false));
        r.push(("Window".into(), if self.p.window.load(Ordering::Relaxed) { "open" } else { "closed" }.into(), false));
        for (i, name) in OUTS.iter().enumerate() {
            r.push((format!("Out {name} App"), self.outs.app_label(&self.modbus, i), false));
            r.push((format!("Out {name} Input"), self.outs.input_label(&self.modbus, i), false));
        }
        r
    }

    fn rows_len(&self) -> usize {
        N_CONTROLS + 2 + OUTS.len() * 2
    }

    fn edit_row(&mut self, row: usize, delta: i32) {
        if delta == 0 {
            return;
        }
        if row < N_CONTROLS {
            self.kit_edit(row, delta);
        } else if row == N_CONTROLS {
            self.step_variant(delta);
        } else if row == N_CONTROLS + 1 {
            self.p.window.store(delta > 0, Ordering::Relaxed);
        } else {
            let k = row - N_CONTROLS - 2;
            if k % 2 == 0 {
                self.outs.step_app(&self.modbus, k / 2, delta.signum());
            } else {
                self.outs.step_input(&self.modbus, k / 2, delta.signum());
            }
        }
    }

    /// Persists the firmware's EEPROM when it has changed (a settings save).
    fn save_eeprom(&mut self) {
        if !self.persist || self.last_save.elapsed().as_secs_f32() < 2.0 {
            return;
        }
        let Some(fw) = self.fw() else { return };
        self.last_save = std::time::Instant::now();
        let mut now = vec![0u8; EEPROM_SIZE];
        fw.eeprom_read(&mut now);
        if now != self.eeprom_saved {
            let path = eeprom_file(self.instance, self.variant_id());
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).ok();
            }
            if std::fs::write(&path, &now).is_ok() {
                self.eeprom_saved = now;
            }
        }
    }

    fn release_all(&mut self) {
        for b in 0..4 {
            self.p.held[b].store(false, Ordering::Relaxed);
            self.p.gate_pads[b].store(false, Ordering::Relaxed);
        }
    }

    fn draw_oled(&mut self, f: &mut FrameBuffer) {
        if let Some(fw) = self.fw() {
            fw.frame(&mut self.frame);
        }
        // The 128x64 panel at 4x, centred; its bytes are 8 pages of 128 columns,
        // bit 0 the top row of a page.
        let (x0, y0, k) = (64, 8, 4);
        Rectangle::new(Point::new(x0 - 4, y0 - 4), Size::new(128 * k as u32 + 8, 64 * k as u32 + 8)).into_styled(PrimitiveStyle::with_fill(OLED_OFF)).draw(f).ok();
        for y in 0..64usize {
            let (page, bit) = (y / 8, y % 8);
            let mut x = 0;
            while x < 128 {
                if self.frame[page * 128 + x] >> bit & 1 == 0 {
                    x += 1;
                    continue;
                }
                // a run of lit pixels is one rectangle
                let start = x;
                while x < 128 && self.frame[page * 128 + x] >> bit & 1 == 1 {
                    x += 1;
                }
                Rectangle::new(Point::new(x0 + start as i32 * k, y0 + y as i32 * k), Size::new(((x - start) as i32 * k) as u32, k as u32)).into_styled(PrimitiveStyle::with_fill(OLED)).draw(f).ok();
            }
        }
    }

    fn draw_status(&self, f: &mut FrameBuffer) {
        let ink = MonoTextStyle::new(&SPLEEN_6X12, INK);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        let acc = MonoTextStyle::new(&SPLEEN_6X12, ACCENT);
        let y = 282;
        // Inputs and outputs, in volts, with the trigger lamps.
        for i in 0..4 {
            let x = 16 + i as i32 * 150;
            let vin = self.p.in_mv[i].load(Ordering::Relaxed) as f32 / 1000.0;
            let vout = self.p.out_mv[i].load(Ordering::Relaxed) as f32 / 1000.0;
            Text::new(&format!("CV{} {:+.2}V", i + 1, vin), Point::new(x, y), ink).draw(f).ok();
            Text::new(&format!("OUT {} {:+.2}V", OUTS[i], vout), Point::new(x, y + 14), acc).draw(f).ok();
            let lit = self.p.gates[i].load(Ordering::Relaxed);
            Rectangle::new(Point::new(x, y + 22), Size::new(12, 6)).into_styled(PrimitiveStyle::with_fill(if lit { ACCENT } else { FAINT })).draw(f).ok();
            Text::new(&format!("TR{}", i + 1), Point::new(x + 18, y + 28), dim).draw(f).ok();
        }
        let app = self.firmware_app();
        Text::new(if app.is_empty() { "starting..." } else { app.as_str() }, Point::new(16, 344), ink).draw(f).ok();
        Text::new("D-pad: encoders  SELECT: R button  pads: buttons, triggers", Point::new(180, 344), dim).draw(f).ok();
    }
}

impl PlayHost for OcApp {
    fn kit_control_count(&self) -> usize {
        N_CONTROLS
    }
    fn kit_label(&self, i: usize) -> String {
        self.text(i.min(N_CONTROLS - 1)).0
    }
    fn kit_value(&self, i: usize) -> String {
        self.text(i.min(N_CONTROLS - 1)).1
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        Some(match i {
            i if i < 4 => (self.p.cv_knob[i].get() + 5.0) / 10.0,
            C_NOTE_CH => self.p.note_channel.load(Ordering::Relaxed) as f32 / 4.0,
            C_NOTE_GATE => self.p.note_gate.load(Ordering::Relaxed) as f32 / 4.0,
            _ => return None,
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        i >= C_NOTE_CH
    }
    fn kit_pads_play(&self, _layer: u8) -> bool {
        false
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match i {
            i if i < 4 => {
                let step = delta as f32 * 0.05 * self.sensitivity.get().max(0.01) * 10.0;
                self.p.cv_knob[i].set((self.p.cv_knob[i].get() + step).clamp(-5.0, 5.0));
            }
            C_NOTE_CH => self.p.note_channel.store((self.p.note_channel.load(Ordering::Relaxed) as i32 + delta.signum()).rem_euclid(5) as u32, Ordering::Relaxed),
            C_NOTE_GATE => self.p.note_gate.store((self.p.note_gate.load(Ordering::Relaxed) as i32 + delta.signum()).rem_euclid(5) as u32, Ordering::Relaxed),
            _ => {
                if delta != 0 {
                    self.note_route.step(delta);
                }
            }
        }
    }
    fn kit_reset(&mut self, i: usize) {
        match i {
            i if i < 4 => self.p.cv_knob[i].set(0.0),
            C_NOTE_CH => self.p.note_channel.store(0, Ordering::Relaxed),
            C_NOTE_GATE => self.p.note_gate.store(0, Ordering::Relaxed),
            C_PLAYS => self.note_route.reset(),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i {
            i if i < 4 => self.p.cv_knob[i].set(v * 10.0 - 5.0),
            C_NOTE_CH => self.p.note_channel.store((v * 4.0).round() as u32, Ordering::Relaxed),
            C_NOTE_GATE => self.p.note_gate.store((v * 4.0).round() as u32, Ordering::Relaxed),
            _ => {}
        }
    }
    fn kit_line(&self) -> String {
        self.firmware_app()
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        match pad {
            0..=3 => BUTTON_NAMES[pad].to_string(),
            4..=7 => format!("TR{}", pad - 3),
            _ => String::new(),
        }
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, held: bool) -> crate::led_output::PadColor {
        use crate::led_output::PadColor;
        match pad {
            0..=3 if held => PadColor::Green,
            0..=3 => PadColor::Blue,
            4..=7 if self.p.gates[pad - 4].load(Ordering::Relaxed) => PadColor::Red,
            4..=7 => PadColor::Yellow,
            _ => PadColor::Off,
        }
    }
}

impl App for OcApp {
    fn play_surface(&self) -> bool {
        true
    }
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        None
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
    fn needs_background_audio(&self) -> bool {
        true
    }
    fn popout(&mut self) -> Option<(String, crate::app::ScreenExtra)> {
        if !self.p.window.load(Ordering::Relaxed) {
            return None;
        }
        let mut fb = FrameBuffer::new();
        fb.clear(BG).ok();
        self.draw_oled(&mut fb);
        self.draw_status(&mut fb);
        match kids_kit::screen_extra(&fb) {
            SlintExtra::Screen(s) => Some((format!("{} - {}", APP_NAMES[self.instance], self.firmware_app()), s)),
            _ => None,
        }
    }
    fn popout_key(&mut self, key: &str, pressed: bool) {
        // Arrows are the encoders (a detent per press, and the keyboard's own
        // repeat turns it faster), Return and R the right button, U/D/L the other
        // three buttons, 1-4 the trigger inputs, held while the key is.
        let add = |a: &AtomicI32, n: i32| {
            let cur = a.load(Ordering::Relaxed);
            a.store((cur + n).clamp(-64, 64), Ordering::Relaxed);
        };
        match key {
            "\u{f700}" if pressed => add(&self.p.detents[0], -1),
            "\u{f701}" if pressed => add(&self.p.detents[0], 1),
            "\u{f702}" if pressed => add(&self.p.detents[1], -1),
            "\u{f703}" if pressed => add(&self.p.detents[1], 1),
            "u" | "U" => self.p.key_held[0].store(pressed, Ordering::Relaxed),
            "d" | "D" => self.p.key_held[1].store(pressed, Ordering::Relaxed),
            "l" | "L" => self.p.key_held[2].store(pressed, Ordering::Relaxed),
            "r" | "R" | "\n" => self.p.key_held[3].store(pressed, Ordering::Relaxed),
            "1" | "2" | "3" | "4" => self.p.key_gates[key.parse::<usize>().unwrap_or(1) - 1].store(pressed, Ordering::Relaxed),
            _ => {}
        }
    }
    fn popout_closed(&mut self) {
        self.p.window.store(false, Ordering::Relaxed);
        for i in 0..4 {
            self.p.key_held[i].store(false, Ordering::Relaxed);
            self.p.key_gates[i].store(false, Ordering::Relaxed);
        }
    }
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        self.save_eeprom();
        // A retired firmware is unloaded once the audio thread has let go of it.
        self.retiring.retain(|f| Arc::strong_count(f) > 1);
        if step.menu {
            let i = &step.input;
            let n = self.rows_len();
            self.list.navigate_input(i, n, self.nav.get() as i32);
            let sel = self.list.selected.min(n - 1);
            self.edit_row(sel, i.knob2);
            if i.knob2_press {
                self.kit_reset(sel.min(N_CONTROLS - 1));
            }
            self.release_all();
            return;
        }
        if step.native.is_none() {
            self.release_all();
            return;
        }
        // The raw D-pad, since the kit would turn a dial with it: down turns the
        // left encoder clockwise, right turns the right one.
        let add = |a: &AtomicI32, n: i32| {
            let cur = a.load(Ordering::Relaxed);
            a.store((cur + n).clamp(-64, 64), Ordering::Relaxed);
        };
        add(&self.p.detents[0], input.navigation_steps);
        add(&self.p.detents[1], input.nav_x);
        // SELECT: a tap presses the right encoder, a hold presses it long.
        if input.knob1_press {
            self.p.pulse_ms[3].store(40, Ordering::Relaxed);
        }
        if input.knob2_press {
            self.p.pulse_ms[3].store(1100, Ordering::Relaxed);
        }
        for b in 0..4 {
            self.p.held[b].store(step.input.grid[b], Ordering::Relaxed);
            self.p.gate_pads[b].store(step.input.grid[4 + b], Ordering::Relaxed);
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if self.kit.menu {
            Text::new(APP_NAMES[self.instance], Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(f).ok();
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 44, 24, r.len().min(10), &r, BG, DIM, ACCENT);
            return;
        }
        if matches!(self.kit.layer(), Layer::Controls | Layer::Moments) {
            let col = self.kit.column(self);
            let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
            kit::draw::column(f, &col, 16, 40, 350, 230, pal);
        } else {
            self.draw_oled(f);
        }
        self.draw_status(f);
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kids_kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if self.taken || FIRMWARE_CLAIMED[self.instance].swap(true, Ordering::SeqCst) {
            return None;
        }
        self.taken = true;
        self.p.driving.store(true, Ordering::Relaxed);
        self.ensure_firmware();
        Some(Box::new(Processor {
            p: Arc::clone(&self.p),
            fw_id: 0,
            outs: Arc::clone(&self.outs),
            modbus: Arc::clone(&self.modbus),
            notes: self.note_out.take().unwrap_or_else(NoteOut::detached),
            core_credit: 0.0,
            ui_credit: 0.0,
            buttons: [Button::default(); 4],
            encoders: [Encoder::default(); 2],
            gate_was: [false; 4],
            note_wait: 0,
            note_playing: None,
            note_held_ms: 0,
        }))
    }
}

impl Drop for OcApp {
    fn drop(&mut self) {
        // Free for the next one (tests); the firmware thread itself keeps running.
        if self.taken {
            self.p.driving.store(false, Ordering::Relaxed);
            FIRMWARE_CLAIMED[self.instance].store(false, Ordering::SeqCst);
        }
    }
}

/// A button's pin: low while held, plus a requested extra low time.
#[derive(Clone, Copy, Default)]
struct Button {
    low: bool,
}

/// An encoder's pins: the quadrature sequence for one detent plays out over
/// five 1 ms ticks, which the firmware's own debounce and edge logic read.
#[derive(Clone, Copy, Default)]
struct Encoder {
    phase: usize,
    direction: i32,
}

const CW: [(i32, i32); 5] = [(1, 0), (1, 0), (0, 0), (1, 0), (1, 1)];
const CCW: [(i32, i32); 5] = [(0, 1), (0, 1), (0, 0), (0, 1), (1, 1)];

struct Processor {
    p: Arc<Shared>,
    /// Which firmware the panel state below belongs to (its address); a new one starts fresh.
    fw_id: usize,
    outs: Arc<CvOuts>,
    modbus: Arc<ModBus>,
    notes: NoteOut,
    core_credit: f64,
    ui_credit: f64,
    buttons: [Button; 4],
    encoders: [Encoder; 2],
    gate_was: [bool; 4],
    /// Milliseconds until a struck note reads its output (the firmware needs a
    /// few interrupts to answer a trigger).
    note_wait: u32,
    note_playing: Option<u8>,
    note_held_ms: u32,
}

impl Processor {
    /// One millisecond of the module's front panel.
    fn panel_tick(&mut self, fw: &Firmware) {
        for b in 0..4 {
            let extra = self.p.pulse_ms[b].load(Ordering::Relaxed);
            if extra > 0 {
                self.p.pulse_ms[b].store(extra - 1, Ordering::Relaxed);
            }
            let want_low = self.p.held[b].load(Ordering::Relaxed) || self.p.key_held[b].load(Ordering::Relaxed) || extra > 0;
            if want_low != self.buttons[b].low {
                self.buttons[b].low = want_low;
                fw.set_pin(PIN_BUTTON[b], if want_low { 0 } else { 1 });
            }
        }
        // Encoders: one detent at a time.
        for (e, pins) in [PIN_ENC_LEFT, PIN_ENC_RIGHT].iter().enumerate() {
            let enc = &mut self.encoders[e];
            if enc.direction == 0 {
                let pending = self.p.detents[e].load(Ordering::Relaxed);
                if pending != 0 {
                    enc.direction = pending.signum();
                    enc.phase = 0;
                    self.p.detents[e].store(pending - pending.signum(), Ordering::Relaxed);
                }
            }
            if enc.direction != 0 {
                let (a, b) = if enc.direction > 0 { CW[enc.phase] } else { CCW[enc.phase] };
                fw.set_pin(pins[0], a);
                fw.set_pin(pins[1], b);
                enc.phase += 1;
                if enc.phase >= CW.len() {
                    enc.direction = 0;
                }
            }
        }
        // Trigger inputs, active low: a pad or a mod-bus value above half.
        for g in 0..4 {
            let high = self.p.gate_pads[g].load(Ordering::Relaxed) || self.p.key_gates[g].load(Ordering::Relaxed) || self.p.gate_mod[g].get() > 0.5;
            if high != self.gate_was[g] {
                self.gate_was[g] = high;
                self.p.gates[g].store(high, Ordering::Relaxed);
                fw.set_pin(PIN_TR[g], if high { 0 } else { 1 });
                if g as u32 + 1 == self.p.note_gate.load(Ordering::Relaxed) && high {
                    self.note_wait = 3;
                }
            }
        }
        // CV inputs.
        for c in 0..4 {
            let v = (self.p.cv_knob[c].get() + self.p.cv_mod[c].get() * 5.0).clamp(-5.0, 5.0);
            let mv = (v * 1000.0) as i32;
            self.p.in_mv[c].store(mv, Ordering::Relaxed);
            fw.set_cv_millivolts(c as c_int, mv);
        }
    }

    /// Notes: struck when the chosen trigger rises, from the chosen output's
    /// pitch (0 V is C1, a volt an octave), released when the trigger falls
    /// (but not before 20 ms).
    fn note_tick(&mut self) {
        let channel = self.p.note_channel.load(Ordering::Relaxed) as usize;
        let gate = self.p.note_gate.load(Ordering::Relaxed) as usize;
        if channel == 0 || gate == 0 {
            if let Some(n) = self.note_playing.take() {
                self.notes.note_off(n);
            }
            self.note_wait = 0;
            return;
        }
        if self.note_wait > 0 {
            self.note_wait -= 1;
            if self.note_wait == 0 {
                if let Some(n) = self.note_playing.take() {
                    self.notes.note_off(n);
                }
                let volts = self.p.out_mv[channel - 1].load(Ordering::Relaxed) as f32 / 1000.0;
                let note = (24.0 + (volts * 12.0).round()).clamp(0.0, 127.0) as u8;
                self.notes.note_on(note, 100);
                self.note_playing = Some(note);
                self.note_held_ms = 0;
            }
        }
        if let Some(n) = self.note_playing {
            self.note_held_ms += 1;
            if !self.gate_was[gate - 1] && self.note_held_ms >= 20 {
                self.notes.note_off(n);
                self.note_playing = None;
            }
        }
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.0);
        if channels == 0 || !rate.is_finite() || rate < 1000.0 {
            return;
        }
        let frames = out.len() / channels;
        let fw = match self.p.fw.try_lock() {
            Ok(g) => g.clone(),
            Err(_) => None,
        };
        let Some(fw) = fw else { return };
        if !fw.timers_running() {
            return;
        }
        let id = Arc::as_ptr(&fw) as usize;
        if id != self.fw_id {
            // A different firmware: its pins start at rest, so ours must too.
            self.fw_id = id;
            self.buttons = [Button::default(); 4];
            self.encoders = [Encoder::default(); 2];
            self.gate_was = [false; 4];
            self.note_wait = 0;
        }
        // Millisecond by millisecond: the front panel, then that millisecond's
        // share of the 16.666 kHz interrupt, then the outputs.
        let per_ms = rate as f64 / 1000.0;
        let mut remaining = frames as f64 + self.ui_credit;
        while remaining >= per_ms {
            remaining -= per_ms;
            self.panel_tick(&fw);
            self.core_credit += 16.666;
            let core = self.core_credit as i32;
            self.core_credit -= core as f64;
            fw.run_isrs(core, 1);
            for c in 0..4 {
                let mv = fw.dac_millivolts(c as c_int);
                self.p.out_mv[c as usize].store(mv, Ordering::Relaxed);
            }
            self.note_tick();
            self.notes.advance(per_ms as u32);
        }
        self.ui_credit = remaining;
        for c in 0..4 {
            let v = (self.p.out_mv[c].load(Ordering::Relaxed) as f32 / 1000.0 / 5.0).clamp(-1.0, 1.0);
            self.outs.send(&self.modbus, c, v);
        }
        self.p.frames.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn create(ctx: &crate::app::AppContext, id: &str) -> Box<dyn crate::app::App> {
    let instance = APP_IDS.iter().position(|i| *i == id).unwrap_or(0);
    Box::new(OcApp::new(instance, ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()).with_notes(ctx.try_get()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// One firmware per process, so the tests that run it take turns.
    static LOCK: Mutex<()> = Mutex::new(());

    fn app() -> OcApp {
        app_n(0)
    }

    fn app_n(n: usize) -> OcApp {
        OcApp::build(n, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(MixerBus::new()), false)
    }

    /// Runs audio blocks until `done` (or `max_ms` of wall clock): the firmware
    /// thread lives in real time, so the test lets it.
    fn run_until(p: &mut Box<dyn AudioProcessor>, max_ms: u64, mut done: impl FnMut() -> bool) -> bool {
        let t0 = std::time::Instant::now();
        let mut buf = vec![0.0f32; 480 * 2];
        while t0.elapsed().as_millis() < max_ms as u128 {
            p.process(&mut buf, 2, 48_000.0);
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        false
    }

    /// Runs `ms` of simulated time (10 ms blocks) without waiting for the wall clock.
    fn run_ms(p: &mut Box<dyn AudioProcessor>, ms: usize) {
        let mut buf = vec![0.0f32; 480 * 2];
        for _ in 0..ms / 10 {
            p.process(&mut buf, 2, 48_000.0);
        }
    }

    /// A firmware's splash screen holds for about three seconds of real time, and each
    /// module's firmware boots from scratch, so wait that long from its own start.
    fn boot(a: &OcApp, p: &mut Box<dyn AudioProcessor>) {
        let t0 = a.started_at.expect("the firmware was started");
        run_until(p, 12_000, || t0.elapsed().as_secs_f32() > 6.5);
    }

    fn frame_hash(a: &OcApp) -> u64 {
        let mut f = [0u8; 1024];
        a.fw().unwrap().frame(&mut f);
        f.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
    }

    fn out(a: &OcApp, c: usize) -> i32 {
        a.p.out_mv[c].load(Ordering::Relaxed)
    }

    fn pulse_gate(a: &OcApp, p: &mut Box<dyn AudioProcessor>, g: usize) {
        a.p.gate_pads[g].store(true, Ordering::Relaxed);
        run_ms(p, 30);
        a.p.gate_pads[g].store(false, Ordering::Relaxed);
        run_ms(p, 40);
    }

    #[test]
    #[ignore = "writes a screenshot to the path in PORTAMAX_OC_SHOT"]
    fn screenshot() {
        let Some(path) = std::env::var_os("PORTAMAX_OC_SHOT") else { return };
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut a = app();
        let mut p = a.audio_processor().unwrap();
        boot(&a, &mut p);
        a.p.cv_knob[0].set(1.25);
        run_ms(&mut p, 60);
        pulse_gate(&a, &mut p, 0);
        a.tick(&Input { nav_x: 1, ..Default::default() });
        run_ms(&mut p, 100);
        run_until(&mut p, 300, || false);
        let mut fb = FrameBuffer::new();
        a.draw(&mut fb);
        let bytes: Vec<u8> = fb.buffer().iter().flat_map(|px| [(px >> 16) as u8, (px >> 8) as u8, *px as u8]).collect();
        let mut out = b"P6\n640 360\n255\n".to_vec();
        out.extend(bytes);
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn the_real_firmware_boots_and_quantizes_a_cv_on_a_trigger() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let a = app();
        let mut a = a;
        let mut p = a.audio_processor().expect("the first O&C owns the firmware");
        boot(&a, &mut p);
        assert_eq!(a.firmware_app(), "CopierMaschine", "the firmware's default app is up");
        // CopierMaschine samples CV 1 on trigger 1 and quantizes it: a volt is a volt,
        // and 0.58 V is the seventh semitone's 0.583.
        a.p.cv_knob[0].set(1.0);
        run_ms(&mut p, 60);
        pulse_gate(&a, &mut p, 0);
        assert!((out(&a, 0) - 1000).abs() < 15, "1 V in, 1 V out: {} mV", out(&a, 0));
        a.p.cv_knob[0].set(0.58);
        run_ms(&mut p, 60);
        pulse_gate(&a, &mut p, 0);
        assert!((out(&a, 0) - 583).abs() < 15, "0.58 V quantizes to the nearest semitone: {} mV", out(&a, 0));
        assert!((out(&a, 1) - 1000).abs() < 15, "and the register shifted the old value to B: {} mV", out(&a, 1));
    }

    #[test]
    fn a_module_can_be_reflashed_with_another_firmware_while_running() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let avail = firmware::available();
        let hemi = VARIANTS.iter().position(|v| v.id == "hemi").unwrap();
        if !avail.contains(&hemi) {
            return;
        }
        let mut a = app();
        a.set_variant(hemi);
        let mut p = a.audio_processor().expect("the first O&C owns the firmware");
        boot(&a, &mut p);
        assert_eq!(a.variant_id(), "hemi");
        assert_eq!(a.firmware_app(), "Hemisphere", "the Hemisphere Suite's own app is up");
        // Reflash back to stock while the audio thread is running.
        a.set_variant(0);
        a.tick(&Input::default());
        boot(&a, &mut p);
        assert_eq!(a.firmware_app(), "CopierMaschine");
        a.tick(&Input::default());
        assert!(a.retiring.is_empty() || a.retiring.iter().all(|f| Arc::strong_count(f) >= 1));
    }

    #[test]
    fn the_right_encoder_changes_what_the_firmware_shows() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut a = app();
        let mut p = a.audio_processor().expect("the first O&C owns the firmware");
        boot(&a, &mut p);
        run_until(&mut p, 1000, || false);
        let before = frame_hash(&a);
        // One click right on the D-pad turns the right encoder once.
        a.tick(&Input { nav_x: 1, ..Default::default() });
        run_ms(&mut p, 80);
        run_until(&mut p, 200, || false);
        assert_ne!(frame_hash(&a), before, "the selected value on the OLED changed");
        // And down on the D-pad turns the left encoder: the menu cursor moves.
        let before = frame_hash(&a);
        a.tick(&Input { navigation_steps: 1, ..Default::default() });
        run_ms(&mut p, 80);
        run_until(&mut p, 200, || false);
        assert_ne!(frame_hash(&a), before, "the cursor moved to the next row");
    }

    #[test]
    fn a_trigger_can_come_from_the_mod_bus_and_the_pitch_can_play_another_app() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let bus = Arc::new(NoteBus::new());
        let inbox = bus.register_instrument("synth", "Test Synth").unwrap();
        let mut a = app().with_notes(Some(Arc::clone(&bus)));
        a.kit_edit(C_PLAYS, 1); // none -> the first instrument
        assert_eq!(a.note_route.label(), "Test Synth");
        a.kit_set_norm(C_NOTE_CH, 0.25); // output A
        a.kit_set_norm(C_NOTE_GATE, 0.25); // struck by TR 1
        let mut p = a.audio_processor().expect("the first O&C owns the firmware");
        boot(&a, &mut p);
        a.p.cv_knob[0].set(2.0);
        run_ms(&mut p, 60);
        let mut view = crate::note_bus::NoteView::default();
        // TR 1 high through the mod bus, as another app's output would drive it.
        a.p.gate_mod[0].set(1.0);
        let mut struck = None;
        for _ in 0..10 {
            run_ms(&mut p, 10);
            inbox.poll(&mut view);
            if let Some(n) = (0..128).find(|&n| view.keys[n] > 0) {
                struck = Some(n);
            }
        }
        a.p.gate_mod[0].set(0.0);
        run_ms(&mut p, 60);
        assert_eq!(struck, Some(48), "2 V is two octaves above C1: MIDI 48");
        inbox.poll(&mut view);
        assert!(view.keys.iter().all(|v| *v == 0), "and the note ends with the trigger");
    }

    #[test]
    fn there_is_one_firmware_so_a_second_app_is_silent_until_the_first_lets_go() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut first = app();
        let p1 = first.audio_processor();
        assert!(p1.is_some());
        let mut second = app();
        assert!(second.audio_processor().is_none());
        drop(p1);
        drop(first);
        let mut third = app();
        assert!(third.audio_processor().is_some(), "released when the first is dropped");
    }

    #[test]
    fn its_inputs_are_mod_inputs_and_the_pads_are_the_panel() {
        let modbus = Arc::new(ModBus::new());
        let a = OcApp::build(0, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::clone(&modbus), Arc::new(MixerBus::new()), false);
        for name in ["O&C: CV 1", "O&C: CV 4", "O&C: TR 1", "O&C: TR 4"] {
            assert!(modbus.index_of(name).is_some(), "{name}");
        }
        assert_eq!(a.kit.layer_label(), "PANEL");
        assert_eq!((a.kit_pad_label(0, 0), a.kit_pad_label(0, 3), a.kit_pad_label(0, 4)), ("UP".to_string(), "R".to_string(), "TR1".to_string()));
        assert!(a.wants_fullscreen());
    }

    #[test]
    fn four_modules_run_at_once_each_with_its_own_firmware_state() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut apps: Vec<OcApp> = (0..INSTANCES).map(app_n).collect();
        let mut procs: Vec<Box<dyn AudioProcessor>> = apps.iter_mut().map(|a| a.audio_processor().expect("each instance owns its firmware")).collect();
        let mut buf = vec![0.0f32; 480 * 2];
        // Boot them all together; their splash screens last about three seconds of real time.
        let t0 = std::time::Instant::now();
        while t0.elapsed().as_secs_f32() < 7.0 {
            for p in procs.iter_mut() {
                p.process(&mut buf, 2, 48_000.0);
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        for (i, a) in apps.iter().enumerate() {
            assert_eq!(a.firmware_app(), "CopierMaschine", "instance {i} is up");
        }
        // Different CVs in, a trigger on each: each quantizes its own input.
        let volts = [1.0, 2.0, 0.58, -1.0];
        for (a, v) in apps.iter().zip(volts) {
            a.p.cv_knob[0].set(v);
        }
        for _ in 0..6 {
            for p in procs.iter_mut() {
                p.process(&mut buf, 2, 48_000.0);
            }
        }
        for a in &apps {
            a.p.gate_pads[0].store(true, Ordering::Relaxed);
        }
        for _ in 0..3 {
            for p in procs.iter_mut() {
                p.process(&mut buf, 2, 48_000.0);
            }
        }
        for a in &apps {
            a.p.gate_pads[0].store(false, Ordering::Relaxed);
        }
        for _ in 0..6 {
            for p in procs.iter_mut() {
                p.process(&mut buf, 2, 48_000.0);
            }
        }
        let expect = [1000, 2000, 583, -1000];
        for (i, a) in apps.iter().enumerate() {
            assert!((out(a, 0) - expect[i]).abs() < 20, "instance {i}: {} mV, wanted about {}", out(a, 0), expect[i]);
        }
        // Their screens are their own: turning one's encoder leaves the others as they were.
        let hashes = |apps: &[OcApp]| -> Vec<u64> {
            apps.iter()
                .enumerate()
                .map(|(_, a)| {
                    let mut f = [0u8; 1024];
                    a.fw().unwrap().frame(&mut f);
                    f.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
                })
                .collect()
        };
        let before = hashes(&apps);
        apps[2].tick(&Input { nav_x: 1, ..Default::default() });
        for _ in 0..10 {
            for p in procs.iter_mut() {
                p.process(&mut buf, 2, 48_000.0);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let after = hashes(&apps);
        assert_ne!(before[2], after[2], "instance 2's screen changed");
        for i in [0, 1, 3] {
            assert_eq!(before[i], after[i], "instance {i}'s screen did not");
        }
    }

    #[test]
    fn a_module_can_open_its_own_window_that_takes_the_keyboard() {
        let mut a = app_n(1);
        assert!(a.popout().is_none(), "no window until asked");
        a.edit_row(N_CONTROLS + 1, 1); // the menu's Window row
        let (title, frame) = a.popout().expect("a window is open");
        assert!(title.starts_with("O&C 2"), "{title}");
        assert_eq!((frame.width, frame.height), (640, 360));
        a.popout_key("\u{f703}", true); // right arrow: the right encoder
        a.popout_key("\u{f701}", true); // down arrow: the left encoder
        assert_eq!((a.p.detents[1].load(Ordering::Relaxed), a.p.detents[0].load(Ordering::Relaxed)), (1, 1));
        a.popout_key("\n", true);
        a.popout_key("3", true);
        assert!(a.p.key_held[3].load(Ordering::Relaxed) && a.p.key_gates[2].load(Ordering::Relaxed), "Return holds R, 3 holds TR 3");
        a.popout_key("\n", false);
        assert!(!a.p.key_held[3].load(Ordering::Relaxed));
        a.popout_closed();
        assert!(a.popout().is_none() && !a.p.key_gates[2].load(Ordering::Relaxed), "closing releases everything");
    }
}
