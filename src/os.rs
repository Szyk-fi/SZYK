//! Game-Boy-style shell: a home menu listing installed apps, and a
//! currently-running app that owns the screen until the user backs out.
//!
//! This only controls what's on screen -- NOT what's making sound. Every
//! app's audio processor was registered once into the shared mix bus at
//! startup (see main.rs) and keeps running for the program's life,
//! mixed together, regardless of which app (or the launcher) is on
//! screen. `on_enter`/`on_exit` below are UI-focus hooks only now (e.g.
//! Settings re-scanning devices when you look at it) -- they don't
//! start or stop any sound.
//!
//! The launcher itself is driven by arrow keys + Enter (Input::nav_*) —
//! a fixed, app-independent way to navigate the menu. Once inside an app,
//! the grid/top buttons/knobs (Input::grid/top/knob1/knob2) are that
//! app's own to interpret; see app.rs.

use crate::app::{App, Input};
use crate::audio_devices::{AudioDeviceState, AudioHost};
use crate::controller::{ControllerState, GRID_NOTES, TOP_NOTES};
use crate::display::{self, FrameBuffer};
use crate::led_output::{LedOutput, PadColor};
use crate::paramlist::{ACCENT, SELECTED_CHIP_BG};
use crate::spleen_fonts::{SPLEEN_6X12, SPLEEN_8X16};
use crate::startup_logo;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{CornerRadii, Line, PrimitiveStyle, Rectangle, RoundedRectangle};
use embedded_graphics::text::Text;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// F1-F4 are OS-level, not app-facing (like `home`/`nav_*` -- see
/// app.rs) -- always visible, always live, the same on every screen.
/// F1 (Home) is first/far-left -- the one button that always gets you
/// somewhere known, so it gets the position your thumb lands on
/// without thinking. F3 (Start/Stop) and F2 (MIDI-armed) are each
/// their own toggle -- they used to share one slot (doing double duty
/// depending on whether the active app had a Start/Stop concept),
/// which meant an app with *both* a transport and a reason to arm/
/// disarm MIDI (e.g. the Sequencer) couldn't get to both. Splitting
/// them onto separate buttons needed a free slot, which is why
/// Settings no longer has a dedicated button here -- it's still just
/// an app in the launcher list like any other, one knob-scroll away.
/// F2/F3 are edge-triggered (see `prev_top`) since toggling every
/// frame they're held would just chatter back and forth; F1/F4 are
/// plain level-triggered jumps (`go_home`/`jump_to`, idempotent if
/// held). Labels for F2-F4 are computed live (see
/// `Os::bottom_bar_labels`/`draw_bottom_bar`) rather than hardcoded
/// here, since `jump_to` silently no-ops when no app with that name
/// was actually discovered (see manifest.rs) -- a missing manifest
/// should show a blank button, not a label that lies about what
/// pressing it does.
const BOTTOM_BAR_HEIGHT: i32 = 17;

/// How long each startup logo stays up before advancing (SZYK -> MX1
/// -> launcher) on its own -- any input (any key/knob/button) skips
/// the *whole* remaining sequence immediately, same as a real
/// device's boot screen letting you mash past the splash.
const SPLASH_DURATION: Duration = Duration::from_secs(2);

/// Which logo the boot sequence is currently showing -- see
/// `Os::splash` and `startup_logo.rs`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SplashStage {
    Szyk,
    Mx1,
}

pub struct Os {
    apps: Vec<(String, Box<dyn App>)>,
    selected: usize,
    active: Option<usize>,
    controller: Arc<ControllerState>,
    audio_host: AudioHost,
    device_state: Arc<AudioDeviceState>,
    /// F3 (Start/Stop) and F2 (MIDI-armed) need the *rising* edge, not
    /// "currently held" -- see the const doc comment above.
    prev_top: [bool; 4],
    /// `Some((stage, deadline))` while the startup logos are still
    /// showing; `None` once the whole sequence has been dismissed (by
    /// running past MX1's deadline, or by any input) -- see
    /// `SPLASH_DURATION`.
    splash: Option<(SplashStage, Instant)>,
    /// Per-app "is the MIDI controller allowed to feed this app's pads"
    /// latch, index-aligned with `apps`, defaulting to *off* -- a
    /// stray Push 2 touch or another connected keyboard shouldn't be
    /// able to trigger notes in whatever app happens to be on screen
    /// until it's explicitly armed (F2, independent of F3's Start/Stop
    /// -- see `toggle_midi`/`bottom_bar_labels`).
    midi_armed: Vec<bool>,
    /// MIDI output to whatever physical controller fed `controller`,
    /// so its pad/button LEDs can mirror on-screen state -- see
    /// `led_output.rs` and `update_leds`.
    leds: LedOutput,
    /// What `update_leds` last actually sent, so it only sends a
    /// message when a pad's/button's color changes rather than
    /// flooding the same Note On every single frame.
    prev_grid_lit: [PadColor; 16],
    prev_top_lit: bool,
}

