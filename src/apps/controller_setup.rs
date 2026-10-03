//! Controller: map any game controller (a PS5 DualSense, an Xbox pad, a
//! generic USB gamepad) onto Portamax's controls.
//!
//! Two maps, switched on the first row: **Navigate** (home screen and
//! apps without a play view) and **Play** (while an app's play view is up
//! -- the joystick, depth sensors and shoulders live here). Every action
//! is a row showing what it's bound to:
//!
//! - knob 2 press (or turn right) on an action: **learn** -- press the
//!   button, or push the stick / trigger, you want for it. Learning frees
//!   that input from anything else it did in this map, so what you press
//!   is always what you get. Knob 2 press again cancels.
//! - knob 2 turn left: clear the action.
//! - "Reset this map" restores the defaults (the shipped DualSense layout).
//!
//! Changes save to the SD card immediately (`saves/controller_map.json`).
//! The right-hand panel shows the controller live, so you can see what
//! the backend reads before mapping it.

use crate::app::{App, Input};
use crate::controller_map::{self, detect, Action, Context, Snapshot, AXIS_NAMES, BUTTON_NAMES};
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::paramlist::ParamList;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use crate::util::AtomicF32;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

/// Learning gives up after this long without an input.
const LEARN_TIMEOUT_S: f32 = 10.0;

// Controller's own palette: graphite with a signal-green accent.
const BG: Rgb565 = Rgb565::new(2, 5, 3);
const INK: Rgb565 = Rgb565::new(27, 58, 28);
const ACCENT: Rgb565 = Rgb565::new(12, 60, 14);
const DIM: Rgb565 = Rgb565::new(12, 26, 13);
const FAINT: Rgb565 = Rgb565::new(4, 10, 5);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Map,
    Action(Action),
    Reset,
}

struct Learning {
    action: Action,
    rest: Snapshot,
    since: Instant,
}

pub struct ControllerSetupApp {
    nav_speed: Arc<AtomicF32>,
    list: ParamList,
    ctx: Context,
    learning: Option<Learning>,
    status: String,
}

impl ControllerSetupApp {
    pub fn new(nav_speed: Arc<AtomicF32>) -> Self {
        Self { nav_speed, list: ParamList::new(), ctx: Context::Navigate, learning: None, status: String::new() }
    }

    fn rows(&self) -> Vec<Row> {
        let mut v = vec![Row::Map];
        v.extend(Action::all().into_iter().filter(|a| self.ctx == Context::Play || !matches!(a, Action::L1 | Action::R1 | Action::StickX | Action::StickY | Action::StickClick | Action::HandL | Action::HandR)).map(Row::Action));
        v.push(Row::Reset);
        v
    }

    fn bound(&self, a: Action) -> String {
        let pad = controller_map::shared();
        let els = pad.map.lock().map(|m| m.elements_for(self.ctx, a)).unwrap_or_default();
        if els.is_empty() { "-".into() } else { els.iter().map(|e| e.name()).collect::<Vec<_>>().join(", ") }
    }

