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
//! Button layout mirrors what `gamepad_probe`/the original `gilrs`
//! version aimed for: RetroArch's standard SNES remap (B=Cross,
//! A=Circle, Y=Square, X=Triangle) so the Retro app's own D-Pad/face-
//! button/shoulder/Select-Start pad mapping (see apps/retro.rs) lines
//! up with a real gamepad directly. `GCExtendedGamepad`'s own
//! `buttonA`/`B`/`X`/`Y` naming already follows Xbox-style physical
//! positions (A=bottom, B=right, X=left, Y=top) regardless of brand,
//! which is exactly Cross/Circle/Square/Triangle on a PlayStation pad.
//!
//! Menu navigation (browsing the OS/app list up and down, and diving
//! into the highlighted row) deliberately does *not* use the left
//! stick at all -- this controller's left stick has real drift, so it
//! was dropped entirely rather than fighting spurious ticks. The right
//! stick took over browsing (with up/down reversed from the original
//! mapping, per explicit request), and D-Pad Left/Right became an
//! edge-triggered alternative way to "dive" into the highlighted row
//! (see `apply_gamepad_state`'s own comments for the specifics).
//!
//! macOS-only, deliberately (`GameController.framework` doesn't exist
//! elsewhere) -- `main.rs` only spawns this listener under `#[cfg(target_os
//! = "macos")]`.

use crate::controller::ControllerState;
use objc2::rc::Retained;
use objc2_foundation::NSArray;
use objc2_game_controller::{GCController, GCControllerButtonInput, GCControllerDirectionPad, GCExtendedGamepad};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// Below this, stick deflection is treated as centered -- DualSense
/// sticks report nonzero noise near rest.
const STICK_DEADZONE: f32 = 0.35;
const STICK_REPEAT: Duration = Duration::from_millis(60);

/// Tracks which edge-triggered buttons were already down last poll,
/// so a held button fires its action once, not every ~8ms it stays
/// pressed -- polling has no built-in press/release edge the way
/// `gilrs`'s event stream did, so this app tracks it by hand.
#[derive(Default)]
struct EdgeState {
    menu: bool,
    options: bool,
    left_trigger: bool,
    right_trigger: bool,
    dpad_left: bool,
    dpad_right: bool,
    last_knob1_tick: Option<std::time::Instant>,
}

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
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

fn run_gamepad_session(controller: &Arc<ControllerState>) {
    println!("gamepad: polling GameController.framework (macOS native) for a connected controller");
    let mut was_connected = false;
    let mut edges = EdgeState::default();
    let (mut stick2_x, mut stick2_y) = (0.0f32, 0.0f32);

    loop {
        let gamepad = unsafe { current_extended_gamepad() };
        let connected = gamepad.is_some();
        if connected != was_connected {
            was_connected = connected;
            println!("gamepad: {}", if connected { "controller connected" } else { "no controller (waiting -- plug in any time)" });
        }

        if let Some(gamepad) = gamepad {
            unsafe { apply_gamepad_state(controller, &gamepad, &mut edges, &mut stick2_x, &mut stick2_y) };
        }

        std::thread::sleep(Duration::from_millis(8));
    }
}

/// The first connected controller's extended-gamepad profile, if any
/// is both attached and actually supports it (a DualSense always
/// does; a bare joystick/wheel might not).
unsafe fn current_extended_gamepad() -> Option<Retained<GCExtendedGamepad>> {
    let controllers: Retained<NSArray<GCController>> = GCController::controllers();
    for i in 0..controllers.count() {
        let c = controllers.objectAtIndex(i);
        if let Some(gamepad) = c.extendedGamepad() {
            return Some(gamepad);
        }
    }
    None
}

unsafe fn is_down(button: &GCControllerButtonInput) -> bool {
    button.isPressed()
}

