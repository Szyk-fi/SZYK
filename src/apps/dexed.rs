//! Dexed: a Yamaha DX7-compatible six-operator FM synth, running Dexed's own
//! engine (vendor/dexed/msfa, Apache-2.0, (c) Pascal Gauthier and Google)
//! through vendor/bridge/dexed_bridge.cc. It replaces the earlier Cascade:
//! where Cascade approximated the DX7's envelopes and operator math, this
//! runs the engine that Dexed spent years making match the real chip's
//! output -- the DX7's integer envelopes, rate and level scaling, keyboard
//! scaling, LFO, pitch envelope, operator feedback and all 32 algorithms.
//!
//! Voices come from real DX7 SysEx: drop `.syx` banks (32-voice bulk dumps or
//! single-voice dumps) in `dx7_presets/`, one folder per bank collection (see
//! its README). Pick a Bank, then browse its Voices with the D-pad. There are
//! no voices bundled -- Yamaha's factory sounds are Yamaha's -- so with none
//! present it plays the DX7's own power-on "INIT VOICE" (a plain sine).
//!
//! The pads play chromatically from C3, sixteen voices deep. Bend is a pitch
//! wheel (two semitones); Mod is the mod wheel, which the engine routes to
//! the voice's own LFO depth; Pressure is channel aftertouch, so a pad (or a
//! hand) pressed into the sensor swells the vibrato the way a DX7's keybed
//! does. Editing the sound itself -- operators, envelopes, algorithms -- is
//! done in a DX7 editor and loaded as SysEx, as on the hardware.
//!
//! Only Dexed's "Modern" engine is used; its Mark I and OPL engines are GPL.

use super::mi_kit::{pad_note, Keys, NoteQueue};
use crate::{
    app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12},
    util::AtomicF32,
};
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}, text::Text};
use std::ffi::{c_double, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "Dexed";

/// Where banks live, anchored at the crate root like `samples/`.
const DX7_PRESETS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/dx7_presets");

// Own palette: the DX7's green-black glass and amber legends.
const BG: Rgb565 = Rgb565::new(1, 4, 3);
const INK: Rgb565 = Rgb565::new(24, 56, 18);
const ACCENT: Rgb565 = Rgb565::new(31, 42, 4);
const DIM: Rgb565 = Rgb565::new(9, 26, 11);
const FAINT: Rgb565 = Rgb565::new(3, 9, 6);

unsafe extern "C" {
    fn dexed_create(rate: c_double) -> *mut c_void;
    fn dexed_destroy(h: *mut c_void);
    fn dexed_set_rate(h: *mut c_void, rate: c_double);
    fn dexed_load_packed_voice(h: *mut c_void, packed: *const u8);
    fn dexed_load_unpacked_voice(h: *mut c_void, unpacked: *const u8);
    fn dexed_note_on(h: *mut c_void, midi: c_int, velocity: c_int);
    fn dexed_note_off(h: *mut c_void, midi: c_int);
    fn dexed_sustain(h: *mut c_void, on: c_int);
    fn dexed_all_off(h: *mut c_void);
    fn dexed_controllers(h: *mut c_void, pitch_bend: c_int, mod_wheel: c_int, breath: c_int, foot: c_int, aftertouch: c_int, bend_range: c_int);
    fn dexed_render(h: *mut c_void, out: *mut f32, n: c_int);
    fn dexed_active_voices(h: *mut c_void) -> c_int;
}

/// One DX7 voice as it sits in a SysEx file.
#[derive(Clone, Copy, PartialEq, Debug)]
enum VoiceData {
    /// 128 bytes, from a 32-voice bulk dump.
    Packed([u8; 128]),
    /// 155 bytes, from a single-voice dump.
    Unpacked([u8; 155]),
}

#[derive(Clone)]
struct Voice {
    name: String,
    data: VoiceData,
}

struct Bank {
    name: String,
    voices: Vec<Voice>,
}

/// The ten ASCII characters of a voice name, trimmed.
fn voice_name(bytes: &[u8]) -> String {
    let name: String = bytes.iter().map(|&b| (b & 0x7f) as char).collect();
    let trimmed = name.trim_end_matches(['\0', ' ']).trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Every voice in one SysEx file's bytes: messages from `F0` to `F7`, either a
/// 32-voice bulk dump (format 9, 4096 bytes) or a single-voice dump (format 0,
/// 155 bytes). Anything else, or anything truncated, is skipped.
fn parse_syx(data: &[u8]) -> Vec<Voice> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if data[i] != 0xF0 {
            i += 1;
            continue;
        }
        let Some(end) = data[i..].iter().position(|&b| b == 0xF7) else { break };
        let message = &data[i..i + end + 1];
        i += end + 1;
        if message.len() < 6 || message[1] != 0x43 {
            continue;
        }
        let format = message[3];
        let count = ((message[4] as usize) << 7) | message[5] as usize;
        let body = &message[6..];
        if body.len() < count + 1 {
            continue;
        }
        match format {
            9 if count == 4096 => {
                for v in 0..32 {
                    let mut packed = [0u8; 128];
                    packed.copy_from_slice(&body[v * 128..v * 128 + 128]);
                    out.push(Voice { name: voice_name(&packed[118..128]), data: VoiceData::Packed(packed) });
                }
            }
            0 if count == 155 => {
                let mut single = [0u8; 155];
                single.copy_from_slice(&body[..155]);
                out.push(Voice { name: voice_name(&single[145..155]), data: VoiceData::Unpacked(single) });
            }
            _ => {}
        }
    }
    out
}