impl Os {
    pub fn new(
        apps: Vec<(String, Box<dyn App>)>,
        controller: Arc<ControllerState>,
        audio_host: AudioHost,
        device_state: Arc<AudioDeviceState>,
        leds: LedOutput,
    ) -> Self {
        let midi_armed = vec![false; apps.len()];
        Self {
            apps,
            selected: 0,
            active: None,
            controller,
            audio_host,
            device_state,
            prev_top: [false; 4],
            splash: Some((SplashStage::Szyk, Instant::now() + SPLASH_DURATION)),
            midi_armed,
            leds,
            prev_grid_lit: [PadColor::Off; 16],
            prev_top_lit: false,
        }
    }

    pub fn run(mut self) {
        let mut window = display::open_window();
        let mut fb = FrameBuffer::new();

        // Dev convenience: skip the launcher and go straight into an app,
        // e.g. `PORTAMAX_APP=plaits cargo run` -- useful for testing an
        // app without a controller wired up to drive the menu. Skips the
        // startup logo too -- no reason to sit through a boot screen
        // every time this is used to jump straight to one app.
        if let Ok(name) = std::env::var("PORTAMAX_APP") {
            if let Some(i) = self
                .apps
                .iter()
                .position(|(n, _)| n.eq_ignore_ascii_case(&name))
            {
                let (_, app) = &mut self.apps[i];
                app.on_enter();
                self.selected = i;
                self.active = Some(i);
                self.splash = None;
                println!("PORTAMAX_APP: auto-started '{name}'");
            } else {
                eprintln!("PORTAMAX_APP: no app named '{name}'");
            }
        }

        while window.is_open() {
            // If Settings requested a different output device, this is
            // where it actually gets swapped -- see audio_devices.rs.
            self.audio_host.poll(&self.device_state);

            // Every installed app, not just the active one -- see
            // `App::background_tick`'s doc comment for why this exists
            // (continuous work, like CV Out's MIDI CC stream, that
            // can't stop just because you looked at another screen,
            // but also can't safely live on the real-time audio
            // thread).
            for (_, app) in self.apps.iter_mut() {
                app.background_tick();
            }

            // Grid/pad input from the controller is gated by the active
            // app's own MIDI-armed toggle (F2 -- see the `midi_armed`
            // field doc and `bottom_bar_labels`) so it can't leak notes
            // into whatever app happens to be on screen; during the
            // splash there's no active app to arm, so it stays live --
            // any Push 2 pad still cuts the boot screen short, same as
            // every other input already does below.
            let midi_armed = self.splash.is_some() || self.active.map(|i| self.midi_armed[i]).unwrap_or(false);
            let input = Input::poll(&window, &self.controller, midi_armed);

            if let Some((stage, deadline)) = self.splash {
                let any_input = input.grid.iter().any(|&p| p)
                    || input.top.iter().any(|&p| p)
                    || input.knob1 != 0
                    || input.knob2 != 0
                    || input.knob1_press
                    || input.knob2_press
                    || input.home
                    || input.nav_up
                    || input.nav_down
                    || input.nav_select;
                if any_input {
                    self.splash = None;
                } else if Instant::now() >= deadline {
                    self.splash = match stage {
                        SplashStage::Szyk => Some((SplashStage::Mx1, Instant::now() + SPLASH_DURATION)),
                        SplashStage::Mx1 => None,
                    };
                } else {
                    fb.clear(Rgb565::BLACK).ok();
                    self.draw_splash(stage, &mut fb);
                    window.update_with_buffer(fb.buffer(), display::WIDTH, display::HEIGHT).expect("failed to present frame");
                    continue;
                }
            }

            let top_pressed = [
                input.top[0] && !self.prev_top[0],
                input.top[1] && !self.prev_top[1],
                input.top[2] && !self.prev_top[2],
                input.top[3] && !self.prev_top[3],
            ];
            self.prev_top = input.top;

            // F1-F4 preempt everything else this frame, same as `home`
            // does below -- a global jump/toggle shouldn't also feed
            // this frame's input to whatever screen we're leaving.
            if top_pressed[0] {
                if self.active.is_some() { self.go_home(); } else { self.jump_to_role(crate::app::SystemRole::Settings); }
            } else if top_pressed[1] {
                self.toggle_midi();
            } else if top_pressed[2] {
                self.toggle_running();
            } else if top_pressed[3] {
                self.jump_to_role(crate::app::SystemRole::Mixer);
            } else {
                match self.active {
                    None => self.tick_launcher(&input),
                    Some(i) => {
                        if input.home {
                            self.apps[i].1.on_exit();
                            self.active = None;
                        } else {
                            self.apps[i].1.tick(&input);
                        }
                    }
                }
            }

            self.update_leds(&input);

            fb.clear(Rgb565::BLACK).ok();
            match self.active {
                None => self.draw_launcher(&mut fb),
                Some(i) => self.apps[i].1.draw(&mut fb),
            }
            self.draw_bottom_bar(&mut fb);

            window
                .update_with_buffer(fb.buffer(), display::WIDTH, display::HEIGHT)
                .expect("failed to present frame");
        }
    }

