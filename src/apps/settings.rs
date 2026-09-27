//! Lists available audio devices, plus two global controls shared by
//! every app that uses them: knob sensitivity for continuous value edits,
//! and list nav speed for discrete list navigation (see paramlist.rs for
//! why those are deliberately different kinds of control, not the same
//! thing tuned differently). Added so tuning either is a menu item, not
//! a code change. Selecting an output actually switches the live stream
//! (see audio_devices.rs). Selecting an input just records the
//! capture request; apps opt in by selecting Hardware input.
//!
//! Uses the same Group/Leaf/`ParamList` navigation every other app in
//! this build uses (knob1: browse + press to expand/collapse a group,
//! knob2: edit + press to reset/select) -- this used to be the one app
//! with its own arrow-key-driven navigation, which meant "how do I move
//! around a menu" had two different answers depending which app you
//! were in. Each device is its own action-only leaf (press to select
//! it), the same "press to act" idiom Bloom's Randomize/Clouds'
//! Trigger already use, rather than a special selection mechanism.

use crate::app::{App, Input, SlintExtra, ThemeExtra};
use crate::audio_devices::{self, AudioDeviceState};
use crate::display::FrameBuffer;
use crate::paramlist::ParamList;
use crate::theme::ThemeColor;
use crate::util::{accelerate, AtomicF32};
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const DEFAULT_SENSITIVITY: f32 = 0.1;
const DEFAULT_NAV_SPEED: f32 = 3.0;
const MIN_SENSITIVITY: f32 = 0.02;
const MAX_SENSITIVITY: f32 = 1.0;
const MIN_NAV_SPEED: f32 = 1.0;
const MAX_NAV_SPEED: f32 = 10.0;
/// The wheel's real on-screen radius (px) in the live Slint panel --
/// shared with `slint_extra`'s marker math so a click on the wheel
/// and the dot it leaves behind always agree (see
/// `ThemeColor::set_from_wheel`/`wheel_marker`).
pub const WHEEL_RADIUS_PX: f32 = 70.0;

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    OutputDevice(usize),
    InputDevice(usize),
    Sensitivity,
    NavSpeed,
    ShowCpu,
    ColorTarget,
    Hue,
    Saturation,
    Brightness,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 4;

pub struct SettingsApp {
    devices: Arc<AudioDeviceState>,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
    /// Whether the live engine-load readout should be shown in the
    /// UI's chrome -- read directly by whatever screen owns that
    /// chrome (see `examples/slint_home_live.rs`), this app just
    /// flips it.
    show_cpu: Arc<AtomicBool>,
    accent: Arc<ThemeColor>,
    background: Arc<ThemeColor>,
    /// Which of `accent`/`background` the Hue/Saturation/Brightness
    /// rows (and the wheel) currently edit -- false = accent.
    editing_background: bool,
    outputs: Vec<String>,
    inputs: Vec<String>,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

impl SettingsApp {
    pub fn new(
        devices: Arc<AudioDeviceState>,
        sensitivity: Arc<AtomicF32>,
        nav_speed: Arc<AtomicF32>,
        show_cpu: Arc<AtomicBool>,
        accent: Arc<ThemeColor>,
        background: Arc<ThemeColor>,
    ) -> Self {
        Self {
            devices,
            sensitivity,
            nav_speed,
            show_cpu,
            accent,
            background,
            editing_background: false,
            outputs: Vec::new(),
            inputs: Vec::new(),
            list: ParamList::new(),
            expanded: [false; NUM_GROUPS],
        }
    }

    fn target(&self) -> &ThemeColor {
        if self.editing_background { &self.background } else { &self.accent }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => (0..self.outputs.len()).map(Selection::OutputDevice).collect(),
            1 => (0..self.inputs.len()).map(Selection::InputDevice).collect(),
            2 => vec![Selection::Sensitivity, Selection::NavSpeed, Selection::ShowCpu],
            _ => vec![Selection::ColorTarget, Selection::Hue, Selection::Saturation, Selection::Brightness],
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
            2 => "Controls",
            _ => "Appearance",
        }
    }