fn is_syx(path: &Path) -> bool {
    path.is_file() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("syx"))
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    v.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
    v
}

/// `dir/*.syx` and each subfolder's `*.syx` as a named bank; loose files at
/// the top form "(root)". Empty banks are dropped.
fn scan_banks(dir: &Path) -> Vec<Bank> {
    let load = |files: Vec<PathBuf>| -> Vec<Voice> { files.iter().filter(|p| is_syx(p)).filter_map(|p| std::fs::read(p).ok()).flat_map(|d| parse_syx(&d)).collect() };
    let entries = sorted_entries(dir);
    let mut banks = Vec::new();
    let root = load(entries.clone());
    if !root.is_empty() {
        banks.push(Bank { name: "(root)".into(), voices: root });
    }
    for e in entries.iter().filter(|p| p.is_dir()) {
        let voices = load(sorted_entries(e));
        if !voices.is_empty() {
            banks.push(Bank { name: e.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), voices });
        }
    }
    banks
}

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "dexed",
        layers: vec![Layer::Native(0, "KEYS"), Layer::Controls, Layer::Moments],
        hero: vec![[C_VOICE, C_LEVEL], [C_BEND, C_MOD], [C_PRESSURE, C_OCTAVE], [C_BANK, C_SUSTAIN]],
        browse: Some(C_VOICE),
        // The stick is the pitch wheel and the mod wheel; a hand swells pressure.
        routes: Routes { stick_x: Some(C_BEND), stick_y: Some(C_MOD), hand_l: Some(C_PRESSURE), hand_r: Some(C_LEVEL) },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: false,
    }
}

const C_VOICE: usize = 0;
const C_BANK: usize = 1;
const C_LEVEL: usize = 2;
const C_BEND: usize = 3;
const C_MOD: usize = 4;
const C_PRESSURE: usize = 5;
const C_OCTAVE: usize = 6;
const C_SUSTAIN: usize = 7;
const N_CONTROLS: usize = 8;

struct Shared {
    level: AtomicF32,
    /// -1..1, two semitones each way.
    bend: AtomicF32,
    modwheel: AtomicF32,
    pressure: AtomicF32,
    octave: AtomicI32,
    sustain: AtomicBool,
    cv: [Arc<AtomicF32>; 4],
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    peak: AtomicF32,
    voices: AtomicUsize,
    rate: AtomicF32,
    /// A voice waiting for the audio thread.
    pending: Mutex<Option<VoiceData>>,
}

pub struct DexedApp {
    p: Arc<Shared>,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
    keys: Keys,
    banks: Vec<Bank>,
    bank: usize,
    voice: usize,
}

