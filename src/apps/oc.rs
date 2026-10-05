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
//! The firmware's settings are kept in its EEPROM, saved in `saves/oc/`. One
//! copy of the firmware exists per process; a second O&C app is silent.

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
use std::ffi::{c_char, c_int, CStr};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering},
    Arc,
};

const APP_NAME: &str = "O&C";

unsafe extern "C" {
    fn oc_start();
    fn oc_timers_running() -> c_int;
    fn oc_run_isrs(core_ticks: c_int, ui_ticks: c_int);
    fn oc_set_pin(pin: c_int, level: c_int);
    fn oc_set_cv_millivolts(channel: c_int, millivolts: c_int);
    fn oc_dac_millivolts(channel: c_int) -> c_int;
    fn oc_frame(out: *mut u8) -> u64;
    fn oc_eeprom_read(out: *mut u8);
    fn oc_eeprom_write(input: *const u8);
    fn oc_app_name() -> *const c_char;
}

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

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "oc",
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
}

/// One copy of the firmware per process.
static FIRMWARE_CLAIMED: AtomicBool = AtomicBool::new(false);

pub struct OcApp {
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
    eeprom_path: Option<PathBuf>,
    eeprom_saved: Vec<u8>,
    last_save: std::time::Instant,
}

fn eeprom_file() -> PathBuf {
    let root = std::env::var_os("PORTAMAX_SAVES_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves")));
    root.join("oc").join("eeprom.bin")
}

