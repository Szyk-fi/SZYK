//! The contract every Portamax OS screen implements. Keeping this trait
//! small is what lets the launcher add more audio tools later without
//! touching the OS or the other apps — that's the "modular app system."

#[path = "music_scales.rs"]
pub mod music_scales;
#[path = "play_kit.rs"]
pub mod play_kit;

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
    /// Discrete D-pad/joystick row steps, independent of encoder sensitivity.
    pub navigation_steps: i32,
    pub knob2: i32,
    /// D-pad left/right steps. Portamax has no encoders: the shell adds
    /// these into `knob2` too (so every menu edits with them), and the play
    /// view uses them on its own to turn the focused dial (play_kit.rs).
    pub nav_x: i32,
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
    // --- Play surface: continuous controls an instrument can play with.
    // They only reach an app that asks for them (`App::play_surface`);
    // for every other app the shell keeps using the stick, shoulders and
    // depth sensors for navigation, exactly as before. ---
    /// Joystick, -1..1 each axis, +x right, +y up. Springs to 0.
    pub stick: [f32; 2],
    /// Joystick click (edge).
    pub stick_click: bool,
    /// Depth sensors (left, right): 0 = no hand, rising to 1 as a hand
    /// comes closer. On hardware these are the two time-of-flight
    /// sensors; in the sim, the frame's sensor strips or L2/R2.
    pub hands: [f32; 2],
    /// L1 / R1 held state.
    pub shoulders: [bool; 2],
    /// R1 press edge (L1's is reserved for the shell outside play apps).
    pub shoulder_press: [bool; 2],
    /// MIDI keyboard: held notes by MIDI number, velocity 1..127 (0 = up).
    pub midi_keys: MidiKeys,
    /// MIDI pitch bend, -1..1.
    pub pitch_bend: f32,
    /// MIDI mod wheel (CC1), 0..1.
    pub mod_wheel: f32,
    /// MIDI channel aftertouch, 0..1.
    pub aftertouch: f32,
    /// How firmly each pad is pressed, 0..1 (0 = not touched). On hardware
    /// the pads are pressure sensitive and the driver fills this in; a
    /// Push 2 supplies note-on velocity and polyphonic aftertouch. Pads
    /// with no pressure data (computer keyboard, gamepad) report
    /// `SIM_PAD_PRESSURE` while held, so a pressure route still does
    /// something audible in the simulator.
    pub pad_pressure: [f32; 16],
}

/// What a held pad with no real pressure data reports (see
/// `Input::pad_pressure`).
pub const SIM_PAD_PRESSURE: f32 = 0.6;

/// 128 MIDI note velocities -- a newtype only because `Default` isn't
/// derived for arrays longer than 32.
#[derive(Clone, Copy, PartialEq)]
pub struct MidiKeys(pub [u8; 128]);

impl Default for MidiKeys {
    fn default() -> Self {
        MidiKeys([0; 128])
    }
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
    /// contributes, per the active app's own "MIDI" toggle (F2 -- see
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
                || (midi_armed && controller.grid[i].load(std::sync::atomic::Ordering::Relaxed))
                || controller.gamepad_grid[i].load(std::sync::atomic::Ordering::Relaxed);
        }
        let mut top = [false; 4];
        for (i, (slot, key)) in top.iter_mut().zip(Self::TOP_KEYS).enumerate() {
            *slot = pressed(key) || controller.take_top(i);
        }
        // Real pressure where a driver reported it; held pads without any
        // (keyboard, gamepad, a controller that only sends velocity 0)
        // get the simulator default.
        let pad_pressure: [f32; 16] = std::array::from_fn(|i| {
            if !grid[i] {
                return 0.0;
            }
            let real = controller.pad_pressure[i].get();
            if real > 0.0 { real } else { SIM_PAD_PRESSURE }
        });

        // Auto-repeating (unlike `pressed`) so holding the key down keeps
        // spinning the knob instead of needing repeated taps.
        let repeating = |k| window.is_key_pressed(k, KeyRepeat::Yes);
        let knob1 = repeating(Key::RightBracket) as i32 - repeating(Key::LeftBracket) as i32
            + controller.take_knob1_delta();
        // , and . are the D-pad's left/right (Portamax has no encoders):
        // they edit values everywhere and turn the play view's focused dial.
        let dpad_x = repeating(Key::Period) as i32 - repeating(Key::Comma) as i32;
        let knob2 = dpad_x + controller.take_knob2_delta();
        let nav_x = dpad_x + controller.take_nav_x();
        let knob1_press = pressed(Key::Backslash) || controller.take_knob1_press();
        let navigation_steps = controller.take_nav_delta();

        Self {
            grid,
            top,
            knob1,
            navigation_steps,
            knob2,
            nav_x,
            knob1_press,
            knob2_press: pressed(Key::Slash) || controller.take_knob2_press(),
            home: pressed(Key::Escape) || controller.take_home(),
            // The home menu has no knobs of its own to read, so it
            // reuses knob1 (the same encoder every in-app menu
            // browses its own list with) to scroll, and its press to
            // select -- lets a MIDI controller (or the keyboard knob
            // keys) navigate the launcher too, not just arrow keys/
            // Enter, without needing its own separate mapping.
            nav_up: pressed(Key::Up) || knob1 < 0 || navigation_steps < 0,
            nav_down: pressed(Key::Down) || knob1 > 0 || navigation_steps > 0,
            nav_select: pressed(Key::Enter) || knob1_press,
            pad_pressure,
            ..controller.play_surface_input(Self::keyboard_play_surface(window))
        }
    }

    /// Keyboard stand-ins for the play surface (framebuffer runtime):
    /// arrow keys = joystick, O / P = left / right hand over the depth
    /// sensors, K = stick click, 9 / 0 = L1 / R1 held.
    fn keyboard_play_surface(window: &Window) -> Input {
        let down = |k| window.is_key_down(k);
        let axis = |neg, pos| (down(pos) as i32 - down(neg) as i32) as f32;
        Input {
            stick: [axis(Key::Left, Key::Right), axis(Key::Down, Key::Up)],
            stick_click: window.is_key_pressed(Key::K, KeyRepeat::No),
            hands: [if down(Key::O) { 0.7 } else { 0.0 }, if down(Key::P) { 0.7 } else { 0.0 }],
            shoulders: [down(Key::Key9), down(Key::Key0)],
            shoulder_press: [window.is_key_pressed(Key::Key9, KeyRepeat::No), window.is_key_pressed(Key::Key0, KeyRepeat::No)],
            ..Default::default()
        }
    }
}

/// Optional shell services are identified independently of manifest display names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemRole { Mixer, Settings }