    fn group_summary(&self, g: usize) -> String {
        match g {
            0 => self.devices.current_output(),
            1 => {
                let cur = self.devices.current_input();
                if cur.is_empty() { "No input selected".into() } else { cur }
            }
            2 => format!(
                "sens {:.2}, nav {:.0}, cpu {}",
                self.sensitivity.get(),
                self.nav_speed.get(),
                if self.show_cpu.load(Ordering::Relaxed) { "on" } else { "off" }
            ),
            _ => format!("accent {}, background {}", self.accent.hex(), self.background.hex()),
        }
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::OutputDevice(i) => self.outputs.get(i).cloned().unwrap_or_default(),
            Selection::InputDevice(i) => self.inputs.get(i).cloned().unwrap_or_default(),
            Selection::Sensitivity => "Knob Sensitivity".into(),
            Selection::NavSpeed => "List Nav Speed".into(),
            Selection::ShowCpu => "Show CPU Usage".into(),
            Selection::ColorTarget => "Editing".into(),
            Selection::Hue => "Hue".into(),
            Selection::Saturation => "Saturation".into(),
            Selection::Brightness => "Brightness".into(),
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
            Selection::ShowCpu => if self.show_cpu.load(Ordering::Relaxed) { "on".into() } else { "off".into() },
            Selection::ColorTarget => if self.editing_background { "Background".into() } else { "Accent".into() },
            Selection::Hue => format!("{}\u{b0} ({})", self.target().hsv().0, self.target().hex()),
            Selection::Saturation => format!("{}%", self.target().hsv().1),
            Selection::Brightness => format!("{}%", self.target().hsv().2),
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
            Selection::ShowCpu => self.show_cpu.store(delta > 0, Ordering::Relaxed),
            Selection::ColorTarget => self.editing_background = delta > 0,
            Selection::Hue => {
                let cur = self.target().hsv().0 as i32;
                self.target().set_hue((cur + delta.signum()).rem_euclid(360) as u32);
            }
            Selection::Saturation => {
                let cur = self.target().hsv().1 as i32;
                self.target().set_sat((cur + delta.signum()).clamp(0, 100) as u32);
            }
            Selection::Brightness => {
                let cur = self.target().hsv().2 as i32;
                self.target().set_val((cur + delta.signum()).clamp(0, 100) as u32);
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
            Selection::ShowCpu => self.show_cpu.store(false, Ordering::Relaxed),
            Selection::ColorTarget => self.editing_background = false,
            Selection::Hue | Selection::Saturation | Selection::Brightness => {
                let (h, s, v) = if self.editing_background {
                    (crate::theme::BG_DEFAULT_HUE, crate::theme::BG_DEFAULT_SAT, crate::theme::BG_DEFAULT_VAL)
                } else {
                    (crate::theme::ACCENT_DEFAULT_HUE, crate::theme::ACCENT_DEFAULT_SAT, crate::theme::ACCENT_DEFAULT_VAL)
                };
                let target = self.target();
                target.set_hue(h);
                target.set_sat(s);
                target.set_val(v);
            }
        }
    }
}

impl SettingsApp {
    /// Real, windowed `(name, value, is_group)` rows -- mirrors this
    /// app's own `draw()` row-building, exposed for an alternate
    /// renderer (a live Slint screen) instead of drawn.
    pub(crate) fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.visible_rows()
            .iter()
            .map(|row| match row {
                Row::Group(g) => {
                    let arrow = if self.expanded[*g] { "v" } else { ">" };
                    (format!("{arrow} {}", self.group_name(*g)), self.group_summary(*g), true)
                }
                Row::Leaf(sel) => (self.leaf_name(*sel), self.leaf_value(*sel), false),
            })
            .collect()
    }

    pub(crate) fn selected_row(&self) -> usize {
        self.list.selected
    }

    /// `display_rows`, windowed to at most `visible` rows around the
    /// current selection -- see `ParamList::centered_scroll_window`. Returns
    /// `(window, selected_index_in_window, has_more_above,
    /// has_more_below)`.
    pub(crate) fn windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        let window = rows[start..end].to_vec();
        (window, self.list.selected - start, start > 0, end < rows.len())
    }
}

