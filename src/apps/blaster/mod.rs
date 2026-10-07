//! Blaster: hold a key and it charges; let go and it fires.
//!
//! The charge is a tone that climbs and chirps faster the longer you hold
//! (and flutters once it is full); the blast that follows is a pitch sweep
//! with a burst of noise, as big as the charge was. A quick tap fires a small
//! shot. That is the whole idea, and everything about it is a dial: the
//! charge time, how far and how fast it climbs, the blast's wave, pitch
//! sweep, length and noise, how much a bigger charge changes the blast (its
//! pitch, length, noise and volume), 8-bit crush and echo.
//!
//! Twenty-eight characters sit in the preset browser, from a pea shot to a
//! meteor, as starting points for designing your own. Notes from any app
//! play it too: a note-on starts a charge and the note-off fires it, so how
//! long a sequencer holds a note is how big the blast is.

pub mod engine;
pub mod params;
pub mod presets;
pub mod store;
pub mod voice;

use crate::app::music_scales::{self, ROOT_NAMES, SCALE_TYPES};
use crate::app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes};
use crate::app::{App, Input};
use crate::apps::hydra::dsp::Rng;
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
use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Line, PrimitiveStyle, Rectangle},
    text::Text,
};
use engine::{Cv, Engine, Shared};
use params::*;
use presets::Preset;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use store::Params;

const APP_NAME: &str = "Blaster";

// Own palette: near-black with a hot cyan charge and a white-hot flash.
const BG: Rgb565 = Rgb565::new(0, 2, 5);
const INK: Rgb565 = Rgb565::new(20, 58, 31);
const ACCENT: Rgb565 = Rgb565::new(31, 62, 8);
const DIM: Rgb565 = Rgb565::new(6, 24, 22);
const FAINT: Rgb565 = Rgb565::new(2, 9, 10);
const CHIP: Rgb565 = Rgb565::new(3, 12, 14);
const FLASH: Rgb565 = Rgb565::new(31, 63, 31);

/// The play view's controls, most important first (the first land on the
/// knobs and the bottom pads).
const KIT: [(P, &str); 15] = [
    (P::ChargeTime, "Charge time"),
    (P::Length, "Blast length"),
    // slot 2 is the preset browser
    (P::Level, "Level"),
    (P::ChgPitch, "Charge pitch"),
    (P::Climb, "Climb"),
    (P::SubCharge, "Sub (charge)"),
    (P::ChirpDepth, "Chirp depth"),
    (P::BlastStart, "Blast start"),
    (P::BlastEnd, "Blast end"),
    (P::SweepTime, "Sweep time"),
    (P::Noise, "Noise"),
    (P::Body, "Body"),
    (P::SubBlast, "Sub (blast)"),
    (P::EchoMix, "Echo"),
    (P::AutoFire, "Fire at full"),
];
const C_PRESET: usize = 2;
const CONTROLS: usize = 16;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "blaster",
        layers: vec![Layer::Native(0, "SHOOT"), Layer::Controls, Layer::Moments],
        hero: vec![[0, 1], [2, 3], [4, 5], [6, 7], [8, 9], [10, 11], [12, 13], [14, 15]],
        browse: Some(C_PRESET),
        routes: Routes { stick_x: Some(0), stick_y: Some(1), hand_l: Some(5), hand_r: Some(10) },
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

const SLOTS: usize = 8;

pub struct BlasterApp {
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
    pad_down: [bool; 16],
    /// The blast count last seen, and frames of flash left.
    seen_fired: u32,
    flash: u32,
}

