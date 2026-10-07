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
//! **Playing.** Poly, mono or legato with glide, an arpeggiator, chords
//! (fixed shapes, or built from the pad scale so one pad plays the right
//! chord for its degree), Hold, scale, chromatic or fourths pads, MIDI keys
//! and notes from any app over the note bus. Chords and Hold work on every
//! source of notes, not just the pads.
//!
//! **Sounds.** Over two hundred factory presets in sixteen folders (Bass,
//! Lead, Pad, Keys...), your own saved into the same folders under
//! `saves/hydra/presets/<Folder>/` (drop a file there and it appears),
//! favorites, Randomize, Mutate and Morph (slide the whole patch toward
//! another preset). Every preset names its four macros and wires each to
//! two destinations, so the four macro dials always do something useful.
//!
//! **Play view.** Sixteen controls on the pads and knobs: Cutoff,
//! Resonance, the preset and folder browsers, a bank selector that swaps
//! six controls between Osc / Filter / Env / Mod / FX / Play pages, Level
//! and the four macros (which the hand sensors and stick drive).
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
use presets::{Library, Preset};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use store::{from_norm, to_norm, Params};

const APP_NAME: &str = "Hydra";

// Own palette: deep ocean with a hot magenta accent.
const BG: Rgb565 = Rgb565::new(1, 4, 7);
const INK: Rgb565 = Rgb565::new(24, 60, 28);
const ACCENT: Rgb565 = Rgb565::new(31, 22, 24);
const DIM: Rgb565 = Rgb565::new(10, 26, 22);
const FAINT: Rgb565 = Rgb565::new(4, 10, 12);
const CHIP: Rgb565 = Rgb565::new(5, 14, 16);

/// What a play-view control is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ctl {
    Param(P),
    /// The preset browser (steps through the current folder).
    Preset,
    /// The folder browser.
    Folder,
    /// Which page of the six swappable controls is showing.
    Bank,
    /// Macro 1..4, named by the sound.
    Macro(usize),
    /// Slides the whole patch toward another preset.
    Morph,
}

/// The pages of six controls that share slots 4..9 of the play view.
const BANK_NAMES: [&str; 6] = ["Osc", "Filter", "Env", "Mod", "FX", "Play"];
const BANKS: [[(Ctl, &str); 6]; 6] = [
    [
        (Ctl::Param(P::A_P1), "A"),
        (Ctl::Param(P::A_P2), "A"),
        (Ctl::Param(P::B_P1), "B"),
        (Ctl::Param(P::B_P2), "B"),
        (Ctl::Param(P::B_Level), "B level"),
        (Ctl::Param(P::SubLevel), "Sub"),
    ],
    [
        (Ctl::Param(P::F1_Env), "Filter env"),
        (Ctl::Param(P::F1_Drive), "Filter drive"),
        (Ctl::Param(P::F2_Cut), "Ladder cutoff"),
        (Ctl::Param(P::F2_Res), "Ladder res"),
        (Ctl::Param(P::F2_Env), "Ladder env"),
        (Ctl::Param(P::F1_Key), "Key track"),
    ],
    [
        (Ctl::Param(P::AmpA), "Attack"),
        (Ctl::Param(P::AmpD), "Decay"),
        (Ctl::Param(P::AmpS), "Sustain"),
        (Ctl::Param(P::AmpR), "Release"),
        (Ctl::Param(P::E2D), "Env 2 decay"),
        (Ctl::Param(P::E2S), "Env 2 sustain"),
    ],
    [
        (Ctl::Param(P::L1_Rate), "LFO 1 rate"),
        (Ctl::Param(P::L2_Rate), "LFO 2 rate"),
        (Ctl::Param(P::XMod), "Cross mod"),
        (Ctl::Param(P::Glide), "Glide"),
        (Ctl::Param(P::VelSens), "Velocity"),
        (Ctl::Morph, "Morph"),
    ],
    [
        (Ctl::Param(P::Drv_Amt), "Drive"),
        (Ctl::Param(P::Cho_Mix), "Chorus"),
        (Ctl::Param(P::Dly_Mix), "Delay"),
        (Ctl::Param(P::Dly_Fb), "Delay fb"),
        (Ctl::Param(P::Rev_Mix), "Reverb"),
        (Ctl::Param(P::Rev_Decay), "Reverb time"),
    ],
    [
        (Ctl::Param(P::Chord), "Chord"),
        (Ctl::Param(P::Hold), "Hold"),
        (Ctl::Param(P::Arp_On), "Arp"),
        (Ctl::Param(P::Arp_Rate), "Arp rate"),
        (Ctl::Param(P::Octave), "Octave"),
        (Ctl::Param(P::Unison), "Unison"),
    ],
];
const CONTROLS: usize = 16;
const C_PRESET: usize = 2;
const C_BANK: usize = 10;

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
    Folder,
    Preset,
    Favorite,
    /// Opens the list of the folder's presets right under it.
    Browse,
    Listed(usize),
    Save,
    Randomize,
    Mutate,
    MorphTo,
    MorphAmount,
    Init,
    Panic,
    Group(usize),
    Param(usize),
}