pub trait App {
    /// Configured live routes, frozen buffers, or external control outputs that
    /// must continue after leaving this screen. Transport is handled separately.
    fn needs_background_audio(&self) -> bool { false }
    fn system_role(&self) -> Option<SystemRole> { None }
    /// Only apps that meaningfully consume performance pads can own the pad lock.
    fn supports_pad_lock(&self) -> bool { false }
    /// True for an instrument that plays the joystick, depth sensors,
    /// L1/R1 and MIDI keyboard itself (see the `Input` play-surface
    /// fields). The shell then stops using those for navigation while
    /// this app is on screen.
    fn play_surface(&self) -> bool { false }
    /// The shared play column (see play_kit.rs) when the app is on its
    /// play view; `None` shows the usual parameter list instead.
    fn play_column(&self) -> Option<PlayColumn> { None }
    /// The one line every screen spends above the F bar. Derived from what
    /// the app is showing, so an app can't advertise a control that its
    /// current view ignores; override only for a screen with its own verbs.
    fn hint(&self) -> String {
        match self.play_column() {
            Some(col) => hints::play(&col.layer).into(),
            None if self.play_surface() => hints::MENU_OF_PLAY_APP.into(),
            None => hints::LIST.into(),
        }
    }
    /// The action F3 will perform, supplied by the app rather than inferred by name.
    fn transport_action(&self) -> Option<&'static str> {
        self.running().map(|running| if running { "STOP" } else { "PLAY" })
    }

    /// Called once when the launcher switches into this app.
    fn on_enter(&mut self) {}

    /// Called once when the user backs out to the home screen.
    fn on_exit(&mut self) {}

    fn tick(&mut self, input: &Input);

    fn draw(&mut self, fb: &mut FrameBuffer);

    /// Creates this app's processor when its lazy factory is first opened.
    /// The registry installs a dormant proxy at startup; active transports,
    /// routes, tails and visible screens keep it awake. Closing an idle app
    /// suspends DSP without discarding its settings or recordings.
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

    /// Whether this screen wants the *entire* device screen to itself
    /// -- no OS-drawn bottom bar composited on top (see `Os::run`).
    /// `false` (the default) for every ordinary instrument/menu
    /// screen; Retro's game video is the one real user of this so far
    /// (see `apps::retro::RetroApp`), since a game filling the whole
    /// screen with an F1-F4 strip painted over its bottom edge would
    /// defeat the entire point of "full screen".
    fn wants_fullscreen(&self) -> bool {
        false
    }

    /// Whether this app has its own notion of a grid-wide input mode
    /// (e.g. the Sequencer's Step-sequencing vs. Pad Perform, which
    /// both claim the *entire* 4x4 grid for two different, mutually
    /// exclusive things) and, if so, that mode's short label -- what
    /// the OS's global F2 button (normally "Pad Lock", see os.rs/
    /// slint_home_live.rs) shows and controls instead, while this app
    /// is active. `None` (every app but the Sequencer today) leaves
    /// F2 as Pin. Read-only; see `toggle_grid_mode` for the action.
    /// Deliberately not a fit for something track-scoped like the
    /// Sequencer's own Grid Edit (see its `Selection::GridEdit`) --
    /// that stays a per-track menu toggle, since a single global
    /// button can't say *which* track without more context than F2
    /// has.
    fn grid_mode_label(&self) -> Option<&'static str> {
        None
    }

    /// Cycles whatever `grid_mode_label` reports. Default: no-op,
    /// matching the default `grid_mode_label() -> None`.
    fn toggle_grid_mode(&mut self) {}

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

    /// A bespoke Slint panel's touch area reporting a raw pointer
    /// position, in pixels relative to whatever origin that panel's
    /// own `.slint` markup defines (see the callback it wires up) --
    /// for the one app that needs point-and-click rather than
    /// knob-only input instead of a menu row. Default: nothing. The
    /// one real use so far is Settings' color wheel (`apps/settings.
    /// rs`): `x`/`y` are the click offset from the wheel's center.
    fn slint_pointer_pick(&mut self, x: f32, y: f32) {
        let _ = (x, y);
    }

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

    /// Real menu-list rows `(name, value, is_group)` for an alternate
    /// renderer (a live Slint screen, see examples/slint_*_live.rs)
    /// instead of `draw`'s own embedded_graphics list. Named
    /// `slint_*` rather than reusing each app's own `display_rows`/
    /// etc. so overriding these here can just forward to that
    /// existing inherent method without any name collision. Default:
    /// empty (an app with nothing menu-like to show).
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        Vec::new()
    }

    /// Which row `slint_rows` should show as selected.
    fn slint_scale_info(&self) -> Option<music_scales::ScaleInfo> { None }

    fn slint_selected(&self) -> usize {
        0
    }

    /// `slint_rows`, windowed to at most `visible` rows around the
    /// current selection -- default just returns everything in one
    /// "window" with no scroll indicators, correct for any app whose
    /// list is short enough to never need scrolling; an app with a
    /// longer real list overrides this with real
    /// `ParamList::centered_scroll_window`-backed windowing (see e.g.
    /// `PlaitsApp::windowed_rows`). Returns `(window,
    /// selected_index_in_window, has_more_above, has_more_below)`.
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        // Every app's menu is windowed around its selection, so a long
        // list scrolls instead of running off the bottom of the screen
        // (into the F-button bar). Same windowing as `ParamList`.
        let rows = self.slint_rows();
        let selected = self.slint_selected().min(rows.len().saturating_sub(1));
        let mut list = crate::paramlist::ParamList::new();
        list.selected = selected;
        let (a, b) = list.centered_scroll_window(visible.max(1), rows.len());
        let more_below = b < rows.len();
        (rows[a..b].to_vec(), selected - a, a > 0, more_below)
    }

    /// Real fader-level fraction (0..1) per windowed row, aligned with
    /// `slint_windowed_rows`' own window -- `None` per row for an app
    /// with nothing level-like to show (every app except Mixer).
    fn slint_levels(&mut self, visible: usize) -> Vec<Option<f32>> {
        let _ = visible;
        Vec::new()
    }

    /// Bespoke live state a specific app's live Slint screen wants
    /// beyond the generic `slint_rows`/`slint_levels` list -- e.g.
    /// Plaits' engine-select dots + real analyzer. Default: none.
    /// Named/shaped this way (an enum a caller matches on) rather
    /// than requiring a downcast from `Box<dyn App>`, since the real
    /// `Registry` only ever hands apps out as that trait object.
    fn slint_extra(&mut self) -> SlintExtra {
        SlintExtra::None
    }
}