impl DexedApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        Self::with_dir(Path::new(DX7_PRESETS_DIR), sensitivity, nav, mods, bus, mixer)
    }

    pub fn with_dir(dir: &Path, sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let mut app = Self {
            p: Arc::new(Shared {
                level: AtomicF32::new(0.8),
                bend: AtomicF32::new(0.0),
                modwheel: AtomicF32::new(0.0),
                pressure: AtomicF32::new(0.0),
                octave: AtomicI32::new(0),
                sustain: AtomicBool::new(false),
                cv: [
                    mods.register(format!("{APP_NAME}: Level")),
                    mods.register(format!("{APP_NAME}: Bend")),
                    mods.register(format!("{APP_NAME}: Mod")),
                    mods.register(format!("{APP_NAME}: Pressure")),
                ],
                mix_level,
                ext_mix_level,
                output,
                peak: AtomicF32::new(0.0),
                voices: AtomicUsize::new(0),
                rate: AtomicF32::new(48_000.0),
                pending: Mutex::new(None),
            }),
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            keys: Keys::default(),
            banks: scan_banks(dir),
            bank: 0,
            voice: 0,
        };
        app.send_voice();
        app
    }

    fn current(&self) -> Option<&Voice> {
        self.banks.get(self.bank).and_then(|b| b.voices.get(self.voice))
    }

    fn voice_name(&self) -> String {
        self.current().map_or_else(|| "INIT VOICE".to_string(), |v| v.name.clone())
    }

    fn bank_name(&self) -> String {
        self.banks.get(self.bank).map_or_else(|| "(none)".to_string(), |b| b.name.clone())
    }

    fn voices_in_bank(&self) -> usize {
        self.banks.get(self.bank).map_or(0, |b| b.voices.len())
    }

    /// Queues the current voice for the audio thread.
    fn send_voice(&self) {
        if let Some(v) = self.current() {
            if let Ok(mut slot) = self.p.pending.lock() {
                *slot = Some(v.data);
            }
        }
    }

    fn step_voice(&mut self, d: i32) {
        let n = self.voices_in_bank() as i32;
        if n > 0 && d != 0 {
            self.voice = (self.voice as i32 + d.signum()).rem_euclid(n) as usize;
            self.send_voice();
        }
    }

    fn step_bank(&mut self, d: i32) {
        let n = self.banks.len() as i32;
        if n > 1 && d != 0 {
            self.bank = (self.bank as i32 + d.signum()).rem_euclid(n) as usize;
            self.voice = 0;
            self.send_voice();
        }
    }

    fn octave(&self) -> i32 {
        self.p.octave.load(Ordering::Relaxed)
    }

    fn level(&self) -> f32 {
        (self.p.level.get() + self.p.cv[0].get()).clamp(0.0, 1.0)
    }
    fn bend(&self) -> f32 {
        (self.p.bend.get() + self.p.cv[1].get() * 2.0).clamp(-1.0, 1.0)
    }
    fn modwheel(&self) -> f32 {
        (self.p.modwheel.get() + self.p.cv[2].get()).clamp(0.0, 1.0)
    }
    fn pressure(&self) -> f32 {
        (self.p.pressure.get() + self.p.cv[3].get()).clamp(0.0, 1.0)
    }

    fn text(&self, i: usize) -> (String, String) {
        match i {
            C_VOICE => ("Voice".into(), format!("{} {}/{}", self.voice_name(), if self.voices_in_bank() > 0 { self.voice + 1 } else { 0 }, self.voices_in_bank())),
            C_BANK => ("Bank".into(), if self.banks.is_empty() { "(none in dx7_presets/)".into() } else { format!("{} {}/{}", self.bank_name(), self.bank + 1, self.banks.len()) }),
            C_LEVEL => ("Level".into(), format!("{:.0}%", self.level() * 100.0)),
            C_BEND => ("Bend".into(), format!("{:+.1} st", self.bend() * 2.0)),
            C_MOD => ("Mod".into(), format!("{:.0}%", self.modwheel() * 100.0)),
            C_PRESSURE => ("Pressure".into(), format!("{:.0}%", self.pressure() * 100.0)),
            C_OCTAVE => ("Octave".into(), format!("{:+}", self.octave())),
            _ => ("Sustain".into(), if self.p.sustain.load(Ordering::Relaxed) { "on" } else { "off" }.into()),
        }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        (0..N_CONTROLS).map(|i| {
            let (n, v) = self.text(i);
            (n, v, false)
        }).collect()
    }

    fn edit_continuous(&mut self, i: usize, d: i32) {
        let step = d as f32 * 0.01 * self.sensitivity.get().max(0.01) * 10.0;
        match i {
            C_LEVEL => self.p.level.set((self.p.level.get() + step).clamp(0.0, 1.0)),
            C_BEND => self.p.bend.set((self.p.bend.get() + step * 2.0).clamp(-1.0, 1.0)),
            C_MOD => self.p.modwheel.set((self.p.modwheel.get() + step).clamp(0.0, 1.0)),
            C_PRESSURE => self.p.pressure.set((self.p.pressure.get() + step).clamp(0.0, 1.0)),
            _ => {}
        }
    }
}

