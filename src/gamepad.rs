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
//! stick still browses up/down (reversed from the original mapping, per
//! an earlier explicit request); D-Pad Up/Down duplicate that same
//! browsing action (not reversed -- a real button has no drift to work
//! around, so it gets the plain, intuitive polarity), and D-Pad
//! Left/Right edit the highlighted row's value. The face buttons and
//! shoulders double as the OS's own F1-F4/Home/select controls
//! alongside their existing Retro pad-input role -- see
//! `apply_gamepad_state`'s own comments for the full, current layout.
//!
//! macOS-only, deliberately (`GameController.framework` doesn't exist
//! elsewhere) -- `main.rs` only spawns this listener under `#[cfg(target_os
//! = "macos")]`.

use crate::controller::ControllerState;
use objc2::rc::Retained;
use objc2::Message;
use objc2_foundation::NSArray;
use objc2_game_controller::{GCController, GCControllerButtonInput, GCControllerDirectionPad, GCDualSenseGamepad, GCExtendedGamepad};
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
    cross: bool,
    circle: bool,
    square: bool,
    triangle: bool,
    left_shoulder: bool,
    right_shoulder: bool,
    touchpad: bool,
    last_knob1_tick: Option<std::time::Instant>,
    last_knob2_tick: Option<std::time::Instant>,
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
    let dpad_up_down = is_down(&dpad.up());
    let dpad_down_down = is_down(&dpad.down());
    let dpad_left_down = is_down(&dpad.left());
    let dpad_right_down = is_down(&dpad.right());
    controller.grid[0].store(dpad_up_down, Ordering::Relaxed);
    controller.grid[1].store(dpad_down_down, Ordering::Relaxed);
    controller.grid[2].store(dpad_left_down, Ordering::Relaxed);
    controller.grid[3].store(dpad_right_down, Ordering::Relaxed);
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

    // The OS-level control layout: X = select (dive into the
    // highlighted row), Circle = back (F1 -- the OS's own "return to
    // the home list" button, which also jumps into Settings when
    // already there), Square = F2, Triangle = F3, R1 = jump straight to
    // the Mixer (F4), L1 and the DualSense's own touchpad click = Home.
    // These are real, explicit per-button assignments, not derived from
    // the RetroArch-style face-button remap above -- that remap is only
    // about lining up with a game's own face buttons while playing,
    // and coexists fine with these OS-level meanings since apps only
    // ever look at one or the other depending on what's focused.
    let cross_down = is_down(&gamepad.buttonA());
    if cross_down && !edges.cross {
        controller.set_knob1_press();
    }
    edges.cross = cross_down;

    let circle_down = is_down(&gamepad.buttonB());
    if circle_down && !edges.circle {
        controller.set_top(0);
    }
    edges.circle = circle_down;

    let square_down = is_down(&gamepad.buttonX());
    if square_down && !edges.square {
        controller.set_top(1);
    }
    edges.square = square_down;

    let triangle_down = is_down(&gamepad.buttonY());
    if triangle_down && !edges.triangle {
        controller.set_top(2);
    }
    edges.triangle = triangle_down;

    let left_shoulder_down = is_down(&gamepad.leftShoulder());
    if left_shoulder_down && !edges.left_shoulder {
        controller.set_home();
    }
    edges.left_shoulder = left_shoulder_down;

    let right_shoulder_down = is_down(&gamepad.rightShoulder());
    if right_shoulder_down && !edges.right_shoulder {
        controller.set_top(3);
    }
    edges.right_shoulder = right_shoulder_down;

    // The DualSense's own touchpad-click button isn't part of the
    // generic `GCExtendedGamepad` profile at all (Apple exposes it only
    // on the DualSense-specific subclass) -- retaining+downcasting the
    // same underlying object is how objc2 gets from one to the other;
    // this is a no-op (touchpad simply never presses) on any other
    // controller, not a crash.
    if let Ok(dualsense) = gamepad.retain().downcast::<GCDualSenseGamepad>() {
        let touchpad_down = is_down(&dualsense.touchpadButton());
        if touchpad_down && !edges.touchpad {
            controller.set_home();
        }
        edges.touchpad = touchpad_down;
    }

    let now = std::time::Instant::now();

    // D-Pad Up/Down duplicate the right stick's own browsing action
    // (see below) -- reversed per explicit request (pushing Up now
    // navigates down, and vice versa), same as the right stick's own
    // reversed polarity below.
    let ready_dpad_nav = edges.last_knob1_tick.is_none_or(|t| now.duration_since(t) >= STICK_REPEAT);
    if (dpad_up_down || dpad_down_down) && ready_dpad_nav {
        controller.add_knob1_delta(if dpad_up_down { 1 } else { -1 });
        edges.last_knob1_tick = Some(now);
    }

    // D-Pad Left/Right edit the highlighted row's own value (knob2),
    // repeating at the same rate while held as every other repeating
    // control here -- also reversed per the same request.
    let ready_dpad_edit = edges.last_knob2_tick.is_none_or(|t| now.duration_since(t) >= STICK_REPEAT);
    if (dpad_left_down || dpad_right_down) && ready_dpad_edit {
        controller.add_knob2_delta(if dpad_left_down { 1 } else { -1 });
        edges.last_knob2_tick = Some(now);
    }

    // The left stick is deliberately not read at all -- disabled due to
    // real drift on this controller (spurious nav ticks with the stick
    // sitting still). The right stick still browses the menu (knob1);
    // there's no second stick left to also carry knob2 (editing a
    // value), which the D-Pad's own Left/Right now covers instead.
    let right_stick = gamepad.rightThumbstick();
    *stick2_x = right_stick.xAxis().value();
    *stick2_y = right_stick.yAxis().value();

    if (stick2_x.abs() > STICK_DEADZONE || stick2_y.abs() > STICK_DEADZONE) && ready_dpad_nav {
        // Reversed from the original mapping (`-y.signum()`) per
        // explicit request: pushing the stick the way that used to
        // navigate up now navigates down, and vice versa. (D-Pad
        // Up/Down above is intentionally the other polarity -- see its
        // own comment.)
        let delta = if stick2_y.abs() > stick2_x.abs() { stick2_y.signum() as i32 } else { stick2_x.signum() as i32 };
        controller.add_knob1_delta(delta);
        edges.last_knob1_tick = Some(now);
    }
}
