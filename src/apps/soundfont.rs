//! SoundFont: plays SF2/SF3 banks with TinySoundFont (vendor/tinysoundfont,
//! MIT, (c) Bernhard Schelling) -- the real sample-playback engine, with the
//! bank's own envelopes, loops, key and velocity zones, filters and
//! modulators, not an imitation of them.
//!
//! Banks come from `soundfonts/` (see its README; nothing is committed). Pick
//! a Soundfont, then browse its presets with the D-pad; the pads play the
//! preset chromatically from C3 and any keyboard or app on the note bus plays
//! it too. Bend is a pitch wheel (two semitones each way), so the stick and a
//! hand bend notes; Pan, Level, Octave and Sustain are the rest of the panel.
//!
//! A bank is loaded on the UI thread, and handed to the audio thread through a
//! try_lock slot with the old one handed back the same way, so the audio
//! callback never loads, allocates or frees. Voices are allocated up front
//! (`MAX_VOICES`); past that a new note takes the voice furthest into its
//! release, or is dropped, as TinySoundFont does.

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
use std::ffi::{c_char, c_float, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "SoundFont";

/// Where banks live, anchored at the crate root like `samples/` and
/// `dx7_presets/`.
const SOUNDFONT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/soundfonts");

/// A chord with a long release plus a held pedal can use a lot of voices.
const MAX_VOICES: c_int = 48;
const CHUNK: usize = 1024;

// Own palette: lacquered black and ivory with a brass accent.
const BG: Rgb565 = Rgb565::new(2, 4, 4);
const INK: Rgb565 = Rgb565::new(29, 56, 26);
const ACCENT: Rgb565 = Rgb565::new(27, 44, 6);
const DIM: Rgb565 = Rgb565::new(13, 26, 14);
const FAINT: Rgb565 = Rgb565::new(4, 9, 8);

#[allow(non_camel_case_types)]
type tsf = c_void;

/// TSFOutputMode::TSF_STEREO_INTERLEAVED.
const TSF_STEREO_INTERLEAVED: c_int = 0;

unsafe extern "C" {
    fn tsf_load_filename(filename: *const c_char) -> *mut tsf;
    fn tsf_close(f: *mut tsf);
    fn tsf_get_presetcount(f: *const tsf) -> c_int;
    fn tsf_get_presetname(f: *const tsf, preset: c_int) -> *const c_char;
    fn tsf_set_output(f: *mut tsf, mode: c_int, samplerate: c_int, gain_db: c_float);
    fn tsf_set_max_voices(f: *mut tsf, n: c_int) -> c_int;
    fn tsf_active_voice_count(f: *mut tsf) -> c_int;
    fn tsf_render_float(f: *mut tsf, buffer: *mut c_float, samples: c_int, mixing: c_int);
    fn tsf_channel_set_presetindex(f: *mut tsf, channel: c_int, preset: c_int) -> c_int;
    fn tsf_channel_set_pan(f: *mut tsf, channel: c_int, pan: c_float) -> c_int;
    fn tsf_channel_set_volume(f: *mut tsf, channel: c_int, volume: c_float) -> c_int;
    fn tsf_channel_set_pitchwheel(f: *mut tsf, channel: c_int, wheel: c_int) -> c_int;
    fn tsf_channel_set_pitchrange(f: *mut tsf, channel: c_int, range: c_float) -> c_int;
    fn tsf_channel_set_sustain(f: *mut tsf, channel: c_int, on: c_int) -> c_int;
    fn tsf_channel_note_on(f: *mut tsf, channel: c_int, key: c_int, vel: c_float) -> c_int;
    fn tsf_channel_note_off(f: *mut tsf, channel: c_int, key: c_int);
}

/// A loaded bank. Everything that allocates happens in `load`.
struct Bank {
    f: *mut tsf,
    presets: usize,
    /// The preset the audio thread last selected, so it only changes on a change.
    selected: i32,
}

// A bank is only ever used by whoever holds the box.
unsafe impl Send for Bank {}

impl Bank {
    fn load(path: &Path, rate: i32) -> Option<Bank> {
        let c = CString::new(path.to_str()?).ok()?;
        let f = unsafe { tsf_load_filename(c.as_ptr()) };
        if f.is_null() {
            return None;
        }
        let presets = unsafe { tsf_get_presetcount(f) }.max(0) as usize;
        unsafe {
            tsf_set_output(f, TSF_STEREO_INTERLEAVED, rate, 0.0);
            tsf_set_max_voices(f, MAX_VOICES);
            // Channel 0 is created here, off the audio thread, not on its first note.
            tsf_channel_set_volume(f, 0, 1.0);
            tsf_channel_set_pitchrange(f, 0, 2.0);
            tsf_channel_set_presetindex(f, 0, 0);
        }
        Some(Bank { f, presets, selected: 0 })
    }

    fn preset_names(&self) -> Vec<String> {
        (0..self.presets)
            .map(|i| {
                let p = unsafe { tsf_get_presetname(self.f, i as c_int) };
                if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().trim().to_string() }
            })
            .collect()
    }
}

