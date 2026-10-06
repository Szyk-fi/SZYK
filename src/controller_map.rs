//! Game controller mapping: any gamepad's buttons, sticks and triggers
//! onto Portamax's own controls, editable from the Controller app (learn
//! mode) and saved to the SD card (`saves/controller_map.json`).
//!
//! The backends (`gamepad.rs` on macOS via GameController.framework,
//! `gamepad_gilrs.rs` everywhere else and as the macOS fallback) only
//! read the controller into a platform-neutral `Snapshot`; everything
//! about what a button *does* lives here, in two maps:
//!
//! - **Navigate**: used on the home screen and in apps without a play
//!   view.
//! - **Play**: used while a play-view app is on screen (`App::play_surface`).
//!
//! Both defaults follow the control contract (docs/CONTROL_CONTRACT.md) and
//! agree with each other: D-pad up/down browse, left/right change a value,
//! the touchpad click is SELECT (tap to select, hold half a second to reset,
//! exactly like the device's D-pad centre), the PS button is Home. The right
//! stick is the device's joystick (the left stick is unused: it is broken on
//! the test controller), L2 and R2 are unbound, and the four face buttons
//! play pads 1, 5, 9 and 13 (the left column) so small functions can be
//! tried quickly. In play apps L1/R1 are the app's shoulders and Options and
//! Create are F2 and F3; elsewhere the D-pad, shoulders and Options/Create
//! keep doubling as Retro's pad.
//!
//! An element can drive several actions (the default Cross is both
//! "select" and Retro's B button); learning an action rebinds it to one
//! element and frees that element from its other duties in that map, so
//! what you press is always what you get.

use crate::controller::ControllerState;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Buttons in `Snapshot::buttons` order. Names use both PlayStation and
/// Xbox/generic labels since any controller can be mapped.
pub const BUTTON_NAMES: [&str; 18] = [
    "Cross / A",
    "Circle / B",
    "Square / X",
    "Triangle / Y",
    "L1 / LB",
    "R1 / RB",
    "L2 / LT",
    "R2 / RT",
    "L3 (left stick click)",
    "R3 (right stick click)",
    "D-pad up",
    "D-pad down",
    "D-pad left",
    "D-pad right",
    "Options / Start",
    "Create / Select",
    "PS / Home",
    "Touchpad",
];
pub const NUM_BUTTONS: usize = BUTTON_NAMES.len();

/// Axes in `Snapshot::axes` order. Sticks are -1..1 (+x right, +y up);
/// triggers are 0..1.
pub const AXIS_NAMES: [&str; 6] = ["Left stick X", "Left stick Y", "Right stick X", "Right stick Y", "L2 pressure", "R2 pressure"];
pub const NUM_AXES: usize = AXIS_NAMES.len();

pub mod b {
    pub const SOUTH: u8 = 0;
    pub const EAST: u8 = 1;
    pub const WEST: u8 = 2;
    pub const NORTH: u8 = 3;
    pub const L1: u8 = 4;
    pub const R1: u8 = 5;
    pub const L2: u8 = 6;
    pub const R2: u8 = 7;
    pub const L3: u8 = 8;
    pub const R3: u8 = 9;
    pub const UP: u8 = 10;
    pub const DOWN: u8 = 11;
    pub const LEFT: u8 = 12;
    pub const RIGHT: u8 = 13;
    pub const START: u8 = 14;
    pub const SELECT: u8 = 15;
    pub const HOME: u8 = 16;
    pub const TOUCHPAD: u8 = 17;
}
pub mod ax {
    pub const LX: u8 = 0;
    pub const LY: u8 = 1;
    pub const RX: u8 = 2;
    pub const RY: u8 = 3;
    pub const L2: u8 = 4;
    pub const R2: u8 = 5;
}

/// One reading of a controller.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub buttons: [bool; NUM_BUTTONS],
    pub axes: [f32; NUM_AXES],
}

/// A physical input: a button, a whole axis (analog actions), or one
/// direction of an axis (a stick pushed right used as a button, or a
/// trigger's pressure).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Element {
    Button(u8),
    Axis(u8),
    AxisPos(u8),
    AxisNeg(u8),
}

impl Element {
    pub fn name(self) -> String {
        let axis = |a: u8| AXIS_NAMES.get(a as usize).copied().unwrap_or("?");
        match self {
            Element::Button(i) => BUTTON_NAMES.get(i as usize).copied().unwrap_or("?").to_string(),
            Element::Axis(a) => axis(a).to_string(),
            Element::AxisPos(a) if a >= ax::L2 => axis(a).to_string(),
            Element::AxisPos(a) => format!("{} +", axis(a)),
            Element::AxisNeg(a) => format!("{} -", axis(a)),
        }
    }

