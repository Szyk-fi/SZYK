//! The contract every Portamax OS screen implements. Keeping this trait
//! small is what lets the launcher add more audio tools later without
//! touching the OS or the other apps — that's the "modular app system."

use crate::audio::AudioProcessor;
use crate::controller::ControllerState;
use crate::display::FrameBuffer;
use minifb::{Key, KeyRepeat, Window};

/// One frame's worth of control-surface state, matching the real front
/// panel this device is heading toward: a 4x4 button grid, 4 buttons
/// above it, 2 knobs, and a dedicated home button. This replaces an
/// earlier D-pad placeholder now that the real layout is known — see the
/// PCB's 22 switches, which is roughly 16 (grid) + 4 (top) + a couple
/// more not modeled here yet (power, etc).
///
/// `top`/`home`/`nav_*` are edge-triggered (true only the frame a button
/// transitions from up to down) — right for discrete selection. `grid` is
/// raw held-down state instead, since a synth needs to know when a key
/// is released, not just pressed. Knob fields are a delta in "clicks"
/// since the last frame (the sim has no real encoders, so each keypress
/// is one click; positive = clockwise).
#[derive(Default, Clone, Copy)]
pub struct Input {
    /// 4x4 grid, row-major: `grid[row * 4 + col]`. True while held down.
    pub grid: [bool; 16],
    /// The 4 buttons above the grid (F1-F4). Global quick-nav, not part
    /// of the app-facing control surface -- the OS always shows and
    /// consumes these itself (see os.rs's top bar), the same way it
    /// consumes `home`/`nav_*` below, so apps don't need to check them.
    pub top: [bool; 4],
    pub knob1: i32,
    pub knob2: i32,
    /// Knobs are pushable (a clickable encoder) — real hardware knobs will
    /// be too. Edge-triggered like the other buttons.
    pub knob1_press: bool,
    pub knob2_press: bool,
    /// Always means "back to launcher" — the OS itself consumes this,
    /// apps don't need to check it.
    pub home: bool,
    /// Launcher navigation (arrow keys + Enter). The OS itself consumes
    /// these for the home menu; not part of the app-facing control
    /// surface above, so apps don't need to check them either.
    pub nav_up: bool,
    pub nav_down: bool,
    pub nav_select: bool,
}

impl Input {
    const GRID_KEYS: [Key; 16] = [
        Key::Key1,
        Key::Key2,
        Key::Key3,
        Key::Key4,
        Key::Q,
        Key::W,
        Key::E,
        Key::R,
        Key::A,
        Key::S,
        Key::D,
        Key::F,
        Key::Z,
        Key::X,
        Key::C,
        Key::V,
    ];
    const TOP_KEYS: [Key; 4] = [Key::F1, Key::F2, Key::F3, Key::F4];

