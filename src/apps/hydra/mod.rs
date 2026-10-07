//! Hydra: a hybrid polyphonic synthesizer, built to be the one instrument
//! that can make almost any sound.
//!
//! **Voices.** Sixteen voices, each with three oscillators that can be
//! analog (PolyBLEP saw, pulse, triangle, sine, morphing, with a wavefolder),
//! wavetable (six banks of sixteen frames, built by inverse FFT and
//! band-limited per octave), two-operator FM with feedback, Karplus-Strong
//! plucked string or noise (white to brown, crackle, or tuned). Analog and
//! wavetable oscillators take up to seven unison voices with detune and
//! stereo spread. Each voice also has a sub oscillator, ring modulation of A
//! by B, hard sync of B to A and cross modulation of A into B.
//!
//! **Filters.** A state-variable filter (LP12, LP24, HP12, HP24, band, notch,
//! peak, or a tuned comb) and a four-pole ladder (LP24, LP12, band, HP24)
//! that can run in series or in parallel, each with drive, key tracking and
//! envelope amount.
//!
//! **Modulation.** Three envelopes, two LFOs (six shapes each, free or
//! key-synced, with fade-in), and a twelve-slot matrix from fourteen sources
//! to thirty-one targets, plus four macros.
//!
//! **Effects.** Drive (soft, hard, fold, crush), chorus, phaser, ping-pong
//! delay and a feedback-delay-network reverb.
//!
//! **Playing.** Poly, mono or legato with glide, an arpeggiator, scale-aware
//! pads, MIDI keys and notes from any app over the note bus. Twenty-six
//! factory sounds, Randomize and Mutate, sixteen user slots and the play
//! kit's Moments.
//!
//! Every setting lives in one table (params.rs), so the menu, the play-view
//! dials, the settings other apps list under their "Plays" row, the
//! modulation inputs and Moments all come from it.

pub mod dsp;
pub mod engine;
pub mod fx;
pub mod params;
pub mod presets;
pub mod store;
pub mod tables;
pub mod voice;

use crate::app::music_scales::{self, ROOT_NAMES, SCALE_TYPES};
use crate::app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes};
use crate::app::{App, Input};
use crate::apps::mi_kit::{Keys, NoteQueue};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::paramlist::ParamList;
use crate::spleen_fonts::{SPLEEN_6X12, SPLEEN_8X16};
use crate::util::{note_name, AtomicF32};
use dsp::Rng;
use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::Text,
};
use engine::{Cv, Engine, Shared, MAX_VOICES};
use params::*;
use presets::Preset;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use store::Params;

const APP_NAME: &str = "Hydra";

// Own palette: deep ocean with a hot magenta accent.
const BG: Rgb565 = Rgb565::new(1, 4, 7);
const INK: Rgb565 = Rgb565::new(24, 60, 28);
const ACCENT: Rgb565 = Rgb565::new(31, 22, 24);
const DIM: Rgb565 = Rgb565::new(10, 26, 22);
const FAINT: Rgb565 = Rgb565::new(4, 10, 12);
const CHIP: Rgb565 = Rgb565::new(5, 14, 16);

/// What a play-view control is: a table parameter, or the preset browser.
#[derive(Clone, Copy)]
enum Ctl {
    Param(P),
    Preset,
}