/// What an oscillator's three morph controls are called for each type, so
/// the menu says what they do.
const OSC_LABELS: [[&str; 3]; 5] = [["Wave", "Width", "Fold"], ["Position", "Bank", "Warp"], ["Ratio", "Index", "Feedback"], ["Damping", "Bright", "Stretch"], ["Color", "Crackle", "Tone"]];
const FM_RATIOS: [&str; 16] = ["0.25", "0.5", "0.75", "1", "1.5", "2", "2.5", "3", "3.5", "4", "5", "6", "7", "8", "10", "12"];

/// A slide from the sound as it was toward another preset.
struct Morph {
    from: Vec<f32>,
    to: Vec<f32>,
    target: usize,
    from_macros: [String; 4],
}

pub struct HydraApp {
    sh: Arc<Shared>,
    lib: Library,
    /// The preset loaded (an index into `lib.presets`).
    cur: usize,
    /// The folder being browsed (an index into `lib.folders()`).
    folder: usize,
    bank: usize,
    browse_open: bool,
    macros: [String; 4],
    morph: Option<Morph>,
    morph_amt: f32,
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
        Self::with_library(sensitivity, nav, mods, bus, mixer, if cfg!(test) { Library::with(presets::factory(), Vec::new(), Default::default()) } else { Library::new() })
    }

    fn with_library(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>, lib: Library) -> Self {
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
        let mut app = Self {
            sh,
            lib,
            cur: 0,
            folder: 0,
            bank: 0,
            browse_open: false,
            macros: Default::default(),
            morph: None,
            morph_amt: 0.0,
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
        };
        // Opens on the first real sound, not the blank init patch.
        let start = app.lib.presets.iter().position(|p| p.name == "Super Saw").unwrap_or(0);
        app.folder = app.lib.folders().iter().position(|f| *f == app.lib.presets[start].category).unwrap_or(1);
        app.load_preset(start);
        app.status.clear();
        app
    }

    fn sens(&self) -> f32 {
        self.sensitivity.get().max(0.01) * 10.0
    }

    // ------------------------------------------------------------ pads

    /// The note a physical pad plays, rising from the bottom left: the pad
    /// scale, every semitone, or rows a fourth apart like a guitar.
    fn pad_note(&self, pad: usize) -> u8 {
        let root = self.sh.params.get(P::Root) as i32;
        let rank = kit::pad_rank(pad);
        let offset = match self.sh.params.get(P::PadMode) as usize {
            1 => rank as i32,
            2 => (rank % 4) as i32 + 5 * (rank / 4) as i32,
            _ => music_scales::degree(self.sh.params.get(P::Scale) as usize, rank),
        };
        (48 + root + offset).clamp(0, 127) as u8
    }

    /// Whether a pad plays the root (lit blue).
    fn pad_is_root(&self, pad: usize) -> bool {
        let rank = kit::pad_rank(pad);
        match self.sh.params.get(P::PadMode) as usize {
            1 => rank % 12 == 0,
            2 => ((rank % 4) as i32 + 5 * (rank / 4) as i32) % 12 == 0,
            _ => rank % music_scales::intervals(self.sh.params.get(P::Scale) as usize).len() == 0,
        }
    }

    // --------------------------------------------------------- presets

    fn folder_name(&self) -> String {
        let f = self.lib.folders();
        f[self.folder.min(f.len() - 1)].clone()
    }

    fn folder_list(&self) -> Vec<usize> {
        self.lib.in_folder(&self.folder_name())
    }

    /// Loads preset `index` of the library and ends any morph.
    fn load_preset(&mut self, index: usize) {
        self.cur = index.min(self.lib.presets.len() - 1);
        self.morph = None;
        self.morph_amt = 0.0;
        presets::apply(&self.sh.params, &self.lib.presets[self.cur]);
        self.macros = self.lib.presets[self.cur].macros.clone();
        self.status = format!("{} > {}", self.lib.presets[self.cur].category, self.lib.presets[self.cur].name);
    }

    /// Steps to the next or previous preset of the folder being browsed.
    fn step_preset(&mut self, d: i32) {
        let list = self.folder_list();
        if list.is_empty() {
            self.status = "no favorites yet".into();
            return;
        }
        let at = list.iter().position(|&i| i == self.cur);
        let next = match at {
            Some(p) => (p as i32 + d.signum()).rem_euclid(list.len() as i32) as usize,
            None => 0,
        };
        self.load_preset(list[next]);
    }

    fn step_folder(&mut self, d: i32) {
        let n = self.lib.folders().len() as i32;
        self.folder = (self.folder as i32 + d.signum()).rem_euclid(n) as usize;
        // moving between folders auditions the folder's first sound
        if let Some(&first) = self.folder_list().first() {
            self.load_preset(first);
        } else {
            self.status = "no favorites yet".into();
        }
    }

    fn set_folder_norm(&mut self, v: f32) {
        let n = self.lib.folders().len();
        let target = (v.clamp(0.0, 1.0) * (n - 1) as f32).round() as usize;
        if target != self.folder {
            self.folder = target;
            if let Some(&first) = self.folder_list().first() {
                self.load_preset(first);
            }
        }
    }

    fn set_preset_norm(&mut self, v: f32) {
        let list = self.folder_list();
        if list.is_empty() {
            return;
        }
        let target = list[(v.clamp(0.0, 1.0) * (list.len() - 1) as f32).round() as usize];
        if target != self.cur {
            self.load_preset(target);
        }
    }

    fn toggle_favorite(&mut self) {
        let on = self.lib.toggle_favorite(self.cur);
        self.status = if on { "added to favorites".into() } else { "removed from favorites".into() };
    }

    /// Saves the sound into the folder being browsed (or the sound's own
    /// when browsing favorites), as "User N".
    fn save_preset(&mut self) {
        let category = match self.folder_name().as_str() {
            "Favorites" => self.lib.presets[self.cur].category.clone(),
            other => other.to_string(),
        };
        let n = self.lib.presets.iter().filter(|p| p.user && p.category == category).count() + 1;
        let name = format!("User {n}");
        let values = self.sh.params.snapshot().to_vec();
        self.status = match presets::save_user(&category, &name, &values, &self.macros) {
            Ok(_) => {
                let mut p = Preset::new(&name, values.iter().copied().enumerate().collect());
                p.category = category.clone();
                p.user = true;
                p.macros = self.macros.clone();
                self.cur = self.lib.add_user(p);
                format!("saved {category} > {name}")
            }
            Err(e) => format!("could not save: {e}"),
        };
    }

    // ----------------------------------------------------------- morph

    fn start_morph(&mut self, target: usize) {
        self.morph = Some(Morph { from: self.sh.params.snapshot().to_vec(), to: presets::resolve(&self.lib.presets[target]), target, from_macros: self.macros.clone() });
        self.morph_amt = 0.0;
    }

    /// Slides the patch `amount` of the way toward the morph target (the
    /// next preset of the folder if none was chosen).
    fn set_morph(&mut self, amount: f32) {
        if self.morph.is_none() {
            let list = self.folder_list();
            let target = list.iter().position(|&i| i == self.cur).map(|p| list[(p + 1) % list.len()]).unwrap_or(self.cur);
            self.start_morph(target);
        }
        let x = amount.clamp(0.0, 1.0);
        self.morph_amt = x;
        let Some(m) = &self.morph else { return };
        for i in 0..COUNT {
            let (a, b) = (m.from[i], m.to[i]);
            let v = if Params::stepped(i) {
                if x < 0.5 { a } else { b }
            } else {
                from_norm(&DEFS[i], to_norm(&DEFS[i], a) * (1.0 - x) + to_norm(&DEFS[i], b) * x)
            };
            self.sh.params.set_at(i, v);
        }
        self.macros = if x < 0.5 { m.from_macros.clone() } else { self.lib.presets[m.target].macros.clone() };
        self.status = format!("morph {:.0}% > {}", x * 100.0, self.lib.presets[m.target].name);
    }

    fn morph_text(&self) -> String {
        match &self.morph {
            Some(m) => format!("{} {:.0}%", self.lib.presets[m.target].name, self.morph_amt * 100.0),
            None => "next in folder".into(),
        }
    }

    // ------------------------------------------------------- controls

    /// What play-view control `i` is right now.
    fn ctl(&self, i: usize) -> (Ctl, &'static str) {
        match i % CONTROLS {
            0 => (Ctl::Param(P::F1_Cut), "Cutoff"),
            1 => (Ctl::Param(P::F1_Res), "Resonance"),
            2 => (Ctl::Preset, "Preset"),
            3 => (Ctl::Folder, "Folder"),
            4..=9 => BANKS[self.bank % BANKS.len()][i - 4],
            10 => (Ctl::Bank, "Page"),
            11 => (Ctl::Param(P::Level), "Level"),
            m => (Ctl::Macro(m - 12), ""),
        }
    }

    fn macro_name(&self, n: usize) -> String {
        if self.macros[n].is_empty() { format!("Macro {}", n + 1) } else { self.macros[n].clone() }
    }

    // ------------------------------------------------------------ menu

    fn rows(&self) -> Vec<Row> {
        let mut r = vec![Row::Folder, Row::Preset, Row::Favorite, Row::Browse];
        if self.browse_open {
            r.extend(self.folder_list().into_iter().map(Row::Listed));
        }
        r.extend([Row::Save, Row::Randomize, Row::Mutate, Row::MorphTo, Row::MorphAmount, Row::Init, Row::Panic]);
        for g in 0..GROUPS.len() {
            r.push(Row::Group(g));
            if self.expanded[g] {
                r.extend((0..COUNT).filter(|&i| DEFS[i].group == g).map(Row::Param));
            }
        }
        r
    }

    /// The name of parameter `i`, which for an oscillator's P1-P3 depends on
    /// its type, and for a macro's assignments is the macro's name.
    fn label(&self, i: usize) -> String {
        for base in [P::A_Type as usize, P::B_Type as usize, P::C_Type as usize] {
            if (base + 4..=base + 6).contains(&i) {
                let kind = (self.sh.params.at(base) as usize).min(OSC_LABELS.len() - 1);
                return OSC_LABELS[kind][i - base - 4].to_string();
            }
        }
        if (P::Mac1_DA as usize..=P::Mac4_AB as usize).contains(&i) {
            let n = (i - P::Mac1_DA as usize) / 4;
            let which = (i - P::Mac1_DA as usize) % 4;
            return format!("{} {}", self.macro_name(n), ["to", "by", "and", "by"][which]);
        }
        if (P::Mac1 as usize..=P::Mac4 as usize).contains(&i) {
            return self.macro_name(i - P::Mac1 as usize);
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

    fn star(&self) -> &'static str {
        if self.lib.is_favorite(self.cur) { " *" } else { "" }
    }

    fn row_text(&self, row: Row) -> (String, String, bool) {
        let cur = &self.lib.presets[self.cur];
        match row {
            Row::Folder => {
                let f = self.lib.folders();
                ("Folder".into(), format!("{} ({}/{})", f[self.folder.min(f.len() - 1)], self.folder + 1, f.len()), false)
            }
            Row::Preset => {
                let (at, n) = self.lib.place(&self.folder_name(), self.cur);
                ("Preset".into(), format!("{}{} ({}/{})", cur.name, self.star(), at, n), false)
            }
            Row::Favorite => ("Favorite".into(), if self.lib.is_favorite(self.cur) { "on".into() } else { "off".into() }, false),
            Row::Browse => (format!("{} Browse {}", if self.browse_open { "-" } else { "+" }, self.folder_name()), String::new(), true),
            Row::Listed(i) => {
                let p = &self.lib.presets[i];
                (format!("  {}{}", p.name, if self.lib.is_favorite(i) { " *" } else { "" }), if i == self.cur { "loaded".into() } else if p.user { "user".into() } else { String::new() }, false)
            }
            Row::Save => ("Save as new preset".into(), "press".into(), false),
            Row::Randomize => ("Randomize".into(), "press".into(), false),
            Row::Mutate => ("Mutate".into(), "press".into(), false),
            Row::MorphTo => ("Morph toward".into(), self.morph_text(), false),
            Row::MorphAmount => ("Morph".into(), format!("{:.0}%", self.morph_amt * 100.0), false),
            Row::Init => ("Init patch".into(), "press".into(), false),
            Row::Panic => ("All notes off".into(), "press".into(), false),
            Row::Group(g) => (format!("{} {}", if self.expanded[g] { "-" } else { "+" }, GROUPS[g].to_uppercase()), String::new(), true),
            Row::Param(i) => (format!("  {}", self.label(i)), self.value(i), false),
        }
    }

    fn row_edit(&mut self, row: Row, delta: i32, press: bool) {
        let sens = self.sens();
        match row {
            Row::Folder if delta != 0 => self.step_folder(delta),
            Row::Preset if delta != 0 => self.step_preset(delta),
            Row::Favorite if press || delta != 0 => self.toggle_favorite(),
            Row::Browse if press || delta != 0 => self.browse_open = !self.browse_open,
            Row::Listed(i) if press || delta != 0 => self.load_preset(i),
            Row::Save if press || delta != 0 => self.save_preset(),
            Row::Randomize if press || delta != 0 => {
                self.morph = None;
                presets::randomize(&self.sh.params, &mut self.rng);
                self.status = "random patch".into();
            }
            Row::Mutate if press || delta != 0 => {
                self.morph = None;
                presets::mutate(&self.sh.params, &mut self.rng);
                self.status = "mutated".into();
            }
            Row::MorphTo if delta != 0 => {
                let n = self.lib.presets.len() as i32;
                let from = self.morph.as_ref().map(|m| m.target).unwrap_or(self.cur) as i32;
                let target = (from + delta.signum()).rem_euclid(n) as usize;
                self.start_morph(target);
            }
            Row::MorphAmount if delta != 0 => self.set_morph(self.morph_amt + delta as f32 * 0.02 * sens.max(1.0).min(5.0)),
            Row::Init if press || delta != 0 => {
                if let Some(i) = self.lib.presets.iter().position(|p| p.category == "Init") {
                    self.load_preset(i);
                }
            }
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
        let mut play = format!("{}  uni {}  voices {}", p.text_at(P::Mode as usize), p.text_at(P::Unison as usize), p.text_at(P::Voices as usize));
        if p.get(P::Chord) > 0.0 {
            play = format!("{play}  {}", p.text_at(P::Chord as usize));
        }
        if p.get(P::Hold) >= 0.5 {
            play = format!("{play}  HOLD");
        }
        [
            format!("{}  {}  {}", osc(P::A_Type as usize, "A"), osc(P::B_Type as usize, "B"), osc(P::C_Type as usize, "C")),
            format!("{} {}  res {}", p.text_at(P::F1_Type as usize), p.text_at(P::F1_Cut as usize), p.text_at(P::F1_Res as usize)),
            play,
            if p.get(P::Arp_On) >= 0.5 { format!("arp {} {}", p.text_at(P::Arp_Mode as usize), p.text_at(P::Arp_Rate as usize)) } else { String::new() },
        ]
    }
}

impl PlayHost for HydraApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS
    }
    fn kit_label(&self, i: usize) -> String {
        let (c, name) = self.ctl(i);
        match c {
            Ctl::Macro(n) => self.macro_name(n),
            // an oscillator's controls say what they are for its type
            Ctl::Param(p) if (name == "A" || name == "B") => format!("{name} {}", self.label(p as usize)),
            _ => name.to_string(),
        }
    }
    fn kit_value(&self, i: usize) -> String {
        match self.ctl(i).0 {
            Ctl::Preset => self.lib.presets[self.cur].name.clone(),
            Ctl::Folder => self.folder_name(),
            Ctl::Bank => BANK_NAMES[self.bank % BANKS.len()].to_string(),
            Ctl::Macro(n) => self.sh.params.text_at(P::Mac1 as usize + n),
            Ctl::Morph => self.morph_text(),
            Ctl::Param(p) => self.value(p as usize),
        }
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        Some(match self.ctl(i).0 {
            Ctl::Preset => {
                let (at, n) = self.lib.place(&self.folder_name(), self.cur);
                if n > 1 { (at.max(1) - 1) as f32 / (n - 1) as f32 } else { 0.0 }
            }
            Ctl::Folder => self.folder as f32 / (self.lib.folders().len() - 1).max(1) as f32,
            Ctl::Bank => self.bank as f32 / (BANKS.len() - 1) as f32,
            Ctl::Macro(n) => self.sh.params.norm_at(P::Mac1 as usize + n),
            Ctl::Morph => self.morph_amt,
            Ctl::Param(p) => self.sh.params.norm_at(p as usize),
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        match self.ctl(i).0 {
            Ctl::Preset | Ctl::Folder | Ctl::Bank => true,
            Ctl::Macro(_) | Ctl::Morph => false,
            Ctl::Param(p) => Params::stepped(p as usize),
        }
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match self.ctl(i).0 {
            Ctl::Preset if delta != 0 => self.step_preset(delta),
            Ctl::Folder if delta != 0 => self.step_folder(delta),
            Ctl::Bank if delta != 0 => {
                self.bank = (self.bank as i32 + delta.signum()).rem_euclid(BANKS.len() as i32) as usize;
                self.status = format!("page: {}", BANK_NAMES[self.bank]);
            }
            Ctl::Macro(n) => self.sh.params.step_at(P::Mac1 as usize + n, delta, self.sens()),
            Ctl::Morph if delta != 0 => self.set_morph(self.morph_amt + delta as f32 * 0.02 * self.sens().max(1.0).min(5.0)),
            Ctl::Param(p) => self.sh.params.step_at(p as usize, delta, self.sens()),
            _ => {}
        }
    }
    fn kit_reset(&mut self, i: usize) {
        match self.ctl(i).0 {
            // back to the sound as it was saved, undoing the tweaks
            Ctl::Preset => self.load_preset(self.cur),
            Ctl::Folder => {}
            Ctl::Bank => self.bank = 0,
            Ctl::Macro(n) => self.sh.params.reset_at(P::Mac1 as usize + n),
            Ctl::Morph => {
                self.morph = None;
                self.morph_amt = 0.0;
                self.load_preset(self.cur);
            }
            Ctl::Param(p) => self.sh.params.reset_at(p as usize),
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        match self.ctl(i).0 {
            Ctl::Preset => self.set_preset_norm(v),
            Ctl::Folder => self.set_folder_norm(v),
            Ctl::Bank => self.bank = (v.clamp(0.0, 1.0) * (BANKS.len() - 1) as f32).round() as usize,
            Ctl::Macro(n) => self.sh.params.set_norm_at(P::Mac1 as usize + n, v),
            Ctl::Morph => self.set_morph(v),
            Ctl::Param(p) => self.sh.params.set_norm_at(p as usize, v),
        }
    }
    /// A whole sound: every parameter, not just the play-view dials.
    fn kit_snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "preset": self.lib.key(self.cur), "values": self.sh.params.snapshot().to_vec(), "macros": self.macros })
    }
    fn kit_recall(&mut self, v: &serde_json::Value) {
        if let Some(items) = v["values"].as_array() {
            let values: Vec<f32> = items.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            self.sh.params.load(&values);
            self.morph = None;
            self.morph_amt = 0.0;
            if let Some(key) = v["preset"].as_str() {
                if let Some(i) = (0..self.lib.presets.len()).find(|&i| self.lib.key(i) == key) {
                    self.cur = i;
                }
            }
            if let Some(m) = v["macros"].as_array() {
                for (n, name) in m.iter().take(4).enumerate() {
                    self.macros[n] = name.as_str().unwrap_or("").to_string();
                }
            }
        }
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        note_name(self.pad_note(pad) as i32)
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, held: bool) -> PadColor {
        if held || self.pad_down[pad] {
            PadColor::Green
        } else if self.pad_is_root(pad) {
            PadColor::Blue
        } else {
            PadColor::Off
        }
    }
    fn kit_line(&self) -> String {
        format!("{} > {}{}  {} voices", self.lib.presets[self.cur].category, self.lib.presets[self.cur].name, self.star(), self.sh.active.load(Ordering::Relaxed))
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
        let small = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        Text::new(&self.lib.presets[self.cur].category.to_uppercase(), Point::new(x0, 56), MonoTextStyle::new(&SPLEEN_8X16, ACCENT)).draw(f).ok();
        Text::new(&format!("{}{}", self.lib.presets[self.cur].name, self.star()), Point::new(x0, 78), MonoTextStyle::new(&SPLEEN_8X16, INK)).draw(f).ok();
        // the four macros, named by the sound
        for n in 0..4 {
            let y = 98 + n as i32 * 14;
            Text::new(&self.macro_name(n), Point::new(x0, y), small).draw(f).ok();
            let v = self.sh.params.norm_at(P::Mac1 as usize + n);
            Rectangle::new(Point::new(x0 + 96, y - 8), Size::new(100, 8)).into_styled(PrimitiveStyle::with_stroke(FAINT, 1)).draw(f).ok();
            Rectangle::new(Point::new(x0 + 96, y - 8), Size::new((v * 100.0) as u32, 8)).into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(f).ok();
        }
        for (n, line) in self.summary().iter().enumerate() {
            Text::new(line, Point::new(x0, 168 + n as i32 * 12), small).draw(f).ok();
        }
        // voices
        let active = self.sh.active.load(Ordering::Relaxed);
        for v in 0..MAX_VOICES {
            let c = if v < active { ACCENT } else { FAINT };
            Rectangle::new(Point::new(x0 + (v as i32 % 8) * 29, 222 + (v as i32 / 8) * 10), Size::new(25, 7)).into_styled(PrimitiveStyle::with_fill(c)).draw(f).ok();
        }
        // scope
        if let Ok(s) = self.sh.scope.try_lock() {
            self.scope.copy_from_slice(&s);
        }
        let (sy, sh_) = (246, 48);
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
        Rectangle::new(Point::new(x0, 300), Size::new(peak.max(1), 6)).into_styled(PrimitiveStyle::with_fill(if peak as f32 > w as f32 * 0.9 { ACCENT } else { INK })).draw(f).ok();
        if self.sh.params.get(P::Arp_On) >= 0.5 {
            let n = self.sh.arp_now.load(Ordering::Relaxed);
            if n < 128 {
                Text::new(&format!("arp {}", note_name(n as i32)), Point::new(x0 + 150, 56), small).draw(f).ok();
            }
        }
        Text::new(&format!("page: {}   pads: notes   F2: layer   R1: menu", BANK_NAMES[self.bank % BANKS.len()]), Point::new(16, 340), small).draw(f).ok();
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