    /// 0..1 how far this element is pushed (a whole axis reads its
    /// magnitude here; `signed` gives its direction).
    fn amount(self, s: &Snapshot) -> f32 {
        match self {
            Element::Button(i) => s.buttons.get(i as usize).copied().unwrap_or(false) as u8 as f32,
            Element::Axis(a) => s.axes.get(a as usize).copied().unwrap_or(0.0).abs(),
            Element::AxisPos(a) => s.axes.get(a as usize).copied().unwrap_or(0.0).max(0.0),
            Element::AxisNeg(a) => (-s.axes.get(a as usize).copied().unwrap_or(0.0)).max(0.0),
        }
    }

    fn signed(self, s: &Snapshot) -> f32 {
        match self {
            Element::Axis(a) => s.axes.get(a as usize).copied().unwrap_or(0.0),
            _ => self.amount(s),
        }
    }

    fn to_json(self) -> serde_json::Value {
        match self {
            Element::Button(i) => serde_json::json!({ "button": i }),
            Element::Axis(a) => serde_json::json!({ "axis": a }),
            Element::AxisPos(a) => serde_json::json!({ "axis": a, "dir": 1 }),
            Element::AxisNeg(a) => serde_json::json!({ "axis": a, "dir": -1 }),
        }
    }

    fn from_json(v: &serde_json::Value) -> Option<Element> {
        if let Some(i) = v.get("button").and_then(|x| x.as_u64()) {
            return ((i as usize) < NUM_BUTTONS).then_some(Element::Button(i as u8));
        }
        let a = v.get("axis")?.as_u64()? as usize;
        if a >= NUM_AXES {
            return None;
        }
        Some(match v.get("dir").and_then(|d| d.as_i64()) {
            Some(1) => Element::AxisPos(a as u8),
            Some(-1) => Element::AxisNeg(a as u8),
            _ => Element::Axis(a as u8),
        })
    }
}

/// A Portamax control a controller element can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// One of the 16 pads, held for as long as the element is.
    Pad(u8),
    /// F1-F4.
    F(u8),
    Home,
    /// Knob 1 press: select / open / next knob pair.
    Select,
    /// Knob 2 press: reset.
    Reset,
    /// The device D-pad: up/down move through rows (or browse on a play
    /// view), left/right change the value.
    NavUp,
    NavDown,
    ValueUp,
    ValueDown,
    /// Shoulder buttons as a play-view app sees them.
    L1,
    R1,
    StickClick,
    /// Analog: the joystick's axes and the two depth sensors.
    StickX,
    StickY,
    HandL,
    HandR,
}

impl Action {
    /// Every action, in the order the Controller app lists them.
    pub fn all() -> Vec<Action> {
        let mut v = vec![
            Action::NavUp,
            Action::NavDown,
            Action::ValueUp,
            Action::ValueDown,
            Action::Select,
            Action::Reset,
            Action::Home,
            Action::F(0),
            Action::F(1),
            Action::F(2),
            Action::F(3),
            Action::L1,
            Action::R1,
            Action::StickX,
            Action::StickY,
            Action::StickClick,
            Action::HandL,
            Action::HandR,
        ];
        v.extend((0..16).map(Action::Pad));
        v
    }

    pub fn name(self) -> String {
        match self {
            Action::Pad(i) => format!("Pad {}", i + 1),
            Action::F(i) => format!("F{}", i + 1),
            Action::Home => "Home".into(),
            Action::Select => "Select (tap) / Reset (hold)".into(),
            Action::Reset => "Reset only (knob 2 press)".into(),
            Action::NavUp => "Up (row / browse)".into(),
            Action::NavDown => "Down (row / browse)".into(),
            Action::ValueUp => "Right (value +)".into(),
            Action::ValueDown => "Left (value -)".into(),
            Action::L1 => "L1 (play)".into(),
            Action::R1 => "R1 (play / menu)".into(),
            Action::StickClick => "Stick click (keep)".into(),
            Action::StickX => "Joystick X".into(),
            Action::StickY => "Joystick Y".into(),
            Action::HandL => "Left hand sensor".into(),
            Action::HandR => "Right hand sensor".into(),
        }
    }

    /// Actions that want a continuous value rather than on/off.
    pub fn analog(self) -> bool {
        matches!(self, Action::StickX | Action::StickY | Action::HandL | Action::HandR)
    }

    /// Bipolar actions bind a whole axis when learned from a stick.
    fn bipolar(self) -> bool {
        matches!(self, Action::StickX | Action::StickY)
    }

