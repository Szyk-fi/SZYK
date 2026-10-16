//! Native Ableton Push 2 surface. MIDI and USB run on a worker, never audio.
//! Protocol: https://github.com/Ableton/push-interface (documented commands only).
//! Included under controller so all simulator/Slint entry points share it.
use super::ControllerState;
#[cfg(test)]
use crate::display::FrameBuffer;
use crate::{
    app::{App, Input, Setting},
    spleen_fonts::{SPLEEN_12X24, SPLEEN_8X16},
    util::AtomicF32,
};
use embedded_graphics::{
    mono_font::{
        ascii::{FONT_10X20, FONT_6X10},
        MonoFont, MonoTextStyle,
    },
    pixelcolor::Rgb565,
    prelude::*,
    text::Text,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
pub fn owns_port(name: &str) -> bool {
    ACTIVE.load(Ordering::Relaxed) && is_push(name)
}
fn is_push(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("push 2") || n.contains("push2")
}
fn user_port(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("user") || n.contains("midiin2") || n.contains("midiout2") || n.ends_with(":1")
}
fn relative(v: u8) -> i32 {
    if v < 64 {
        v as i32
    } else {
        v as i32 - 128
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    App,
    Chromatic,
    Fourths,
    Scale,
    Drums,
    Grid,
}
impl Layout {
    fn next(self) -> Self {
        match self {
            Self::App => Self::Chromatic,
            Self::Chromatic => Self::Fourths,
            Self::Fourths => Self::Scale,
            Self::Scale => Self::Drums,
            Self::Drums => Self::Grid,
            Self::Grid => Self::App,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::App => "APP PADS",
            Self::Chromatic => "CHROMATIC 64",
            Self::Fourths => "FOURTHS",
            Self::Scale => "IN KEY",
            Self::Drums => "DRUMS 64",
            Self::Grid => "MONOME GRID",
        }
    }
}

pub struct Surface {
    pub layout: Layout,
    root: i32,
    octave: i32,
    scale: usize,
    grid_bank: usize,
    velocity: [u8; 64],
    pressure: [f32; 64],
    encoders: [i32; 8],
    touched: [bool; 8],
    select: [bool; 8],
    page: i32,
    shift: bool,
    setup: bool,
    focused: usize,
    target_select: bool,
    target_nav: i32,
    target_pick: Option<usize>,
    target_confirm: bool,
    target_toggle: bool,
    bend: f32,
    wheel: f32,
    pub connected: bool,
    pub display_connected: bool,
    pub display_frames: u64,
    frame: Option<Arc<OutputFrame>>,
}
impl Default for Surface {
    fn default() -> Self {
        Self {
            layout: Layout::App,
            root: 0,
            octave: 3,
            scale: 1,
            grid_bank: 0,
            velocity: [0; 64],
            pressure: [0.; 64],
            encoders: [0; 8],
            touched: [false; 8],
            select: [false; 8],
            page: 0,
            shift: false,
            setup: false,
            focused: 0,
            target_select: false,
            target_nav: 0,
            target_pick: None,
            target_confirm: false,
            target_toggle: false,
            bend: 0.,
            wheel: 0.,
            connected: false,
            display_connected: false,
            display_frames: 0,
            frame: None,
        }
    }
}
/// The shared grid lives in `apps/grid_kit`, which the small standalone
/// examples can't compile (it needs arc_kit and more). Whoever owns the real
/// grid registers it here; without it Push 2's Grid layout is simply dark.
pub struct GridLeds {
    pub rows: usize,
    pub cols: usize,
    pub leds: Vec<u8>,
}
#[derive(Clone, Copy)]
pub struct GridHooks {
    pub press: fn(usize, usize, bool),
    pub leds: fn() -> GridLeds,
}
static GRID_HOOKS: std::sync::OnceLock<GridHooks> = std::sync::OnceLock::new();
pub fn set_grid_hooks(h: GridHooks) {
    let _ = GRID_HOOKS.set(h);
}
fn grid_press(x: usize, y: usize, down: bool) {
    if let Some(h) = GRID_HOOKS.get() {
        (h.press)(x, y, down);
    }
}
fn grid_leds() -> Option<GridLeds> {
    GRID_HOOKS.get().map(|h| (h.leds)())
}

impl Surface {
    fn release(&mut self) {
        if self.layout == Layout::Grid {
            for i in 0..64 {
                if self.velocity[i] > 0 {
                    grid_press(i % 8 + self.grid_bank * 8, 7 - i / 8, false);
                }
            }
        }
        self.velocity.fill(0);
        self.pressure.fill(0.);
        self.bend = 0.;
        self.wheel = 0.;
    }
    fn disconnect(&mut self) {
        self.release();
        self.connected = false;
        self.encoders.fill(0);
        self.select.fill(false);
        self.touched.fill(false);
        self.shift = false;
        self.page = 0;
        self.target_select = false;
        self.target_nav = 0;
        self.target_pick = None;
        self.target_confirm = false;
        self.target_toggle = false;
    }
    fn choose_target(&mut self) {
        self.target_select = !self.target_select;
        self.target_nav = 0;
        self.target_pick = None;
        self.target_confirm = false;
        self.encoders.fill(0);
        self.select.fill(false);
        self.page = 0;
        self.setup = false;
    }
    fn request_page(&mut self, delta: i32) {
        if self.target_select {
            self.target_nav += delta * 8;
            return;
        }
        // A queued turn belongs to the page it arrived on, never the next one.
        self.encoders.fill(0);
        self.select.fill(false);
        self.page += delta * if self.shift { 8 } else { 1 };
    }
    fn change_layout(&mut self, layout: Layout) {
        self.release();
        self.layout = layout;
    }
    pub fn note(&self, i: usize) -> Option<u8> {
        if i >= 64 {
            return None;
        }
        let row = (i / 8) as i32;
        let col = (i % 8) as i32;
        let n = match self.layout {
            Layout::Chromatic | Layout::Drums => self.octave * 12 + self.root + i as i32,
            Layout::Fourths => self.octave * 12 + self.root + row * 5 + col,
            Layout::Scale => {
                self.octave * 12
                    + self.root
                    + crate::app::music_scales::degree(self.scale, (row * 3 + col) as usize)
            }
            _ => return None,
        };
        (0..=127).contains(&n).then_some(n as u8)
    }
    /// Only called for the selected Push port; low touch notes never become keys.
    fn message(&mut self, msg: &[u8], controller: &ControllerState, volume: &AtomicF32) {
        let Some(&status) = msg.first() else { return };
        let expected = match status & 0xf0 {
            0xd0 => 2,
            0x80 | 0x90 | 0xa0 | 0xb0 | 0xe0 => 3,
            _ => return,
        };
        if msg.len() != expected || msg[1..].iter().any(|v| *v > 127) {
            return;
        }
        let d = msg[1];
        let v = msg.get(2).copied().unwrap_or(0);
        match status & 0xf0 {
            0x80 | 0x90 => {
                let on = status & 0xf0 == 0x90 && v > 0;
                if d < 8 {
                    self.touched[d as usize] = on;
                } else if (36..100).contains(&d) {
                    let i = (d - 36) as usize;
                    self.velocity[i] = if on {
                        if self.shift {
                            127
                        } else {
                            v
                        }
                    } else {
                        0
                    };
                    self.pressure[i] = if on { v as f32 / 127. } else { 0. };
                    if self.layout == Layout::Grid {
                        grid_press(i % 8 + self.grid_bank * 8, 7 - i / 8, on);
                    }
                }
            }
            0xa0 if (36..100).contains(&d) => {
                let i = (d - 36) as usize;
                if self.velocity[i] > 0 {
                    self.pressure[i] = v as f32 / 127.;
                }
            }
            0xd0 => {
                for i in 0..64 {
                    if self.velocity[i] > 0 {
                        self.pressure[i] = d as f32 / 127.;
                    }
                }
            }
            0xe0 => self.bend = (((v as i32) << 7 | d as i32) - 8192) as f32 / 8192.,
            0xb0 if (71..79).contains(&d) => {
                let i = (d - 71) as usize;
                let delta = relative(v);
                if self.target_select {
                    if i == 0 {
                        self.target_nav = self.target_nav.saturating_add(delta);
                    }
                } else if self.setup {
                    if delta != 0 {
                        self.release();
                        match i {
                            0 => self.root = (self.root + delta.signum()).rem_euclid(12),
                            1 => {
                                self.scale = (self.scale as i32 + delta.signum())
                                    .rem_euclid(crate::app::music_scales::SCALE_TYPES.len() as i32)
                                    as usize
                            }
                            2 => self.octave = (self.octave + delta.signum()).clamp(0, 9),
                            3 => {
                                self.grid_bank =
                                    (self.grid_bank as i32 + delta.signum()).clamp(0, 15) as usize
                            }
                            _ => (),
                        }
                    }
                } else {
                    self.encoders[i] = self.encoders[i].saturating_add(delta).clamp(-4096, 4096);
                }
            }
            0xb0 if d == 79 => {
                volume.set((volume.get() + relative(v) as f32 * 0.005).clamp(0., 2.))
            }
            0xb0 if d == 14 => {
                let c = crate::clock::Clock::shared();
                c.set_bpm(c.bpm() + relative(v) as f32 * if self.shift { 0.1 } else { 1. });
            }
            0xb0 if d == 15 => self.request_page(relative(v).signum()),
            0xb0 if d == 1 => self.wheel = v as f32 / 127.,
            0xb0 if d == 49 => self.shift = v > 0,
            0xb0 if (102..110).contains(&d) && v > 0 => {
                if self.target_select {
                    self.target_pick = Some((d - 102) as usize);
                } else {
                    self.select[(d - 102) as usize] = true;
                }
            }
            0xb0 if v > 0 => match d {
                21 => {
                    if self.shift {
                        self.choose_target();
                    } else {
                        self.target_toggle = true;
                        self.target_select = false;
                    }
                }
                20..=23 => controller.set_top((d - 20) as usize),
                24 | 62 => self.request_page(-1),
                25 | 63 => self.request_page(1),
                26 | 31 => self.change_layout(self.layout.next()),
                27 | 58 => {
                    self.setup = !self.setup;
                    self.encoders.fill(0);
                    self.select.fill(false);
                }
                30 => controller.set_top(0),
                44 if self.target_select => self.target_nav -= 8,
                45 if self.target_select => self.target_nav += 8,
                44 => controller.add_nav_x(-1),
                45 => controller.add_nav_x(1),
                46 if self.target_select => self.target_nav -= 1,
                47 if self.target_select => self.target_nav += 1,
                48 if self.target_select => self.target_confirm = true,
                46 => controller.add_nav_delta(-1),
                47 => controller.add_nav_delta(1),
                48 => controller.set_knob1_press(),
                50 if self.shift => self.choose_target(),
                50 => {
                    self.change_layout(Layout::Fourths);
                    self.setup = false;
                }
                51 => {
                    self.change_layout(Layout::App);
                    self.setup = false;
                }
                54 | 55 => {
                    self.release();
                    self.octave = (self.octave + if d == 55 { 1 } else { -1 }).clamp(0, 9);
                }
                85 => controller.set_top(2),
                110 => {
                    self.setup = false;
                    self.target_select = false;
                }
                111 => controller.set_home(),
                112 => controller.set_top(3),
                59 => (), // This host already selected User mode, no toggle race.
                _ => (),
            },
            _ => (),
        }
    }
}

/// All app calls run on the UI thread. A menu edit is injected into its ONE
/// normal tick; direct instrument edits use the existing settings bridge.
pub struct Host {
    controller: Arc<ControllerState>,
    target: Option<usize>,
    bank: usize,
    identity: String,
    frame_due: Instant,
    banks: std::collections::HashMap<String, usize>,
    control_labels: Option<Vec<String>>,
    key_options: Vec<super::performance::Target>,
    key_names: Vec<String>,
    key_cursor: usize,
    target_select_open: bool,
    performance_name: String,
    performance_pinned: bool,
    performance_overlay: Option<[crate::led_output::PadColor; 16]>,
}
impl Host {
    pub fn new(controller: Arc<ControllerState>) -> Self {
        Self {
            controller,
            target: None,
            bank: 0,
            identity: String::new(),
            frame_due: Instant::now(),
            banks: Default::default(),
            control_labels: None,
            key_options: Vec::new(),
            key_names: Vec::new(),
            key_cursor: 0,
            target_select_open: false,
            performance_name: String::new(),
            performance_pinned: false,
            performance_overlay: None,
        }
    }
    pub fn target(&mut self, target: Option<usize>) {
        if target != self.target {
            self.controller.push2.lock().unwrap().release();
            self.target = target;
        }
    }
    /// Shared with ROVE's selector: only the play owner changes, never the view.
    pub fn update_targets(
        &mut self,
        routing: &mut super::performance::Routing,
        active: Option<usize>,
        apps: &super::performance::Apps,
    ) {
        let options = super::performance::Routing::options(apps);
        let mut s = self.controller.push2.lock().unwrap();
        if std::mem::take(&mut s.target_toggle) {
            routing.toggle(active, apps);
        }
        if s.target_select && !self.target_select_open {
            self.key_cursor = routing.selected(apps);
        } else if options != self.key_options {
            if let Some(old) = self.key_options.get(self.key_cursor) {
                self.key_cursor = options.iter().position(|i| i == old).unwrap_or(0);
            }
        }
        self.key_options = options;
        self.key_names = super::performance::Routing::option_names(apps);
        if s.target_select {
            self.key_cursor = (self.key_cursor as i32 + std::mem::take(&mut s.target_nav))
                .rem_euclid(self.key_options.len().max(1) as i32)
                as usize;
            let pick = s
                .target_pick
                .take()
                .map(|slot| self.key_cursor / 8 * 8 + slot)
                .or_else(|| std::mem::take(&mut s.target_confirm).then_some(self.key_cursor));
            if let Some(option) = pick.filter(|i| *i < self.key_options.len()) {
                routing.select(option, apps);
                s.target_select = false;
            }
        }
        self.target_select_open = s.target_select;
        self.performance_pinned = routing.pinned().is_some();
        self.performance_name = routing
            .target(active)
            .and_then(|i| apps.get(i))
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| if routing.selection() == super::performance::Target::Off {
                "DISARMED".into()
            } else {
                "NONE".into()
            });
        drop(s);
        self.target(routing.target(active));
    }
    pub fn choose_keyboard_target(&mut self) {
        self.controller.push2.lock().unwrap().choose_target();
    }
    pub fn keyboard_target_menu(&self) -> Option<(&[String], usize)> {
        self.controller
            .push2
            .lock()
            .unwrap()
            .target_select
            .then_some((&self.key_names, self.key_cursor))
    }
    pub fn keyboard_target_input(&mut self, delta: i32, confirm: bool, cancel: bool) {
        let mut s = self.controller.push2.lock().unwrap();
        s.target_nav = s.target_nav.saturating_add(delta);
        s.target_confirm |= confirm;
        if cancel {
            s.target_select = false;
        }
    }
    pub fn performance_overlay(&mut self, overlay: Option<[crate::led_output::PadColor; 16]>) {
        self.performance_overlay = overlay;
    }
    #[allow(dead_code)] // the Slint shell routes a pinned app separately
    pub fn performance_if(&self, enabled: bool, input: &mut Input) {
        if enabled {
            self.performance(input);
        }
    }
    pub fn performance(&self, input: &mut Input) {
        let s = self.controller.push2.lock().unwrap();
        if s.layout == Layout::App {
            for (p, n) in super::GRID_NOTES.iter().enumerate() {
                let i = (n - 36) as usize;
                input.grid[p] |= s.velocity[i] > 0;
                input.pad_pressure[p] = input.pad_pressure[p].max(s.pressure[i]);
            }
        } else {
            for i in 0..64 {
                if let Some(n) = s.note(i) {
                    input.midi_keys.0[n as usize] =
                        input.midi_keys.0[n as usize].max(s.velocity[i]);
                }
            }
        }
        input.aftertouch = input
            .aftertouch
            .max(s.pressure.iter().copied().fold(0., f32::max));
        if s.bend.abs() > input.pitch_bend.abs() {
            input.pitch_bend = s.bend;
        }
        input.mod_wheel = input.mod_wheel.max(s.wheel);
    }
    fn controls(app: &dyn App) -> (Vec<Setting>, bool) {
        if let Some(settings) = app.controller_settings() {
            (settings, true)
        } else {
            (
                app.slint_rows()
                    .into_iter()
                    .map(|(label, value, _)| Setting { label, value })
                    .collect(),
                false,
            )
        }
    }
    fn identify(&mut self, title: &str) {
        if self.identity != title {
            if !self.identity.is_empty() {
                self.banks.insert(self.identity.clone(), self.bank);
            }
            self.bank = *self.banks.get(title).unwrap_or(&0);
            if !self.identity.is_empty() {
                let mut s = self.controller.push2.lock().unwrap();
                s.encoders.fill(0);
                s.select.fill(false);
                s.page = 0;
            }
            self.identity = title.into();
            self.control_labels = None;
        }
    }
    pub fn prepare(&mut self, title: &str, app: &mut dyn App, input: &mut Input) {
        self.identify(title);
        let (controls, direct) = Self::controls(app);
        let mut s = self.controller.push2.lock().unwrap();
        if !s.connected {
            return;
        }
        if s.setup || s.target_select {
            return;
        }
        let labels: Vec<_> = controls.iter().map(|c| c.label.clone()).collect();
        if self
            .control_labels
            .as_ref()
            .is_some_and(|old| *old != labels)
        {
            // A menu/source change can replace the rows behind a physical knob.
            s.encoders.fill(0);
            s.select.fill(false);
        }
        self.control_labels = Some(labels.clone());
        let pages = controls.len().max(1).div_ceil(8);
        self.bank = (self.bank as i32 + s.page).rem_euclid(pages as i32) as usize;
        s.page = 0;
        let mut catalog_dirty = false;
        for slot in 0..8 {
            if catalog_dirty && (s.encoders[slot] != 0 || s.select[slot]) {
                let fresh: Vec<_> = Self::controls(app).0.into_iter().map(|c| c.label).collect();
                if fresh != labels {
                    s.encoders.fill(0);
                    s.select.fill(false);
                    self.control_labels = Some(fresh);
                    break;
                }
                catalog_dirty = false;
            }
            let index = self.bank * 8 + slot;
            let select = std::mem::take(&mut s.select[slot]);
            if index >= controls.len() {
                s.encoders[slot] = 0;
                continue;
            }
            if select || s.touched[slot] {
                s.focused = slot;
            }
            if direct {
                let delta = std::mem::take(&mut s.encoders[slot]);
                if select || delta != 0 {
                    app.controller_select(index);
                }
                if delta != 0 {
                    app.controller_adjust(index, delta);
                }
                catalog_dirty |= select || delta != 0;
                // Controller catalogs can use different indices from the
                // sequencer-facing settings and visual play column.
            } else if s.encoders[slot] != 0 || select {
                let delta = std::mem::take(&mut s.encoders[slot]);
                input.controller_edit = true;
                input.controller_row = Some(index);
                input.knob1 = 0;
                input.nav_x = 0;
                input.navigation_steps = index as i32 - app.slint_selected() as i32;
                input.knob2 += delta;
                if select && app.slint_rows().get(index).is_some_and(|r| r.2) {
                    input.knob1_press = true;
                }
                break; // Remaining encoder deltas stay queued for the next tick.
            }
        }
    }
    pub fn home(&mut self) {
        self.identify("ROVE");
        // Browse with encoder 1, select with its upper display button.
        let mut s = self.controller.push2.lock().unwrap();
        if s.target_select {
            drop(s);
            self.present("ROVE", None);
            return;
        }
        self.bank = 0;
        let nav = std::mem::take(&mut s.encoders[0]);
        if nav != 0 {
            self.controller.add_nav_delta(nav.signum());
        }
        if std::mem::take(&mut s.select[0]) {
            self.controller.set_knob1_press();
        }
        drop(s);
        self.present("ROVE", None);
    }
    pub fn present(&mut self, title: &str, mut app: Option<&mut dyn App>) {
        if !ACTIVE.load(Ordering::Relaxed) || Instant::now() < self.frame_due {
            return;
        }
        self.frame_due = Instant::now() + Duration::from_millis(50);
        let controls = if let Some(a) = app.as_deref_mut() {
            Self::controls(a).0
        } else {
            Vec::new()
        };
        let overlay = self.performance_overlay.unwrap_or_else(|| {
            app.as_ref()
                .map(|a| a.grid_led_overlay())
                .unwrap_or_default()
        });
        let running = app.as_ref().and_then(|a| a.running());
        let mut s = self.controller.push2.lock().unwrap();
        let settings = [
            Setting {
                label: "Root".into(),
                value: crate::app::music_scales::ROOT_NAMES[s.root as usize].into(),
            },
            Setting {
                label: "Scale".into(),
                value: crate::app::music_scales::SCALE_TYPES[s.scale].0.into(),
            },
            Setting {
                label: "Octave".into(),
                value: (s.octave - 1).to_string(),
            },
            Setting {
                label: "Grid bank".into(),
                value: (s.grid_bank + 1).to_string(),
            },
        ];
        let total = if s.target_select {
            self.key_names.len()
        } else if s.setup {
            settings.len()
        } else {
            controls.len()
        };
        let pages = total.max(1).div_ceil(8);
        if !s.setup && !s.target_select {
            self.bank = self.bank.min(pages - 1);
        }
        let bank = if s.target_select {
            self.key_cursor / 8
        } else if s.setup {
            0
        } else {
            self.bank
        };
        let shown: Vec<_> = if s.target_select {
            self.key_names
                .iter()
                .enumerate()
                .skip(bank * 8)
                .take(8)
                .map(|(i, n)| Setting {
                    label: format!("MIDI ARM / {n}"),
                    value: if i == 0 {
                        "AUTO"
                    } else if i == 1 {
                        "OFF"
                    } else {
                        "ARM"
                    }
                    .into(),
                })
                .collect()
        } else if s.setup {
            settings.to_vec()
        } else {
            controls.into_iter().skip(bank * 8).take(8).collect()
        };
        let mut pixels = vec![0x0b1017; 960 * 160];
        let start = if total == 0 { 0 } else { bank * 8 + 1 };
        let label = format!(
            "EDIT: {} | MIDI/PADS: {}{} | {} {}-{} / {} | PAGE {}/{}{}",
            title,
            self.performance_name,
            if self.performance_pinned {
                " [ARM]"
            } else {
                ""
            },
            if s.target_select { "TARGETS" } else { "PARAMS" },
            start,
            (bank * 8 + shown.len()).min(total),
            total,
            bank + 1,
            pages,
            if running == Some(true) { " | PLAY" } else { "" }
        );
        draw_text(&mut pixels, &label, 5, 13, 0xb8ead7);
        for (i, c) in shown.iter().enumerate() {
            let x = 5 + i * 120;
            let (group, name) = c.label.rsplit_once(" / ").unwrap_or(("", c.label.as_str()));
            let highlight = if s.target_select {
                self.key_cursor % 8 == i
            } else {
                s.touched[i] || s.focused == i
            };
            let ink = if highlight { 0xffc864 } else { 0xb8ead7 };
            draw_text(
                &mut pixels,
                &format!("{} {}", i + 1, group)
                    .chars()
                    .take(18)
                    .collect::<String>(),
                x,
                31,
                ink,
            );
            draw_parameter_name(&mut pixels, name, x);
            let value_font = match c.value.chars().count() {
                0..=9 => &SPLEEN_12X24,
                10..=11 => &FONT_10X20,
                _ => &SPLEEN_8X16,
            };
            draw_text_font(
                &mut pixels,
                &c.value
                    .chars()
                    .take(112 / value_font.character_size.width as usize)
                    .collect::<String>(),
                x,
                99,
                0xf1f4fa,
                value_font,
            );
        }
        let detail = shown
            .get(if s.target_select {
                self.key_cursor % 8
            } else {
                s.focused
            })
            .map(|c| if s.target_select {
                format!("SELECT: {} = {}", c.label, c.value)
            } else {
                format!("KNOB {}: {} = {}", s.focused + 1, c.label, c.value)
            })
            .unwrap_or_default();
        draw_text(&mut pixels, &detail, 5, 119, 0xffc864);
        let hint = if s.target_select {
            "ENC 1: MIDI ARM TARGET | UPPER 1-8: ARM | DEVICE: CANCEL | MIDI DISARMED: OFF | FOLLOW SCREEN: AUTO".into()
        } else if s.setup {
            "ENC 1 ROOT / 2 SCALE / 3 OCTAVE / 4 GRID BANK".into()
        } else if app.is_none() {
            "ENC 1 BROWSE / UPPER 1 OPEN / SETUP SETTINGS".into()
        } else {
            format!(
                "{} | PAGE/SWING: PARAMETERS | LOWER 2: MIDI ARM | SHIFT+LOWER 2 / NOTE: ARM TARGET",
                s.layout.label()
            )
        };
        draw_text(&mut pixels, &hint, 5, 137, 0x83b6c9);
        for (i, t) in [
            "HOME",
            if self.performance_pinned {
                "DISARM"
            } else {
                "MIDI ARM"
            },
            "PLAY",
            "MIXER",
            "< PAGE",
            "PAGE >",
            "LAYOUT",
            "SCALE",
        ]
        .iter()
        .enumerate()
        {
            draw_text(&mut pixels, t, 5 + i * 120, 156, 0xb8ead7);
        }
        let mut leds = [0u8; 64];
        let grid = (s.layout == Layout::Grid).then(grid_leds).flatten();
        for i in 0..64 {
            leds[i] = if s.velocity[i] > 0 {
                5
            } else if let Some(g) = grid.as_ref() {
                let x = i % 8 + s.grid_bank * 8;
                let y = 7 - i / 8;
                if x < g.cols && y < g.rows && g.leds[y * g.cols + x] > 0 {
                    13
                } else {
                    0
                }
            } else if let Some(n) = s.note(i) {
                if n as i32 % 12 == s.root {
                    47
                } else if s.layout == Layout::Drums {
                    if i / 16 % 2 == 0 {
                        13
                    } else {
                        47
                    }
                } else {
                    3
                }
            } else if let Some(p) = super::GRID_NOTES.iter().position(|n| *n == 36 + i as u8) {
                overlay[p].velocity()
            } else {
                0
            };
        }
        let buttons = std::array::from_fn(|i| {
            if i < 8 {
                if i < shown.len() {
                    if s.touched[i] {
                        127
                    } else {
                        20
                    }
                } else {
                    0
                }
            } else {
                if i == 9 && self.performance_pinned {
                    5
                } else if i == 10 && running == Some(true) {
                    13
                } else {
                    30
                }
            }
        });
        s.frame = Some(Arc::new(OutputFrame {
            pixels,
            leds,
            buttons,
        }));
    }
}

fn draw_parameter_name(pixels: &mut [u32], mut name: &str, x: usize) {
    // Two lines preserve useful names at a readable size without crossing knobs.
    for line in 0..2 {
        name = name.trim_start();
        let end = name.char_indices().nth(14).map_or(name.len(), |(i, _)| i);
        let split = if line == 0 && end < name.len() {
            name[..end].rfind(' ').filter(|&i| i > 0).unwrap_or(end)
        } else {
            end
        };
        draw_text_font(
            pixels,
            &name[..split],
            x,
            50 + line * 18,
            0xf1f4fa,
            &SPLEEN_8X16,
        );
        name = &name[split..];
        if name.is_empty() {
            break;
        }
    }
}

fn draw_text(pixels: &mut [u32], text: &str, x: usize, y: usize, color: u32) {
    draw_text_font(pixels, text, x, y, color, &FONT_6X10);
}

fn draw_text_font(
    pixels: &mut [u32],
    text: &str,
    x: usize,
    y: usize,
    color: u32,
    font: &'static MonoFont<'static>,
) {
    struct Canvas<'a>(&'a mut [u32]);
    impl OriginDimensions for Canvas<'_> {
        fn size(&self) -> Size {
            Size::new(960, 160)
        }
    }
    impl DrawTarget for Canvas<'_> {
        type Color = Rgb565;
        type Error = core::convert::Infallible;
        fn draw_iter<I: IntoIterator<Item = Pixel<Rgb565>>>(
            &mut self,
            p: I,
        ) -> Result<(), Self::Error> {
            for Pixel(p, c) in p {
                if p.x >= 0 && p.y >= 0 && p.x < 960 && p.y < 160 {
                    self.0[p.y as usize * 960 + p.x as usize] = ((c.r() as u32 * 255 / 31) << 16)
                        | ((c.g() as u32 * 255 / 63) << 8)
                        | (c.b() as u32 * 255 / 31);
                }
            }
            Ok(())
        }
    }
    let limit = (960 - x) / font.character_size.width as usize;
    let ascii: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .take(limit)
        .collect();
    let c = Rgb565::new(
        ((color >> 16) & 255) as u8 >> 3,
        ((color >> 8) & 255) as u8 >> 2,
        (color & 255) as u8 >> 3,
    );
    Text::new(
        &ascii,
        Point::new(x as i32, y as i32),
        MonoTextStyle::new(font, c),
    )
    .draw(&mut Canvas(pixels))
    .ok();
}

