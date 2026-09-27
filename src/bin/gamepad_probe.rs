//! Standalone `GameController.framework` diagnostic -- touches nothing
//! else in this project. Run with `cargo run --bin gamepad_probe`
//! while the real simulator (`cargo run`) is *not* running, so there's
//! no ambiguity about which process is actually seeing (or not
//! seeing) the controller.
//!
//! Prints whether a controller with an extended-gamepad profile is
//! currently attached, then polls and prints its live button/stick
//! state every half second. Ctrl+C to quit.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("gamepad_probe: this diagnostic is macOS-only (GameController.framework doesn't exist elsewhere)");
}

#[cfg(target_os = "macos")]
fn main() {
    use objc2::rc::Retained;
    use objc2_foundation::NSArray;
    use objc2_game_controller::{GCController, GCDevice, GCExtendedGamepad};

    println!("gamepad_probe: polling GCController.controllers() (GameController.framework)...");

    unsafe fn find_gamepad() -> Option<Retained<GCExtendedGamepad>> {
        let controllers: Retained<NSArray<GCController>> = GCController::controllers();
        println!("gamepad_probe: {} controller(s) attached at the OS level", controllers.count());
        for i in 0..controllers.count() {
            let c = controllers.objectAtIndex(i);
            println!("gamepad_probe:   [{i}] vendorName={:?} extendedGamepad={}", c.vendorName(), c.extendedGamepad().is_some());
            if let Some(gamepad) = c.extendedGamepad() {
                return Some(gamepad);
            }
        }
        None
    }

    let mut was_connected = false;
    loop {
        let gamepad = unsafe { find_gamepad() };
        if gamepad.is_some() != was_connected {
            was_connected = gamepad.is_some();
            println!("gamepad_probe: extended gamepad {}", if was_connected { "AVAILABLE" } else { "not available" });
        }
        if let Some(gamepad) = &gamepad {
            unsafe {
                let dpad = gamepad.dpad();
                println!(
                    "gamepad_probe: dpad(u/d/l/r)=({},{},{},{}) A={} B={} X={} Y={} L1={} R1={} L2={:.2} R2={:.2} LS=({:.2},{:.2}) RS=({:.2},{:.2}) menu={} options={:?}",
                    dpad.up().isPressed(),
                    dpad.down().isPressed(),
                    dpad.left().isPressed(),
                    dpad.right().isPressed(),
                    gamepad.buttonA().isPressed(),
                    gamepad.buttonB().isPressed(),
                    gamepad.buttonX().isPressed(),
                    gamepad.buttonY().isPressed(),
                    gamepad.leftShoulder().isPressed(),
                    gamepad.rightShoulder().isPressed(),
                    gamepad.leftTrigger().value(),
                    gamepad.rightTrigger().value(),
                    gamepad.leftThumbstick().xAxis().value(),
                    gamepad.leftThumbstick().yAxis().value(),
                    gamepad.rightThumbstick().xAxis().value(),
                    gamepad.rightThumbstick().yAxis().value(),
                    gamepad.buttonMenu().isPressed(),
                    gamepad.buttonOptions().map(|b| b.isPressed()),
                );
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}