unsafe fn apply_gamepad_state(controller: &ControllerState, gamepad: &GCExtendedGamepad, edges: &mut EdgeState, stick2_x: &mut f32, stick2_y: &mut f32) {
    // Grid pads: level state, mirrors real held-down semantics (same
    // as the keyboard's own grid keys) -- this is what makes the
    // D-Pad/face-buttons/shoulders line up with `apps/retro.rs`'s own
    // NES/SNES pad mapping (grid 0-3 = D-Pad, 4-7 = B/A/Y/X, 8-9 =
    // L/R, 10-11 = Select/Start).
    let dpad: Retained<GCControllerDirectionPad> = gamepad.dpad();
    controller.grid[0].store(is_down(&dpad.up()), Ordering::Relaxed);
    controller.grid[1].store(is_down(&dpad.down()), Ordering::Relaxed);
    let dpad_left_down = is_down(&dpad.left());
    let dpad_right_down = is_down(&dpad.right());
    controller.grid[2].store(dpad_left_down, Ordering::Relaxed);
    controller.grid[3].store(dpad_right_down, Ordering::Relaxed);
    // D-Pad Left/Right double as "menu dive" (select/expand the
    // highlighted row) -- an edge-triggered alternative to clicking the
    // nav stick, since a drifting left stick (see below) makes that
    // stick's own click awkward to land precisely.
    if dpad_left_down && !edges.dpad_left {
        controller.set_knob1_press();
    }
    edges.dpad_left = dpad_left_down;
    if dpad_right_down && !edges.dpad_right {
        controller.set_knob1_press();
    }
    edges.dpad_right = dpad_right_down;
    controller.grid[4].store(is_down(&gamepad.buttonA()), Ordering::Relaxed); // Cross -> B
    controller.grid[5].store(is_down(&gamepad.buttonB()), Ordering::Relaxed); // Circle -> A
    controller.grid[6].store(is_down(&gamepad.buttonX()), Ordering::Relaxed); // Square -> Y
    controller.grid[7].store(is_down(&gamepad.buttonY()), Ordering::Relaxed); // Triangle -> X
    controller.grid[8].store(is_down(&gamepad.leftShoulder()), Ordering::Relaxed); // L1 -> L
    controller.grid[9].store(is_down(&gamepad.rightShoulder()), Ordering::Relaxed); // R1 -> R
    // `buttonOptions`/`buttonMenu` (Share/Options on a DualSense) also
    // double as this app's Select/Start grid pads -- Retro wants both
    // a level-state Select/Start (for held-during-boot combos some
    // games expect) and an edge-triggered Home/F-button meaning
    // elsewhere (see below); reading `isPressed()` twice per button is
    // cheap and keeps both meanings genuinely independent.
    if let Some(options) = gamepad.buttonOptions() {
        controller.grid[10].store(is_down(&options), Ordering::Relaxed);
    }
    controller.grid[11].store(is_down(&gamepad.buttonMenu()), Ordering::Relaxed);

    // Everything past here is edge-triggered (only a fresh press
    // matters, same as the MIDI listener's own latches) -- polling has
    // no built-in edge, so `edges` tracks last-frame state by hand.
    let menu_down = is_down(&gamepad.buttonMenu());
    if menu_down && !edges.menu {
        controller.set_home(); // PS button convention: Menu = Home
    }
    edges.menu = menu_down;

    if let Some(options) = gamepad.buttonOptions() {
        let options_down = is_down(&options);
        edges.options = options_down;
    }

    let left_trigger_down = is_down(&gamepad.leftTrigger());
    if left_trigger_down && !edges.left_trigger {
        controller.set_top(1); // L2 -> F2
    }
    edges.left_trigger = left_trigger_down;

    let right_trigger_down = is_down(&gamepad.rightTrigger());
    if right_trigger_down && !edges.right_trigger {
        controller.set_top(2); // R2 -> F3
    }
    edges.right_trigger = right_trigger_down;

    // The left stick is deliberately not read at all -- disabled due to
    // real drift on this controller (spurious nav ticks with the stick
    // sitting still). The right stick takes over its old job (browsing
    // the menu via knob1) instead of its own old job (editing the
    // selected value via knob2), so it's the only stick driving
    // navigation now; there's no second stick left to also carry knob2,
    // so that role has no analog gamepad source any more (still
    // reachable via the on-screen/keyboard/MIDI paths).
    let right_stick = gamepad.rightThumbstick();
    *stick2_x = right_stick.xAxis().value();
    *stick2_y = right_stick.yAxis().value();

    let now = std::time::Instant::now();
    let ready1 = edges.last_knob1_tick.is_none_or(|t| now.duration_since(t) >= STICK_REPEAT);
    if (stick2_x.abs() > STICK_DEADZONE || stick2_y.abs() > STICK_DEADZONE) && ready1 {
        // Reversed from the original mapping (`-y.signum()`) per
        // explicit request: pushing the stick the way that used to
        // navigate up now navigates down, and vice versa.
        let delta = if stick2_y.abs() > stick2_x.abs() { stick2_y.signum() as i32 } else { stick2_x.signum() as i32 };
        controller.add_knob1_delta(delta);
        edges.last_knob1_tick = Some(now);
    }
}