/// Turns a series of samples into real connected line-segment
/// geometry (not a bar chart) for a live Slint screen -- each
/// consecutive pair of samples becomes one rotated, positioned
/// rectangle, since Slint has no native polyline primitive. Assumes a
/// fixed `width_px`/`height_px` canvas (the live screens size their
/// curve panels to match exactly what this was computed for, rather
/// than a stretchy container, so the angles come out right).
/// `centered`: true maps -1..1 samples around the vertical middle
/// (an oscillator/LFO/oscilloscope trace); false maps 0..1 samples up
/// from the bottom (a filter response or envelope curve). Returns
/// parallel `(mid_x, mid_y, length, angle_deg)` arrays, one entry per
/// segment (samples.len() - 1 of them), all in pixels except the
/// angle.
pub fn polyline_segments(samples: &[f32], width_px: f32, height_px: f32, centered: bool) -> (Vec<f32>, Vec<f32>, Vec<f32>, Vec<f32>) {
    if samples.len() < 2 {
        return (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    }
    let n = samples.len();
    let point = |i: usize| -> (f32, f32) {
        let x = i as f32 / (n - 1) as f32 * width_px;
        let s = samples[i];
        let y = if centered { height_px / 2.0 - s.clamp(-1.0, 1.0) * (height_px / 2.0) } else { height_px - s.clamp(0.0, 1.0) * height_px };
        (x, y)
    };
    let mut mid_x = Vec::with_capacity(n - 1);
    let mut mid_y = Vec::with_capacity(n - 1);
    let mut length = Vec::with_capacity(n - 1);
    let mut angle = Vec::with_capacity(n - 1);
    for i in 0..n - 1 {
        let (x0, y0) = point(i);
        let (x1, y1) = point(i + 1);
        let dx = x1 - x0;
        let dy = y1 - y0;
        mid_x.push((x0 + x1) / 2.0);
        mid_y.push((y0 + y1) / 2.0);
        length.push((dx * dx + dy * dy).sqrt().max(0.5));
        angle.push(dy.atan2(dx).to_degrees());
    }
    (mid_x, mid_y, length, angle)
}

/// See `App::slint_extra`.
pub enum SlintExtra {
    None,
    Plaits(PlaitsExtra),
    Controller(ControllerExtra),
    Analyzer(AnalyzerExtra),
    Voltage(VoltageExtra),
    Cascade(CascadeExtra),
    Shape(ShapeVisual),
    Bloom(BloomVisual),
    Nebula(NebulaExtra),
    Pams(PamsExtra),
    Orbit(OrbitExtra),
    Prism(PrismExtra),
    Nautilus(NautilusExtra),
    Sequencer(SequencerExtra),
    Clouds(CloudsExtra),
    Beads(BeadsExtra),
    BlackHole(BlackHoleExtra),
    QueenOfPentacles(QueenOfPentaclesExtra),
    NaturalGate(NaturalGateExtra),
    TuringMachine(TuringMachineExtra),
    Rainmaker(RainmakerExtra),
    Starlab(StarlabExtra),
    Warps(WarpsExtra),
    Mixer(MixerExtra),
    CvOut(CvOutExtra),
    Magnito(MagnitoExtra),
    Theme(ThemeExtra),
    Tonestack(TonestackExtra),
    SampleDrum(SampleDrumExtra),
    Visualizer(VisualizerExtra),
    VectorFilter(VectorFilterExtra),
    Forge(ForgeExtra),
    Retro(RetroExtra),
    Collection(CollectionExtra),
    Portal(PortalExtra),
    Oracle(OracleExtra),
    Pulsar(PulsarExtra),
    Tinkertone(TinkertoneExtra),
    Norns(NornsExtra),
    Grid(GridExtra),
    Atlas(AtlasExtra),
    Screen(ScreenExtra),
}

/// An app that draws its whole screen itself (the Kids apps): the GUI
/// shows this picture full screen, with no menu column or chrome.
#[allow(dead_code)] // Slint GUI only
pub struct ScreenExtra {
    /// `width * height` RGBA pixels.
    pub frame_rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A text grid panel -- see `GridPanel` in the Slint GUI. Used by
/// Ledger (the pattern), Mosaic (its step rows) and Squeeze (meters).
#[allow(dead_code)] // Slint GUI only
pub struct GridExtra {
    pub caption: String,
    pub title: String,
    /// Row-major cells, `col_x.len()` per row.
    pub cells: Vec<String>,
    /// Each column's x position, px.
    pub col_x: Vec<f32>,
    /// Row to highlight, or -1.
    pub highlight: i32,
    pub footer: String,
    /// A bar under the title (0..1), or negative for none.
    pub meter: f32,
}

/// The norns cartridge's screen and status -- see `NornsApp::slint_extra`.
#[allow(dead_code)] // Slint GUI only
pub struct NornsExtra {
    /// 128x64 RGBA: the script's screen, or the SELECT / PARAMS menus.
    pub frame_rgba: Vec<u8>,
    pub title: String,
    /// "SELECT", "PLAY" or "PARAMS".
    pub mode: String,
    pub status: String,
    pub peak: f32,
}

/// Retro's real per-frame telemetry -- see `RetroApp::slint_extra`.
pub struct RetroExtra {
    /// Every system Retro supports, in chip order.
    pub consoles: Vec<String>,
    pub loaded_name: String,
    pub rom_count: i32,
    pub console_name: String,
    pub rom_name: String,
    pub running: bool,
    pub menu_visible: bool,
    pub status: String,
    /// The active console's own real rendered frame dimensions --
    /// varies by console (and even by game), so the Slint side needs
    /// these to size its `Image` correctly rather than assuming a
    /// fixed resolution.
    pub frame_w: u32,
    pub frame_h: u32,
    /// The actual last rendered frame, raw RGBA8 bytes (`frame_w x
    /// frame_h x 4`), or empty when no game is loaded yet. Not
    /// resized/converted here -- the Slint side turns this straight
    /// into an `Image` since it's already in a format
    /// `slint::SharedPixelBuffer<Rgba8Pixel>` can consume directly.
    pub frame_rgba: Vec<u8>,
}

/// Visualizer's real analysis/scene telemetry -- see
/// `VisualizerApp::slint_extra`. `mode_kind`/`spectrum`/`waveform`/
/// `peak_level`/`rms_level`/`pitch_name` mirror `AnalyzerExtra`
/// exactly (Visualizer wraps `AnalyzerApp` verbatim for these), only
/// populated when `scene_kind == 0`; otherwise the Boxer/Car fields
/// (`bass_level`/`treble_level`/`beat_pulse`/`car_x`) drive the scene.
pub struct VisualizerExtra {
    pub mode_kind: u32,
    pub mode_name: String,
    pub spectrum: Vec<f32>,
    pub waveform: CurveSegments,
    pub peak_level: f32,
    pub rms_level: f32,
    pub pitch_name: String,
    pub scene_kind: u32,
    pub scene_name: String,
    pub bass_level: f32,
    pub treble_level: f32,
    pub beat_pulse: f32,
    pub car_x: f32,
    /// Which side the Cat scene's raised paw is swung to -- flips on
    /// every beat onset, see `VisualizerApp::cat_paw_left`.
    pub cat_paw_left: bool,
    pub monitor_on: bool,
}

/// Sample Drum's real per-channel telemetry -- see
/// `SampleDrumApp::output_visual`.
pub struct SampleDrumExtra {
    pub channel: usize,
    pub sample_name: String,
    pub num_slices: usize,
    /// Which slice the *next* trigger will play (FWD/BKW stepping).
    pub step_index: usize,
    /// The real live output waveform, downsampled once per audio
    /// block.
    pub waveform: Vec<f32>,
    /// The source sample's own waveform (not the live output), peak-
    /// downsampled for display -- see `SampleDrumApp::
    /// source_waveform_and_slices`. Empty when no sample is assigned.
    pub source_waveform: Vec<f32>,
    /// This channel's slice cut points as `(lower, upper)` fractions
    /// (0..1 of the whole sample) -- one entry per slice, already
    /// zero-crossing-snapped if that slicing mode is on. Exactly one
    /// entry, spanning Start..End, when slicing is off.
    pub slice_bounds: Vec<(f32, f32)>,
}

/// Tonestack's real post-chain telemetry -- see
/// `TonestackApp::output_visual`.
pub struct TonestackExtra {
    pub voicing_name: String,
    /// The real post-chain output waveform, downsampled once per
    /// audio block.
    pub waveform: Vec<f32>,
    pub output_peak: f32,
    pub gate_closed: bool,
}

// Read only by the Slint GUI (examples/slint_home_live.rs), which the
// framebuffer binary doesn't build -- hence the allow.
#[allow(dead_code)]
/// One block of Oracle's live signal graph, already laid out in the
/// side panel's own pixel space -- see `OracleApp::slint_extra`.
pub struct OracleNode {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub label: String,
    /// Real per-block output activity, 0..1.
    pub activity: f32,
    /// Per-voice block (instrument patches) vs. shared/global block.
    pub voice: bool,
}

// Read only by the Slint GUI (examples/slint_home_live.rs), which the
// framebuffer binary doesn't build -- hence the allow.
#[allow(dead_code)]
/// Oracle's live patch telemetry -- see `OracleApp::slint_extra`.
pub struct OracleExtra {
    pub patch_name: String,
    pub kind_label: String,
    /// 0 = signal graph, 1 = snapshot grid, 2 = AI thinking /
    /// transcribing, 3 = the AI's explanation text, 4 = listening.
    pub mode: u8,
    pub nodes: Vec<OracleNode>,
    /// Graph wiring as line segments, forward edges first; the first
    /// `forward_edges` segments are forward, the rest feedback.
    pub edges: CurveSegments,
    pub forward_edges: usize,
    /// Real output scope (oldest first), -1..1.
    pub scope: Vec<f32>,
    /// Per snapshot pad: 0 empty, 1 stored, 2 morph A, 3 morph B.
    pub snaps: [u8; 16],
    /// Morph position when two snapshots are being morphed.
    pub morph: Option<f32>,
    pub info: String,
    pub status: String,
    pub unstable: bool,
    pub explain: String,
    pub thinking: String,
    /// Live voice-input level while listening, 0..1.
    pub mic_level: f32,
}

// Read only by the Slint GUI (examples/slint_home_live.rs), which the
// framebuffer binary doesn't build -- hence the allow.
#[allow(dead_code)]
/// Pulsar's live pattern state -- see `PulsarApp::slint_extra`.
pub struct PulsarExtra {
    pub genre_name: String,
    pub bpm: f32,
    pub slot: usize,
    pub playing_slot: Option<usize>,
    /// Per slot A-H: holds a pattern.
    pub slot_filled: [bool; 8],
    pub in_fill: bool,
    pub bar: usize,
    pub bars: usize,
    pub pad_mode: String,
    pub record: bool,
    pub lane_names: [&'static str; 8],
    pub lane: usize,
    pub lane_locked: [bool; 8],
    pub lane_muted: [bool; 8],
    pub lane_flash: [f32; 8],
    /// 8 lanes x 16 steps of the bar in view, row-major: velocity
    /// 0..1 (0 = no hit).
    pub cell_vel: Vec<f32>,
    /// Same layout: 0 plain, 1 chance < 100 %, 2 roll/ratchet.
    pub cell_mark: Vec<i32>,
    /// Column of the playhead within the bar in view, if it's there.
    pub playhead: Option<usize>,
    /// Step-edit cursor (lane, column) when the pads are in Steps mode.
    pub cursor: Option<(usize, usize)>,
    pub swing_pct: f32,
    pub peak: f32,
    pub status: String,
}

// Read only by the Slint GUI (examples/slint_home_live.rs), which the
// framebuffer binary doesn't build -- hence the allow.
#[allow(dead_code)]
/// Tinkertone's live state -- see `TinkertoneApp::panel_extra`.
pub struct TinkertoneExtra {
    /// 37 melody keys, held or not.
    pub keys_held: Vec<bool>,
    /// First key of the 16-pad window.
    pub window: usize,
    pub bass_layer: bool,
    /// Bass key held / sounding (0..15), or -1.
    pub bass_held: i32,
    pub bass_sounding: i32,
    pub preset: usize,
    pub preset_tones: Vec<String>,
    pub vibrato: bool,
    pub sustain: bool,
    pub rhythm: usize,
    pub tempo: f32,
    pub playing: bool,
    pub synchro: bool,
    pub fill: bool,
    /// "Manual", "Auto" or "My Line".
    pub bass_mode: String,
    /// Recording your own line.
    pub line_rec: bool,
    /// Your line, one entry per step: bass key, -1 hold, -2 rest.
    pub line: Vec<i32>,
    /// Step of the line playing now, or -1.
    pub line_pos: i32,
    pub chord: String,
    pub step: usize,
    pub steps: usize,
    pub steps_per_beat: usize,
    pub bar: usize,
    pub drum_flash: Vec<f32>,
    pub volume: f32,
    pub accomp: f32,
    pub peak: f32,
}

/// Settings' live color-wheel state -- see `SettingsApp::slint_extra`.
pub struct ThemeExtra {
    pub section: usize,
    pub detail: String,
    pub value: String,
    pub help: String,
    pub output: String,
    pub input: String,
    pub device_count: usize,
    /// True while the wheel/Hue/Saturation/Brightness rows are
    /// editing the background instead of the accent.
    pub editing_background: bool,
    pub hue: f32,
    pub saturation: f32,
    pub brightness: f32,
    /// The currently-edited color's marker position on the wheel,
    /// pixels offset from its center (see `ThemeColor::wheel_marker`).
    pub marker_x: f32,
    pub marker_y: f32,
    pub accent_rgb: (u8, u8, u8),
    pub background_rgb: (u8, u8, u8),
}

/// Magnito's real hysteresis-loop XY trace + tape-character state --
/// see `MagnitoApp::output_visual`.
pub struct MagnitoExtra {
    /// Real connected-line-segment geometry (see `polyline_segments`-
    /// style construction) of the (input, output) XY trace across the
    /// hysteresis follower -- a straight diagonal = no coloration, a
    /// fattened/tilted S-curve = real saturation, visible splitting
    /// between the rising/falling halves = the genuine memory effect.
    pub loop_trace: CurveSegments,
    pub bias: f32,
    pub unlimited: bool,
    /// Real connected-line-segment geometry of the combined Wow+
    /// Flutter modulation depth over time.
    pub wobble_trace: CurveSegments,
    pub wow_flutter_depth: f32,
    pub hiss_level: f32,
    pub wear: f32,
    pub dropout_active: bool,
    pub output_peak: f32,
}

/// CV Out's real 32-channel output state -- see
/// `CvOutApp::slint_extra`.
pub struct CvOutExtra {
    pub midi_channel: u32,
    /// Each channel's real combined output (manual offset + external
    /// modulation), 0..1 -- the same value actually sent as a MIDI CC.
    pub levels: [f32; 32],
}

/// The real mixer-strip state -- see `MixerApp::output_visual`.
pub struct MixerExtra {
    pub master: f32,
    /// One entry per channel: `(name, fader level 0..1.5, live peak
    /// amplitude this block 0..~1.5)`.
    pub channels: Vec<(String, f32, f32)>,
}

/// Warps' real carrier/modulator/output traces + vocoder band levels
/// -- see `WarpsApp::output_visual`.
pub struct WarpsExtra {
    pub algorithm_name: String,
    pub next_algorithm_name: Option<String>,
    pub crossfade_frac: f32,
    pub is_vocoder: bool,
    pub vocoder_frozen: bool,
    pub osc_enabled: bool,
    pub osc_waveform_name: String,
    /// Real connected-line-segment geometry (see `polyline_segments`)
    /// for each trace.
    pub carrier_trace: CurveSegments,
    pub modulator_trace: CurveSegments,
    pub output_trace: CurveSegments,
    pub vocoder_band_levels: [f32; 12],
}

/// Nautilus' real 8-line delay network + mode/Chroma/Sonar state --
/// see `NautilusApp::output_visual`. Flattened out of its native
/// `[T; 8]`-per-field shape (Slint has no array-of-struct property
/// type).
pub struct NautilusExtra {
    pub delay_mode_name: String,
    pub delay_mode_index: usize,
    pub feedback_mode_name: String,
    pub feedback_mode_index: usize,
    pub chroma_name: String,
    pub chroma_index: usize,
    pub frozen: bool,
    pub sensors: usize,
    pub reversal_count: usize,
    pub line_delay_ms: [f32; 8],
    pub line_pulse: [bool; 8],
    pub line_level: [f32; 8],
    pub line_active: [bool; 8],
    pub sonar_gate: bool,
    pub sonar_cv: f32,
    pub feedback_amount: f32,
}

/// StarLab's real oscilloscope + texture/LFO/tank-energy state -- see
/// `StarlabApp::output_visual`.
pub struct StarlabExtra {
    pub waveform: CurveSegments,
    pub texture_name: String,
    pub texture_index: usize,
    pub infinite: bool,
    pub karplus_mode: bool,
    pub lfo_phase: f32,
    pub lfo_value: f32,
    pub tank_energy: f32,
}

/// Rainmaker's real 16-tap timing map -- see
/// `RainmakerApp::output_visual`.
pub struct RainmakerExtra {
    pub groove_name: String,
    pub groove_amount: f32,
    pub grid_label: String,
    pub beats_spanned: f32,
    /// One entry per tap: `(time_fraction 0..1, level 0..1, pan
    /// -1..1, muted)`.
    pub taps: Vec<(f32, f32, f32, bool)>,
}

/// Turing Machine's real 16-bit shift register + Locks-derived lock
/// state -- see `TuringMachineApp::output_visual`.
pub struct TuringMachineExtra {
    pub bits: [bool; 16],
    pub active_len: usize,
    pub write_index: usize,
    pub pulse: bool,
    pub cv: f32,
    pub keep_probability: f32,
    pub inverted_feedback: bool,
    pub double_locked: bool,
    pub exp_gate: bool,
    pub volts_cv: f32,
}

/// Natural Gate's real per-channel envelope trace + gate-open/hit
/// state -- see `NaturalGateApp::output_visual`. Flattened out of its
/// native `[ChannelVisual; 2]` shape (Slint has no array-of-struct
/// property type) into two explicit channel slots.
pub struct NaturalGateExtra {
    /// Real connected-line-segment geometry (see `polyline_segments`)
    /// of each channel's envelope history.
    pub ch1_trace: CurveSegments,
    pub ch1_now: f32,
    pub ch1_open: f32,
    pub ch1_hit: bool,
    pub ch2_trace: CurveSegments,
    pub ch2_now: f32,
    pub ch2_open: f32,
    pub ch2_hit: bool,
}

/// Queen of Pentacles' real chaotic-map trajectory + live CV/Gate/
/// Delta output state -- see `QueenOfPentaclesApp::output_visual`.
pub struct QueenOfPentaclesExtra {
    pub map_name: String,
    pub r: f32,
    pub frozen: bool,
    /// Real connected-line-segment geometry (see `polyline_segments`)
    /// of the map's raw value over its last `HISTORY_LEN` internal
    /// clock ticks.
    pub trajectory: CurveSegments,
    pub cv: f32,
    pub smooth_cv: f32,
    pub gate: bool,
    pub gate_mode_name: String,
    pub delta: f32,
    pub threshold: f32,
    pub bipolar: bool,
}

/// Black Hole's real category badge + oscilloscope + per-algorithm
/// parameter meters -- see `BlackHoleApp::output_visual`.
pub struct BlackHoleExtra {
    pub algorithm_name: String,
    pub category_name: String,
    /// Index into a fixed 9-entry category-color palette (see
    /// `algo_category`) the Slint side owns.
    pub category_index: usize,
    pub clip: bool,
    /// Real connected-line-segment geometry (see `polyline_segments`)
    /// of this block's actual processed output.
    pub waveform: CurveSegments,
    /// This algorithm's real per-slot parameter labels/raw values
    /// (0..1) and whether the manual flags that slot as one that "may
    /// increase output level radically".
    pub param_labels: [String; 3],
    pub param_values: [f32; 3],
    pub param_bold: [bool; 3],
}

/// Beads' real "grain cloud" -- see `BeadsApp::output_visual`. A
/// scrolling snapshot of the live capture buffer, with each currently
/// active grain plotted at its real read position within it, sized/
/// glowing by its real current envelope amplitude.
pub struct BeadsExtra {
    pub mode_name: String,
    pub frozen: bool,
    pub is_delay_mode: bool,
    /// Real connected-line-segment geometry (see `polyline_segments`)
    /// of the capture-buffer snapshot.
    pub waveform: CurveSegments,
    /// One entry per currently active grain: `(position fraction
    /// 0..1 across the waveform above, envelope amplitude 0..1)`.
    pub grain_dots: Vec<(f32, f32)>,
}

/// Clouds' real output monitor -- see `CloudsApp::output_visual`.
pub struct CloudsExtra {
    pub playback_mode: usize,
    pub playback_mode_name: String,
    pub frozen: bool,
    /// Real connected-line-segment geometry (see `polyline_segments`)
    /// of the granular output's scrolling monitor history.
    pub waveform: CurveSegments,
}

/// Sequencer's real on-screen 4x4 step grid for the currently-browsed
/// track -- see `SequencerApp::step_grid_visual`. Separate from the
/// *physical* pad lighting (`App::grid_led_overlay`, always active
/// regardless of which app is on screen); this is the same real
/// step/playhead/focus state, just also shown on the screen itself,
/// matching what the real embedded_graphics `draw()` does.
pub struct SequencerExtra {
    pub header: String,
    /// One entry per step (16, row-major): `(active, trimmed,
    /// is_playhead, is_focused, label)`.
    pub steps: Vec<(bool, bool, bool, bool, String)>,
    /// Which of the pattern bank's slots is currently live (see
    /// `Selection::Pattern`/`switch_pattern`) -- drives the pattern
    /// strip's highlight.
    pub current_pattern: usize,
    pub num_patterns: usize,
    pub song_mode: bool,
    /// Which song slot is currently playing -- meaningless while
    /// `song_mode` is off.
    pub song_pos: usize,
    pub song_length: usize,
    /// One entry per song slot: `(pattern index, repeat count)`.
    pub song_slots: Vec<(usize, usize)>,
    pub pad_perform: bool,
    /// Which pad a grid press most recently focused, for the pad
    /// bank's own highlight -- meaningless while `pad_perform` is off.
    pub last_touched_pad: usize,
    /// One entry per pad (16): whether it currently resolves to a
    /// real sample.
    pub pad_loaded: Vec<bool>,
}

/// One of the four dials on a play view. `knob` says which encoder
/// turns it right now (0 = neither), so the screen can badge it.
#[derive(Default, Clone)]
pub struct PlayDial {
    pub label: String,
    pub value: String,
    pub norm: f32,
    pub knob: u8,
}

/// The shared play column (play_kit.rs): what the pads, knobs and
/// expression surfaces are doing, in place of the parameter list.
#[derive(Default, Clone)]
pub struct PlayColumn {
    pub layer: String,
    pub dials: Vec<PlayDial>,
    /// The control knob 2 turns when it isn't one of the dials.
    pub knob2_extra: String,
    /// Physical pad order (row 0 on top), like `Input::grid`.
    pub pad_labels: Vec<String>,
    /// 0 off, 1 marked, 2 available, 3 lit, 4 held.
    pub pad_state: Vec<i32>,
    pub stick: [f32; 2],
    pub stick_label: String,
    pub hands: [f32; 2],
    pub hand_labels: [String; 2],
    pub line: String,
    pub status: String,
}


/// The words on the hint line. One table, so the same control is always
/// named the same way and the lines can be checked against each other.
pub mod hints {
    /// Launcher (and anything that lays apps out in a grid).
    pub const HOME: &str = "\u{2191}\u{2193}\u{25C0}\u{25B6} BROWSE  \u{B7}  SELECT OPEN  \u{B7}  F2 CATEGORY";
    /// Any ordinary parameter list.
    pub const LIST: &str = "\u{2191}\u{2193} ROW  \u{B7}  \u{25C0}\u{25B6} VALUE (HOLD: FASTER)  \u{B7}  SELECT OPEN  \u{B7}  HOLD SELECT RESET";
    /// A play app's full menu: same as a list, plus the way back.
    pub const MENU_OF_PLAY_APP: &str = "\u{2191}\u{2193} ROW  \u{B7}  \u{25C0}\u{25B6} VALUE (HOLD: FASTER)  \u{B7}  SELECT OPEN  \u{B7}  R1 PLAY VIEW";
    const CONTROLS: &str = "PAD PICKS A DIAL  \u{B7}  WIGGLE STICK OR HAND TO BIND  \u{B7}  \u{25C0}\u{25B6} TURN  \u{B7}  R1 MENU";
    const PLAYING: &str = "L1 + STICK SETS DIAL  \u{B7}  \u{25C0}\u{25B6} TURN  \u{B7}  F2 PAD LAYER  \u{B7}  R1 MENU";

    /// The play view's line, by pad layer label.
    pub fn play(layer: &str) -> &'static str {
        if layer.eq_ignore_ascii_case("CONTROLS") { CONTROLS } else { PLAYING }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn every_hint_fits_one_line_of_the_screen() {
            // 640 px at the shell's 12 px mono face is about 80 characters.
            for h in [HOME, LIST, MENU_OF_PLAY_APP, CONTROLS, PLAYING] {
                assert!(h.chars().count() <= 80, "{h:?} is {} characters", h.chars().count());
            }
        }

        #[test]
        fn the_play_view_names_the_way_to_the_menu_and_the_menu_the_way_back() {
            assert!(PLAYING.contains("R1 MENU") && CONTROLS.contains("R1 MENU"));
            assert!(MENU_OF_PLAY_APP.contains("R1 PLAY VIEW"));
        }
    }
}

/// The Controller app's live view of the connected game controller.
#[derive(Default)]
#[allow(dead_code)] // read only by the Slint renderer
pub struct ControllerExtra {
    pub name: String,
    pub connected: bool,
    pub map: String,
    /// The action being learned, or empty.
    pub learning: String,
    /// In controller_map::BUTTON_NAMES / AXIS_NAMES order.
    pub buttons: Vec<bool>,
    pub axes: Vec<f32>,
    pub status: String,
}

pub struct PlaitsExtra {
    pub engine_name: String,
    pub engine_bank: usize,
    pub engine_led: usize,
    pub analyzer_kind: u32,
    pub analyzer_name: String,
    pub spectrum: Vec<f32>,
    /// Real connected-line-segment geometry (see `polyline_segments`)
    /// -- not raw samples, so the oscilloscope draws a genuine curve
    /// instead of a bar chart.
    pub waveform: CurveSegments,
    pub peak_level: f32,
    pub rms_level: f32,
    pub pitch_name: String,
}

/// The standalone Analyzer app's own real mode-switching panel --
/// same shape as `PlaitsExtra`'s analyzer half, without the
/// engine-selection dots Plaits alone has.
pub struct AnalyzerExtra {
    pub analyzer_kind: u32,
    pub analyzer_name: String,
    pub spectrum: Vec<f32>,
    pub waveform: CurveSegments,
    pub peak_level: f32,
    pub rms_level: f32,
    pub pitch_name: String,
}

/// One curve's real connected-line-segment geometry -- see
/// `polyline_segments`.
#[derive(Default)]
pub struct CurveSegments {
    pub mid_x: Vec<f32>,
    pub mid_y: Vec<f32>,
    pub length: Vec<f32>,
    pub angle_deg: Vec<f32>,
}

/// Voltage's real 2x2 panel grid -- see `VoltageApp::voltage_panels`.
/// Pre-converted to real line-segment geometry (see
/// `polyline_segments`) rather than raw samples, so the live screen
/// draws genuine connected curves instead of a bar chart.
pub struct VoltageExtra {
    /// The loaded preset, "07 Neon Arp", with " *" once edited.
    pub preset: String,
    pub oscillator: CurveSegments,
    pub filter: CurveSegments,
    pub filter_cutoff_frac: f32,
    pub filter_swept_frac: Option<f32>,
    pub amp_env: CurveSegments,
    pub lfo: CurveSegments,
}

/// Madness's real orbiting-dots clock face -- see `MadnessApp::
/// shape_visual`. Bloom used to share this exact struct too (both
/// have the same underlying shape: outer trigger-reference dots on a
/// fixed ring, inner note dots each drifting on their own ring), but
/// now has its own dedicated `BloomVisual` for its botanical styling
/// -- see that struct's doc comment for why.
pub struct ShapeVisual {
    pub shape_index: usize,
    pub running: bool,
    /// `(x, y)` in -1..1 (center-relative, ring radius = 1), `active`,
    /// `lit` (just fired) -- one entry per outer trigger dot.
    pub outer: Vec<(f32, f32, bool, bool)>,
    /// `(x, y)` in -1..1, `lit` -- one entry per inner note dot. For
    /// Bloom, ordered innermost-to-outermost (an open spiral); for
    /// Madness, all on the same ring in sequence (see `closed`).
    pub inner: Vec<(f32, f32, bool)>,
    /// Whether `inner`'s connecting line should also close the loop
    /// (last dot back to the first) -- Madness's notes form a closed
    /// polygon on one ring; Bloom's don't (each is on its own ring).
    pub closed: bool,
}

/// Bloom's own dedicated visual -- distinct from the shared
/// `ShapeVisual` (still used by Madness) so Bloom can carry its own
/// botanical styling (a rose-to-gold petal gradient by ring, plus its
/// current Pattern name) without changing Madness's look.
pub struct BloomVisual {
    pub shape_index: usize,
    pub running: bool,
    pub pattern_name: String,
    /// `(x, y)` in -1..1 (center-relative, ring radius = 1), `active`,
    /// `lit` (just fired) -- one entry per outer trigger dot.
    pub outer: Vec<(f32, f32, bool, bool)>,
    /// `(x, y)` in -1..1, `lit`, `frac` (0 innermost/lowest note, 1
    /// outermost/highest -- the same ring fraction the petal color
    /// gradient is keyed to) -- one entry per inner note dot, ordered
    /// innermost to outermost.
    pub inner: Vec<(f32, f32, bool, f32)>,
}

/// Nebula's real gravity-well particle sim -- see
/// `NebulaApp::arena_visual`.
pub struct NebulaExtra {
    /// `(x, y)` in -1..1, `active`, `lit` (just fired) -- one entry
    /// per gravity well.
    pub wells: Vec<(f32, f32, bool, bool)>,
    /// `(x, y)` in -1..1, `brightness` 0..1 (scaled from real speed)
    /// -- one entry per live particle.
    pub particles: Vec<(f32, f32, f32)>,
}

/// Pam's real CV monitor scope for the currently-browsed channel --
/// see `PamsApp::channel_monitor`.
pub struct PamsExtra {
    pub channel_index: usize,
    /// -1..1, oldest first.
    pub history: Vec<f32>,
    pub value: f32,
}

/// Singularity's real chaotic orbiting point -- see
/// `SingularityApp::orbit_visual`.
pub struct OrbitExtra {
    pub x: f32,
    pub y: f32,
    pub value: f32,
}

/// Prism's real tap-time/rate map -- see `PrismApp::tap_map`.
pub struct PrismExtra {
    /// `(x_frac, height_frac)` per tap/tick, both 0..1.
    pub ticks: Vec<(f32, f32)>,
    pub caption: String,
}

/// Cascade's real FM operator-routing graph -- see
/// `CascadeApp::operator_graph`.
pub struct CascadeExtra {
    pub algorithm_name: String,
    /// Whether each operator (fixed at 6, see `NUM_OPS`) is a carrier
    /// (summed into the audible output) or a pure modulator.
    pub carriers: Vec<bool>,
    /// `(from_op, to_op)` pairs -- a modulator routed into another
    /// operator, same connections the real operator graph draws.
    pub connections: Vec<(usize, usize)>,
    pub feedback_op: usize,
    /// Real straight-line segment geometry for each entry in
    /// `connections`, one segment per connection (see
    /// `polyline_segments`) -- computed here in Rust, against the same
    /// operator-box layout the Slint side draws, so the line always
    /// hits true box centers instead of an L-shaped bounding-box
    /// outline.
    pub connection_lines: CurveSegments,
}

/// Center of operator box `op` in Cascade's 3-column x 2-row grid, as
/// laid out on the Slint side (origin 20px/10px, 70x26px boxes on a
/// 90x60px cell grid). Operators are arranged in a serpentine
/// (boustrophedon) order -- 0,1,2 left-to-right on top, then 5,4,3
/// left-to-right on bottom -- rather than a plain row-major grid, so
/// that a chain of consecutive operator indices (the common case,
/// e.g. the "Stack" algorithm's 5->4->3->2->1->0) always connects
/// adjacent boxes instead of jumping diagonally across the grid.
fn cascade_op_center(op: usize) -> (f32, f32) {
    let row = (op / 3) as f32;
    let col = if op < 3 { (op % 3) as f32 } else { (5 - op) as f32 };
    (20.0 + col * 90.0 + 35.0, 10.0 + row * 60.0 + 13.0)
}

pub fn cascade_connection_lines(connections: &[(usize, usize)]) -> CurveSegments {
    let mut mid_x = Vec::with_capacity(connections.len());
    let mut mid_y = Vec::with_capacity(connections.len());
    let mut length = Vec::with_capacity(connections.len());
    let mut angle_deg = Vec::with_capacity(connections.len());
    for &(from, to) in connections {
        let (x0, y0) = cascade_op_center(from);
        let (x1, y1) = cascade_op_center(to);
        let dx = x1 - x0;
        let dy = y1 - y0;
        mid_x.push((x0 + x1) / 2.0);
        mid_y.push((y0 + y1) / 2.0);
        length.push((dx * dx + dy * dy).sqrt().max(0.5));
        angle_deg.push(dy.atan2(dx).to_degrees());
    }
    CurveSegments { mid_x, mid_y, length, angle_deg }
}

/// Bounded live telemetry for the independently installed collection apps.
pub struct CollectionExtra {
    pub visual_lines:Vec<f32>,
    pub terrain:Vec<f32>,
    pub controls:Vec<f32>,pub buffer:Vec<f32>,pub tracks:Vec<f32>,pub voice_phases:Vec<f32>,pub notes:Vec<f32>,pub genes:Vec<f32>,
    pub files:Vec<String>,pub source_names:Vec<String>,pub reference:String,pub instrument:String,pub frozen:bool,pub alternate:bool,
    pub kind: i32, pub status: String, pub wave: Vec<f32>, pub spectrum: Vec<f32>,
    pub levels: Vec<f32>, pub peak: f32, pub rms: f32, pub phase: f32,
    pub duration: f32, pub step: i32, pub pads: Vec<bool>, pub recording: bool,
    pub playing: bool, pub source: String, pub hint: String,
}

#[derive(Clone,Debug)]
pub struct PortalExtra {pub sources:Vec<String>,pub targets:Vec<String>,pub amounts:Vec<f32>,pub enabled:Vec<bool>,pub levels:Vec<f32>,pub selected:i32,pub active:bool,pub status:String}

pub struct VectorFilterExtra {pub xyz:Vec<f32>,pub wave:Vec<f32>,pub source:String,pub mode:String,pub enabled:bool}

pub struct ForgeExtra{pub wave:Vec<f32>,pub starts:Vec<f32>,pub ends:Vec<f32>,pub labels:Vec<String>,pub levels:Vec<f32>,pub name:String,pub status:String,pub mode:String,pub source:String,pub selected:i32,pub busy:bool,pub recording:bool,pub duration:f32}

/// Atlas's panel in the Slint GUI -- see `AtlasPanel`.
#[allow(dead_code)] // Slint GUI only
pub struct AtlasExtra {
    pub name: String,
    pub category: String,
    pub description: String,
    pub index: String,
    pub macro_names: Vec<String>,
    pub macro_values: Vec<f32>,
    pub macro_used: Vec<bool>,
    pub morph: f32,
    pub states: usize,
    pub morph_label: String,
    /// 1 = spectrum bars, 0 = scope trace.
    pub spectrum_view: bool,
    pub spectrum: Vec<f32>,
    pub scope: Vec<f32>,
    pub voices: String,
    pub load: f32,
    pub depth: String,
    pub status: String,
}

/// The shared services an app is built from: buses, settings, devices.
///
/// Type-erased on purpose, so a new service (a new bus, a new device
/// manager) can be added without touching any app's constructor and an
/// app asks only for what it uses -- `ctx.get::<ModBus>()` for a service
/// there is one of, `ctx.named::<AtomicF32>("sensitivity")` for shared
/// values that share a type. See `registry::Registry` for what the OS
/// provides, and each app's `create` for what it takes.
#[derive(Default, Clone)]
pub struct AppContext {
    typed: std::collections::HashMap<std::any::TypeId, std::sync::Arc<dyn std::any::Any + Send + Sync>>,
    named: std::collections::HashMap<String, std::sync::Arc<dyn std::any::Any + Send + Sync>>,
}

#[allow(dead_code)] // the preview binaries use a subset
impl AppContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Provide the one instance of a service type.
    pub fn provide<T: std::any::Any + Send + Sync>(&mut self, service: std::sync::Arc<T>) -> &mut Self {
        self.typed.insert(std::any::TypeId::of::<T>(), service);
        self
    }

    /// Provide a value by name (for shared values whose type isn't unique).
    pub fn provide_named<T: std::any::Any + Send + Sync>(&mut self, name: &str, value: std::sync::Arc<T>) -> &mut Self {
        self.named.insert(name.to_string(), value);
        self
    }

    pub fn try_get<T: std::any::Any + Send + Sync>(&self) -> Option<std::sync::Arc<T>> {
        self.typed.get(&std::any::TypeId::of::<T>()).and_then(|s| std::sync::Arc::clone(s).downcast::<T>().ok())
    }

    /// A service the OS provides. Missing services are a wiring bug in the
    /// host, never in an app, so this names exactly what's missing.
    pub fn get<T: std::any::Any + Send + Sync>(&self) -> std::sync::Arc<T> {
        self.try_get().unwrap_or_else(|| panic!("AppContext: no {} provided", std::any::type_name::<T>()))
    }

    pub fn try_named<T: std::any::Any + Send + Sync>(&self, name: &str) -> Option<std::sync::Arc<T>> {
        self.named.get(name).and_then(|s| std::sync::Arc::clone(s).downcast::<T>().ok())
    }

    pub fn named<T: std::any::Any + Send + Sync>(&self, name: &str) -> std::sync::Arc<T> {
        self.try_named(name).unwrap_or_else(|| panic!("AppContext: no {} named {name:?} provided", std::any::type_name::<T>()))
    }
}

/// How the registry builds an app: the shared services, and the manifest
/// id (one module can implement several apps, e.g. the Collection).
pub type AppFactory = fn(&AppContext, &str) -> Box<dyn App>;
