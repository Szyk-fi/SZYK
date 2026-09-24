//! The on-device UI for `midi_map.rs`'s real CC -> modulation-target
//! table: browse and remove existing mappings, or add a new one by
//! picking any currently-registered target (the same live
//! `modbus.names()` list every other routing screen in this build
//! reads -- Pam's channel routing, Queen of Pentacles' outputs,
//! Natural Gate's Env Out, CV Out, ...) and "Learn"-ing a CC from a
//! real physical controller rather than typing a number in.
//!
//! Not itself an audio app -- no `audio_processor`, nothing to mix or
//! tap. Purely a real-time-safe front end onto `MidiMap`'s own state,
//! same relationship Settings has to `AudioDeviceState`.

use crate::app::{App, Input};
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::midi_map::MidiMap;
use crate::modbus::ModBus;
use crate::paramlist::{ParamList, ACCENT};
use crate::util::AtomicF32;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq)]
enum Selection {
    Mapping(usize),
    Target,
    LearnCc,
}

#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Leaf(Selection),
}

const NUM_GROUPS: usize = 2;

pub struct MidiLearnApp {
    modbus: Arc<ModBus>,
    midi_map: Arc<MidiMap>,
    nav_speed: Arc<AtomicF32>,
    /// Which registered target the "Add Mapping" group currently has
    /// selected -- browsed with knob2, same idea as every other
    /// dynamic-target picker in this build.
    target_browse: usize,
    list: ParamList,
    expanded: [bool; NUM_GROUPS],
}

// --- MIDI Learn's own palette: flat solid colors, not a
// device-wide theme -- Minimal graphite and cool blue-white -- plain infrastructure, not an instrument, so deliberately understated. ---

const MIDI_LEARN_BG: Rgb565 = Rgb565::new(3, 6, 4);
const MIDI_LEARN_TITLE: Rgb565 = Rgb565::new(29, 60, 29);
const MIDI_LEARN_ACCENT: Rgb565 = Rgb565::new(22, 52, 31);
const MIDI_LEARN_DIM: Rgb565 = Rgb565::new(15, 35, 19);

impl MidiLearnApp {
    pub fn new(nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, midi_map: Arc<MidiMap>) -> Self {
        Self { modbus, midi_map, nav_speed, target_browse: 0, list: ParamList::new(), expanded: [false; NUM_GROUPS] }
    }

    fn group_leaves(&self, g: usize) -> Vec<Selection> {
        match g {
            0 => (0..self.midi_map.len()).map(Selection::Mapping).collect(),
            _ => vec![Selection::Target, Selection::LearnCc],
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
        if g == 0 { "Mappings" } else { "Add Mapping" }
    }

    fn group_summary(&self, g: usize) -> String {
        if g == 0 {
            format!("{} mapped", self.midi_map.len())
        } else if self.midi_map.is_armed() {
            "listening...".into()
        } else {
            "press Learn CC to add".into()
        }
    }

    fn target_name(&self) -> String {
        self.modbus.names().get(self.target_browse).cloned().unwrap_or_else(|| "none found".into())
    }

    fn leaf_name(&self, sel: Selection) -> String {
        match sel {
            Selection::Mapping(_) => "Mapping".into(),
            Selection::Target => "Target".into(),
            Selection::LearnCc => "Learn CC".into(),
        }
    }

    fn leaf_value(&self, sel: Selection) -> String {
        match sel {
            Selection::Mapping(i) => match self.midi_map.mappings().get(i) {
                Some((cc, name)) => format!("CC{cc} -> {name}"),
                None => String::new(),
            },
            Selection::Target => self.target_name(),
            Selection::LearnCc => {
                if self.midi_map.is_armed() {
                    "listening... (press to cancel)".into()
                } else {
                    "press to learn".into()
                }
            }
        }
    }

    fn edit(&mut self, sel: Selection, delta: i32) {
        if delta == 0 {
            return;
        }
        if let Selection::Target = sel {
            let n = self.modbus.len().max(1);
            let cur = self.target_browse as i32;
            self.target_browse = (cur + delta.signum()).rem_euclid(n as i32) as usize;
        }
    }

    fn press(&mut self, sel: Selection) {
        match sel {
            Selection::Mapping(i) => self.midi_map.remove_at(i),
            Selection::Target => {}
            Selection::LearnCc => {
                if self.midi_map.is_armed() {
                    self.midi_map.cancel_learn();
                } else {
                    self.midi_map.arm_learn();
                }
            }
        }
    }
}

impl MidiLearnApp {
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

impl App for MidiLearnApp {
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
            if input.knob2_press {
                self.press(sel);
            }
        }

