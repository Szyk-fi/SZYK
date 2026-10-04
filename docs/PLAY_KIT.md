# The play kit: giving an app a play view

Every playable Portamax app opens on a **play view** rather than its menu.
The structure is shared (`src/play_kit.rs`) so every app puts the same
things in the same places:

| Control | On the play view |
|---|---|
| **R1** | Toggle the full menu (nothing from the menu is lost) |
| **F2** | Cycle pad layers: the app's own layer(s) first, then **Controls**, **Moments**, and for effects **Throws** |
| **L1 (hold)** | Peek at the Controls layer, and with the joystick set the selected dial directly (left 0%, right 100%; the value stays when you let go) |
| **Knob 1 / Knob 2** | Turn the current pair of hero controls (badged 1 and 2) |
| **Knob 1 press** | Next hero pair |
| **Knob 2 press** | Reset the pair (or the grabbed control on Controls) |
| **D-pad up/down** | Step the app's "browse" control (engine, preset, octave, algorithm...) |
| **D-pad left/right** | Nudge knob 2's control |
| **Stick / hands / mod wheel / aftertouch** | Push routed controls away from their knob setting; letting go returns exactly to it. Stick click keeps the pushed sound |
| **Pads, Controls layer** | 16 controls, one per pad (bottom-left = most important); tap to grab for knob 2 |
| **Pads, Moments layer** | Tap = recall a whole sound, hold 0.6 s = store. Saved to `saves/<app>/moments.json` |
| **Pads, Throws layer** | Hold = push a control to a set value; release = spring back |

## No dials: how modulation works on the device

The device has no knobs, so nothing here needs one. The "Knob 1/Knob 2" rows
above describe the *simulator's* encoders and a plugged-in MIDI controller;
on the device the same jobs are done like this:

| Job | On the device |
|---|---|
| Turn one control | D-pad left/right turns the focused dial; SELECT moves the focus |
| Set one control in one move | Hold L1 and move the joystick: left is 0%, right is 100%, up or down 50%. The stick doesn't push its routes while L1 is held, and a choice (preset, voices) refuses |
| Play a control with your hands | The stick (X/Y), the two depth sensors and **pad pressure** each push one control |
| Choose *which* control a source pushes | **Bind by wiggling** (below) |
| Change a whole sound at once | **Moments** (tap to recall), or an app's own state pads |

**Pad pressure.** The pads are pressure sensitive (`Input::pad_pressure`,
0..1; the simulator reports 0.6 for a held pad that has no pressure data, a
Push 2 supplies note-on velocity and polyphonic aftertouch). The firmest pad
being *played* pushes the app's pressure route by up to 0.6, and lets go
exactly like the stick does. Pads on a layer that picks things rather than
plays (`PlayHost::kit_pads_play`) don't count. An app suggests a default with
`kit.suggest_pressure_route(control)`; Atlas sends it to ENERGY.

**Bind by wiggling.** On the Controls layer, tap a control's pad to grab it,
then *move the source you want on it*: sweep the stick, wave a hand over a
sensor, or lean hard into the control's own pad (about a third of a second at
85% pressure) for pressure. That source now pushes that control, moving off
whatever it pushed before. Doing it again unbinds it. Stepped controls (a
mode, a preset) refuse, since nothing can push a choice. Holding L1 to peek at
Controls never rebinds. Bindings are saved in `saves/<app>/routes.json`; the
app's own routes are only the starting point.

**Atlas's STATES layer.** F2 in Atlas goes PLAY, STATES, CONTROLS, MOMENTS. On
STATES the bottom row of pads is A B C D: tap one and MORPH glides there in
about a second. Nothing sounds, and the pads light to show which state the
sound is at (yellow while gliding, green once there).

The app's own pad behaviour is always the first layer, so its pads work
exactly as before until F2 is pressed. In the menu (R1) the knobs and
D-pad behave exactly as they always did, and the pads stay on whatever
layer was showing.

## Converting an app

1. **Pick the controls** (up to 16), most important first. Indexes 0-7
   are the hero pairs on the knobs; all 16 are the Controls layer, in pad
   rank order (rank 0 = bottom-left pad). Prefer continuous controls for
   0-7 (they can be pushed by expression).
2. **Add the kit:**
   ```rust
   use crate::app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes, Throw};
   // field
   kit: PlayKit,
   // constructor
   kit: PlayKit::new(kit_config(), !cfg!(test)),
   ```
3. **`kit_config()`**: `app_id` (the folder under `saves/`), `layers`
   (`Layer::Native(0, "PLAY")` first, using a short upper-case name for
   what the pads do), `hero` pairs, `browse`, `routes` (stick X/Y,
   hand L/R), `throws` (effects), `midi_to_pads` (true unless the app
   reads `Input::midi_keys` itself).
4. **`impl PlayHost`**: map each control index to the app's existing
   `Selection` (or equivalent) for label/value/edit/reset, and to a
   `Knob` (`F`/`U`/`I`/`UR`/`B`) for position and setting. Override
   `kit_pad_label` / `kit_pad_color` / `kit_line` to show what the pads
   and the app are doing, and `kit_midi_pad` if the pads are pitched.
5. **`tick`**: first thing,
   ```rust
   let mut play = std::mem::take(&mut self.kit);
   let step = play.tick(self, input);
   self.kit = play;
   let input = &step.input;
   ```
   then the existing body unchanged. On the play view the knobs, knob
   presses and D-pad are already consumed (zeroed), so the list code does
   nothing; on kit layers the pads are cleared. `step.native` says which
   of the app's own layers is showing.
6. **App trait**: `play_surface() -> true`; `play_column()` returns
   `(!self.kit.menu).then(|| self.kit.column(self))`;
   `grid_mode_label`/`toggle_grid_mode` go to the kit (unless the app
   already used F2 for its own pad modes -- then make those modes native
   layers); `grid_led_overlay` returns `self.kit.led_overlay(self)`
   (keep the app's own overlay inside `kit_pad_color` for native layers).
7. **`draw`**: where the menu list is drawn, draw the column instead
   when `!self.kit.menu` (`kit::draw::column`, in the app's palette).
8. **Tests**: the app opens on its play view; knob 1 turns hero control
   0; the native layer still plays/does what the pads did; R1 opens the
   menu. Existing tests that drive the menu with the knobs need
   `app.kit.menu = true` first.

The Slint screen needs no per-app change: when `play_column()` returns
`Some`, the shared `PlayColumn` replaces the parameter list next to the
app's own panel.
