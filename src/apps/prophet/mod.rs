//! SZYK Prophet: an original virtual-analogue instrument inspired by the
//! Prophet-5 and PikoPiko Factory's Profree-4 hardware concept. No PikoPiko,
//! Sequential firmware, factory patches or logos are included. Two VCO-style
//! oscillators, sync, Poly-Mod, resonant 24 dB/oct filtering, two ADSRs,
//! poly/mono/unison allocation and 64 original presets share Portamax's buses.
mod dsp;
mod panel;
mod presets;
mod spec;
#[cfg(test)]
mod tests;

use crate::apps::mi_kit::Controls;
use crate::{
    app::{App, Input, SlintExtra},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    util::AtomicF32,
};
use presets::{Preset, BANK_NAMES};
use serde::{Deserialize, Serialize};
use spec::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone, Copy)]
struct KeyState {
    notes: [u8; 128],
    bend: f32,
    wheel: f32,
    aftertouch: f32,
    stick_x: f32,
    stick_y: f32,
    hand: f32,
    generation: u32,
}
impl Default for KeyState {
    fn default() -> Self {
        Self {
            notes: [0; 128],
            bend: 0.,
            wheel: 0.,
            aftertouch: 0.,
            stick_x: 0.,
            stick_y: 0.,
            hand: 0.,
            generation: 0,
        }
    }
}
struct Shared {
    controls: Arc<Controls>,
    keys: Mutex<KeyState>,
    output: Arc<Mutex<Vec<f32>>>,
    mix_level: Arc<AtomicF32>,
    ext_level: Arc<AtomicF32>,
    peak: AtomicF32,
    meters: [AtomicF32; 8],
    tail: AtomicBool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPatch {
    version: u32,
    name: String,
    parameters: BTreeMap<String, f32>,
}
impl StoredPatch {
    fn from_preset(p: &Preset) -> Self {
        Self {
            version: 1,
            name: p.name.clone(),
            parameters: (0..N)
                .map(|i| (SPECS[i].name.into(), p.values[i]))
                .collect(),
        }
    }
    fn preset(self) -> Result<Preset, String> {
        if self.version != 1 {
            return Err("Unsupported patch version".into());
        }
        if self.name.trim().is_empty()
            || self.name.chars().count() > 32
            || self.parameters.len() != N
        {
            return Err("Incomplete patch".into());
        }
        let mut values = [0.; N];
        for (i, s) in SPECS.iter().enumerate() {
            let v = *self.parameters.get(s.name).ok_or("Missing parameter")?;
            if !v.is_finite() || !(s.min..=s.max).contains(&v) || (s.is_switch() && v.fract() != 0.)
            {
                return Err(format!("Invalid {}", s.name));
            }
            values[i] = v;
        }
        Ok(Preset {
            name: self.name,
            bank: "USER".into(),
            values,
        })
    }
}

pub struct ProphetApp {
    shared: Arc<Shared>,
    factory: Vec<Preset>,
    users: Vec<Option<Preset>>,
    save_dir: PathBuf,
    program: usize,
    baseline: [f32; N],
    edited: Option<[f32; N]>,
    view: usize,
    group: usize,
    selected: usize,
    bank: usize,
    user_slot: usize,
    status: String,
    input: Input,
    pointer_note: Option<u8>,
    drag: Option<(usize, f32, f32)>,
    generation: u32,
}
impl ProphetApp {
    pub fn new(mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        Self::with_dir(mods, bus, mixer, Path::new("saves/prophet"))
    }
    fn with_dir(mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>, dir: &Path) -> Self {
        let controls = Arc::new(Controls::new(&SPECS, "Prophet", &mods));
        let (mix_level, ext_level) = mixer.register("Prophet", &mods);
        let output = bus.register("Prophet");
        // The callback may resize within this capacity, but never allocate.
        output.lock().unwrap().reserve(8192);
        let shared = Arc::new(Shared {
            controls,
            keys: Mutex::new(KeyState::default()),
            output,
            mix_level,
            ext_level,
            peak: AtomicF32::new(0.),
            meters: std::array::from_fn(|_| AtomicF32::new(0.)),
            tail: AtomicBool::new(false),
        });
        let users = (0..16)
            .map(|i| {
                std::fs::read(Self::slot_path(dir, i))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<StoredPatch>(&b).ok())
                    .and_then(|p| p.preset().ok())
            })
            .collect();
        let factory = presets::factory();
        let baseline = factory[0].values;
        let mut a = Self {
            shared,
            factory,
            users,
            save_dir: dir.into(),
            program: 0,
            baseline,
            edited: None,
            view: 0,
            group: 3,
            selected: CUTOFF,
            bank: 0,
            user_slot: 0,
            status: "64 factory sounds / 16 user slots".into(),
            input: Input::default(),
            pointer_note: None,
            drag: None,
            generation: 0,
        };
        a.apply(&baseline);
        a
    }
    fn slot_path(dir: &Path, i: usize) -> PathBuf {
        dir.join(format!("user-{:02}.json", i + 1))
    }
    fn apply(&mut self, p: &[f32; N]) {
        for (i, &v) in p.iter().enumerate() {
            self.shared.controls.set(i, v);
        }
        self.generation = self.generation.wrapping_add(1);
        self.publish_keys();
    }
    fn values(&self) -> [f32; N] {
        std::array::from_fn(|i| self.shared.controls.raw(i))
    }
    fn preset(&self, i: usize) -> Option<&Preset> {
        if i < 64 {
            self.factory.get(i)
        } else {
            self.users.get(i - 64).and_then(|p| p.as_ref())
        }
    }
    fn name(&self) -> &str {
        self.preset(self.program)
            .map_or("Edited patch", |p| p.name.as_str())
    }
    fn dirty(&self) -> bool {
        self.edited.is_some() || self.values() != self.baseline
    }
    fn load(&mut self, i: usize) {
        let Some(p) = self.preset(i) else {
            self.status = "Empty user slot: SAVE writes here".into();
            return;
        };
        let values = p.values;
        let name = p.name.clone();
        self.edited = None;
        self.program = i;
        self.baseline = values;
        self.apply(&values);
        self.status = format!("Loaded {name}");
    }
    fn compare(&mut self) {
        if let Some(p) = self.edited.take() {
            self.apply(&p);
            self.status = "Edited sound restored".into();
        } else {
            self.edited = Some(self.values());
            let p = self.baseline;
            self.apply(&p);
            self.status = "COMPARE: original preset".into();
        }
    }
    fn leave_compare(&mut self) {
        if let Some(p) = self.edited.take() {
            self.apply(&p);
        }
    }
    fn save(&mut self) -> Result<(), String> {
        self.leave_compare();
        let p = Preset {
            name: format!(
                "{} U{:02}",
                self.name().chars().take(26).collect::<String>(),
                self.user_slot + 1
            ),
            bank: "USER".into(),
            values: self.values(),
        };
        let bytes =
            serde_json::to_vec_pretty(&StoredPatch::from_preset(&p)).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&self.save_dir).map_err(|e| e.to_string())?;
        let path = Self::slot_path(&self.save_dir, self.user_slot);
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &path).map_err(|e| e.to_string())?;
        self.baseline = p.values;
        self.users[self.user_slot] = Some(p);
        self.program = 64 + self.user_slot;
        self.status = format!("Saved user {:02}", self.user_slot + 1);
        Ok(())
    }
    fn save_action(&mut self) {
        if let Err(e) = self.save() {
            self.status = format!("Save failed: {e}");
        }
    }
    fn step_program(&mut self, d: i32) {
        let mut i = self.program as i32;
        for _ in 0..80 {
            i = (i + d.signum()).rem_euclid(80);
            if self.preset(i as usize).is_some() {
                self.load(i as usize);
                break;
            }
        }
    }
    fn select_group(&mut self, g: usize) {
        self.group = g.min(GROUPS.len() - 1);
        self.selected = GROUPS[self.group].1[0];
    }
    fn edit(&mut self, d: i32) {
        self.leave_compare();
        if matches!(
            self.selected,
            A_TUNE | B_TUNE | B_FINE | OCTAVE | BEND_RANGE
        ) {
            let value = self.shared.controls.raw(self.selected).round() + d as f32;
            self.shared.controls.set(self.selected, value);
        } else {
            self.shared.controls.edit(self.selected, d, 0.5);
        }
        self.status = format!(
            "{}: {}",
            SPECS[self.selected].name,
            self.shared.controls.text(self.selected)
        );
    }
    fn publish_keys(&mut self) {
        let mut notes = self.input.midi_keys.0;
        let octave = self.shared.controls.raw(OCTAVE).round() as i32;
        if self.view != 1 {
            for (i, &on) in self.input.grid.iter().enumerate() {
                if on {
                    let n = crate::apps::mi_kit::pad_note(i, octave) as usize;
                    let pressure = if self.input.pad_pressure[i] > 0. {
                        self.input.pad_pressure[i]
                    } else {
                        0.6
                    };
                    notes[n] = notes[n].max((pressure * 127.).clamp(1., 127.) as u8);
                }
            }
        }
        if let Some(n) = self.pointer_note {
            notes[n as usize] = notes[n as usize].max(100);
        }
        *self.shared.keys.lock().unwrap() = KeyState {
            notes,
            bend: self.input.pitch_bend,
            wheel: self.input.mod_wheel,
            aftertouch: self.input.aftertouch,
            stick_x: self.input.stick[0],
            stick_y: self.input.stick[1],
            hand: self.input.hands[0],
            generation: self.generation,
        };
    }
    fn bank_name(&self) -> &str {
        if self.bank < 8 {
            BANK_NAMES[self.bank]
        } else if self.bank == 8 {
            "USER 01-08"
        } else {
            "USER 09-16"
        }
    }
}
impl App for ProphetApp {
    fn supports_pad_lock(&self) -> bool {
        true
    }
    fn play_surface(&self) -> bool {
        true
    }
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn needs_background_audio(&self) -> bool {
        self.shared.tail.load(Ordering::Relaxed)
    }
    fn on_exit(&mut self) {
        self.input = Input::default();
        self.pointer_note = None;
        self.drag = None;
        self.publish_keys();
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(match self.view {
            0 => "PRESETS",
            1 => "MENU",
            _ => "PANEL",
        })
    }
    fn toggle_grid_mode(&mut self) {
        self.view = (self.view + 1) % 3;
        self.pointer_note = None;
        self.drag = None;
        self.publish_keys();
    }
    fn hint(&self) -> String {
        "Pads: keys / D-pad: select + edit / R1: section / F2: view".into()
    }
    fn tick(&mut self, input: &Input) {
        let old = self.input;
        self.input = *input;
        if input.shoulder_press[1] {
            self.select_group((self.group + 1) % GROUPS.len());
        }
        if input.shoulder_press[0] {
            self.step_program(1);
        }
        if self.view == 1 {
            if input.knob1 + input.navigation_steps != 0 {
                self.bank = (self.bank as i32 + (input.knob1 + input.navigation_steps).signum())
                    .rem_euclid(10) as usize;
            }
            if input.knob2 != 0 {
                self.step_program(input.knob2);
            }
            for i in 0..16 {
                if input.grid[i] && !old.grid[i] {
                    if i < 8 {
                        self.load(self.bank * 8 + i);
                    } else {
                        self.bank = (i - 8).min(7);
                    }
                }
            }
        } else {
            let nav = input.knob1 + input.navigation_steps;
            if nav != 0 {
                let items = GROUPS[self.group].1;
                let at = items.iter().position(|&i| i == self.selected).unwrap_or(0) as i32;
                self.selected = items[(at + nav).rem_euclid(items.len() as i32) as usize];
            }
            if input.knob2 != 0 {
                self.edit(input.knob2);
            }
            if input.knob1_press {
                self.leave_compare();
                self.shared.controls.reset(self.selected);
            }
            if input.knob2_press {
                self.compare();
            }
        }
        self.publish_keys();
    }
    fn draw(&mut self, fb: &mut FrameBuffer) {
        panel::draw(self, fb);
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        crate::apps::kids_kit::screen_extra(&fb)
    }
    fn slint_pointer_pick(&mut self, x: f32, y: f32) {
        panel::pointer(self, x, y);
    }
    fn slint_selected(&self) -> usize {
        self.selected + 1
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        let mut rows = vec![(
            "Program".into(),
            format!(
                "{:02} {}{}",
                self.program + 1,
                self.name(),
                if self.dirty() { " *" } else { "" }
            ),
            false,
        )];
        rows.extend((0..N).map(|i| (SPECS[i].name.into(), self.shared.controls.text(i), false)));
        rows
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(dsp::Processor::new(Arc::clone(&self.shared))))
    }
}
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    Box::new(ProphetApp::new(
        ctx.get::<ModBus>(),
        ctx.get::<AudioBus>(),
        ctx.get::<MixerBus>(),
    ))
}