        // A Learn just captured a real CC -- complete the mapping
        // against whatever target is currently browsed. Polled here
        // (not pushed from the MIDI thread) so it's applied on the
        // UI thread, same as every other cross-thread real-time
        // state in this build.
        if let Some(cc) = self.midi_map.take_learned() {
            if let Some(name) = self.modbus.names().get(self.target_browse).cloned() {
                self.midi_map.set(cc, name);
            }
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32))
            .into_styled(PrimitiveStyle::with_fill(MIDI_LEARN_BG))
            .draw(fb)
            .ok();

        let title = MonoTextStyle::new(&SPLEEN_16X32, MIDI_LEARN_TITLE);
        Text::new("MIDI Learn", Point::new(16, 30), title).draw(fb).ok();
        let dim = MonoTextStyle::new(&SPLEEN_6X12, MIDI_LEARN_DIM);
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
        self.list.draw_themed(fb, 16, 56, 22, 10, &display_rows, MIDI_LEARN_BG, MIDI_LEARN_DIM, MIDI_LEARN_ACCENT);
        let _ = dim;
        let _ = ACCENT;
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.selected_row()
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        self.windowed_rows(visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app() -> (MidiLearnApp, Arc<ModBus>, Arc<MidiMap>) {
        let nav_speed = Arc::new(AtomicF32::new(3.0));
        let modbus = Arc::new(ModBus::new());
        let midi_map = Arc::new(MidiMap::new());
        let app = MidiLearnApp::new(nav_speed, Arc::clone(&modbus), Arc::clone(&midi_map));
        (app, modbus, midi_map)
    }

    /// Pressing Learn CC arms the map; the next observed CC completes
    /// a mapping against whatever target is currently browsed.
    #[test]
    fn learn_then_a_real_cc_completes_a_mapping_against_the_browsed_target() {
        let (mut app, modbus, midi_map) = new_app();
        modbus.register("App A: Param");
        modbus.register("App B: Param");
        app.target_browse = 1; // "App B: Param"

        app.press(Selection::LearnCc);
        assert!(midi_map.is_armed());

        midi_map.observe_cc(30, 64, &modbus);
        app.tick(&Input::default());

        assert_eq!(midi_map.mappings(), vec![(30, "App B: Param".to_string())]);
    }

    /// Pressing a mapping row removes exactly that mapping.
    #[test]
    fn pressing_a_mapping_removes_it() {
        let (mut app, _modbus, midi_map) = new_app();
        midi_map.set(10, "A".into());
        midi_map.set(11, "B".into());
        app.press(Selection::Mapping(0));
        assert_eq!(midi_map.mappings(), vec![(11, "B".to_string())]);
    }

    /// Target browsing wraps in both directions across every
    /// registered target.
    #[test]
    fn target_browsing_wraps_both_directions() {
        let (mut app, modbus, _midi_map) = new_app();
        modbus.register("A");
        modbus.register("B");
        modbus.register("C");
        app.edit(Selection::Target, -1);
        assert_eq!(app.target_browse, 2, "backward from 0 must wrap to the last target");
        app.edit(Selection::Target, 1);
        assert_eq!(app.target_browse, 0);
    }
}