    fn to_json(self) -> serde_json::Value {
        match self {
            Action::Pad(i) => serde_json::json!({ "pad": i }),
            Action::F(i) => serde_json::json!({ "f": i }),
            other => serde_json::json!(format!("{other:?}")),
        }
    }

    fn from_json(v: &serde_json::Value) -> Option<Action> {
        if let Some(i) = v.get("pad").and_then(|x| x.as_u64()) {
            return (i < 16).then_some(Action::Pad(i as u8));
        }
        if let Some(i) = v.get("f").and_then(|x| x.as_u64()) {
            return (i < 4).then_some(Action::F(i as u8));
        }
        Action::all().into_iter().find(|a| v.as_str() == Some(format!("{a:?}").as_str()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    Navigate,
    Play,
}

impl Context {
    pub fn name(self) -> &'static str {
        match self {
            Context::Navigate => "Navigate",
            Context::Play => "Play",
        }
    }
}

pub type Bindings = Vec<(Element, Action)>;

#[derive(Clone, Debug, PartialEq)]
pub struct ControllerMap {
    pub navigate: Bindings,
    pub play: Bindings,
}

impl ControllerMap {
    pub fn defaults() -> ControllerMap {
        use Action::*;
        use Element::*;
        let mut navigate: Bindings = vec![
            // Retro's pad, on the buttons that are not face buttons: the
            // D-pad (pads 0-3), shoulders (8, 9) and Create/Options (10, 11).
            // The face buttons are the test pads below, so Retro's A/B/X/Y
            // are no longer on them; learn them in the Controller app.
            (Button(b::UP), Pad(0)),
            (Button(b::DOWN), Pad(1)),
            (Button(b::LEFT), Pad(2)),
            (Button(b::RIGHT), Pad(3)),
            (Button(b::L1), Pad(8)),
            (Button(b::R1), Pad(9)),
            (Button(b::SELECT), Pad(10)),
            (Button(b::START), Pad(11)),
            // Face buttons: pads 1, 5, 9 and 13, for quick tests.
            (Button(b::WEST), Pad(0)),
            (Button(b::EAST), Pad(4)),
            (Button(b::SOUTH), Pad(8)),
            (Button(b::NORTH), Pad(12)),
            // The device's D-pad: up/down browse, left/right change a value.
            (Button(b::UP), NavUp),
            (Button(b::DOWN), NavDown),
            (Button(b::RIGHT), ValueUp),
            (Button(b::LEFT), ValueDown),
            (Button(b::TOUCHPAD), Select),
            (Button(b::L1), Home),
            (Button(b::R1), F(3)),
            (Button(b::START), Home),
            (Button(b::HOME), Home),
            // F2 and F3 on the stick clicks (the face buttons used to be F1-F3).
            (Button(b::R3), F(1)),
            (Button(b::L3), F(2)),
            // The right stick is the device's joystick, which in a list
            // moves rows up/down and changes the value left/right. Up is
            // "down the list", the direction asked for originally.
            (AxisPos(ax::RY), NavDown),
            (AxisNeg(ax::RY), NavUp),
            (AxisPos(ax::RX), ValueUp),
            (AxisNeg(ax::RX), ValueDown),
        ];
        navigate.shrink_to_fit();
        let play: Bindings = vec![
            (Button(b::UP), NavUp),
            (Button(b::DOWN), NavDown),
            (Button(b::RIGHT), ValueUp),
            (Button(b::LEFT), ValueDown),
            // Face buttons: pads 1, 5, 9 and 13, for quick tests.
            (Button(b::WEST), Pad(0)),
            (Button(b::EAST), Pad(4)),
            (Button(b::SOUTH), Pad(8)),
            (Button(b::NORTH), Pad(12)),
            (Button(b::L1), L1),
            (Button(b::R1), R1),
            (Axis(ax::RX), StickX),
            (Axis(ax::RY), StickY),
            (Button(b::R3), StickClick),
            (Button(b::TOUCHPAD), Select),
            (Button(b::START), F(1)),
            (Button(b::SELECT), F(2)),
            (Button(b::HOME), Home),
        ];
        ControllerMap { navigate, play }
    }

    pub fn bindings(&self, ctx: Context) -> &Bindings {
        match ctx {
            Context::Navigate => &self.navigate,
            Context::Play => &self.play,
        }
    }

    fn bindings_mut(&mut self, ctx: Context) -> &mut Bindings {
        match ctx {
            Context::Navigate => &mut self.navigate,
            Context::Play => &mut self.play,
        }
    }

    pub fn elements_for(&self, ctx: Context, action: Action) -> Vec<Element> {
        self.bindings(ctx).iter().filter(|(_, a)| *a == action).map(|(e, _)| *e).collect()
    }

    /// Binds `action` to exactly `element` in `ctx`, freeing the element
    /// from anything else it did there.
    pub fn learn(&mut self, ctx: Context, action: Action, element: Element) {
        let list = self.bindings_mut(ctx);
        list.retain(|(e, a)| *a != action && !overlaps(*e, element));
        list.push((element, action));
    }

    pub fn clear(&mut self, ctx: Context, action: Action) {
        self.bindings_mut(ctx).retain(|(_, a)| *a != action);
    }

    pub fn reset(&mut self, ctx: Context) {
        let d = ControllerMap::defaults();
        *self.bindings_mut(ctx) = d.bindings(ctx).clone();
    }

    pub fn to_json(&self) -> serde_json::Value {
        let list = |b: &Bindings| serde_json::Value::Array(b.iter().map(|(e, a)| serde_json::json!({ "element": e.to_json(), "action": a.to_json() })).collect());
        serde_json::json!({ "version": 1, "navigate": list(&self.navigate), "play": list(&self.play) })
    }

    pub fn from_json(v: &serde_json::Value) -> Option<ControllerMap> {
        let list = |k: &str| -> Option<Bindings> {
            Some(v.get(k)?.as_array()?.iter().filter_map(|item| Some((Element::from_json(item.get("element")?)?, Action::from_json(item.get("action")?)?))).collect())
        };
        Some(ControllerMap { navigate: list("navigate")?, play: list("play")? })
    }
}

/// Two elements that would fire together: the same button, or parts of
/// the same axis.
fn overlaps(a: Element, b: Element) -> bool {
    let axis = |e: Element| match e {
        Element::Axis(x) | Element::AxisPos(x) | Element::AxisNeg(x) => Some(x),
        Element::Button(_) => None,
    };
    match (a, b) {
        (Element::Button(x), Element::Button(y)) => x == y,
        (Element::Axis(_), _) | (_, Element::Axis(_)) => axis(a).is_some() && axis(a) == axis(b),
        _ => a == b,
    }
}

/// Above this, a stick or trigger used as a button counts as pressed.
const PRESS_THRESHOLD: f32 = 0.5;
/// Analog stick dead zone: sticks rest slightly off-centre.
const ANALOG_DEADZONE: f32 = 0.08;
/// Repeat timing for held navigation, matching the on-screen D-pad.
/// Holding SELECT this long resets instead of selecting.
const SELECT_HOLD: Duration = Duration::from_millis(500);
const REPEAT_DELAY: Duration = Duration::from_millis(250);
const REPEAT_EVERY: Duration = Duration::from_millis(100);

/// How many steps one repeat of a held ◄/► is worth: x1, x5 after 0.75 s,
/// x20 after 1.75 s (the contract's schedule), so a long hold sweeps a whole
/// range in about two seconds instead of crawling at 10 steps a second.
/// Browsing (▲▼) never multiplies: skipping rows would lose your place.
pub fn hold_multiplier(held: Duration) -> i32 {
    match held.as_millis() {
        0..=749 => 1,
        750..=1749 => 5,
        _ => 20,
    }
}

/// Turns snapshots into Portamax input, tracking press edges and
/// key-repeat per binding.
#[derive(Default)]
pub struct Mapper {
    was_down: Vec<bool>,
    next_repeat: Vec<Option<Instant>>,
    /// When each binding went down, for the hold multiplier.
    down_since: Vec<Option<Instant>>,
    /// When a SELECT binding went down, and whether holding it has
    /// already fired Reset (so letting go doesn't also "tap").
    select_since: Vec<Option<Instant>>,
    select_held: Vec<bool>,
    last_ctx: Option<Context>,
}

impl Mapper {
    pub fn apply(&mut self, map: &ControllerMap, snap: &Snapshot, c: &ControllerState, now: Instant) {
        let ctx = if c.play_surface.load(Ordering::Relaxed) { Context::Play } else { Context::Navigate };
        let list = map.bindings(ctx);
        if self.last_ctx != Some(ctx) || self.was_down.len() != list.len() {
            // A map or context change: start clean so nothing fires on
            // a button that was already held.
            self.was_down = list.iter().map(|(e, _)| e.amount(snap) > PRESS_THRESHOLD).collect();
            self.next_repeat = vec![None; list.len()];
            self.down_since = vec![None; list.len()];
            self.select_since = vec![None; list.len()];
            self.select_held = vec![false; list.len()];
            self.last_ctx = Some(ctx);
        }
        let mut pads = [false; 16];
        let mut stick = [0.0f32; 2];
        let mut hands = [0.0f32; 2];
        let mut shoulders = [false; 2];
        for (i, &(el, action)) in list.iter().enumerate() {
            if action.analog() {
                let v = el.signed(snap);
                match action {
                    Action::StickX | Action::StickY => {
                        let shaped = if v.abs() < ANALOG_DEADZONE { 0.0 } else { v.signum() * (v.abs() - ANALOG_DEADZONE) / (1.0 - ANALOG_DEADZONE) };
                        let k = (action == Action::StickY) as usize;
                        if shaped.abs() > stick[k].abs() {
                            stick[k] = shaped;
                        }
                    }
                    Action::HandL => hands[0] = hands[0].max(el.amount(snap)),
                    Action::HandR => hands[1] = hands[1].max(el.amount(snap)),
                    _ => {}
                }
                continue;
            }
            let down = el.amount(snap) > PRESS_THRESHOLD;
            let was = self.was_down[i];
            let edge = down && !was;
            let released = was && !down;
            self.was_down[i] = down;
            if action == Action::Select {
                // Like the device's D-pad centre: a tap selects (on release),
                // holding for half a second resets instead.
                if edge {
                    self.select_since[i] = Some(now);
                    self.select_held[i] = false;
                } else if down && !self.select_held[i] && self.select_since[i].is_some_and(|t| now.duration_since(t) >= SELECT_HOLD) {
                    self.select_held[i] = true;
                    c.set_knob2_press();
                } else if released {
                    if !self.select_held[i] && self.select_since[i].is_some() {
                        c.set_knob1_press();
                    }
                    self.select_since[i] = None;
                }
            }
            if edge {
                self.down_since[i] = Some(now);
            } else if !down {
                self.down_since[i] = None;
            }
            let held = self.down_since[i].map_or(Duration::ZERO, |t| now.duration_since(t));
            let repeat = if edge {
                self.next_repeat[i] = Some(now + REPEAT_DELAY);
                true
            } else if down && self.next_repeat[i].is_some_and(|t| now >= t) {
                self.next_repeat[i] = Some(now + REPEAT_EVERY);
                true
            } else {
                if !down {
                    self.next_repeat[i] = None;
                }
                false
            };
            match action {
                Action::Pad(p) => pads[p as usize % 16] |= down,
                Action::L1 => shoulders[0] |= down,
                Action::R1 => shoulders[1] |= down,
                _ => {}
            }
            if edge {
                match action {
                    Action::F(f) => c.set_top(f as usize % 4),
                    Action::Home => c.set_home(),
                    Action::Reset => c.set_knob2_press(),
                    Action::L1 => c.set_shoulder_press(0),
                    Action::R1 => c.set_shoulder_press(1),
                    Action::StickClick => c.set_stick_click(),
                    _ => {}
                }
            }
            if repeat {
                match action {
                    Action::NavUp => c.add_nav_delta(-1),
                    Action::NavDown => c.add_nav_delta(1),
                    Action::ValueUp => c.add_nav_x(hold_multiplier(held)),
                    Action::ValueDown => c.add_nav_x(-hold_multiplier(held)),
                    _ => {}
                }
            }
        }
        for (slot, v) in c.gamepad_grid.iter().zip(pads) {
            slot.store(v, Ordering::Relaxed);
        }
        c.stick[0].set(stick[0]);
        c.stick[1].set(stick[1]);
        c.hands[0].set(hands[0]);
        c.hands[1].set(hands[1]);
        c.shoulders[0].store(shoulders[0], Ordering::Relaxed);
        c.shoulders[1].store(shoulders[1], Ordering::Relaxed);
    }

    /// The controller went away: let go of everything it was holding.
    pub fn release(&mut self, c: &ControllerState) {
        for slot in &c.gamepad_grid {
            slot.store(false, Ordering::Relaxed);
        }
        c.stick[0].set(0.0);
        c.stick[1].set(0.0);
        c.hands[0].set(0.0);
        c.hands[1].set(0.0);
        c.shoulders[0].store(false, Ordering::Relaxed);
        c.shoulders[1].store(false, Ordering::Relaxed);
        self.last_ctx = None;
    }
}

/// The one physical controller port, shared by the backend threads and
/// the Controller app: the current map, the latest raw reading (for the
/// learn screen) and whether learning is in progress (backends then stop
/// applying the map, so the button you press to learn doesn't also act).
pub struct SharedPad {
    pub map: Mutex<ControllerMap>,
    pub raw: Mutex<Snapshot>,
    pub name: Mutex<String>,
    pub connected: AtomicBool,
    pub learning: AtomicBool,
    /// Set while the macOS GameController backend has a controller, so
    /// the gilrs fallback doesn't apply the same pad twice.
    pub native_active: AtomicBool,
    path: Option<PathBuf>,
}

impl SharedPad {
    fn load(path: Option<PathBuf>) -> SharedPad {
        let map = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| ControllerMap::from_json(&v))
            .unwrap_or_else(ControllerMap::defaults);
        SharedPad {
            map: Mutex::new(map),
            raw: Mutex::new(Snapshot::default()),
            name: Mutex::new(String::new()),
            connected: AtomicBool::new(false),
            learning: AtomicBool::new(false),
            native_active: AtomicBool::new(false),
            path,
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        let json = self.map.lock().map_err(|e| e.to_string())?.to_json();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&json).unwrap_or_default()).map_err(|e| e.to_string())
    }