impl OcApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, _bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        Self::build(sensitivity, nav, mods, mixer, !cfg!(test))
    }

    fn build(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, mixer: Arc<MixerBus>, persist: bool) -> Self {
        // The module has no audio of its own, but its level is registered like
        // everyone's so the mixer can list it.
        let _ = mixer.register(APP_NAME, &mods);
        let (route, out) = NoteRoute::new(None, APP_NAME, "oc", false);
        let eeprom_path = persist.then(eeprom_file);
        let saved = eeprom_path.as_ref().and_then(|p| std::fs::read(p).ok()).filter(|d| d.len() == EEPROM_SIZE);
        if let Some(d) = &saved {
            // The firmware reads its settings when it boots, so they go in first.
            unsafe { oc_eeprom_write(d.as_ptr()) };
        }
        let p = Arc::new(Shared {
            detents: [AtomicI32::new(0), AtomicI32::new(0)],
            held: std::array::from_fn(|_| AtomicBool::new(false)),
            pulse_ms: std::array::from_fn(|_| AtomicU32::new(0)),
            gate_pads: std::array::from_fn(|_| AtomicBool::new(false)),
            cv_knob: std::array::from_fn(|_| AtomicF32::new(0.0)),
            cv_mod: std::array::from_fn(|i| mods.register(format!("{APP_NAME}: CV {}", i + 1))),
            gate_mod: std::array::from_fn(|i| mods.register(format!("{APP_NAME}: TR {}", i + 1))),
            note_channel: AtomicU32::new(0),
            note_gate: AtomicU32::new(0),
            out_mv: std::array::from_fn(|_| AtomicI32::new(0)),
            in_mv: std::array::from_fn(|_| AtomicI32::new(0)),
            gates: std::array::from_fn(|_| AtomicBool::new(false)),
            frames: AtomicU64::new(0),
            driving: AtomicBool::new(false),
        });
        Self {
            p,
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), persist),
            outs: Arc::new(CvOuts::new(&OUTS)),
            modbus: mods,
            note_route: route,
            note_out: Some(out),
            frame: [0; 1024],
            eeprom_path,
            eeprom_saved: saved.unwrap_or_default(),
            last_save: std::time::Instant::now(),
        }
    }

    /// Lets the module play other apps (see note_bus.rs).
    pub fn with_notes(mut self, bus: Option<Arc<NoteBus>>) -> Self {
        let (route, out) = NoteRoute::new(bus, APP_NAME, "oc", false);
        self.note_route = route;
        self.note_out = Some(out);
        self
    }

    /// The name of the firmware app on screen ("CopierMaschine"...).
    pub fn firmware_app(&self) -> String {
        if self.p.driving.load(Ordering::Relaxed) {
            unsafe { CStr::from_ptr(oc_app_name()) }.to_string_lossy().into_owned()
        } else {
            String::new()
        }
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
        for (i, name) in OUTS.iter().enumerate() {
            r.push((format!("Out {name} App"), self.outs.app_label(&self.modbus, i), false));
            r.push((format!("Out {name} Input"), self.outs.input_label(&self.modbus, i), false));
        }
        r
    }

    fn rows_len(&self) -> usize {
        N_CONTROLS + OUTS.len() * 2
    }

    fn edit_row(&mut self, row: usize, delta: i32) {
        if delta == 0 {
            return;
        }
        if row < N_CONTROLS {
            self.kit_edit(row, delta);
        } else {
            let k = row - N_CONTROLS;
            if k % 2 == 0 {
                self.outs.step_app(&self.modbus, k / 2, delta.signum());
            } else {
                self.outs.step_input(&self.modbus, k / 2, delta.signum());
            }
        }
    }

    /// Persists the firmware's EEPROM when it has changed (a settings save).
    fn save_eeprom(&mut self) {
        let Some(path) = self.eeprom_path.clone() else { return };
        if self.last_save.elapsed().as_secs_f32() < 2.0 || !self.p.driving.load(Ordering::Relaxed) {
            return;
        }
        self.last_save = std::time::Instant::now();
        let mut now = vec![0u8; EEPROM_SIZE];
        unsafe { oc_eeprom_read(now.as_mut_ptr()) };
        if now != self.eeprom_saved {
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
        unsafe { oc_frame(self.frame.as_mut_ptr()) };
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
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        self.save_eeprom();
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
            Text::new(APP_NAME, Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(f).ok();
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
        if self.taken || FIRMWARE_CLAIMED.swap(true, Ordering::SeqCst) {
            return None;
        }
        self.taken = true;
        self.p.driving.store(true, Ordering::Relaxed);
        unsafe { oc_start() };
        Some(Box::new(Processor {
            p: Arc::clone(&self.p),
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
            FIRMWARE_CLAIMED.store(false, Ordering::SeqCst);
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
    fn panel_tick(&mut self) {
        for b in 0..4 {
            let extra = self.p.pulse_ms[b].load(Ordering::Relaxed);
            if extra > 0 {
                self.p.pulse_ms[b].store(extra - 1, Ordering::Relaxed);
            }
            let want_low = self.p.held[b].load(Ordering::Relaxed) || extra > 0;
            if want_low != self.buttons[b].low {
                self.buttons[b].low = want_low;
                unsafe { oc_set_pin(PIN_BUTTON[b], if want_low { 0 } else { 1 }) };
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
                unsafe {
                    oc_set_pin(pins[0], a);
                    oc_set_pin(pins[1], b);
                }
                enc.phase += 1;
                if enc.phase >= CW.len() {
                    enc.direction = 0;
                }
            }
        }
        // Trigger inputs, active low: a pad or a mod-bus value above half.
        for g in 0..4 {
            let high = self.p.gate_pads[g].load(Ordering::Relaxed) || self.p.gate_mod[g].get() > 0.5;
            if high != self.gate_was[g] {
                self.gate_was[g] = high;
                self.p.gates[g].store(high, Ordering::Relaxed);
                unsafe { oc_set_pin(PIN_TR[g], if high { 0 } else { 1 }) };
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
            unsafe { oc_set_cv_millivolts(c as c_int, mv) };
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
        if unsafe { oc_timers_running() } == 0 {
            return;
        }
        // Millisecond by millisecond: the front panel, then that millisecond's
        // share of the 16.666 kHz interrupt, then the outputs.
        let per_ms = rate as f64 / 1000.0;
        let mut remaining = frames as f64 + self.ui_credit;
        while remaining >= per_ms {
            remaining -= per_ms;
            self.panel_tick();
            self.core_credit += 16.666;
            let core = self.core_credit as i32;
            self.core_credit -= core as f64;
            unsafe { oc_run_isrs(core, 1) };
            for c in 0..4 {
                let mv = unsafe { oc_dac_millivolts(c) };
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

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(OcApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()).with_notes(ctx.try_get()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// One firmware per process, so the tests that run it take turns.
    static LOCK: Mutex<()> = Mutex::new(());

    fn app() -> OcApp {
        OcApp::build(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(MixerBus::new()), false)
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

    /// The firmware's splash screen holds for about three seconds of real time, so
    /// the first test to need it waits that long; later ones find it running.
    fn boot(p: &mut Box<dyn AudioProcessor>) {
        static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let t0 = *STARTED.get_or_init(std::time::Instant::now);
        run_until(p, 10_000, || t0.elapsed().as_secs_f32() > 6.5);
    }

    fn frame_hash() -> u64 {
        let mut f = [0u8; 1024];
        unsafe { oc_frame(f.as_mut_ptr()) };
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
    fn the_real_firmware_boots_and_quantizes_a_cv_on_a_trigger() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let a = app();
        let mut a = a;
        let mut p = a.audio_processor().expect("the first O&C owns the firmware");
        boot(&mut p);
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
    fn the_right_encoder_changes_what_the_firmware_shows() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut a = app();
        let mut p = a.audio_processor().expect("the first O&C owns the firmware");
        boot(&mut p);
        run_until(&mut p, 1000, || false);
        let before = frame_hash();
        // One click right on the D-pad turns the right encoder once.
        a.tick(&Input { nav_x: 1, ..Default::default() });
        run_ms(&mut p, 80);
        run_until(&mut p, 200, || false);
        assert_ne!(frame_hash(), before, "the selected value on the OLED changed");
        // And down on the D-pad turns the left encoder: the menu cursor moves.
        let before = frame_hash();
        a.tick(&Input { navigation_steps: 1, ..Default::default() });
        run_ms(&mut p, 80);
        run_until(&mut p, 200, || false);
        assert_ne!(frame_hash(), before, "the cursor moved to the next row");
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
        boot(&mut p);
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
        let a = OcApp::build(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::clone(&modbus), Arc::new(MixerBus::new()), false);
        for name in ["O&C: CV 1", "O&C: CV 4", "O&C: TR 1", "O&C: TR 4"] {
            assert!(modbus.index_of(name).is_some(), "{name}");
        }
        assert_eq!(a.kit.layer_label(), "PANEL");
        assert_eq!((a.kit_pad_label(0, 0), a.kit_pad_label(0, 3), a.kit_pad_label(0, 4)), ("UP".to_string(), "R".to_string(), "TR1".to_string()));
        assert!(a.wants_fullscreen());
    }
}