    /// Headless diagnostic: composes exactly what `run()`'s loop draws
    /// each frame -- the named app (or the launcher, if `app_name`
    /// doesn't match anything or is `None`), started/held the same
    /// "pad 0 down, transport started if it has one" way
    /// `render_diagnostic_wav` drives audio with, plus the bottom bar
    /// on top -- but into a `FrameBuffer` directly, no window. Lets a
    /// rendered screen be inspected (e.g. dumped to a PNG) without a
    /// real display, the same reasoning behind the WAV render escape
    /// hatch for audio.
    pub fn draw_diagnostic_frame(&mut self, app_name: Option<&str>, fb: &mut FrameBuffer) {
        if let Some(name) = app_name {
            self.jump_to(name);
        }
        let held_input = Input { grid: std::array::from_fn(|i| i == 0), ..Default::default() };
        match self.active {
            None => self.tick_launcher(&Input::default()),
            Some(i) => {
                if self.apps[i].1.running() == Some(false) {
                    self.apps[i].1.toggle_running();
                }
                self.apps[i].1.tick(&held_input);
            }
        }

        fb.clear(Rgb565::BLACK).ok();
        match self.active {
            None => self.draw_launcher(fb),
            Some(i) => self.apps[i].1.draw(fb),
        }
        self.draw_bottom_bar(fb);
    }

    /// Jumps straight to the named app (matched case-insensitively
    /// against the manifest's display `name`, same as the `PORTAMAX_APP`
    /// dev shortcut above) from wherever we currently are -- launcher or
    /// another app. A no-op if that app is already active, or if no
    /// app has that name.
    fn jump_to_role(&mut self, role: crate::app::SystemRole) {
        let Some(i) = self.apps.iter().position(|(_,app)| app.system_role() == Some(role)) else { return };
        if self.active == Some(i) { return; }
        if let Some(old) = self.active { self.apps[old].1.tick(&Input::default()); self.apps[old].1.on_exit(); }
        self.apps[i].1.on_enter(); self.selected = i; self.active = Some(i);
    }

    fn jump_to(&mut self, app_name: &str) {
        let Some(i) = self.apps.iter().position(|(n, _)| n.eq_ignore_ascii_case(app_name)) else {
            return;
        };
        if self.active == Some(i) {
            return;
        }
        if let Some(cur) = self.active {
            self.apps[cur].1.on_exit();
        }
        let (_, app) = &mut self.apps[i];
        app.on_enter();
        self.selected = i;
        self.active = Some(i);
    }

