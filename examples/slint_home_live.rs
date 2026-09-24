//! The real, unified device -- not one isolated app in its own
//! process, but the same `Registry`/manifest-driven app list, shared
//! `ModBus`/`AudioBus`/`MixerBus`/audio engine, and F1(Home)/F4(Mixer)
//! navigation `main.rs`/`os.rs`/`registry.rs` use for the real
//! embedded_graphics firmware -- just rendered through Slint instead.
//! Every app's audio processor is registered into the one shared
//! engine at startup and keeps running regardless of which app is on
//! screen, exactly like the real firmware; only the *active* app
//! receives `tick`/`Input`.
//!
//! This is the actual assembly the individual `slint_*_live.rs`
//! examples were scoping work for -- each of those proved one app's
//! screen could drive real logic/audio in isolation; this wires all
//! of them together into the one device those screens were always
//! meant to live inside. Every app currently renders through the same
//! generic `ParamListColumn` (real windowed rows, real knob nav/edit/
//! press, real pad triggering, Mixer's real fader bars) rather than
//! each app's own bespoke visualizer (Plaits' engine dots/analyzer,
//! Sequencer's step grid, Cascade's operator graph, ...) -- those are
//! still real per-app screens in their own `slint_*_live.rs` file;
//! folding each one's bespoke visual into this shared shell is the
//! next pass, once this connected shell itself is right.
//!
//! Run it:
//!     cargo run --example slint_home_live
//!
//! Controls: left knob scrolls the home list (or the active app's real
//! menu); knob press selects/expands; right knob edits a leaf's value;
//! pads trigger/hold whatever the active app reads them as (a note,
//! a step toggle, ...); F1 returns to the home list; F3
//! starts/stops the active app's transport, if it has one; F4 jumps
//! straight to the Mixer from anywhere. A real MIDI controller (if
//! one's plugged in) drives the same controls alongside the mouse --
//! see live_midi.rs.

#[path = "../src/app.rs"]
mod app;
#[path = "../src/arpeggiator.rs"]
mod arpeggiator;
#[path = "../src/audio.rs"]
mod audio;
#[path = "../src/audio_bus.rs"]
mod audio_bus;
#[path = "../src/audio_devices.rs"]
mod audio_devices;
#[path = "../src/clouds_ffi.rs"]
mod clouds_ffi;
#[path = "../src/controller.rs"]
mod controller;
#[path = "../src/display.rs"]
mod display;
#[path = "../src/led_output.rs"]
mod led_output;
#[path = "../src/manifest.rs"]
mod manifest;
#[path = "../src/midi_map.rs"]
mod midi_map;
#[path = "../src/mixer_bus.rs"]
mod mixer_bus;
#[path = "../src/modbus.rs"]
mod modbus;
#[path = "../src/paramlist.rs"]
mod paramlist;
#[path = "../src/plaits_ffi.rs"]
mod plaits_ffi;
#[path = "../src/registry.rs"]
mod registry;
#[path = "../src/spleen_fonts.rs"]
mod spleen_fonts;
#[path = "../src/startup_logo.rs"]
mod startup_logo;
#[path = "../src/theme.rs"]
mod theme;
#[path = "../src/util.rs"]
mod util;
#[path = "slint_common/live_midi.rs"]
mod live_midi;

// Every real app, flat at this example's own crate root (no real
// `examples/apps/` directory exists -- see slint_mixer_live.rs's doc
// comment for the fuller "why `#[path]`, why flat" explanation).
// `registry.rs` (and several apps themselves, e.g. Bloom/Madness/
// Nebula/Pam's pulling in Plaits' engine tables) refer to these by
// the real crate's `crate::apps::X` path, so `pub mod apps` below
// re-exports the same flat modules under that path too.
#[path = "../src/apps/analyzer.rs"]
pub mod analyzer;
#[path = "../src/apps/beads.rs"]
pub mod beads;
#[path = "../src/apps/black_hole.rs"]
pub mod black_hole;
#[path = "../src/apps/bloom.rs"]
pub mod bloom;
#[path = "../src/apps/cascade.rs"]
pub mod cascade;
#[path = "../src/apps/clouds.rs"]
pub mod clouds;
#[path = "../src/apps/cv_out.rs"]
pub mod cv_out;
#[path = "../src/apps/morph.rs"]
pub mod morph;
#[path = "../src/apps/madness.rs"]
pub mod madness;
#[path = "../src/apps/magnito.rs"]
pub mod magnito;
#[path = "../src/apps/midi_learn.rs"]
pub mod midi_learn;
#[path = "../src/apps/mixer.rs"]
pub mod mixer;
#[path = "../src/apps/natural_gate.rs"]
pub mod natural_gate;
#[path = "../src/apps/nautilus.rs"]
pub mod nautilus;
#[path = "../src/apps/nebula.rs"]
pub mod nebula;
#[path = "../src/apps/pams.rs"]
pub mod pams;
#[path = "../src/apps/plaits.rs"]
pub mod plaits;
#[path = "../src/apps/plaits_layout.rs"]
pub mod plaits_layout;
#[path = "../src/apps/prism.rs"]
pub mod prism;
#[path = "../src/apps/queen_of_pentacles.rs"]
pub mod queen_of_pentacles;
#[path = "../src/apps/rainmaker.rs"]
pub mod rainmaker;
#[path = "../src/apps/sample_drum.rs"]
pub mod sample_drum;
#[path = "../src/apps/sequencer.rs"]
pub mod sequencer;
#[path = "../src/apps/settings.rs"]
pub mod settings;
#[path = "../src/apps/singularity.rs"]
pub mod singularity;
#[path = "../src/apps/starlab.rs"]
pub mod starlab;
#[path = "../src/apps/warps.rs"]
pub mod warps;
#[path = "../src/apps/synth.rs"]
pub mod synth;
#[path = "../src/apps/tape.rs"]
pub mod tape;
#[path = "../src/apps/tonestack.rs"]
pub mod tonestack;
#[path = "../src/apps/turing_machine.rs"]
pub mod turing_machine;
#[path = "../src/apps/voltage.rs"]
pub mod voltage;
#[path = "../src/apps/visualizer.rs"]
pub mod visualizer;
mod apps {
    pub use super::{
        analyzer, beads, black_hole, bloom, cascade, clouds, cv_out, madness, magnito, midi_learn, mixer, morph, natural_gate, nautilus,
        nebula, pams, plaits, plaits_layout, prism, queen_of_pentacles, rainmaker, sample_drum, sequencer, settings, singularity,
        starlab, synth, tape, tonestack, turing_machine, visualizer, voltage, warps,
    };
}

/// Stand-in for the eventual STM32Cube.AI inference call -- same no-op
/// `main.rs` defines at its own crate root.
pub fn run_inference(block: &mut [f32]) {
    let _ = block;
}

use app::{App, Input};
use audio_bus::AudioBus;
use audio_devices::AudioHost;
use controller::ControllerState;
use mixer_bus::MixerBus;
use modbus::ModBus;
use paramlist::ParamList;
use registry::Registry;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use util::AtomicF32;

const APPS_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/apps");
/// How many rows fit on screen at once -- both the home app list and
/// every real app's own menu share this window size.
const VISIBLE_ROWS: usize = 10;
// The home/launcher list's own window size -- fewer rows than
// `VISIBLE_ROWS` since its larger text takes more vertical space per
// row (see `ParamListColumn`'s `big-text`).
// 5, not 6 -- 6 rows at 44px each (264px) plus both "more" indicators
// (22px each when scrolled to the middle of a long list) came to
// 308px, overflowing the screen's 284px content area and visually
// colliding with the last row. 5 rows (220px) leaves real headroom.
const HOME_VISIBLE_ROWS: usize = 5;