struct OutputFrame {
    pixels: Vec<u32>,
    leds: [u8; 64],
    buttons: [u8; 16],
}
const HEADER: [u8; 16] = [0xff, 0xcc, 0xaa, 0x88, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
/// Documented BGR565 little-endian, 128 filler bytes per row, shaping XOR.
pub fn encode_frame(pixels: &[u32]) -> Vec<u8> {
    assert_eq!(pixels.len(), 960 * 160);
    let mut data = vec![0; 2048 * 160];
    for y in 0..160 {
        for x in 0..960 {
            let p = pixels[y * 960 + x];
            let r = (p >> 16) & 255;
            let g = (p >> 8) & 255;
            let b = p & 255;
            let word = ((b >> 3) << 11) | ((g >> 2) << 5) | (r >> 3);
            let i = y * 2048 + x * 2;
            data[i] = word as u8;
            data[i + 1] = (word >> 8) as u8;
        }
    }
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= [0xe7, 0xf3, 0xe7, 0xff][i % 4];
    }
    data
}
#[cfg(unix)]
mod usb {
    use super::*;
    use std::ffi::{c_int, c_uchar, c_void};
    // libusb is loaded at runtime: MIDI/ordinary builds work without it installed.
    // Stable libusb-1.0 ABI; claim only interface 0, leave MIDI to the OS driver.
    type Init = unsafe extern "C" fn(*mut *mut c_void) -> c_int;
    type Exit = unsafe extern "C" fn(*mut c_void);
    type Open = unsafe extern "C" fn(*mut c_void, u16, u16) -> *mut c_void;
    type Close = unsafe extern "C" fn(*mut c_void);
    type Interface = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
    type Bulk =
        unsafe extern "C" fn(*mut c_void, c_uchar, *mut c_uchar, c_int, *mut c_int, u32) -> c_int;
    pub struct UsbDisplay {
        library: *mut c_void,
        context: *mut c_void,
        handle: *mut c_void,
        exit: Exit,
        close: Close,
        release: Interface,
        bulk: Bulk,
    }
    impl UsbDisplay {
        pub fn open() -> Option<Self> {
            unsafe {
                let mut library = std::ptr::null_mut();
                for name in [
                    b"/opt/homebrew/lib/libusb-1.0.dylib\0".as_slice(),
                    b"/usr/local/lib/libusb-1.0.dylib\0",
                    b"libusb-1.0.so.0\0",
                    b"libusb-1.0.dylib\0",
                ] {
                    library = libc::dlopen(name.as_ptr().cast(), libc::RTLD_NOW | libc::RTLD_LOCAL);
                    if !library.is_null() {
                        break;
                    }
                }
                if library.is_null() {
                    return None;
                }
                macro_rules! sym {
                    ($name:literal,$ty:ty) => {{
                        let ptr = libc::dlsym(library, concat!($name, "\0").as_ptr().cast());
                        if ptr.is_null() {
                            libc::dlclose(library);
                            return None;
                        }
                        std::mem::transmute::<*mut c_void, $ty>(ptr)
                    }};
                }
                let init = sym!("libusb_init", Init);
                let exit = sym!("libusb_exit", Exit);
                let open = sym!("libusb_open_device_with_vid_pid", Open);
                let close = sym!("libusb_close", Close);
                let claim = sym!("libusb_claim_interface", Interface);
                let release = sym!("libusb_release_interface", Interface);
                let bulk = sym!("libusb_bulk_transfer", Bulk);
                let mut context = std::ptr::null_mut();
                if init(&mut context) != 0 {
                    libc::dlclose(library);
                    return None;
                }
                let handle = open(context, 0x2982, 0x1967);
                if handle.is_null() {
                    exit(context);
                    libc::dlclose(library);
                    return None;
                }
                if claim(handle, 0) != 0 {
                    close(handle);
                    exit(context);
                    libc::dlclose(library);
                    return None;
                }
                Some(Self {
                    library,
                    context,
                    handle,
                    exit,
                    close,
                    release,
                    bulk,
                })
            }
        }
        pub fn send(&self, pixels: &[u32]) -> Result<(), i32> {
            let mut header = HEADER;
            self.write(&mut header)?;
            for chunk in encode_frame(pixels).chunks_mut(16384) {
                self.write(chunk)?;
            }
            Ok(())
        }
        fn write(&self, bytes: &mut [u8]) -> Result<(), i32> {
            let mut count = 0;
            let result = unsafe {
                (self.bulk)(
                    self.handle,
                    1,
                    bytes.as_mut_ptr(),
                    bytes.len() as c_int,
                    &mut count,
                    100,
                )
            };
            if result != 0 {
                Err(result)
            } else if count != bytes.len() as c_int {
                Err(-1)
            } else {
                Ok(())
            }
        }
    }
    impl Drop for UsbDisplay {
        fn drop(&mut self) {
            unsafe {
                (self.release)(self.handle, 0);
                (self.close)(self.handle);
                (self.exit)(self.context);
                libc::dlclose(self.library);
            }
        }
    }
}
#[cfg(not(unix))]
mod usb {
    pub struct UsbDisplay;
    impl UsbDisplay {
        pub fn open() -> Option<Self> {
            None
        }
        pub fn send(&self, _: &[u32]) -> Result<(), i32> {
            Err(-12)
        }
    }
}
use usb::UsbDisplay;