impl PlayHost for DexedApp {
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
            C_VOICE => if self.voices_in_bank() > 1 { self.voice as f32 / (self.voices_in_bank() - 1) as f32 } else { 0.0 },
            C_BANK => if self.banks.len() > 1 { self.bank as f32 / (self.banks.len() - 1) as f32 } else { 0.0 },
            C_LEVEL => self.p.level.get(),
            C_BEND => (self.p.bend.get() + 1.0) / 2.0,
            C_MOD => self.p.modwheel.get(),
            C_PRESSURE => self.p.pressure.get(),
            C_OCTAVE => (self.octave() + 3) as f32 / 6.0,
            C_SUSTAIN => self.p.sustain.load(Ordering::Relaxed) as u8 as f32,
            _ => return None,
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        matches!(i, C_VOICE | C_BANK | C_OCTAVE | C_SUSTAIN)
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match i {
            C_VOICE => self.step_voice(delta),
            C_BANK => self.step_bank(delta),
            C_OCTAVE => self.p.octave.store((self.octave() + delta.signum()).clamp(-3, 3), Ordering::Relaxed),
            C_SUSTAIN => {
                if delta != 0 {
                    self.p.sustain.store(delta > 0, Ordering::Relaxed);
                }
            }
            _ => self.edit_continuous(i, delta),
        }
    }
    fn kit_reset(&mut self, i: usize) {
        match i {
            C_VOICE => {
                self.voice = 0;
                self.send_voice();
            }
            C_LEVEL => self.p.level.set(0.8),
            C_BEND => self.p.bend.set(0.0),
            C_MOD => self.p.modwheel.set(0.0),
            C_PRESSURE => self.p.pressure.set(0.0),
            C_OCTAVE => self.p.octave.store(0, Ordering::Relaxed),
            C_SUSTAIN => self.p.sustain.store(false, Ordering::Relaxed),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i {
            C_VOICE => {
                let n = self.voices_in_bank();
                if n > 0 {
                    let target = (v * (n - 1) as f32).round() as usize;
                    if target != self.voice {
                        self.voice = target;
                        self.send_voice();
                    }
                }
            }
            C_BANK => {
                if self.banks.len() > 1 {
                    let target = (v * (self.banks.len() - 1) as f32).round() as usize;
                    if target != self.bank {
                        self.bank = target;
                        self.voice = 0;
                        self.send_voice();
                    }
                }
            }
            C_LEVEL => self.p.level.set(v),
            C_BEND => self.p.bend.set(v * 2.0 - 1.0),
            C_MOD => self.p.modwheel.set(v),
            C_PRESSURE => self.p.pressure.set(v),
            C_OCTAVE => self.p.octave.store((v * 6.0).round() as i32 - 3, Ordering::Relaxed),
            C_SUSTAIN => self.p.sustain.store(v >= 0.5, Ordering::Relaxed),
            _ => {}
        }
    }
    fn kit_line(&self) -> String {
        format!("{} · {}", self.bank_name(), self.voice_name())
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        crate::util::note_name(pad_note(pad, self.octave()) as i32)
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, held: bool) -> crate::led_output::PadColor {
        use crate::led_output::PadColor;
        if held {
            PadColor::Green
        } else if pad_note(pad, self.octave()) % 12 == 0 {
            PadColor::Blue
        } else {
            PadColor::Off
        }
    }
}

