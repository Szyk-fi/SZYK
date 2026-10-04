//! The shared play layer: one structure every instrument, effect and
//! sequencer uses so each app opens as something to *play* rather than a
//! menu to dive into. Plaits was the first app built this way; this is
//! that structure pulled out so every app gets the same controls in the
//! same places:
//!
//! - **R1** flips between the play view and the app's full menu. Nothing
//!   from the menu is lost; the play view sits in front of it.
//! - **F2** cycles pad layers. An app's own pad behaviour always comes
//!   first (`Layer::Native`), so its pads work exactly as before until F2
//!   is pressed. Then the shared ones: **Controls** (16 parameters on the
//!   pads, knob 2 turns the grabbed one), **Moments** (tap to recall a
//!   whole sound, hold to store it, saved to the SD card) and, for effects,
//!   **Throws** (hold a pad to push a control somewhere; it springs back on
//!   release). Holding **L1** peeks at Controls from any layer.
//! - **Knobs** turn the current hero pair; knob 1 press = next pair, knob 2
//!   press = reset the pair. **D-pad up/down** steps the app's "browse"
//!   control (engine, preset, algorithm...); left/right nudge knob 2.
//! - **Stick, hands, mod wheel, aftertouch, pad pressure** push routed
//!   controls away from their knob setting without overwriting it (see
//!   `Expression`). A stick click keeps where you've pushed to.
//! - **Hold L1 and move the stick to set a dial.** With a dial selected, L1
//!   plus the joystick sets it directly: pushed fully left is 0%, fully
//!   right is 100%, straight down or up is 50%. Let go of the stick and the
//!   value stays; it never snaps back to the middle. Holding L1 also shows
//!   the Controls layer (a peek), and tapping a control's pad while it's
//!   held picks that control instead.
//! - **Binding by wiggling.** There are no dials to assign things with, so
//!   on the Controls layer the player grabs a control (tap its pad), then
//!   *moves the source they want on it*: sweep the stick, wave a hand, or
//!   lean hard into the control's own pad for pressure. That source now
//!   pushes that control (moving it off whatever it pushed before; doing
//!   it again unbinds). Bindings are saved per app.
//!
//! The app describes itself through `PlayHost` (its controls in order of
//! importance) and a `KitConfig`; this module owns all the shared
//! behaviour, the play column on screen and the LED colours.

// Reached through app.rs, so every preview binary compiles this module,
// and most of those include only one app (or none that plays); what a
// given binary leaves unused here is expected, not dead.
#![allow(dead_code)]

use crate::app::{Input, PlayColumn, PlayDial};
use crate::led_output::PadColor;
use std::path::PathBuf;
use std::time::Instant;

/// MIDI notes below this are left alone when a keyboard plays an app's
/// own pads -- the same floor the shell's fallback pad mapping uses, so
/// controllers' encoder-touch notes (0..20) never press a pad.
const MIN_MIDI_PAD_NOTE: usize = 21;
/// Holding a Moments pad this long stores instead of recalling.
pub const STORE_HOLD_S: f32 = 0.6;
/// Stick travel moves a routed control by up to this much each way.
const STICK_SPAN: f32 = 0.5;
/// A hand fully in the beam moves its control by this much.
const HAND_SPAN: f32 = 0.6;
/// Full pad pressure moves its control by this much.
const PRESSURE_SPAN: f32 = 0.6;
/// While L1 is held, the stick must be pushed at least this far from
/// centre to set the dial; inside it, the last value is simply kept.
const SET_ENGAGE: f32 = 0.35;
/// A source must travel this far from where it was when a control was
/// grabbed to bind (or unbind) it, and come back within `LEARN_REARM`
/// before it can do so again.
const LEARN_THRESHOLD: f32 = 0.35;
const LEARN_REARM: f32 = 0.12;
/// Leaning on the grabbed control's own pad at least this hard, for
/// `LEAN_FRAMES` frames (~0.33 s), binds pressure to it.
const LEAN_PRESSURE: f32 = 0.85;
const LEAN_FRAMES: u32 = 20;
/// Depth-sensor noise floor: below it, no hand.
pub const HAND_FLOOR: f32 = 0.02;
/// Frames a status message stays up (~1.5 s at 60 fps).
const STATUS_FRAMES: u32 = 90;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    /// The app's own pad behaviour; the id lets an app have several
    /// (Plaits' Notes and Chords, the Sequencer's Steps and Perform).
    Native(u8, &'static str),
    Controls,
    Moments,
    Throws,
}

impl Layer {
    pub fn label(self) -> &'static str {
        match self {
            Layer::Native(_, l) => l,
            Layer::Controls => "CONTROLS",
            Layer::Moments => "MOMENTS",
            Layer::Throws => "THROWS",
        }
    }
}

/// A Throws pad: while held, `control` sits at `to` (0..1).
#[derive(Clone, Copy, Debug)]
pub struct Throw {
    pub control: usize,
    pub to: f32,
    pub label: &'static str,
}

/// Which control (index into the app's list) each surface pushes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Routes {
    pub stick_x: Option<usize>,
    pub stick_y: Option<usize>,
    pub hand_l: Option<usize>,
    pub hand_r: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct KitConfig {
    /// Folder name under `saves/` for this app's moments.
    pub app_id: &'static str,
    /// Pad layers in F2 order; the first is what the pads do on entry.
    pub layers: Vec<Layer>,
    /// Pairs of controls the knobs turn, cycled by knob 1 press.
    pub hero: Vec<[usize; 2]>,
    /// The control D-pad up/down steps.
    pub browse: Option<usize>,
    pub routes: Routes,
    pub throws: Vec<Throw>,
    /// A MIDI keyboard presses the app's pads (note % 16) on a native
    /// layer. Off for apps that read `Input::midi_keys` themselves.
    pub midi_to_pads: bool,
    /// The app handles stick/hands itself (Plaits renders them in DSP);
    /// the kit only shows them.
    pub own_expression: bool,
}

/// What the app tells the kit about itself. Controls are indexed in order
/// of importance: the first ones land on the knobs and the bottom pads.
pub trait PlayHost {
    /// Whether the pads on native layer `layer` are being *played* (so
    /// pressing them is musical pressure) rather than used to pick
    /// things, as Atlas's STATES pads are. Pad pressure only drives its
    /// route while this is true.
    fn kit_pads_play(&self, _layer: u8) -> bool {
        true
    }
    fn kit_control_count(&self) -> usize;
    fn kit_label(&self, i: usize) -> String;
    fn kit_value(&self, i: usize) -> String;
    /// 0..1 position for dials and expression; None for actions/choices
    /// that have no meaningful position.
    fn kit_norm(&self, i: usize) -> Option<f32>;
    /// Stepped choices (a waveform, a mode) are shown on dials but never
    /// pushed by expression, which needs a continuous control.
    fn kit_stepped(&self, _i: usize) -> bool {
        false
    }
    fn kit_edit(&mut self, i: usize, delta: i32);
    fn kit_reset(&mut self, i: usize);
    /// Set a control from a 0..1 position (expression, throws, moments).
    fn kit_set_norm(&mut self, i: usize, v: f32);
    /// A whole sound, for Moments. Default: every control's position.
    fn kit_snapshot(&self) -> serde_json::Value {
        serde_json::Value::Array((0..self.kit_control_count()).map(|i| self.kit_norm(i).map_or(serde_json::Value::Null, |v| serde_json::json!(v))).collect())
    }
    fn kit_recall(&mut self, v: &serde_json::Value) {
        if let Some(items) = v.as_array() {
            for (i, item) in items.iter().enumerate().take(self.kit_control_count()) {
                if let Some(x) = item.as_f64() {
                    self.kit_set_norm(i, (x as f32).clamp(0.0, 1.0));
                }
            }
        }
    }
    /// One line of what's happening (notes sounding, step, level...).
    fn kit_line(&self) -> String {
        String::new()
    }
    /// A pad's label on one of the app's own layers (physical index).
    fn kit_pad_label(&self, _layer: u8, _pad: usize) -> String {
        String::new()
    }
    /// Which pad a MIDI keyboard note presses on a native layer (with
    /// `midi_to_pads`). Default: the shell's legacy note % 16. Apps whose
    /// pads are pitched override this so a key plays its own pitch.
    fn kit_midi_pad(&self, note: u8) -> Option<usize> {
        (note as usize >= MIN_MIDI_PAD_NOTE).then_some(note as usize % 16)
    }
    /// A pad's colour on one of the app's own layers.
    fn kit_pad_color(&self, _layer: u8, _pad: usize, held: bool) -> PadColor {
        if held { PadColor::Green } else { PadColor::Off }
    }
}