    /// Same "back to launcher" transition `home` already does.
    fn go_home(&mut self) {
        if let Some(i) = self.active {
            self.apps[i].1.tick(&Input::default());
            self.apps[i].1.on_exit();
            self.active = None;
        }
    }

    /// Each top button's own fixed color -- distinct per button (not
    /// just "on") since each does something completely different
    /// (see `TOP_NOTES` for which physical button is which index):
    /// yellow for Home (F1), green for context (F2), blue for Start/Stop
    /// (F3), red for Mixer (F4).
    const TOP_COLORS: [PadColor; 4] = [PadColor::Yellow, PadColor::Green, PadColor::Blue, PadColor::Red];

    /// Drives a physical controller's LEDs to mirror what's actually
    /// happening: the top row (F1-F4) is always lit, each in its own
    /// fixed color (`TOP_COLORS`) -- those always do something no
    /// matter what's on screen -- and each pad shows red while it's
    /// currently held (a note actively playing, on whichever app that
    /// means something to) or whatever color the active app's own
    /// overlay wants otherwise (`App::grid_led_overlay` -- e.g. the
    /// Sequencer's programmed-but-not-playing steps show green). Held
    /// takes priority over the overlay -- a step that's both
    /// "programmed" (green) and being played right now shows the live
    /// red, not the static green, same as the on-screen playhead
    /// border already does. Only ever sends a message when a pad's
    /// color actually changes since the last frame, not every frame.
    fn update_leds(&mut self, input: &Input) {
        self.leds.poll();
        let overlay = match self.active {
            Some(i) => self.apps[i].1.grid_led_overlay(),
            None => [PadColor::Off; 16],
        };
        let grid_color: [PadColor; 16] = std::array::from_fn(|i| if input.grid[i] { PadColor::Red } else { overlay[i] });
        for i in 0..16 {
            if grid_color[i] != self.prev_grid_lit[i] {
                self.leds.note_on(GRID_NOTES[i], grid_color[i].velocity());
            }
        }
        self.prev_grid_lit = grid_color;

        if !self.prev_top_lit {
            for (i, &note) in TOP_NOTES.iter().enumerate() {
                self.leds.note_on(note, Self::TOP_COLORS[i].velocity());
            }
            self.prev_top_lit = true;
        }
    }

    /// F3: toggles whatever the active app's Start/Stop concept
    /// currently controls (see `App::toggle_running`) -- a no-op on
    /// the launcher, or on an app with no such concept (most of them;
    /// `App::toggle_running`'s default is already a no-op).
    fn toggle_running(&mut self) {
        if let Some(i) = self.active {
            self.apps[i].1.toggle_running();
        }
    }

    /// F2: toggles whether the controller's pads are allowed to feed
    /// the active app (see `midi_armed`) -- independent of, and no
    /// longer sharing a button with, Start/Stop (F3) -- an app like
    /// the Sequencer has real uses for both at once. A no-op on the
    /// launcher.
    fn toggle_midi(&mut self) {
        if let Some(i) = self.active {
            if self.apps[i].1.grid_mode_label().is_some() { self.apps[i].1.toggle_grid_mode(); }
            else if self.apps[i].1.supports_pad_lock() { self.midi_armed[i] = !self.midi_armed[i]; }
        }
    }

    /// Labels for the bottom bar's 4 buttons. F2/F3 are placeholders
    /// here -- `draw_bottom_bar` overwrites them live with the active
    /// app's actual Start/Stop/MIDI-armed state. F1 (Home) is first/
    /// far-left as the one button that always gets you somewhere known
    /// no matter what's going on. F4 is computed live since `jump_to`
    /// silently no-ops when no app with that name exists (see the
    /// module-level doc comment on `BOTTOM_BAR_HEIGHT`) -- no
    /// equivalent needed for Home/Start-Stop/MIDI, none of which can
    /// go missing the way a manifest-driven jump target can.
    fn bottom_bar_labels(&self) -> [String; 4] {
        let has = |role| self.apps.iter().any(|(_,app)| app.system_role() == Some(role));
        [
            if self.active.is_some() { "Home".into() } else if has(crate::app::SystemRole::Settings) { "Settings".into() } else { String::new() },
            String::new(), String::new(),
            if has(crate::app::SystemRole::Mixer) { "Mixer".into() } else { String::new() },
        ]
    }