    fn display_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
            .into_iter()
            .map(|r| match r {
                Row::Map => ("Map".into(), format!("{}  (knob 2)", self.ctx.name()), true),
                Row::Action(a) => {
                    let learning = self.learning.as_ref().is_some_and(|l| l.action == a);
                    (a.name(), if learning { "press / move now...".into() } else { self.bound(a) }, false)
                }
                Row::Reset => ("Reset this map".into(), "knob 2 press".into(), false),
            })
            .collect()
    }

    fn save(&mut self) {
        if let Err(e) = controller_map::shared().save() {
            self.status = format!("Couldn't save: {e}");
        }
    }

    fn start_learning(&mut self, action: Action) {
        let pad = controller_map::shared();
        if !pad.connected.load(Ordering::Relaxed) {
            self.status = "No controller connected".into();
            return;
        }
        let rest = pad.raw.lock().map(|r| *r).unwrap_or_default();
        pad.learning.store(true, Ordering::Relaxed);
        self.learning = Some(Learning { action, rest, since: Instant::now() });
        self.status = format!("{}: press a button or move a stick", action.name());
    }

    fn stop_learning(&mut self) {
        controller_map::shared().learning.store(false, Ordering::Relaxed);
        self.learning = None;
    }

    fn poll_learning(&mut self) {
        let Some(l) = &self.learning else { return };
        let pad = controller_map::shared();
        let now = pad.raw.lock().map(|r| *r).unwrap_or_default();
        if let Some(el) = detect(&l.rest, &now, l.action) {
            let action = l.action;
            if let Ok(mut m) = pad.map.lock() {
                m.learn(self.ctx, action, el);
            }
            self.stop_learning();
            self.save();
            self.status = format!("{} -> {}", action.name(), el.name());
        } else if l.since.elapsed().as_secs_f32() > LEARN_TIMEOUT_S {
            self.stop_learning();
            self.status = "Learn timed out".into();
        }
    }

    fn live(&self) -> (String, bool, Snapshot) {
        let pad = controller_map::shared();
        let connected = pad.connected.load(Ordering::Relaxed);
        let name = if connected { pad.name.lock().map(|n| n.clone()).unwrap_or_default() } else { "No controller connected".into() };
        (name, connected, pad.raw.lock().map(|r| *r).unwrap_or_default())
    }
}