slint::slint! {
    import { DeviceFrame, BarSegment, ParamListColumn } from "slint_common/device_frame.slint";

    import { InstrumentLabel, InstrumentHeading, ScopeSurface, SignalTrace, ValueTrack, InstrumentPanel, VectorSegment } from "slint_common/instrument_widgets.slint";

    import { BloomPanel } from "slint_common/bloom_panel.slint";

    export component LiveHomeScreen inherits DeviceFrame {
        // Boot sequence (SZYK -> MX1, see startup_logo.rs): blanks the
        // title/breadcrumb/bottom bar entirely while a splash stage is
        // showing, same bare-screen look `os.rs`'s own `draw_splash`
        // has on the real device -- only the centered logo, nothing
        // else. See the full-screen overlay at the end of the body
        // below for the logo itself.
        in-out property <bool> splash-active: true;
        in-out property <bool> preview-render: false;
        in-out property <image> splash-image;
        in-out property <bool> splash-is-szyk: true;
        app-name: root.splash-active ? "" : (root.on-home ? "PORTAMAX" : root.active-app-name);
        in property <bool> audio-connected: true;
        breadcrumb: root.splash-active ? "" : ((!root.audio-connected && !root.preview-render ? "AUDIO UNAVAILABLE / SETTINGS" : root.on-home ? "SELECT AN APP" : (root.midi-target-label != "" ? "PADS → " + root.midi-target-label : root.preview-render ? "OFFLINE / SOFTWARE RENDER" : root.audio-connected ? "LIVE -- REAL AUDIO" : "AUDIO UNAVAILABLE / SETTINGS")) + (root.cpu-visible ? "   CPU " + Math.round(root.cpu-load-pct) + "%" : ""));
        // F1 doubles as a shortcut to Settings while already on the
        // home screen (Home itself would be a no-op there); F3 only
        // shows Start/Stop when the active app actually has a
        // transport (`transport-label` is empty for one that
        // doesn't -- see `App::running`), instead of showing a
        // Start/Stop button on apps with nothing to start.
        bar: root.splash-active ? [
            { label: "", active: false },
            { label: "", active: false },
            { label: "", active: false },
            { label: "", active: false },
        ] : [
            { label: root.on-home ? (root.has-settings ? "F1  SETTINGS" : "F1  —") : "F1  HOME", active: false },
            {
                label: root.grid-mode-label != "" ? (root.grid-mode-label == "STEP" ? "F2  PAD MODE" : "F2  STEP MODE") : (root.midi-target-label != "" ? "F2  UNLOCK" : root.pad-lock-available && !root.on-home ? "F2  PAD LOCK" : "F2  —"),
                active: root.grid-mode-label != "" ? root.grid-mode-label == "PAD" : root.midi-target-label != "",
            },
            { label: root.on-home || root.transport-action == "" ? "F3  —" : "F3  " + root.transport-action, active: !root.on-home && root.transport-label == "RUNNING" },
            { label: root.has-mixer ? "F4  MIXER" : "F4  —", active: false },
        ];
        // Both driven live from `theme::ThemeColor` every tick (see
        // Settings' color wheel, `active-kind == 25` below) --
        // recolors every `root.accent`/`root.screen-bg` reference in
        // this whole file at once, not just Settings' own panel.
        accent: root.live-accent;
        screen-bg: root.live-bg;
        screen-ink: root.live-ink;
        in-out property <color> live-accent: #5CF07A;
        in-out property <color> live-bg: #0B100C;
        // The active app's own text-ink color (white by default,
        // dark for a light-bodied app) -- see `ParamListColumn.ink`.
        // Home/launcher and Settings' own wheel stay plain white,
        // same as before; only an app with its own fixed palette
        // (see `app_palette` in Rust) overrides this.
        in-out property <color> live-ink: #ffffff;

        in-out property <bool> on-home: true;
        in-out property <string> active-app-name: "";
        in-out property <string> transport-label: "";
        in-out property <string> transport-action: "";
        in-out property <bool> pad-lock-available: false;
        in-out property <bool> has-settings: true;
        in-out property <bool> has-mixer: true;
        function-enabled: [!root.splash-active && (!root.on-home || root.has-settings), !root.splash-active && (root.grid-mode-label != "" || root.pad-lock-available || root.midi-target-label != ""), !root.splash-active && !root.on-home && root.transport-action != "", !root.splash-active && root.has-mixer];
        // Which app pads/notes currently go to -- "" when nothing's
        // pinned (pads follow the active screen), "PINNED" while
        // you're looking at the pinned app's own screen, or that
        // app's name in caps while browsing a different one. See F2.
        in-out property <string> midi-target-label: "";
        // Non-"" while the active app claims F2 for its own grid mode
        // (see `App::grid_mode_label`) instead of the default Pin
        // behavior above -- the Sequencer's Step/Pad toggle today.
        in-out property <string> grid-mode-label: "";

        in-out property <[string]> home-names: [];
        in-out property <int> home-selected: 0;
        in-out property <bool> home-more-above: false;
        in-out property <bool> home-more-below: false;

        in-out property <[string]> row-names: [];
        in-out property <[string]> row-values: [];
        in-out property <[bool]> row-is-group: [];
        in-out property <[float]> row-levels: [];
        in-out property <int> selected-row: 0;
        in-out property <bool> more-above: false;
        in-out property <bool> more-below: false;

        in-out property <int> navigation-delta: 0;
        in-out property <float> live-stick-x <=> self.stick-x;
        in-out property <float> live-stick-y <=> self.stick-y;
        navigation-pressed(direction) => {
            if direction == 0 { root.navigation-delta -= 1; }
            if direction == 2 { root.navigation-delta += 1; }
            if direction == 1 { root.knob2-delta += 1; }
            if direction == 3 { root.knob2-delta -= 1; }
            if direction == 4 { root.knob1-clicked(); }
        }
        shoulder-pressed(side) => {
            if side == 0 { root.f-clicked(0); }
            else { root.knob1-clicked(); }
        }
        in-out property <float> knob1-delta: 0;
        in-out property <float> knob2-delta: 0;

        // Which bespoke visual (if any) the active app gets, beyond
        // the generic list every app already has -- 0 = generic list
        // only, full width (this covers Sequencer too: its own real
        // visual is the physical pad matrix lighting via
        // `live-pad-colors` below, not a side panel); 1 = Plaits
        // (engine dots + analyzer); 2 = Voltage (2x2 panel grid); 3 =
        // Analyzer (its own mode-switching panel); 4 = Cascade
        // (operator routing graph).
        in-out property <int> active-kind: 0;

        // --- Plaits-specific state (active-kind == 1) ---
        in-out property <string> plaits-engine-name: "";
        in-out property <int> plaits-engine-bank: 0;
        in-out property <int> plaits-engine-led: 0;
        property <[color]> plaits-bank-colors: [#b33168, #087c77, #8b5d17];
        in-out property <int> plaits-analyzer-kind: 0;
        in-out property <string> plaits-analyzer-name: "Spectrum";
        in-out property <[float]> plaits-spectrum: [];
        // Real connected-line-segment geometry (see
        // `polyline_segments`), not raw samples -- 4 parallel arrays,
        // one entry per segment: center x/y, length, rotation.
        in-out property <[float]> plaits-waveform-mid-x: [];
        in-out property <[float]> plaits-waveform-mid-y: [];
        in-out property <[float]> plaits-waveform-length: [];
        in-out property <[float]> plaits-waveform-angle: [];
        in-out property <float> plaits-peak-level: 0;
        in-out property <float> plaits-rms-level: 0;
        in-out property <string> plaits-pitch-name: "--";

        // --- Analyzer-specific state (active-kind == 3) -- same
        // shape as Plaits' own analyzer half, no engine dots. ---
        in-out property <int> analyzer-kind: 0;
        in-out property <string> analyzer-name: "Spectrum";
        in-out property <[float]> analyzer-spectrum: [];
        in-out property <[float]> analyzer-waveform-mid-x: [];
        in-out property <[float]> analyzer-waveform-mid-y: [];
        in-out property <[float]> analyzer-waveform-length: [];
        in-out property <[float]> analyzer-waveform-angle: [];
        in-out property <float> analyzer-peak-level: 0;
        in-out property <float> analyzer-rms-level: 0;
        in-out property <string> analyzer-pitch-name: "--";

        // --- Voltage-specific state (active-kind == 2): the real
        // 2x2 Oscillators/Filter/Amp-Envelope/LFO panel grid, each as
        // real connected-line-segment geometry (see
        // `polyline_segments`) -- 4 parallel arrays per curve.
        in-out property <[float]> voltage-oscillator-mid-x: [];
        in-out property <[float]> voltage-oscillator-mid-y: [];
        in-out property <[float]> voltage-oscillator-length: [];
        in-out property <[float]> voltage-oscillator-angle: [];
        in-out property <[float]> voltage-filter-mid-x: [];
        in-out property <[float]> voltage-filter-mid-y: [];
        in-out property <[float]> voltage-filter-length: [];
        in-out property <[float]> voltage-filter-angle: [];
        in-out property <float> voltage-filter-cutoff-frac: 0;
        in-out property <bool> voltage-filter-has-swept: false;
        in-out property <float> voltage-filter-swept-frac: 0;
        in-out property <[float]> voltage-amp-env-mid-x: [];
        in-out property <[float]> voltage-amp-env-mid-y: [];
        in-out property <[float]> voltage-amp-env-length: [];
        in-out property <[float]> voltage-amp-env-angle: [];
        in-out property <[float]> voltage-lfo-mid-x: [];
        in-out property <[float]> voltage-lfo-mid-y: [];
        in-out property <[float]> voltage-lfo-length: [];
        in-out property <[float]> voltage-lfo-angle: [];

        // --- Cascade-specific state (active-kind == 4): the real FM
        // operator-routing graph. ---
        in-out property <string> cascade-algorithm-name: "";
        in-out property <[bool]> cascade-carriers: [];
        in-out property <[float]> cascade-connection-mid-x: [];
        in-out property <[float]> cascade-connection-mid-y: [];
        in-out property <[float]> cascade-connection-length: [];
        in-out property <[float]> cascade-connection-angle: [];
        in-out property <int> cascade-feedback-op: -1;

        // --- Bloom-specific state (active-kind == 29): its own
        // dedicated botanical panel (see `BloomVisual`/
        // `BloomPalette`) -- Madness keeps the shared "Shape" look
        // below since it wasn't asked to change.
        in-out property <int> bloom-shape-index: 0;
        in-out property <bool> bloom-running: false;
        in-out property <string> bloom-pattern-name: "Spiral";
        in-out property <[float]> bloom-outer-x: [];
        in-out property <[float]> bloom-outer-y: [];
        in-out property <[bool]> bloom-outer-active: [];
        in-out property <[bool]> bloom-outer-lit: [];
        in-out property <[float]> bloom-outer-flash-angle: [];
        in-out property <[bool]> bloom-outer-flash-active: [];
        in-out property <[float]> bloom-inner-x: [];
        in-out property <[float]> bloom-inner-y: [];
        in-out property <[bool]> bloom-inner-lit: [];
        in-out property <[float]> bloom-inner-frac: [];
        in-out property <[float]> bloom-line-mid-x: [];
        in-out property <[float]> bloom-line-mid-y: [];
        in-out property <[float]> bloom-line-length: [];
        in-out property <[float]> bloom-line-angle: [];

        // --- Madness-specific state (active-kind == 5): the real
        // orbiting-dots clock face, still on the shared "Shape" look
        // (see `ShapeVisual`). Flattened into parallel arrays (Slint
        // has no array-of-struct property type).
        in-out property <[float]> shape-outer-x: [];
        in-out property <[float]> shape-outer-y: [];
        in-out property <[bool]> shape-outer-active: [];
        in-out property <[bool]> shape-outer-lit: [];
        in-out property <[float]> shape-inner-x: [];
        in-out property <[float]> shape-inner-y: [];
        in-out property <[bool]> shape-inner-lit: [];
        in-out property <bool> shape-closed: false;
        in-out property <bool> shape-running: false;
        // The connecting web (Harmony-Bloom-style glow lines) --
        // derived once per frame from the same `shape-inner-x/y`
        // points above (see the `SlintExtra::Shape` match arm), same
        // "fraction of radius, angle in degrees" convention every
        // other rotated line-segment visual in this file uses
        // (Magnito's loop trace, Voltage's curves, ...).
        in-out property <[float]> shape-inner-line-mid-x: [];
        in-out property <[float]> shape-inner-line-mid-y: [];
        in-out property <[float]> shape-inner-line-length: [];
        in-out property <[float]> shape-inner-line-angle: [];
        // One flash line per *lit* outer dot, center to that dot --
        // same fraction-of-radius convention, length always 1.0 since
        // outer dots sit exactly on the boundary ring.
        in-out property <[float]> shape-outer-flash-angle: [];
        in-out property <[bool]> shape-outer-flash-active: [];

        // --- Nebula-specific state (active-kind == 6): the real
        // gravity-well particle arena. ---
        in-out property <[float]> nebula-well-x: [];
        in-out property <[float]> nebula-well-y: [];
        in-out property <[bool]> nebula-well-active: [];
        in-out property <[bool]> nebula-well-lit: [];
        in-out property <[float]> nebula-particle-x: [];
        in-out property <[float]> nebula-particle-y: [];
        in-out property <[float]> nebula-particle-brightness: [];

        // --- Tape-specific state (active-kind == 7): 4 real track
        // lanes + playhead. ---
        in-out property <[string]> tape-track-status: [];
        in-out property <[int]> tape-track-kind: [];
        in-out property <float> tape-playhead-frac: 0;

        // --- Pam's-specific state (active-kind == 8): the real CV
        // monitor scope for the browsed channel. ---
        in-out property <int> pams-channel-index: 0;
        in-out property <[float]> pams-history: [];
        in-out property <float> pams-value: 0;

        // --- Singularity-specific state (active-kind == 9): the real
        // chaotic orbiting point. ---
        in-out property <float> orbit-x: 0;
        in-out property <float> orbit-y: 0;
        in-out property <float> orbit-value: 0;

        // --- Prism-specific state (active-kind == 10): the real
        // tap-time/rate map. ---
        in-out property <[float]> prism-tick-x: [];
        in-out property <[float]> prism-tick-height: [];
        in-out property <string> prism-caption: "";

        // --- Nautilus-specific state (active-kind == 11): the real
        // 8-line delay network + mode/Chroma/Sonar state. ---
        property <[color]> nautilus-mode-colors: [
            #4090E0, #B060E0, #30C0A0, #E0C030, #FF4D4D, #E060A0,
        ];
        in-out property <string> nautilus-delay-mode-name: "";
        in-out property <int> nautilus-delay-mode-index: 0;
        in-out property <string> nautilus-feedback-mode-name: "";
        in-out property <int> nautilus-feedback-mode-index: 0;
        in-out property <string> nautilus-chroma-name: "";
        in-out property <int> nautilus-chroma-index: 0;
        in-out property <bool> nautilus-frozen: false;
        in-out property <float> nautilus-feedback-amount: 0;
        in-out property <[float]> nautilus-line-level: [];
        in-out property <[bool]> nautilus-line-pulse: [];
        in-out property <[bool]> nautilus-line-active: [];

        // --- Sequencer-specific state (active-kind == 12): the real
        // on-screen 4x4 step grid. ---
        in-out property <string> sequencer-header: "";
        in-out property <[bool]> sequencer-step-active: [];
        in-out property <[bool]> sequencer-step-trimmed: [];
        in-out property <[bool]> sequencer-step-playhead: [];
        in-out property <[bool]> sequencer-step-focused: [];
        in-out property <[string]> sequencer-step-label: [];
        // Pattern bank strip + Song arrangement strip -- always shown
        // above the step grid/pad bank, since which pattern is live
        // (and where a running Song is) matters regardless of which
        // track/pad you're currently focused on.
        in-out property <int> sequencer-current-pattern: 0;
        in-out property <int> sequencer-num-patterns: 8;
        in-out property <bool> sequencer-song-mode: false;
        in-out property <int> sequencer-song-pos: 0;
        in-out property <int> sequencer-song-length: 8;
        in-out property <[int]> sequencer-song-slot-pattern: [];
        in-out property <[int]> sequencer-song-slot-repeats: [];
        // Pad Perform's own 4x4 bank -- replaces the step grid above
        // while active (see the `sequencer-pad-perform` gate below).
        in-out property <bool> sequencer-pad-perform: false;
        in-out property <int> sequencer-last-touched-pad: 0;
        in-out property <[bool]> sequencer-pad-loaded: [];

        // --- Clouds-specific state (active-kind == 13): the real
        // playback-mode indicator + granular output monitor. ---
        property <[color]> clouds-mode-colors: [#5CF07A, #FF4D4D, #E0C030, #4090E0];
        in-out property <int> clouds-mode: 0;
        in-out property <string> clouds-mode-name: "";
        in-out property <bool> clouds-frozen: false;
        in-out property <[float]> clouds-waveform-mid-x: [];
        in-out property <[float]> clouds-waveform-mid-y: [];
        in-out property <[float]> clouds-waveform-length: [];
        in-out property <[float]> clouds-waveform-angle: [];

        // --- Beads-specific state (active-kind == 14): the real
        // grain cloud -- a scrolling capture-buffer snapshot with
        // each currently active grain plotted at its real read
        // position, sized/glowing by its real envelope amplitude. ---
        in-out property <string> beads-mode-name: "";
        in-out property <bool> beads-frozen: false;
        in-out property <bool> beads-delay-mode: false;
        in-out property <[float]> beads-waveform-mid-x: [];
        in-out property <[float]> beads-waveform-mid-y: [];
        in-out property <[float]> beads-waveform-length: [];
        in-out property <[float]> beads-waveform-angle: [];
        in-out property <[float]> beads-grain-x: [];
        in-out property <[float]> beads-grain-brightness: [];

        // --- Black Hole-specific state (active-kind == 15): the real
        // category badge + oscilloscope + per-algorithm parameter
        // meters. ---
        property <[color]> black-hole-category-colors: [
            #4090E0, #B060E0, #30C0A0, #E0C030, #FF4D4D, #E060A0, #FF9040, #5CF07A, #C0C0C0,
        ];
        in-out property <string> black-hole-algorithm-name: "";
        in-out property <string> black-hole-category-name: "";
        in-out property <int> black-hole-category-index: 0;
        in-out property <bool> black-hole-clip: false;
        in-out property <[float]> black-hole-waveform-mid-x: [];
        in-out property <[float]> black-hole-waveform-mid-y: [];
        in-out property <[float]> black-hole-waveform-length: [];
        in-out property <[float]> black-hole-waveform-angle: [];
        in-out property <[string]> black-hole-param-labels: [];
        in-out property <[float]> black-hole-param-values: [];
        in-out property <[bool]> black-hole-param-bold: [];

        // --- Queen of Pentacles-specific state (active-kind == 16):
        // the real chaotic-map trajectory + live CV/Gate/Delta output
        // state. ---
        in-out property <string> qop-map-name: "";
        in-out property <float> qop-r: 0;
        in-out property <bool> qop-frozen: false;
        in-out property <[float]> qop-trajectory-mid-x: [];
        in-out property <[float]> qop-trajectory-mid-y: [];
        in-out property <[float]> qop-trajectory-length: [];
        in-out property <[float]> qop-trajectory-angle: [];
        in-out property <float> qop-cv: 0;
        in-out property <float> qop-smooth-cv: 0;
        in-out property <bool> qop-gate: false;
        in-out property <string> qop-gate-mode-name: "";
        in-out property <float> qop-delta: 0;
        in-out property <float> qop-threshold: 0;

        // --- Natural Gate-specific state (active-kind == 17): the
        // real per-channel envelope trace + gate-open/hit state. ---
        in-out property <[float]> ng-ch1-trace-mid-x: [];
        in-out property <[float]> ng-ch1-trace-mid-y: [];
        in-out property <[float]> ng-ch1-trace-length: [];
        in-out property <[float]> ng-ch1-trace-angle: [];
        in-out property <float> ng-ch1-now: 0;
        in-out property <float> ng-ch1-open: 0;
        in-out property <bool> ng-ch1-hit: false;
        in-out property <[float]> ng-ch2-trace-mid-x: [];
        in-out property <[float]> ng-ch2-trace-mid-y: [];
        in-out property <[float]> ng-ch2-trace-length: [];
        in-out property <[float]> ng-ch2-trace-angle: [];
        in-out property <float> ng-ch2-now: 0;
        in-out property <float> ng-ch2-open: 0;
        in-out property <bool> ng-ch2-hit: false;

        // --- Turing Machine-specific state (active-kind == 18): the
        // real 16-bit shift register + Locks-derived lock state. ---
        in-out property <[bool]> tm-bits: [];
        in-out property <int> tm-active-len: 16;
        in-out property <int> tm-write-index: 0;
        in-out property <bool> tm-pulse: false;
        in-out property <float> tm-cv: 0;
        in-out property <float> tm-keep-probability: 0;
        in-out property <bool> tm-inverted-feedback: false;
        in-out property <bool> tm-double-locked: false;

        // --- Rainmaker-specific state (active-kind == 19): the real
        // 16-tap timing map. ---
        in-out property <string> rm-groove-name: "";
        in-out property <float> rm-groove-amount: 0;
        in-out property <string> rm-grid-label: "";
        in-out property <float> rm-beats-spanned: 1;
        in-out property <[float]> rm-tap-time: [];
        in-out property <[float]> rm-tap-level: [];
        in-out property <[float]> rm-tap-pan: [];
        in-out property <[bool]> rm-tap-muted: [];

        // --- StarLab-specific state (active-kind == 20): the real
        // oscilloscope + texture/LFO/tank-energy state. ---
        property <[color]> starlab-texture-colors: [#4090E0, #B060E0, #30C0A0];
        in-out property <string> starlab-texture-name: "";
        in-out property <int> starlab-texture-index: 0;
        in-out property <bool> starlab-infinite: false;
        in-out property <bool> starlab-karplus: false;
        in-out property <float> starlab-lfo-phase: 0;
        in-out property <float> starlab-tank-energy: 0;
        in-out property <[float]> starlab-waveform-mid-x: [];
        in-out property <[float]> starlab-waveform-mid-y: [];
        in-out property <[float]> starlab-waveform-length: [];
        in-out property <[float]> starlab-waveform-angle: [];

        // --- Warps-specific state (active-kind == 21): real carrier/
        // modulator/output traces, or a vocoder band meter when that
        // algorithm slot is engaged. ---
        in-out property <string> warps-algorithm-name: "";
        in-out property <bool> warps-is-vocoder: false;
        in-out property <bool> warps-vocoder-frozen: false;
        in-out property <bool> warps-osc-enabled: false;
        in-out property <string> warps-osc-waveform-name: "";
        in-out property <[float]> warps-carrier-mid-x: [];
        in-out property <[float]> warps-carrier-mid-y: [];
        in-out property <[float]> warps-carrier-length: [];
        in-out property <[float]> warps-carrier-angle: [];
        in-out property <[float]> warps-modulator-mid-x: [];
        in-out property <[float]> warps-modulator-mid-y: [];
        in-out property <[float]> warps-modulator-length: [];
        in-out property <[float]> warps-modulator-angle: [];
        in-out property <[float]> warps-output-mid-x: [];
        in-out property <[float]> warps-output-mid-y: [];
        in-out property <[float]> warps-output-length: [];
        in-out property <[float]> warps-output-angle: [];
        in-out property <[float]> warps-vocoder-bands: [];

        // --- Mixer-specific state (active-kind == 22): real per-
        // channel strips -- fader position plus a live VU peak. ---
        in-out property <float> mixer-master: 1;
        in-out property <[float]> mixer-fader: [];
        in-out property <[float]> mixer-live: [];

        // --- CV Out-specific state (active-kind == 23): real per-
        // channel output levels (manual offset + external modulation,
        // the same value actually sent as a MIDI CC). ---
        in-out property <int> cv-out-midi-channel: 1;
        in-out property <[float]> cv-out-levels: [];

        // --- Magnito-specific state (active-kind == 24): the real
        // hysteresis-loop XY trace + tape-character state. ---
        in-out property <[float]> magnito-loop-mid-x: [];
        in-out property <[float]> magnito-loop-mid-y: [];
        in-out property <[float]> magnito-loop-length: [];
        in-out property <[float]> magnito-loop-angle: [];
        in-out property <float> magnito-bias: 0;
        in-out property <bool> magnito-unlimited: false;
        in-out property <[float]> magnito-wobble-mid-x: [];
        in-out property <[float]> magnito-wobble-mid-y: [];
        in-out property <[float]> magnito-wobble-length: [];
        in-out property <[float]> magnito-wobble-angle: [];
        in-out property <float> magnito-wow-flutter-depth: 0;
        in-out property <float> magnito-hiss-level: 0;
        in-out property <float> magnito-wear: 0;
        in-out property <bool> magnito-dropout: false;
        in-out property <float> magnito-output-peak: 0;

        // --- Settings' color wheel (active-kind == 25): hue +
        // saturation picked directly off the wheel below, brightness
        // browsed via the ordinary list -- see `ThemeExtra`. ---
        in-out property <bool> theme-editing-bg: false;
        in-out property <string> theme-hex: "";
        in-out property <float> theme-marker-x: 0;
        in-out property <float> theme-marker-y: 0;
        in-out property <color> theme-accent-swatch: #5CF07A;
        in-out property <color> theme-bg-swatch: #0B100C;
        callback theme-wheel-picked(float, float);

        // --- Tonestack-specific state (active-kind == 26): the real
        // post-chain output waveform + level/gate telemetry. ---
        in-out property <string> tonestack-voicing-name: "";
        in-out property <[float]> tonestack-waveform-mid-x: [];
        in-out property <[float]> tonestack-waveform-mid-y: [];
        in-out property <[float]> tonestack-waveform-length: [];
        in-out property <[float]> tonestack-waveform-angle: [];
        in-out property <float> tonestack-output-peak: 0;
        in-out property <bool> tonestack-gate-closed: false;

        // --- Sample Drum-specific state (active-kind == 27): the
        // real per-channel output waveform + slice position. ---
        in-out property <int> sample-drum-channel: 0;
        in-out property <string> sample-drum-sample-name: "";
        in-out property <int> sample-drum-num-slices: 1;
        in-out property <int> sample-drum-step-index: 0;
        in-out property <[float]> sample-drum-waveform-mid-x: [];
        in-out property <[float]> sample-drum-waveform-mid-y: [];
        in-out property <[float]> sample-drum-waveform-length: [];
        in-out property <[float]> sample-drum-waveform-angle: [];
        // The *source* sample's own waveform (0..1 peaks) and slice
        // cut points (0..1 fractions of the sample), for the slice
        // view -- see `SampleDrumExtra::source_waveform`/
        // `slice_bounds`. Distinct from the live-output waveform
        // above, which is what's actually sounding right now.
        in-out property <[float]> sample-drum-source-waveform: [];
        in-out property <[float]> sample-drum-slice-lowers: [];
        in-out property <[float]> sample-drum-slice-uppers: [];

        // --- Visualizer-specific state (active-kind == 28): the
        // real analyzer readout (scene-kind == 0) or one of the two
        // audio-reactive pixel-art scenes (see `VisualizerExtra`). ---
        in-out property <int> visualizer-mode-kind: 0;
        in-out property <string> visualizer-mode-name: "";
        in-out property <[float]> visualizer-spectrum: [];
        in-out property <[float]> visualizer-waveform-mid-x: [];
        in-out property <[float]> visualizer-waveform-mid-y: [];
        in-out property <[float]> visualizer-waveform-length: [];
        in-out property <[float]> visualizer-waveform-angle: [];
        in-out property <float> visualizer-peak-level: 0;
        in-out property <float> visualizer-rms-level: 0;
        in-out property <string> visualizer-pitch-name: "--";
        in-out property <int> visualizer-scene-kind: 0;
        in-out property <string> visualizer-scene-name: "Off";
        in-out property <float> visualizer-bass-level: 0;
        in-out property <float> visualizer-treble-level: 0;
        in-out property <float> visualizer-beat-pulse: 0;
        in-out property <float> visualizer-car-x: 0;
        in-out property <bool> visualizer-cat-paw-left: true;
        in-out property <bool> visualizer-monitor-on: false;

        // --- Real engine-load readout (Settings > Show CPU Usage) --
        // see `MixBus::load`'s own doc comment for exactly what this
        // measures (this simulator's own real processing cost versus
        // real-time, not a cycle-accurate STM32H7 model). ---
        in-out property <bool> cpu-visible: false;
        in-out property <float> cpu-load-pct: 0;

        // Re-exposed so Rust can hook them: callbacks/properties
        // declared on the inherited `DeviceFrame` aren't automatically
        // visible to this component's own generated Rust API.
        callback pad-toggled(int, bool);
        pad-pressed(i, down) => { root.pad-toggled(i, down); }
        callback knob1-clicked();
        callback knob2-clicked();
        knob1-pressed => { root.knob1-clicked(); }
        knob2-pressed => { root.knob2-clicked(); }
        callback f-clicked(int);
        f-pressed(i) => { root.f-clicked(i); }
        in-out property <angle> live-knob1-angle <=> self.knob1-angle;
        in-out property <angle> live-knob2-angle <=> self.knob2-angle;
        in-out property <[color]> live-pad-colors: [
            #232323, #232323, #232323, #232323,
            #232323, #232323, #232323, #232323,
            #232323, #232323, #232323, #232323,
            #232323, #232323, #232323, #232323,
        ];
        pad-colors: root.live-pad-colors;

        if !root.splash-active : Rectangle {
        HorizontalLayout {
            padding-left: 18px;
            padding-right: 18px;
            padding-top: 4px;
            spacing: 20px;

            if root.on-home : ParamListColumn {
                width: 604px;
                big-text: true;
                row-names: root.home-names;
                selected-row: root.home-selected;
                more-above: root.home-more-above;
                more-below: root.home-more-below;
                accent: root.accent;
            }
            // Bloom keeps its dedicated orbital layout; all other apps use
            // the shared parameter rail with their own ink, paper, and accent.
            if !root.on-home && root.active-kind != 29 : ParamListColumn {
                width: 278px;
                row-names: root.row-names;
                row-values: root.row-values;
                row-is-group: root.row-is-group;
                row-levels: root.row-levels;
                selected-row: root.selected-row;
                more-above: root.more-above;
                more-below: root.more-below;
                accent: root.accent;
                ink: root.live-ink;
                instrument-style: true;
                paper: root.live-bg;
            }

            if !root.on-home && root.active-kind == 0 : InstrumentPanel {
                width: 306px;
                caption: root.active-app-name == "MIDI Learn" ? "CONTROL / MIDI MAPPING" : "VOICE / CONTROL";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                    spacing: 12px;
                    padding-top: 18px;
                    Text {
                        text: root.selected-row < root.row-names.length ? root.row-names[root.selected-row] : root.active-app-name;
                        color: root.live-ink;
                        font-family: "Space Grotesk"; font-size: 24px; font-weight: 600;
                        wrap: word-wrap;
                    }
                    Rectangle { height: 2px; background: root.accent; }
                    Text {
                        text: root.selected-row < root.row-values.length ? root.row-values[root.selected-row] : "";
                        color: root.accent; font-family: "JetBrains Mono"; font-size: 17px;
                        wrap: word-wrap;
                    }
                    Rectangle { vertical-stretch: 1; }
                    Text {
                        text: root.active-app-name == "MIDI Learn" ? "Select a mapping to learn or edit its MIDI control." : "MIDI 1: cutoff · MIDI 2: volume. Press MIDI 2 to change waveform.";
                        color: root.live-ink.with-alpha(0.6); font-family: "Space Grotesk"; font-size: 12px; wrap: word-wrap;
                    }
                    InstrumentLabel { text: root.active-app-name == "Synth" ? "PADS PLAY  ·  R1 RESET CUTOFF" : "R1 SELECT  ·  ◀ ▶ ADJUST"; ink: root.live-ink; font-size: 9px; }
                }
            }

            // --- Plaits: real engine-selection dots + real
            // mode-switching analyzer -- same content
            // slint_plaits_live.rs has on its own. ---
            if !root.on-home && root.active-kind == 1 : VerticalLayout {
                padding-bottom: 8px;
                spacing: 6px;
                InstrumentHeading {
                    caption: "MACRO OSCILLATOR / BANK " + (root.plaits-engine-bank + 1);
                    title: root.plaits-engine-name;
                    ink: root.live-ink;
                    accent: root.plaits-bank-colors[root.plaits-engine-bank];
                }
                HorizontalLayout {
                    height: 30px;
                    spacing: 5px;
                    for model in [0, 1, 2, 3, 4, 5, 6, 7] : Rectangle {
                        border-width: 1px;
                        border-color: root.plaits-bank-colors[root.plaits-engine-bank].with-alpha(model == root.plaits-engine-led ? 1 : 0.2);
                        background: model == root.plaits-engine-led ? root.plaits-bank-colors[root.plaits-engine-bank] : transparent;
                        Text {
                            text: model + 1;
                            horizontal-alignment: center;
                            vertical-alignment: center;
                            font-family: "JetBrains Mono";
                            font-size: 12px;
                            color: model == root.plaits-engine-led ? root.live-bg : root.live-ink.with-alpha(0.6);
                        }
                    }
                }
                InstrumentLabel { text: root.plaits-analyzer-name + " / LIVE OUTPUT"; ink: root.live-ink; }
                ScopeSurface {
                    ink: root.live-ink;
                    accent: root.accent;
                    vertical-stretch: 1;

                    if root.plaits-analyzer-kind == 0 : HorizontalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        spacing: 2px;
                        for level in root.plaits-spectrum : Rectangle {
                            horizontal-stretch: 1;
                            Rectangle {
                                y: parent.height - self.height;
                                width: 100%;
                                height: level * parent.height;
                                background: root.accent;
                            }
                        }
                    }

                    if root.plaits-analyzer-kind == 1 : Rectangle {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        Rectangle {
                            y: parent.height / 2;
                            width: 100%; height: 1px;
                            background: root.live-ink.with-alpha(0.12);
                        }
                        SignalTrace {
                            width: 100%; height: 100%;
                            stroke-width: 3px;
                            round-caps: true;
                            mid-x: root.plaits-waveform-mid-x;
                            mid-y: root.plaits-waveform-mid-y;
                            lengths: root.plaits-waveform-length;
                            angles: root.plaits-waveform-angle;
                            trace: root.accent;
                        }
                    }

                    if root.plaits-analyzer-kind == 2 : VerticalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        spacing: 14px;
                        for row in [{ label: "Peak", level: root.plaits-peak-level }, { label: "RMS", level: root.plaits-rms-level }] : VerticalLayout {
                            height: 30px;
                            spacing: 4px;
                            Rectangle {
                                height: 20px;
                                border-width: 1px;
                                border-color: root.live-ink.with-alpha(0.15);
                                Rectangle {
                                    x: 0px; y: 0px;
                                    width: parent.width * row.level;
                                    height: 100%;
                                    background: root.accent;
                                }
                            }
                            Text {
                                text: row.label;
                                color: root.live-ink.with-alpha(0.65);
                                font-family: "JetBrains Mono";
                                font-size: 11px;
                            }
                        }
                    }

                    if root.plaits-analyzer-kind == 3 : VerticalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        alignment: center;
                        Text {
                            horizontal-alignment: center;
                            text: root.plaits-pitch-name;
                            color: root.accent;
                            font-family: "Space Grotesk";
                            font-weight: 700;
                            font-size: 36px;
                        }
                        Text {
                            horizontal-alignment: center;
                            text: "(autocorrelation)";
                            color: root.live-ink.with-alpha(0.35);
                            font-family: "JetBrains Mono";
                            font-size: 11px;
                        }
                    }
                }
            }

            // --- Analyzer: the same real mode-switching panel as
            // Plaits, no engine dots. ---
            if !root.on-home && root.active-kind == 3 : InstrumentPanel {
                width: 306px;
                caption: "SIGNAL / ANALYSIS";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                Text {
                    text: root.analyzer-name;
                    color: root.accent;
                    font-family: "Space Grotesk";
                    font-weight: 700;
                    font-size: 22px;
                }
                Rectangle {
                    vertical-stretch: 1;

                    if root.analyzer-kind == 0 : HorizontalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        spacing: 2px;
                        for level in root.analyzer-spectrum : Rectangle {
                            horizontal-stretch: 1;
                            Rectangle {
                                y: parent.height - self.height;
                                width: 100%;
                                height: level * parent.height;
                                background: root.accent;
                            }
                        }
                    }

                    if root.analyzer-kind == 1 : Rectangle {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        Rectangle {
                            y: parent.height / 2;
                            width: 100%; height: 1px;
                            background: root.live-ink.with-alpha(0.12);
                        }
                        for i in root.analyzer-waveform-mid-x.length : VectorSegment {
                        center-x: root.analyzer-waveform-mid-x[i] * 1px;
                        center-y: root.analyzer-waveform-mid-y[i] * 1px;
                        delta-x: root.analyzer-waveform-length[i] * cos(root.analyzer-waveform-angle[i] * 1deg);
                        delta-y: root.analyzer-waveform-length[i] * sin(root.analyzer-waveform-angle[i] * 1deg);
                        trace: root.accent; line-width: 2px;
                    }
                    }

                    if root.analyzer-kind == 2 : VerticalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        alignment: center;
                        Text {
                            horizontal-alignment: center;
                            text: "Spectrogram";
                            color: root.live-ink.with-alpha(0.35);
                            font-family: "JetBrains Mono";
                            font-size: 12px;
                        }
                        Text {
                            horizontal-alignment: center;
                            text: "(not rendered live -- see Spectrum)";
                            color: root.live-ink.with-alpha(0.25);
                            font-family: "JetBrains Mono";
                            font-size: 11px;
                        }
                    }

                    if root.analyzer-kind == 3 : VerticalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        spacing: 14px;
                        for row in [{ label: "Peak", level: root.analyzer-peak-level }, { label: "RMS", level: root.analyzer-rms-level }] : VerticalLayout {
                            height: 30px;
                            spacing: 4px;
                            Rectangle {
                                height: 20px;
                                border-width: 1px;
                                border-color: root.live-ink.with-alpha(0.15);
                                Rectangle {
                                    x: 0px; y: 0px;
                                    width: parent.width * row.level;
                                    height: 100%;
                                    background: root.accent;
                                }
                            }
                            Text {
                                text: row.label;
                                color: root.live-ink.with-alpha(0.65);
                                font-family: "JetBrains Mono";
                                font-size: 11px;
                            }
                        }
                    }

                    if root.analyzer-kind == 4 : VerticalLayout {
                        x: 0px; y: 0px;
                        width: 100%; height: 100%;
                        alignment: center;
                        Text {
                            horizontal-alignment: center;
                            text: root.analyzer-pitch-name;
                            color: root.accent;
                            font-family: "Space Grotesk";
                            font-weight: 700;
                            font-size: 36px;
                        }
                        Text {
                            horizontal-alignment: center;
                            text: "(autocorrelation)";
                            color: root.live-ink.with-alpha(0.35);
                            font-family: "JetBrains Mono";
                            font-size: 11px;
                        }
                    }
                }

                }
            }

            // --- Voltage: the real 2x2 Oscillators/Filter/Amp
            // Envelope/LFO panel grid -- each a bar-plot sketch of
            // the actual curve `draw_*_panel` computes from the
            // current knobs (see `VoltageApp::voltage_panels`). ---
            if !root.on-home && root.active-kind == 2 : InstrumentPanel {
                width: 306px;
                caption: "SUBTRACTIVE / VOICE";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 10px;
                // Explicit equal widths on all 4 boxes (matching the
                // PANEL_W the Rust side assumes when it lays out each
                // curve's segments) instead of GridLayout's own
                // column auto-sizing -- that sized each column by its
                // widest child (the "Amp Envelope"/"Oscillators"
                // labels are wider text than "Filter"/"LFO"), so the
                // left column ended up wider than the right one while
                // every curve still assumed the same fixed width,
                // overflowing the narrower Filter/LFO boxes.
                HorizontalLayout {
                    spacing: 10px;
                    Rectangle {
                        width: 138px;
                        border-width: 1px;
                        border-color: root.live-ink.with-alpha(0.12);
                        VerticalLayout {
                            padding: 4px;
                            Text { text: "Oscillators"; color: root.accent; font-family: "JetBrains Mono"; font-size: 10px; }
                            Rectangle {
                                vertical-stretch: 1;
                                for i in root.voltage-oscillator-mid-x.length : VectorSegment {
                        center-x: root.voltage-oscillator-mid-x[i] * 1px;
                        center-y: root.voltage-oscillator-mid-y[i] * 1px;
                        delta-x: root.voltage-oscillator-length[i] * cos(root.voltage-oscillator-angle[i] * 1deg);
                        delta-y: root.voltage-oscillator-length[i] * sin(root.voltage-oscillator-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.9); line-width: 2px;
                    }
                            }
                        }
                    }
                    Rectangle {
                        width: 138px;
                        border-width: 1px;
                        border-color: root.live-ink.with-alpha(0.12);
                        VerticalLayout {
                            padding: 4px;
                            Text { text: "Filter"; color: root.accent; font-family: "JetBrains Mono"; font-size: 10px; }
                            Rectangle {
                                vertical-stretch: 1;
                                for i in root.voltage-filter-mid-x.length : VectorSegment {
                        center-x: root.voltage-filter-mid-x[i] * 1px;
                        center-y: root.voltage-filter-mid-y[i] * 1px;
                        delta-x: root.voltage-filter-length[i] * cos(root.voltage-filter-angle[i] * 1deg);
                        delta-y: root.voltage-filter-length[i] * sin(root.voltage-filter-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.9); line-width: 2px;
                    }
                                Rectangle {
                                    x: root.voltage-filter-cutoff-frac * parent.width;
                                    y: 0px; width: 1px; height: 100%;
                                    background: rgba(255, 80, 80, 0.7);
                                }
                                if root.voltage-filter-has-swept : Rectangle {
                                    x: root.voltage-filter-swept-frac * parent.width;
                                    y: 0px; width: 1px; height: 100%;
                                    background: rgba(220, 180, 30, 0.7);
                                }
                            }
                        }
                    }
                }
                HorizontalLayout {
                    spacing: 10px;
                    Rectangle {
                        width: 138px;
                        border-width: 1px;
                        border-color: root.live-ink.with-alpha(0.12);
                        VerticalLayout {
                            padding: 4px;
                            Text { text: "Amp Envelope"; color: root.accent; font-family: "JetBrains Mono"; font-size: 10px; }
                            Rectangle {
                                vertical-stretch: 1;
                                for i in root.voltage-amp-env-mid-x.length : VectorSegment {
                        center-x: root.voltage-amp-env-mid-x[i] * 1px;
                        center-y: root.voltage-amp-env-mid-y[i] * 1px;
                        delta-x: root.voltage-amp-env-length[i] * cos(root.voltage-amp-env-angle[i] * 1deg);
                        delta-y: root.voltage-amp-env-length[i] * sin(root.voltage-amp-env-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.9); line-width: 2px;
                    }
                            }
                        }
                    }
                    Rectangle {
                        width: 138px;
                        border-width: 1px;
                        border-color: root.live-ink.with-alpha(0.12);
                        VerticalLayout {
                            padding: 4px;
                            Text { text: "LFO"; color: root.accent; font-family: "JetBrains Mono"; font-size: 10px; }
                            Rectangle {
                                vertical-stretch: 1;
                                for i in root.voltage-lfo-mid-x.length : VectorSegment {
                        center-x: root.voltage-lfo-mid-x[i] * 1px;
                        center-y: root.voltage-lfo-mid-y[i] * 1px;
                        delta-x: root.voltage-lfo-length[i] * cos(root.voltage-lfo-angle[i] * 1deg);
                        delta-y: root.voltage-lfo-length[i] * sin(root.voltage-lfo-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.9); line-width: 2px;
                    }
                            }
                        }
                    }
                }

                }
            }

            // --- Cascade: the real FM operator-routing graph. ---
            if !root.on-home && root.active-kind == 4 : InstrumentPanel {
                width: 306px;
                caption: "FM / OPERATOR ROUTING";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 8px;
                Text {
                    text: "ALGORITHM: " + root.cascade-algorithm-name;
                    color: root.accent;
                    font-family: "JetBrains Mono";
                    font-size: 12px;
                }
                Rectangle {
                    vertical-stretch: 1;
                    // Connections, drawn first so operator boxes sit
                    // on top of the lines feeding them -- real straight
                    // segments between true box centers (computed in
                    // Rust, see `cascade_connection_lines`).
                    for i in root.cascade-connection-mid-x.length : VectorSegment {
                        center-x: root.cascade-connection-mid-x[i] * 1px;
                        center-y: root.cascade-connection-mid-y[i] * 1px;
                        delta-x: root.cascade-connection-length[i] * cos(root.cascade-connection-angle[i] * 1deg);
                        delta-y: root.cascade-connection-length[i] * sin(root.cascade-connection-angle[i] * 1deg);
                        trace: rgba(60, 140, 200, 0.6); line-width: 2px;
                    }
                    for op in [0, 1, 2, 3, 4, 5] : Rectangle {
                        property <bool> carrier: op < root.cascade-carriers.length && root.cascade-carriers[op];
                        // Serpentine layout -- 0,1,2 left-to-right on
                        // top, 5,4,3 left-to-right on bottom -- so a
                        // consecutive-index chain (the common case)
                        // always connects adjacent boxes instead of
                        // jumping diagonally across the grid. Must
                        // match `cascade_op_center` in app.rs exactly.
                        property <int> op-col: op < 3 ? mod(op, 3) : 5 - op;
                        x: 20px + self.op-col * 90px;
                        y: 10px + floor(op / 3) * 60px;
                        width: 70px; height: 26px;
                        border-radius: 4px;
                        background: self.carrier ? root.accent.with-alpha(0.18) : rgba(60, 140, 200, 0.12);
                        border-width: 1px;
                        border-color: self.carrier ? root.accent : rgba(60, 140, 200, 0.4);
                        // A real lit LED per carrier -- the same way
                        // the physical module's own carrier-output
                        // indicators work, not just a text label.
                        Rectangle {
                            x: 6px; y: parent.height / 2 - 3px;
                            width: 6px; height: 6px;
                            border-radius: 3px;
                            background: parent.carrier ? root.accent : root.live-ink.with-alpha(0.08);
                        }
                        Text {
                            text: "Op" + (op + 1) + (op == root.cascade-feedback-op ? " FB" : "");
                            color: parent.carrier ? root.accent : rgba(140, 190, 230, 0.9);
                            font-family: "JetBrains Mono";
                            font-size: 10px;
                            horizontal-alignment: center;
                            vertical-alignment: center;
                            width: 100%; height: 100%;
                        }
                    }
                }

                }
            }

            // --- Bloom/Madness: the real orbiting-dots clock face --
            // outer trigger-reference dots on a fixed ring, inner
            // note dots each at their own live-drifting angle. ---
            if !root.on-home && root.active-kind == 5 : InstrumentPanel {
                width: 306px;
                caption: "GENERATIVE / GEOMETRY";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                Text {
                    text: root.shape-running ? "running" : "stopped";
                    color: root.accent;
                    font-family: "JetBrains Mono";
                    font-size: 12px;
                }
                Rectangle {
                    vertical-stretch: 1;
                    property <length> cx: self.width / 2;
                    property <length> cy: self.height / 2;
                    property <length> radius: min(self.width, self.height) / 2 - 14px;
                    Rectangle {
                        x: parent.cx - parent.radius; y: parent.cy - parent.radius;
                        width: parent.radius * 2; height: parent.radius * 2;
                        border-radius: parent.radius;
                        border-width: 1px;
                        border-color: root.live-ink.with-alpha(0.1);
                    }
                    // The connecting web between consecutive inner
                    // dots -- Bloom's own spiral (innermost to
                    // outermost), Madness's closed ring. Drawn under
                    // the dots themselves.
                    for i in root.shape-inner-line-length.length : VectorSegment {
                        center-x: parent.cx + root.shape-inner-line-mid-x[i] * parent.radius;
                        center-y: parent.cy + root.shape-inner-line-mid-y[i] * parent.radius;
                        delta-x: root.shape-inner-line-length[i] * parent.radius / 1px * cos(root.shape-inner-line-angle[i] * 1deg);
                        delta-y: root.shape-inner-line-length[i] * parent.radius / 1px * sin(root.shape-inner-line-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.55); line-width: 1.5px;
                    }
                    for i in root.shape-outer-flash-active.length : VectorSegment {
                        visible: root.shape-outer-flash-active[i];
                        center-x: parent.cx + cos(root.shape-outer-flash-angle[i] * 1deg) * parent.radius / 2;
                        center-y: parent.cy + sin(root.shape-outer-flash-angle[i] * 1deg) * parent.radius / 2;
                        delta-x: parent.radius / 1px * cos(root.shape-outer-flash-angle[i] * 1deg);
                        delta-y: parent.radius / 1px * sin(root.shape-outer-flash-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.7); line-width: 2px;
                    }
                    // Denser patterns (more inner dots -- Bloom now
                    // allows up to 32, Harmony-Bloom-style, instead of
                    // the original 16) shrink every dot a step so a
                    // fully-populated shape reads as a woven web
                    // instead of a smear of overlapping circles.
                    property <bool> dense: root.shape-inner-x.length > 16;
                    for i in [0, 1, 2, 3, 4, 5, 6, 7] : Rectangle {
                        property <bool> valid: i < root.shape-outer-x.length;
                        property <length> dd: (self.valid && root.shape-outer-lit[i]) ? (parent.dense ? 11px : 14px) : (parent.dense ? 7px : 9px);
                        visible: self.valid;
                        x: parent.cx + (self.valid ? root.shape-outer-x[i] : 0) * parent.radius - self.dd / 2;
                        y: parent.cy + (self.valid ? root.shape-outer-y[i] : 0) * parent.radius - self.dd / 2;
                        width: self.dd; height: self.dd;
                        border-radius: self.dd / 2;
                        background: !self.valid ? transparent : (!root.shape-outer-active[i] ? root.live-ink.with-alpha(0.08) : root.shape-outer-lit[i] ? root.accent : root.accent.with-alpha(0.4));
                    }
                    for i in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31] : Rectangle {
                        property <bool> valid: i < root.shape-inner-x.length;
                        property <length> dd: (self.valid && root.shape-inner-lit[i]) ? (parent.dense ? 8px : 12px) : (parent.dense ? 4px : 7px);
                        visible: self.valid;
                        x: parent.cx + (self.valid ? root.shape-inner-x[i] : 0) * parent.radius - self.dd / 2;
                        y: parent.cy + (self.valid ? root.shape-inner-y[i] : 0) * parent.radius - self.dd / 2;
                        width: self.dd; height: self.dd;
                        border-radius: self.dd / 2;
                        background: !self.valid ? transparent : root.shape-inner-lit[i] ? root.accent : root.accent.with-alpha(0.45);
                    }
                }

                }
            }

            // --- Nebula: the real gravity-well particle arena. ---
            if !root.on-home && root.active-kind == 6 : InstrumentPanel {
                width: 306px;
                caption: "PARTICLES / GRAVITY";
                ink: root.live-ink; accent: root.accent;
                Rectangle {
                property <length> cx: self.width / 2;
                property <length> cy: self.height / 2;
                property <length> radius: min(self.width, self.height) / 2 - 14px;
                    Rectangle { x: parent.cx - parent.radius; y: parent.cy; width: 2 * parent.radius; height: 1px; background: root.live-ink.with-alpha(0.08); }
                    Rectangle { x: parent.cx; y: parent.cy - parent.radius; width: 1px; height: 2 * parent.radius; background: root.live-ink.with-alpha(0.08); }

                Rectangle {
                    x: parent.cx - parent.radius; y: parent.cy - parent.radius;
                    width: parent.radius * 2; height: parent.radius * 2;
                    border-radius: parent.radius;
                    border-width: 1px;
                    border-color: root.live-ink.with-alpha(0.1);
                }
                for i in [0, 1, 2] : Rectangle {
                    property <bool> valid: i < root.nebula-well-x.length;
                    property <length> dd: (self.valid && root.nebula-well-lit[i]) ? 18px : 13px;
                    visible: self.valid;
                    x: parent.cx + (self.valid ? root.nebula-well-x[i] : 0) * parent.radius - self.dd / 2;
                    y: parent.cy + (self.valid ? root.nebula-well-y[i] : 0) * parent.radius - self.dd / 2;
                    width: self.dd; height: self.dd;
                    border-radius: self.dd / 2;
                    border-width: self.valid && root.nebula-well-lit[i] ? 3px : 2px;
                    border-color: !self.valid ? transparent : (!root.nebula-well-active[i] ? root.live-ink.with-alpha(0.1) : root.nebula-well-lit[i] ? #00ff88 : #0088cc);
                }
                for i in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] : Rectangle {
                    property <bool> valid: i < root.nebula-particle-x.length;
                    visible: self.valid;
                    x: parent.cx + (self.valid ? root.nebula-particle-x[i] : 0) * parent.radius - 4px;
                    y: parent.cy + (self.valid ? root.nebula-particle-y[i] : 0) * parent.radius - 4px;
                    width: 8px; height: 8px;
                    border-radius: 4px;
                    background: !self.valid ? transparent : root.accent.with-alpha(0.35 + 0.65 * root.nebula-particle-brightness[i]);
                }

                }
            }

            // --- Tape: 4 real track lanes + playhead. ---
            if !root.on-home && root.active-kind == 7 : InstrumentPanel {
                width: 306px;
                caption: "RECORDER / FOUR TRACK";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 12px;
                padding-top: 8px;
                for t in [0, 1, 2, 3] : Rectangle {
                    property <bool> valid: t < root.tape-track-status.length;
                    property <int> kind: self.valid ? root.tape-track-kind[t] : 0;
                    height: 42px;
                    border-width: 1px;
                    border-color: root.live-ink.with-alpha(0.12);
                    background: self.kind == 2 ? rgba(180, 30, 30, 0.25) : self.kind == 1 ? root.accent.with-alpha(0.15) : root.live-ink.with-alpha(0.03);
                    Text { x: 8px; y: 5px; text: "0" + (t + 1); color: root.live-ink.with-alpha(0.6); font-family: "JetBrains Mono"; font-size: 10px; }
                    if self.kind != 0 : Rectangle {
                        x: root.tape-playhead-frac * (parent.width - 2px);
                        y: 0px; width: 2px; height: 100%;
                        background: root.accent;
                    }
                    Text {
                        text: parent.valid ? root.tape-track-status[t] : "";
                        color: root.accent;
                        font-family: "JetBrains Mono";
                        font-size: 12px;
                        vertical-alignment: center;
                        horizontal-alignment: center;
                        width: 100%; height: 100%;
                    }
                }

                }
            }

            // --- Pam's: the real CV monitor scope. ---
            if !root.on-home && root.active-kind == 8 : VerticalLayout {
                padding-bottom: 8px;
                spacing: 8px;
                InstrumentHeading {
                    caption: "CLOCK / MODULATION";
                    title: "CHANNEL " + (root.pams-channel-index < 9 ? "0" : "") + (root.pams-channel-index + 1);
                    ink: root.live-ink; accent: root.accent;
                }
                HorizontalLayout {
                    height: 24px; spacing: 4px;
                    for channel in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] : Rectangle {
                        background: channel == root.pams-channel-index ? root.accent : root.accent.with-alpha(0.07);
                        Text {
                            text: channel + 1; horizontal-alignment: center;
                            vertical-alignment: center; font-size: 11px;
                            color: channel == root.pams-channel-index ? root.live-bg : root.accent.with-alpha(0.65);
                        }
                    }
                }
                InstrumentLabel { text: "BIPOLAR CV / CHANNEL HISTORY"; ink: root.live-ink; }
                ScopeSurface {
                    ink: root.live-ink; accent: root.accent;
                    vertical-stretch: 1;
                    Rectangle {
                        y: parent.height / 2;
                        width: 100%; height: 1px;
                        background: rgba(255, 255, 255, 0.1);
                    }
                    HorizontalLayout {
                        x: 0px; y: 0px; width: 100%; height: 100%;
                        spacing: 0px;
                        for v in root.pams-history : Rectangle {
                            horizontal-stretch: 1;
                            property <length> mag: (v >= 0 ? v : -v) * (parent.height / 2);
                            Rectangle {
                                y: v >= 0 ? parent.height / 2 - mag : parent.height / 2;
                                width: 100%; height: mag;
                                background: root.accent.with-alpha(0.75);
                            }
                        }
                    }
                }
                Text {
                    text: "value: " + Math.round(root.pams-value * 100) / 100;
                    color: rgba(255, 255, 255, 0.4);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                }
            }

            // --- Singularity: the real chaotic orbiting point. ---
            if !root.on-home && root.active-kind == 9 : InstrumentPanel {
                width: 306px;
                caption: "CHAOS / ORBIT";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 8px;
                Text {
                    text: "chaos: " + Math.round(root.orbit-value * 10000) / 10000;
                    color: root.accent;
                    font-family: "JetBrains Mono";
                    font-size: 12px;
                }
                Rectangle {
                    vertical-stretch: 1;
                    property <length> cx: self.width / 2;
                    property <length> cy: self.height / 2;
                    property <length> radius: min(self.width, self.height) / 2 - 14px;
                    Rectangle { x: parent.cx - parent.radius; y: parent.cy; width: 2 * parent.radius; height: 1px; background: root.live-ink.with-alpha(0.08); }
                    Rectangle { x: parent.cx; y: parent.cy - parent.radius; width: 1px; height: 2 * parent.radius; background: root.live-ink.with-alpha(0.08); }

                    Rectangle {
                        x: parent.cx - parent.radius; y: parent.cy - parent.radius;
                        width: parent.radius * 2; height: parent.radius * 2;
                        border-radius: parent.radius;
                        border-width: 1px;
                        border-color: root.live-ink.with-alpha(0.1);
                    }
                    Rectangle {
                        x: parent.cx + root.orbit-x * parent.radius - 7px;
                        y: parent.cy + root.orbit-y * parent.radius - 7px;
                        width: 14px; height: 14px;
                        border-radius: 7px;
                        background: root.accent;
                    }
                }

                }
            }

            // --- Prism: the real tap-time/rate map. ---
            if !root.on-home && root.active-kind == 10 : InstrumentPanel {
                width: 306px;
                caption: "DELAY / TAP DISTRIBUTION";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 8px;
                padding-top: 0px;
                Rectangle {
                    vertical-stretch: 1;
                    Rectangle {
                        y: parent.height - 1px;
                        width: 100%; height: 1px;
                        background: root.live-ink.with-alpha(0.15);
                    }
                    for i in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] : Rectangle {
                        property <bool> valid: i < root.prism-tick-x.length;
                        visible: self.valid;
                        x: (self.valid ? root.prism-tick-x[i] : 0) * (parent.width - 4px);
                        width: 3px;
                        property <length> h: (self.valid ? root.prism-tick-height[i] : 0) * parent.height;
                        y: parent.height - self.h;
                        height: self.h;
                        background: root.accent.with-alpha(0.4 + 0.6 * (self.valid ? root.prism-tick-height[i] : 0));
                    }
                }
                Text {
                    text: root.prism-caption;
                    color: root.accent;
                    font-family: "JetBrains Mono";
                    font-size: 12px;
                }

                }
            }

            // --- Nautilus: the real 8-channel gate LEDs. ---
            if !root.on-home && root.active-kind == 11 : InstrumentPanel {
                width: 306px;
                caption: "DELAY / EIGHT LINES";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                Text {
                    text: root.nautilus-delay-mode-name + " / " + root.nautilus-feedback-mode-name + (root.nautilus-frozen ? "  (frozen)" : "");
                    color: root.nautilus-mode-colors[root.nautilus-delay-mode-index];
                    font-family: "Space Grotesk";
                    font-weight: 700;
                    font-size: 15px;
                    wrap: word-wrap;
                    width: 260px;
                }
                Text {
                    text: "chroma: " + root.nautilus-chroma-name;
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 10px;
                }
                Rectangle { height: 4px; }
                Text {
                    text: "DELAY LINES";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                Rectangle {
                    height: 130px;
                    for i in root.nautilus-line-level.length : Rectangle {
                        Text {
                            x: 6px; y: 4px; text: i + 1;
                            color: root.live-ink.with-alpha(0.65);
                            font-family: "JetBrains Mono"; font-size: 9px;
                        }

                        property <bool> active: i < root.nautilus-line-active.length && root.nautilus-line-active[i];
                        property <bool> pulse: i < root.nautilus-line-pulse.length && root.nautilus-line-pulse[i];
                        property <float> lvl: root.nautilus-line-level[i];
                        x: mod(i, 4) * (parent.width / 4);
                        y: floor(i / 4) * (parent.height / 2);
                        width: parent.width / 4 - 6px;
                        height: parent.height / 2 - 6px;
                        border-radius: 4px;
                        opacity: self.active ? 1.0 : 0.3;
                        background: root.nautilus-mode-colors[root.nautilus-delay-mode-index].with-alpha(0.15 + min(1.0, self.lvl) * 0.65);
                        border-width: self.pulse ? 2px : 1px;
                        border-color: self.pulse ? #ffffff : root.live-ink.with-alpha(0.15);
                        animate background { duration: 60ms; }
                    }
                }
                Rectangle { height: 2px; }
                Text { text: "feedback " + Math.round(root.nautilus-feedback-amount * 100) + "%"; color: root.live-ink.with-alpha(0.65); font-family: "JetBrains Mono"; font-size: 10px; }

                }
            }

            // --- Sequencer: pattern bank + (Song mode) arrangement
            // strip, above whichever 4x4 grid is live -- the real
            // step grid normally, or the Pad Perform bank while that
            // mode's on (see `sequencer-pad-perform`). ---
            if !root.on-home && root.active-kind == 12 : InstrumentPanel {
                width: 306px;
                caption: "PATTERN / PERFORMANCE";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                HorizontalLayout {
                    spacing: 3px;
                    height: 14px;
                    for p in root.sequencer-num-patterns : Rectangle {
                        border-radius: 2px;
                        background: p == root.sequencer-current-pattern ? root.accent : root.live-ink.with-alpha(0.12);
                        Text {
                            text: p + 1;
                            color: p == root.sequencer-current-pattern ? #0a0a0a : root.live-ink.with-alpha(0.5);
                            font-family: "JetBrains Mono";
                            font-size: 8px;
                            horizontal-alignment: center;
                            vertical-alignment: center;
                            width: 100%; height: 100%;
                        }
                    }
                }
                if root.sequencer-song-mode : HorizontalLayout {
                    spacing: 3px;
                    height: 20px;
                    for pat[i] in root.sequencer-song-slot-pattern : Rectangle {
                        property <bool> in-range: i < root.sequencer-song-length;
                        property <bool> playing: i == root.sequencer-song-pos;
                        visible: self.in-range;
                        border-radius: 2px;
                        background: self.playing ? root.accent : root.live-ink.with-alpha(0.08);
                        border-width: self.playing ? 0px : 1px;
                        border-color: root.live-ink.with-alpha(0.15);
                        Text {
                            text: (pat + 1) + "x" + (i < root.sequencer-song-slot-repeats.length ? root.sequencer-song-slot-repeats[i] : 1);
                            color: parent.playing ? #0a0a0a : root.live-ink.with-alpha(0.5);
                            font-family: "JetBrains Mono";
                            font-size: 8px;
                            horizontal-alignment: center;
                            vertical-alignment: center;
                            width: 100%; height: 100%;
                        }
                    }
                }
                Text {
                    text: root.sequencer-header;
                    color: root.accent;
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    wrap: word-wrap;
                }
                if !root.sequencer-pad-perform : Rectangle {
                    vertical-stretch: 1;
                    for label[i] in root.sequencer-step-label : Rectangle {
                        property <bool> active: i < root.sequencer-step-active.length && root.sequencer-step-active[i];
                        property <bool> trimmed: i < root.sequencer-step-trimmed.length && root.sequencer-step-trimmed[i];
                        property <bool> playhead: i < root.sequencer-step-playhead.length && root.sequencer-step-playhead[i];
                        property <bool> focused: i < root.sequencer-step-focused.length && root.sequencer-step-focused[i];
                        x: mod(i, 4) * (parent.width / 4);
                        y: floor(i / 4) * (parent.height / 4);
                        width: parent.width / 4 - 5px;
                        height: parent.height / 4 - 5px;
                        border-radius: 3px;
                        opacity: self.trimmed ? 0.35 : 1.0;
                        background: self.active ? root.accent.with-alpha(self.playhead ? 1.0 : 0.55) : root.live-ink.with-alpha(0.06);
                        border-width: self.focused ? 2px : 1px;
                        border-color: self.focused ? #ffffff : (self.playhead ? root.accent : root.live-ink.with-alpha(0.15));
                        Text {
                            text: label;
                            color: parent.active ? #0a0a0a : root.live-ink.with-alpha(0.5);
                            font-family: "JetBrains Mono";
                            font-size: 10px;
                            horizontal-alignment: right;
                            vertical-alignment: top;
                            x: 0px; y: 2px;
                            width: parent.width - 4px; height: 100%;
                        }
                    }
                }
                if root.sequencer-pad-perform : Rectangle {
                    vertical-stretch: 1;
                    for loaded[i] in root.sequencer-pad-loaded : Rectangle {
                        property <bool> focused: i == root.sequencer-last-touched-pad;
                        x: mod(i, 4) * (parent.width / 4);
                        y: floor(i / 4) * (parent.height / 4);
                        width: parent.width / 4 - 5px;
                        height: parent.height / 4 - 5px;
                        border-radius: 3px;
                        background: loaded ? root.accent.with-alpha(0.55) : root.live-ink.with-alpha(0.06);
                        border-width: self.focused ? 2px : 1px;
                        border-color: self.focused ? #ffffff : root.live-ink.with-alpha(0.15);
                        Text {
                            text: i + 1;
                            color: loaded ? #0a0a0a : root.live-ink.with-alpha(0.5);
                            font-family: "JetBrains Mono";
                            font-size: 10px;
                            horizontal-alignment: right;
                            vertical-alignment: top;
                            x: 0px; y: 2px;
                            width: parent.width - 4px; height: 100%;
                        }
                    }
                }

                }
            }

            // --- Clouds: real playback-mode indicator dots (one per
            // mode, current one lit in its own color) + the real
            // granular output's scrolling monitor. ---
            if !root.on-home && root.active-kind == 13 : InstrumentPanel {
                width: 306px;
                caption: "GRANULAR / CLOUD";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                Text {
                    text: "MODE";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                HorizontalLayout {
                    spacing: 10px;
                    height: 22px;
                    for m in [0, 1, 2, 3] : Rectangle {
                        width: 20px; height: 20px;
                        border-radius: 10px;
                        background: m == root.clouds-mode ? root.clouds-mode-colors[m] : root.live-ink.with-alpha(0.14);
                        animate background { duration: 60ms; }
                    }
                }
                Text {
                    text: root.clouds-mode-name + (root.clouds-frozen ? "  (frozen)" : "");
                    color: root.clouds-mode-colors[root.clouds-mode];
                    font-family: "Space Grotesk";
                    font-weight: 700;
                    font-size: 22px;
                    wrap: word-wrap;
                    width: 260px;
                }
                Rectangle { height: 6px; }
                Text {
                    text: "OUTPUT";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                Rectangle {
                    vertical-stretch: 1;
                    Rectangle {
                        y: parent.height / 2;
                        width: 100%; height: 1px;
                        background: root.live-ink.with-alpha(0.12);
                    }
                    for i in root.clouds-waveform-mid-x.length : VectorSegment {
                        center-x: root.clouds-waveform-mid-x[i] * 1px;
                        center-y: root.clouds-waveform-mid-y[i] * 1px;
                        delta-x: root.clouds-waveform-length[i] * cos(root.clouds-waveform-angle[i] * 1deg);
                        delta-y: root.clouds-waveform-length[i] * sin(root.clouds-waveform-angle[i] * 1deg);
                        trace: root.clouds-frozen ? #2090C8 : root.accent; line-width: 2px;
                    }
                }

                }
            }

            // --- Beads: the real grain cloud -- a scrolling capture-
            // buffer snapshot with each currently active grain
            // plotted at its real read position, glowing by its real
            // envelope amplitude. ---
            if !root.on-home && root.active-kind == 14 : VerticalLayout {
                padding-bottom: 8px;
                spacing: 6px;
                InstrumentHeading {
                    caption: "TEXTURE SYNTHESIZER";
                    title: root.beads-delay-mode ? "DELAY" : root.beads-mode-name;
                    ink: root.live-ink; accent: root.accent;
                }
                InstrumentLabel {
                    text: (root.beads-frozen ? "FROZEN" : "CAPTURING") + " / " + root.beads-grain-x.length + " ACTIVE GRAINS";
                    ink: root.live-ink;
                }
                ScopeSurface {
                    ink: root.live-ink;
                    accent: root.accent;
                    vertical-stretch: 1;
                    border-width: 1px;
                    border-color: rgba(255, 255, 255, 0.1);
                    border-radius: 4px;
                    Rectangle {
                        y: parent.height / 2;
                        width: 100%; height: 1px;
                        background: rgba(255, 255, 255, 0.12);
                    }
                    // The buffer waveform, as a real connected curve.
                    SignalTrace {
                        width: 100%; height: 100%;
                        source-height: 150;
                        mid-x: root.beads-waveform-mid-x;
                        mid-y: root.beads-waveform-mid-y;
                        lengths: root.beads-waveform-length;
                        angles: root.beads-waveform-angle;
                        trace: root.accent.with-alpha(0.7);
                    }
                    // Each active grain, scattered along the buffer
                    // at its real read position, glowing by its real
                    // envelope amplitude.
                    for i in root.beads-grain-x.length : Rectangle {
                        property <float> b: root.beads-grain-brightness[i];
                        x: 8px + root.beads-grain-x[i] * (parent.width - 16px) - self.width / 2;
                        y: parent.height * (0.8 - self.b * 0.6) - self.height / 2;
                        width: 5px + self.b * 9px;
                        height: self.width;
                        border-radius: self.width / 2;
                        background: root.accent.with-alpha(0.25 + self.b * 0.75);

                    }
                }
                InstrumentLabel { text: "X  BUFFER POSITION / Y  ENVELOPE"; ink: root.live-ink; }
            }

            // --- Black Hole: real category badge + clip LED + an
            // oscilloscope of the actual processed output (works the
            // same regardless of which of the 24 algorithms is
            // active) + this algorithm's real per-slot parameter
            // meters. ---
            if !root.on-home && root.active-kind == 15 : VerticalLayout {
                padding-bottom: 8px;
                spacing: 6px;
                Text {
                    text: "MULTI EFFECT / CATEGORY";
                    color: rgba(255, 255, 255, 0.4);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                HorizontalLayout {
                    spacing: 8px;
                    height: 16px;
                    Rectangle {
                        width: 10px; height: 10px;
                        y: 3px;
                        border-radius: 5px;
                        background: root.black-hole-category-colors[root.black-hole-category-index];
                    }
                    Text {
                        text: root.black-hole-category-name;
                        color: root.black-hole-category-colors[root.black-hole-category-index];
                        font-family: "JetBrains Mono";
                        font-weight: 700;
                        font-size: 12px;
                    }
                    Rectangle { horizontal-stretch: 1; }
                    Rectangle {
                        width: 10px; height: 10px;
                        y: 3px;
                        border-radius: 5px;
                        background: root.black-hole-clip ? #FF4D4D : rgba(255, 255, 255, 0.12);
                    }
                    Text {
                        text: "CLIP";
                        color: root.black-hole-clip ? #FF4D4D : rgba(255, 255, 255, 0.3);
                        font-family: "JetBrains Mono";
                        font-size: 10px;
                    }
                }
                Text {
                    text: root.black-hole-algorithm-name;
                    color: root.black-hole-category-colors[root.black-hole-category-index];
                    font-family: "Space Grotesk";
                    font-weight: 700;
                    font-size: 22px;
                    overflow: elide;
                }
                Rectangle { height: 4px; }
                Text {
                    text: "OUTPUT";
                    color: rgba(255, 255, 255, 0.4);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                ScopeSurface {
                    ink: root.live-ink;
                    accent: root.accent;
                    height: 90px;
                    border-width: 1px;
                    border-color: rgba(255, 255, 255, 0.1);
                    border-radius: 4px;
                    Rectangle {
                        y: parent.height / 2;
                        width: 100%; height: 1px;
                        background: rgba(255, 255, 255, 0.12);
                    }
                    SignalTrace {
                        width: 100%; height: 100%;
                        source-height: 110;
                        mid-x: root.black-hole-waveform-mid-x;
                        mid-y: root.black-hole-waveform-mid-y;
                        lengths: root.black-hole-waveform-length;
                        angles: root.black-hole-waveform-angle;
                        trace: root.accent.with-alpha(0.7);
                    }
                }
                Rectangle { height: 4px; }
                Text {
                    text: "PARAMETERS";
                    color: rgba(255, 255, 255, 0.4);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                HorizontalLayout {
                    spacing: 10px;
                    for i in [0, 1, 2] : ValueTrack {
                        label: i < root.black-hole-param-labels.length ? root.black-hole-param-labels[i] : "";
                        value: i < root.black-hole-param-values.length ? root.black-hole-param-values[i] : 0;
                        ink: root.live-ink;
                        accent: i < root.black-hole-param-bold.length && root.black-hole-param-bold[i] ? #e59df2 : root.accent;
                    }
                }
            }

            // --- Queen of Pentacles: the real chaotic-map trajectory
            // + live CV/Gate/Delta output state. ---
            if !root.on-home && root.active-kind == 16 : InstrumentPanel {
                width: 306px;
                caption: "CHAOS / GATE + CV";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                Text {
                    text: "MAP";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                Text {
                    text: root.qop-map-name + "  r=" + Math.round(root.qop-r * 1000) / 1000 + (root.qop-frozen ? "  (frozen)" : "");
                    color: root.accent;
                    font-family: "Space Grotesk";
                    font-weight: 700;
                    font-size: 20px;
                    wrap: word-wrap;
                    width: 260px;
                }
                Rectangle { height: 4px; }
                Text {
                    text: "TRAJECTORY";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                Rectangle {
                    height: 110px;
                    border-width: 1px;
                    border-color: root.live-ink.with-alpha(0.1);
                    border-radius: 4px;
                    Rectangle {
                        y: parent.height * (1.0 - root.qop-threshold);
                        width: 100%; height: 1px;
                        background: rgba(255, 80, 80, 0.4);
                    }
                    for i in root.qop-trajectory-mid-x.length : VectorSegment {
                        center-x: root.qop-trajectory-mid-x[i] * 1px;
                        center-y: root.qop-trajectory-mid-y[i] * 1px;
                        delta-x: root.qop-trajectory-length[i] * cos(root.qop-trajectory-angle[i] * 1deg);
                        delta-y: root.qop-trajectory-length[i] * sin(root.qop-trajectory-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.8); line-width: 2px;
                    }
                }
                Rectangle { height: 4px; }
                HorizontalLayout {
                    spacing: 14px;
                    height: 40px;
                    Rectangle {
                        width: 14px; height: 14px;
                        y: 4px;
                        border-radius: 7px;
                        background: root.qop-gate ? root.accent : root.live-ink.with-alpha(0.12);
                    }
                    VerticalLayout {
                        Text { text: root.qop-gate-mode-name; color: root.live-ink.with-alpha(0.65); font-family: "JetBrains Mono"; font-size: 10px; }
                        Text { text: "CV " + Math.round(root.qop-cv * 100) + "%  smooth " + Math.round(root.qop-smooth-cv * 100) + "%"; color: root.live-ink.with-alpha(0.6); font-family: "JetBrains Mono"; font-size: 10px; }
                        Text { text: "delta " + Math.round(root.qop-delta * 100) + "%"; color: root.live-ink.with-alpha(0.6); font-family: "JetBrains Mono"; font-size: 10px; }
                    }
                }

                }
            }

            // --- Natural Gate: two mirrored channel panels, each a
            // real envelope trace (the same value simultaneously
            // driving the VCA and the lowpass brightness) + a hit LED.
            if !root.on-home && root.active-kind == 17 : InstrumentPanel {
                width: 306px;
                caption: "DYNAMICS / DUAL ENVELOPE";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                Text {
                    text: "ENVELOPES";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                HorizontalLayout {
                    spacing: 10px;
                    vertical-stretch: 1;
                    VerticalLayout {
                        spacing: 4px;
                        HorizontalLayout {
                            spacing: 6px;
                            height: 16px;
                            Text { text: "CH1"; color: root.accent; font-family: "JetBrains Mono"; font-weight: 700; font-size: 11px; }
                            Rectangle { horizontal-stretch: 1; }
                            Rectangle {
                                width: 8px; height: 8px; y: 2px;
                                border-radius: 4px;
                                background: root.ng-ch1-hit ? root.accent : root.live-ink.with-alpha(0.12);
                            }
                        }
                        Rectangle {
                            vertical-stretch: 1;
                            border-width: 1px;
                            border-color: root.live-ink.with-alpha(0.1);
                            border-radius: 4px;
                            background: root.accent.with-alpha(root.ng-ch1-open * 0.12);
                            for i in root.ng-ch1-trace-mid-x.length : VectorSegment {
                        center-x: root.ng-ch1-trace-mid-x[i] * 1px;
                        center-y: root.ng-ch1-trace-mid-y[i] * 1px;
                        delta-x: root.ng-ch1-trace-length[i] * cos(root.ng-ch1-trace-angle[i] * 1deg);
                        delta-y: root.ng-ch1-trace-length[i] * sin(root.ng-ch1-trace-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.8); line-width: 2px;
                    }
                        }
                        Text { text: "open " + Math.round(root.ng-ch1-open * 100) + "%"; color: root.live-ink.with-alpha(0.65); font-family: "JetBrains Mono"; font-size: 10px; }
                    }
                    VerticalLayout {
                        spacing: 4px;
                        HorizontalLayout {
                            spacing: 6px;
                            height: 16px;
                            Text { text: "CH2"; color: root.accent; font-family: "JetBrains Mono"; font-weight: 700; font-size: 11px; }
                            Rectangle { horizontal-stretch: 1; }
                            Rectangle {
                                width: 8px; height: 8px; y: 2px;
                                border-radius: 4px;
                                background: root.ng-ch2-hit ? root.accent : root.live-ink.with-alpha(0.12);
                            }
                        }
                        Rectangle {
                            vertical-stretch: 1;
                            border-width: 1px;
                            border-color: root.live-ink.with-alpha(0.1);
                            border-radius: 4px;
                            background: root.accent.with-alpha(root.ng-ch2-open * 0.12);
                            for i in root.ng-ch2-trace-mid-x.length : VectorSegment {
                        center-x: root.ng-ch2-trace-mid-x[i] * 1px;
                        center-y: root.ng-ch2-trace-mid-y[i] * 1px;
                        delta-x: root.ng-ch2-trace-length[i] * cos(root.ng-ch2-trace-angle[i] * 1deg);
                        delta-y: root.ng-ch2-trace-length[i] * sin(root.ng-ch2-trace-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.8); line-width: 2px;
                    }
                        }
                        Text { text: "open " + Math.round(root.ng-ch2-open * 100) + "%"; color: root.live-ink.with-alpha(0.65); font-family: "JetBrains Mono"; font-size: 10px; }
                    }
                }

                }
            }

            // --- Turing Machine: the real 16-bit shift register as a
            // 4x4 grid, plus the real Locks-derived keep-probability
            // and lock-side state. ---
            if !root.on-home && root.active-kind == 18 : InstrumentPanel {
                width: 306px;
                caption: "RANDOM / SHIFT REGISTER";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                Text {
                    text: "REGISTER";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                Rectangle {
                    height: 138px;
                    for i in root.tm-bits.length : Rectangle {
                        Text {
                            x: 6px; y: 4px; text: i + 1;
                            color: root.live-ink.with-alpha(0.65);
                            font-family: "JetBrains Mono"; font-size: 9px;
                        }

                        property <bool> active: i < root.tm-active-len;
                        property <bool> lit: i < root.tm-bits.length && root.tm-bits[i];
                        property <bool> write-here: i == root.tm-write-index;
                        x: mod(i, 4) * (parent.width / 4);
                        y: floor(i / 4) * (parent.height / 4);
                        width: parent.width / 4 - 5px;
                        height: parent.height / 4 - 5px;
                        border-radius: 4px;
                        opacity: self.active ? 1.0 : 0.25;
                        background: self.lit ? root.accent.with-alpha(0.85) : root.live-ink.with-alpha(0.07);
                        border-width: self.write-here ? 2px : 1px;
                        border-color: self.write-here ? #ffffff : root.live-ink.with-alpha(0.15);
                        animate background { duration: 60ms; }
                    }
                }
                Rectangle { height: 2px; }
                HorizontalLayout {
                    spacing: 10px;
                    height: 14px;
                    Rectangle {
                        width: 10px; height: 10px; y: 2px;
                        border-radius: 5px;
                        background: root.tm-pulse ? root.accent : root.live-ink.with-alpha(0.12);
                    }
                    Text { text: "pulse"; color: root.live-ink.with-alpha(0.65); font-family: "JetBrains Mono"; font-size: 10px; }
                    Text { text: "cv " + Math.round(root.tm-cv * 100) + "%"; color: root.live-ink.with-alpha(0.65); font-family: "JetBrains Mono"; font-size: 10px; }
                    Rectangle { horizontal-stretch: 1; }
                    Text {
                        text: root.tm-double-locked ? "dbl lock" : (root.tm-inverted-feedback ? "locking" : "");
                        color: root.tm-double-locked ? #FFA040 : root.live-ink.with-alpha(0.4);
                        font-family: "JetBrains Mono";
                        font-size: 10px;
                    }
                }
                Rectangle {
                    height: 8px;
                    border-radius: 4px;
                    background: root.live-ink.with-alpha(0.1);
                    Rectangle {
                        x: 0px; y: 0px;
                        width: parent.width * root.tm-keep-probability;
                        height: 100%;
                        border-radius: 4px;
                        background: root.tm-inverted-feedback ? #FFA040 : root.accent;
                        animate width { duration: 80ms; }
                    }
                }
                Text { text: "keep probability " + Math.round(root.tm-keep-probability * 100) + "%"; color: root.live-ink.with-alpha(0.65); font-family: "JetBrains Mono"; font-size: 10px; }

                }
            }

            // --- Rainmaker: the real 16-tap timing map -- each tap
            // positioned by its true GRID+GROOVE-adjusted delay time,
            // height/brightness from its real level, color from its
            // real pan, muted taps greyed rather than hidden. ---
            if !root.on-home && root.active-kind == 19 : InstrumentPanel {
                width: 306px;
                caption: "RHYTHMIC / TAP MAP";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                Text {
                    text: root.rm-grid-label + "  --  " + root.rm-groove-name + " " + Math.round(root.rm-groove-amount * 100) + "%";
                    color: root.accent;
                    font-family: "Space Grotesk";
                    font-weight: 700;
                    font-size: 16px;
                    wrap: word-wrap;
                    width: 260px;
                }
                Rectangle { height: 4px; }
                Text {
                    text: "TAP MAP";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                Rectangle {
                    vertical-stretch: 1;
                    border-width: 1px;
                    border-color: root.live-ink.with-alpha(0.1);
                    border-radius: 4px;
                    // Beat-boundary gridlines.
                    for b in [1, 2, 3, 4, 5, 6, 7] : Rectangle {
                        property <float> frac: b / max(1.0, root.rm-beats-spanned);
                        visible: self.frac <= 1.0;
                        x: self.frac * parent.width;
                        width: 1px; height: 100%;
                        background: root.live-ink.with-alpha(0.08);
                    }
                    Rectangle {
                        y: parent.height - 1px;
                        width: 100%; height: 1px;
                        background: root.live-ink.with-alpha(0.15);
                    }
                    for i in root.rm-tap-time.length : Rectangle {
                        property <float> lvl: root.rm-tap-level[i];
                        property <float> pan: root.rm-tap-pan[i];
                        property <bool> muted: root.rm-tap-muted[i];
                        x: root.rm-tap-time[i] * (parent.width - 6px);
                        width: 6px;
                        height: max(3px, self.lvl * (parent.height - 6px));
                        y: parent.height - self.height;
                        border-radius: 2px;
                        opacity: self.muted ? 0.25 : 1.0;
                        background: self.pan < -0.15 ? #4090E0.with-alpha(0.5 + self.lvl * 0.5) : self.pan > 0.15 ? root.accent.with-alpha(0.5 + self.lvl * 0.5) : root.live-ink.with-alpha(0.6 + self.lvl * 0.4);
                        animate height { duration: 80ms; }
                    }
                }

                }
            }

            // --- StarLab: real texture/mode badges + a moving LFO
            // phase indicator + a real oscilloscope of the processed
            // output + a real tank-energy meter. ---
            if !root.on-home && root.active-kind == 20 : InstrumentPanel {
                width: 306px;
                caption: "REVERB / TEXTURE";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                HorizontalLayout {
                    spacing: 8px;
                    height: 16px;
                    Rectangle {
                        width: 10px; height: 10px; y: 3px;
                        border-radius: 5px;
                        background: root.starlab-texture-colors[root.starlab-texture-index];
                    }
                    Text {
                        text: root.starlab-texture-name + (root.starlab-karplus ? "  (Karplus)" : "") + (root.starlab-infinite ? "  \u{221E}" : "");
                        color: root.starlab-texture-colors[root.starlab-texture-index];
                        font-family: "JetBrains Mono";
                        font-weight: 700;
                        font-size: 12px;
                    }
                }
                Rectangle {
                    height: 100px;
                    border-width: 1px;
                    border-color: root.live-ink.with-alpha(0.1);
                    border-radius: 4px;
                    Rectangle {
                        y: parent.height / 2;
                        width: 100%; height: 1px;
                        background: root.live-ink.with-alpha(0.12);
                    }
                    for i in root.starlab-waveform-mid-x.length : VectorSegment {
                        center-x: root.starlab-waveform-mid-x[i] * 1px;
                        center-y: root.starlab-waveform-mid-y[i] * 1px;
                        delta-x: root.starlab-waveform-length[i] * cos(root.starlab-waveform-angle[i] * 1deg);
                        delta-y: root.starlab-waveform-length[i] * sin(root.starlab-waveform-angle[i] * 1deg);
                        trace: root.starlab-texture-colors[root.starlab-texture-index].with-alpha(0.8); line-width: 2px;
                    }
                    // Real LFO phase, as a dot sweeping the top edge.
                    Rectangle {
                        x: root.starlab-lfo-phase * (parent.width - 8px);
                        y: -3px;
                        width: 8px; height: 8px;
                        border-radius: 4px;
                        background: #FFA040;
                    }
                }
                Rectangle { height: 2px; }
                Text { text: "TANK ENERGY"; color: root.live-ink.with-alpha(0.65); font-family: "JetBrains Mono"; font-size: 11px; letter-spacing: 0.5px; }
                Rectangle {
                    height: 8px;
                    border-radius: 4px;
                    background: root.live-ink.with-alpha(0.1);
                    Rectangle {
                        x: 0px; y: 0px;
                        width: parent.width * root.starlab-tank-energy;
                        height: 100%;
                        border-radius: 4px;
                        background: root.starlab-texture-colors[root.starlab-texture-index];
                        animate width { duration: 120ms; }
                    }
                }

                }
            }

            // --- Warps: real carrier/modulator/output traces
            // overlaid (so how two signals combine into a third is
            // visible at a glance), or a real 12-band vocoder meter
            // when that algorithm slot is engaged. ---
            if !root.on-home && root.active-kind == 21 : InstrumentPanel {
                width: 306px;
                caption: "MODULATION / CROSSOVER";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                Text {
                    text: root.warps-algorithm-name + (root.warps-osc-enabled ? "  (osc: " + root.warps-osc-waveform-name + ")" : "") + (root.warps-vocoder-frozen ? "  frozen" : "");
                    color: root.accent;
                    font-family: "Space Grotesk";
                    font-weight: 700;
                    font-size: 16px;
                    wrap: word-wrap;
                    width: 260px;
                }
                Rectangle { height: 4px; }
                if !root.warps-is-vocoder : VerticalLayout {
                    spacing: 3px;
                    Text { text: "CARRIER / MODULATOR / OUTPUT"; color: root.live-ink.with-alpha(0.65); font-family: "JetBrains Mono"; font-size: 10px; letter-spacing: 0.5px; }
                    Rectangle {
                        vertical-stretch: 1;
                        border-width: 1px;
                        border-color: root.live-ink.with-alpha(0.1);
                        border-radius: 4px;
                        Rectangle {
                            y: parent.height / 2;
                            width: 100%; height: 1px;
                            background: root.live-ink.with-alpha(0.1);
                        }
                        for i in root.warps-carrier-mid-x.length : VectorSegment {
                        center-x: root.warps-carrier-mid-x[i] * 1px;
                        center-y: root.warps-carrier-mid-y[i] * 1px;
                        delta-x: root.warps-carrier-length[i] * cos(root.warps-carrier-angle[i] * 1deg);
                        delta-y: root.warps-carrier-length[i] * sin(root.warps-carrier-angle[i] * 1deg);
                        trace: #4090E0.with-alpha(0.55); line-width: 2px;
                    }
                        for i in root.warps-modulator-mid-x.length : VectorSegment {
                        center-x: root.warps-modulator-mid-x[i] * 1px;
                        center-y: root.warps-modulator-mid-y[i] * 1px;
                        delta-x: root.warps-modulator-length[i] * cos(root.warps-modulator-angle[i] * 1deg);
                        delta-y: root.warps-modulator-length[i] * sin(root.warps-modulator-angle[i] * 1deg);
                        trace: #FFA040.with-alpha(0.55); line-width: 2px;
                    }
                        for i in root.warps-output-mid-x.length : VectorSegment {
                        center-x: root.warps-output-mid-x[i] * 1px;
                        center-y: root.warps-output-mid-y[i] * 1px;
                        delta-x: root.warps-output-length[i] * cos(root.warps-output-angle[i] * 1deg);
                        delta-y: root.warps-output-length[i] * sin(root.warps-output-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.9); line-width: 2px;
                    }
                    }
                }
                if root.warps-is-vocoder : VerticalLayout {
                    spacing: 3px;
                    Text { text: "VOCODER BANDS"; color: root.live-ink.with-alpha(0.65); font-family: "JetBrains Mono"; font-size: 10px; letter-spacing: 0.5px; }
                    HorizontalLayout {
                        vertical-stretch: 1;
                        spacing: 3px;
                        for i in root.warps-vocoder-bands.length : Rectangle {
                            horizontal-stretch: 1;
                            Rectangle {
                                y: parent.height - self.height;
                                width: 100%;
                                height: max(2px, min(1.0, root.warps-vocoder-bands[i]) * parent.height);
                                border-radius: 2px;
                                background: root.accent.with-alpha(0.5 + min(1.0, root.warps-vocoder-bands[i]) * 0.5);
                                animate height { duration: 60ms; }
                            }
                        }
                    }
                }

                }
            }

            // --- Mixer: real per-channel strips -- fader position
            // (dim background bar) plus a live VU peak (bright
            // overlay, matched by name against audio_bus). ---
            if !root.on-home && root.active-kind == 22 : InstrumentPanel {
                width: 306px;
                caption: "MIX / CHANNEL LEVELS";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                Text {
                    text: "MASTER " + Math.round(root.mixer-master * 100) + "%";
                    color: root.accent;
                    font-family: "JetBrains Mono";
                    font-weight: 700;
                    font-size: 12px;
                }
                Rectangle { height: 4px; }
                Text {
                    text: "CHANNELS";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                HorizontalLayout {
                    vertical-stretch: 1;
                    spacing: 2px;
                    for i in root.mixer-fader.length : Rectangle {
                        horizontal-stretch: 1;
                        border-width: 1px;
                        border-color: root.live-ink.with-alpha(0.08);
                        // Fader position -- a dim full-height reference
                        // bar up to where the fader itself is set.
                        Rectangle {
                            x: 0px;
                            y: parent.height * (1.0 - min(1.0, root.mixer-fader[i] / 1.5));
                            width: 100%;
                            height: parent.height * min(1.0, root.mixer-fader[i] / 1.5);
                            background: root.accent.with-alpha(0.18);
                        }
                        // Live signal peak -- the bright real-time bar
                        // on top of it.
                        Rectangle {
                            x: 0px;
                            y: parent.height * (1.0 - min(1.0, root.mixer-live[i]));
                            width: 100%;
                            height: parent.height * min(1.0, root.mixer-live[i]);
                            background: root.accent.with-alpha(0.85);
                            animate height, y { duration: 40ms; }
                        }
                    }
                }

                }
            }

            // --- CV Out: real per-channel output levels (manual
            // offset + external modulation), as a compact 8x4 grid --
            // the same value actually sent as a MIDI CC. ---
            if !root.on-home && root.active-kind == 23 : InstrumentPanel {
                width: 306px;
                caption: "CONTROL / MIDI OUTPUT";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                Text {
                    text: "MIDI CHANNEL " + root.cv-out-midi-channel;
                    color: root.accent;
                    font-family: "JetBrains Mono";
                    font-weight: 700;
                    font-size: 12px;
                }
                Rectangle { height: 4px; }
                Text {
                    text: "32 OUTPUTS";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 11px;
                    letter-spacing: 0.5px;
                }
                Rectangle {
                    vertical-stretch: 1;
                    for i in root.cv-out-levels.length : Rectangle {
                        property <float> lvl: root.cv-out-levels[i];
                        x: mod(i, 8) * (parent.width / 8);
                        y: floor(i / 8) * (parent.height / 4);
                        width: parent.width / 8 - 4px;
                        height: parent.height / 4 - 4px;
                        border-radius: 3px;
                        border-width: 1px;
                        border-color: root.live-ink.with-alpha(0.1);
                        background: root.accent.with-alpha(0.12 + self.lvl * 0.75);
                        animate background { duration: 60ms; }
                        Text {
                            text: i + 1;
                            color: parent.lvl > 0.5 ? #0a0a0a : root.live-ink.with-alpha(0.4);
                            font-family: "JetBrains Mono";
                            font-size: 10px;
                            horizontal-alignment: center;
                            vertical-alignment: center;
                            width: 100%; height: 100%;
                        }
                    }
                }

                }
            }

            // --- Magnito: the real hysteresis-loop XY trace (input
            // vs. output across the tape-saturation follower) plus
            // real tape-character meters. ---
            if !root.on-home && root.active-kind == 24 : InstrumentPanel {
                width: 306px;
                caption: "TAPE / SATURATION";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 4px;
                HorizontalLayout {
                    spacing: 8px;
                    height: 16px;
                    Text {
                        text: "HYSTERESIS LOOP";
                        color: root.live-ink.with-alpha(0.65);
                        font-family: "JetBrains Mono";
                        font-size: 11px;
                        letter-spacing: 0.5px;
                    }
                    Rectangle { horizontal-stretch: 1; }
                    Text {
                        text: root.magnito-unlimited ? "UNLIMITED" : "bias " + Math.round(root.magnito-bias * 100) + "%";
                        color: root.magnito-unlimited ? #FF4D4D : root.live-ink.with-alpha(0.4);
                        font-family: "JetBrains Mono";
                        font-size: 10px;
                    }
                }
                HorizontalLayout {
                    alignment: center;
                    Rectangle {
                        width: 108px; height: 108px;
                        border-width: 1px;
                        border-color: root.live-ink.with-alpha(0.1);
                        border-radius: 4px;
                        // Reference diagonal -- what a perfectly clean,
                        // uncolored signal would trace.
                        VectorSegment {
                            center-x: parent.width / 2; center-y: parent.height / 2;
                            delta-x: parent.width / 1px - 4; delta-y: -parent.height / 1px + 4;
                            trace: root.live-ink.with-alpha(0.16); line-width: 1px;
                        }
                        for i in root.magnito-loop-mid-x.length : VectorSegment {
                        center-x: root.magnito-loop-mid-x[i] * (108px / 132);
                        center-y: root.magnito-loop-mid-y[i] * (108px / 132);
                        delta-x: root.magnito-loop-length[i] * (108.0 / 132.0) * cos(root.magnito-loop-angle[i] * 1deg);
                        delta-y: root.magnito-loop-length[i] * (108.0 / 132.0) * sin(root.magnito-loop-angle[i] * 1deg);
                        trace: root.magnito-unlimited ? #FF4D4D.with-alpha(0.8) : root.accent.with-alpha(0.8); line-width: 2px;
                    }
                    }
                }
                Rectangle { height: 2px; }
                Text {
                    text: "WOW / FLUTTER";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 10px;
                    letter-spacing: 0.5px;
                }
                Rectangle {
                    height: 46px;
                    border-width: 1px;
                    border-color: root.live-ink.with-alpha(0.1);
                    border-radius: 4px;
                    Rectangle {
                        y: parent.height / 2;
                        width: 100%; height: 1px;
                        background: root.live-ink.with-alpha(0.08);
                    }
                    for i in root.magnito-wobble-mid-x.length : VectorSegment {
                        center-x: root.magnito-wobble-mid-x[i] * 1px;
                        center-y: root.magnito-wobble-mid-y[i] * 1px;
                        delta-x: root.magnito-wobble-length[i] * cos(root.magnito-wobble-angle[i] * 1deg);
                        delta-y: root.magnito-wobble-length[i] * sin(root.magnito-wobble-angle[i] * 1deg);
                        trace: #FFA040.with-alpha(0.7); line-width: 2px;
                    }
                }
                Rectangle { height: 4px; }
                HorizontalLayout {
                    spacing: 10px;
                    height: 24px;
                    for m in [
                        { label: "WOW/FL", value: root.magnito-wow-flutter-depth },
                        { label: "HISS", value: root.magnito-hiss-level },
                        { label: "WEAR", value: root.magnito-wear },
                        { label: "OUT", value: root.magnito-output-peak },
                    ] : VerticalLayout {
                        spacing: 2px;
                        Rectangle {
                            height: 6px;
                            border-radius: 3px;
                            background: root.live-ink.with-alpha(0.1);
                            Rectangle {
                                x: 0px; y: 0px;
                                width: parent.width * min(1.0, m.value);
                                height: 100%;
                                border-radius: 3px;
                                background: root.accent.with-alpha(0.75);
                                animate width { duration: 80ms; }
                            }
                        }
                        Text {
                            text: m.label;
                            color: root.live-ink.with-alpha(0.65);
                            font-family: "JetBrains Mono";
                            font-size: 10px;
                        }
                    }
                    Rectangle {
                        width: 10px; height: 10px; y: 2px;
                        border-radius: 5px;
                        background: root.magnito-dropout ? #FF4D4D : root.live-ink.with-alpha(0.12);
                    }
                }

                }
            }

            // --- Settings: the real accent/background color wheel
            // (hue + saturation picked directly off it; brightness
            // stays a plain browsed row, like every other Settings
            // value) -- see `ThemeExtra`. ---
            if !root.on-home && root.active-kind == 25 : InstrumentPanel {
                width: 306px;
                caption: "SYSTEM / APPEARANCE";
                ink: root.live-ink; accent: root.accent;
                HorizontalLayout {
                spacing: 12px;
                alignment: start;
                padding-top: 4px;
                Rectangle {
                    width: 140px; height: 140px;
                    wheel-area := TouchArea {
                        width: 100%; height: 100%;
                        pointer-event(event) => {
                            if (event.kind == PointerEventKind.down) {
                                root.theme-wheel-picked(self.mouse-x / 1px - 70, self.mouse-y / 1px - 70);
                            }
                        }
                        moved => {
                            if (self.pressed) {
                                root.theme-wheel-picked(self.mouse-x / 1px - 70, self.mouse-y / 1px - 70);
                            }
                        }
                    }
                    Rectangle {
                        width: 100%; height: 100%;
                        border-radius: 70px;
                        background: @conic-gradient(#ff0000 0deg, #ffff00 60deg, #00ff00 120deg, #00ffff 180deg, #0000ff 240deg, #ff00ff 300deg, #ff0000 360deg);
                    }
                    Rectangle {
                        width: 100%; height: 100%;
                        border-radius: 70px;
                        background: @radial-gradient(circle, #ffffff 0%, root.live-ink.with-alpha(0) 75%);
                    }
                    Rectangle {
                        width: 100%; height: 100%;
                        border-radius: 70px;
                        border-width: 1px;
                        border-color: root.live-ink.with-alpha(0.15);
                    }
                    Rectangle {
                        x: 70px + root.theme-marker-x * 1px - 5px;
                        y: 70px + root.theme-marker-y * 1px - 5px;
                        width: 10px; height: 10px;
                        border-radius: 5px;
                        background: root.theme-editing-bg ? root.theme-bg-swatch : root.theme-accent-swatch;
                        border-width: 2px;
                        border-color: #ffffff;
                    }
                }
                VerticalLayout {
                    width: 124px;
                    spacing: 8px;
                    alignment: start;
                    Text {
                        text: (root.theme-editing-bg ? "BACKGROUND" : "ACCENT") + "\n" + root.theme-hex;
                        color: root.accent;
                        font-family: "JetBrains Mono";
                        font-weight: 700;
                        font-size: 12px;
                        wrap: word-wrap;
                    }
                    HorizontalLayout {
                        spacing: 6px;
                        height: 28px;
                        Rectangle {
                            width: 28px;
                            border-radius: 6px;
                            background: root.theme-accent-swatch;
                            border-width: root.theme-editing-bg ? 0px : 2px;
                            border-color: #ffffff;
                        }
                        Rectangle {
                            width: 28px;
                            border-radius: 6px;
                            background: root.theme-bg-swatch;
                            border-width: root.theme-editing-bg ? 2px : 0px;
                            border-color: #ffffff;
                        }
                    }
                    Text {
                        text: "click the wheel to set hue/\nsaturation, or browse Hue/\nSaturation/Brightness below";
                        color: root.live-ink.with-alpha(0.35);
                        font-family: "JetBrains Mono";
                        font-size: 10px;
                        wrap: word-wrap;
                    }
                }

                }
            }

            // --- Tonestack: the real post-chain output waveform +
            // level/gate telemetry (see `TonestackExtra`). ---
            if !root.on-home && root.active-kind == 26 : InstrumentPanel {
                width: 306px;
                caption: "TONE / SIGNAL CHAIN";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                HorizontalLayout {
                    spacing: 8px;
                    height: 16px;
                    Text {
                        text: root.tonestack-voicing-name;
                        color: root.accent;
                        font-family: "JetBrains Mono";
                        font-weight: 700;
                        font-size: 12px;
                    }
                    Rectangle { horizontal-stretch: 1; }
                    Rectangle {
                        width: 10px; height: 10px; y: 3px;
                        border-radius: 5px;
                        background: root.tonestack-gate-closed ? #FF4D4D : root.live-ink.with-alpha(0.12);
                    }
                    Text {
                        text: root.tonestack-gate-closed ? "GATED" : "open";
                        color: root.tonestack-gate-closed ? #FF4D4D : root.live-ink.with-alpha(0.4);
                        font-family: "JetBrains Mono";
                        font-size: 10px;
                    }
                }
                Rectangle {
                    height: 90px;
                    border-width: 1px;
                    border-color: root.live-ink.with-alpha(0.1);
                    border-radius: 4px;
                    Rectangle {
                        y: parent.height / 2;
                        width: 100%; height: 1px;
                        background: root.live-ink.with-alpha(0.08);
                    }
                    for i in root.tonestack-waveform-mid-x.length : VectorSegment {
                        center-x: root.tonestack-waveform-mid-x[i] * 1px;
                        center-y: root.tonestack-waveform-mid-y[i] * 1px;
                        delta-x: root.tonestack-waveform-length[i] * cos(root.tonestack-waveform-angle[i] * 1deg);
                        delta-y: root.tonestack-waveform-length[i] * sin(root.tonestack-waveform-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.85); line-width: 2px;
                    }
                }
                Rectangle { height: 2px; }
                Text {
                    text: "OUTPUT";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 10px;
                    letter-spacing: 0.5px;
                }
                Rectangle {
                    height: 8px;
                    border-radius: 4px;
                    background: root.live-ink.with-alpha(0.1);
                    Rectangle {
                        x: 0px; y: 0px;
                        width: parent.width * min(1.0, root.tonestack-output-peak);
                        height: 100%;
                        border-radius: 4px;
                        background: root.tonestack-output-peak > 0.95 ? #FF4D4D : root.accent;
                        animate width { duration: 60ms; }
                    }
                }

                }
            }

            // --- Sample Drum: the real per-channel output waveform
            // + slice position (see `SampleDrumExtra`). ---
            if !root.on-home && root.active-kind == 27 : InstrumentPanel {
                width: 306px;
                caption: "SAMPLE / SLICE ENGINE";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                HorizontalLayout {
                    spacing: 8px;
                    height: 16px;
                    Text {
                        text: "CH" + (root.sample-drum-channel + 1);
                        color: root.accent;
                        font-family: "JetBrains Mono";
                        font-weight: 700;
                        font-size: 12px;
                    }
                    Text {
                        text: root.sample-drum-sample-name;
                        color: root.live-ink.with-alpha(0.6);
                        font-family: "JetBrains Mono";
                        font-size: 11px;
                        vertical-alignment: center;
                    }
                    Rectangle { horizontal-stretch: 1; }
                    Text {
                        text: "slice " + (root.sample-drum-step-index + 1) + "/" + root.sample-drum-num-slices;
                        color: root.live-ink.with-alpha(0.65);
                        font-family: "JetBrains Mono";
                        font-size: 10px;
                    }
                }
                Rectangle {
                    height: 88px;
                    border-width: 1px;
                    border-color: root.live-ink.with-alpha(0.1);
                    border-radius: 4px;
                    Rectangle {
                        y: parent.height / 2;
                        width: 100%; height: 1px;
                        background: root.live-ink.with-alpha(0.08);
                    }
                    for i in root.sample-drum-waveform-mid-x.length : VectorSegment {
                        center-x: root.sample-drum-waveform-mid-x[i] * 1px;
                        center-y: root.sample-drum-waveform-mid-y[i] * 1px;
                        delta-x: root.sample-drum-waveform-length[i] * cos(root.sample-drum-waveform-angle[i] * 1deg);
                        delta-y: root.sample-drum-waveform-length[i] * sin(root.sample-drum-waveform-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.85); line-width: 2px;
                    }
                }
                Text {
                    text: "SOURCE / SLICE BOUNDARIES";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 10px;
                }

                // The source sample with its slice cut points marked
                // -- "designate when the cuts are going to be,
                // visually" (see `SampleDrumExtra::slice_bounds`).
                // The slice the next trigger will fire is highlighted.
                Rectangle {
                    height: 60px;
                    border-width: 1px;
                    border-color: root.live-ink.with-alpha(0.1);
                    border-radius: 4px;
                    clip: true;
                    if root.sample-drum-slice-lowers.length > root.sample-drum-step-index && root.sample-drum-step-index >= 0 : Rectangle {
                        x: root.sample-drum-slice-lowers[root.sample-drum-step-index] * parent.width;
                        width: (root.sample-drum-slice-uppers[root.sample-drum-step-index] - root.sample-drum-slice-lowers[root.sample-drum-step-index]) * parent.width;
                        height: 100%;
                        background: root.accent.with-alpha(0.18);
                    }
                    Rectangle {
                        y: parent.height / 2;
                        width: 100%; height: 1px;
                        background: root.live-ink.with-alpha(0.08);
                    }
                    for peak[i] in root.sample-drum-source-waveform : Rectangle {
                        x: i * (parent.width / root.sample-drum-source-waveform.length);
                        y: parent.height / 2 - (peak * parent.height / 2);
                        width: (parent.width / root.sample-drum-source-waveform.length) + 1px;
                        height: peak * parent.height;
                        background: root.live-ink.with-alpha(0.55);
                    }
                    for lower[i] in root.sample-drum-slice-lowers : Rectangle {
                        x: lower * parent.width;
                        width: 1px;
                        height: 100%;
                        background: root.accent.with-alpha(0.7);
                    }
                }
                Text {
                    text: "PAD 1 / CH1  ·  PAD 2 / CH2";
                    color: root.live-ink.with-alpha(0.65);
                    font-family: "JetBrains Mono";
                    font-size: 10px;
                }

                }
            }

            // --- Visualizer: the real analyzer readout, or one of
            // the two audio-reactive pixel-art scenes (see
            // `VisualizerExtra`). ---
            if !root.on-home && root.active-kind == 28 : InstrumentPanel {
                width: 306px;
                caption: "SIGNAL / VISUALIZER";
                ink: root.live-ink; accent: root.accent;
                VerticalLayout {
                spacing: 6px;
                HorizontalLayout {
                    spacing: 8px;
                    height: 16px;
                    Text {
                        text: root.visualizer-scene-kind == 0 ? root.visualizer-mode-name : root.visualizer-scene-name;
                        color: root.accent;
                        font-family: "JetBrains Mono";
                        font-weight: 700;
                        font-size: 12px;
                    }
                    Rectangle { horizontal-stretch: 1; }
                    Rectangle {
                        width: 8px; height: 8px; y: 4px;
                        border-radius: 4px;
                        background: root.visualizer-monitor-on ? root.accent : root.live-ink.with-alpha(0.15);
                    }
                    Text {
                        text: root.visualizer-monitor-on ? "MON" : "muted";
                        color: root.visualizer-monitor-on ? root.accent : root.live-ink.with-alpha(0.3);
                        font-family: "JetBrains Mono";
                        font-size: 10px;
                    }
                }
                Rectangle {
                    height: 130px;
                    border-width: 1px;
                    border-color: root.live-ink.with-alpha(0.1);
                    border-radius: 4px;
                    clip: true;

                    if root.visualizer-scene-kind == 0 && root.visualizer-mode-kind == 0 : HorizontalLayout {
                        x: 4px; y: 4px;
                        width: parent.width - 8px; height: parent.height - 8px;
                        spacing: 2px;
                        for level in root.visualizer-spectrum : Rectangle {
                            horizontal-stretch: 1;
                            Rectangle {
                                y: parent.height - self.height;
                                width: 100%;
                                height: max(level, 0.01) * parent.height;
                                background: root.accent;
                            }
                        }
                    }

                    if root.visualizer-scene-kind == 0 && root.visualizer-mode-kind == 1 : Rectangle {
                        Rectangle {
                            y: parent.height / 2;
                            width: 100%; height: 1px;
                            background: root.live-ink.with-alpha(0.08);
                        }
                        for i in root.visualizer-waveform-mid-x.length : VectorSegment {
                        center-x: root.visualizer-waveform-mid-x[i] * 1px;
                        center-y: root.visualizer-waveform-mid-y[i] * 1px;
                        delta-x: root.visualizer-waveform-length[i] * cos(root.visualizer-waveform-angle[i] * 1deg);
                        delta-y: root.visualizer-waveform-length[i] * sin(root.visualizer-waveform-angle[i] * 1deg);
                        trace: root.accent.with-alpha(0.85); line-width: 2px;
                    }
                    }

                    if root.visualizer-scene-kind == 0 && root.visualizer-mode-kind == 3 : VerticalLayout {
                        x: 10px; y: 10px;
                        width: parent.width - 20px; height: parent.height - 20px;
                        spacing: 14px;
                        for row in [{ label: "PEAK", level: root.visualizer-peak-level }, { label: "RMS", level: root.visualizer-rms-level }] : VerticalLayout {
                            spacing: 4px;
                            Text {
                                text: row.label;
                                color: root.live-ink.with-alpha(0.65);
                                font-family: "JetBrains Mono";
                                font-size: 10px;
                            }
                            Rectangle {
                                height: 16px;
                                border-width: 1px;
                                border-color: root.live-ink.with-alpha(0.15);
                                Rectangle {
                                    x: 0px; y: 0px;
                                    width: parent.width * row.level;
                                    height: 100%;
                                    background: root.accent;
                                }
                            }
                        }
                    }

                    if root.visualizer-scene-kind == 0 && root.visualizer-mode-kind == 4 : VerticalLayout {
                        alignment: center;
                        Text {
                            horizontal-alignment: center;
                            text: root.visualizer-pitch-name;
                            color: root.accent;
                            font-family: "JetBrains Mono";
                            font-weight: 700;
                            font-size: 28px;
                        }
                    }

                    if root.visualizer-scene-kind == 0 && root.visualizer-mode-kind == 2 : VerticalLayout {
                        alignment: center;
                        Text {
                            horizontal-alignment: center;
                            text: "(spectrogram not rendered here)";
                            color: root.live-ink.with-alpha(0.65);
                            font-family: "JetBrains Mono";
                            font-size: 10px;
                        }
                    }

                    // Boxer -- bobs on bass energy, jabs both gloves
                    // outward on a beat hit.
                    if root.visualizer-scene-kind == 1 : Rectangle {
                        x: 0px; y: 0px; width: 100%; height: 100%;
                        property <length> bob: root.visualizer-bass-level * 6px;
                        property <length> punch: root.visualizer-beat-pulse * 26px;
                        Rectangle {
                            y: parent.height - 10px;
                            width: 100%; height: 1px;
                            background: root.live-ink.with-alpha(0.1);
                        }
                        Rectangle {
                            x: parent.width / 2 - 16px; y: parent.height - 44px - parent.bob;
                            width: 32px; height: 36px;
                            background: root.accent;
                            border-radius: 4px;
                        }
                        Rectangle {
                            x: parent.width / 2 - 9px; y: parent.height - 62px - parent.bob;
                            width: 18px; height: 18px;
                            background: root.accent;
                            border-radius: 4px;
                        }
                        Rectangle {
                            x: parent.width / 2 - 26px - parent.punch; y: parent.height - 48px - parent.bob;
                            width: 14px; height: 14px;
                            background: white;
                            border-radius: 7px;
                        }
                        Rectangle {
                            x: parent.width / 2 + 12px + parent.punch; y: parent.height - 48px - parent.bob;
                            width: 14px; height: 14px;
                            background: white;
                            border-radius: 7px;
                        }
                    }

                    // Car -- drives left to right on a loop, hops on
                    // a beat, headlight brightens with treble energy.
                    if root.visualizer-scene-kind == 2 : Rectangle {
                        x: 0px; y: 0px; width: 100%; height: 100%;
                        property <length> car-x-px: root.visualizer-car-x * (parent.width - 70px);
                        property <length> car-y: parent.height - 42px - root.visualizer-beat-pulse * 8px;
                        Rectangle {
                            y: parent.height - 10px;
                            width: 100%; height: 1px;
                            background: root.live-ink.with-alpha(0.1);
                        }
                        Rectangle {
                            x: parent.car-x-px; y: parent.car-y;
                            width: 70px; height: 18px;
                            background: root.accent;
                            border-radius: 6px;
                        }
                        Rectangle {
                            x: parent.car-x-px + 14px; y: parent.car-y - 9px;
                            width: 40px; height: 10px;
                            background: root.accent;
                            border-radius: 4px;
                        }
                        Rectangle {
                            x: parent.car-x-px + 64px; y: parent.car-y + 5px;
                            width: 6px; height: 6px;
                            background: white;
                            opacity: 0.35 + root.visualizer-treble-level * 0.65;
                            border-radius: 3px;
                        }
                    }

                    // Cat -- a maneki-neko DJ: head/ears nod on the
                    // beat, its raised paw swings side to side with
                    // every beat onset, boombox shows a tiny live
                    // spectrum readout.
                    if root.visualizer-scene-kind == 3 : Rectangle {
                        x: 0px; y: 0px; width: 100%; height: 100%;
                        property <length> bob: root.visualizer-beat-pulse * 5px;
                        property <length> paw-x: parent.width / 2 - 40px + (root.visualizer-cat-paw-left ? -1 : 1) * root.visualizer-beat-pulse * 14px;
                        Rectangle {
                            y: parent.height - 10px;
                            width: 100%; height: 1px;
                            background: root.live-ink.with-alpha(0.1);
                        }
                        // Body + head.
                        Rectangle {
                            x: parent.width / 2 - 26px; y: parent.height - 66px;
                            width: 52px; height: 50px;
                            background: white;
                            border-radius: 4px;
                        }
                        Rectangle {
                            x: parent.width / 2 - 22px; y: parent.height - 100px - parent.bob;
                            width: 44px; height: 38px;
                            background: white;
                            border-radius: 4px;
                        }
                        // Headphones.
                        for ear_x[i] in [parent.width / 2 - 30px, parent.width / 2 + 12px] : Rectangle {
                            x: ear_x; y: parent.height - 108px - parent.bob;
                            width: 18px; height: 18px;
                            background: black;
                            border-radius: 9px;
                            Rectangle {
                                x: 3px; y: 3px; width: 12px; height: 12px;
                                background: #cc1a10;
                                border-radius: 6px;
                            }
                        }
                        // Eyes + bell/collar.
                        for eye_x in [parent.width / 2 - 12px, parent.width / 2 + 6px] : Rectangle {
                            x: eye_x; y: parent.height - 84px - parent.bob;
                            width: 5px; height: 7px;
                            background: black;
                        }
                        Rectangle {
                            x: parent.width / 2 - 26px; y: parent.height - 56px;
                            width: 52px; height: 5px;
                            background: #cc1a10;
                        }
                        Rectangle {
                            x: parent.width / 2 - 5px; y: parent.height - 54px;
                            width: 9px; height: 9px;
                            background: #d6ae3a;
                            border-radius: 4px;
                        }
                        // Raised, swinging paw.
                        Rectangle {
                            x: parent.paw-x; y: parent.height - 108px - parent.bob;
                            width: 11px; height: 22px;
                            background: white;
                            border-radius: 2px;
                            Rectangle {
                                width: 100%; height: 5px;
                                background: #cc1a10;
                            }
                        }
                        // Boombox + tiny spectrum readout.
                        Rectangle {
                            x: parent.width / 2 - 40px; y: parent.height - 34px;
                            width: 80px; height: 24px;
                            background: #0a140a;
                            border-radius: 3px;
                        }
                        HorizontalLayout {
                            x: parent.width / 2 - 38px; y: parent.height - 46px;
                            width: 76px; height: 12px;
                            spacing: 2px;
                            for level in root.visualizer-spectrum : Rectangle {
                                horizontal-stretch: 1;
                                Rectangle {
                                    y: parent.height - self.height;
                                    width: 100%;
                                    height: max(level, 0.05) * parent.height;
                                    background: root.accent;
                                }
                            }
                        }
                    }
                }

                }
            }

            if !root.on-home && root.active-kind == 29 : BloomPanel {
                row-names: root.row-names;
                row-values: root.row-values;
                row-is-group: root.row-is-group;
                selected-row: root.selected-row;
                more-above: root.more-above;
                more-below: root.more-below;
                running: root.bloom-running;
                shape-index: root.bloom-shape-index;
                pattern-name: root.bloom-pattern-name;
                outer-x: root.bloom-outer-x;
                outer-y: root.bloom-outer-y;
                outer-active: root.bloom-outer-active;
                outer-lit: root.bloom-outer-lit;
                outer-flash-angle: root.bloom-outer-flash-angle;
                inner-x: root.bloom-inner-x;
                inner-y: root.bloom-inner-y;
                inner-lit: root.bloom-inner-lit;
                inner-frac: root.bloom-inner-frac;
                line-mid-x: root.bloom-line-mid-x;
                line-mid-y: root.bloom-line-mid-y;
                line-length: root.bloom-line-length;
                line-angle: root.bloom-line-angle;
                toggle-running => { root.f-clicked(2); }
            }
        }

        }

        // Cover the complete display, including the title and footer. The
        // launcher is not instantiated during boot, so no rows can bleed out.
        if root.splash-active : Rectangle {
            y: -40px; width: 640px; height: 360px;
            background: root.screen-bg;
            if root.splash-is-szyk : Image {
                source: @image-url("../assets/svg_src/szyk_logo.svg");
                colorize: #ffffff;
                x: 24px; y: 20px; width: 592px; height: 320px;
                image-fit: contain;
            }
            if !root.splash-is-szyk : Image {
                source: root.splash-image;
                width: 100%; height: 100%;
                image-fit: contain;
            }
        }

        knob1-dragged(delta) => { root.knob1-delta += delta; }
        knob2-dragged(delta) => { root.knob2-delta += delta; }
    }
}