impl SettingsApp {
    fn theme_extra(&self) -> ThemeExtra {
        let (h, s, v) = self.target().hsv();
        let (mx, my) = self.target().wheel_marker(WHEEL_RADIUS_PX);
        let row=self.visible_rows().get(self.list.selected).copied();
        let section=match row {Some(Row::Group(g))=>g,Some(Row::Leaf(Selection::OutputDevice(_)))=>0,Some(Row::Leaf(Selection::InputDevice(_)))=>1,Some(Row::Leaf(Selection::Sensitivity|Selection::NavSpeed|Selection::ShowCpu))=>2,_=>3};
        let (detail,value)=match row {Some(Row::Leaf(sel))=>(self.leaf_name(sel),self.leaf_value(sel)),_ => (self.group_name(section).into(), self.group_summary(section))};
        let help=match row {
            Some(Row::Leaf(Selection::Sensitivity))=>"Adjust how quickly MIDI knobs change a parameter. Left / right to adjust.",
            Some(Row::Leaf(Selection::NavSpeed))=>"Encoder ticks per menu step. D-pad taps always move one row. Left / right to adjust.",
            Some(Row::Leaf(Selection::ShowCpu))=>"Show the live audio-engine load in the system header. Left / right to change.",
            _=>match section {0=>"Expand Output Device, choose an output, then press R1 to connect.",1=>"Choose an input and press R1. Route Hardware input inside an app to use its signal.",2=>"Expand Controls to adjust controls and the CPU display.",_=>"Choose Accent or Background. Touch the wheel, or adjust Hue, Saturation and Brightness."},
        };
        ThemeExtra {
            section,detail,value,help:help.into(),output:self.devices.current_output(),input:self.devices.current_input(),device_count:if section==0 {self.outputs.len()}else{self.inputs.len()},
            editing_background: self.editing_background,
            hue: h as f32,
            saturation: s as f32,
            brightness: v as f32,
            marker_x: mx,
            marker_y: my,
            accent_rgb: self.accent.rgb(),
            background_rgb: self.background.rgb(),
        }
    }
}

impl App for SettingsApp {
    fn system_role(&self) -> Option<crate::app::SystemRole> { Some(crate::app::SystemRole::Settings) }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.selected_row()
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        self.windowed_rows(visible.min(7))
    }
    fn slint_extra(&mut self) -> SlintExtra {
        SlintExtra::Theme(self.theme_extra())
    }
    fn slint_pointer_pick(&mut self, x: f32, y: f32) {
        if (1000. ..1004.).contains(&x) {
            let group=(x-1000.) as usize;self.expanded=[false;NUM_GROUPS];self.expanded[group]=true;
            self.list.selected=self.visible_rows().iter().position(|r|matches!(r,Row::Group(g) if *g==group)).unwrap_or(0);
        } else if x==1004. || x==1005. {self.editing_background=x==1005.;} else {self.target().set_from_wheel(x, y, WHEEL_RADIUS_PX);}
    }

    fn on_enter(&mut self) {
        // Re-scan every time -- cheap, and devices can change between visits.
        self.outputs = audio_devices::output_device_names();
        self.inputs = audio_devices::input_device_names();
    }

    fn tick(&mut self, input: &Input) {
        let rows = self.visible_rows();
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
        let current = rows.get(self.list.selected).copied();

        if input.knob1_press {
            if let Some(Row::Group(g)) = current {
                self.expanded[g] = !self.expanded[g];
            }
        }
        if let Some(Row::Leaf(sel)) = current {
            self.edit(sel, input.knob2);
            if input.knob2_press || (input.knob1_press && matches!(sel,Selection::OutputDevice(_)|Selection::InputDevice(_))) {
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

#[cfg(test)]
mod redesign_tests {
    use super::*;
    fn fixture()->SettingsApp {SettingsApp::new(Arc::new(AudioDeviceState::new("Test output".into())),Arc::new(AtomicF32::new(0.1)),Arc::new(AtomicF32::new(3.)),Arc::new(AtomicBool::new(false)),Arc::new(ThemeColor::new(132,62,94)),Arc::new(ThemeColor::new(132,31,6)))}
    #[test] fn tabs_preserve_theme_and_preferences_remain_editable() {
        let mut app=fixture();let original=app.accent.hsv();
        for group in 0..4 {app.slint_pointer_pick(1000.+group as f32,0.);assert_eq!(app.theme_extra().section,group);assert_eq!(app.accent.hsv(),original);assert_eq!(app.expanded.iter().filter(|v|**v).count(),1);}
        app.slint_pointer_pick(1002.,0.);app.tick(&Input{navigation_steps:1,knob2:1,..Default::default()});assert!(app.sensitivity.get()>0.1);
        let (rows,selected,_,_)=app.slint_windowed_rows(10);assert!(rows.len()<=7&&selected<rows.len());
    }
    #[test] fn appearance_wheel_still_edits_selected_color() {
        let mut app=fixture();let background=app.background.hsv();app.slint_pointer_pick(70.,0.);assert_ne!(app.accent.hsv(),(132,62,94));assert_eq!(app.background.hsv(),background);
        app.editing_background=true;let accent=app.accent.hsv();app.slint_pointer_pick(0.,70.);assert_ne!(app.background.hsv(),background);assert_eq!(app.accent.hsv(),accent);
    }
}