    fn tick_launcher(&mut self, input: &Input) {
        let count = self.apps.len();
        if count == 0 {
            return;
        }
        if input.nav_down {
            self.selected = (self.selected + 1) % count;
        }
        if input.nav_up {
            self.selected = (self.selected + count - 1) % count;
        }
        if input.nav_select {
            let (_, app) = &mut self.apps[self.selected];
            app.on_enter();
            self.active = Some(self.selected);
        }
    }

    /// Thin public hook so `PORTAMAX_RENDER_PNG` (see main.rs) can dump
    /// a splash stage too, the same "look at it without a real window"
    /// reasoning behind `draw_diagnostic_frame`. `PORTAMAX_SPLASH=mx1`
    /// previews the second stage; anything else (including unset)
    /// previews SZYK, the first stage.
    pub fn draw_splash_for_diagnostics(&self, fb: &mut FrameBuffer) {
        let stage = match std::env::var("PORTAMAX_SPLASH") {
            Ok(v) if v.eq_ignore_ascii_case("mx1") => SplashStage::Mx1,
            _ => SplashStage::Szyk,
        };
        self.draw_splash(stage, fb);
    }

    /// The boot sequence: SZYK (the brand) then MX1 (the device),
    /// each up for `SPLASH_DURATION` (or until any input) before the
    /// launcher takes over -- see `startup_logo.rs` for why these are
    /// pre-converted bitmaps instead of SVGs rendered live.
    fn draw_splash(&self, stage: SplashStage, fb: &mut FrameBuffer) {
        let logo = match stage {
            SplashStage::Szyk => &startup_logo::SZYK,
            SplashStage::Mx1 => &startup_logo::MX1,
        };
        logo.draw_centered(fb, Rgb565::WHITE);
    }

    /// The very first screen someone sees, so it uses the same accent
    /// chip + font every app's own menu does (see `paramlist.rs`)
    /// rather than a separate one-off style.
    fn draw_launcher(&self, fb: &mut FrameBuffer) {
        let dim = MonoTextStyle::new(&SPLEEN_8X16, Rgb565::new(18, 36, 18));
        let accent = MonoTextStyle::new(&SPLEEN_8X16, ACCENT);
        // Tighter than the in-app menus' 24px row -- same reasoning
        // Settings' own list uses this exact height for (see
        // settings.rs): this list has no scrolling, so all of it has
        // to fit above the bottom bar. 23px (with headroom for a
        // couple more apps) started overlapping the bottom bar the
        // moment CV Out became the 15th installed app -- caught by
        // actually rendering and looking, not by any test. Comfortably
        // fits up to ~17 at 19px; past that, this needs to become a
        // real scrolling list (see `ParamList`) instead of a smaller
        // constant.
        let row_height = 19;

        for (i, (name, _app)) in self.apps.iter().enumerate() {
            let y = 28 + (i as i32) * row_height;
            if i == self.selected {
                let chip_w = name.chars().count() as u32 * SPLEEN_8X16.character_size.width;
                let chip = Rectangle::new(Point::new(9, y - 13), Size::new(chip_w + 6, 19));
                RoundedRectangle::new(chip, CornerRadii::new(Size::new(3, 3)))
                    .into_styled(PrimitiveStyle::with_fill(SELECTED_CHIP_BG))
                    .draw(fb)
                    .ok();
            }
            let style = if i == self.selected { accent } else { dim };
            Text::new(name, Point::new(12, y), style).draw(fb).ok();
        }
    }