/// Which boot stage is currently showing -- see `startup_logo.rs`.
/// Own copy of `os.rs`'s `SplashStage` (that one's private to `Os`,
/// and this example doesn't otherwise depend on `os.rs` at all).
#[derive(Clone, Copy, PartialEq)]
enum SplashStage {
    Szyk,
    Mx1,
}

/// Rasterizes a `startup_logo::Logo` (a pre-thresholded 1bpp bitmap,
/// drawn via `embedded_graphics` onto the same `FrameBuffer` the real
/// device's screen uses) into a `slint::Image` -- lets the boot
/// sequence reuse the exact same asset both binaries already share,
/// instead of needing a second, Slint-native copy of the logos.
/// Unlit pixels become fully transparent (not opaque black), so this
/// composites cleanly over whatever `screen-bg` is live at the time.
fn logo_to_slint_image(logo: &startup_logo::Logo) -> slint::Image {
    let mut fb = display::FrameBuffer::new();
    logo.draw_centered(&mut fb, embedded_graphics::pixelcolor::Rgb565::new(31, 63, 31));
    let mut pixel_buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(display::WIDTH as u32, display::HEIGHT as u32);
    let slice = pixel_buffer.make_mut_slice();
    for (dst, &packed) in slice.iter_mut().zip(fb.buffer().iter()) {
        let r = ((packed >> 16) & 0xFF) as u8;
        let g = ((packed >> 8) & 0xFF) as u8;
        let b = (packed & 0xFF) as u8;
        let a = if packed != 0 { 255 } else { 0 };
        *dst = slint::Rgba8Pixel { r, g, b, a };
    }
    slint::Image::from_rgba8(pixel_buffer)
}