impl BlasterApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let cv = |name: &str| mods.register(format!("{APP_NAME}: {name}"));
        let keys = Keys::default();
        let queue: Arc<NoteQueue> = Arc::clone(&keys.queue);
        let sh = Arc::new(Shared {
            params: Params::new(),
            queue,
            cv: Cv { pitch: cv("Pitch"), charge: cv("Charge Time"), length: cv("Blast Length") },
            mix_level,
            ext_mix_level,
            output,
            charge: AtomicF32::new(0.0),
            active: AtomicUsize::new(0),
            fired: AtomicU32::new(0),
            last_power: AtomicF32::new(0.0),
            peak: AtomicF32::new(0.0),
            scope: Mutex::new(vec![0.0; engine::SCOPE_LEN]),
            panic: AtomicBool::new(false),
        });
        let presets = presets::factory();
        presets::apply(&sh.params, &presets[0]);
        Self {
            sh,
            presets,
            preset: 0,
            slot: 0,
            status: String::new(),
            list: ParamList::new(),
            expanded: [false; GROUPS.len()],
            nav,
            sensitivity,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            keys,
            rng: Rng::new(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(11)),
            scope: vec![0.0; engine::SCOPE_LEN],
            pad_down: [false; 16],
            seen_fired: 0,
            flash: 0,
        }
    }

    fn sens(&self) -> f32 {
        self.sensitivity.get().max(0.01) * 10.0
    }

    /// The note a physical pad plays, rising from the bottom left.
    fn pad_note(&self, pad: usize) -> u8 {
        let scale = self.sh.params.get(P::Scale) as usize;
        let root = self.sh.params.get(P::Root) as i32;
        (48 + root + music_scales::degree(scale, kit::pad_rank(pad))).clamp(0, 127) as u8
    }

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
        std::path::PathBuf::from("saves/blaster").join(format!("slot_{:02}.json", slot + 1))
    }

    fn save_slot(&mut self) {
        let values: Vec<f32> = self.sh.params.snapshot().to_vec();
        let doc = serde_json::json!({ "name": format!("User {}", self.slot + 1), "values": values });
        let result = std::fs::create_dir_all("saves/blaster").and_then(|_| std::fs::write(Self::slot_path(self.slot), doc.to_string()));
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

    fn value(&self, i: usize) -> String {
        if i == P::Scale as usize {
            return SCALE_TYPES[self.sh.params.at(i) as usize % SCALE_TYPES.len()].0.to_string();
        }
        if i == P::Root as usize {
            return ROOT_NAMES[self.sh.params.at(i) as usize % 12].to_string();
        }
        self.sh.params.text_at(i)
    }

    /// A parameter's name, with its group where the bare name would be unclear.
    fn label(&self, i: usize) -> String {
        DEFS[i].name.to_string()
    }

    fn row_text(&self, row: Row) -> (String, String, bool) {
        match row {
            Row::Preset => ("Character".into(), format!("{} ({}/{})", self.presets[self.preset].name, self.preset + 1, self.presets.len()), false),
            Row::Slot => ("User slot".into(), format!("{}", self.slot + 1), false),
            Row::Save => ("Save to slot".into(), "press".into(), false),
            Row::Load => ("Load from slot".into(), "press".into(), false),
            Row::Randomize => ("Randomize".into(), "press".into(), false),
            Row::Mutate => ("Mutate".into(), "press".into(), false),
            Row::Init => ("Init patch".into(), "press".into(), false),
            Row::Panic => ("Silence".into(), "press".into(), false),
            Row::Group(g) => (format!("{} {}", if self.expanded[g] { "-" } else { "+" }, GROUPS[g].to_uppercase()), String::new(), true),
            Row::Param(i) => (format!("  {}", self.label(i)), self.value(i), false),
        }
    }

    fn row_edit(&mut self, row: Row, delta: i32, press: bool) {
        let sens = self.sens();
        match row {
            Row::Preset if delta != 0 => self.step_preset(delta),
            Row::Slot if delta != 0 => self.slot = (self.slot as i32 + delta.signum()).rem_euclid(SLOTS as i32) as usize,
            Row::Save if press || delta != 0 => self.save_slot(),
            Row::Load if press || delta != 0 => self.load_slot(),
            Row::Randomize if press || delta != 0 => {
                presets::randomize(&self.sh.params, &mut self.rng);
                self.status = "random blast".into();
            }
            Row::Mutate if press || delta != 0 => {
                presets::mutate(&self.sh.params, &mut self.rng);
                self.status = "mutated".into();
            }
            Row::Init if press || delta != 0 => {
                self.sh.params.reset_all();
                self.status = "init patch".into();
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

    /// Which play-view control is a parameter (all but the preset browser).
    fn control(&self, i: usize) -> Option<P> {
        match i % CONTROLS {
            C_PRESET => None,
            n if n < C_PRESET => Some(KIT[n].0),
            n => Some(KIT[n - 1].0),
        }
    }

    fn control_label(&self, i: usize) -> &'static str {
        match i % CONTROLS {
            C_PRESET => "Character",
            n if n < C_PRESET => KIT[n].1,
            n => KIT[n - 1].1,
        }
    }
}

impl PlayHost for BlasterApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS
    }
    fn kit_label(&self, i: usize) -> String {
        self.control_label(i).to_string()
    }
    fn kit_value(&self, i: usize) -> String {
        match self.control(i) {
            None => self.presets[self.preset].name.to_string(),
            Some(p) => self.value(p as usize),
        }
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        Some(match self.control(i) {
            None => self.preset as f32 / (self.presets.len() - 1).max(1) as f32,
            Some(p) => self.sh.params.norm_at(p as usize),
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        match self.control(i) {
            None => true,
            Some(p) => Params::stepped(p as usize),
        }
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match self.control(i) {
            None if delta != 0 => self.step_preset(delta),
            None => {}
            Some(p) => self.sh.params.step_at(p as usize, delta, self.sens()),
        }
    }
    fn kit_reset(&mut self, i: usize) {
        match self.control(i) {
            // back to the character as it was designed, undoing the tweaks
            None => self.load_preset(self.preset),
            Some(p) => self.sh.params.reset_at(p as usize),
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        match self.control(i) {
            None => {
                let target = (v.clamp(0.0, 1.0) * (self.presets.len() - 1) as f32).round() as usize;
                if target != self.preset {
                    self.load_preset(target);
                }
            }
            Some(p) => self.sh.params.set_norm_at(p as usize, v),
        }
    }
    /// A whole blast: every parameter, not just the play-view dials.
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
            // a held pad is green while it charges and turns yellow when it is full
            if self.sh.charge.get() >= 1.0 { PadColor::Yellow } else { PadColor::Green }
        } else if kit::pad_rank(pad) % music_scales::intervals(scale).len() == 0 {
            PadColor::Blue
        } else {
            PadColor::Off
        }
    }
    fn kit_line(&self) -> String {
        format!("{}  charge {:.0}%", self.presets[self.preset].name, self.sh.charge.get() * 100.0)
    }
}

impl App for BlasterApp {
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
        // keep rendering while a charge or blast (or the echo's tail) is going
        self.sh.active.load(Ordering::Relaxed) > 0 || self.sh.peak.get() > 0.0003
    }
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let in_menu = step.menu;
        let pads_play = step.native.is_some() || in_menu;
        let notes: [u8; 16] = std::array::from_fn(|p| self.pad_note(p));
        // notes from a keyboard or the note bus charge and fire too
        let mut keyed = step.input;
        keyed.midi_keys = input.midi_keys;
        self.pad_down = std::array::from_fn(|p| pads_play && keyed.grid[p]);
        self.keys.update(&keyed, pads_play, |p| notes[p]);
        let fired = self.sh.fired.load(Ordering::Relaxed);
        if fired != self.seen_fired {
            self.seen_fired = fired;
            self.flash = 10;
        }
        self.flash = self.flash.saturating_sub(1);
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
        // let go of whatever the pads were holding (which fires it)
        self.pad_down = [false; 16];
        self.keys.update(&Input::default(), false, |_| 0);
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if self.kit.menu {
            Text::new("BLASTER", Point::new(16, 30), MonoTextStyle::new(&SPLEEN_8X16, ACCENT)).draw(f).ok();
            if !self.status.is_empty() {
                Text::new(&self.status, Point::new(130, 30), MonoTextStyle::new(&SPLEEN_6X12, DIM)).draw(f).ok();
            }
            let rows: Vec<(String, String)> = self.menu_rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 44, 24, 11, &rows, INK, DIM, CHIP);
            return;
        }
        if let Some(col) = self.play_column() {
            let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
            kit::draw::column(f, &col, 16, 40, 350, 285, pal);
        }
        // the right-hand panel: the charge meter and what it will fire
        let (x0, w) = (392, 232);
        let small = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        Text::new("BLASTER", Point::new(x0, 56), MonoTextStyle::new(&SPLEEN_8X16, ACCENT)).draw(f).ok();
        Text::new(self.presets[self.preset].name, Point::new(x0, 78), MonoTextStyle::new(&SPLEEN_8X16, INK)).draw(f).ok();
        // charge meter, with a tick where each third of the charge is
        let charge = self.sh.charge.get().clamp(0.0, 1.0);
        let (my, mh) = (100, 34);
        let frame = if self.flash > 0 { FLASH } else { FAINT };
        Rectangle::new(Point::new(x0, my), Size::new(w as u32, mh as u32)).into_styled(PrimitiveStyle::with_stroke(frame, 2)).draw(f).ok();
        let bar = (charge * (w as f32 - 4.0)) as u32;
        let full = charge >= 1.0 && self.flash == 0;
        Rectangle::new(Point::new(x0 + 2, my + 2), Size::new(bar.max(0), (mh - 4) as u32)).into_styled(PrimitiveStyle::with_fill(if full { FLASH } else { ACCENT })).draw(f).ok();
        for third in 1..3 {
            let x = x0 + (w * third / 3);
            Line::new(Point::new(x, my), Point::new(x, my + mh)).into_styled(PrimitiveStyle::with_stroke(DIM, 1)).draw(f).ok();
        }
        Text::new(if charge >= 1.0 { "FULL" } else { "charge" }, Point::new(x0, my + mh + 14), small).draw(f).ok();
        let power = self.sh.last_power.get();
        Text::new(&format!("last blast {:.0}%", power * 100.0), Point::new(x0 + 120, my + mh + 14), small).draw(f).ok();
        // what the character does, in words
        let p = &self.sh.params;
        let lines = [
            format!("charge {} {}  climb {} oct", p.text_at(P::ChgWave as usize), p.text_at(P::ChgPitch as usize), p.text_at(P::Climb as usize)),
            format!("blast {} {} > {}", p.text_at(P::BlastWave as usize), p.text_at(P::BlastStart as usize), p.text_at(P::BlastEnd as usize)),
            format!("length {}  noise {}", p.text_at(P::Length as usize), p.text_at(P::Noise as usize)),
            if p.get(P::AutoFire) >= 0.5 { "fires itself at full charge".to_string() } else { String::new() },
        ];
        for (n, line) in lines.iter().enumerate() {
            Text::new(line, Point::new(x0, 176 + n as i32 * 13), small).draw(f).ok();
        }
        // scope
        if let Ok(s) = self.sh.scope.try_lock() {
            self.scope.copy_from_slice(&s);
        }
        let (sy, sh_) = (236, 56);
        Rectangle::new(Point::new(x0, sy), Size::new(w as u32, sh_ as u32)).into_styled(PrimitiveStyle::with_stroke(FAINT, 1)).draw(f).ok();
        let n = self.scope.len();
        let mut prev: Option<Point> = None;
        for (i, v) in self.scope.iter().enumerate() {
            let pt = Point::new(x0 + (i * (w as usize - 2) / n) as i32 + 1, sy + sh_ / 2 - (v.clamp(-1.0, 1.0) * (sh_ as f32 / 2.0 - 3.0)) as i32);
            if let Some(q) = prev {
                Line::new(q, pt).into_styled(PrimitiveStyle::with_stroke(INK, 1)).draw(f).ok();
            }
            prev = Some(pt);
        }
        let peak = (self.sh.peak.get().clamp(0.0, 1.0) * w as f32) as u32;
        Rectangle::new(Point::new(x0, 300), Size::new(peak.max(1), 6)).into_styled(PrimitiveStyle::with_fill(if peak as f32 > w as f32 * 0.9 { ACCENT } else { INK })).draw(f).ok();
        Text::new("hold a pad to charge, let go to fire   F2: layer   R1: menu", Point::new(16, 340), small).draw(f).ok();
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
    Box::new(BlasterApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}

#[cfg(test)]
mod tests;