/// The play view's controls, most important first (the first ones land on
/// the knobs and the bottom pads), with the short name each is shown under.
/// Other apps list these under their own "Plays" row too.
const KIT: [(Ctl, &str); 20] = [
    (Ctl::Param(P::F1_Cut), "Cutoff"),
    (Ctl::Param(P::F1_Res), "Resonance"),
    (Ctl::Preset, "Preset"),
    (Ctl::Param(P::Level), "Level"),
    (Ctl::Param(P::F1_Env), "Filter env"),
    (Ctl::Param(P::Drv_Amt), "Drive"),
    (Ctl::Param(P::AmpA), "Attack"),
    (Ctl::Param(P::AmpR), "Release"),
    (Ctl::Param(P::A_P1), "Osc A morph"),
    (Ctl::Param(P::B_P1), "Osc B morph"),
    (Ctl::Param(P::UniDetune), "Detune"),
    (Ctl::Param(P::L1_Rate), "LFO rate"),
    (Ctl::Param(P::Mac1), "Macro 1"),
    (Ctl::Param(P::Mac2), "Macro 2"),
    (Ctl::Param(P::Mac3), "Macro 3"),
    (Ctl::Param(P::Mac4), "Macro 4"),
    (Ctl::Param(P::Dly_Mix), "Delay"),
    (Ctl::Param(P::Rev_Mix), "Reverb"),
    (Ctl::Param(P::Cho_Mix), "Chorus"),
    (Ctl::Param(P::F2_Cut), "Ladder cutoff"),
];
const C_PRESET: usize = 2;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "hydra",
        layers: vec![Layer::Native(0, "KEYS"), Layer::Controls, Layer::Moments],
        hero: vec![[0, 1], [2, 3], [4, 5], [6, 7], [8, 9], [10, 11], [12, 13], [14, 15]],
        browse: Some(C_PRESET),
        routes: Routes { stick_x: Some(0), stick_y: Some(1), hand_l: Some(12), hand_r: Some(13) },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: false,
    }
}

/// What a row of the full menu is.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Preset,
    Slot,
    Save,
    Load,
    Randomize,
    Mutate,
    Init,
    Panic,
    Group(usize),
    Param(usize),
}

const SLOTS: usize = 16;

/// What an oscillator's three morph controls are called for each type, so
/// the menu says what they do.
const OSC_LABELS: [[&str; 3]; 5] = [["Wave", "Width", "Fold"], ["Position", "Bank", "Warp"], ["Ratio", "Index", "Feedback"], ["Damping", "Bright", "Stretch"], ["Color", "Crackle", "Tone"]];
const FM_RATIOS: [&str; 16] = ["0.25", "0.5", "0.75", "1", "1.5", "2", "2.5", "3", "3.5", "4", "5", "6", "7", "8", "10", "12"];

pub struct HydraApp {
    sh: Arc<Shared>,
    presets: Vec<Preset>,
    preset: usize,
    slot: usize,
    status: String,
    list: ParamList,
    expanded: [bool; GROUPS.len()],
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    kit: PlayKit,
    keys: Keys,
    rng: Rng,
    scope: Vec<f32>,
    /// The notes of the pads that are down (for their lights).
    pad_down: [bool; 16],
}