impl Drop for Bank {
    fn drop(&mut self) {
        unsafe { tsf_close(self.f) };
    }
}

/// `.sf2` / `.sf3` files in `dir`, sorted by name.
fn scan(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("sf2") || e.eq_ignore_ascii_case("sf3")))
        .collect();
    found.sort_by_key(|p| p.file_name().map(|n| n.to_ascii_lowercase()));
    found
}

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "soundfont",
        layers: vec![Layer::Native(0, "KEYS"), Layer::Controls, Layer::Moments],
        hero: vec![[C_PRESET, C_LEVEL], [C_BEND, C_PAN], [C_OCTAVE, C_SUSTAIN], [C_FONT, C_LEVEL]],
        browse: Some(C_PRESET),
        routes: Routes { stick_x: Some(C_BEND), stick_y: Some(C_PAN), hand_l: Some(C_LEVEL), hand_r: None },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: false,
    }
}

const C_PRESET: usize = 0;
const C_FONT: usize = 1;
const C_LEVEL: usize = 2;
const C_BEND: usize = 3;
const C_PAN: usize = 4;
const C_OCTAVE: usize = 5;
const C_SUSTAIN: usize = 6;
const N_CONTROLS: usize = 7;

struct Shared {
    /// Chosen preset index in the loaded bank.
    preset: AtomicUsize,
    level: AtomicF32,
    /// -1..1, two semitones each way.
    bend: AtomicF32,
    pan: AtomicF32,
    octave: AtomicI32,
    sustain: AtomicBool,
    cv: [Arc<AtomicF32>; 3],
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    voices: AtomicUsize,
    peak: AtomicF32,
    rate: AtomicF32,
    incoming: Mutex<Option<Box<Bank>>>,
    retired: Mutex<Vec<Box<Bank>>>,
}

pub struct SoundFontApp {
    p: Arc<Shared>,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
    keys: Keys,
    files: Vec<PathBuf>,
    font: usize,
    preset_names: Vec<String>,
    loaded: Option<String>,
    status: String,
}