    /// Drawn last, over whatever the launcher or the active app just
    /// drew, so F1-F4 stay visible everywhere -- not something each
    /// app has to remember to leave room for.
    fn draw_bottom_bar(&self, fb: &mut FrameBuffer) {
        let labeled = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));
        let blank = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(8, 12, 8));
        // Dark text on a filled accent chip, not just colored text --
        // same "active" treatment the selected list row uses now (see
        // `paramlist::ACCENT`), applied to whichever segment(s) are
        // currently "on" (Stop running, MIDI armed) rather than every
        // segment looking equally inert.
        let lit_style = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::BLACK);

        let mut labels = self.bottom_bar_labels();
        let active = self.active.map(|i| self.apps[i].1.as_ref());
        labels[2] = active.and_then(|a| a.transport_action()).unwrap_or("").into();
        let f3_chip_lit = active.and_then(|a| a.running()) == Some(true);
        let midi_on = self.active.map(|i| self.midi_armed[i]).unwrap_or(false);
        labels[1] = match active {
            Some(a) => match a.grid_mode_label() {
                Some("STEP") => "PAD MODE".into(), Some(_) => "STEP MODE".into(),
                None if a.supports_pad_lock() => if midi_on { "MIDI ON".into() } else { "MIDI OFF".into() },
                _ => String::new(),
            },
            None => String::new(),
        };
        let f2_chip_lit = active.map(|a| a.grid_mode_label() == Some("PAD") || (a.supports_pad_lock() && midi_on)).unwrap_or(false);

        let seg_w = display::WIDTH as i32 / labels.len() as i32;
        let bar_y = display::HEIGHT as i32 - BOTTOM_BAR_HEIGHT;

        Line::new(Point::new(0, bar_y), Point::new(display::WIDTH as i32, bar_y))
            .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(8, 12, 8), 1))
            .draw(fb)
            .ok();

        for (i, label) in labels.iter().enumerate() {
            let x = i as i32 * seg_w + 6;
            let y = display::HEIGHT as i32 - 4;
            if label.is_empty() {
                Text::new(&format!("F{}", i + 1), Point::new(x, y), blank).draw(fb).ok();
            } else {
                let text = format!("F{}: {label}", i + 1);
                let is_lit_chip = (i == 1 && f2_chip_lit) || (i == 2 && f3_chip_lit);
                if is_lit_chip {
                    let text_w = text.chars().count() as u32 * SPLEEN_6X12.character_size.width;
                    let chip = Rectangle::new(Point::new(x - 3, y - 12), Size::new(text_w + 6, 16));
                    chip.into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(fb).ok();
                }
                let style = if is_lit_chip { lit_style } else { labeled };
                Text::new(&text, Point::new(x, y), style).draw(fb).ok();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "F3: MIDI: Off" is the longest label this bar ever actually
    /// draws (see `draw_bottom_bar`'s F3 match), in one of its 4 equal
    /// segments. At the taller `SPLEEN_6X12` it must still fit that
    /// segment's width and stay below the bar's own top edge -- both
    /// the text itself and its chip, which is sized the same way as
    /// the F2/F3 lit-state chip.
    #[test]
    fn bottom_bar_text_and_chip_fit_their_segment_and_the_bar_height() {
        let mut fb = FrameBuffer::new();
        let style = MonoTextStyle::new(&SPLEEN_6X12, Rgb565::new(18, 36, 18));
        let text = "F3: MIDI: Off";
        let x = 6;
        let y = display::HEIGHT as i32 - 4;

        // Same chip geometry `draw_bottom_bar` uses for the running/
        // MIDI-armed segment, exercised here against the worst-case
        // label.
        let text_w = text.chars().count() as u32 * SPLEEN_6X12.character_size.width;
        let chip = Rectangle::new(Point::new(x - 3, y - 12), Size::new(text_w + 6, 16));
        chip.into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(&mut fb).ok();
        Text::new(text, Point::new(x, y), style).draw(&mut fb).ok();

        let seg_w = display::WIDTH as i32 / 4;
        let bar_y = display::HEIGHT as i32 - BOTTOM_BAR_HEIGHT;

        let mut max_x = 0i32;
        let mut min_y = display::HEIGHT as i32;
        for (i, &pixel) in fb.buffer().iter().enumerate() {
            if pixel != 0 {
                let px = (i % display::WIDTH) as i32;
                let py = (i / display::WIDTH) as i32;
                max_x = max_x.max(px);
                min_y = min_y.min(py);
            }
        }
        assert!(max_x < seg_w, "\"F3: MIDI: Off\" + its chip spilled to x={max_x}, past its segment's right edge at {seg_w}");
        assert!(min_y >= bar_y, "\"F3: MIDI: Off\" + its chip reached y={min_y}, above the bar's own top edge at {bar_y}");
    }
}