/// One redesigned app's fixed (bg, ink/title, accent, dim) palette --
/// The four instrument redesigns have Slint-specific palettes; other
/// entries retain the values from `draw()` in `src/apps/
/// *.rs` (converted from its Rgb565 5/6/5 consts), so this Slint
/// dev-preview tool and the real device screen read as one design
/// per app, not two. Keyed by the app's manifest `name` (what
/// `apps_ref`'s own display name already is), not its id -- avoids
/// needing a second id lookup just for this.
///
/// Deliberately excludes Settings -- while inside Settings' own
/// screen, `live-accent`/`live-bg`/`live-ink` keep following the live
/// `ThemeColor` wheel it's editing (see the timer's own branch),
/// since overriding them with a fixed palette would defeat the whole
/// point of that screen. Every *other* app now has its own fixed
/// identity that the wheel no longer touches -- see this file's own
/// `live-accent`/`live-bg` doc comment for the "menu only" scoping
/// this enables.
fn app_palette(name: &str) -> Option<(slint::Color, slint::Color, slint::Color, slint::Color)> {
    fn c(hex: u32) -> slint::Color {
        slint::Color::from_rgb_u8(((hex >> 16) & 0xFF) as u8, ((hex >> 8) & 0xFF) as u8, (hex & 0xFF) as u8)
    }
    // (bg, ink/title, accent, dim)
    let (bg, ink, accent, dim): (u32, u32, u32, u32) = match name {
        "Analyzer" => (0x101b22, 0xe5eff3, 0x75dcd3, 0x81959e),
        "Synth" => (0x151c21, 0xe5eff3, 0x8adbc4, 0x81959e),
        "Settings" => (0x14191f, 0xe7edf4, 0xa5bce9, 0x8994aa),
        "Bloom" => (0x0c1918, 0xe7edda, 0xd8f580, 0x203b33),
        "Visualizer" => (0x151b25, 0xebedf3, 0xf3b980, 0x8994aa),
        "Nebula" => (0x100821, 0xe6dbef, 0xa58af7, 0x8c75a5),
        "Black Hole" => (0x0b1021, 0xe0e6ff, 0x9e93ff, 0x687399),
        "Magnito" => (0x211810, 0xfff3e6, 0xef9a71, 0xa48a77),
        "Rainmaker" => (0x101821, 0xd6e7ef, 0x73aac5, 0x52697b),
        "Warps" => (0x080808, 0xffe3de, 0xff2d52, 0x7b4952),
        "Tonestack" => (0xeee6d3, 0x3a2d21, 0x995020, 0x80715b),
        "Sample Drum" => (0x211c19, 0xefebe6, 0xff6919, 0x7b716b),
        "Starlab" | "StarLab" => (0x080c21, 0xd6e7ff, 0x8cbeff, 0x4a4d6b),
        "Beads" => (0x251d19, 0xf4e4cf, 0xeda76b, 0xa68c74),
        "MIDI Learn" => (0x191821, 0xeff3ef, 0xb5d2ff, 0x7b8e9c),
        "CV Out" => (0x191c21, 0xefefef, 0xffaa00, 0x6b7d84),
        "Natural Gate" => (0x191410, 0xefdbbd, 0xe4b569, 0xa68e6e),
        "Nautilus" => (0x001019, 0xdef7ef, 0x29dfc5, 0x3a696b),
        "Queen of Pentacles" => (0x081810, 0xe6ca7b, 0xd6ae3a, 0x4a6952),
        "Singularity" => (0x000400, 0xe6ffce, 0xbdff29, 0x5a793a),
        "Mixer" => (0x101820, 0xe2edf5, 0x76cddd, 0x81949f),
        "Cascade" => (0x081019, 0xe6f3ff, 0x7bdbff, 0x5a697b),
        "Clouds" => (0xe0e9ee, 0x19283a, 0x326c97, 0x627b8a),
        "Morph" => (0x101e24, 0xeaf3ec, 0x9fe7c3, 0xf8b489),
        "Madness" => (0x080408, 0xffe3f7, 0xff2d94, 0x7b4963),
        "Pam's Workout" => (0x151715, 0xf2e9ce, 0xf6bd46, 0x8e805e),
        "Prism" => (0x080410, 0xefe3ff, 0xb565ff, 0x6b597b),
        "Sequencer" => (0x080408, 0xffe3de, 0xff393a, 0x7b5152),
        "Turing Machine" => (0xebe8de, 0x232e3a, 0x286c9c, 0x657887),
        "Voltage" => (0x101819, 0xdef3ef, 0x4acea5, 0x4a6963),
        "Plaits" => (0xf0eee5, 0x252d30, 0x087c77, 0x647370),
        "Tape" => (0x201b19, 0xf5e6d6, 0xfa986d, 0xa18b7e),
        _ => return None,
    };
    Some((c(bg), c(ink), c(accent), c(dim)))
}