impl SoundFontApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        Self::with_dir(Path::new(SOUNDFONT_DIR), sensitivity, nav, mods, bus, mixer)
    }

    pub fn with_dir(dir: &Path, sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let mut app = Self {
            p: Arc::new(Shared {
                preset: AtomicUsize::new(0),
                level: AtomicF32::new(0.8),
                bend: AtomicF32::new(0.0),
                pan: AtomicF32::new(0.5),
                octave: AtomicI32::new(0),
                sustain: AtomicBool::new(false),
                cv: [mods.register(format!("{APP_NAME}: Level")), mods.register(format!("{APP_NAME}: Bend")), mods.register(format!("{APP_NAME}: Pan"))],
                mix_level,
                ext_mix_level,
                output,
                voices: AtomicUsize::new(0),
                peak: AtomicF32::new(0.0),
                rate: AtomicF32::new(48_000.0),
                incoming: Mutex::new(None),
                retired: Mutex::new(Vec::new()),
            }),
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            keys: Keys::default(),
            files: scan(dir),
            font: 0,
            preset_names: Vec::new(),
            loaded: None,
            status: String::new(),
        };
        app.load_font(0);
        app
    }

    fn level(&self) -> f32 {
        (self.p.level.get() + self.p.cv[0].get()).clamp(0.0, 1.0)
    }
    fn bend(&self) -> f32 {
        (self.p.bend.get() + self.p.cv[1].get() * 2.0).clamp(-1.0, 1.0)
    }
    fn pan(&self) -> f32 {
        (self.p.pan.get() + self.p.cv[2].get()).clamp(0.0, 1.0)
    }

    /// Loads the bank at `index` on this (UI) thread and queues it for the audio thread.
    fn load_font(&mut self, index: usize) {
        let Some(path) = self.files.get(index).cloned() else {
            self.status = format!("Put .sf2 files in {}", SOUNDFONT_DIR);
            return;
        };
        let rate = self.p.rate.get() as i32;
        match Bank::load(&path, rate) {
            Some(bank) => {
                self.preset_names = bank.preset_names();
                self.loaded = path.file_stem().map(|s| s.to_string_lossy().into_owned());
                self.status.clear();
                self.font = index;
                self.p.preset.store(0, Ordering::Relaxed);
                if let Ok(mut slot) = self.p.incoming.lock() {
                    *slot = Some(Box::new(bank));
                }
            }
            None => self.status = format!("Could not read {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        }
    }

    fn preset(&self) -> usize {
        self.p.preset.load(Ordering::Relaxed).min(self.preset_names.len().saturating_sub(1))
    }

    fn preset_name(&self) -> String {
        self.preset_names.get(self.preset()).cloned().unwrap_or_else(|| "(no bank)".into())
    }

    fn font_name(&self) -> String {
        self.loaded.clone().unwrap_or_else(|| "(none)".into())
    }

    fn step_preset(&mut self, d: i32) {
        let n = self.preset_names.len() as i32;
        if n > 0 && d != 0 {
            self.p.preset.store((self.preset() as i32 + d.signum()).rem_euclid(n) as usize, Ordering::Relaxed);
        }
    }

    fn step_font(&mut self, d: i32) {
        let n = self.files.len() as i32;
        if n > 1 && d != 0 {
            self.load_font((self.font as i32 + d.signum()).rem_euclid(n) as usize);
        }
    }

    fn edit_continuous(&mut self, i: usize, d: i32) {
        let step = d as f32 * 0.01 * self.sensitivity.get().max(0.01) * 10.0;
        match i {
            C_LEVEL => self.p.level.set((self.p.level.get() + step).clamp(0.0, 1.0)),
            C_BEND => self.p.bend.set((self.p.bend.get() + step * 2.0).clamp(-1.0, 1.0)),
            C_PAN => self.p.pan.set((self.p.pan.get() + step).clamp(0.0, 1.0)),
            _ => {}
        }
    }

    fn text(&self, i: usize) -> (String, String) {
        match i {
            C_PRESET => ("Preset".into(), if self.preset_names.is_empty() { "(no bank)".into() } else { format!("{} {}/{}", self.preset_name(), self.preset() + 1, self.preset_names.len()) }),
            C_FONT => ("Soundfont".into(), if self.files.is_empty() { "(none in soundfonts/)".into() } else { format!("{} {}/{}", self.font_name(), self.font + 1, self.files.len()) }),
            C_LEVEL => ("Level".into(), format!("{:.0}%", self.level() * 100.0)),
            C_BEND => ("Bend".into(), format!("{:+.1} st", self.bend() * 2.0)),
            C_PAN => ("Pan".into(), match self.pan() {
                p if (p - 0.5).abs() < 0.02 => "center".into(),
                p if p < 0.5 => format!("L{:.0}", (0.5 - p) * 200.0),
                p => format!("R{:.0}", (p - 0.5) * 200.0),
            }),
            C_OCTAVE => ("Octave".into(), format!("{:+}", self.p.octave.load(Ordering::Relaxed))),
            _ => ("Sustain".into(), if self.p.sustain.load(Ordering::Relaxed) { "on" } else { "off" }.into()),
        }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        (0..N_CONTROLS).map(|i| {
            let (n, v) = self.text(i);
            (n, v, false)
        }).collect()
    }

    fn octave(&self) -> i32 {
        self.p.octave.load(Ordering::Relaxed)
    }
}

impl PlayHost for SoundFontApp {
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
            C_PRESET => if self.preset_names.len() > 1 { self.preset() as f32 / (self.preset_names.len() - 1) as f32 } else { 0.0 },
            C_FONT => if self.files.len() > 1 { self.font as f32 / (self.files.len() - 1) as f32 } else { 0.0 },
            C_LEVEL => self.p.level.get(),
            C_BEND => (self.p.bend.get() + 1.0) / 2.0,
            C_PAN => self.p.pan.get(),
            C_OCTAVE => (self.octave() + 3) as f32 / 6.0,
            C_SUSTAIN => self.p.sustain.load(Ordering::Relaxed) as u8 as f32,
            _ => return None,
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        matches!(i, C_PRESET | C_FONT | C_OCTAVE | C_SUSTAIN)
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match i {
            C_PRESET => self.step_preset(delta),
            C_FONT => self.step_font(delta),
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
            C_PRESET => self.p.preset.store(0, Ordering::Relaxed),
            C_LEVEL => self.p.level.set(0.8),
            C_BEND => self.p.bend.set(0.0),
            C_PAN => self.p.pan.set(0.5),
            C_OCTAVE => self.p.octave.store(0, Ordering::Relaxed),
            C_SUSTAIN => self.p.sustain.store(false, Ordering::Relaxed),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i {
            C_PRESET => {
                if !self.preset_names.is_empty() {
                    self.p.preset.store((v * (self.preset_names.len() - 1) as f32).round() as usize, Ordering::Relaxed);
                }
            }
            C_FONT => {
                if self.files.len() > 1 {
                    let target = (v * (self.files.len() - 1) as f32).round() as usize;
                    if target != self.font {
                        self.load_font(target);
                    }
                }
            }
            C_LEVEL => self.p.level.set(v),
            C_BEND => self.p.bend.set(v * 2.0 - 1.0),
            C_PAN => self.p.pan.set(v),
            C_OCTAVE => self.p.octave.store((v * 6.0).round() as i32 - 3, Ordering::Relaxed),
            C_SUSTAIN => self.p.sustain.store(v >= 0.5, Ordering::Relaxed),
            _ => {}
        }
    }
    fn kit_line(&self) -> String {
        if !self.status.is_empty() {
            self.status.clone()
        } else {
            format!("{} · {}", self.font_name(), self.preset_name())
        }
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

impl App for SoundFontApp {
    fn instrument_settings(&self) -> Vec<crate::app::Setting> {
        crate::app::play_kit::settings_of(self)
    }
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        crate::app::play_kit::adjust_in(self, index, delta)
    }
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
    fn needs_background_audio(&self) -> bool {
        false
    }
    fn tick(&mut self, input: &Input) {
        if let Ok(mut r) = self.p.retired.try_lock() {
            r.clear();
        }
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
        let ink = MonoTextStyle::new(&SPLEEN_6X12, INK);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        let clip = |s: &str, n: usize| s.chars().take(n).collect::<String>();
        Text::new(&clip(&self.font_name(), 38), Point::new(384, 60), ink).draw(f).ok();
        Text::new(&clip(&self.preset_name(), 38), Point::new(384, 80), MonoTextStyle::new(&SPLEEN_6X12, ACCENT)).draw(f).ok();
        if !self.status.is_empty() {
            Text::new(&clip(&self.status, 38), Point::new(384, 110), dim).draw(f).ok();
        }
        // Neighbouring presets, so browsing shows where you are in the bank.
        let sel = self.preset();
        for k in 0..9usize {
            let idx = sel as i32 - 4 + k as i32;
            if idx < 0 || idx as usize >= self.preset_names.len() {
                continue;
            }
            let y = 130 + k as i32 * 15;
            let style = if idx as usize == sel { MonoTextStyle::new(&SPLEEN_6X12, ACCENT) } else { dim };
            Text::new(&clip(&format!("{:>3} {}", idx + 1, self.preset_names[idx as usize]), 38), Point::new(384, y), style).draw(f).ok();
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
        Some(Box::new(Processor { p: Arc::clone(&self.p), notes: Arc::clone(&self.keys.queue), bank: None, stash: None, buf: [0.0; CHUNK * 2], last_sustain: false }))
    }
}

struct Processor {
    p: Arc<Shared>,
    notes: Arc<NoteQueue>,
    bank: Option<Box<Bank>>,
    /// A finished bank the retired slot had no room for yet.
    stash: Option<Box<Bank>>,
    buf: [f32; CHUNK * 2],
    last_sustain: bool,
}

// The bank is only ever touched from the audio thread once it is here.
unsafe impl Send for Processor {}

impl Processor {
    fn collect_new_bank(&mut self, rate: f32) {
        if let Some(old) = self.stash.take() {
            match self.p.retired.try_lock() {
                Ok(mut r) => r.push(old),
                Err(_) => self.stash = Some(old),
            }
        }
        if self.stash.is_some() {
            return;
        }
        let Ok(mut slot) = self.p.incoming.try_lock() else { return };
        if let Some(mut new) = slot.take() {
            // The bank was prepared at whatever rate the UI guessed.
            unsafe { tsf_set_output(new.f, TSF_STEREO_INTERLEAVED, rate as c_int, 0.0) };
            new.selected = -1;
            self.stash = self.bank.replace(new);
        }
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.0);
        if channels == 0 || !rate.is_finite() || rate < 1000.0 {
            return;
        }
        self.p.rate.set(rate);
        self.collect_new_bank(rate);
        let frames = out.len() / channels;
        let Some(bank) = self.bank.as_mut() else {
            while self.notes.pop().is_some() {}
            if let Ok(mut b) = self.p.output.try_lock() {
                b.clear();
                b.resize(frames, 0.0);
            }
            return;
        };
        let f = bank.f;
        let want = self.p.preset.load(Ordering::Relaxed).min(bank.presets.saturating_sub(1)) as i32;
        if want != bank.selected && bank.presets > 0 {
            unsafe {
                // A changed preset ends what the old one was playing, as a program change would.
                tsf_channel_set_presetindex(f, 0, want);
            }
            bank.selected = want;
        }
        let octave = self.p.octave.load(Ordering::Relaxed);
        while let Some(e) = self.notes.pop() {
            let key = (e.note as i32 + octave * 12).clamp(0, 127);
            unsafe {
                if e.velocity > 0 {
                    tsf_channel_note_on(f, 0, key, e.velocity as f32 / 127.0);
                } else {
                    tsf_channel_note_off(f, 0, key);
                }
            }
        }
        let level = (self.p.level.get() + self.p.cv[0].get()).clamp(0.0, 1.0);
        let bend = (self.p.bend.get() + self.p.cv[1].get() * 2.0).clamp(-1.0, 1.0);
        let pan = (self.p.pan.get() + self.p.cv[2].get()).clamp(0.0, 1.0);
        let sustain = self.p.sustain.load(Ordering::Relaxed);
        unsafe {
            // Channel volume, not the global gain: the global gain only reaches
            // notes struck afterwards, the channel's moves sounding ones too.
            // Never exactly 0 (that is -inf dB).
            tsf_channel_set_volume(f, 0, (level * level * 2.0).max(1e-4));
            tsf_channel_set_pitchwheel(f, 0, (8192.0 + bend * 8191.0) as c_int);
            tsf_channel_set_pan(f, 0, pan);
            if sustain != self.last_sustain {
                tsf_channel_set_sustain(f, 0, sustain as c_int);
                self.last_sustain = sustain;
            }
        }
        let master = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0);
        let mut peak = 0.0f32;
        // The bus buffer keeps its capacity between blocks, so refilling it doesn't allocate.
        let mut bus = self.p.output.try_lock().ok();
        if let Some(b) = bus.as_mut() {
            b.clear();
        }
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(CHUNK);
            unsafe { tsf_render_float(f, self.buf.as_mut_ptr(), n as c_int, 0) };
            for k in 0..n {
                let (l, r) = (self.buf[k * 2] * master, self.buf[k * 2 + 1] * master);
                let (l, r) = (if l.is_finite() { l } else { 0.0 }, if r.is_finite() { r } else { 0.0 });
                peak = peak.max(l.abs().max(r.abs()));
                let frame = &mut out[(done + k) * channels..(done + k + 1) * channels];
                match frame {
                    [a, b, ..] => {
                        *a = l;
                        *b = r;
                    }
                    [a] => *a = (l + r) * 0.5,
                    [] => {}
                }
                if let Some(b) = bus.as_mut() {
                    b.push((l + r) * 0.5);
                }
            }
            done += n;
        }
        self.p.peak.set(peak);
        self.p.voices.store(unsafe { tsf_active_voice_count(f) }.max(0) as usize, Ordering::Relaxed);
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(SoundFontApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete, minimal SoundFont 2 file: one looped sine sample (root key
    /// 69, so key 69 plays it untransposed) and `presets` identical presets.
    /// Built here, byte by byte, because the real banks are user-supplied and
    /// never committed.
    pub fn tiny_sf2(presets: usize, hz: f32) -> Vec<u8> {
        fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut v = id.to_vec();
            v.extend((body.len() as u32).to_le_bytes());
            v.extend(body);
            if body.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        fn list(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut inner = kind.to_vec();
            inner.extend(body);
            chunk(b"LIST", &inner)
        }
        fn name(s: &str) -> [u8; 20] {
            let mut n = [0u8; 20];
            n[..s.len().min(19)].copy_from_slice(&s.as_bytes()[..s.len().min(19)]);
            n
        }
        let rate = 44_100u32;
        // One cycle's worth of whole periods, so the loop is seamless.
        let cycles = 20usize;
        let len = ((rate as f32 / hz) * cycles as f32).round() as usize;
        let mut smpl = Vec::new();
        for i in 0..len {
            let s = ((i as f32 / len as f32 * cycles as f32 * std::f32::consts::TAU).sin() * 20_000.0) as i16;
            smpl.extend(s.to_le_bytes());
        }
        smpl.extend(vec![0u8; 46 * 2]);

        let mut phdr = Vec::new();
        for i in 0..presets {
            phdr.extend(name(&format!("Test {}", i + 1)));
            phdr.extend((i as u16).to_le_bytes()); // preset number
            phdr.extend(0u16.to_le_bytes()); // bank
            phdr.extend((i as u16).to_le_bytes()); // bag index
            phdr.extend([0u8; 12]);
        }
        phdr.extend(name("EOP"));
        phdr.extend([0u8; 4]);
        phdr.extend((presets as u16).to_le_bytes());
        phdr.extend([0u8; 12]);
        let mut pbag = Vec::new();
        for i in 0..=presets {
            pbag.extend((i as u16).to_le_bytes());
            pbag.extend(0u16.to_le_bytes());
        }
        let pmod = vec![0u8; 10];
        let mut pgen = Vec::new();
        for _ in 0..presets {
            pgen.extend(41u16.to_le_bytes()); // instrument
            pgen.extend(0u16.to_le_bytes());
        }
        pgen.extend([0u8; 4]);
        let mut inst = name("Sine").to_vec();
        inst.extend(0u16.to_le_bytes());
        inst.extend(name("EOI"));
        inst.extend(1u16.to_le_bytes());
        let mut ibag = Vec::new();
        ibag.extend(0u16.to_le_bytes());
        ibag.extend(0u16.to_le_bytes());
        ibag.extend(2u16.to_le_bytes());
        ibag.extend(0u16.to_le_bytes());
        let imod = vec![0u8; 10];
        let mut igen = Vec::new();
        igen.extend(54u16.to_le_bytes()); // sampleModes
        igen.extend(1u16.to_le_bytes()); // loop continuously
        igen.extend(53u16.to_le_bytes()); // sampleID
        igen.extend(0u16.to_le_bytes());
        igen.extend([0u8; 4]);
        let record = |label: &str, start: u32, end: u32, loop_start: u32, loop_end: u32, pitch: u8, kind: u16| {
            let mut r = name(label).to_vec();
            for v in [start, end, loop_start, loop_end, rate] {
                r.extend(v.to_le_bytes());
            }
            r.push(pitch);
            r.push(0); // pitch correction
            r.extend(0u16.to_le_bytes()); // sample link
            r.extend(kind.to_le_bytes());
            r
        };
        let mut shdr = record("sine", 0, len as u32, 0, len as u32, 69, 1);
        shdr.extend(record("EOS", 0, 0, 0, 0, 0, 0));

        let mut info = chunk(b"ifil", &[2, 0, 1, 0]);
        info.extend(chunk(b"isng", b"EMU8000\0"));
        info.extend(chunk(b"INAM", b"Portamax test\0\0\0"));
        let sdta = chunk(b"smpl", &smpl);
        let mut pdta = Vec::new();
        for (id, body) in [(b"phdr", &phdr), (b"pbag", &pbag), (b"pmod", &pmod), (b"pgen", &pgen), (b"inst", &inst), (b"ibag", &ibag), (b"imod", &imod), (b"igen", &igen)] {
            pdta.extend(chunk(id, body));
        }
        pdta.extend(chunk(b"shdr", &shdr));
        let mut body = b"sfbk".to_vec();
        body.extend(list(b"INFO", &info));
        body.extend(list(b"sdta", &sdta));
        body.extend(list(b"pdta", &pdta));
        chunk(b"RIFF", &body)
    }

    fn dir_with_fonts(names: &[(&str, usize)]) -> PathBuf {
        use std::sync::atomic::AtomicUsize;
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("portamax-sf-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        for (n, presets) in names {
            std::fs::write(dir.join(n), tiny_sf2(*presets, 440.0)).unwrap();
        }
        dir
    }

    fn app(dir: &Path) -> SoundFontApp {
        SoundFontApp::with_dir(dir, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
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

    /// Frequency of a sine by counting rising zero crossings over the whole span.
    fn hz(x: &[f32]) -> f32 {
        let crossings = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        crossings as f32 / (x.len() as f32 / 48_000.0)
    }

    #[test]
    fn a_bank_loads_names_its_presets_and_a_key_plays_at_its_pitch() {
        let dir = dir_with_fonts(&[("a.sf2", 3)]);
        let mut a = app(&dir);
        assert_eq!(a.preset_names, vec!["Test 1", "Test 2", "Test 3"]);
        let mut p = a.audio_processor().unwrap();
        assert!(rms(&render(&mut p, 5)) < 1e-6, "silent before a note");
        let mut input = Input::default();
        input.midi_keys.0[69] = 100;
        a.tick(&input);
        render(&mut p, 4);
        let out = render(&mut p, 40);
        assert!(rms(&out) > 0.02, "a key sounds ({})", rms(&out));
        let f = hz(&out);
        assert!((f - 440.0).abs() < 8.0, "A4 plays at 440 Hz, got {f}");
        input.midi_keys.0[69] = 0;
        a.tick(&input);
        render(&mut p, 80);
        assert!(rms(&render(&mut p, 10)) < 0.01, "and goes quiet when released");
    }

    #[test]
    fn octave_bend_and_level_move_the_sound() {
        let dir = dir_with_fonts(&[("a.sf2", 1)]);
        let mut a = app(&dir);
        let mut p = a.audio_processor().unwrap();
        let mut input = Input::default();
        input.midi_keys.0[69] = 100;
        a.tick(&input);
        render(&mut p, 4);
        let base = render(&mut p, 40);
        a.kit_set_norm(C_BEND, 1.0); // +2 semitones
        render(&mut p, 4);
        let bent = render(&mut p, 40);
        let ratio = hz(&bent) / hz(&base);
        assert!((ratio - 1.1225).abs() < 0.03, "a full bend is two semitones up ({ratio})");
        a.kit_set_norm(C_BEND, 0.5);
        a.kit_set_norm(C_LEVEL, 0.2);
        render(&mut p, 4);
        let quiet = render(&mut p, 40);
        assert!(rms(&quiet) < rms(&base) * 0.5, "level turns it down");
    }

    #[test]
    fn presets_step_and_wrap_and_banks_switch_without_dropping_the_audio_thread() {
        let dir = dir_with_fonts(&[("a.sf2", 3), ("b.sf2", 5)]);
        let mut a = app(&dir);
        let mut p = a.audio_processor().unwrap();
        render(&mut p, 2);
        a.kit_edit(C_PRESET, -1);
        assert_eq!(a.preset_name(), "Test 3", "wraps backwards");
        a.kit_edit(C_FONT, 1);
        assert_eq!(a.preset_names.len(), 5, "the second bank's presets");
        assert_eq!(a.preset(), 0, "a new bank starts at its first preset");
        render(&mut p, 2);
        assert!(!a.p.retired.lock().unwrap().is_empty(), "the old bank was handed back, not freed in the callback");
        a.tick(&Input::default());
        assert!(a.p.retired.lock().unwrap().is_empty(), "the UI frees it");
    }

    #[test]
    fn with_no_banks_it_says_so_and_stays_silent() {
        let dir = dir_with_fonts(&[]);
        let mut a = app(&dir);
        assert!(a.status.contains("soundfonts"), "{}", a.status);
        let mut p = a.audio_processor().unwrap();
        let mut input = Input::default();
        input.midi_keys.0[60] = 100;
        a.tick(&input);
        assert!(rms(&render(&mut p, 10)) < 1e-9);
    }

    #[test]
    fn a_file_that_is_not_a_soundfont_is_reported_not_trusted() {
        let dir = dir_with_fonts(&[]);
        std::fs::write(dir.join("junk.sf2"), b"not a soundfont at all").unwrap();
        let a = app(&dir);
        assert!(a.status.contains("Could not read"), "{}", a.status);
        assert!(a.preset_names.is_empty());
    }

    #[test]
    fn its_knobs_are_mod_inputs_and_it_opens_on_the_play_view() {
        let dir = dir_with_fonts(&[("a.sf2", 1)]);
        let modbus = Arc::new(ModBus::new());
        let a = SoundFontApp::with_dir(&dir, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::clone(&modbus), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()));
        for name in ["SoundFont: Level", "SoundFont: Bend", "SoundFont: Pan", "Mixer: SoundFont Level"] {
            assert!(modbus.index_of(name).is_some(), "{name}");
        }
        assert!(a.play_column().is_some());
    }
}