/// What the app should do with this frame after the kit took its share.
pub struct Step {
    /// True while the full menu is showing: the app runs its list.
    pub menu: bool,
    /// The app's own pad layer, if that's what the pads are right now.
    pub native: Option<u8>,
    /// The input for the app: knobs/D-pad already consumed on the play
    /// view, pads cleared unless a native layer has them, MIDI keys folded
    /// into pads when configured.
    pub input: Input,
}

#[derive(Default)]
pub struct PlayKit {
    pub cfg: KitConfig,
    pub menu: bool,
    layer: usize,
    hero_page: usize,
    /// Which dial of the hero pair the D-pad turns (0 or 1).
    focus_side: usize,
    pub focused: usize,
    /// The offset currently applied to each control by expression/throws.
    applied: Vec<f32>,
    /// A stepped control's own setting while a throw holds it elsewhere.
    throw_orig: Vec<Option<f32>>,
    /// After a stick click keeps a sound, the stick is ignored until it
    /// springs back to centre -- otherwise letting go would push the
    /// sound the other way.
    stick_kept: bool,
    moments: Vec<Option<serde_json::Value>>,
    moments_path: Option<PathBuf>,
    pad_since: [Option<Instant>; 16],
    pad_stored: [bool; 16],
    prev_grid: [bool; 16],
    peek: bool,
    status: String,
    status_frames: u32,
    stick: [f32; 2],
    hands: [f32; 2],
    /// The routes in force: the app's own (`cfg.routes`) until the player
    /// rebinds one, or the saved set if there is one.
    routes: Routes,
    /// Which control pad pressure pushes (not part of `Routes`, which
    /// every app already builds by hand).
    pressure_route: Option<usize>,
    routes_path: Option<PathBuf>,
    /// A saved set was found, so an app's suggested defaults don't apply.
    routes_loaded: bool,
    /// Stick x/y and hands l/r when the current control was grabbed.
    learn_base: [f32; 4],
    learn_armed: [bool; 4],
    lean_frames: u32,
    /// A control tapped on the peeked Controls layer this L1 hold; it is
    /// what the stick sets instead of the Play view's selected dial.
    peek_grab: Option<usize>,
    /// The stick is setting a dial this frame, so it isn't also pushing
    /// its routes.
    setting: bool,
}

/// Where each source is bound, for saving.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct SavedRoutes {
    stick_x: Option<usize>,
    stick_y: Option<usize>,
    hand_l: Option<usize>,
    hand_r: Option<usize>,
    pressure: Option<usize>,
}

/// Where moments and bindings are saved: the project's `saves/` folder,
/// unless `PORTAMAX_SAVES_DIR` points elsewhere (screenshot and demo runs
/// use a scratch folder so they never touch the player's own saves).
fn saves_root() -> PathBuf {
    std::env::var_os("PORTAMAX_SAVES_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/saves")))
}

const SOURCE_NAMES: [&str; 4] = ["Stick X", "Stick Y", "Left hand", "Right hand"];