impl HydraApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let cv = |name: &str| mods.register(format!("{APP_NAME}: {name}"));
        let keys = Keys::default();
        let queue: Arc<NoteQueue> = Arc::clone(&keys.queue);
        let sh = Arc::new(Shared {
            params: Params::new(),
            queue,
            cv: Cv {
                cutoff: cv("Cutoff"),
                cutoff2: cv("Ladder Cutoff"),
                reso: cv("Resonance"),
                drive: cv("Drive"),
                pitch: cv("Pitch"),
                wheel: cv("Mod Wheel"),
                detune: cv("Detune"),
                delay: cv("Delay Mix"),
                reverb: cv("Reverb Mix"),
                macros: [cv("Macro 1"), cv("Macro 2"), cv("Macro 3"), cv("Macro 4")],
            },
            mix_level,
            ext_mix_level,
            output,
            active: AtomicUsize::new(0),
            peak: AtomicF32::new(0.0),
            scope: Mutex::new(vec![0.0; engine::SCOPE_LEN]),
            arp_now: AtomicUsize::new(255),
            panic: AtomicBool::new(false),
        });
        let presets = presets::factory();
        // Opens on the first real sound, not the blank init patch.
        let start = 1.min(presets.len() - 1);
        presets::apply(&sh.params, &presets[start]);
        Self {
            sh,
            presets,
            preset: start,
            slot: 0,
            status: String::new(),
            list: ParamList::new(),
            expanded: [false; GROUPS.len()],
            nav,
            sensitivity,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            keys,
            rng: Rng::new(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(7)),
            scope: vec![0.0; engine::SCOPE_LEN],
            pad_down: [false; 16],
        }
    }

    fn sens(&self) -> f32 {
        self.sensitivity.get().max(0.01) * 10.0
    }

    // ------------------------------------------------------------ pads

    /// The note a physical pad plays: the pad scale, rising from the bottom
    /// left, from C at the pad root.
    fn pad_note(&self, pad: usize) -> u8 {
        let scale = self.sh.params.get(P::Scale) as usize;
        let root = self.sh.params.get(P::Root) as i32;
        (48 + root + music_scales::degree(scale, kit::pad_rank(pad))).clamp(0, 127) as u8
    }

    // --------------------------------------------------------- presets

    fn load_preset(&mut self, index: usize) {
        self.preset = index.min(self.presets.len() - 1);
        presets::apply(&self.sh.params, &self.presets[self.preset]);
        self.status = self.presets[self.preset].name.to_string();
    }

    fn step_preset(&mut self, d: i32) {
        let n = self.presets.len() as i32;
        self.load_preset((self.preset as i32 + d.signum()).rem_euclid(n) as usize);
    }

    fn slot_path(slot: usize) -> std::path::PathBuf {
        std::path::PathBuf::from("saves/hydra").join(format!("slot_{:02}.json", slot + 1))
    }

    fn save_slot(&mut self) {
        let values: Vec<f32> = self.sh.params.snapshot().to_vec();
        let doc = serde_json::json!({ "name": format!("User {}", self.slot + 1), "values": values });
        let path = Self::slot_path(self.slot);
        let result = std::fs::create_dir_all("saves/hydra").and_then(|_| std::fs::write(&path, doc.to_string()));
        self.status = match result {
            Ok(()) => format!("saved to slot {}", self.slot + 1),
            Err(e) => format!("could not save: {e}"),
        };
    }

    fn load_slot(&mut self) {
        let text = std::fs::read_to_string(Self::slot_path(self.slot));
        self.status = match text.ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
            Some(doc) => match doc["values"].as_array() {
                Some(v) => {
                    let values: Vec<f32> = v.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
                    self.sh.params.load(&values);
                    format!("loaded slot {}", self.slot + 1)
                }
                None => "that slot is damaged".into(),
            },
            None => format!("slot {} is empty", self.slot + 1),
        };
    }

    // ------------------------------------------------------------ menu

    fn rows(&self) -> Vec<Row> {
        let mut r = vec![Row::Preset, Row::Slot, Row::Save, Row::Load, Row::Randomize, Row::Mutate, Row::Init, Row::Panic];
        for g in 0..GROUPS.len() {
            r.push(Row::Group(g));
            if self.expanded[g] {
                r.extend((0..COUNT).filter(|&i| DEFS[i].group == g).map(Row::Param));
            }
        }
        r
    }

    /// The name of parameter `i`, which for an oscillator's P1-P3 depends on
    /// its type.
    fn label(&self, i: usize) -> String {
        for base in [P::A_Type as usize, P::B_Type as usize, P::C_Type as usize] {
            if (base + 4..=base + 6).contains(&i) {
                let kind = (self.sh.params.at(base) as usize).min(OSC_LABELS.len() - 1);
                return OSC_LABELS[kind][i - base - 4].to_string();
            }
        }
        DEFS[i].name.to_string()
    }

    /// The text of parameter `i`'s value, naming wavetable banks and FM ratios.
    fn value(&self, i: usize) -> String {
        for base in [P::A_Type as usize, P::B_Type as usize, P::C_Type as usize] {
            let kind = self.sh.params.at(base) as usize;
            if i == base + 5 && kind == 1 {
                let bank = (self.sh.params.at(i) * (tables::BANKS - 1) as f32).round() as usize;
                return tables::BANK_NAMES[bank.min(tables::BANKS - 1)].to_string();
            }
            if i == base + 4 && kind == 2 {
                return format!("x{}", FM_RATIOS[((self.sh.params.at(i) * 15.0).round() as usize).min(15)]);
            }
        }
        if i == P::Scale as usize {
            return SCALE_TYPES[self.sh.params.at(i) as usize % SCALE_TYPES.len()].0.to_string();
        }
        if i == P::Root as usize {
            return ROOT_NAMES[self.sh.params.at(i) as usize % 12].to_string();
        }
        self.sh.params.text_at(i)
    }

    fn row_text(&self, row: Row) -> (String, String, bool) {
        match row {
            Row::Preset => ("Preset".into(), format!("{} ({}/{})", self.presets[self.preset].name, self.preset + 1, self.presets.len()), false),
            Row::Slot => ("User slot".into(), format!("{}", self.slot + 1), false),
            Row::Save => ("Save to slot".into(), "press".into(), false),
            Row::Load => ("Load from slot".into(), "press".into(), false),
            Row::Randomize => ("Randomize".into(), "press".into(), false),
            Row::Mutate => ("Mutate".into(), "press".into(), false),
            Row::Init => ("Init patch".into(), "press".into(), false),
            Row::Panic => ("All notes off".into(), "press".into(), false),
            Row::Group(g) => (format!("{} {}", if self.expanded[g] { "-" } else { "+" }, GROUPS[g].to_uppercase()), String::new(), true),
            Row::Param(i) => (format!("  {}", self.label(i)), self.value(i), false),
        }
    }

    fn row_edit(&mut self, row: Row, delta: i32, press: bool) {
        let sens = self.sens();
        match row {
            Row::Preset => {
                if delta != 0 {
                    self.step_preset(delta);
                }
            }
            Row::Slot => {
                if delta != 0 {
                    self.slot = (self.slot as i32 + delta.signum()).rem_euclid(SLOTS as i32) as usize;
                }
            }
            Row::Save if press || delta != 0 => self.save_slot(),
            Row::Load if press || delta != 0 => self.load_slot(),
            Row::Randomize if press || delta != 0 => {
                presets::randomize(&self.sh.params, &mut self.rng);
                self.status = "random patch".into();
            }
            Row::Mutate if press || delta != 0 => {
                presets::mutate(&self.sh.params, &mut self.rng);
                self.status = "mutated".into();
            }
            Row::Init if press || delta != 0 => self.load_preset(0),
            Row::Panic if press || delta != 0 => self.sh.panic.store(true, Ordering::Relaxed),
            Row::Param(i) => {
                if press {
                    self.sh.params.reset_at(i);
                } else if delta != 0 {
                    self.sh.params.step_at(i, delta, sens);
                }
            }
            _ => {}
        }
    }

    fn menu_rows(&self) -> Vec<(String, String, bool)> {
        self.rows().into_iter().map(|r| self.row_text(r)).collect()
    }

    // --------------------------------------------------------- display

    fn summary(&self) -> [String; 4] {
        let p = &self.sh.params;
        let osc = |base: usize, name: &str| -> String {
            if p.at(base + 1) <= 0.0 {
                return format!("{name} -");
            }
            let kind = ["Analog", "Wavetable", "FM", "Pluck", "Noise"][(p.at(base) as usize).min(4)];
            format!("{name} {kind}")
        };
        [
            format!("{}  {}  {}", osc(P::A_Type as usize, "A"), osc(P::B_Type as usize, "B"), osc(P::C_Type as usize, "C")),
            format!("{} {}  res {}", p.text_at(P::F1_Type as usize), p.text_at(P::F1_Cut as usize), p.text_at(P::F1_Res as usize)),
            format!("{}  uni {}  voices {}", p.text_at(P::Mode as usize), p.text_at(P::Unison as usize), p.text_at(P::Voices as usize)),
            if p.get(P::Arp_On) >= 0.5 { format!("arp {} {}", p.text_at(P::Arp_Mode as usize), p.text_at(P::Arp_Rate as usize)) } else { String::new() },
        ]
    }
}