/// Owns exactly ONE Push MIDI port, preferring User. No LED writes to synths.
/// Reconnects MIDI and display independently, so USB errors leave pads usable.
pub struct Device {
    stop: Arc<AtomicBool>,
    workers: Vec<std::thread::JoinHandle<()>>,
}
impl Device {
    pub fn start(controller: Arc<ControllerState>, volume: Arc<AtomicF32>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        if std::env::var("PORTAMAX_PUSH2").as_deref() == Ok("0") {
            return Self {
                stop,
                workers: Vec::new(),
            };
        }
        ACTIVE.store(true, Ordering::Relaxed);
        let done = stop.clone();
        let midi_controller = controller.clone();
        let worker = std::thread::spawn(move || {
            let controller = midi_controller;
            eprintln!("Push 2: discovering MIDI ports (USB display runs independently)");
            let mut input = None;
            let mut output = None;
            let mut last_scan = Instant::now() - Duration::from_secs(3);
            let mut last_leds = [255u8; 64];
            let mut last_buttons = [255u8; 16];
            while !done.load(Ordering::Relaxed) {
                if last_scan.elapsed() >= Duration::from_secs(2) {
                    last_scan = Instant::now();
                    if output.is_none() {
                        let probe = midir::MidiOutput::new("rove-push2").ok();
                        if let Some(probe) = probe {
                            let mut ports: Vec<_> = probe
                                .ports()
                                .into_iter()
                                .filter_map(|p| {
                                    let n = probe.port_name(&p).ok()?;
                                    is_push(&n).then_some((p, n))
                                })
                                .collect();
                            ports.sort_by_key(|(_, n)| !user_port(n));
                            if let Some((p, n)) = ports.first() {
                                if let Ok(mut out) = probe.connect(p, "rove-push2-out") {
                                    out.send(&[0xf0, 0, 0x21, 0x1d, 1, 1, 0x0a, 1, 0xf7]).ok();
                                    out.send(&[0xf0, 0, 0x21, 0x1d, 1, 1, 0x1e, 1, 0xf7]).ok();
                                    eprintln!("Push 2: MIDI connected ({n})");
                                    output = Some(out);
                                    last_leds.fill(255);
                                    last_buttons.fill(255);
                                }
                            }
                        }
                    }
                    if input.is_none() {
                        if let Ok(mut probe) = midir::MidiInput::new("rove-push2") {
                            probe.ignore(midir::Ignore::None);
                            let mut ports: Vec<_> = probe
                                .ports()
                                .into_iter()
                                .filter_map(|p| {
                                    let n = probe.port_name(&p).ok()?;
                                    is_push(&n).then_some((p, n))
                                })
                                .collect();
                            ports.sort_by_key(|(_, n)| !user_port(n));
                            if let Some((p, _)) = ports.first() {
                                let c = controller.clone();
                                let vol = volume.clone();
                                input = probe
                                    .connect(
                                        p,
                                        "rove-push2-in",
                                        move |_, msg, _| {
                                            c.push2.lock().unwrap().message(msg, &c, &vol);
                                        },
                                        (),
                                    )
                                    .ok();
                            }
                        }
                    }
                    // CoreMIDI can retain dead handles after unplugging.
                    let present = midir::MidiInput::new("rove-push2-probe")
                        .ok()
                        .is_some_and(|p| {
                            p.ports()
                                .iter()
                                .any(|port| p.port_name(port).ok().is_some_and(|n| is_push(&n)))
                        });
                    if !present {
                        input = None;
                        output = None;
                    }
                    let mut s = controller.push2.lock().unwrap();
                    if !present {
                        s.disconnect();
                    }
                    s.connected = input.is_some() && output.is_some();
                }

                let frame = controller.push2.lock().unwrap().frame.clone();
                if let Some(frame) = frame {
                    let mut failed = false;
                    if let Some(out) = output.as_mut() {
                        for i in 0..64 {
                            if last_leds[i] != frame.leds[i] {
                                failed |= out.send(&[0x90, 36 + i as u8, frame.leds[i]]).is_err();
                                last_leds[i] = frame.leds[i];
                            }
                        }
                        for i in 0..16 {
                            if last_buttons[i] != frame.buttons[i] {
                                let cc = if i < 8 {
                                    102 + i as u8
                                } else {
                                    20 + (i - 8) as u8
                                };
                                failed |= out.send(&[0xb0, cc, frame.buttons[i]]).is_err();
                                last_buttons[i] = frame.buttons[i];
                            }
                        }
                    }
                    if failed {
                        output = None;
                        input = None;
                        let mut s = controller.push2.lock().unwrap();
                        s.disconnect();
                    }
                }
                std::thread::sleep(Duration::from_millis(8));
            }
            controller.push2.lock().unwrap().disconnect();
            if let Some(out) = output.as_mut() {
                for n in 36..100 {
                    out.send(&[0x90, n, 0]).ok();
                }
                for cc in (20..28).chain(102..110) {
                    out.send(&[0xb0, cc, 0]).ok();
                }
                out.send(&[0xf0, 0, 0x21, 0x1d, 1, 1, 0x1e, 0, 0xf7]).ok();
                out.send(&[0xf0, 0, 0x21, 0x1d, 1, 1, 0x0a, 0, 0xf7]).ok();
            }
            ACTIVE.store(false, Ordering::Relaxed);
        });
        let done = stop.clone();
        let display_controller = controller.clone();
        let display_worker = std::thread::spawn(move || {
            let mut usb = None;
            let mut scan = Instant::now() - Duration::from_secs(3);
            let mut reported = false;
            while !done.load(Ordering::Relaxed) {
                if usb.is_none() && scan.elapsed() >= Duration::from_secs(2) {
                    scan = Instant::now();
                    usb = UsbDisplay::open();
                    if usb.is_some() {
                        eprintln!("Push 2: USB display connected");
                        reported = false;
                    } else if !reported {
                        eprintln!("Push 2: USB display not available to this process (libusb/access/device in use)");
                        reported = true;
                    }
                    display_controller.push2.lock().unwrap().display_connected = usb.is_some();
                }
                let frame = display_controller.push2.lock().unwrap().frame.clone();
                if let (Some(display), Some(frame)) = (usb.as_ref(), frame) {
                    match display.send(&frame.pixels) {
                        Ok(()) => display_controller.push2.lock().unwrap().display_frames += 1,
                        Err(e) => {
                            eprintln!("Push 2 display transfer: {e}");
                            usb = None;
                            display_controller.push2.lock().unwrap().display_connected = false;
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            display_controller.push2.lock().unwrap().display_connected = false;
        });
        Self {
            stop,
            workers: vec![worker, display_worker],
        }
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // CoreMIDI discovery can block inside the OS. It must not make closing
        // the app block as well. Finished workers are joined; stuck OS calls
        // clean up when they return, or when this process exits.
        let deadline = Instant::now() + Duration::from_millis(200);
        while self.workers.iter().any(|w| !w.is_finished()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        for worker in self.workers.drain(..) {
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn send(c: &ControllerState, msg: &[u8]) {
        c.push2.lock().unwrap().message(msg, c, &AtomicF32::new(1.));
    }
    fn make() -> (Arc<ControllerState>, Host) {
        let c = Arc::new(ControllerState::new());
        c.push2.lock().unwrap().connected = true;
        let h = Host::new(c.clone());
        (c, h)
    }
    #[test]
    fn chromatic_plays_all_64_distinct_notes_and_releases_every_one() {
        let (c, h) = make();
        c.push2.lock().unwrap().layout = Layout::Chromatic;
        for n in 36..100 {
            send(&c, &[0x90, n, 100]);
        }
        let mut input = Input::default();
        h.performance(&mut input);
        assert_eq!(input.midi_keys.0.iter().filter(|v| **v > 0).count(), 64);
        assert!(input.midi_keys.0[36..100].iter().all(|v| *v == 100));
        for n in 36..100 {
            send(&c, &[0x90, n, 0]);
        }
        let mut input = Input::default();
        h.performance(&mut input);
        assert!(input.midi_keys.0.iter().all(|v| *v == 0));
    }
    #[test]
    fn overlapping_fourths_keys_keep_sounding_until_last_pad_releases() {
        let (c, h) = make();
        c.push2.lock().unwrap().layout = Layout::Fourths;
        send(&c, &[0x90, 41, 100]);
        send(&c, &[0x90, 44, 80]);
        send(&c, &[0x80, 41, 0]);
        let mut i = Input::default();
        h.performance(&mut i);
        assert_eq!(i.midi_keys.0[41], 80);
        send(&c, &[0x80, 44, 0]);
        let mut i = Input::default();
        h.performance(&mut i);
        assert_eq!(i.midi_keys.0[41], 0);
    }
    #[test]
    fn scale_notes_remain_in_key_and_out_of_range_pads_are_silent() {
        let mut s = Surface::default();
        s.layout = Layout::Scale;
        s.root = 2;
        s.scale = 2;
        for i in 0..64 {
            let n = s.note(i).unwrap();
            assert!(
                crate::app::music_scales::intervals(2).contains(&((n as i32 - 2).rem_euclid(12)))
            );
        }
        s.layout = Layout::Chromatic;
        s.root = 0;
        s.octave = 9;
        assert_eq!(s.note(19), Some(127));
        assert_eq!(s.note(20), None);
    }
    #[test]
    fn touching_encoders_never_plays_notes_and_relative_magnitude_survives() {
        let (c, h) = make();
        for n in 0..8 {
            send(&c, &[0x90, n, 127]);
            send(&c, &[0xb0, 71 + n, 3]);
        }
        let mut i = Input::default();
        h.performance(&mut i);
        assert!(!i.grid.iter().any(|v| *v));
        assert!(i.midi_keys.0.iter().all(|v| *v == 0));
        assert_eq!(c.push2.lock().unwrap().encoders, [3; 8]);
        send(&c, &[0xb0, 71, 127]);
        assert_eq!(c.push2.lock().unwrap().encoders[0], 2);
    }
    #[test]
    fn pressure_and_pitch_reach_the_performance_input_without_changing_velocity() {
        let (c, h) = make();
        c.push2.lock().unwrap().layout = Layout::Chromatic;
        send(&c, &[0x90, 36, 70]);
        send(&c, &[0xa0, 36, 110]);
        send(&c, &[0xe0, 127, 127]);
        let mut i = Input::default();
        h.performance(&mut i);
        assert_eq!(i.midi_keys.0[36], 70);
        assert!((i.aftertouch - 110. / 127.).abs() < 1e-6);
        assert!(i.pitch_bend > 0.99);
    }
    #[test]
    fn layout_octave_target_changes_release_notes_and_preserve_external_keyboard() {
        let (c, mut h) = make();
        c.push2.lock().unwrap().layout = Layout::Chromatic;
        send(&c, &[0x90, 36, 100]);
        h.target(Some(4));
        let mut i = Input::default();
        i.midi_keys.0[36] = 90;
        h.performance(&mut i);
        assert_eq!(i.midi_keys.0[36], 90);
        send(&c, &[0x90, 36, 100]);
        send(&c, &[0xb0, 55, 127]);
        let mut i = Input::default();
        h.performance(&mut i);
        assert!(!i.midi_keys.0.iter().any(|v| *v > 0));
        send(&c, &[0x90, 36, 100]);
        send(&c, &[0xb0, 31, 127]);
        assert!(c.push2.lock().unwrap().velocity.iter().all(|v| *v == 0));
    }
    #[test]
    fn app_mode_preserves_grid_orientation_and_no_longer_uses_pads_as_f_buttons() {
        let (c, h) = make();
        send(&c, &[0x90, 60, 90]);
        send(&c, &[0x90, 36, 80]);
        send(&c, &[0x90, 68, 100]);
        let mut i = Input::default();
        h.performance(&mut i);
        assert!(i.grid[0] && i.grid[12]);
        assert_eq!(i.grid.iter().filter(|v| **v).count(), 2);
        assert!(!c.take_top(0));
        send(&c, &[0xb0, 20, 127]);
        assert!(c.take_top(0));
    }
    #[test]
    fn disconnect_clears_notes_modifiers_and_queued_actions() {
        let (c, _) = make();
        send(&c, &[0xb0, 49, 127]);
        send(&c, &[0x90, 0, 127]);
        send(&c, &[0x90, 36, 100]);
        send(&c, &[0xb0, 71, 3]);
        let mut s = c.push2.lock().unwrap();
        s.disconnect();
        assert!(!s.connected && !s.shift);
        assert!(s.velocity.iter().all(|v| *v == 0));
        assert!(s.touched.iter().all(|v| !*v));
        assert_eq!(s.encoders, [0; 8]);
    }
    struct Menu {
        values: Vec<i32>,
        sel: usize,
        ticks: usize,
    }
    impl App for Menu {
        fn tick(&mut self, i: &Input) {
            self.ticks += 1;
            self.sel = (self.sel as i32 + i.navigation_steps).rem_euclid(self.values.len() as i32)
                as usize;
            self.values[self.sel] += i.knob2;
        }
        fn draw(&mut self, _: &mut FrameBuffer) {
            panic!("Push must never render the app screen");
        }
        fn slint_rows(&self) -> Vec<(String, String, bool)> {
            self.values
                .iter()
                .enumerate()
                .map(|(n, v)| (format!("Control {n}"), v.to_string(), false))
                .collect()
        }
        fn slint_selected(&self) -> usize {
            self.sel
        }
    }
    #[test]
    fn menu_banks_edit_eight_distinct_parameters_once_per_normal_tick() {
        let (c, mut h) = make();
        let mut app = Menu {
            values: vec![0; 18],
            sel: 0,
            ticks: 0,
        };
        for n in 71..79 {
            send(&c, &[0xb0, n, 2]);
        }
        for _ in 0..8 {
            let mut i = Input::default();
            h.prepare("Test", &mut app, &mut i);
            assert!(i.controller_edit);
            app.tick(&i);
        }
        assert_eq!(&app.values[..8], &[2; 8]);
        assert_eq!(app.ticks, 8);
        send(&c, &[0xb0, 63, 127]);
        send(&c, &[0xb0, 78, 127]);
        let mut i = Input::default();
        h.prepare("Test", &mut app, &mut i);
        app.tick(&i);
        assert_eq!(app.values[15], -1);
    }
    #[test]
    fn page_buttons_and_swing_reach_every_parameter_and_wrap_partial_banks() {
        let (c, mut h) = make();
        let mut app = Menu {
            values: vec![0; 18],
            sel: 0,
            ticks: 0,
        };
        for page in 0..3 {
            for slot in 0..8 {
                send(&c, &[0xb0, 71 + slot, 1]);
                let mut input = Input::default();
                h.prepare("Test", &mut app, &mut input);
                app.tick(&input);
            }
            assert_eq!(h.bank, page);
            send(&c, &[0xb0, 63, 127]);
        }
        h.prepare("Test", &mut app, &mut Input::default());
        assert_eq!(app.values, vec![1; 18]);
        assert_eq!(h.bank, 0);
        send(&c, &[0xb0, 15, 127]);
        h.prepare("Test", &mut app, &mut Input::default());
        assert_eq!(h.bank, 2);
        send(&c, &[0xb0, 49, 127]);
        send(&c, &[0xb0, 63, 127]);
        h.prepare("Test", &mut app, &mut Input::default());
        assert_eq!(h.bank, 1, "Shift moves eight pages with wraparound");
    }
    #[test]
    fn changing_page_discards_turns_from_the_old_bank() {
        let (c, mut h) = make();
        let mut app = Menu {
            values: vec![0; 18],
            sel: 0,
            ticks: 0,
        };
        send(&c, &[0xb0, 71, 7]);
        send(&c, &[0xb0, 63, 127]);
        send(&c, &[0xb0, 71, 2]);
        let mut input = Input::default();
        h.prepare("Test", &mut app, &mut input);
        app.tick(&input);
        assert_eq!(app.values[0], 0);
        assert_eq!(app.values[8], 2);
    }
    #[test]
    fn catalog_changes_do_not_retarget_simultaneous_turns() {
        struct Dynamic {
            mode: bool,
            value: i32,
        }
        impl App for Dynamic {
            fn tick(&mut self, _: &Input) {}
            fn draw(&mut self, _: &mut FrameBuffer) {
                panic!("no app image");
            }
            fn controller_settings(&self) -> Option<Vec<Setting>> {
                Some(vec![
                    Setting {
                        label: "Mode".into(),
                        value: self.mode.to_string(),
                    },
                    Setting {
                        label: if self.mode { "New" } else { "Old" }.into(),
                        value: self.value.to_string(),
                    },
                ])
            }
            fn controller_adjust(&mut self, index: usize, delta: i32) {
                if index == 0 {
                    self.mode = !self.mode;
                } else {
                    self.value += delta;
                }
            }
        }
        let (c, mut h) = make();
        let mut app = Dynamic {
            mode: false,
            value: 0,
        };
        send(&c, &[0xb0, 71, 1]);
        send(&c, &[0xb0, 72, 9]);
        h.prepare("Test", &mut app, &mut Input::default());
        assert!(app.mode);
        assert_eq!(app.value, 0, "the Old knob turn must not edit New");
        send(&c, &[0xb0, 72, 2]);
        h.prepare("Test", &mut app, &mut Input::default());
        assert_eq!(app.value, 2);
    }
    #[test]
    fn display_never_draws_the_app_and_keyboard_settings_preserve_the_app_page() {
        let (c, mut h) = make();
        let mut app = Menu {
            values: vec![0; 18],
            sel: 0,
            ticks: 0,
        };
        h.identify("Test");
        h.bank = 2;
        ACTIVE.store(true, Ordering::Relaxed);
        h.present("Test", Some(&mut app));
        assert_eq!(
            c.push2.lock().unwrap().frame.as_ref().unwrap().pixels.len(),
            960 * 160
        );
        send(&c, &[0xb0, 58, 127]);
        h.frame_due = Instant::now();
        h.present("Test", Some(&mut app));
        assert_eq!(h.bank, 2);
        send(&c, &[0xb0, 110, 127]);
        assert!(!c.push2.lock().unwrap().setup);
        h.frame_due = Instant::now();
        h.present("Test", Some(&mut app));
        assert_eq!(h.bank, 2);
        ACTIVE.store(false, Ordering::Relaxed);
    }
    #[test]
    fn display_encoding_has_exact_header_bgr_order_stride_padding_and_xor() {
        assert_eq!(&HEADER[..4], &[255, 204, 170, 136]);
        let mut p = vec![0; 960 * 160];
        p[0] = 0xff0000;
        p[1] = 0x00ff00;
        p[960] = 0x0000ff;
        let bytes = encode_frame(&p);
        assert_eq!(bytes.len(), 327680);
        assert_eq!(&bytes[..4], &[0x1f ^ 0xe7, 0xf3, 0xe0 ^ 0xe7, 0x07 ^ 0xff]);
        assert_eq!(&bytes[2048..2050], &[0xe7, 0xf8 ^ 0xf3]);
        assert_eq!(&bytes[1920..1924], &[0xe7, 0xf3, 0xe7, 0xff]);
    }
    #[test]
    fn malformed_messages_and_other_device_names_are_ignored() {
        let (c, _) = make();
        for msg in [
            &[0x90, 36][..],
            &[0xb0, 71, 255],
            &[0xf0, 1, 2],
            &[0x90, 128, 50],
        ] {
            send(&c, msg);
        }
        let s = c.push2.lock().unwrap();
        assert!(s.velocity.iter().all(|v| *v == 0));
        assert_eq!(s.encoders, [0; 8]);
        assert!(!is_push("Hydrasynth"));
        assert!(is_push("Ableton Push 2 User Port"));
        assert!(user_port("Ableton Push 2 20:1"));
    }
}

#[cfg(test)]
mod real_device_tests {
    use super::*;
    #[test]
    #[ignore = "opens the owner's real Push 2; opt in explicitly"]
    fn connected_push_accepts_native_display_frames() {
        let controller = Arc::new(ControllerState::new());
        let _device = Device::start(controller.clone(), Arc::new(AtomicF32::new(0.)));
        let mut host = Host::new(controller.clone());
        for _ in 0..90 {
            host.home();
            std::thread::sleep(Duration::from_millis(50));
        }
        let (midi, display, frames) = {
            let s = controller.push2.lock().unwrap();
            (s.connected, s.display_connected, s.display_frames)
        };
        eprintln!("Push 2 hardware: midi={midi}, usb={display}, complete_frames={frames}");
        // Drop the guard before assertions: a failed optional probe must still
        // let the worker clean up normally.
        assert!(display, "Push display interface could not be claimed");
        assert!(frames > 5, "No complete USB frames delivered");
    }
}

#[cfg(test)]
mod app_integration_tests {
    use super::*;
    pub(super) fn context() -> crate::app::AppContext {
        let mut c = crate::app::AppContext::new();
        c.provide(Arc::new(crate::audio_bus::AudioBus::new()))
            .provide(Arc::new(crate::modbus::ModBus::new()))
            .provide(Arc::new(crate::mixer_bus::MixerBus::new()))
            .provide(Arc::new(crate::note_bus::NoteBus::new()));
        c.provide_named("sensitivity", Arc::new(AtomicF32::new(0.1)))
            .provide_named("nav_speed", Arc::new(AtomicF32::new(3.)));
        c
    }
    pub(super) fn performance_apps() -> Vec<(String, Box<dyn App>)> {
        let registry = crate::registry::Registry::with_factories(
            context(),
            &[
                ("bebot", crate::apps::bebot::create),
                ("beads", crate::apps::beads::create),
            ],
        );
        registry.build(&[
            crate::manifest::AppManifest {
                id: "bebot".into(),
                name: "Bebot".into(),
                notes_in: true,
                ..Default::default()
            },
            crate::manifest::AppManifest {
                id: "beads".into(),
                name: "Beads".into(),
                ..Default::default()
            },
        ])
    }
    #[test]
    fn push_keeps_real_bebot_keyboard_audio_and_releases_while_editing_beads() {
        let mut apps = performance_apps();
        apps[0].1.on_enter();
        for i in [13, 14] {
            apps[0].1.adjust_setting(i, -100);
        }
        let before_bebot = apps[0].1.instrument_settings();
        let mut dsp = apps[0].1.audio_processor().unwrap();
        let c = Arc::new(ControllerState::new());
        c.push2.lock().unwrap().connected = true;
        let mut h = Host::new(c.clone());
        let mut routing = super::super::performance::Routing::default();
        h.update_targets(&mut routing, Some(0), &apps);
        c.push2.lock().unwrap().layout = Layout::Chromatic;
        c.push2
            .lock()
            .unwrap()
            .message(&[0x90, 60, 100], &c, &AtomicF32::new(1.));
        let mut input = Input::default();
        h.performance(&mut input);
        let input = routing.route(Some(0), &mut apps, input);
        apps[0].1.tick(&input);
        c.push2
            .lock()
            .unwrap()
            .message(&[0xb0, 21, 127], &c, &AtomicF32::new(1.));
        h.update_targets(&mut routing, Some(0), &apps);
        assert_eq!(routing.pinned(), Some(0));
        apps[0].1.on_exit();
        apps[1].1.on_enter();
        h.update_targets(&mut routing, Some(1), &apps);
        h.identify("Beads");
        let controls = apps[1].1.controller_settings().unwrap();
        let mix = controls
            .iter()
            .position(|r| r.label.ends_with(" / Dry/Wet"))
            .unwrap();
        h.bank = mix / 8;
        c.push2.lock().unwrap().encoders[mix % 8] = -10;
        let mut held_energy = 0.;
        for _ in 0..60 {
            let mut input = Input::default();
            h.performance(&mut input);
            let mut screen = routing.route(Some(1), &mut apps, input);
            assert!(!screen.midi_keys.0.iter().any(|v| *v > 0));
            for (_, app) in &mut apps {
                app.background_tick();
            }
            h.prepare("Beads", apps[1].1.as_mut(), &mut screen);
            apps[1].1.tick(&screen);
            let mut audio = [0.; 1024];
            dsp.process(&mut audio, 2, 48000.);
            held_energy = audio.iter().map(|v| v * v).sum::<f32>();
        }
        assert!(
            held_energy > 0.01,
            "Bebot must keep sounding while Beads is edited"
        );
        assert_ne!(
            apps[1].1.controller_settings().unwrap()[mix].value,
            controls[mix].value
        );
        assert_eq!(apps[0].1.instrument_settings(), before_bebot);
        assert_eq!(h.performance_name, "Bebot");
        assert!(h.performance_pinned);
        c.push2
            .lock()
            .unwrap()
            .message(&[0x80, 60, 0], &c, &AtomicF32::new(1.));
        let mut input = Input::default();
        h.performance(&mut input);
        routing.route(Some(1), &mut apps, input);
        let mut last_energy = 0.;
        for _ in 0..180 {
            let mut audio = [0.; 1024];
            dsp.process(&mut audio, 2, 48000.);
            last_energy = audio.iter().map(|v| v * v).sum::<f32>();
        }
        assert!(
            last_energy < held_energy * 0.01,
            "the release must reach hidden Bebot"
        );
    }
    #[test]
    fn push_target_picker_pins_another_app_and_cancels_without_editing_controls() {
        let mut apps = performance_apps();
        for (_, app) in &mut apps {
            app.on_enter();
        }
        let before = apps[1].1.controller_settings().unwrap();
        let c = Arc::new(ControllerState::new());
        c.push2.lock().unwrap().connected = true;
        let mut h = Host::new(c.clone());
        let mut routing = super::super::performance::Routing::default();
        let send = |m: &[u8]| c.push2.lock().unwrap().message(m, &c, &AtomicF32::new(1.));
        send(&[0xb0, 49, 127]);
        send(&[0xb0, 21, 127]);
        send(&[0xb0, 49, 0]);
        h.update_targets(&mut routing, Some(1), &apps);
        send(&[0xb0, 71, 1]);
        let mut input = Input::default();
        h.prepare("Beads", apps[1].1.as_mut(), &mut input);
        assert_eq!(apps[1].1.controller_settings().unwrap(), before);
        h.update_targets(&mut routing, Some(1), &apps);
        send(&[0xb0, 104, 127]);
        h.update_targets(&mut routing, Some(1), &apps);
        assert_eq!(routing.pinned(), Some(0));
        assert!(!c.take_top(1));
        assert!(h.keyboard_target_menu().is_none());
        send(&[0xb0, 49, 127]);
        send(&[0xb0, 50, 127]);
        send(&[0xb0, 49, 0]);
        h.update_targets(&mut routing, Some(1), &apps);
        send(&[0xb0, 71, 1]);
        send(&[0xb0, 110, 127]);
        h.update_targets(&mut routing, Some(1), &apps);
        assert_eq!(routing.pinned(), Some(0));
        send(&[0xb0, 21, 127]);
        h.update_targets(&mut routing, Some(1), &apps);
        assert_eq!(routing.pinned(), None);
        assert_eq!(routing.target(Some(1)), None);
        routing.select(0, &apps);
        assert_eq!(routing.target(Some(1)), Some(1));
    }
    #[test]
    fn real_lazy_instrument_receives_encoder_edits_and_full_keyboard_audio() {
        let registry = crate::registry::Registry::with_factories(
            context(),
            &[("june60", crate::apps::june60::create)],
        );
        let manifests = [crate::manifest::AppManifest {
            id: "june60".into(),
            name: "June-60".into(),
            ..Default::default()
        }];
        let mut apps = registry.build(&manifests);
        let app = apps[0].1.as_mut();
        app.on_enter();
        let c = Arc::new(ControllerState::new());
        c.push2.lock().unwrap().connected = true;
        let mut h = Host::new(c.clone());
        let before = app.controller_settings().unwrap()[1].value.clone();
        c.push2.lock().unwrap().encoders[1] = 20;
        let mut input = Input::default();
        h.prepare("June-60", app, &mut input);
        assert_ne!(
            app.controller_settings().unwrap()[1].value,
            before,
            "real cutoff changed through LazyApp"
        );
        c.push2.lock().unwrap().layout = Layout::Chromatic;
        c.push2
            .lock()
            .unwrap()
            .message(&[0x90, 60, 100], &c, &AtomicF32::new(1.));
        h.performance(&mut input);
        app.tick(&input);
        let mut dsp = app.audio_processor().unwrap();
        let mut peak = 0f32;
        for _ in 0..30 {
            let mut b = [0.; 1024];
            dsp.process(&mut b, 2, 48000.);
            assert!(b.iter().all(|v| v.is_finite()));
            peak = b.iter().fold(peak, |p, v| p.max(v.abs()));
        }
        assert!(
            peak > 0.001,
            "Push keyboard must produce real instrument audio"
        );
    }
    #[test]
    fn collapsed_pams_channels_are_all_browsable_and_edit_the_labelled_channel() {
        let mut app = crate::apps::pams::create(&context(), "pams");
        app.on_enter();
        assert!(app.play_column().is_some());
        let rows = app.controller_settings().unwrap();
        assert!(
            rows.len() > 100,
            "all eight channels must be exposed while collapsed"
        );
        let index = rows
            .iter()
            .position(|r| r.label.contains("Ch8") && r.label.ends_with("Width"))
            .or_else(|| rows.iter().rposition(|r| r.label.ends_with("Width")))
            .unwrap();
        let c = Arc::new(ControllerState::new());
        c.push2.lock().unwrap().connected = true;
        let mut h = Host::new(c.clone());
        h.identify("Pam's");
        h.bank = index / 8;
        let before = rows[index].value.clone();
        let first = rows
            .iter()
            .position(|r| r.label.ends_with("Width"))
            .unwrap();
        let other = rows[first].value.clone();
        c.push2.lock().unwrap().encoders[index % 8] = 30;
        h.prepare("Pam's", app.as_mut(), &mut Input::default());
        let after = app.controller_settings().unwrap();
        assert_ne!(
            after[index].value, before,
            "last channel's labelled width changes"
        );
        assert_eq!(after[first].value, other, "first channel is untouched");
        assert!(app.play_column().is_some());
    }
    #[test]
    fn controller_labels_match_the_selected_native_menu_in_grouped_apps() {
        use crate::apps;
        type Factory = fn(&crate::app::AppContext, &str) -> Box<dyn App>;
        let factories: [(&str, Factory); 9] = [
            ("pams", apps::pams::create),
            ("plaits", apps::plaits::create),
            ("beads", apps::beads::create),
            ("tinkertone", apps::tinkertone::create),
            ("voltage", apps::voltage::create),
            ("rainmaker", apps::rainmaker::create),
            ("starlab", apps::starlab::create),
            ("sample_drum", apps::sample_drum::create),
            ("clouds", apps::clouds::create),
        ];
        for (name, factory) in factories {
            let mut app = factory(&context(), name);
            let controls = app.controller_settings().unwrap();
            assert!(!controls.is_empty(), "{name}");
            for (index, control) in controls.iter().enumerate() {
                app.controller_select(index);
                let rows = app.slint_rows();
                let row = &rows[app.slint_selected()];
                assert!(!row.2, "{name}: {} selected a header", control.label);
                assert!(
                    control.label.ends_with(&format!(" / {}", row.0)),
                    "{name}: {} must name {}",
                    control.label,
                    row.0
                );
                assert_eq!(row.1, control.value, "{name}: {} value", control.label);
            }
        }
    }
    #[test]
    fn rev2_both_layers_and_distant_sequence_steps_are_editable_independently() {
        let mut app = crate::apps::prophet::create(&context(), "prophet");
        let rows = app.controller_settings().unwrap();
        assert!(rows.len() > 1800);
        let a = rows
            .iter()
            .position(|r| r.label.starts_with("A ") && r.label.ends_with(" / Filter Cutoff"))
            .unwrap();
        let b = rows
            .iter()
            .position(|r| r.label.starts_with("B ") && r.label.ends_with(" / Filter Cutoff"))
            .unwrap();
        app.controller_adjust(b, 7);
        let after = app.controller_settings().unwrap();
        assert_ne!(after[b].value, rows[b].value);
        assert_eq!(after[a].value, rows[a].value);
        for prefix in ["B Poly notes 5-6", "B Poly velocities"] {
            let step = if prefix.ends_with("velocities") {
                "Track 6 step 62"
            } else {
                "Track 6 step 64"
            };
            let index = rows
                .iter()
                .position(|r| r.label.starts_with(prefix) && r.label.ends_with(step))
                .unwrap();
            app.controller_adjust(index, 140);
            assert_ne!(
                app.controller_settings().unwrap()[index].value,
                rows[index].value
            );
        }
    }
    #[test]
    fn plaits_catalog_includes_closed_envelopes_and_distinct_modulator_slots() {
        let mut app = crate::apps::plaits::create(&context(), "plaits");
        let rows = app.controller_settings().unwrap();
        let release = rows
            .iter()
            .position(|r| r.label == "Envelope / Release")
            .unwrap();
        assert!(rows.iter().any(|r| r.label.starts_with("Piano Roll /")));
        assert!(rows.iter().any(|r| r.label.starts_with("Arp /")));
        let slots: std::collections::HashSet<_> = rows
            .iter()
            .filter_map(|r| {
                r.label
                    .strip_prefix("Modulator ")
                    .and_then(|s| s.split_once(" / "))
                    .map(|(slot, _)| slot)
            })
            .collect();
        assert!(slots.len() >= 4);
        app.controller_adjust(release, 5);
        assert_ne!(
            app.controller_settings().unwrap()[release].value,
            rows[release].value
        );
    }
    #[test]
    fn chop_sample_and_fx_controls_are_available_from_play_view() {
        let mut app = crate::apps::chop::create(&context(), "chop");
        let rows = app.controller_settings().unwrap();
        for name in ["Sample / Hit sensitivity", "Fx / Tempo"] {
            let index = rows.iter().position(|r| r.label == name).unwrap();
            app.controller_adjust(index, 1);
            assert_ne!(
                app.controller_settings().unwrap()[index].value,
                rows[index].value
            );
        }
        let pad = rows.iter().position(|r| r.label == "Pad").unwrap();
        app.controller_adjust(pad, 1);
        assert_eq!(app.controller_settings().unwrap()[pad].value, "2");
    }
    #[test]
    fn skins_voice_selection_edits_the_named_voice_without_altering_another() {
        let mut app = crate::apps::skins::create(&context(), "skins");
        let kick = app
            .controller_settings()
            .unwrap()
            .into_iter()
            .find(|r| r.label == "Kick / Level")
            .unwrap()
            .value;
        app.controller_adjust(1, 1);
        let rows = app.controller_settings().unwrap();
        let index = rows
            .iter()
            .position(|r| r.label == "Snare / Level")
            .unwrap();
        app.controller_adjust(index, -5);
        assert_ne!(
            app.controller_settings().unwrap()[index].value,
            rows[index].value
        );
        app.controller_adjust(1, -1);
        assert_eq!(
            app.controller_settings().unwrap()[index].label,
            "Kick / Level"
        );
        assert_eq!(app.controller_settings().unwrap()[index].value, kick);
    }
    #[test]
    fn grid_bank_changes_send_releases_to_the_original_grid_coordinates() {
        crate::apps::grid_kit::register_with_push2();
        let grid = crate::apps::grid_kit::grid();
        grid.set_size(8, 16);
        grid.register("Push test");
        grid.set_focus("Push test");
        let c = ControllerState::new();
        let mut s = Surface::default();
        s.layout = Layout::Grid;
        s.message(&[0x90, 36, 100], &c, &AtomicF32::new(1.));
        let keys = grid.keys("Push test");
        assert_eq!((keys[0].x, keys[0].y, keys[0].down), (0, 7, true));
        s.setup = true;
        s.message(&[0xb0, 74, 1], &c, &AtomicF32::new(1.));
        let keys = grid.keys("Push test");
        assert_eq!((keys[0].x, keys[0].y, keys[0].down), (0, 7, false));
        s.message(&[0x90, 36, 100], &c, &AtomicF32::new(1.));
        let keys = grid.keys("Push test");
        assert_eq!((keys[0].x, keys[0].y), (8, 7));
        s.release();
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    fn save_frame(directory: &str, file: &str, c: &ControllerState) {
        let frame = c.push2.lock().unwrap().frame.take().unwrap();
        let bytes: Vec<u8> = frame.pixels.iter()
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
            .collect();
        let output = std::fs::File::create(std::path::Path::new(directory).join(file)).unwrap();
        let mut encoder = png::Encoder::new(output, 960, 160);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&bytes).unwrap();
    }
    #[test]
    #[ignore = "writes native Push display previews, no hardware"]
    fn export_push_display_previews() {
        let directory =
            std::env::var("PORTAMAX_PUSH2_PREVIEW_DIR").expect("set PORTAMAX_PUSH2_PREVIEW_DIR");
        std::fs::create_dir_all(&directory).unwrap();
        ACTIVE.store(true, Ordering::Relaxed);
        for (file, setup, long_labels) in [
            ("push2-parameters.png", false, false),
            ("push2-keyboard.png", true, false),
            ("push2-long-labels.png", false, true),
        ] {
            let (mut app, title) = if long_labels {
                (
                    crate::apps::prophet::create(
                        &super::app_integration_tests::context(),
                        "prophet",
                    ),
                    "Rev2",
                )
            } else {
                (
                    crate::apps::june60::create(&super::app_integration_tests::context(), "june60"),
                    "June-60",
                )
            };
            app.on_enter();
            app.tick(&Input::default());
            let c = Arc::new(ControllerState::new());
            {
                let mut s = c.push2.lock().unwrap();
                s.layout = Layout::Fourths;
                s.setup = setup;
            }
            let mut h = Host::new(c.clone());
            h.performance_name = title.into();
            if long_labels {
                h.bank = app
                    .controller_settings()
                    .unwrap()
                    .iter()
                    .position(|r| r.label == "Performance / Follow device clock")
                    .unwrap()
                    / 8;
            }
            h.present(title, Some(app.as_mut()));
            save_frame(&directory, file, &c);
        }
        let registry = crate::registry::Registry::new(super::app_integration_tests::context());
        let mut apps = registry.build(&crate::manifest::discover(std::path::Path::new("apps")));
        let bebot = apps.iter().position(|(n, _)| n == "Bebot").unwrap();
        let beads = apps.iter().position(|(n, _)| n == "Beads").unwrap();
        apps[beads].1.on_enter();
        apps[beads].1.tick(&Input::default());
        let mut routing = super::super::performance::Routing::default();
        routing.set_target(Some(bebot), &apps);
        let c = Arc::new(ControllerState::new());
        let mut h = Host::new(c.clone());
        h.update_targets(&mut routing, Some(beads), &apps);
        h.present("Beads", Some(apps[beads].1.as_mut()));
        save_frame(&directory, "push2-midi-arm.png", &c);
        h.choose_keyboard_target();
        h.update_targets(&mut routing, Some(beads), &apps);
        assert!(h.key_names.len() > 20);
        h.frame_due = Instant::now();
        h.present("Beads", Some(apps[beads].1.as_mut()));
        save_frame(&directory, "push2-midi-arm-picker.png", &c);
        ACTIVE.store(false, Ordering::Relaxed);
    }
}
