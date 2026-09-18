//! Lists available audio devices, plus two global controls shared by
//! every app that uses them: knob sensitivity for continuous value edits,
//! and list nav speed for discrete list navigation (see paramlist.rs for
//! why those are deliberately different kinds of control, not the same
//! thing tuned differently). Added so tuning either is a menu item, not
//! a code change. Selecting an output actually switches the live stream
//! (see audio_devices.rs). Selecting an input just records the
//! preference -- there's no microphone pipeline in this build to plug
//! it into yet.
//!
//! Uses the same Group/Leaf/`ParamList` navigation every other app in
//! this build uses (knob1: browse + press to expand/collapse a group,
//! knob2: edit + press to reset/select) -- this used to be the one app
//! with its own arrow-key-driven navigation, which meant "how do I move
//! around a menu" had two different answers depending which app you
//! were in. Each device is its own action-only leaf (press to select
//! it), the same "press to act" idiom Bloom's Randomize/Clouds'
//! Trigger already use, rather than a special selection mechanism.

use crate::app::{App, Input};
use crate::audio_devices::{self, AudioDeviceState};
use crate::display::FrameBuffer;
use crate::paramlist::ParamList;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use std::sync::Arc;

const DEFAULT_SENSITIVITY: f32 = 0.1;
const DEFAULT_NAV_SPEED: f32 = 6.0;
const MIN_SENSITIVITY: f32 = 0.02;
const MAX_SENSITIVITY: f32 = 1.0;
const MIN_NAV_SPEED: f32 = 1.0;
const MAX_NAV_SPEED: f32 = 10.0;

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    OutputDevice(usize),
    InputDevice(usize),
    Sensitivity,
    NavSpeed,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 3;

pub struct SettingsApp {
    devices: Arc<AudioDeviceState>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    outputs: Vec<String>,
    inputs: Vec<String>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

impl SettingsApp {
    pub fn new(devices: Arc<AudioDeviceState>, sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>) -> Self {
        Self {
            devices,
            sensitivity,
            nav_speed,
            outputs: Vec::new(),
            inputs: Vec::new(),
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
        }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => (0..self.outputs.len()).map(Selection::OutputDevice).collect(),
            1 => (0..self.inputs.len()).map(Selection::InputDevice).collect(),
            _ => vec![Selection::Sensitivity, Selection::NavSpeed],
        }
    }

    fn visible_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for g in 0..NUM_GROUPS {
            rows.push(Row::Group(g));
            if self.expanded[g] {
                for sel in self.group_leaves(g) {
                    rows.push(Row::Leaf(sel));
                }
            }
        }
        rows
    }

    fn group_name(&self, g: usize) -> &'static str {
        match g {
            0 => "Output Device",
            1 => "Input Device",
            _ => "Preferences",
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => self.devices.current_output(),
            1 => {
                let cur = self.devices.current_input();
                if cur.is_empty() { "none (not wired to any app yet)".into() } else { format!("{cur} (not wired to any app yet)") }
            }
            _ => format!("sens {:.2}, nav {:.0}", self.sensitivity.get(), self.nav_speed.get()),
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::OutputDevice(i) => self.outputs.get(i).cloned().unwrap_or_default(),
            Selection::InputDevice(i) => self.inputs.get(i).cloned().unwrap_or_default(),
            Selection::Sensitivity => "Knob Sensitivity".into(),
            Selection::NavSpeed => "List Nav Speed".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::OutputDevice(i) => {
                let name = self.outputs.get(i).cloned().unwrap_or_default();
                if name == self.devices.current_output() { "active".into() } else { "press to select".into() }
            }
            Selection::InputDevice(i) => {
                let name = self.inputs.get(i).cloned().unwrap_or_default();
                if name == self.devices.current_input() { "active".into() } else { "press to select".into() }
            }
            Selection::Sensitivity => format!("{:.2}", self.sensitivity.get()),
            Selection::NavSpeed => format!("{:.0}", self.nav_speed.get()),
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        match sel {
            Selection::OutputDevice(_) | Selection::InputDevice(_) => {} // press-only actions, see `reset`
            Selection::Sensitivity => {
                let next = (self.sensitivity.get() + accelerate(delta) * 0.01).clamp(MIN_SENSITIVITY, MAX_SENSITIVITY);
                self.sensitivity.set(next);
            }
            Selection::NavSpeed => {
                let next = (self.nav_speed.get() + accelerate(delta)).clamp(MIN_NAV_SPEED, MAX_NAV_SPEED);
                self.nav_speed.set(next);
            }
        }
    }

    fn reset(&mut self, sel: Selection) {
        match sel {
            Selection::OutputDevice(i) => {
                if let Some(name) = self.outputs.get(i).cloned() {
                    self.devices.request_output(name);
                }
            }
            Selection::InputDevice(i) => {
                if let Some(name) = self.inputs.get(i).cloned() {
                    self.devices.set_input(name);
                }
            }
            Selection::Sensitivity => self.sensitivity.set(DEFAULT_SENSITIVITY),
            Selection::NavSpeed => self.nav_speed.set(DEFAULT_NAV_SPEED),
        }
    }
}

impl App for SettingsApp {
    fn on_enter(&mut self) {
        // Re-scan every time -- cheap, and devices can change between visits.
        self.outputs = audio_devices::output_device_names();
        self.inputs = audio_devices::input_device_names();
    }

    fn tick(&mut self, input: &Input) {
        let rows = self.visible_rows();
        self.list.navigate(input.knob1, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();

        if input.knob1_press {
            if let Some(Row::Group(g)) = current {
                self.expanded[g] = !self.expanded[g];
            }
        }
        if let Some(Row::Leaf(sel)) = current {
            self.edit(sel, input.knob2);
            if input.knob2_press {
                self.reset(sel);
            }
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let title = MonoTextStyle::new(&SPLEEN_16X32, Rgb565::WHITE);
        Text::new("Settings", Point::new(20, 30), title).draw(fb).ok();

        let dim = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));

        let rows = self.visible_rows();
        let display_rows: Vec<(String, String)> = rows
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g))
                }
                Row::Leaf(sel) => (format!("    {}", self.leaf_name(*sel)), self.leaf_value(*sel)),
            })
            .collect();
        self.list.draw(fb, 20, 55, 19, 12, &display_rows);

        let hint = match rows.get(self.list.selected) {
            Some(Row::Group(_)) => "knob1: browse   press knob1: expand/collapse".to_string(),
            Some(Row::Leaf(sel)) => format!("knob2: change {}   press knob2: select/reset", self.leaf_name(*sel)),
            None => String::new(),
        };
        Text::new(&hint, Point::new(20, 337), dim).draw(fb).ok();
    }
}
