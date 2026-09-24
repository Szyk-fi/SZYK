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
    /// Discrete D-pad/joystick row steps, independent of encoder sensitivity.
    pub navigation_steps: i32,
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
            navigation_steps: 0,
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
        let _ = visible;
        let rows = self.slint_rows();
        let selected = self.slint_selected();
        (rows, selected, false, false)
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
    Analyzer(AnalyzerExtra),
    Voltage(VoltageExtra),
    Cascade(CascadeExtra),
    Shape(ShapeVisual),
    Bloom(BloomVisual),
    Nebula(NebulaExtra),
    Tape(TapeExtra),
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

/// Settings' live color-wheel state -- see `SettingsApp::slint_extra`.
pub struct ThemeExtra {
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

/// Tape's real 4-track lane view -- see `TapeApp::track_lanes`.
pub struct TapeExtra {
    /// `(status_text, fill_kind, playhead_frac)` per track --
    /// `fill_kind`: 0 = empty, 1 = has content, 2 = recording.
    pub tracks: Vec<(String, u8, f32)>,
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
