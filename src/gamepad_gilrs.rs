//! Game controllers through `gilrs`: the backend for Linux and Windows,
//! and the fallback on macOS for pads Apple's GameController.framework
//! doesn't list (generic USB/HID gamepads). On macOS the native backend
//! (gamepad.rs) wins whenever it has a controller -- `gilrs`'s legacy
//! IOHIDManager path can't see first-party pads there anyway (see
//! gamepad.rs's doc comment), and the two must never apply one pad twice.
//!
//! Like gamepad.rs, this only reads the controller into a
//! `controller_map::Snapshot`; what each input does is the user's
//! mapping (controller_map.rs).

use crate::controller::ControllerState;
use crate::controller_map::{self, ax, b, Mapper, Snapshot};
use gilrs::{Axis, Button, Gilrs};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

pub fn run_gilrs_listener(controller: Arc<ControllerState>) {
    let mut gilrs = match Gilrs::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("gamepad (gilrs): unavailable ({e}); controllers will only work through the native backend");
            return;
        }
    };
    let pad = controller_map::shared();
    let mut mapper = Mapper::default();
    let mut was_connected = false;
    loop {
        // Events keep gilrs' cached state current; we read the state below.
        while gilrs.next_event().is_some() {}
        let native = pad.native_active.load(Ordering::Relaxed);
        let found = if native { None } else { gilrs.gamepads().next().map(|(_, g)| (g.name().to_string(), read_snapshot(&g))) };
        match found {
            Some((name, snap)) => {
                if !was_connected {
                    println!("gamepad (gilrs): {name} connected");
                    was_connected = true;
                }
                pad.feed(&name, snap, &mut mapper, &controller);
            }
            None => {
                if was_connected {
                    was_connected = false;
                    // Only let go if the native backend isn't now driving.
                    if !native {
                        pad.disconnected(&mut mapper, &controller);
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}

fn read_snapshot(g: &gilrs::Gamepad<'_>) -> Snapshot {
    let mut s = Snapshot::default();
    let buttons = [
        (b::SOUTH, Button::South),
        (b::EAST, Button::East),
        (b::WEST, Button::West),
        (b::NORTH, Button::North),
        (b::L1, Button::LeftTrigger),
        (b::R1, Button::RightTrigger),
        (b::L2, Button::LeftTrigger2),
        (b::R2, Button::RightTrigger2),
        (b::L3, Button::LeftThumb),
        (b::R3, Button::RightThumb),
        (b::UP, Button::DPadUp),
        (b::DOWN, Button::DPadDown),
        (b::LEFT, Button::DPadLeft),
        (b::RIGHT, Button::DPadRight),
        (b::START, Button::Start),
        (b::SELECT, Button::Select),
        (b::HOME, Button::Mode),
    ];
    for (i, btn) in buttons {
        s.buttons[i as usize] = g.is_pressed(btn);
    }
    // gilrs already reports stick Y with up positive, like GameController.
    s.axes[ax::LX as usize] = g.value(Axis::LeftStickX);
    s.axes[ax::LY as usize] = g.value(Axis::LeftStickY);
    s.axes[ax::RX as usize] = g.value(Axis::RightStickX);
    s.axes[ax::RY as usize] = g.value(Axis::RightStickY);
    // Analog triggers come as button pressure on most mappings; some pads
    // expose them as the Z axes instead.
    let trigger = |btn: Button, axis: Axis| g.button_data(btn).map(|d| d.value()).unwrap_or(0.0).max(g.value(axis)).clamp(0.0, 1.0);
    s.axes[ax::L2 as usize] = trigger(Button::LeftTrigger2, Axis::LeftZ);
    s.axes[ax::R2 as usize] = trigger(Button::RightTrigger2, Axis::RightZ);
    s
}