impl PlayKit {
    /// `persist`: false keeps moments in memory (tests, previews).
    pub fn new(cfg: KitConfig, persist: bool) -> PlayKit {
        let path = persist.then(|| saves_root().join(cfg.app_id).join("moments.json"));
        let mut moments = vec![None; 16];
        if let Some(text) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            if let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(&text) {
                for (i, item) in items.into_iter().take(16).enumerate() {
                    moments[i] = (!item.is_null()).then_some(item);
                }
            }
        }
        let routes_path = persist.then(|| saves_root().join(cfg.app_id).join("routes.json"));
        let mut routes = cfg.routes;
        let mut pressure_route = None;
        let mut routes_loaded = false;
        if let Some(saved) = routes_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<SavedRoutes>(&t).ok()) {
            routes = Routes { stick_x: saved.stick_x, stick_y: saved.stick_y, hand_l: saved.hand_l, hand_r: saved.hand_r };
            pressure_route = saved.pressure;
            routes_loaded = true;
        }
        PlayKit { cfg, moments, moments_path: path, routes, pressure_route, routes_path, routes_loaded, learn_armed: [true; 4], ..Default::default() }
    }

    /// An app's suggestion for what pad pressure should push, used only
    /// until the player (or a saved set) says otherwise.
    pub fn suggest_pressure_route(&mut self, control: usize) {
        if !self.routes_loaded && self.pressure_route.is_none() {
            self.pressure_route = Some(control);
        }
    }

    /// Where pad pressure is routed right now.
    pub fn pressure_route(&self) -> Option<usize> {
        self.pressure_route
    }

    pub fn layer(&self) -> Layer {
        if self.peek {
            return Layer::Controls;
        }
        self.cfg.layers.get(self.layer).copied().unwrap_or(Layer::Controls)
    }

    pub fn layer_label(&self) -> &'static str {
        self.cfg.layers.get(self.layer).map_or("CONTROLS", |l| l.label())
    }

    pub fn next_layer(&mut self) {
        if !self.cfg.layers.is_empty() {
            self.layer = (self.layer + 1) % self.cfg.layers.len();
            self.flash(format!("Pads: {}", self.layer_label()));
        }
    }

    /// Jump straight to a native layer (an app whose own mode button
    /// changes what the pads do keeps F2 and this in step).
    pub fn set_native(&mut self, id: u8) {
        if let Some(i) = self.cfg.layers.iter().position(|l| matches!(l, Layer::Native(n, _) if *n == id)) {
            self.layer = i;
        }
    }

    pub fn flash(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_frames = STATUS_FRAMES;
    }

    pub fn status(&self) -> &str {
        if self.status_frames > 0 { &self.status } else { "" }
    }

    #[allow(dead_code)] // tests and previews read stored moments
    pub fn moment(&self, i: usize) -> Option<&serde_json::Value> {
        self.moments.get(i).and_then(|m| m.as_ref())
    }

    fn hero_pair(&self) -> Option<[usize; 2]> {
        self.cfg.hero.get(self.hero_page % self.cfg.hero.len().max(1)).copied()
    }

    /// The control a MIDI controller's encoder 2 turns: the pair's second
    /// dial, or the grabbed pad on Controls.
    pub fn knob2_control(&self) -> Option<usize> {
        if self.layer() == Layer::Controls { Some(self.focused) } else { self.hero_pair().map(|p| p[1]) }
    }

    /// The focused dial: what the D-pad's left/right turns and holding
    /// SELECT resets (the grabbed pad on Controls).
    pub fn focus_control(&self) -> Option<usize> {
        // During an L1 hold the dial, the lit pad, the D-pad and the stick
        // all mean the same control: the one the stick sets.
        if self.peek {
            return self.set_target();
        }
        if self.layer() == Layer::Controls { Some(self.focused) } else { self.hero_pair().map(|p| p[self.focus_side.min(1)]) }
    }

    pub fn tick(&mut self, host: &mut dyn PlayHost, input: &Input) -> Step {
        let n = host.kit_control_count();
        self.applied.resize(n, 0.0);
        self.throw_orig.resize(n, None);
        if self.status_frames > 0 {
            self.status_frames -= 1;
        }
        if input.shoulder_press[1] {
            self.menu = !self.menu;
            self.flash(if self.menu { "Menu  (R1: back to play)" } else { "Play" });
        }
        self.peek = input.shoulders[0];
        if !self.peek {
            self.peek_grab = None;
        }
        self.setting = false;
        let layer = self.layer();
        let mut out = *input;
        out.shoulder_press = [false; 2];

        if !self.menu {
            self.play_controls(host, input, layer);
            out.knob1 = 0;
            out.knob2 = 0;
            out.knob1_press = false;
            out.knob2_press = false;
            out.navigation_steps = 0;
        }

        // A MIDI keyboard plays the app's own pads, held for as long as
        // the key is (the shell's fallback only pulses them).
        if self.cfg.midi_to_pads && matches!(layer, Layer::Native(..)) {
            for (k, &v) in input.midi_keys.0.iter().enumerate() {
                if v > 0 {
                    if let Some(pad) = host.kit_midi_pad(k as u8).filter(|&p| p < 16) {
                        out.grid[pad] = true;
                    }
                }
            }
        }

        let native = match layer {
            Layer::Native(id, _) => Some(id),
            _ => {
                self.kit_pads(host, input, layer, n);
                out.grid = [false; 16];
                None
            }
        };
        // Kit layers need the raw pads for edges; native ones the pads
        // the app actually saw, for the held colour.
        self.prev_grid = if native.is_some() { out.grid } else { input.grid };

        self.stick = input.stick;
        self.hands = input.hands;
        if self.peek && !self.menu {
            self.set_by_stick(host, input, n);
        }
        if !self.cfg.own_expression {
            self.expression(host, input, layer, n);
            if !self.peek && layer == Layer::Controls && !self.menu {
                self.learn(host, input, n);
            }
        }
        Step { menu: self.menu, native, input: out }
    }

    /// The play view's controls. Portamax has no encoders: one dial is
    /// focused, the D-pad's left/right turns it (`nav_x`), SELECT moves the
    /// focus on (`knob1_press`: the pair's other dial, then the next pair)
    /// and holding SELECT resets it (`knob2_press`). Up/down steps the
    /// app's browse control. A MIDI controller's two encoders, if one is
    /// plugged in, still turn the pair's two dials directly.
    fn play_controls(&mut self, host: &mut dyn PlayHost, input: &Input, layer: Layer) {
        if input.knob1_press && !self.cfg.hero.is_empty() {
            if self.focus_side == 0 && layer != Layer::Controls {
                self.focus_side = 1;
            } else {
                self.focus_side = 0;
                self.hero_page = (self.hero_page + 1) % self.cfg.hero.len();
            }
            if let Some(c) = self.focus_control() {
                self.flash(format!("Dial: {}", host.kit_label(c)));
            }
        }
        if input.knob2_press {
            if let Some(c) = self.focus_control() {
                host.kit_reset(c);
                self.flash(format!("Reset {}", host.kit_label(c)));
            }
        }
        if let Some(c) = self.focus_control() {
            if input.nav_x != 0 {
                host.kit_edit(c, input.nav_x);
            }
        }
        // encoders (the D-pad's share of knob2 is taken out above)
        let encoder2 = input.knob2 - input.nav_x;
        if let Some([a, _]) = self.hero_pair() {
            if input.knob1 != 0 && layer != Layer::Controls {
                host.kit_edit(a, input.knob1);
            }
        }
        if let Some(c) = self.knob2_control() {
            if encoder2 != 0 {
                host.kit_edit(c, encoder2);
            }
        }
        // D-pad up (navigation_steps -1) = next.
        if let Some(b) = self.cfg.browse {
            if input.navigation_steps != 0 {
                host.kit_edit(b, -input.navigation_steps);
                self.flash(host.kit_value(b));
            }
        }
    }

    /// Controls / Moments / Throws pads.
    fn kit_pads(&mut self, host: &mut dyn PlayHost, input: &Input, layer: Layer, n: usize) {
        let now = Instant::now();
        for i in 0..16 {
            let (down, was) = (input.grid[i], self.prev_grid[i]);
            let rank = pad_rank(i);
            match layer {
                Layer::Controls if down && !was && rank < n => {
                    self.focused = rank;
                    if self.peek {
                        self.peek_grab = Some(rank);
                    }
                    self.learn_base = [input.stick[0], input.stick[1], input.hands[0], input.hands[1]];
                    self.learn_armed = [true; 4];
                    self.lean_frames = 0;
                    self.flash(format!("Dial: {}", host.kit_label(rank)));
                }
                Layer::Moments => {
                    if down && !was {
                        self.pad_since[i] = Some(now);
                        self.pad_stored[i] = false;
                    } else if down && !self.pad_stored[i] && self.pad_since[i].is_some_and(|t| t.elapsed().as_secs_f32() >= STORE_HOLD_S) {
                        self.pad_stored[i] = true;
                        self.clear_offsets(host);
                        self.moments[rank] = Some(host.kit_snapshot());
                        match self.save_moments() {
                            Ok(()) => self.flash(format!("Stored moment {}", rank + 1)),
                            Err(e) => self.flash(format!("Couldn't save moment: {e}")),
                        }
                    } else if !down && was {
                        if !self.pad_stored[i] {
                            match self.moments[rank].clone() {
                                Some(m) => {
                                    self.clear_offsets(host);
                                    host.kit_recall(&m);
                                    self.flash(format!("Moment {}", rank + 1));
                                }
                                None => self.flash(format!("Moment {} is empty: hold to store", rank + 1)),
                            }
                        }
                        self.pad_since[i] = None;
                    }
                }
                _ => {}
            }
        }
    }

    /// Puts every control back on its knob setting (before a snapshot or
    /// recall, so moments never capture a passing gesture).
    fn clear_offsets(&mut self, host: &mut dyn PlayHost) {
        for i in 0..self.applied.len() {
            if self.applied[i] != 0.0 {
                if let Some(cur) = host.kit_norm(i) {
                    host.kit_set_norm(i, (cur - self.applied[i]).clamp(0.0, 1.0));
                }
                self.applied[i] = 0.0;
            }
        }
    }

    /// Stick, hands, mod wheel, aftertouch and held throws as offsets on
    /// top of each control's own setting. The base is always recovered as
    /// "current minus what we added", so turning a knob mid-gesture moves
    /// the base, and letting go returns exactly to it.
    ///
    /// Values are written at frame rate (~60 Hz); apps that read their
    /// controls once per audio block hear that as fine steps on fast
    /// gestures. Plaits smooths in its DSP; others may want to as well.
    fn expression(&mut self, host: &mut dyn PlayHost, input: &Input, layer: Layer, n: usize) {
        let r = self.routes;
        if input.stick_click {
            // Keep: whatever is applied becomes the new setting.
            self.applied.iter_mut().for_each(|a| *a = 0.0);
            self.stick_kept = true;
            self.flash("Kept the stick's sound");
        }
        if input.stick[0].abs() < 0.08 && input.stick[1].abs() < 0.08 {
            self.stick_kept = false;
        }
        let stick = if self.stick_kept || self.setting { [0.0; 2] } else { input.stick };
        let mut want = vec![0.0f32; n];
        let mut fixed: Vec<Option<f32>> = vec![None; n];
        let mut add = |c: Option<usize>, v: f32| {
            if let Some(c) = c.filter(|&c| c < n) {
                want[c] += v;
            }
        };
        add(r.stick_x, stick[0] * STICK_SPAN);
        add(r.stick_y, stick[1] * STICK_SPAN);
        let hand = |v: f32| if v > HAND_FLOOR { v * HAND_SPAN } else { 0.0 };
        add(r.hand_l, hand(input.hands[0]));
        add(r.hand_r, hand(input.hands[1]));
        // A keyboard's wheel and pressure follow the stick's routing.
        add(r.stick_y, input.mod_wheel * STICK_SPAN);
        add(r.stick_x, input.aftertouch * STICK_SPAN);
        // The firmest pad being played pushes the pressure route. (Pads
        // are only "played" on a native layer whose pads make sound.)
        if let Layer::Native(id, _) = layer {
            if host.kit_pads_play(id) {
                let firmest = input.pad_pressure.iter().zip(input.grid.iter()).filter(|(_, &g)| g).map(|(&p, _)| p).fold(0.0f32, f32::max);
                add(self.pressure_route, firmest * PRESSURE_SPAN);
            }
        }
        if layer == Layer::Throws {
            for (i, t) in self.cfg.throws.iter().enumerate().take(16) {
                if input.grid[rank_pad(i)] && t.control < n {
                    fixed[t.control] = Some(t.to);
                }
            }
        }
        for c in 0..n {
            if want[c] == 0.0 && fixed[c].is_none() && self.applied[c] == 0.0 && self.throw_orig[c].is_none() {
                continue;
            }
            if host.kit_stepped(c) {
                // Stepped choices (freeze, a mode) can be thrown -- held at
                // the throw's value, put back exactly on release -- but are
                // never pushed by the stick or hands.
                match (fixed[c], self.throw_orig[c]) {
                    (Some(to), orig) => {
                        if orig.is_none() {
                            self.throw_orig[c] = host.kit_norm(c);
                        }
                        host.kit_set_norm(c, to);
                    }
                    (None, Some(orig)) => {
                        host.kit_set_norm(c, orig);
                        self.throw_orig[c] = None;
                    }
                    (None, None) => {}
                }
                continue;
            }
            let Some(cur) = host.kit_norm(c) else { continue };
            let base = (cur - self.applied[c]).clamp(0.0, 1.0);
            let new = fixed[c].unwrap_or(base + want[c]).clamp(0.0, 1.0);
            if (new - cur).abs() > 1e-5 {
                host.kit_set_norm(c, new);
            }
            // What actually landed, so a stepped setter can't drift the base.
            let landed = host.kit_norm(c).unwrap_or(new);
            self.applied[c] = landed - base;
        }
    }

    /// The dial L1 + stick sets: a control tapped during this hold, else
    /// the grabbed one if the Controls layer is really showing, else the
    /// dial selected on the Play view.
    fn set_target(&self) -> Option<usize> {
        self.peek_grab.or_else(|| {
            if matches!(self.cfg.layers.get(self.layer), Some(Layer::Controls)) {
                Some(self.focused)
            } else {
                self.hero_pair().map(|p| p[self.focus_side.min(1)])
            }
        })
    }

    /// Hold L1 and push the stick: the selected dial follows the stick's
    /// left-right position, 0% at the far left to 100% at the far right.
    /// Inside the dead zone nothing changes, so the spring-return stick
    /// leaves the value where it was put instead of snapping to 50%.
    fn set_by_stick(&mut self, host: &mut dyn PlayHost, input: &Input, n: usize) {
        // For the whole of an L1 hold the stick belongs to this: it must
        // not also push its routed controls, nor jump them when L1 is let
        // go with the stick still over.
        self.setting = true;
        let [x, y] = input.stick;
        if x.hypot(y) > 0.08 {
            self.stick_kept = true;
        }
        let Some(c) = self.set_target().filter(|&c| c < n) else { return };
        if x.hypot(y) < SET_ENGAGE {
            return;
        }
        if host.kit_stepped(c) {
            self.flash(format!("{} is a choice: use the D-pad", host.kit_label(c)));
            return;
        }
        // Take expression's offsets off first so what's written is the base.
        self.clear_offsets(host);
        host.kit_set_norm(c, ((x + 1.0) / 2.0).clamp(0.0, 1.0));
        self.flash(format!("{} {}", host.kit_label(c), host.kit_value(c)));
    }

    /// Bind-by-wiggling on the Controls layer: whichever source travels
    /// furthest from where it was when the control was grabbed is bound to
    /// it (or unbound, if it already was), and leaning hard on the
    /// control's own pad binds pressure.
    fn learn(&mut self, host: &mut dyn PlayHost, input: &Input, n: usize) {
        let c = self.focused;
        if c >= n {
            return;
        }
        let now = [input.stick[0], input.stick[1], input.hands[0], input.hands[1]];
        for s in 0..4 {
            let d = (now[s] - self.learn_base[s]).abs();
            if d < LEARN_REARM {
                self.learn_armed[s] = true;
            } else if d > LEARN_THRESHOLD && self.learn_armed[s] {
                self.learn_armed[s] = false;
                self.bind_source(host, s, c);
            }
        }
        let pad = rank_pad(c);
        if input.grid[pad] && input.pad_pressure[pad] >= LEAN_PRESSURE {
            self.lean_frames += 1;
            if self.lean_frames == LEAN_FRAMES {
                self.bind_pressure(host, c);
            }
        } else {
            self.lean_frames = 0;
        }
    }

    fn bind_source(&mut self, host: &mut dyn PlayHost, source: usize, control: usize) {
        if host.kit_stepped(control) {
            self.flash(format!("{} is a choice: nothing can push it", host.kit_label(control)));
            return;
        }
        // Put everything back on its base before a route moves, so the old
        // target isn't left pushed.
        self.clear_offsets(host);
        let slot = match source {
            0 => &mut self.routes.stick_x,
            1 => &mut self.routes.stick_y,
            2 => &mut self.routes.hand_l,
            _ => &mut self.routes.hand_r,
        };
        let label = host.kit_label(control);
        if *slot == Some(control) {
            *slot = None;
            self.flash(format!("{} no longer pushes {label}", SOURCE_NAMES[source]));
        } else {
            *slot = Some(control);
            self.flash(format!("{} now pushes {label}", SOURCE_NAMES[source]));
        }
        self.save_routes();
    }

    fn bind_pressure(&mut self, host: &mut dyn PlayHost, control: usize) {
        if host.kit_stepped(control) {
            self.flash(format!("{} is a choice: nothing can push it", host.kit_label(control)));
            return;
        }
        self.clear_offsets(host);
        let label = host.kit_label(control);
        if self.pressure_route == Some(control) {
            self.pressure_route = None;
            self.flash(format!("Pressure no longer pushes {label}"));
        } else {
            self.pressure_route = Some(control);
            self.flash(format!("Pressure now pushes {label}"));
        }
        self.save_routes();
    }

    fn save_routes(&self) {
        let Some(path) = &self.routes_path else { return };
        let saved = SavedRoutes {
            stick_x: self.routes.stick_x,
            stick_y: self.routes.stick_y,
            hand_l: self.routes.hand_l,
            hand_r: self.routes.hand_r,
            pressure: self.pressure_route,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        // A failed save only loses the binding on the next launch.
        std::fs::write(path, serde_json::to_string_pretty(&saved).unwrap_or_default()).ok();
    }

    fn save_moments(&self) -> Result<(), String> {
        let Some(path) = &self.moments_path else { return Ok(()) };
        let arr = serde_json::Value::Array(self.moments.iter().map(|m| m.clone().unwrap_or(serde_json::Value::Null)).collect());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&arr).unwrap_or_default()).map_err(|e| e.to_string())
    }

    /// Pad colour, for the controller LEDs and the on-screen map alike.
    pub fn pad_color(&self, host: &dyn PlayHost, pad: usize) -> PadColor {
        let rank = pad_rank(pad);
        let held = self.prev_grid[pad];
        match self.layer() {
            Layer::Native(id, _) => host.kit_pad_color(id, pad, held),
            Layer::Controls => {
                // While L1 is held the lit pad is the dial the joystick will set.
                let shown = if self.peek { self.set_target().unwrap_or(self.focused) } else { self.focused };
                if rank == shown { PadColor::Green } else if rank < host.kit_control_count() { PadColor::Blue } else { PadColor::Off }
            }
            Layer::Moments => {
                if held { PadColor::Red } else if self.moments.get(rank).is_some_and(|m| m.is_some()) { PadColor::Green } else { PadColor::Off }
            }
            Layer::Throws => {
                if rank >= self.cfg.throws.len() { PadColor::Off } else if held { PadColor::Red } else { PadColor::Yellow }
            }
        }
    }

    pub fn led_overlay(&self, host: &dyn PlayHost) -> [PadColor; 16] {
        std::array::from_fn(|i| self.pad_color(host, i))
    }

    pub fn pad_label(&self, host: &dyn PlayHost, pad: usize) -> String {
        let rank = pad_rank(pad);
        match self.layer() {
            Layer::Native(id, _) => host.kit_pad_label(id, pad),
            Layer::Controls => if rank < host.kit_control_count() { host.kit_label(rank) } else { String::new() },
            Layer::Moments => {
                if self.moments.get(rank).is_some_and(|m| m.is_some()) { format!("{} ●", rank + 1) } else { format!("{} ·", rank + 1) }
            }
            Layer::Throws => self.cfg.throws.get(rank).map_or(String::new(), |t| t.label.to_string()),
        }
    }

    fn route_label(host: &dyn PlayHost, c: Option<usize>) -> String {
        c.filter(|&c| c < host.kit_control_count()).map_or("-".into(), |c| host.kit_label(c))
    }

    /// Everything the play column shows. `own` overrides the expression
    /// labels for an app that renders expression itself.
    pub fn column(&self, host: &dyn PlayHost) -> PlayColumn {
        let pair = self.hero_pair();
        let k2 = self.focus_control();
        // Four dials: this page's pair and its neighbour, so the screen
        // doesn't jump on every knob-1 press.
        let pages = self.cfg.hero.len();
        let first = if pages == 0 { 0 } else { (self.hero_page % pages) & !1 };
        let mut dial_controls: Vec<usize> = Vec::new();
        for p in first..(first + 2).min(pages) {
            dial_controls.extend_from_slice(&self.cfg.hero[p]);
        }
        let dials = dial_controls
            .iter()
            .map(|&c| PlayDial {
                label: host.kit_label(c),
                value: host.kit_value(c),
                norm: host.kit_norm(c).unwrap_or(0.0).clamp(0.0, 1.0),
                // 2 = the focused dial (the D-pad turns it), 1 = its partner
                knob: if k2 == Some(c) { 2 } else if pair.is_some_and(|p| p.contains(&c)) { 1 } else { 0 },
            })
            .collect();
        let r = self.routes;
        PlayColumn {
            layer: self.layer().label().to_string(),
            dials,
            knob2_extra: k2.filter(|c| !dial_controls.contains(c)).map(|c| format!("{}  {}", host.kit_label(c), host.kit_value(c))).unwrap_or_default(),
            pad_labels: (0..16).map(|i| self.pad_label(host, i)).collect(),
            pad_state: (0..16)
                .map(|i| match self.pad_color(host, i) {
                    PadColor::Off => 0,
                    PadColor::Blue => 1,
                    PadColor::Yellow => 2,
                    PadColor::Green => 3,
                    PadColor::Red => 4,
                })
                .collect(),
            stick: self.stick,
            stick_label: format!(
                "{} / {}{}",
                Self::route_label(host, r.stick_x),
                Self::route_label(host, r.stick_y),
                self.pressure_route.map(|c| format!(" · press {}", host.kit_label(c))).unwrap_or_default()
            ),
            hands: self.hands,
            hand_labels: [Self::route_label(host, r.hand_l), Self::route_label(host, r.hand_r)],
            line: host.kit_line(),
            status: self.status().to_string(),
        }
    }
}

