//! Real gamepad input via Apple's `GameController.framework` -- the
//! native macOS API for a PS5 DualSense, not the raw-HID approach.
//!
//! **Why not `gilrs`** (this project's first attempt, kept as an
//! unused dependency for reference/possible non-Apple use later): its
//! macOS backend talks to controllers through the legacy
//! `IOHIDManager` API. Modern macOS (Big Sur 11.3+) routes first-
//! party controllers -- DualSense, DualShock 4, Xbox controllers --
//! through `GameController.framework` specifically, and Apple's own
//! docs describe this as the intended path for them. In practice this
//! meant `gilrs` silently saw zero gamepads no matter the connection
//! (Bluetooth or USB) or permissions, confirmed by direct testing (a
//! standalone `gilrs`-only probe -- see `src/bin/gamepad_probe.rs` --
//! detected nothing even while `system_profiler` confirmed the OS
//! itself had the controller correctly paired). This is the real fix,
//! not a workaround: use the framework Apple actually built for this
//! device.
//!
//! Polls `GCController.controllers()` and its `extendedGamepad`
//! profile every ~8ms rather than registering for
//! `NSNotificationCenter` connect/disconnect notifications -- simpler,
//! and correct here since this only ever needs "is a controller
//! attached right now", not to react instantly to a mid-session
//! plug/unplug.
//!
//! Merges into the same shared `ControllerState` the MIDI listener
//! thread already writes into (see controller.rs) -- `Input::poll`
//! doesn't need to know or care where a press came from.
//!
//! What each button *does* is no longer decided here: this file only
//! reads the controller into a `controller_map::Snapshot`, and
//! `controller_map.rs` applies the user's mapping (editable in the
//! Controller app, saved to the SD card). Its defaults reproduce the
//! layout this file used to hard-code.
//!
//! macOS-only, deliberately (`GameController.framework` doesn't exist
//! elsewhere) -- `main.rs` only spawns this listener under `#[cfg(target_os
//! = "macos")]`.