impl PlayHost for HydraApp {
    fn kit_control_count(&self) -> usize {
        KIT.len()
    }
    fn kit_label(&self, i: usize) -> String {
        KIT[i % KIT.len()].1.to_string()
    }
    fn kit_value(&self, i: usize) -> String {
        match KIT[i % KIT.len()].0 {
            Ctl::Preset => self.presets[self.preset].name.to_string(),
            Ctl::Param(p) => self.value(p as usize),
        }
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        Some(match KIT[i % KIT.len()].0 {
            Ctl::Preset => self.preset as f32 / (self.presets.len() - 1).max(1) as f32,
            Ctl::Param(p) => self.sh.params.norm_at(p as usize),
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        match KIT[i % KIT.len()].0 {
            Ctl::Preset => true,
            Ctl::Param(p) => Params::stepped(p as usize),
        }
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match KIT[i % KIT.len()].0 {
            Ctl::Preset => {
                if delta != 0 {
                    self.step_preset(delta);
                }
            }
            Ctl::Param(p) => self.sh.params.step_at(p as usize, delta, self.sens()),
        }
    }
    fn kit_reset(&mut self, i: usize) {
        match KIT[i % KIT.len()].0 {
            Ctl::Preset => self.load_preset(0),
            Ctl::Param(p) => self.sh.params.reset_at(p as usize),
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        match KIT[i % KIT.len()].0 {
            Ctl::Preset => {
                let target = (v.clamp(0.0, 1.0) * (self.presets.len() - 1) as f32).round() as usize;
                if target != self.preset {
                    self.load_preset(target);
                }
            }
            Ctl::Param(p) => self.sh.params.set_norm_at(p as usize, v),
        }
    }
    /// A whole sound: every parameter, not just the play-view dials.
    fn kit_snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "preset": self.preset, "values": self.sh.params.snapshot().to_vec() })
    }
    fn kit_recall(&mut self, v: &serde_json::Value) {
        if let Some(items) = v["values"].as_array() {
            let values: Vec<f32> = items.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            self.sh.params.load(&values);
            self.preset = (v["preset"].as_u64().unwrap_or(0) as usize).min(self.presets.len() - 1);
        }
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        note_name(self.pad_note(pad) as i32)
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, held: bool) -> PadColor {
        let scale = self.sh.params.get(P::Scale) as usize;
        if held || self.pad_down[pad] {
            PadColor::Green
        } else if kit::pad_rank(pad) % music_scales::intervals(scale).len() == 0 {
            PadColor::Blue
        } else {
            PadColor::Off
        }
    }
    fn kit_line(&self) -> String {
        format!("{}  {} voices", self.presets[self.preset].name, self.sh.active.load(Ordering::Relaxed))
    }
}