fn apply_instrument_visual(ui: &LiveHomeScreen, extra: app::SlintExtra) {
            match extra {
                app::SlintExtra::Plaits(p) => {
                    ui.set_active_kind(1);
                    ui.set_plaits_engine_name(p.engine_name.into());
                    ui.set_plaits_engine_bank(p.engine_bank as i32);
                    ui.set_plaits_engine_led(p.engine_led as i32);
                    ui.set_plaits_analyzer_kind(p.analyzer_kind as i32);
                    ui.set_plaits_analyzer_name(p.analyzer_name.into());
                    ui.set_plaits_spectrum(Rc::new(slint::VecModel::from(p.spectrum)).into());
                    ui.set_plaits_waveform_mid_x(Rc::new(slint::VecModel::from(p.waveform.mid_x)).into());
                    ui.set_plaits_waveform_mid_y(Rc::new(slint::VecModel::from(p.waveform.mid_y)).into());
                    ui.set_plaits_waveform_length(Rc::new(slint::VecModel::from(p.waveform.length)).into());
                    ui.set_plaits_waveform_angle(Rc::new(slint::VecModel::from(p.waveform.angle_deg)).into());
                    ui.set_plaits_peak_level(p.peak_level);
                    ui.set_plaits_rms_level(p.rms_level);
                    ui.set_plaits_pitch_name(p.pitch_name.into());
                }
                app::SlintExtra::Voltage(v) => {
                    ui.set_active_kind(2);
                    ui.set_voltage_oscillator_mid_x(Rc::new(slint::VecModel::from(v.oscillator.mid_x)).into());
                    ui.set_voltage_oscillator_mid_y(Rc::new(slint::VecModel::from(v.oscillator.mid_y)).into());
                    ui.set_voltage_oscillator_length(Rc::new(slint::VecModel::from(v.oscillator.length)).into());
                    ui.set_voltage_oscillator_angle(Rc::new(slint::VecModel::from(v.oscillator.angle_deg)).into());
                    ui.set_voltage_filter_mid_x(Rc::new(slint::VecModel::from(v.filter.mid_x)).into());
                    ui.set_voltage_filter_mid_y(Rc::new(slint::VecModel::from(v.filter.mid_y)).into());
                    ui.set_voltage_filter_length(Rc::new(slint::VecModel::from(v.filter.length)).into());
                    ui.set_voltage_filter_angle(Rc::new(slint::VecModel::from(v.filter.angle_deg)).into());
                    ui.set_voltage_filter_cutoff_frac(v.filter_cutoff_frac);
                    ui.set_voltage_filter_has_swept(v.filter_swept_frac.is_some());
                    ui.set_voltage_filter_swept_frac(v.filter_swept_frac.unwrap_or(0.0));
                    ui.set_voltage_amp_env_mid_x(Rc::new(slint::VecModel::from(v.amp_env.mid_x)).into());
                    ui.set_voltage_amp_env_mid_y(Rc::new(slint::VecModel::from(v.amp_env.mid_y)).into());
                    ui.set_voltage_amp_env_length(Rc::new(slint::VecModel::from(v.amp_env.length)).into());
                    ui.set_voltage_amp_env_angle(Rc::new(slint::VecModel::from(v.amp_env.angle_deg)).into());
                    ui.set_voltage_lfo_mid_x(Rc::new(slint::VecModel::from(v.lfo.mid_x)).into());
                    ui.set_voltage_lfo_mid_y(Rc::new(slint::VecModel::from(v.lfo.mid_y)).into());
                    ui.set_voltage_lfo_length(Rc::new(slint::VecModel::from(v.lfo.length)).into());
                    ui.set_voltage_lfo_angle(Rc::new(slint::VecModel::from(v.lfo.angle_deg)).into());
                }
                app::SlintExtra::Analyzer(a) => {
                    ui.set_active_kind(3);
                    ui.set_analyzer_kind(a.analyzer_kind as i32);
                    ui.set_analyzer_name(a.analyzer_name.into());
                    ui.set_analyzer_spectrum(Rc::new(slint::VecModel::from(a.spectrum)).into());
                    ui.set_analyzer_waveform_mid_x(Rc::new(slint::VecModel::from(a.waveform.mid_x)).into());
                    ui.set_analyzer_waveform_mid_y(Rc::new(slint::VecModel::from(a.waveform.mid_y)).into());
                    ui.set_analyzer_waveform_length(Rc::new(slint::VecModel::from(a.waveform.length)).into());
                    ui.set_analyzer_waveform_angle(Rc::new(slint::VecModel::from(a.waveform.angle_deg)).into());
                    ui.set_analyzer_peak_level(a.peak_level);
                    ui.set_analyzer_rms_level(a.rms_level);
                    ui.set_analyzer_pitch_name(a.pitch_name.into());
                }
                app::SlintExtra::Cascade(c) => {
                    ui.set_active_kind(4);
                    ui.set_cascade_algorithm_name(c.algorithm_name.into());
                    ui.set_cascade_carriers(Rc::new(slint::VecModel::from(c.carriers)).into());
                    ui.set_cascade_connection_mid_x(Rc::new(slint::VecModel::from(c.connection_lines.mid_x)).into());
                    ui.set_cascade_connection_mid_y(Rc::new(slint::VecModel::from(c.connection_lines.mid_y)).into());
                    ui.set_cascade_connection_length(Rc::new(slint::VecModel::from(c.connection_lines.length)).into());
                    ui.set_cascade_connection_angle(Rc::new(slint::VecModel::from(c.connection_lines.angle_deg)).into());
                    ui.set_cascade_feedback_op(c.feedback_op as i32);
                }
                app::SlintExtra::Bloom(b) => {
                    ui.set_active_kind(29);
                    ui.set_bloom_running(b.running);
                    ui.set_bloom_shape_index(b.shape_index as i32);
                    ui.set_bloom_pattern_name(b.pattern_name.into());
                    let outer_x: Vec<f32> = b.outer.iter().map(|(x, _, _, _)| *x).collect();
                    let outer_y: Vec<f32> = b.outer.iter().map(|(_, y, _, _)| *y).collect();
                    let outer_active: Vec<bool> = b.outer.iter().map(|(_, _, a, _)| *a).collect();
                    let outer_lit: Vec<bool> = b.outer.iter().map(|(_, _, _, l)| *l).collect();
                    ui.set_bloom_outer_x(Rc::new(slint::VecModel::from(outer_x)).into());
                    ui.set_bloom_outer_y(Rc::new(slint::VecModel::from(outer_y)).into());
                    ui.set_bloom_outer_active(Rc::new(slint::VecModel::from(outer_active)).into());
                    ui.set_bloom_outer_lit(Rc::new(slint::VecModel::from(outer_lit.clone())).into());
                    let inner_x: Vec<f32> = b.inner.iter().map(|(x, _, _, _)| *x).collect();
                    let inner_y: Vec<f32> = b.inner.iter().map(|(_, y, _, _)| *y).collect();
                    let inner_lit: Vec<bool> = b.inner.iter().map(|(_, _, l, _)| *l).collect();
                    let inner_frac: Vec<f32> = b.inner.iter().map(|(_, _, _, f)| *f).collect();
                    ui.set_bloom_inner_x(Rc::new(slint::VecModel::from(inner_x)).into());
                    ui.set_bloom_inner_y(Rc::new(slint::VecModel::from(inner_y)).into());
                    ui.set_bloom_inner_lit(Rc::new(slint::VecModel::from(inner_lit)).into());
                    ui.set_bloom_inner_frac(Rc::new(slint::VecModel::from(inner_frac)).into());

                    // The connecting spiral -- Bloom's own dots are
                    // never a closed ring (each is on its own ring),
                    // so no closing segment, unlike the shared `Shape`
                    // arm below which also handles Madness's ring.
                    let mut line_mid_x = Vec::new();
                    let mut line_mid_y = Vec::new();
                    let mut line_len = Vec::new();
                    let mut line_angle = Vec::new();
                    for w in b.inner.windows(2) {
                        let (x0, y0) = (w[0].0, w[0].1);
                        let (x1, y1) = (w[1].0, w[1].1);
                        let dx = x1 - x0;
                        let dy = y1 - y0;
                        line_mid_x.push((x0 + x1) / 2.0);
                        line_mid_y.push((y0 + y1) / 2.0);
                        line_len.push((dx * dx + dy * dy).sqrt());
                        line_angle.push(dy.atan2(dx).to_degrees());
                    }
                    ui.set_bloom_line_mid_x(Rc::new(slint::VecModel::from(line_mid_x)).into());
                    ui.set_bloom_line_mid_y(Rc::new(slint::VecModel::from(line_mid_y)).into());
                    ui.set_bloom_line_length(Rc::new(slint::VecModel::from(line_len)).into());
                    ui.set_bloom_line_angle(Rc::new(slint::VecModel::from(line_angle)).into());

                    let outer_flash_angle: Vec<f32> = b.outer.iter().map(|(x, y, _, _)| y.atan2(*x).to_degrees()).collect();
                    ui.set_bloom_outer_flash_angle(Rc::new(slint::VecModel::from(outer_flash_angle)).into());
                    ui.set_bloom_outer_flash_active(Rc::new(slint::VecModel::from(outer_lit)).into());
                }
                app::SlintExtra::Shape(s) => {
                    ui.set_active_kind(5);
                    ui.set_shape_running(s.running);
                    ui.set_shape_closed(s.closed);
                    let outer_x: Vec<f32> = s.outer.iter().map(|(x, _, _, _)| *x).collect();
                    let outer_y: Vec<f32> = s.outer.iter().map(|(_, y, _, _)| *y).collect();
                    let outer_active: Vec<bool> = s.outer.iter().map(|(_, _, a, _)| *a).collect();
                    let outer_lit: Vec<bool> = s.outer.iter().map(|(_, _, _, l)| *l).collect();
                    ui.set_shape_outer_x(Rc::new(slint::VecModel::from(outer_x)).into());
                    ui.set_shape_outer_y(Rc::new(slint::VecModel::from(outer_y)).into());
                    ui.set_shape_outer_active(Rc::new(slint::VecModel::from(outer_active)).into());
                    ui.set_shape_outer_lit(Rc::new(slint::VecModel::from(outer_lit.clone())).into());
                    let inner_x: Vec<f32> = s.inner.iter().map(|(x, _, _)| *x).collect();
                    let inner_y: Vec<f32> = s.inner.iter().map(|(_, y, _)| *y).collect();
                    let inner_lit: Vec<bool> = s.inner.iter().map(|(_, _, l)| *l).collect();
                    ui.set_shape_inner_x(Rc::new(slint::VecModel::from(inner_x)).into());
                    ui.set_shape_inner_y(Rc::new(slint::VecModel::from(inner_y)).into());
                    ui.set_shape_inner_lit(Rc::new(slint::VecModel::from(inner_lit)).into());

                    // The connecting web -- consecutive inner dots,
                    // plus the closing segment for Madness's ring (see
                    // `s.closed`). Same "fraction of radius, angle in
                    // degrees" derivation every other rotated
                    // line-segment visual in this file uses, just
                    // computed from the already-normalized -1..1
                    // `s.inner` points instead of a fixed pixel panel.
                    let mut line_mid_x = Vec::new();
                    let mut line_mid_y = Vec::new();
                    let mut line_len = Vec::new();
                    let mut line_angle = Vec::new();
                    let mut push_segment = |(x0, y0): (f32, f32), (x1, y1): (f32, f32)| {
                        let dx = x1 - x0;
                        let dy = y1 - y0;
                        line_mid_x.push((x0 + x1) / 2.0);
                        line_mid_y.push((y0 + y1) / 2.0);
                        line_len.push((dx * dx + dy * dy).sqrt());
                        line_angle.push(dy.atan2(dx).to_degrees());
                    };
                    for w in s.inner.windows(2) {
                        push_segment((w[0].0, w[0].1), (w[1].0, w[1].1));
                    }
                    if s.closed && s.inner.len() >= 2 {
                        let last = s.inner[s.inner.len() - 1];
                        let first = s.inner[0];
                        push_segment((last.0, last.1), (first.0, first.1));
                    }
                    ui.set_shape_inner_line_mid_x(Rc::new(slint::VecModel::from(line_mid_x)).into());
                    ui.set_shape_inner_line_mid_y(Rc::new(slint::VecModel::from(line_mid_y)).into());
                    ui.set_shape_inner_line_length(Rc::new(slint::VecModel::from(line_len)).into());
                    ui.set_shape_inner_line_angle(Rc::new(slint::VecModel::from(line_angle)).into());

                    // One flash line per lit outer dot, center to that
                    // dot -- always length 1.0 (outer dots sit exactly
                    // on the boundary ring), just the angle varies.
                    let outer_flash_angle: Vec<f32> = s.outer.iter().map(|(x, y, _, _)| y.atan2(*x).to_degrees()).collect();
                    ui.set_shape_outer_flash_angle(Rc::new(slint::VecModel::from(outer_flash_angle)).into());
                    ui.set_shape_outer_flash_active(Rc::new(slint::VecModel::from(outer_lit)).into());
                }
                app::SlintExtra::Nebula(n) => {
                    ui.set_active_kind(6);
                    let well_x: Vec<f32> = n.wells.iter().map(|(x, _, _, _)| *x).collect();
                    let well_y: Vec<f32> = n.wells.iter().map(|(_, y, _, _)| *y).collect();
                    let well_active: Vec<bool> = n.wells.iter().map(|(_, _, a, _)| *a).collect();
                    let well_lit: Vec<bool> = n.wells.iter().map(|(_, _, _, l)| *l).collect();
                    ui.set_nebula_well_x(Rc::new(slint::VecModel::from(well_x)).into());
                    ui.set_nebula_well_y(Rc::new(slint::VecModel::from(well_y)).into());
                    ui.set_nebula_well_active(Rc::new(slint::VecModel::from(well_active)).into());
                    ui.set_nebula_well_lit(Rc::new(slint::VecModel::from(well_lit)).into());
                    let px: Vec<f32> = n.particles.iter().map(|(x, _, _)| *x).collect();
                    let py: Vec<f32> = n.particles.iter().map(|(_, y, _)| *y).collect();
                    let pb: Vec<f32> = n.particles.iter().map(|(_, _, b)| *b).collect();
                    ui.set_nebula_particle_x(Rc::new(slint::VecModel::from(px)).into());
                    ui.set_nebula_particle_y(Rc::new(slint::VecModel::from(py)).into());
                    ui.set_nebula_particle_brightness(Rc::new(slint::VecModel::from(pb)).into());
                }
                app::SlintExtra::Tape(t) => {
                    ui.set_active_kind(7);
                    let status: Vec<slint::SharedString> = t.tracks.iter().map(|(s, _, _)| s.as_str().into()).collect();
                    let kind: Vec<i32> = t.tracks.iter().map(|(_, k, _)| *k as i32).collect();
                    let playhead = t.tracks.first().map(|(_, _, p)| *p).unwrap_or(0.0);
                    ui.set_tape_track_status(Rc::new(slint::VecModel::from(status)).into());
                    ui.set_tape_track_kind(Rc::new(slint::VecModel::from(kind)).into());
                    ui.set_tape_playhead_frac(playhead);
                }
                app::SlintExtra::Pams(p) => {
                    ui.set_active_kind(8);
                    ui.set_pams_channel_index(p.channel_index as i32);
                    ui.set_pams_history(Rc::new(slint::VecModel::from(p.history)).into());
                    ui.set_pams_value(p.value);
                }
                app::SlintExtra::Orbit(o) => {
                    ui.set_active_kind(9);
                    ui.set_orbit_x(o.x);
                    ui.set_orbit_y(o.y);
                    ui.set_orbit_value(o.value);
                }
                app::SlintExtra::Prism(p) => {
                    ui.set_active_kind(10);
                    let tx: Vec<f32> = p.ticks.iter().map(|(x, _)| *x).collect();
                    let th: Vec<f32> = p.ticks.iter().map(|(_, h)| *h).collect();
                    ui.set_prism_tick_x(Rc::new(slint::VecModel::from(tx)).into());
                    ui.set_prism_tick_height(Rc::new(slint::VecModel::from(th)).into());
                    ui.set_prism_caption(p.caption.into());
                }
                app::SlintExtra::Nautilus(n) => {
                    ui.set_active_kind(11);
                    ui.set_nautilus_delay_mode_name(n.delay_mode_name.into());
                    ui.set_nautilus_delay_mode_index(n.delay_mode_index as i32);
                    ui.set_nautilus_feedback_mode_name(n.feedback_mode_name.into());
                    ui.set_nautilus_feedback_mode_index(n.feedback_mode_index as i32);
                    ui.set_nautilus_chroma_name(n.chroma_name.into());
                    ui.set_nautilus_chroma_index(n.chroma_index as i32);
                    ui.set_nautilus_frozen(n.frozen);
                    ui.set_nautilus_feedback_amount(n.feedback_amount);
                    ui.set_nautilus_line_level(Rc::new(slint::VecModel::from(n.line_level.to_vec())).into());
                    ui.set_nautilus_line_pulse(Rc::new(slint::VecModel::from(n.line_pulse.to_vec())).into());
                    ui.set_nautilus_line_active(Rc::new(slint::VecModel::from(n.line_active.to_vec())).into());
                }
                app::SlintExtra::Sequencer(s) => {
                    ui.set_active_kind(12);
                    ui.set_sequencer_header(s.header.into());
                    let active: Vec<bool> = s.steps.iter().map(|(a, _, _, _, _)| *a).collect();
                    let trimmed: Vec<bool> = s.steps.iter().map(|(_, t, _, _, _)| *t).collect();
                    let playhead: Vec<bool> = s.steps.iter().map(|(_, _, p, _, _)| *p).collect();
                    let focused: Vec<bool> = s.steps.iter().map(|(_, _, _, f, _)| *f).collect();
                    let labels: Vec<slint::SharedString> = s.steps.iter().map(|(_, _, _, _, l)| l.as_str().into()).collect();
                    ui.set_sequencer_step_active(Rc::new(slint::VecModel::from(active)).into());
                    ui.set_sequencer_step_trimmed(Rc::new(slint::VecModel::from(trimmed)).into());
                    ui.set_sequencer_step_playhead(Rc::new(slint::VecModel::from(playhead)).into());
                    ui.set_sequencer_step_focused(Rc::new(slint::VecModel::from(focused)).into());
                    ui.set_sequencer_step_label(Rc::new(slint::VecModel::from(labels)).into());
                    ui.set_sequencer_current_pattern(s.current_pattern as i32);
                    ui.set_sequencer_num_patterns(s.num_patterns as i32);
                    ui.set_sequencer_song_mode(s.song_mode);
                    ui.set_sequencer_song_pos(s.song_pos as i32);
                    ui.set_sequencer_song_length(s.song_length as i32);
                    let song_pattern: Vec<i32> = s.song_slots.iter().map(|(p, _)| *p as i32).collect();
                    let song_repeats: Vec<i32> = s.song_slots.iter().map(|(_, r)| *r as i32).collect();
                    ui.set_sequencer_song_slot_pattern(Rc::new(slint::VecModel::from(song_pattern)).into());
                    ui.set_sequencer_song_slot_repeats(Rc::new(slint::VecModel::from(song_repeats)).into());
                    ui.set_sequencer_pad_perform(s.pad_perform);
                    ui.set_sequencer_last_touched_pad(s.last_touched_pad as i32);
                    ui.set_sequencer_pad_loaded(Rc::new(slint::VecModel::from(s.pad_loaded)).into());
                }
                app::SlintExtra::Clouds(c) => {
                    ui.set_active_kind(13);
                    ui.set_clouds_mode(c.playback_mode as i32);
                    ui.set_clouds_mode_name(c.playback_mode_name.into());
                    ui.set_clouds_frozen(c.frozen);
                    ui.set_clouds_waveform_mid_x(Rc::new(slint::VecModel::from(c.waveform.mid_x)).into());
                    ui.set_clouds_waveform_mid_y(Rc::new(slint::VecModel::from(c.waveform.mid_y)).into());
                    ui.set_clouds_waveform_length(Rc::new(slint::VecModel::from(c.waveform.length)).into());
                    ui.set_clouds_waveform_angle(Rc::new(slint::VecModel::from(c.waveform.angle_deg)).into());
                }
                app::SlintExtra::Beads(b) => {
                    ui.set_active_kind(14);
                    ui.set_beads_mode_name(b.mode_name.into());
                    ui.set_beads_frozen(b.frozen);
                    ui.set_beads_delay_mode(b.is_delay_mode);
                    ui.set_beads_waveform_mid_x(Rc::new(slint::VecModel::from(b.waveform.mid_x)).into());
                    ui.set_beads_waveform_mid_y(Rc::new(slint::VecModel::from(b.waveform.mid_y)).into());
                    ui.set_beads_waveform_length(Rc::new(slint::VecModel::from(b.waveform.length)).into());
                    ui.set_beads_waveform_angle(Rc::new(slint::VecModel::from(b.waveform.angle_deg)).into());
                    let grain_x: Vec<f32> = b.grain_dots.iter().map(|(x, _)| *x).collect();
                    let grain_brightness: Vec<f32> = b.grain_dots.iter().map(|(_, e)| *e).collect();
                    ui.set_beads_grain_x(Rc::new(slint::VecModel::from(grain_x)).into());
                    ui.set_beads_grain_brightness(Rc::new(slint::VecModel::from(grain_brightness)).into());
                }
                app::SlintExtra::BlackHole(h) => {
                    ui.set_active_kind(15);
                    ui.set_black_hole_algorithm_name(h.algorithm_name.into());
                    ui.set_black_hole_category_name(h.category_name.into());
                    ui.set_black_hole_category_index(h.category_index as i32);
                    ui.set_black_hole_clip(h.clip);
                    ui.set_black_hole_waveform_mid_x(Rc::new(slint::VecModel::from(h.waveform.mid_x)).into());
                    ui.set_black_hole_waveform_mid_y(Rc::new(slint::VecModel::from(h.waveform.mid_y)).into());
                    ui.set_black_hole_waveform_length(Rc::new(slint::VecModel::from(h.waveform.length)).into());
                    ui.set_black_hole_waveform_angle(Rc::new(slint::VecModel::from(h.waveform.angle_deg)).into());
                    let labels: Vec<slint::SharedString> = h.param_labels.iter().map(|s| s.as_str().into()).collect();
                    ui.set_black_hole_param_labels(Rc::new(slint::VecModel::from(labels)).into());
                    ui.set_black_hole_param_values(Rc::new(slint::VecModel::from(h.param_values.to_vec())).into());
                    ui.set_black_hole_param_bold(Rc::new(slint::VecModel::from(h.param_bold.to_vec())).into());
                }
                app::SlintExtra::QueenOfPentacles(q) => {
                    ui.set_active_kind(16);
                    ui.set_qop_map_name(q.map_name.into());
                    ui.set_qop_r(q.r);
                    ui.set_qop_frozen(q.frozen);
                    ui.set_qop_trajectory_mid_x(Rc::new(slint::VecModel::from(q.trajectory.mid_x)).into());
                    ui.set_qop_trajectory_mid_y(Rc::new(slint::VecModel::from(q.trajectory.mid_y)).into());
                    ui.set_qop_trajectory_length(Rc::new(slint::VecModel::from(q.trajectory.length)).into());
                    ui.set_qop_trajectory_angle(Rc::new(slint::VecModel::from(q.trajectory.angle_deg)).into());
                    ui.set_qop_cv(q.cv);
                    ui.set_qop_smooth_cv(q.smooth_cv);
                    ui.set_qop_gate(q.gate);
                    ui.set_qop_gate_mode_name(q.gate_mode_name.into());
                    ui.set_qop_delta(q.delta);
                    ui.set_qop_threshold(q.threshold);
                }
                app::SlintExtra::NaturalGate(n) => {
                    ui.set_active_kind(17);
                    ui.set_ng_ch1_trace_mid_x(Rc::new(slint::VecModel::from(n.ch1_trace.mid_x)).into());
                    ui.set_ng_ch1_trace_mid_y(Rc::new(slint::VecModel::from(n.ch1_trace.mid_y)).into());
                    ui.set_ng_ch1_trace_length(Rc::new(slint::VecModel::from(n.ch1_trace.length)).into());
                    ui.set_ng_ch1_trace_angle(Rc::new(slint::VecModel::from(n.ch1_trace.angle_deg)).into());
                    ui.set_ng_ch1_now(n.ch1_now);
                    ui.set_ng_ch1_open(n.ch1_open);
                    ui.set_ng_ch1_hit(n.ch1_hit);
                    ui.set_ng_ch2_trace_mid_x(Rc::new(slint::VecModel::from(n.ch2_trace.mid_x)).into());
                    ui.set_ng_ch2_trace_mid_y(Rc::new(slint::VecModel::from(n.ch2_trace.mid_y)).into());
                    ui.set_ng_ch2_trace_length(Rc::new(slint::VecModel::from(n.ch2_trace.length)).into());
                    ui.set_ng_ch2_trace_angle(Rc::new(slint::VecModel::from(n.ch2_trace.angle_deg)).into());
                    ui.set_ng_ch2_now(n.ch2_now);
                    ui.set_ng_ch2_open(n.ch2_open);
                    ui.set_ng_ch2_hit(n.ch2_hit);
                }
                app::SlintExtra::TuringMachine(t) => {
                    ui.set_active_kind(18);
                    ui.set_tm_bits(Rc::new(slint::VecModel::from(t.bits.to_vec())).into());
                    ui.set_tm_active_len(t.active_len as i32);
                    ui.set_tm_write_index(t.write_index as i32);
                    ui.set_tm_pulse(t.pulse);
                    ui.set_tm_cv(t.cv);
                    ui.set_tm_keep_probability(t.keep_probability);
                    ui.set_tm_inverted_feedback(t.inverted_feedback);
                    ui.set_tm_double_locked(t.double_locked);
                }
                app::SlintExtra::Rainmaker(r) => {
                    ui.set_active_kind(19);
                    ui.set_rm_groove_name(r.groove_name.into());
                    ui.set_rm_groove_amount(r.groove_amount);
                    ui.set_rm_grid_label(r.grid_label.into());
                    ui.set_rm_beats_spanned(r.beats_spanned.max(1.0));
                    let tap_time: Vec<f32> = r.taps.iter().map(|(t, _, _, _)| *t).collect();
                    let tap_level: Vec<f32> = r.taps.iter().map(|(_, l, _, _)| *l).collect();
                    let tap_pan: Vec<f32> = r.taps.iter().map(|(_, _, p, _)| *p).collect();
                    let tap_muted: Vec<bool> = r.taps.iter().map(|(_, _, _, m)| *m).collect();
                    ui.set_rm_tap_time(Rc::new(slint::VecModel::from(tap_time)).into());
                    ui.set_rm_tap_level(Rc::new(slint::VecModel::from(tap_level)).into());
                    ui.set_rm_tap_pan(Rc::new(slint::VecModel::from(tap_pan)).into());
                    ui.set_rm_tap_muted(Rc::new(slint::VecModel::from(tap_muted)).into());
                }
                app::SlintExtra::Starlab(s) => {
                    ui.set_active_kind(20);
                    ui.set_starlab_texture_name(s.texture_name.into());
                    ui.set_starlab_texture_index(s.texture_index as i32);
                    ui.set_starlab_infinite(s.infinite);
                    ui.set_starlab_karplus(s.karplus_mode);
                    ui.set_starlab_lfo_phase(s.lfo_phase);
                    ui.set_starlab_tank_energy(s.tank_energy);
                    ui.set_starlab_waveform_mid_x(Rc::new(slint::VecModel::from(s.waveform.mid_x)).into());
                    ui.set_starlab_waveform_mid_y(Rc::new(slint::VecModel::from(s.waveform.mid_y)).into());
                    ui.set_starlab_waveform_length(Rc::new(slint::VecModel::from(s.waveform.length)).into());
                    ui.set_starlab_waveform_angle(Rc::new(slint::VecModel::from(s.waveform.angle_deg)).into());
                }
                app::SlintExtra::Warps(w) => {
                    ui.set_active_kind(21);
                    let name = match w.next_algorithm_name {
                        Some(next) if w.crossfade_frac > 0.02 => format!("{} -> {} ({:.0}%)", w.algorithm_name, next, w.crossfade_frac * 100.0),
                        _ => w.algorithm_name,
                    };
                    ui.set_warps_algorithm_name(name.into());
                    ui.set_warps_is_vocoder(w.is_vocoder);
                    ui.set_warps_vocoder_frozen(w.vocoder_frozen);
                    ui.set_warps_osc_enabled(w.osc_enabled);
                    ui.set_warps_osc_waveform_name(w.osc_waveform_name.into());
                    ui.set_warps_carrier_mid_x(Rc::new(slint::VecModel::from(w.carrier_trace.mid_x)).into());
                    ui.set_warps_carrier_mid_y(Rc::new(slint::VecModel::from(w.carrier_trace.mid_y)).into());
                    ui.set_warps_carrier_length(Rc::new(slint::VecModel::from(w.carrier_trace.length)).into());
                    ui.set_warps_carrier_angle(Rc::new(slint::VecModel::from(w.carrier_trace.angle_deg)).into());
                    ui.set_warps_modulator_mid_x(Rc::new(slint::VecModel::from(w.modulator_trace.mid_x)).into());
                    ui.set_warps_modulator_mid_y(Rc::new(slint::VecModel::from(w.modulator_trace.mid_y)).into());
                    ui.set_warps_modulator_length(Rc::new(slint::VecModel::from(w.modulator_trace.length)).into());
                    ui.set_warps_modulator_angle(Rc::new(slint::VecModel::from(w.modulator_trace.angle_deg)).into());
                    ui.set_warps_output_mid_x(Rc::new(slint::VecModel::from(w.output_trace.mid_x)).into());
                    ui.set_warps_output_mid_y(Rc::new(slint::VecModel::from(w.output_trace.mid_y)).into());
                    ui.set_warps_output_length(Rc::new(slint::VecModel::from(w.output_trace.length)).into());
                    ui.set_warps_output_angle(Rc::new(slint::VecModel::from(w.output_trace.angle_deg)).into());
                    ui.set_warps_vocoder_bands(Rc::new(slint::VecModel::from(w.vocoder_band_levels.to_vec())).into());
                }
                app::SlintExtra::Mixer(m) => {
                    ui.set_active_kind(22);
                    ui.set_mixer_master(m.master);
                    let fader: Vec<f32> = m.channels.iter().map(|(_, f, _)| *f).collect();
                    let live: Vec<f32> = m.channels.iter().map(|(_, _, l)| *l).collect();
                    ui.set_mixer_fader(Rc::new(slint::VecModel::from(fader)).into());
                    ui.set_mixer_live(Rc::new(slint::VecModel::from(live)).into());
                }
                app::SlintExtra::CvOut(c) => {
                    ui.set_active_kind(23);
                    ui.set_cv_out_midi_channel(c.midi_channel as i32);
                    ui.set_cv_out_levels(Rc::new(slint::VecModel::from(c.levels.to_vec())).into());
                }
                app::SlintExtra::Magnito(m) => {
                    ui.set_active_kind(24);
                    ui.set_magnito_loop_mid_x(Rc::new(slint::VecModel::from(m.loop_trace.mid_x)).into());
                    ui.set_magnito_loop_mid_y(Rc::new(slint::VecModel::from(m.loop_trace.mid_y)).into());
                    ui.set_magnito_loop_length(Rc::new(slint::VecModel::from(m.loop_trace.length)).into());
                    ui.set_magnito_loop_angle(Rc::new(slint::VecModel::from(m.loop_trace.angle_deg)).into());
                    ui.set_magnito_bias(m.bias);
                    ui.set_magnito_unlimited(m.unlimited);
                    ui.set_magnito_wobble_mid_x(Rc::new(slint::VecModel::from(m.wobble_trace.mid_x)).into());
                    ui.set_magnito_wobble_mid_y(Rc::new(slint::VecModel::from(m.wobble_trace.mid_y)).into());
                    ui.set_magnito_wobble_length(Rc::new(slint::VecModel::from(m.wobble_trace.length)).into());
                    ui.set_magnito_wobble_angle(Rc::new(slint::VecModel::from(m.wobble_trace.angle_deg)).into());
                    ui.set_magnito_wow_flutter_depth(m.wow_flutter_depth);
                    ui.set_magnito_hiss_level(m.hiss_level);
                    ui.set_magnito_wear(m.wear);
                    ui.set_magnito_dropout(m.dropout_active);
                    ui.set_magnito_output_peak(m.output_peak);
                }
                app::SlintExtra::Theme(t) => {
                    ui.set_active_kind(25);
                    ui.set_theme_editing_bg(t.editing_background);
                    ui.set_theme_hex(if t.editing_background {
                        format!("#{:02X}{:02X}{:02X}", t.background_rgb.0, t.background_rgb.1, t.background_rgb.2)
                    } else {
                        format!("#{:02X}{:02X}{:02X}", t.accent_rgb.0, t.accent_rgb.1, t.accent_rgb.2)
                    }.into());
                    ui.set_theme_marker_x(t.marker_x);
                    ui.set_theme_marker_y(t.marker_y);
                }
                app::SlintExtra::Tonestack(t) => {
                    ui.set_active_kind(26);
                    ui.set_tonestack_voicing_name(t.voicing_name.into());
                    let (mid_x, mid_y, length, angle_deg) = app::polyline_segments(&t.waveform, 260.0, 90.0, true);
                    ui.set_tonestack_waveform_mid_x(Rc::new(slint::VecModel::from(mid_x)).into());
                    ui.set_tonestack_waveform_mid_y(Rc::new(slint::VecModel::from(mid_y)).into());
                    ui.set_tonestack_waveform_length(Rc::new(slint::VecModel::from(length)).into());
                    ui.set_tonestack_waveform_angle(Rc::new(slint::VecModel::from(angle_deg)).into());
                    ui.set_tonestack_output_peak(t.output_peak);
                    ui.set_tonestack_gate_closed(t.gate_closed);
                }
                app::SlintExtra::SampleDrum(s) => {
                    ui.set_active_kind(27);
                    ui.set_sample_drum_channel(s.channel as i32);
                    ui.set_sample_drum_sample_name(s.sample_name.into());
                    ui.set_sample_drum_num_slices(s.num_slices as i32);
                    ui.set_sample_drum_step_index(s.step_index as i32);
                    let (mid_x, mid_y, length, angle_deg) = app::polyline_segments(&s.waveform, 260.0, 110.0, true);
                    ui.set_sample_drum_waveform_mid_x(Rc::new(slint::VecModel::from(mid_x)).into());
                    ui.set_sample_drum_waveform_mid_y(Rc::new(slint::VecModel::from(mid_y)).into());
                    ui.set_sample_drum_waveform_length(Rc::new(slint::VecModel::from(length)).into());
                    ui.set_sample_drum_waveform_angle(Rc::new(slint::VecModel::from(angle_deg)).into());
                    ui.set_sample_drum_source_waveform(Rc::new(slint::VecModel::from(s.source_waveform)).into());
                    let (lowers, uppers): (Vec<f32>, Vec<f32>) = s.slice_bounds.into_iter().unzip();
                    ui.set_sample_drum_slice_lowers(Rc::new(slint::VecModel::from(lowers)).into());
                    ui.set_sample_drum_slice_uppers(Rc::new(slint::VecModel::from(uppers)).into());
                }
                app::SlintExtra::Visualizer(v) => {
                    ui.set_active_kind(28);
                    ui.set_visualizer_mode_kind(v.mode_kind as i32);
                    ui.set_visualizer_mode_name(v.mode_name.into());
                    ui.set_visualizer_spectrum(Rc::new(slint::VecModel::from(v.spectrum)).into());
                    ui.set_visualizer_waveform_mid_x(Rc::new(slint::VecModel::from(v.waveform.mid_x)).into());
                    ui.set_visualizer_waveform_mid_y(Rc::new(slint::VecModel::from(v.waveform.mid_y)).into());
                    ui.set_visualizer_waveform_length(Rc::new(slint::VecModel::from(v.waveform.length)).into());
                    ui.set_visualizer_waveform_angle(Rc::new(slint::VecModel::from(v.waveform.angle_deg)).into());
                    ui.set_visualizer_peak_level(v.peak_level);
                    ui.set_visualizer_rms_level(v.rms_level);
                    ui.set_visualizer_pitch_name(v.pitch_name.into());
                    ui.set_visualizer_scene_kind(v.scene_kind as i32);
                    ui.set_visualizer_scene_name(v.scene_name.into());
                    ui.set_visualizer_bass_level(v.bass_level);
                    ui.set_visualizer_treble_level(v.treble_level);
                    ui.set_visualizer_beat_pulse(v.beat_pulse);
                    ui.set_visualizer_car_x(v.car_x);
                    ui.set_visualizer_cat_paw_left(v.cat_paw_left);
                    ui.set_visualizer_monitor_on(v.monitor_on);
                }
                app::SlintExtra::None => ui.set_active_kind(0),
            }
}