use crate::controller::ControllerState;
use crate::controller_map::{self, b, ax, Mapper, Snapshot};
use objc2::rc::Retained;
use objc2::Message;
use objc2_foundation::NSArray;
use objc2_game_controller::{GCController, GCControllerButtonInput, GCDevice, GCDualSenseGamepad, GCExtendedGamepad};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Runs forever on its own thread (spawned once from `main.rs`,
/// exactly like `run_midi_listener`). Panics anywhere in here are
/// caught and the poll loop restarted rather than letting a stray
/// Objective-C bridging issue take the whole app down with it -- same
/// reasoning `apps/retro.rs`'s own core-panic recovery has for its
/// vendored emulator cores; framework FFI is a similarly reasonable
/// place to expect the occasional surprise.
pub fn run_gamepad_listener(controller: Arc<ControllerState>) {
    loop {
        let session_controller = Arc::clone(&controller);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || run_gamepad_session(&session_controller)));
        if let Err(payload) = outcome {
            let message = payload.downcast_ref::<&str>().copied().map(String::from).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no panic message)".into());
            eprintln!("gamepad: listener panicked ({message}) -- restarting in 1s rather than leaving controller input dead for the rest of the run");
            controller_map::shared().native_active.store(false, Ordering::Relaxed);
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

fn run_gamepad_session(controller: &Arc<ControllerState>) {
    println!("gamepad: polling GameController.framework (macOS native) for a connected controller");
    let pad = controller_map::shared();
    let mut was_connected = false;
    let mut mapper = Mapper::default();

    loop {
        let found = unsafe { current_extended_gamepad() };
        let connected = found.is_some();
        if connected != was_connected {
            was_connected = connected;
            println!("gamepad: {}", if connected { "controller connected" } else { "no controller (waiting -- plug in any time)" });
            pad.native_active.store(connected, Ordering::Relaxed);
            if !connected {
                pad.disconnected(&mut mapper, controller);
            }
        }

        if let Some((gamepad, name)) = found {
            let snap = unsafe { read_snapshot(&gamepad) };
            pad.feed(&name, snap, &mut mapper, controller);
        }

        std::thread::sleep(Duration::from_millis(8));
    }
}

/// The first connected controller's extended-gamepad profile and its
/// name, if any is both attached and actually supports it (a DualSense
/// always does; a bare joystick/wheel might not).
unsafe fn current_extended_gamepad() -> Option<(Retained<GCExtendedGamepad>, String)> {
    let controllers: Retained<NSArray<GCController>> = GCController::controllers();
    for i in 0..controllers.count() {
        let c = controllers.objectAtIndex(i);
        if let Some(gamepad) = c.extendedGamepad() {
            let name = c.vendorName().map(|n| n.to_string()).unwrap_or_else(|| "Game controller".into());
            return Some((gamepad, name));
        }
    }
    None
}

unsafe fn is_down(button: &GCControllerButtonInput) -> bool {
    button.isPressed()
}

/// Everything on the controller, in `controller_map`'s neutral layout.
/// `GCExtendedGamepad`'s `buttonA`/`B`/`X`/`Y` follow Xbox-style physical
/// positions (A = bottom) regardless of brand, which is exactly
/// Cross/Circle/Square/Triangle on a PlayStation pad.
unsafe fn read_snapshot(gamepad: &GCExtendedGamepad) -> Snapshot {
    let mut s = Snapshot::default();
    let mut set = |i: u8, v: bool| s.buttons[i as usize] = v;
    let dpad = gamepad.dpad();
    set(b::UP, is_down(&dpad.up()));
    set(b::DOWN, is_down(&dpad.down()));
    set(b::LEFT, is_down(&dpad.left()));
    set(b::RIGHT, is_down(&dpad.right()));
    set(b::SOUTH, is_down(&gamepad.buttonA()));
    set(b::EAST, is_down(&gamepad.buttonB()));
    set(b::WEST, is_down(&gamepad.buttonX()));
    set(b::NORTH, is_down(&gamepad.buttonY()));
    set(b::L1, is_down(&gamepad.leftShoulder()));
    set(b::R1, is_down(&gamepad.rightShoulder()));
    set(b::L2, is_down(&gamepad.leftTrigger()));
    set(b::R2, is_down(&gamepad.rightTrigger()));
    set(b::L3, gamepad.leftThumbstickButton().is_some_and(|x| is_down(&x)));
    set(b::R3, gamepad.rightThumbstickButton().is_some_and(|x| is_down(&x)));
    // On a DualSense, Apple's `buttonMenu` is Options and `buttonOptions`
    // is Create/Share.
    set(b::START, is_down(&gamepad.buttonMenu()));
    set(b::SELECT, gamepad.buttonOptions().is_some_and(|x| is_down(&x)));
    set(b::HOME, gamepad.buttonHome().is_some_and(|x| is_down(&x)));
    // The touchpad click isn't in the generic profile; Apple exposes it
    // only on the DualSense subclass. Any other controller: never pressed.
    if let Ok(dualsense) = gamepad.retain().downcast::<GCDualSenseGamepad>() {
        set(b::TOUCHPAD, is_down(&dualsense.touchpadButton()));
    }
    let left = gamepad.leftThumbstick();
    let right = gamepad.rightThumbstick();
    s.axes[ax::LX as usize] = left.xAxis().value();
    s.axes[ax::LY as usize] = left.yAxis().value();
    s.axes[ax::RX as usize] = right.xAxis().value();
    s.axes[ax::RY as usize] = right.yAxis().value();
    s.axes[ax::L2 as usize] = gamepad.leftTrigger().value().clamp(0.0, 1.0);
    s.axes[ax::R2 as usize] = gamepad.rightTrigger().value().clamp(0.0, 1.0);
    s
}