    /// Merges the keyboard (for testing without hardware) with whatever
    /// the MIDI controller thread has latched into `controller` since the
    /// last frame — both drive the same `Input`, so either works alone or
    /// together.
    ///
    /// `midi_armed` gates only the *pads* (`grid`) the controller
    /// contributes, per the active app's own "MIDI" toggle (F3 -- see
    /// os.rs) so a Push 2 (or any other class-compliant MIDI gear)
    /// sitting there sending notes can't leak into whatever app
    /// happens to be on screen; F1-F4/knobs/home stay live regardless,
    /// since those are OS-level navigation, not an instrument's note
    /// input.
    pub fn poll(window: &Window, controller: &ControllerState, midi_armed: bool) -> Self {
        let pressed = |k| window.is_key_pressed(k, KeyRepeat::No);

        let mut grid = [false; 16];
        for (i, (slot, key)) in grid.iter_mut().zip(Self::GRID_KEYS).enumerate() {
            *slot = window.is_key_down(key)
                || (midi_armed && controller.grid[i].load(std::sync::atomic::Ordering::Relaxed));
        }
        let mut top = [false; 4];
        for (i, (slot, key)) in top.iter_mut().zip(Self::TOP_KEYS).enumerate() {
            *slot = pressed(key) || controller.take_top(i);
        }

        // Auto-repeating (unlike `pressed`) so holding the key down keeps
        // spinning the knob instead of needing repeated taps.
        let repeating = |k| window.is_key_pressed(k, KeyRepeat::Yes);
        let knob1 = repeating(Key::RightBracket) as i32 - repeating(Key::LeftBracket) as i32
            + controller.take_knob1_delta();
        let knob2 = repeating(Key::Period) as i32 - repeating(Key::Comma) as i32
            + controller.take_knob2_delta();
        let knob1_press = pressed(Key::Backslash) || controller.take_knob1_press();

        Self {
            grid,
            top,
            knob1,
            knob2,
            knob1_press,
            knob2_press: pressed(Key::Slash) || controller.take_knob2_press(),
            home: pressed(Key::Escape) || controller.take_home(),
            // The home menu has no knobs of its own to read, so it
            // reuses knob1 (the same encoder every in-app menu
            // browses its own list with) to scroll, and its press to
            // select -- lets a MIDI controller (or the keyboard knob
            // keys) navigate the launcher too, not just arrow keys/
            // Enter, without needing its own separate mapping.
            nav_up: pressed(Key::Up) || knob1 < 0,
            nav_down: pressed(Key::Down) || knob1 > 0,
            nav_select: pressed(Key::Enter) || knob1_press,
        }
    }
}

pub trait App {
    /// Called once when the launcher switches into this app.
    fn on_enter(&mut self) {}

    /// Called once when the user backs out to the home screen.
    fn on_exit(&mut self) {}

    fn tick(&mut self, input: &Input);

    fn draw(&mut self, fb: &mut FrameBuffer);

    /// Audio tools return their processor here; `main.rs` registers it into
    /// the shared mix bus exactly once at startup, and it keeps
    /// running/mixing for the program's life regardless of which app (or
    /// the launcher) is on screen afterward -- NOT re-installed on
    /// `on_enter`/swapped out on `on_exit` (see os.rs's header comment).
    /// Apps that don't touch audio (a settings screen, say) just leave
    /// this as None.
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        None
    }

    /// Whether this app has some notion of a startable/stoppable thing
    /// (e.g. Bloom/Madness's per-shape `Running`, the Sequencer's
    /// transport) and, if so, its current state -- what the OS's global
    /// Start/Stop button (F3, see os.rs) reflects and controls. `None`
    /// means this app has no such concept, and F3 does nothing while it's
    /// active. Read-only; see `toggle_running` for the action.
    fn running(&self) -> Option<bool> {
        None
    }

    /// Toggles whatever `running()` reports, if anything. Default: no-op,
    /// matching the default `running() -> None`.
    fn toggle_running(&mut self) {}

    /// Called once per frame for *every installed app*, not just the
    /// active one (see `Os::run`'s loop) -- for continuous background
    /// work that has to keep happening no matter what's on screen, the
    /// same way audio processors already do (see `audio_processor`),
    /// but that isn't audio and so doesn't belong on the real-time
    /// audio thread. The one real use so far is CV Out (`apps/cv_out.
    /// rs`) sending its channels out over MIDI CC: that's blocking OS
    /// I/O, which must never run on the audio callback (a stall there
    /// is exactly the kind of real-time deadline miss that causes
    /// audible glitching), so it happens here, driven from the main
    /// thread's existing per-frame loop instead. Default: nothing.
    fn background_tick(&mut self) {}

    /// Extra pad colors this app wants on a physical controller right
    /// now, beyond whatever's currently held (see `Os::update_leds`,
    /// which only asks the active app -- so this never fires for an
    /// app that isn't on screen; a held pad always wins over this,
    /// showing red, since that's a note actually playing *right now*).
    /// Default: nothing extra. The one real use so far is the
    /// Sequencer, whose programmed-but-not-currently-playing steps
    /// show green, and whose playhead shows red while it's on an
    /// active step -- see `apps/sequencer.rs`.
    fn grid_led_overlay(&self) -> [crate::led_output::PadColor; 16] {
        [crate::led_output::PadColor::Off; 16]
    }
}