impl App for DexedApp {
    fn supports_pad_lock(&self) -> bool {
        true
    }
    fn play_surface(&self) -> bool {
        true
    }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (!self.kit.menu).then(|| self.kit.column(self))
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
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let octave = self.octave();
        let mut keyed = step.input.clone();
        keyed.midi_keys = input.midi_keys;
        self.keys.update(&keyed, step.native.is_some(), |p| pad_note(p, octave));
        if step.menu {
            let input = &step.input;
            self.list.navigate_input(input, N_CONTROLS, self.nav.get() as i32);
            let sel = self.list.selected.min(N_CONTROLS - 1);
            self.kit_edit(sel, input.knob2);
            if input.knob2_press {
                self.kit_reset(sel);
            }
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        Text::new(APP_NAME, Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(f).ok();
        if self.kit.menu {
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 44, 24, r.len(), &r, BG, DIM, ACCENT);
        } else if let Some(col) = self.play_column() {
            let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
            kit::draw::column(f, &col, 16, 40, 350, 280, pal);
        }
        let dim = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        let clip = |s: &str, n: usize| s.chars().take(n).collect::<String>();
        Text::new(&clip(&self.bank_name(), 38), Point::new(384, 60), MonoTextStyle::new(&SPLEEN_6X12, INK)).draw(f).ok();
        Text::new(&clip(&self.voice_name(), 38), Point::new(384, 80), MonoTextStyle::new(&SPLEEN_6X12, ACCENT)).draw(f).ok();
        // Neighbouring voices, so browsing a 32-voice bank shows where you are.
        if let Some(bank) = self.banks.get(self.bank) {
            for k in 0..9usize {
                let idx = self.voice as i32 - 4 + k as i32;
                if idx < 0 || idx as usize >= bank.voices.len() {
                    continue;
                }
                let style = if idx as usize == self.voice { MonoTextStyle::new(&SPLEEN_6X12, ACCENT) } else { dim };
                Text::new(&clip(&format!("{:>3} {}", idx + 1, bank.voices[idx as usize].name), 38), Point::new(384, 110 + k as i32 * 15), style).draw(f).ok();
            }
        } else {
            Text::new("No banks in dx7_presets/:", Point::new(384, 110), dim).draw(f).ok();
            Text::new("playing the DX7's INIT VOICE", Point::new(384, 125), dim).draw(f).ok();
        }
        let level = self.p.peak.get().clamp(0.0, 1.0).sqrt();
        Rectangle::new(Point::new(384, 290), Size::new(236, 8)).into_styled(PrimitiveStyle::with_stroke(DIM, 1)).draw(f).ok();
        Rectangle::new(Point::new(385, 291), Size::new((level * 234.0) as u32, 6)).into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(f).ok();
        Text::new(&format!("{} voices", self.p.voices.load(Ordering::Relaxed)), Point::new(384, 316), dim).draw(f).ok();
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if self.taken {
            return None;
        }
        self.taken = true;
        let handle = unsafe { dexed_create(self.p.rate.get() as c_double) };
        Some(Box::new(Processor { p: Arc::clone(&self.p), notes: Arc::clone(&self.keys.queue), handle, mono: vec![0.0; CHUNK], last_sustain: false, master_prev: 0.0, sent: false }))
    }
}

const CHUNK: usize = 1024;

/// The voices are summed at full scale, so a chord on a bright patch goes past 1.0 and
/// would clip hard at the device (a crackle, not a tone). Below 0.6 the signal passes
/// untouched; above it, it bends smoothly toward 1.0.
fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.6;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        x.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}

struct Processor {
    p: Arc<Shared>,
    notes: Arc<NoteQueue>,
    handle: *mut c_void,
    mono: Vec<f32>,
    last_sustain: bool,
    /// Last block's master gain, so a level change glides across the block instead of
    /// stepping (a step is a click on a sounding note).
    master_prev: f32,
    /// Whether the app's current voice has reached the engine yet.
    sent: bool,
}

// The engine is only ever touched from the audio thread.
unsafe impl Send for Processor {}