/// One control's storage, so an app can describe its controls once and
/// get position, setting and steppedness for free (see `PlayHost`).
/// Atomics are shared with the audio thread, so setting through `&self`
/// is the normal path in this codebase.
pub enum Knob<'a> {
    /// A continuous value in min..max.
    F(&'a crate::util::AtomicF32, f32, f32),
    /// A choice among `count` options (0..count).
    U(&'a std::sync::atomic::AtomicU32, u32),
    /// An integer stepped value in min..=max (unison count, octave).
    I(&'a std::sync::atomic::AtomicI32, i32, i32),
    /// An unsigned stepped value in min..=max.
    UR(&'a std::sync::atomic::AtomicU32, u32, u32),
    B(&'a std::sync::atomic::AtomicBool),
    /// No position (an action).
    None,
}

impl Knob<'_> {
    pub fn norm(&self) -> Option<f32> {
        use std::sync::atomic::Ordering::Relaxed;
        let frac = |v: f32, lo: f32, hi: f32| if hi > lo { ((v - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
        Some(match self {
            Knob::F(a, lo, hi) => frac(a.get(), *lo, *hi),
            Knob::U(a, n) => frac((a.load(Relaxed) % (*n).max(1)) as f32, 0.0, (*n).saturating_sub(1) as f32),
            Knob::I(a, lo, hi) => frac(a.load(Relaxed) as f32, *lo as f32, *hi as f32),
            Knob::UR(a, lo, hi) => frac(a.load(Relaxed) as f32, *lo as f32, *hi as f32),
            Knob::B(a) => a.load(Relaxed) as u8 as f32,
            Knob::None => return None,
        })
    }
    pub fn set(&self, v: f32) {
        use std::sync::atomic::Ordering::Relaxed;
        let v = v.clamp(0.0, 1.0);
        match self {
            Knob::F(a, lo, hi) => a.set(lo + v * (hi - lo)),
            Knob::U(a, n) => a.store((v * (*n).saturating_sub(1) as f32).round() as u32, Relaxed),
            Knob::I(a, lo, hi) => a.store(lo + (v * (hi - lo) as f32).round() as i32, Relaxed),
            Knob::UR(a, lo, hi) => a.store(lo + (v * (hi - lo) as f32).round() as u32, Relaxed),
            Knob::B(a) => a.store(v >= 0.5, Relaxed),
            Knob::None => {}
        }
    }
    pub fn stepped(&self) -> bool {
        !matches!(self, Knob::F(..))
    }
}

/// Pads are row-major with row 0 on top; musically rank 0 is bottom-left
/// and rank 15 top-right. Its own inverse.
pub fn pad_rank(pad: usize) -> usize {
    (3 - pad / 4) * 4 + pad % 4
}

/// Physical pad of a rank (same flip).
pub fn rank_pad(rank: usize) -> usize {
    pad_rank(rank)
}

/// The play column drawn with embedded-graphics, for the device screen
/// (the bin). Same content as the Slint column, in the app's palette.
pub mod draw {
    use crate::app::PlayColumn;
    use crate::display::FrameBuffer;
    use crate::spleen_fonts::{SPLEEN_6X12, SPLEEN_8X16};
    use embedded_graphics::mono_font::MonoTextStyle;
    use embedded_graphics::pixelcolor::Rgb565;
    use embedded_graphics::prelude::*;
    use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle, Rectangle};
    use embedded_graphics::text::Text;

    #[derive(Clone, Copy)]
    pub struct Palette {
        pub bg: Rgb565,
        pub ink: Rgb565,
        pub accent: Rgb565,
        pub dim: Rgb565,
        /// A barely-there fill for unlit cells and tracks.
        pub faint: Rgb565,
    }

    fn fill(fb: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, c: Rgb565) {
        if w > 0 && h > 0 {
            Rectangle::new(Point::new(x, y), Size::new(w as u32, h as u32)).into_styled(PrimitiveStyle::with_fill(c)).draw(fb).ok();
        }
    }

    fn clip(s: &str, max: usize) -> String {
        s.chars().take(max).collect()
    }

    /// Draws into (x, y, w, h); designed for the ~300 px left column the
    /// apps' menus use, but scales its pad grid to the width given.
    pub fn column(fb: &mut FrameBuffer, col: &PlayColumn, x: i32, y: i32, w: i32, h: i32, p: Palette) {
        let ink = MonoTextStyle::new(&SPLEEN_6X12, p.ink);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, p.dim);
        let on_accent = MonoTextStyle::new(&SPLEEN_6X12, p.bg);
        let big = MonoTextStyle::new(&SPLEEN_8X16, p.accent);
        fill(fb, x, y, w, h, p.bg);

        let chip = format!("PADS: {}", col.layer);
        fill(fb, x, y, chip.len() as i32 * 6 + 8, 14, p.accent);
        Text::new(&chip, Point::new(x + 4, y + 10), on_accent).draw(fb).ok();
        Text::new("R1 menu", Point::new(x + w - 42, y + 10), dim).draw(fb).ok();

        // Dials as labelled bars, two columns.
        let dw = (w - 8) / 2;
        for (i, d) in col.dials.iter().enumerate().take(4) {
            let dx = x + (i as i32 % 2) * (dw + 8);
            let dy = y + 20 + (i as i32 / 2) * 30;
            let mut lx = dx;
            if d.knob > 0 {
                Circle::new(Point::new(dx, dy), 11).into_styled(PrimitiveStyle::with_fill(p.accent)).draw(fb).ok();
                Text::new(&d.knob.to_string(), Point::new(dx + 3, dy + 9), on_accent).draw(fb).ok();
                lx += 14;
            }
            Text::new(&clip(&d.label, ((dw - (lx - dx)) / 6) as usize), Point::new(lx, dy + 9), ink).draw(fb).ok();
            let v = clip(&d.value, (dw / 6) as usize);
            fill(fb, dx, dy + 13, dw, 4, p.faint);
            fill(fb, dx, dy + 13, (dw as f32 * d.norm) as i32, 4, if d.knob > 0 { p.accent } else { p.dim });
            Text::new(&v, Point::new(dx + dw - v.len() as i32 * 6, dy + 26), dim).draw(fb).ok();
        }
        let mut py = y + 82;
        if !col.knob2_extra.is_empty() {
            Text::new(&clip(&format!("2: {}", col.knob2_extra), (w / 6) as usize), Point::new(x, py + 8), ink).draw(fb).ok();
            py += 12;
        }

        // Pad map.
        let cw = (w - 6) / 4;
        let ch = 24;
        for i in 0..16usize {
            let cx = x + (i % 4) as i32 * (cw + 2);
            let cy = py + (i / 4) as i32 * (ch + 2);
            let st = col.pad_state.get(i).copied().unwrap_or(0);
            let (bg, style) = match st {
                3 | 4 => (p.accent, on_accent),
                1 => (p.faint, ink),
                2 => (p.faint, ink),
                _ => (p.bg, ink),
            };
            fill(fb, cx, cy, cw, ch, bg);
            Rectangle::new(Point::new(cx, cy), Size::new(cw as u32, ch as u32)).into_styled(PrimitiveStyle::with_stroke(p.dim, 1)).draw(fb).ok();
            if st == 2 {
                fill(fb, cx + 1, cy + ch - 4, cw - 2, 3, p.dim);
            }
            let label = col.pad_labels.get(i).cloned().unwrap_or_default();
            Text::new(&clip(&label, ((cw - 4) / 6) as usize), Point::new(cx + 3, cy + 15), style).draw(fb).ok();
        }
        py += 4 * (ch + 2) + 4;

        // Stick and hands.
        let s = 40;
        Rectangle::new(Point::new(x, py), Size::new(s as u32, s as u32)).into_styled(PrimitiveStyle::with_stroke(p.dim, 1)).draw(fb).ok();
        Line::new(Point::new(x + s / 2, py), Point::new(x + s / 2, py + s - 1)).into_styled(PrimitiveStyle::with_stroke(p.faint, 1)).draw(fb).ok();
        Line::new(Point::new(x, py + s / 2), Point::new(x + s - 1, py + s / 2)).into_styled(PrimitiveStyle::with_stroke(p.faint, 1)).draw(fb).ok();
        let dot = Point::new(x + s / 2 + (col.stick[0].clamp(-1.0, 1.0) * 16.0) as i32 - 3, py + s / 2 - (col.stick[1].clamp(-1.0, 1.0) * 16.0) as i32 - 3);
        Circle::new(dot, 7).into_styled(PrimitiveStyle::with_fill(p.accent)).draw(fb).ok();
        Text::new(&clip(&col.stick_label, ((w - s - 60) / 6) as usize), Point::new(x + s + 6, py + 10), dim).draw(fb).ok();
        for (hi, hx) in [(0usize, x + w - 40), (1, x + w - 18)] {
            let v = col.hands[hi].clamp(0.0, 1.0);
            fill(fb, hx, py, 12, s - 12, p.faint);
            let hh = ((s - 12) as f32 * v) as i32;
            fill(fb, hx, py + s - 12 - hh, 12, hh, p.accent);
            Text::new(if hi == 0 { "L" } else { "R" }, Point::new(hx + 3, py + s), dim).draw(fb).ok();
        }
        let line = if !col.status.is_empty() { &col.status } else { &col.line };
        Text::new(&clip(line, ((w - s - 50) / 8) as usize), Point::new(x + s + 6, py + 30), big).draw(fb).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny stand-in app: 6 continuous controls and one stepped.
    struct Fake {
        v: Vec<f32>,
        mode: i32,
    }

    impl PlayHost for Fake {
        fn kit_control_count(&self) -> usize {
            7
        }
        fn kit_label(&self, i: usize) -> String {
            format!("C{i}")
        }
        fn kit_value(&self, i: usize) -> String {
            if i == 6 { self.mode.to_string() } else { format!("{:.2}", self.v[i]) }
        }
        fn kit_norm(&self, i: usize) -> Option<f32> {
            Some(if i == 6 { self.mode as f32 / 3.0 } else { self.v[i] })
        }
        fn kit_stepped(&self, i: usize) -> bool {
            i == 6
        }
        fn kit_edit(&mut self, i: usize, d: i32) {
            if i == 6 { self.mode = (self.mode + d).rem_euclid(4) } else { self.v[i] = (self.v[i] + d as f32 * 0.01).clamp(0.0, 1.0) }
        }
        fn kit_reset(&mut self, i: usize) {
            if i < 6 { self.v[i] = 0.5 }
        }
        fn kit_set_norm(&mut self, i: usize, v: f32) {
            if i == 6 { self.mode = (v * 3.0).round() as i32 } else { self.v[i] = v }
        }
    }

    fn fake() -> Fake {
        Fake { v: vec![0.5; 6], mode: 0 }
    }

    fn kit() -> PlayKit {
        PlayKit::new(
            KitConfig {
                app_id: "test",
                layers: vec![Layer::Native(0, "PLAY"), Layer::Controls, Layer::Moments, Layer::Throws],
                hero: vec![[0, 1], [2, 3]],
                browse: Some(6),
                routes: Routes { stick_x: Some(0), stick_y: Some(1), hand_l: Some(2), hand_r: None },
                throws: vec![Throw { control: 3, to: 1.0, label: "MAX" }],
                midi_to_pads: true,
                own_expression: false,
            },
            false,
        )
    }

    fn pad(rank: usize) -> Input {
        Input { grid: std::array::from_fn(|k| k == rank_pad(rank)), ..Default::default() }
    }

    #[test]
    fn native_layer_passes_pads_and_midi_through_untouched() {
        let (mut k, mut f) = (kit(), fake());
        let s = k.tick(&mut f, &pad(0));
        assert_eq!(s.native, Some(0));
        assert!(s.input.grid[rank_pad(0)]);
        let mut keys = crate::app::MidiKeys::default();
        keys.0[60] = 90;
        let s = k.tick(&mut f, &Input { midi_keys: keys, ..Default::default() });
        assert!(s.input.grid[60 % 16], "a held key holds its pad");
    }

    #[test]
    fn knobs_turn_the_hero_pair_and_r1_hands_them_to_the_menu() {
        let (mut k, mut f) = (kit(), fake());
        let s = k.tick(&mut f, &Input { knob1: 10, knob2: -10, ..Default::default() });
        assert!((f.v[0] - 0.6).abs() < 1e-5 && (f.v[1] - 0.4).abs() < 1e-5);
        assert_eq!(s.input.knob1, 0, "consumed on the play view");
        // SELECT moves the focus: the pair's other dial, then the next pair
        k.tick(&mut f, &Input { knob1_press: true, ..Default::default() });
        k.tick(&mut f, &Input { knob2: 3, nav_x: 3, ..Default::default() });
        assert!((f.v[1] - 0.43).abs() < 1e-5, "the D-pad turns the focused (second) dial: {}", f.v[1]);
        k.tick(&mut f, &Input { knob1_press: true, ..Default::default() });
        k.tick(&mut f, &Input { knob1: 5, ..Default::default() });
        assert!((f.v[2] - 0.55).abs() < 1e-5, "second pair");
        k.tick(&mut f, &Input { knob2: -2, nav_x: -2, ..Default::default() });
        assert!((f.v[2] - 0.53).abs() < 1e-5, "focus is the second pair's first dial");
        k.tick(&mut f, &Input { knob2_press: true, ..Default::default() });
        assert!((f.v[2] - 0.5).abs() < 1e-5, "holding SELECT resets the focused dial");
        k.tick(&mut f, &Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(f.mode, 1, "D-pad up browses");
        let s = k.tick(&mut f, &Input { shoulder_press: [false, true], knob1: 3, ..Default::default() });
        assert!(s.menu && s.input.knob1 == 3, "the menu gets the knobs back");
    }

    #[test]
    fn expression_offsets_return_exactly_to_the_knob() {
        let (mut k, mut f) = (kit(), fake());
        k.tick(&mut f, &Input { stick: [1.0, -1.0], hands: [0.5, 0.0], ..Default::default() });
        assert!((f.v[0] - 1.0).abs() < 1e-5 && (f.v[1] - 0.0).abs() < 1e-5 && (f.v[2] - 0.8).abs() < 1e-5);
        // Turn the knob mid-gesture: the base moves, the offset stays.
        k.tick(&mut f, &Input { stick: [0.2, 0.0], knob1: -10, ..Default::default() });
        assert!((f.v[0] - 0.5).abs() < 1e-5, "0.5 base - 0.1 knob + 0.1 stick = {}", f.v[0]);
        k.tick(&mut f, &Input::default());
        assert!((f.v[0] - 0.4).abs() < 1e-5 && (f.v[1] - 0.5).abs() < 1e-5 && (f.v[2] - 0.5).abs() < 1e-5, "all back home");
        // Click keeps, and the stick's way home doesn't push it back.
        k.tick(&mut f, &Input { stick: [0.4, 0.0], ..Default::default() });
        k.tick(&mut f, &Input { stick: [0.4, 0.0], stick_click: true, ..Default::default() });
        k.tick(&mut f, &Input { stick: [0.1, 0.0], ..Default::default() });
        assert!((f.v[0] - 0.6).abs() < 1e-5, "kept on the way back: {}", f.v[0]);
        k.tick(&mut f, &Input::default());
        k.tick(&mut f, &Input { stick: [0.2, 0.0], ..Default::default() });
        assert!((f.v[0] - 0.7).abs() < 1e-5, "re-centred, the stick works again: {}", f.v[0]);
    }

    #[test]
    fn controls_moments_and_throws_layers() {
        let (mut k, mut f) = (kit(), fake());
        k.next_layer();
        assert_eq!(k.layer_label(), "CONTROLS");
        let s = k.tick(&mut f, &pad(4));
        assert!(s.native.is_none() && !s.input.grid.iter().any(|g| *g), "kit layers keep pads from the app");
        k.tick(&mut f, &Input { knob2: 20, ..Default::default() });
        assert!((f.v[4] - 0.7).abs() < 1e-5, "knob 2 turns the grabbed control");

        k.next_layer(); // Moments
        f.v[5] = 0.9;
        k.tick(&mut f, &pad(2));
        std::thread::sleep(std::time::Duration::from_secs_f32(STORE_HOLD_S + 0.05));
        k.tick(&mut f, &pad(2));
        k.tick(&mut f, &Input::default());
        assert!(k.moment(2).is_some());
        f.v[5] = 0.1;
        k.tick(&mut f, &pad(2));
        k.tick(&mut f, &Input::default());
        assert!((f.v[5] - 0.9).abs() < 1e-5, "tap recalls");

        k.next_layer(); // Throws
        k.tick(&mut f, &pad(0));
        assert_eq!(f.v[3], 1.0, "held throw");
        k.tick(&mut f, &Input::default());
        assert!((f.v[3] - 0.5).abs() < 1e-5, "springs back");
        k.cfg.throws.push(Throw { control: 6, to: 1.0, label: "MODE 3" });
        f.mode = 1;
        k.tick(&mut f, &pad(1));
        assert_eq!(f.mode, 3, "a stepped control can be thrown");
        k.tick(&mut f, &Input::default());
        assert_eq!(f.mode, 1, "and is put back exactly");
    }

    #[test]
    fn l1_peeks_at_controls_and_stepped_controls_ignore_expression() {
        let (mut k, mut f) = (kit(), fake());
        let s = k.tick(&mut f, &Input { shoulders: [true, false], ..pad(1) });
        assert!(s.native.is_none() && k.focused == 1);
        k.tick(&mut f, &Input::default());
        assert_eq!(k.layer_label(), "PLAY");
        k.routes.stick_x = Some(6);
        k.tick(&mut f, &Input { stick: [1.0, 0.0], ..Default::default() });
        assert_eq!(f.mode, 0);
    }

    /// A pad held at a given firmness.
    fn press(rank: usize, firmness: f32) -> Input {
        let mut input = pad(rank);
        input.pad_pressure[rank_pad(rank)] = firmness;
        input
    }

    #[test]
    fn wiggling_a_source_binds_it_to_the_grabbed_control_and_again_unbinds() {
        let (mut k, mut f) = (kit(), fake());
        k.next_layer(); // Controls
        k.tick(&mut f, &pad(4)); // grab control 4
        k.tick(&mut f, &Input::default());
        // The stick's X axis already pushed control 0; sweeping it now
        // moves it to the grabbed control instead.
        k.tick(&mut f, &Input { stick: [1.0, 0.0], ..Default::default() });
        assert_eq!(k.routes.stick_x, Some(4));
        assert!(k.status().contains("Stick X now pushes C4"), "{}", k.status());
        k.tick(&mut f, &Input { stick: [1.0, 0.0], ..Default::default() });
        assert!((f.v[4] - 1.0).abs() < 1e-5, "the new target is pushed: {}", f.v[4]);
        assert!((f.v[0] - 0.5).abs() < 1e-5, "and the old one is left where it was: {}", f.v[0]);
        // Hold it there: no flapping between bound and unbound.
        for _ in 0..5 {
            k.tick(&mut f, &Input { stick: [1.0, 0.0], ..Default::default() });
        }
        assert_eq!(k.routes.stick_x, Some(4));
        // Let go, then sweep again: it unbinds.
        k.tick(&mut f, &Input::default());
        k.tick(&mut f, &Input { stick: [1.0, 0.0], ..Default::default() });
        assert_eq!(k.routes.stick_x, None);
        assert!(k.status().contains("no longer pushes C4"), "{}", k.status());
        k.tick(&mut f, &Input::default());
        assert!((f.v[4] - 0.5).abs() < 1e-5, "unbound, it returns home: {}", f.v[4]);
    }

    #[test]
    fn a_hand_wave_binds_the_hand_and_a_stepped_control_refuses() {
        let (mut k, mut f) = (kit(), fake());
        k.next_layer();
        k.tick(&mut f, &pad(3));
        k.tick(&mut f, &Input::default());
        k.tick(&mut f, &Input { hands: [0.0, 0.8], ..Default::default() });
        assert_eq!(k.routes.hand_r, Some(3), "the right hand had no route and now has one");
        // Control 6 is a choice (stepped): nothing may be bound to it.
        k.tick(&mut f, &Input::default());
        k.tick(&mut f, &pad(6));
        k.tick(&mut f, &Input::default());
        k.tick(&mut f, &Input { stick: [0.0, 1.0], ..Default::default() });
        assert_eq!(k.routes.stick_y, Some(1), "unchanged");
        assert!(k.status().contains("is a choice"), "{}", k.status());
    }

    #[test]
    fn pad_pressure_pushes_its_route_and_lets_go_exactly() {
        let (mut k, mut f) = (kit(), fake());
        k.pressure_route = Some(5);
        k.tick(&mut f, &press(0, 0.5));
        assert!((f.v[5] - 0.8).abs() < 1e-5, "0.5 firmness x 0.6 span: {}", f.v[5]);
        k.tick(&mut f, &press(0, 0.25));
        assert!((f.v[5] - 0.65).abs() < 1e-5, "it follows the pressure, not a latch: {}", f.v[5]);
        k.tick(&mut f, &Input::default());
        assert!((f.v[5] - 0.5).abs() < 1e-5, "released: {}", f.v[5]);
        // Unrouted, pressure does nothing.
        k.pressure_route = None;
        k.tick(&mut f, &press(0, 1.0));
        assert!((f.v[5] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn leaning_hard_on_the_grabbed_pad_binds_pressure_and_a_light_press_does_not() {
        let (mut k, mut f) = (kit(), fake());
        k.next_layer();
        for _ in 0..(LEAN_FRAMES * 2) {
            k.tick(&mut f, &press(2, 0.6));
        }
        assert_eq!(k.pressure_route(), None, "an ordinary press is just selecting the control");
        k.tick(&mut f, &Input::default());
        for _ in 0..(LEAN_FRAMES + 2) {
            k.tick(&mut f, &press(2, 0.95));
        }
        assert_eq!(k.pressure_route(), Some(2));
        assert!(k.status().contains("Pressure now pushes C2"), "{}", k.status());
    }

    /// L1 held with a stick position.
    fn l1(stick: [f32; 2]) -> Input {
        Input { shoulders: [true, false], stick, ..Default::default() }
    }

    #[test]
    fn l1_and_the_stick_set_the_selected_dial_from_left_0_to_right_100() {
        let (mut k, mut f) = (kit(), fake());
        k.tick(&mut f, &l1([-1.0, 0.0]));
        assert!((f.v[0] - 0.0).abs() < 1e-5, "fully left is 0%: {}", f.v[0]);
        k.tick(&mut f, &l1([1.0, 0.0]));
        assert!((f.v[0] - 1.0).abs() < 1e-5, "fully right is 100%: {}", f.v[0]);
        k.tick(&mut f, &l1([0.0, -1.0]));
        assert!((f.v[0] - 0.5).abs() < 1e-5, "straight down is the middle: {}", f.v[0]);
        k.tick(&mut f, &l1([0.5, 0.0]));
        assert!((f.v[0] - 0.75).abs() < 1e-5, "halfway right is 75%: {}", f.v[0]);
        assert!(k.status().contains("C0 0.75"), "the new value is shown: {}", k.status());
    }

    #[test]
    fn letting_go_of_the_stick_keeps_the_value_instead_of_snapping_to_the_middle() {
        let (mut k, mut f) = (kit(), fake());
        k.tick(&mut f, &l1([0.5, 0.0]));
        let set = f.v[0];
        assert!((set - 0.75).abs() < 1e-5);
        k.tick(&mut f, &l1([0.0, 0.0]));
        k.tick(&mut f, &l1([0.2, 0.1]));
        assert_eq!(f.v[0], set, "a small push inside the dead zone changes nothing, and pushes no route either");
        // L1 released while the stick is still over: it doesn't push the sound
        k.tick(&mut f, &Input { stick: [0.9, 0.0], ..Default::default() });
        assert_eq!(f.v[0], set, "no jump when L1 comes up");
        k.tick(&mut f, &Input::default());
        k.tick(&mut f, &Input { stick: [0.2, 0.0], ..Default::default() });
        assert!((f.v[0] - (set + 0.1)).abs() < 1e-5, "re-centred, the stick pushes its route again: {}", f.v[0]);
    }

    #[test]
    fn the_stick_does_not_also_push_its_routes_while_it_sets_a_dial() {
        let (mut k, mut f) = (kit(), fake());
        // SELECT three times: the pair's other dial, the next pair, its other dial = control 3
        for _ in 0..3 {
            k.tick(&mut f, &Input { knob1_press: true, ..Default::default() });
        }
        k.tick(&mut f, &l1([1.0, 0.0]));
        assert!((f.v[3] - 1.0).abs() < 1e-5, "the selected dial (3) is set: {}", f.v[3]);
        assert!((f.v[0] - 0.5).abs() < 1e-5 && (f.v[1] - 0.5).abs() < 1e-5, "stick X/Y normally push controls 0 and 1: {} {}", f.v[0], f.v[1]);
    }

    #[test]
    fn tapping_a_pad_while_holding_l1_picks_the_control_the_stick_sets() {
        let (mut k, mut f) = (kit(), fake());
        k.tick(&mut f, &Input { shoulders: [true, false], ..pad(4) });
        k.tick(&mut f, &l1([-1.0, 0.0]));
        assert!((f.v[4] - 0.0).abs() < 1e-5, "control 4 was tapped, so it is set: {}", f.v[4]);
        assert!((f.v[0] - 0.5).abs() < 1e-5, "not the Play view's dial: {}", f.v[0]);
        // a fresh hold goes back to the Play view's dial
        k.tick(&mut f, &Input::default());
        k.tick(&mut f, &l1([-1.0, 0.0]));
        assert!((f.v[0] - 0.0).abs() < 1e-5, "{}", f.v[0]);
    }

    #[test]
    fn while_l1_is_held_the_lit_pad_is_the_dial_the_stick_will_set() {
        let (mut k, mut f) = (kit(), fake());
        k.tick(&mut f, &Input { knob1_press: true, ..Default::default() }); // select control 1
        k.tick(&mut f, &Input { shoulders: [true, false], ..Default::default() });
        assert_eq!(k.pad_color(&f, rank_pad(1)), PadColor::Green, "control 1 is what the stick sets");
        assert_ne!(k.pad_color(&f, rank_pad(0)), PadColor::Green);
        k.tick(&mut f, &Input { shoulders: [true, false], ..pad(4) });
        assert_eq!(k.pad_color(&f, rank_pad(4)), PadColor::Green, "a tapped control takes over");
    }

    #[test]
    fn during_an_l1_hold_the_dpad_and_the_stick_turn_the_same_dial() {
        let (mut k, mut f) = (kit(), fake());
        k.tick(&mut f, &Input { knob1_press: true, ..Default::default() }); // dial 1 selected
        k.tick(&mut f, &Input { shoulders: [true, false], nav_x: 10, knob2: 10, ..Default::default() });
        assert!((f.v[1] - 0.6).abs() < 1e-5, "the D-pad turns the selected dial (1), not some stale pad: {}", f.v[1]);
        assert!((f.v[0] - 0.5).abs() < 1e-5, "{}", f.v[0]);
        assert_eq!(k.column(&f).dials.iter().position(|d| d.knob == 2), Some(1), "and the screen marks it");
    }

    #[test]
    fn a_choice_refuses_the_stick_and_says_so() {
        let (mut k, mut f) = (kit(), fake());
        k.tick(&mut f, &Input { shoulders: [true, false], ..pad(6) });
        k.tick(&mut f, &l1([1.0, 0.0]));
        assert_eq!(f.mode, 0, "stepped controls aren't swept by a stick");
        assert!(k.status().contains("is a choice"), "{}", k.status());
    }

    #[test]
    fn peeking_with_l1_never_rebinds_anything() {
        let (mut k, mut f) = (kit(), fake());
        k.tick(&mut f, &Input { shoulders: [true, false], ..pad(4) });
        k.tick(&mut f, &Input { shoulders: [true, false], stick: [1.0, 0.0], ..Default::default() });
        assert_eq!(k.routes.stick_x, Some(0), "a peek is a look, not an edit");
    }

    #[test]
    fn an_apps_suggested_pressure_route_yields_to_a_saved_one() {
        let mut k = kit();
        k.suggest_pressure_route(3);
        assert_eq!(k.pressure_route(), Some(3));
        k.routes_loaded = true;
        k.pressure_route = None;
        k.suggest_pressure_route(3);
        assert_eq!(k.pressure_route(), None, "the player's saved choice wins");
    }

    #[test]
    fn bindings_persist_to_disk() {
        let dir = std::env::temp_dir().join(format!("kit_routes_{}", std::process::id()));
        let path = dir.join("routes.json");
        let (mut k, mut f) = (kit(), fake());
        k.routes_path = Some(path.clone());
        k.next_layer();
        k.tick(&mut f, &pad(4));
        k.tick(&mut f, &Input::default());
        k.tick(&mut f, &Input { stick: [1.0, 0.0], ..Default::default() });
        let saved: SavedRoutes = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.stick_x, Some(4));
        assert_eq!(saved.stick_y, Some(1), "untouched routes are saved too");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn moments_persist_to_disk() {
        let dir = std::env::temp_dir().join(format!("kit_moments_{}", std::process::id()));
        let path = dir.join("moments.json");
        let mut k = kit();
        k.moments_path = Some(path.clone());
        k.moments[3] = Some(serde_json::json!([0.25]));
        k.save_moments().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v[3], serde_json::json!([0.25]));
        assert!(v[0].is_null());
        std::fs::remove_dir_all(dir).ok();
    }
}