#[path = "slint_common/instrument_preview.rs"]
mod instrument_preview;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--render-instruments") {
        instrument_preview::render(args.get(2).expect("usage: --render-instruments OUTPUT_DIRECTORY"));
        return;
    }
    // --- Real engine, real buses, real registry wiring -- exactly
    // main.rs's own setup, just without the embedded_graphics window
    // loop (Slint owns the window here instead). ---
    let cutoff = Arc::new(AtomicF32::new(1000.0));
    let sensitivity = Arc::new(AtomicF32::new(0.1));
    let nav_speed = Arc::new(AtomicF32::new(3.0));
    let show_cpu = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let midi_map = Arc::new(midi_map::MidiMap::new());
    let accent = Arc::new(theme::ThemeColor::new(theme::ACCENT_DEFAULT_HUE, theme::ACCENT_DEFAULT_SAT, theme::ACCENT_DEFAULT_VAL));
    let background = Arc::new(theme::ThemeColor::new(theme::BG_DEFAULT_HUE, theme::BG_DEFAULT_SAT, theme::BG_DEFAULT_VAL));
    let master_volume = Arc::new(AtomicF32::new(1.0));
    let modbus = Arc::new(ModBus::new());
    let audio_bus = Arc::new(AudioBus::new());
    let mixer_bus = Arc::new(MixerBus::new());
    let prism_cc = Arc::new(apps::prism::PrismCcTargets::new());

    let engine = audio::new_engine(Arc::clone(&master_volume));
    let (mut audio_host, device_state) = AudioHost::open_resilient(Arc::clone(&engine));
    println!("Live Home prototype -- output device: {}", device_state.current_output());
    let device_state = Arc::new(device_state);


    let manifests = manifest::discover(std::path::Path::new(APPS_DIR));
    println!("Found {} app manifest(s) in {APPS_DIR}", manifests.len());
    let registry = Registry::new(
        Arc::clone(&cutoff),
        Arc::clone(&device_state),
        Arc::clone(&sensitivity),
        Arc::clone(&nav_speed),
        Arc::clone(&modbus),
        Arc::clone(&audio_bus),
        Arc::clone(&master_volume),
        Arc::clone(&mixer_bus),
        Arc::clone(&prism_cc),
        Arc::clone(&show_cpu),
        Arc::clone(&midi_map),
        Arc::clone(&accent),
        Arc::clone(&background),
    );
    let mut apps = registry.build(&manifests);

    // Every app's processor goes into the one shared engine exactly
    // once, here -- not on enter/exit -- so it keeps mixing/running
    // for the program's life regardless of which app is on screen,
    // same as the real firmware (see main.rs's own copy of this loop).
    for (_, app) in apps.iter_mut() {
        if let Some(processor) = app.audio_processor() {
            engine.add(processor);
        }
    }

    let apps = Rc::new(RefCell::new(apps));
    // `None` = the home/launcher list; `Some(i)` = that app is active.
    let active: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));
    // The "pinned" live instrument -- pads/notes always go here (see
    // `F2` below) instead of always following whichever screen is on
    // top, so browsing a different app's menu (Clouds) doesn't cut
    // off an instrument you're still playing (Plaits) via the pads.
    // `None` means "no pin, follow the active screen" -- today's
    // original behavior, unchanged unless you actually use F2.
    let midi_target: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));
    let home_list = Rc::new(RefCell::new(ParamList::new()));

    println!("App registry ready; creating Slint window");
    let ui = LiveHomeScreen::new().unwrap();
    {
        let names: Vec<slint::SharedString> = apps.borrow().iter().map(|(n, _)| n.as_str().into()).collect();
        ui.set_home_names(Rc::new(slint::VecModel::from(names)).into());
    }

    // Boot sequence: SZYK -> MX1, each up for 2s or until any input,
    // then the launcher takes over -- same behavior/duration as the
    // real device's `Os::run` (see os.rs's own `SPLASH_DURATION`/
    // `SplashStage`), just driven from this timer loop instead.
    const SPLASH_DURATION: std::time::Duration = std::time::Duration::from_secs(2);
    let splash_stage: Rc<RefCell<Option<SplashStage>>> = Rc::new(RefCell::new(Some(SplashStage::Szyk)));
    let splash_deadline: Rc<RefCell<std::time::Instant>> = Rc::new(RefCell::new(std::time::Instant::now() + SPLASH_DURATION));
    ui.set_splash_active(true);
    ui.set_splash_image(logo_to_slint_image(&startup_logo::SZYK));

    // Real MIDI input, alongside the mouse -- see live_midi.rs.
    let controller = Arc::new(ControllerState::new());
    let _midi_connections = live_midi::connect_all(Arc::clone(&controller), Arc::clone(&midi_map), Arc::clone(&modbus));

    let grid_held: Rc<RefCell<[bool; 16]>> = Rc::new(RefCell::new([false; 16]));
    let grid_for_pad = Rc::clone(&grid_held);
    ui.on_pad_toggled(move |i, down| {
        grid_for_pad.borrow_mut()[i as usize] = down;
    });

    // The color wheel only means anything while Settings is the
    // active screen (it's the one app that draws `active-kind == 25`
    // in the first place) -- forwarded to whichever app that is
    // rather than a direct `SettingsApp` handle, same as every other
    // per-app callback on this shared screen.
    let apps_for_wheel = Rc::clone(&apps);
    let active_for_wheel = Rc::clone(&active);
    ui.on_theme_wheel_picked(move |x, y| {
        if let Some(idx) = *active_for_wheel.borrow() {
            apps_for_wheel.borrow_mut()[idx].1.slint_pointer_pick(x, y);
        }
    });

    let knob1_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let knob2_press: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
    let k1p = Rc::clone(&knob1_press);
    ui.on_knob1_clicked(move || { *k1p.borrow_mut() = true; });
    let k2p = Rc::clone(&knob2_press);
    ui.on_knob2_clicked(move || { *k2p.borrow_mut() = true; });

    // F1 (home)/F3 (start-stop)/F4 (jump to Mixer) -- the real device's
    // global navigation buttons (see os.rs), which work the same way
    // regardless of which app is active.
    let apps_for_f = Rc::clone(&apps);
    let active_for_f = Rc::clone(&active);
    let midi_target_for_f = Rc::clone(&midi_target);
    ui.on_f_clicked(move |i| match i {
        0 => {
            let mut act = active_for_f.borrow_mut();
            if let Some(idx) = act.take() {
                apps_for_f.borrow_mut()[idx].1.on_exit();
            } else {
                // Already home -- Home itself would be a no-op here,
                // so F1 doubles as a shortcut straight to Settings.
                let settings_idx = apps_for_f.borrow().iter().position(|(_, app)| app.system_role() == Some(app::SystemRole::Settings));
                if let Some(idx) = settings_idx {
                    apps_for_f.borrow_mut()[idx].1.on_enter();
                    *act = Some(idx);
                }
            }
        }
        1 => {
            if let Some(idx) = *active_for_f.borrow() {
                if apps_for_f.borrow()[idx].1.grid_mode_label().is_some() {
                    *midi_target_for_f.borrow_mut() = None;
                    apps_for_f.borrow_mut()[idx].1.toggle_grid_mode();
                } else if midi_target_for_f.borrow().is_some() {
                    *midi_target_for_f.borrow_mut() = None;
                } else if apps_for_f.borrow()[idx].1.supports_pad_lock() {
                    *midi_target_for_f.borrow_mut() = Some(idx);
                }
            } else { *midi_target_for_f.borrow_mut() = None; }
        }
        2 => {
            if let Some(idx) = *active_for_f.borrow() {
                apps_for_f.borrow_mut()[idx].1.toggle_running();
            }
        }
        3 => {
            let mixer_idx = apps_for_f.borrow().iter().position(|(_, app)| app.system_role() == Some(app::SystemRole::Mixer));
            if let Some(idx) = mixer_idx {
                let mut act = active_for_f.borrow_mut();
                if *act != Some(idx) {
                    if let Some(old) = *act {
                        apps_for_f.borrow_mut()[old].1.on_exit();
                    }
                    apps_for_f.borrow_mut()[idx].1.on_enter();
                    *act = Some(idx);
                }
            }
        }
        _ => {}
    });

    let ui_weak = ui.as_weak();
    let apps_for_timer = Rc::clone(&apps);
    let active_for_timer = Rc::clone(&active);
    let grid_for_timer = Rc::clone(&grid_held);
    let home_list_for_timer = Rc::clone(&home_list);
    let midi_target_for_timer = Rc::clone(&midi_target);
    let engine_for_timer = Arc::clone(&engine);
    let show_cpu_for_timer = Arc::clone(&show_cpu);
    let accent_for_timer = Arc::clone(&accent);
    let background_for_timer = Arc::clone(&background);
    let timer = slint::Timer::default();
    let stick_repeat = std::time::Duration::from_millis(120);
    let mut last_stick_step = std::time::Instant::now() - stick_repeat;
    let mut last_pad_target: Option<usize> = None;
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(33), move || {
        audio_host.poll(&device_state);
        let Some(ui) = ui_weak.upgrade() else { return };
        ui.set_audio_connected(audio_host.is_connected());

        let show_cpu = show_cpu_for_timer.load(std::sync::atomic::Ordering::Relaxed);
        ui.set_cpu_visible(show_cpu);
        if show_cpu {
            ui.set_cpu_load_pct(engine_for_timer.load() * 100.0);
        }

        // Default: the live ThemeColor wheel drives the whole screen,
        // same as before -- this is what the home/launcher list and
        // Settings' own screen keep using. Below, once we know which
        // app (if any) is active, a redesigned app's own fixed
        // palette overrides live-accent/live-bg/live-ink instead, so
        // the wheel only ever recolors the menu, per the user's
        // "keep that for the menu" instruction.
        let (ar, ag, ab) = accent_for_timer.rgb();
        let (br, bg, bb) = background_for_timer.rgb();
        ui.set_live_accent(slint::Color::from_rgb_u8(ar, ag, ab));
        ui.set_live_bg(slint::Color::from_rgb_u8(br, bg, bb));
        ui.set_live_ink(slint::Color::from_rgb_u8(255, 255, 255));
        ui.set_theme_accent_swatch(slint::Color::from_rgb_u8(ar, ag, ab));
        ui.set_theme_bg_swatch(slint::Color::from_rgb_u8(br, bg, bb));

        let midi_k1 = controller.take_knob1_delta();
        let midi_k2 = controller.take_knob2_delta();
        // The spring-return stick repeats on its dominant axis after a dead zone.
        // Discrete navigation bypasses the encoder ticks-per-row setting.
        let x = ui.get_live_stick_x();
        let y = ui.get_live_stick_y();
        let mut stick_nav = 0;
        let mut stick_edit = 0;
        if x.abs().max(y.abs()) <= 0.35 {
            last_stick_step = std::time::Instant::now() - stick_repeat;
        } else if last_stick_step.elapsed() >= stick_repeat {
            if x.abs() > y.abs() { stick_edit = if x > 0.0 { 1 } else { -1 }; }
            else { stick_nav = if y > 0.0 { 1 } else { -1 }; }
            last_stick_step = std::time::Instant::now();
        }
        let navigation = ui.get_navigation_delta() + stick_nav;
        ui.set_navigation_delta(0);
        let k1 = ui.get_knob1_delta().round() as i32 + midi_k1;
        let k2 = ui.get_knob2_delta().round() as i32 + midi_k2 + stick_edit;
        ui.set_knob1_delta(0.0);
        ui.set_knob2_delta(0.0);
        // Retain encoder state for MIDI/API compatibility; the new shell has no knobs.
        const DEG_PER_TICK: f32 = 1.4 * 4.0;
        if midi_k1 != 0 {
            ui.set_live_knob1_angle(ui.get_live_knob1_angle() + midi_k1 as f32 * DEG_PER_TICK);
        }
        if midi_k2 != 0 {
            ui.set_live_knob2_angle(ui.get_live_knob2_angle() + midi_k2 as f32 * DEG_PER_TICK);
        }
        let press1 = std::mem::take(&mut *knob1_press.borrow_mut()) || controller.take_knob1_press();
        let press2 = std::mem::take(&mut *knob2_press.borrow_mut()) || controller.take_knob2_press();
        let grid: [bool; 16] = std::array::from_fn(|i| grid_for_timer.borrow()[i] || controller.grid[i].load(Ordering::Relaxed));

        // Boot sequence: any input at all dismisses it outright (not
        // just advancing to the next stage), same as the real
        // device's own `any_input` check in os.rs. A timeout instead
        // advances SZYK -> MX1 -> the launcher.
        let splash_stage_now = *splash_stage.borrow(); // a separate `let` so this `Ref` guard drops immediately, not extended across the whole `if let` body below (which would deadlock the `borrow_mut()` inside it)
        if let Some(stage) = splash_stage_now {
            let any_input = navigation != 0 || k1 != 0 || k2 != 0 || press1 || press2 || grid.iter().any(|&p| p);
            let next_stage = if any_input {
                None
            } else if std::time::Instant::now() >= *splash_deadline.borrow() {
                match stage {
                    SplashStage::Szyk => Some(SplashStage::Mx1),
                    SplashStage::Mx1 => None,
                }
            } else {
                Some(stage)
            };
            if next_stage != Some(stage) {
                *splash_stage.borrow_mut() = next_stage;
                match next_stage {
                    Some(SplashStage::Szyk) => unreachable!("never advances backward to Szyk"),
                    Some(SplashStage::Mx1) => {
                        ui.set_splash_is_szyk(false);
                        ui.set_splash_image(logo_to_slint_image(&startup_logo::MX1));
                        *splash_deadline.borrow_mut() = std::time::Instant::now() + SPLASH_DURATION;
                    }
                    None => ui.set_splash_active(false),
                }
            }
            if next_stage.is_some() {
                return; // still showing a splash stage -- skip every other per-frame update below
            }
        }

        // Physical F-button presses (Push 2's top row / a mapped
        // controller) -- `set_top` only stores the press; nothing
        // was ever polling it, so it never reached `f-clicked`.
        for i in 0..4 {
            if controller.take_top(i) {
                ui.invoke_f_clicked(i as i32);
            }
        }

        let mut apps_ref = apps_for_timer.borrow_mut();

        // Real background work for *every* installed app, every frame,
        // regardless of which is active -- same as `Os::run`'s loop
        // (the one real use today is CV Out's MIDI CC sending).
        for (_, app) in apps_ref.iter_mut() {
            app.background_tick();
        }

        ui.set_has_settings(apps_ref.iter().any(|(_,a)| a.system_role() == Some(app::SystemRole::Settings)));
        ui.set_has_mixer(apps_ref.iter().any(|(_,a)| a.system_role() == Some(app::SystemRole::Mixer)));
        let target_label = midi_target_for_timer.borrow().and_then(|i| apps_ref.get(i)).map(|(name,_)| name.clone()).unwrap_or_default();
        ui.set_midi_target_label(target_label.into());
        let next_pad_target = (*midi_target_for_timer.borrow()).or(*active_for_timer.borrow());
        if next_pad_target != last_pad_target {
            if let Some(old) = last_pad_target { if let Some((_, old_app)) = apps_ref.get_mut(old) { old_app.tick(&Input::default()); } }
            last_pad_target = next_pad_target;
        }
        let is_home = active_for_timer.borrow().is_none();
        ui.set_on_home(is_home);

        if is_home {
            // Reset any bespoke visual left over from whichever app
            // was last active, but keep driving the pads if something
            // is pinned (see F2) -- browsing the launcher list
            // shouldn't cut off an instrument you're still playing.
            ui.set_active_kind(0);
            ui.set_grid_mode_label("".into());
            ui.set_transport_action("".into());
            ui.set_transport_label("".into());
            ui.set_pad_lock_available(false);
            if let Some(play_idx) = *midi_target_for_timer.borrow() {
                let play_input = Input { grid, ..Default::default() };
                apps_ref[play_idx].1.tick(&play_input);
                let overlay = apps_ref[play_idx].1.grid_led_overlay();
                let colors: Vec<slint::Color> = (0..16)
                    .map(|i| match overlay[i] {
                        led_output::PadColor::Off if grid[i] => slint::Color::from_rgb_u8(0x2E, 0xCC, 0x55),
                        led_output::PadColor::Off => slint::Color::from_rgb_u8(0x23, 0x23, 0x23),
                        led_output::PadColor::Green => slint::Color::from_rgb_u8(0x2E, 0xCC, 0x55),
                        led_output::PadColor::Red => slint::Color::from_rgb_u8(0xFF, 0x4D, 0x4D),
                        led_output::PadColor::Yellow => slint::Color::from_rgb_u8(0xE0, 0xC0, 0x30),
                        led_output::PadColor::Blue => slint::Color::from_rgb_u8(0x40, 0x90, 0xE0),
                    })
                    .collect();
                ui.set_live_pad_colors(Rc::new(slint::VecModel::from(colors)).into());
            } else {
                let default_colors: Vec<slint::Color> = (0..16).map(|_| slint::Color::from_rgb_u8(0x23, 0x23, 0x23)).collect();
                ui.set_live_pad_colors(Rc::new(slint::VecModel::from(default_colors)).into());
            }

            let mut list = home_list_for_timer.borrow_mut();
            list.navigate(k1, apps_ref.len(), nav_speed.get() as i32);
            list.navigate_steps(navigation, apps_ref.len());
            if press1 && !apps_ref.is_empty() {
                let idx = list.selected;
                apps_ref[idx].1.on_enter();
                ui.set_active_app_name(apps_ref[idx].0.as_str().into());
                *active_for_timer.borrow_mut() = Some(idx);
            }
            let (start, end) = list.centered_scroll_window(HOME_VISIBLE_ROWS, apps_ref.len());
            ui.set_home_selected((list.selected - start) as i32);
            ui.set_home_more_above(start > 0);
            ui.set_home_more_below(end < apps_ref.len());
            let names: Vec<slint::SharedString> = apps_ref[start..end].iter().map(|(n, _)| n.as_str().into()).collect();
            ui.set_home_names(Rc::new(slint::VecModel::from(names)).into());
        } else {
            let idx = active_for_timer.borrow().unwrap();
            // A redesigned app overrides the ThemeColor default set
            // above with its own fixed identity -- see `app_palette`.
            // Apps with no entry there (Settings, plus any not yet
            // redesigned) keep following the live wheel.
            if let Some((pbg, pink, paccent, _pdim)) = app_palette(apps_ref[idx].0.as_str()) {
                ui.set_live_accent(paccent);
                ui.set_live_bg(pbg);
                ui.set_live_ink(pink);
            }
            // Set every tick (not just on the home-list transition
            // that first entered an app) -- F1/F2/F4's own shortcuts
            // jump straight into an app without going through that
            // path, so the title would otherwise keep showing
            // whatever app was active before.
            ui.set_active_app_name(apps_ref[idx].0.as_str().into());
            // Pads/notes always go to the pinned instrument (F2), if
            // any, instead of always following the on-screen app --
            // browsing a different app's menu shouldn't cut off an
            // instrument you're still playing via the pads. The
            // screen app still gets the knobs (menu nav/edit)
            // regardless; it only also gets the pads when nothing
            // else is pinned (today's original, unchanged behavior).
            let play_idx = midi_target_for_timer.borrow().unwrap_or(idx);
            if play_idx != idx {
                let play_input = Input { grid, ..Default::default() };
                apps_ref[play_idx].1.tick(&play_input);
            }
            let pad_overlay = apps_ref[play_idx].1.grid_led_overlay();
            let play_name_upper = apps_ref[play_idx].0.to_uppercase();
            let screen_grid: [bool; 16] = if play_idx == idx { grid } else { [false; 16] };
            let input = Input { grid: screen_grid, navigation_steps: navigation, knob1: k1, knob2: k2, knob1_press: press1, knob2_press: press2, ..Default::default() };
            let app = &mut apps_ref[idx].1;
            app.tick(&input);

            let (rows, selected_in_window, more_above, more_below) = app.slint_windowed_rows(VISIBLE_ROWS);
            let levels = app.slint_levels(VISIBLE_ROWS);
            let names: Vec<slint::SharedString> = rows.iter().map(|(n, _, _)| n.as_str().into()).collect();
            let values: Vec<slint::SharedString> = rows.iter().map(|(_, v, _)| v.as_str().into()).collect();
            let is_group: Vec<bool> = rows.iter().map(|(_, _, g)| *g).collect();
            let level_values: Vec<f32> = if levels.is_empty() {
                vec![-1.0; rows.len()]
            } else {
                levels.iter().map(|l| l.unwrap_or(-1.0)).collect()
            };
            ui.set_row_names(Rc::new(slint::VecModel::from(names)).into());
            ui.set_row_values(Rc::new(slint::VecModel::from(values)).into());
            ui.set_row_is_group(Rc::new(slint::VecModel::from(is_group)).into());
            ui.set_row_levels(Rc::new(slint::VecModel::from(level_values)).into());
            ui.set_selected_row(selected_in_window as i32);
            ui.set_more_above(more_above);
            ui.set_more_below(more_below);
            ui.set_transport_label(match app.running() {
                Some(true) => "RUNNING".into(),
                Some(false) => "STOPPED".into(),
                None => slint::SharedString::from(""),
            });
            ui.set_grid_mode_label(app.grid_mode_label().unwrap_or("").into());
            ui.set_transport_action(app.transport_action().unwrap_or("").into());
            ui.set_pad_lock_available(app.supports_pad_lock());

            // Real pad lighting: an app with its own meaning for the
            // pads (Sequencer's programmed/playhead steps) gets that;
            // any pad actually currently held lights up green as a
            // real "this note is sounding" indicator otherwise --
            // Plaits/Cascade/Voltage/Starlab all read `Input.grid` as
            // a keyboard, but until now nothing on this shared screen
            // ever reflected that back visually.
            let colors: Vec<slint::Color> = (0..16)
                .map(|i| match pad_overlay[i] {
                    led_output::PadColor::Off if grid[i] => slint::Color::from_rgb_u8(0x2E, 0xCC, 0x55),
                    led_output::PadColor::Off => slint::Color::from_rgb_u8(0x23, 0x23, 0x23),
                    led_output::PadColor::Green => slint::Color::from_rgb_u8(0x2E, 0xCC, 0x55),
                    led_output::PadColor::Red => slint::Color::from_rgb_u8(0xFF, 0x4D, 0x4D),
                    led_output::PadColor::Yellow => slint::Color::from_rgb_u8(0xE0, 0xC0, 0x30),
                    led_output::PadColor::Blue => slint::Color::from_rgb_u8(0x40, 0x90, 0xE0),
                })
                .collect();
            ui.set_live_pad_colors(Rc::new(slint::VecModel::from(colors)).into());

            let midi_pinned = midi_target_for_timer.borrow().is_some();


            // Plaits' bespoke visual (engine dots + real analyzer) --
            // every other app just gets `active_kind = 0` (the
            // generic full-width list).
            apply_instrument_visual(&ui, app.slint_extra());
        }
    });

    ui.show().unwrap();
    println!("Portamax window shown; starting UI event loop");
    ui.run().unwrap();
}
