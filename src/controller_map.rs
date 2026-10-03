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
//!   view. The defaults reproduce the hand-tuned DualSense layout this
//!   project shipped with (D-pad left/right browse, up/down edit, face
//!   buttons as select/F1-F3 and Retro's SNES pad, right stick browses,
//!   left stick unused because of drift on the original controller).
//! - **Play**: used while a play-view app is on screen (`App::play_surface`).
//!   The defaults make the pad an instrument controller: right stick =
//!   joystick, L2/R2 = the two depth sensors, L1/R1 = the shoulders, face
//!   buttons = the bottom row of pads.
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
            Action::Select => "Select (knob 1 press)".into(),
            Action::Reset => "Reset (knob 2 press)".into(),
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
            // Retro's SNES pad on pads 0-11 (see apps/retro.rs).
            (Button(b::UP), Pad(0)),
            (Button(b::DOWN), Pad(1)),
            (Button(b::LEFT), Pad(2)),
            (Button(b::RIGHT), Pad(3)),
            (Button(b::SOUTH), Pad(4)),
            (Button(b::EAST), Pad(5)),
            (Button(b::WEST), Pad(6)),
            (Button(b::NORTH), Pad(7)),
            (Button(b::L1), Pad(8)),
            (Button(b::R1), Pad(9)),
            (Button(b::SELECT), Pad(10)),
            (Button(b::START), Pad(11)),
            // Shell controls: left/right browse, up/down edit.
            (Button(b::LEFT), NavUp),
            (Button(b::RIGHT), NavDown),
            (Button(b::UP), ValueUp),
            (Button(b::DOWN), ValueDown),
            (Button(b::SOUTH), Select),
            (Button(b::EAST), F(0)),
            (Button(b::WEST), F(1)),
            (Button(b::NORTH), F(2)),
            (Button(b::L2), F(1)),
            (Button(b::R2), F(2)),
            (Button(b::L1), Home),
            (Button(b::R1), F(3)),
            (Button(b::START), Home),
            (Button(b::HOME), Home),
            (Button(b::TOUCHPAD), Home),
            // Right stick browses: pushing up/right moves down the list
            // (the direction asked for on the original controller).
            (AxisPos(ax::RY), NavDown),
            (AxisNeg(ax::RY), NavUp),
            (AxisPos(ax::RX), NavDown),
            (AxisNeg(ax::RX), NavUp),
        ];
        navigate.shrink_to_fit();
        let play: Bindings = vec![
            (Button(b::UP), NavUp),
            (Button(b::DOWN), NavDown),
            (Button(b::RIGHT), ValueUp),
            (Button(b::LEFT), ValueDown),
            // Face buttons play the bottom row (pitch ranks 0-3).
            (Button(b::SOUTH), Pad(12)),
            (Button(b::EAST), Pad(13)),
            (Button(b::WEST), Pad(14)),
            (Button(b::NORTH), Pad(15)),
            (Button(b::L1), L1),
            (Button(b::R1), R1),
            (AxisPos(ax::L2), HandL),
            (AxisPos(ax::R2), HandR),
            (Axis(ax::RX), StickX),
            (Axis(ax::RY), StickY),
            (Button(b::R3), StickClick),
            (Button(b::L3), Select),
            (Button(b::START), F(1)),
            (Button(b::SELECT), F(2)),
            (Button(b::HOME), Home),
            (Button(b::TOUCHPAD), Home),
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
const REPEAT_DELAY: Duration = Duration::from_millis(250);
const REPEAT_EVERY: Duration = Duration::from_millis(100);

/// Turns snapshots into Portamax input, tracking press edges and
/// key-repeat per binding.
#[derive(Default)]
pub struct Mapper {
    was_down: Vec<bool>,
    next_repeat: Vec<Option<Instant>>,
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
            let edge = down && !self.was_down[i];
            self.was_down[i] = down;
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
                    Action::Select => c.set_knob1_press(),
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
                    Action::ValueUp => c.add_nav_x(1),
                    Action::ValueDown => c.add_nav_x(-1),
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

    #[test]
    fn defaults_keep_the_shipped_dualsense_layout() {
        let (m, c, mut mp) = (ControllerMap::defaults(), ControllerState::new(), Mapper::default());
        let t = Instant::now();
        mp.apply(&m, &Snapshot::default(), &c, t);
        mp.apply(&m, &press(b::SOUTH), &c, t);
        assert!(c.take_knob1_press(), "Cross selects");
        assert!(c.gamepad_grid[4].load(Ordering::Relaxed), "and is Retro's B on pad 4");
        mp.apply(&m, &press(b::EAST), &c, t);
        assert!(c.take_top(0), "Circle is F1");
        mp.apply(&m, &press(b::LEFT), &c, t);
        assert_eq!(c.take_nav_delta(), -1, "D-pad left browses up");
        mp.apply(&m, &press(b::UP), &c, t);
        assert_eq!(c.take_knob2_delta(), 1, "D-pad up edits");
    }

    #[test]
    fn held_navigation_repeats_after_a_delay() {
        let (m, c, mut mp) = (ControllerMap::defaults(), ControllerState::new(), Mapper::default());
        let t = Instant::now();
        mp.apply(&m, &Snapshot::default(), &c, t);
        mp.apply(&m, &press(b::RIGHT), &c, t);
        mp.apply(&m, &press(b::RIGHT), &c, t + Duration::from_millis(100));
        assert_eq!(c.take_nav_delta(), 1, "one step, then a pause");
        mp.apply(&m, &press(b::RIGHT), &c, t + Duration::from_millis(260));
        mp.apply(&m, &press(b::RIGHT), &c, t + Duration::from_millis(370));
        assert_eq!(c.take_nav_delta(), 2, "then repeats");
    }

    #[test]
    fn play_map_turns_the_pad_into_an_instrument_controller() {
        let (m, c, mut mp) = (ControllerMap::defaults(), ControllerState::new(), Mapper::default());
        c.set_play_surface(true);
        let mut s = Snapshot::default();
        mp.apply(&m, &s, &c, Instant::now());
        s.axes[ax::RX as usize] = 1.0;
        s.axes[ax::RY as usize] = -0.04; // inside the dead zone
        s.axes[ax::R2 as usize] = 0.7;
        s.buttons[b::NORTH as usize] = true;
        s.buttons[b::R1 as usize] = true;
        mp.apply(&m, &s, &c, Instant::now());
        assert!((c.stick[0].get() - 1.0).abs() < 1e-5 && c.stick[1].get() == 0.0);
        assert!((c.hands[1].get() - 0.7).abs() < 1e-5, "R2 pressure is the right hand");
        assert!(c.gamepad_grid[15].load(Ordering::Relaxed), "Triangle plays the bottom-right pad");
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
        assert!(m.elements_for(Context::Navigate, Action::Select).is_empty(), "Cross freed from select");
        assert!(m.elements_for(Context::Navigate, Action::Pad(4)).is_empty(), "and from pad 4");
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