impl App for HydraApp {
    fn instrument_settings(&self) -> Vec<crate::app::Setting> {
        crate::app::play_kit::settings_of(self)
    }
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        crate::app::play_kit::adjust_in(self, index, delta)
    }
    fn play_surface(&self) -> bool {
        true
    }
    fn supports_pad_lock(&self) -> bool {
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
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        self.kit.led_overlay(self)
    }
    fn needs_background_audio(&self) -> bool {
        // Keep rendering while anything is still sounding: a held arpeggio,
        // a release or a reverb tail.
        self.sh.active.load(Ordering::Relaxed) > 0 || self.sh.peak.get() > 0.0003 || self.sh.params.get(P::Arp_On) >= 0.5 && self.sh.arp_now.load(Ordering::Relaxed) < 128
    }
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let in_menu = step.menu;
        let pads_play = step.native.is_some() || in_menu;
        let notes: [u8; 16] = std::array::from_fn(|p| self.pad_note(p));
        // Notes from a keyboard or the note bus play at their own pitch,
        // whatever the pads are doing.
        let mut keyed = step.input;
        keyed.midi_keys = input.midi_keys;
        self.pad_down = std::array::from_fn(|p| pads_play && keyed.grid[p]);
        self.keys.update(&keyed, pads_play, |p| notes[p]);
        if in_menu {
            let i = &step.input;
            let rows = self.rows();
            self.list.navigate_input(i, rows.len(), self.nav.get() as i32);
            let sel = self.list.selected.min(rows.len() - 1);
            let row = rows[sel];
            if let Row::Group(g) = row {
                if i.knob1_press || i.knob2_press || i.knob2 != 0 {
                    self.expanded[g] = !self.expanded[g];
                }
            } else {
                self.row_edit(row, i.knob2, i.knob2_press);
            }
        }
    }
    fn on_exit(&mut self) {
        // let go of whatever the pads were holding
        self.pad_down = [false; 16];
        self.keys.update(&Input::default(), false, |_| 0);
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if self.kit.menu {
            Text::new("HYDRA", Point::new(16, 30), MonoTextStyle::new(&SPLEEN_8X16, ACCENT)).draw(f).ok();
            if !self.status.is_empty() {
                Text::new(&self.status, Point::new(120, 30), MonoTextStyle::new(&SPLEEN_6X12, DIM)).draw(f).ok();
            }
            let rows: Vec<(String, String)> = self.menu_rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 44, 24, 11, &rows, INK, DIM, CHIP);
            return;
        }
        if let Some(col) = self.play_column() {
            let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
            kit::draw::column(f, &col, 16, 40, 350, 285, pal);
        }
        // the right-hand panel: what is playing
        let (x0, w) = (392, 232);
        Text::new("HYDRA", Point::new(x0, 56), MonoTextStyle::new(&SPLEEN_8X16, ACCENT)).draw(f).ok();
        Text::new(self.presets[self.preset].name, Point::new(x0, 78), MonoTextStyle::new(&SPLEEN_8X16, INK)).draw(f).ok();
        let small = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        for (n, line) in self.summary().iter().enumerate() {
            Text::new(line, Point::new(x0, 100 + n as i32 * 14), small).draw(f).ok();
        }
        // voices
        let active = self.sh.active.load(Ordering::Relaxed);
        for v in 0..MAX_VOICES {
            let c = if v < active { ACCENT } else { FAINT };
            Rectangle::new(Point::new(x0 + (v as i32 % 8) * 29, 164 + (v as i32 / 8) * 14), Size::new(25, 10)).into_styled(PrimitiveStyle::with_fill(c)).draw(f).ok();
        }
        // scope
        if let Ok(s) = self.sh.scope.try_lock() {
            self.scope.copy_from_slice(&s);
        }
        let (sy, sh_) = (200, 70);
        Rectangle::new(Point::new(x0, sy), Size::new(w as u32, sh_ as u32)).into_styled(PrimitiveStyle::with_stroke(FAINT, 1)).draw(f).ok();
        let n = self.scope.len();
        let mut prev: Option<Point> = None;
        for (i, v) in self.scope.iter().enumerate() {
            let p = Point::new(x0 + (i * (w as usize - 2) / n) as i32 + 1, sy + sh_ / 2 - (v.clamp(-1.0, 1.0) * (sh_ as f32 / 2.0 - 3.0)) as i32);
            if let Some(q) = prev {
                Line::new(q, p).into_styled(PrimitiveStyle::with_stroke(INK, 1)).draw(f).ok();
            }
            prev = Some(p);
        }
        // level
        let peak = (self.sh.peak.get().clamp(0.0, 1.0) * w as f32) as u32;
        Rectangle::new(Point::new(x0, 278), Size::new(peak.max(1), 6)).into_styled(PrimitiveStyle::with_fill(if peak as f32 > w as f32 * 0.9 { ACCENT } else { INK })).draw(f).ok();
        if self.sh.params.get(P::Arp_On) >= 0.5 {
            let n = self.sh.arp_now.load(Ordering::Relaxed);
            if n < 128 {
                Text::new(&format!("arp {}", note_name(n as i32)), Point::new(x0 + 150, 56), small).draw(f).ok();
            }
        }
        Text::new("pads: scale   F2: layer   R1: menu", Point::new(16, 340), small).draw(f).ok();
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.menu_rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_scale_info(&self) -> Option<music_scales::ScaleInfo> {
        Some(music_scales::ScaleInfo::new(self.sh.params.get(P::Scale) as usize, self.sh.params.get(P::Root) as i32))
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(Engine::new(Arc::clone(&self.sh))))
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(HydraApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}

#[cfg(test)]
mod tests;