    /// What a backend calls every poll with a fresh reading.
    pub fn feed(&self, name: &str, snap: Snapshot, mapper: &mut Mapper, c: &ControllerState) {
        if let Ok(mut raw) = self.raw.lock() {
            *raw = snap;
        }
        if !self.connected.swap(true, Ordering::Relaxed) {
            if let Ok(mut n) = self.name.lock() {
                *n = name.to_string();
            }
        }
        if self.learning.load(Ordering::Relaxed) {
            mapper.release(c);
            return;
        }
        if let Ok(map) = self.map.lock() {
            mapper.apply(&map, &snap, c, Instant::now());
        }
    }

    pub fn disconnected(&self, mapper: &mut Mapper, c: &ControllerState) {
        if self.connected.swap(false, Ordering::Relaxed) {
            mapper.release(c);
        }
    }
}

/// The process-wide controller port. Tests get defaults that are never
/// written to disk.
pub fn shared() -> &'static SharedPad {
    static PAD: OnceLock<SharedPad> = OnceLock::new();
    PAD.get_or_init(|| SharedPad::load((!cfg!(test)).then(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves/controller_map.json")))))
}

/// For the learn screen: the first element that moved from `rest`,
/// shaped for `action` (a stick learned for a joystick axis binds the
/// whole axis; anything else binds the direction that moved).
pub fn detect(rest: &Snapshot, now: &Snapshot, action: Action) -> Option<Element> {
    for i in 0..NUM_BUTTONS {
        if now.buttons[i] && !rest.buttons[i] {
            return Some(Element::Button(i as u8));
        }
    }
    let mut best: Option<(f32, usize)> = None;
    for a in 0..NUM_AXES {
        let d = now.axes[a] - rest.axes[a];
        if d.abs() > 0.6 && best.is_none_or(|(m, _)| d.abs() > m) {
            best = Some((d.abs(), a));
        }
    }
    let (_, a) = best?;
    let a8 = a as u8;
    let is_trigger = a8 >= ax::L2;
    Some(if action.bipolar() && !is_trigger {
        Element::Axis(a8)
    } else if now.axes[a] - rest.axes[a] > 0.0 {
        Element::AxisPos(a8)
    } else {
        Element::AxisNeg(a8)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(bt: u8) -> Snapshot {
        let mut s = Snapshot::default();
        s.buttons[bt as usize] = true;
        s
    }

    fn ms(t: Instant, n: u64) -> Instant {
        t + Duration::from_millis(n)
    }

    #[test]
    fn the_dpad_is_the_devices_dpad_in_every_context() {
        for play in [false, true] {
            let (m, c, mut mp) = (ControllerMap::defaults(), ControllerState::new(), Mapper::default());
            c.set_play_surface(play);
            let t = Instant::now();
            mp.apply(&m, &Snapshot::default(), &c, t);
            mp.apply(&m, &press(b::UP), &c, t);
            assert_eq!(c.take_nav_delta(), -1, "up browses up (play: {play})");
            mp.apply(&m, &Snapshot::default(), &c, ms(t, 10));
            mp.apply(&m, &press(b::DOWN), &c, ms(t, 20));
            assert_eq!(c.take_nav_delta(), 1, "down browses down (play: {play})");
            mp.apply(&m, &Snapshot::default(), &c, ms(t, 30));
            mp.apply(&m, &press(b::RIGHT), &c, ms(t, 40));
            assert_eq!((c.take_nav_x(), c.take_knob2_delta()), (1, 1), "right raises the value (play: {play})");
            mp.apply(&m, &Snapshot::default(), &c, ms(t, 50));
            mp.apply(&m, &press(b::LEFT), &c, ms(t, 60));
            assert_eq!(c.take_nav_x(), -1, "left lowers it (play: {play})");
        }
    }

    #[test]
    fn face_buttons_play_pads_1_5_9_and_13_and_the_triggers_are_dead() {
        for play in [false, true] {
            let (m, c, mut mp) = (ControllerMap::defaults(), ControllerState::new(), Mapper::default());
            c.set_play_surface(play);
            let t = Instant::now();
            mp.apply(&m, &Snapshot::default(), &c, t);
            for (button, pad) in [(b::WEST, 0), (b::EAST, 4), (b::SOUTH, 8), (b::NORTH, 12)] {
                mp.apply(&m, &press(button), &c, t);
                for (i, slot) in c.gamepad_grid.iter().enumerate() {
                    assert_eq!(slot.load(Ordering::Relaxed), i == pad, "button {button} lights only pad {} (play: {play})", pad + 1);
                }
                mp.apply(&m, &Snapshot::default(), &c, t);
            }
            let mut triggers = Snapshot::default();
            triggers.buttons[b::L2 as usize] = true;
            triggers.buttons[b::R2 as usize] = true;
            triggers.axes[ax::L2 as usize] = 1.0;
            triggers.axes[ax::R2 as usize] = 1.0;
            mp.apply(&m, &triggers, &c, t);
            assert!((0..4).all(|i| !c.take_top(i)), "L2/R2 press no F button (play: {play})");
            assert_eq!((c.hands[0].get(), c.hands[1].get()), (0.0, 0.0), "and are no hand sensors (play: {play})");
            assert!(c.gamepad_grid.iter().all(|g| !g.load(Ordering::Relaxed)), "and play no pad (play: {play})");
        }
    }

    #[test]
    fn the_touchpad_is_select_tap_to_select_and_hold_to_reset() {
        for play in [false, true] {
            let (m, c, mut mp) = (ControllerMap::defaults(), ControllerState::new(), Mapper::default());
            c.set_play_surface(play);
            let t = Instant::now();
            mp.apply(&m, &Snapshot::default(), &c, t);
            mp.apply(&m, &press(b::TOUCHPAD), &c, t);
            assert!(!c.take_knob1_press(), "nothing yet on the way down (play: {play})");
            mp.apply(&m, &Snapshot::default(), &c, ms(t, 150));
            assert!(c.take_knob1_press(), "a quick tap selects, on release (play: {play})");
            assert!(!c.take_knob2_press());

            mp.apply(&m, &press(b::TOUCHPAD), &c, ms(t, 1000));
            mp.apply(&m, &press(b::TOUCHPAD), &c, ms(t, 1300));
            assert!(!c.take_knob2_press(), "not yet half a second (play: {play})");
            mp.apply(&m, &press(b::TOUCHPAD), &c, ms(t, 1520));
            assert!(c.take_knob2_press(), "held half a second: reset (play: {play})");
            mp.apply(&m, &Snapshot::default(), &c, ms(t, 1700));
            assert!(!c.take_knob1_press(), "and letting go afterwards is not also a tap (play: {play})");
        }
    }

    #[test]
    fn home_is_the_ps_button_and_the_touchpad_no_longer_goes_home() {
        let (m, c, mut mp) = (ControllerMap::defaults(), ControllerState::new(), Mapper::default());
        let t = Instant::now();
        mp.apply(&m, &Snapshot::default(), &c, t);
        mp.apply(&m, &press(b::HOME), &c, t);
        assert!(c.take_home(), "the PS button is Home");
        mp.apply(&m, &Snapshot::default(), &c, ms(t, 20));
        mp.apply(&m, &press(b::TOUCHPAD), &c, ms(t, 40));
        mp.apply(&m, &Snapshot::default(), &c, ms(t, 80));
        assert!(!c.take_home(), "the touchpad is Select now");
    }

    #[test]
    fn the_right_stick_is_the_devices_joystick_in_a_list_too() {
        let (m, c, mut mp) = (ControllerMap::defaults(), ControllerState::new(), Mapper::default());
        let t = Instant::now();
        mp.apply(&m, &Snapshot::default(), &c, t);
        let mut s = Snapshot::default();
        s.axes[ax::RX as usize] = 1.0;
        mp.apply(&m, &s, &c, t);
        assert_eq!(c.take_nav_x(), 1, "right changes the value up");
        s.axes[ax::RX as usize] = 0.0;
        mp.apply(&m, &s, &c, ms(t, 20));
        s.axes[ax::LX as usize] = 1.0;
        s.axes[ax::LY as usize] = 1.0;
        mp.apply(&m, &s, &c, ms(t, 40));
        assert_eq!((c.take_nav_x(), c.take_nav_delta()), (0, 0), "the broken left stick does nothing");
    }

    #[test]
    fn held_navigation_repeats_after_a_delay() {
        let (m, c, mut mp) = (ControllerMap::defaults(), ControllerState::new(), Mapper::default());
        let t = Instant::now();
        mp.apply(&m, &Snapshot::default(), &c, t);
        mp.apply(&m, &press(b::DOWN), &c, t);
        mp.apply(&m, &press(b::DOWN), &c, t + Duration::from_millis(100));
        assert_eq!(c.take_nav_delta(), 1, "one step, then a pause");
        mp.apply(&m, &press(b::DOWN), &c, t + Duration::from_millis(260));
        mp.apply(&m, &press(b::DOWN), &c, t + Duration::from_millis(370));
        assert_eq!(c.take_nav_delta(), 2, "then repeats");
    }

    #[test]
    fn holding_right_sweeps_faster_and_releasing_starts_over() {
        let (m, c, mut mp) = (ControllerMap::defaults(), ControllerState::new(), Mapper::default());
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        mp.apply(&m, &Snapshot::default(), &c, t);
        let mut total = 0;
        for n in (0..=2000).step_by(20) {
            mp.apply(&m, &press(b::RIGHT), &c, ms(n));
            total += c.take_nav_x();
        }
        assert!(total >= 100, "two seconds of hold sweeps a 100-detent range, got {total}");
        mp.apply(&m, &Snapshot::default(), &c, ms(2100));
        mp.apply(&m, &press(b::RIGHT), &c, ms(2200));
        assert_eq!(c.take_nav_x(), 1, "a fresh press is one plain step");
        let mut nav = 0;
        for n in (0..=2000).step_by(20) {
            mp.apply(&m, &press(b::DOWN), &c, ms(3000 + n));
            nav += c.take_nav_delta().abs();
        }
        assert!(nav <= 20, "browsing never multiplies, got {nav}");
    }

    #[test]
    fn play_map_turns_the_pad_into_an_instrument_controller() {
        let (m, c, mut mp) = (ControllerMap::defaults(), ControllerState::new(), Mapper::default());
        c.set_play_surface(true);
        let mut s = Snapshot::default();
        mp.apply(&m, &s, &c, Instant::now());
        s.axes[ax::RX as usize] = 1.0;
        s.axes[ax::RY as usize] = -0.04; // inside the dead zone
        s.buttons[b::NORTH as usize] = true;
        s.buttons[b::R1 as usize] = true;
        mp.apply(&m, &s, &c, Instant::now());
        assert!((c.stick[0].get() - 1.0).abs() < 1e-5 && c.stick[1].get() == 0.0, "the right stick is the joystick");
        assert!(c.gamepad_grid[12].load(Ordering::Relaxed), "Triangle plays pad 13");
        assert!(c.shoulders[1].load(Ordering::Relaxed));
        let i = c.play_surface_input(crate::app::Input::default());
        assert!(i.shoulder_press[1], "R1 press reaches the app (menu toggle)");
        assert!(!c.take_top(3), "and is not F4 here");
    }

    #[test]
    fn learning_rebinds_exclusively_and_survives_json() {
        let mut m = ControllerMap::defaults();
        m.learn(Context::Navigate, Action::F(3), Element::Button(b::SOUTH));
        let f4 = m.elements_for(Context::Navigate, Action::F(3));
        assert_eq!(f4, vec![Element::Button(b::SOUTH)], "R1 no longer F4");
        assert!(!m.elements_for(Context::Navigate, Action::Pad(8)).contains(&Element::Button(b::SOUTH)), "Cross freed from pad 9 (L1 still feeds Retro's L there)");
        assert_eq!(m.elements_for(Context::Navigate, Action::Select), vec![Element::Button(b::TOUCHPAD)], "Select is untouched");
        m.learn(Context::Play, Action::StickY, Element::Axis(ax::LY));
        let back = ControllerMap::from_json(&m.to_json()).unwrap();
        assert_eq!(back, m);
        m.reset(Context::Navigate);
        assert_eq!(m.navigate, ControllerMap::defaults().navigate);
    }

    #[test]
    fn detect_picks_what_moved_from_rest() {
        let mut rest = Snapshot::default();
        rest.axes[ax::LY as usize] = 0.3; // a drifting stick
        let mut now = rest;
        now.axes[ax::LY as usize] = 0.5;
        assert_eq!(detect(&rest, &now, Action::NavUp), None, "drift isn't a press");
        now.axes[ax::RX as usize] = -0.9;
        assert_eq!(detect(&rest, &now, Action::NavUp), Some(Element::AxisNeg(ax::RX)));
        assert_eq!(detect(&rest, &now, Action::StickX), Some(Element::Axis(ax::RX)), "a joystick axis binds whole");
        now.buttons[b::L3 as usize] = true;
        assert_eq!(detect(&rest, &now, Action::StickX), Some(Element::Button(b::L3)));
    }
}