impl App for ControllerSetupApp {
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.display_rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.display_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        (rows[start..end].to_vec(), self.list.selected - start, start > 0, end < rows.len())
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let (name, connected, raw) = self.live();
        crate::app::SlintExtra::Controller(crate::app::ControllerExtra {
            name,
            connected,
            map: self.ctx.name().to_string(),
            learning: self.learning.as_ref().map(|l| l.action.name()).unwrap_or_default(),
            buttons: raw.buttons.to_vec(),
            axes: raw.axes.to_vec(),
            status: self.status.clone(),
        })
    }

    fn on_exit(&mut self) {
        // Never leave the controller deaf because the screen was left mid-learn.
        self.stop_learning();
    }

    fn tick(&mut self, input: &Input) {
        self.poll_learning();
        let rows = self.rows();
        if self.learning.is_some() {
            if input.knob2_press {
                self.stop_learning();
                self.status = "Cancelled".into();
            }
            return;
        }
        self.list.navigate_input(input, rows.len(), self.nav_speed.get() as i32);
        let Some(&row) = rows.get(self.list.selected) else { return };
        match row {
            Row::Map => {
                if input.knob2 != 0 || input.knob2_press {
                    self.ctx = if self.ctx == Context::Navigate { Context::Play } else { Context::Navigate };
                    self.list.selected = 0;
                }
            }
            Row::Action(a) => {
                if input.knob2_press || input.knob2 > 0 {
                    self.start_learning(a);
                } else if input.knob2 < 0 {
                    if let Ok(mut m) = controller_map::shared().map.lock() {
                        m.clear(self.ctx, a);
                    }
                    self.save();
                    self.status = format!("{} cleared", a.name());
                }
            }
            Row::Reset => {
                if input.knob2_press {
                    if let Ok(mut m) = controller_map::shared().map.lock() {
                        m.reset(self.ctx);
                    }
                    self.save();
                    self.status = format!("{} map reset to defaults", self.ctx.name());
                }
            }
        }
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32)).into_styled(PrimitiveStyle::with_fill(BG)).draw(fb).ok();
        Text::new("Controller", Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(fb).ok();
        let rows: Vec<(String, String)> = self.display_rows().into_iter().map(|(a, b, _)| (a, b)).collect();
        self.list.draw_themed(fb, 16, 44, 24, 11, &rows, BG, DIM, ACCENT);

        let (name, connected, raw) = self.live();
        let small = MonoTextStyle::new(&SPLEEN_6X12, INK);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        let (px, py) = (400, 44);
        Text::new(&name, Point::new(px, py), if connected { small } else { dim }).draw(fb).ok();
        for (i, label) in BUTTON_NAMES.iter().enumerate() {
            let x = px + (i as i32 % 2) * 112;
            let y = py + 10 + (i as i32 / 2) * 14;
            let fill = if raw.buttons[i] { ACCENT } else { FAINT };
            Rectangle::new(Point::new(x, y), Size::new(8, 8)).into_styled(PrimitiveStyle::with_fill(fill)).draw(fb).ok();
            let short: String = label.chars().take(16).collect();
            Text::new(&short, Point::new(x + 12, y + 8), if raw.buttons[i] { small } else { dim }).draw(fb).ok();
        }
        for (i, label) in AXIS_NAMES.iter().enumerate() {
            let y = py + 142 + i as i32 * 14;
            let v = raw.axes[i];
            Text::new(label, Point::new(px, y + 8), dim).draw(fb).ok();
            Rectangle::new(Point::new(px + 90, y + 2), Size::new(120, 6)).into_styled(PrimitiveStyle::with_fill(FAINT)).draw(fb).ok();
            let (x0, w) = if i >= 4 { (px + 90, (v.clamp(0.0, 1.0) * 120.0) as i32) } else if v >= 0.0 { (px + 150, (v.min(1.0) * 60.0) as i32) } else { (px + 150 + (v.max(-1.0) * 60.0) as i32, (-v.max(-1.0) * 60.0) as i32) };
            Rectangle::new(Point::new(x0, y + 2), Size::new(w.max(1) as u32, 6)).into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(fb).ok();
        }
        let hint = if self.learning.is_some() { self.status.clone() } else if !self.status.is_empty() { self.status.clone() } else { "knob 2 press: learn   knob 2 left: clear".into() };
        Text::new(&hint, Point::new(16, 337), small).draw(fb).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> ControllerSetupApp {
        ControllerSetupApp::new(Arc::new(AtomicF32::new(1.0)))
    }

    #[test]
    fn learning_binds_what_the_controller_does_and_cancels_cleanly() {
        let pad = controller_map::shared();
        let mut a = app();
        // Row 1 is the first action (D-pad up).
        a.list.selected = 1;
        pad.connected.store(true, Ordering::Relaxed);
        *pad.raw.lock().unwrap() = Snapshot::default();
        a.tick(&Input { knob2_press: true, ..Default::default() });
        assert!(a.learning.is_some() && pad.learning.load(Ordering::Relaxed), "learning mutes the map");
        let mut s = Snapshot::default();
        s.buttons[controller_map::b::L3 as usize] = true;
        *pad.raw.lock().unwrap() = s;
        a.tick(&Input::default());
        assert!(a.learning.is_none() && !pad.learning.load(Ordering::Relaxed));
        assert_eq!(pad.map.lock().unwrap().elements_for(Context::Navigate, Action::NavUp), vec![controller_map::Element::Button(controller_map::b::L3)]);

        *pad.raw.lock().unwrap() = Snapshot::default();
        a.tick(&Input { knob2_press: true, ..Default::default() });
        a.tick(&Input { knob2_press: true, ..Default::default() });
        assert!(a.learning.is_none() && !pad.learning.load(Ordering::Relaxed), "second press cancels");
        a.tick(&Input { knob2: -1, ..Default::default() });
        assert!(pad.map.lock().unwrap().elements_for(Context::Navigate, Action::NavUp).is_empty(), "turn left clears");
        pad.map.lock().unwrap().reset(Context::Navigate);
        pad.connected.store(false, Ordering::Relaxed);
    }

    #[test]
    fn the_play_map_lists_the_instrument_controls() {
        let mut a = app();
        let nav = a.rows().len();
        a.tick(&Input { knob2: 1, ..Default::default() });
        assert_eq!(a.ctx, Context::Play);
        assert!(a.rows().len() > nav, "joystick, hands and shoulders appear");
        assert!(a.display_rows().iter().any(|r| r.0 == "Left hand sensor" && r.1.contains("L2")));
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(ControllerSetupApp::new(ctx.named("nav_speed")))
}