impl Drop for Processor {
    fn drop(&mut self) {
        unsafe { dexed_destroy(self.handle) };
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.0);
        if channels == 0 || !rate.is_finite() || rate < 1000.0 {
            return;
        }
        self.p.rate.set(rate);
        let h = self.handle;
        unsafe { dexed_set_rate(h, rate as c_double) };
        if let Ok(mut slot) = self.p.pending.try_lock() {
            if let Some(v) = slot.take() {
                unsafe {
                    // A new voice ends the old one's notes, as a program change on the hardware.
                    dexed_all_off(h);
                    match &v {
                        VoiceData::Packed(b) => dexed_load_packed_voice(h, b.as_ptr()),
                        VoiceData::Unpacked(b) => dexed_load_unpacked_voice(h, b.as_ptr()),
                    }
                }
                self.sent = true;
            }
        }
        let octave = self.p.octave.load(Ordering::Relaxed);
        while let Some(e) = self.notes.pop() {
            let note = (e.note as i32 + octave * 12).clamp(0, 127);
            unsafe {
                if e.velocity > 0 {
                    dexed_note_on(h, note, e.velocity as c_int);
                } else {
                    dexed_note_off(h, note);
                }
            }
        }
        let bend = (self.p.bend.get() + self.p.cv[1].get() * 2.0).clamp(-1.0, 1.0);
        let wheel = (self.p.modwheel.get() + self.p.cv[2].get()).clamp(0.0, 1.0);
        let pressure = (self.p.pressure.get() + self.p.cv[3].get()).clamp(0.0, 1.0);
        let sustain = self.p.sustain.load(Ordering::Relaxed);
        unsafe {
            dexed_controllers(h, (8192.0 + bend * 8191.0) as c_int, (wheel * 127.0) as c_int, 0, 0, (pressure * 127.0) as c_int, 2);
            if sustain != self.last_sustain {
                dexed_sustain(h, sustain as c_int);
                self.last_sustain = sustain;
            }
        }
        let level = (self.p.level.get() + self.p.cv[0].get()).clamp(0.0, 1.0);
        let master = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0) * level * level * 2.0;
        let frames = out.len() / channels;
        let mut peak = 0.0f32;
        // The bus buffer keeps its capacity between blocks, so refilling it doesn't allocate.
        let mut bus = self.p.output.try_lock().ok();
        if let Some(b) = bus.as_mut() {
            b.clear();
        }
        let master_from = self.master_prev;
        let master_step = (master - master_from) / frames.max(1) as f32;
        self.master_prev = master;
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(CHUNK);
            unsafe { dexed_render(h, self.mono.as_mut_ptr(), n as c_int) };
            for k in 0..n {
                let gain = master_from + master_step * (done + k) as f32;
                let s = soft_limit(self.mono[k] * gain);
                let s = if s.is_finite() { s } else { 0.0 };
                peak = peak.max(s.abs());
                for o in out[(done + k) * channels..(done + k + 1) * channels].iter_mut() {
                    *o = s;
                }
                if let Some(b) = bus.as_mut() {
                    b.push(s);
                }
            }
            done += n;
        }
        self.p.peak.set(peak);
        self.p.voices.store(unsafe { dexed_active_voices(h) }.max(0) as usize, Ordering::Relaxed);
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(DexedApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A packed 128-byte voice with one audible carrier: `algorithm` 0-31,
    /// operator 1 at `level`, the other five silent. Built by hand from the
    /// DX7 SysEx layout, because no voice banks are bundled.
    fn organ_voice(algorithm: u8, level: u8, name: &str) -> [u8; 128] {
        let mut b = [0u8; 128];
        for op in 0..6 {
            let o = &mut b[op * 17..op * 17 + 17];
            o[0..4].copy_from_slice(&[99, 99, 99, 60]); // rates
            o[4..8].copy_from_slice(&[99, 99, 99, 0]); // levels
            o[8] = 39; // break point
            o[12] = 7 << 3; // rate scale 0, detune 7 (centre)
            o[14] = if op == 5 { level } else { 0 }; // output level: DX7 op 1 is stored last
            o[15] = 1 << 1; // coarse 1, ratio mode
        }
        b[102..106].copy_from_slice(&[99, 99, 99, 99]);
        b[106..110].copy_from_slice(&[50, 50, 50, 50]);
        b[110] = algorithm;
        b[117] = 24; // transpose: middle C
        let mut n = [b' '; 10];
        n[..name.len().min(10)].copy_from_slice(&name.as_bytes()[..name.len().min(10)]);
        b[118..128].copy_from_slice(&n);
        b
    }

    fn bulk_dump(voices: &[[u8; 128]]) -> Vec<u8> {
        let mut data = vec![0xF0, 0x43, 0x00, 0x09, 0x20, 0x00];
        for v in voices {
            data.extend_from_slice(v);
        }
        data.resize(6 + 4096, 0);
        data.extend_from_slice(&[0x00, 0xF7]);
        data
    }

    fn dir_with(files: &[(&str, Vec<u8>)]) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("portamax-dexed-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        for (path, data) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, data).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn app(dir: &Path) -> DexedApp {
        DexedApp::with_dir(dir, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
    }

    fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for _ in 0..blocks {
            let mut buf = vec![0.0f32; 512 * 2];
            p.process(&mut buf, 2, 48_000.0);
            assert!(buf.iter().all(|v| v.is_finite()));
            all.extend(buf.iter().step_by(2));
        }
        all
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn hz(x: &[f32]) -> f32 {
        let crossings = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        crossings as f32 / (x.len() as f32 / 48_000.0)
    }

    #[test]
    fn a_bulk_dump_gives_32_named_voices_in_a_named_bank() {
        let dir = dir_with(&[("My Bank/bank.syx", bulk_dump(&[organ_voice(31, 99, "SINE ONE")]))]);
        let a = app(&dir);
        assert_eq!(a.banks.len(), 1);
        assert_eq!(a.banks[0].name, "My Bank");
        assert_eq!(a.banks[0].voices.len(), 32);
        assert_eq!(a.banks[0].voices[0].name, "SINE ONE");
        assert_eq!(a.voice_name(), "SINE ONE");
    }

    #[test]
    fn a_key_plays_a_real_dx7_voice_at_its_pitch_and_releases() {
        let dir = dir_with(&[("bank.syx", bulk_dump(&[organ_voice(31, 99, "SINE")]))]);
        let mut a = app(&dir);
        let mut p = a.audio_processor().unwrap();
        assert!(rms(&render(&mut p, 5)) < 1e-6, "silent before a note");
        let mut input = Input::default();
        input.midi_keys.0[69] = 100;
        a.tick(&input);
        render(&mut p, 4);
        let out = render(&mut p, 40);
        assert!(rms(&out) > 0.02, "a key sounds ({})", rms(&out));
        let f = hz(&out);
        assert!((f - 440.0).abs() < 6.0, "A4 plays at 440 Hz, got {f}");
        input.midi_keys.0[69] = 0;
        a.tick(&input);
        render(&mut p, 120);
        assert!(rms(&render(&mut p, 10)) < 0.005, "the envelope releases");
    }

    #[test]
    fn with_no_banks_it_plays_the_dx7s_power_on_voice() {
        let dir = dir_with(&[]);
        let mut a = app(&dir);
        assert!(a.banks.is_empty());
        assert_eq!(a.voice_name(), "INIT VOICE");
        let mut p = a.audio_processor().unwrap();
        let mut input = Input::default();
        input.midi_keys.0[60] = 100;
        a.tick(&input);
        render(&mut p, 4);
        let out = render(&mut p, 30);
        assert!(rms(&out) > 0.02, "INIT VOICE is audible ({})", rms(&out));
        let f = hz(&out);
        assert!((f - 261.6).abs() < 5.0, "middle C, got {f}");
    }

    /// A big chord on a loud patch never leaves ±1 (it would clip at the device), and
    /// the limiter leaves a quiet note exactly alone.
    #[test]
    fn a_chord_never_clips_and_a_single_note_is_untouched() {
        let dir = dir_with(&[("bank.syx", bulk_dump(&[organ_voice(31, 99, "SINE")]))]);
        let peak = |keys: &[usize], level: f32| {
            let mut a = app(&dir);
            a.p.level.set(level);
            let mut p = a.audio_processor().unwrap();
            let mut input = Input::default();
            for k in keys {
                input.midi_keys.0[*k] = 127;
            }
            a.tick(&input);
            render(&mut p, 60).iter().fold(0.0f32, |m, v| m.max(v.abs()))
        };
        let one = peak(&[69], 0.8);
        assert!(one < 0.6, "one note stays under the knee: {one}");
        let many: Vec<usize> = (0..12).map(|k| 48 + k * 3).collect();
        let loud = peak(&many, 1.0);
        assert!(loud <= 1.0, "twelve notes at full level never exceed full scale: {loud}");
        assert!((soft_limit(0.3) - 0.3).abs() < 1e-6 && soft_limit(5.0) <= 1.0 && soft_limit(-5.0) >= -1.0);
    }

    /// Turning the level knob mid-note glides: no jump from one sample to the next.
    #[test]
    fn a_level_change_does_not_click() {
        let dir = dir_with(&[("bank.syx", bulk_dump(&[organ_voice(31, 99, "SINE")]))]);
        let mut a = app(&dir);
        let mut p = a.audio_processor().unwrap();
        let mut input = Input::default();
        input.midi_keys.0[69] = 100;
        a.tick(&input);
        render(&mut p, 20);
        a.p.level.set(0.3);
        let out = render(&mut p, 4);
        let worst = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.03, "largest step across the level change: {worst}");
    }

    #[test]
    fn browsing_changes_the_voice_that_sounds() {
        let quiet = organ_voice(31, 20, "QUIET");
        let loud = organ_voice(31, 99, "LOUD");
        let mut voices = vec![quiet, loud];
        voices.resize(32, loud);
        let dir = dir_with(&[("b.syx", bulk_dump(&voices))]);
        let mut a = app(&dir);
        let mut p = a.audio_processor().unwrap();
        let mut input = Input::default();
        input.midi_keys.0[60] = 100;
        a.tick(&input);
        render(&mut p, 4);
        let first = rms(&render(&mut p, 30));
        a.kit_edit(C_VOICE, 1);
        assert_eq!(a.voice_name(), "LOUD");
        render(&mut p, 1);
        a.tick(&input);
        input.midi_keys.0[60] = 0;
        a.tick(&input);
        input.midi_keys.0[60] = 100;
        a.tick(&input);
        render(&mut p, 4);
        let second = rms(&render(&mut p, 30));
        assert!(second > first * 3.0, "a louder voice is louder: {first} -> {second}");
        a.kit_edit(C_VOICE, -1);
        a.kit_edit(C_VOICE, -1);
        assert_eq!(a.voice, 31, "wraps backwards");
    }

    #[test]
    fn all_32_algorithms_make_sound_and_stay_bounded() {
        for alg in 0..32u8 {
            let dir = dir_with(&[("b.syx", bulk_dump(&[organ_voice(alg, 99, "ALG")]))]);
            let mut a = app(&dir);
            let mut p = a.audio_processor().unwrap();
            let mut input = Input::default();
            input.midi_keys.0[60] = 100;
            a.tick(&input);
            render(&mut p, 2);
            let out = render(&mut p, 20);
            assert!(rms(&out) > 0.005 && out.iter().all(|v| v.abs() <= 2.0), "algorithm {}: {}", alg + 1, rms(&out));
        }
    }

    #[test]
    fn bend_moves_the_pitch_by_the_wheel_range() {
        let dir = dir_with(&[("b.syx", bulk_dump(&[organ_voice(31, 99, "SINE")]))]);
        let mut a = app(&dir);
        let mut p = a.audio_processor().unwrap();
        let mut input = Input::default();
        input.midi_keys.0[69] = 100;
        a.tick(&input);
        render(&mut p, 6);
        let base = hz(&render(&mut p, 40));
        a.kit_set_norm(C_BEND, 1.0);
        render(&mut p, 4);
        let bent = hz(&render(&mut p, 40));
        assert!((bent / base - 1.1225).abs() < 0.02, "a full bend is two semitones up ({base} -> {bent})");
    }

    #[test]
    fn the_real_syx_in_the_repo_loads_and_every_voice_plays() {
        let dir = Path::new(DX7_PRESETS_DIR);
        let banks = scan_banks(dir);
        if banks.is_empty() {
            return; // no banks checked out
        }
        let voices: usize = banks.iter().map(|b| b.voices.len()).sum();
        assert!(voices >= 1);
        let mut a = DexedApp::with_dir(dir, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()));
        let mut p = a.audio_processor().unwrap();
        for v in 0..a.voices_in_bank() {
            a.voice = v;
            a.send_voice();
            let mut input = Input::default();
            input.midi_keys.0[57] = 100;
            input.midi_keys.0[64] = 100;
            a.tick(&input);
            render(&mut p, 2);
            let out = render(&mut p, 12);
            assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0), "voice {} ({}) is finite and never clips", v + 1, a.voice_name());
            input.midi_keys.0[57] = 0;
            input.midi_keys.0[64] = 0;
            a.tick(&input);
            render(&mut p, 2);
        }
    }

    #[test]
    fn its_knobs_are_mod_inputs_and_it_opens_on_the_play_view() {
        let dir = dir_with(&[]);
        let modbus = Arc::new(ModBus::new());
        let a = DexedApp::with_dir(&dir, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::clone(&modbus), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()));
        for name in ["Dexed: Level", "Dexed: Bend", "Dexed: Mod", "Dexed: Pressure", "Mixer: Dexed Level"] {
            assert!(modbus.index_of(name).is_some(), "{name}");
        }
        assert!(a.play_column().is_some());
    }
}
