# The Portamax control contract

**Status: decided by Max on 2026-10-04, partly built.** Section 8 records each
decision. Built so far: the PS5 profile (section 11) and the L1 + joystick
set-a-dial gesture in play apps. Everything else is still to build (section 9).
Owner of the contract: Max; changes to it go through the Work log in
`AGENTS.md`.

## 1. Why this exists

The device has no dials. Every app was written when it did, and each one
solved "how do I change a value" its own way. The result is that the same
button press does different things in different apps, some things take
ten seconds that should take two, and some screens say one thing while the
buttons do another.

This contract says what each control means **everywhere**, so a person who
has learned one app can use the next without reading anything. An app is
allowed to *add* meaning to a control; it is not allowed to change what the
contract says it means.

### The principles (everything below follows from these)

1. **Same gesture, same result, in every app.** No exceptions without a
   written reason in section 7.
2. **Nothing needs a dial.** Every value is reachable with the pads, D-pad,
   stick, hands and shoulders.
3. **Quick to get near, slow to get exact.** Big moves take about two seconds;
   fine moves are one click.
4. **The screen always says what the controls will do right now.** The hint
   line is generated from the same table that defines the behaviour, so it
   cannot disagree with it.
5. **Nothing destructive happens on a single tap.**
6. **Home always works**, in every app, including full-screen ones.

## 2. The hardware the contract is written for

16 pressure-sensitive pads · D-pad with a centre SELECT button · joystick with
a click · L1 and R1 · two hand (depth) sensors · F1 to F4. A MIDI keyboard or
controller is optional and adds to this, never replaces it.

## 3. What is true today (the evidence)

Measured from the source on this branch (counts are from searching source
files, so treat them as approximate; the pattern, not the exact number, is
the point).

| Finding | Evidence |
|---|---|
| The same press moves values by very different amounts | Every app keeps a private multiplier (`sensitivity × 0.005` up to `× 0.2`, with 0.01, 0.02, 0.04 and 0.05 between) applied to ranges of very different size. Three apps checked, at the default sensitivity of 0.1: Synth's volume moves 0.5% of its range per press, Atlas's controls 1%, and Plaits' and Cascade's rate (range 0.5 to 30) about 0.07%, a spread of more than 10x |
| Even the best case is slow, and acceleration never engages | One press is 1% in Atlas; holding repeats at about ten a second (250 ms, then 100 ms), so a 0 to 100% sweep takes about ten seconds. The existing `accelerate()` (used in about 30 app files) grows with *several encoder ticks arriving in one frame*; a D-pad delivers at most one step per repeat, so for the D-pad it always returns 1 |
| The stick is a four-way button in most apps | Outside play apps the shell turns it into one step per repeat on its dominant axis once it passes 0.35 deflection. Pushing harder is no faster |
| The D-pad's meaning depends on where you are (PS5) | Home and menus: ◄ ► browse, ▲ ▼ edit. Play apps: ▲ ▼ browse, ◄ ► edit. The device's own D-pad is ▲ ▼ rows, ◄ ► value everywhere |
| L1 and R1 change meaning | Play apps: L1 peeks at Controls, R1 flips Play/Menu. Everything else: L1 is Home, R1 is SELECT |
| F2 is four different things | A pad layer (play apps, Sequencer included), Pad Lock (instruments without a play view), a category (Home), nothing (other apps) |
| Button hints are hand-written per screen | Atlas's menu says "R1 OPEN" while R1 there returns to the Play view; other apps say "R1 SELECT · ◄ ► ADJUST" |
| Text is cut off where it matters | "STICK MORPH / COLO…", and status messages such as "Stick X now pushes SP…" |
| A destructive action takes one tap | Atlas's Revert reloads the sound and discards edits on a single SELECT; Store → overwrites a state the same way |
| Reset is not reachable everywhere | Hold SELECT resets on the device, but on the gamepad no button is bound to it by default, and F4 (Mixer) is unbound in play apps |
| Pad pressure is unused outside the play kit | No app reads it directly; only the shared kit does |

Shape of the app set: about 83 app source files; roughly 36 already share the
play kit; roughly 24 are full-screen (Retro, the Kids apps, and similar); the
rest are list-driven. That shape matters for the migration plan (section 9):
fixing the shared kit and the shared list fixes most apps at once.

## 4. The contract

Five contexts exist. Every control is defined once, then only the *exceptions*
per context are listed.

* **Home**: the launcher.
* **List**: an app that is a menu of rows.
* **Play**: the play view of a play-kit app (pads are an instrument).
* **Menu**: a play app's full menu (R1 from Play).
* **Full-screen**: an app that owns the whole display (Retro, Kids).

### 4.1 Moving and choosing

| Control | Means, everywhere | Per context |
|---|---|---|
| **D-pad ▲ ▼** | Move through things: rows, apps, sounds | Home: apps. List/Menu: rows. Play: previous/next of the app's browse control (sound, engine, algorithm) |
| **SELECT, tap** | Go into / choose / next | Home and List: open or run the row, or expand a group. Play: next dial. Menu: run an action row |
| **SELECT, hold ½ s** | Reset the focused value. A ring fills while held, so it is never a surprise | Everywhere there is a focused value |
| **R1** | Go one level deeper, or back: the Enter/Back toggle between two levels | Home and List: open. Play: open the Menu. Menu: back to Play |
| **L1** | Look, and set | **Peek** means seeing every control laid out as pads (the Controls layer) for as long as L1 is held, without leaving what you are playing. In Play and Menu, hold L1 and move the joystick to set the selected dial (4.2). Elsewhere (to build): peek at what the focused row will do (value, range, what it affects) |
| **F1** | Home | From Home itself: Settings |
| **F4** | Mixer | Everywhere. Must never be unbound on any input device |

*Change from today:* L1 stops being Home in list apps (F1 and the PS button
already are), so L1 means one thing everywhere.

### 4.2 Changing a value (the adjust model)

One model, three speeds, in every app:

| Speed | Gesture | What happens |
|---|---|---|
| **Nudge** | D-pad ◄ ► tap | One detent. The detent is defined per parameter and defaults to 1% of its range, on its natural scale (linear, logarithmic, or dB). A choice moves one item. This replaces every app's private multiplier; the Settings "sensitivity" scales all of them together |
| **Sweep** | D-pad ◄ ► hold | After 250 ms, repeats every 100 ms. At 0.75 s each repeat moves 5 detents; at 1.75 s, 20. A full-range sweep takes about 2 seconds. Letting go or reversing returns to one detent |
| **Set** (play mode) | Hold **L1** and move the joystick | Jump straight to a value. Built |

**Set by joystick (play mode).** With a dial selected, hold **L1** and move the
joystick: pushed fully left is 0%, fully right is 100%, straight up or down is
50%. The dial follows the stick while it is pushed beyond a small dead zone; let
go and the value **stays**, it never snaps back to the middle. While L1 is held
the stick belongs to this, so it does not also push its routed controls (and
does not jump them when L1 comes up). The selected dial is the one highlighted
on the Play view; holding L1 also shows the Controls layer, and tapping a
control's pad during the hold picks that control instead. A choice (a preset, a
voice count) refuses, since sweeping it would flood the app; use the D-pad.

**Stick scrubbing (List apps only).** Where the stick is free (it is an
instrument in play apps), left/right scrubs the focused value at a speed
proportional to how far it is pushed: just past the dead zone is one detent a
repeat; fully pushed is a full sweep in about 1.5 seconds. Up/down moves rows.

The existing `accelerate()` exponent stays only for real encoders on a MIDI
controller, where several ticks genuinely arrive per frame; the D-pad and
stick use the schedule above.

**Reset** is hold-SELECT (4.1). There is no undo in this contract; if wanted
later it needs its own gesture, since L1 plus SELECT is better kept for moving
the selection while L1 is held.

### 4.3 Playing

| Source | Means, everywhere | Notes |
|---|---|---|
| **Pads** | The app's instrument, on its native layer | F2 steps layers; the badge on screen always names the layer |
| **Pad pressure** | One routed control, firmest pad wins | Per-note pressure is a later extension, never a different gesture |
| **Stick** | Two routed controls, springing back | Click keeps the pushed sound |
| **Hands** | One routed control each, closer = more | Never navigation inside a play app |
| **Routing** | Rebindable by wiggling (grab a control, move the source) | Same gesture in every app that has a Controls layer; saved per app |

### 4.4 The F row

| Button | Means |
|---|---|
| **F1** | Home |
| **F2** | The pad mode of this app: the next pad layer, grid mode or Pad Lock. One meaning per app, shown in the bar. Home: next category |
| **F3** | This app's transport (Play/Stop/Record). A dash when it has none. Home: Recent |
| **F4** | Mixer |

The bottom bar always shows all four labels as they apply *right now*.

### 4.5 Never on a single tap

Any action that discards or overwrites work (Revert, Store over an existing
state, Delete, Clear, Overwrite, Load over unsaved edits) requires **hold
SELECT** with the same filling ring as reset. A tap shows "hold to confirm".
Saving, storing into an empty slot, and anything undoable stay one tap.

## 5. What the screen must show

These are the rules that make it *readable*; they are checked, not hoped for.

1. **A hint line that cannot be wrong.** One line, generated from the control
   table above for the current context: for example `▲▼ browse · ◄► change ·
   SELECT next dial · R1 menu`. It is produced by the shell/kit from the same
   data that routes the buttons. No screen writes its own hint text.
2. **The focused value is always on screen with its unit and its range
   position.** The player never has to guess what the next press will do.
3. **Nothing the player must read to act is ever truncated.** Values, units,
   the focused row's label and status messages wrap to two lines or scroll;
   only a long unfocused label may end in an ellipsis.
4. **Density is allowed, clutter is not.** The screen may be as dense as it is
   today as long as the hierarchy is clear: what you are changing is the
   largest and brightest thing, supporting values are smaller, and nothing
   overlaps. There is no minimum point size; the test is that someone new can
   read each label and value in a screenshot without squinting or guessing.
5. **One layout skeleton.** Title and layer badge top-left; the thing you are
   changing (dials or rows) on the left; a live monitor on the right; hint line
   above the F bar; the F bar at the bottom. Apps fill the regions; they do
   not move them. Full-screen apps are exempt but keep the hint line and F bar
   available on demand (hold F1).
6. **Same visual language for state**: focused = lit centre or highlight bar;
   held or playing = filled; gliding = yellow; unavailable = dim. The same
   pad colours (Off, Blue, Yellow, Green, Red) in every app; they already exist.

## 6. Every app must be able to answer these

An app conforms when, using only the hardware above, a new player can:

1. See what every button does now (hint line).
2. Move to any value and change it by one detent, by a sweep, and by a jump.
3. Put any value back where it started (hold SELECT).
4. Get Home from anywhere (F1), and to the Mixer (F4).
5. Not lose work to a stray tap (4.5).
6. Read every label and value they need without guessing.

## 7. Exceptions (the only ones)

| App(s) | Exception | Why |
|---|---|---|
| Retro and games | D-pad, face buttons and shoulders belong to the game; R1 flips to the menu as today | The game is the instrument. F1 and F4 still work, and hold-F1 shows the hint line |
| Kids apps | Their own large-target UI; the contract's gestures still apply to anything they expose as a value | Children's UI is deliberately different. Home still works |
| Oracle, Norns and other script/AI apps | Text entry and scripts may define their own gestures inside their pane | The pane is not a value; the frame around it follows the contract |

Anything else that wants an exception writes it here first.

## 8. Decisions

Made by Max on 2026-10-04.

| # | Question | Decision |
|---|---|---|
| 1 | D-pad axes | ▲ ▼ browse, ◄ ► change a value, **everywhere**, the PS5 included |
| 2 | Adjust numbers (detent 1%, repeat 100 ms, x5 at 0.75 s, x20 at 1.75 s) | Fine for now |
| 3 | Pads as a slider | Not wanted for now. SELECT, the D-pad's centre button, is the select |
| 4 | Confirm destructive actions by holding SELECT | **Still open** |
| 5 | L1 | Peek is kept ("could be good"), and L1 + joystick sets a dial in play mode (new, 4.2) |
| 6 | Undo | Dropped (not asked for; see 4.2) |
| 7 | Minimum text size | None. A denser screen is fine if it is laid out well and readable (5.4) |
| 8 | One hint line above the F bar in every app | Yes |
| 9 | PS5 layout | Changed and built (section 11) |

Still to decide: #4; and whether the joystick should also scrub values in
list apps (4.2, "Stick scrubbing") and set a dial there with L1 held (today L1
is Home in list apps).

## 9. How it would be built, and checked

Fix the shared parts once, then walk the apps.

1. **Shared kit and shell** (one pass): the adjust model with time-based
   acceleration and a per-parameter detent (retiring the private multipliers
   and the `accelerate()` call sites in about 30 files as each app is walked); stick scrubbing in lists; hold-ring and
   confirm-by-hold; the generated hint line; text wrap/scroll; the
   L1 and gamepad changes; F4 never unbound. Roughly 36 play-kit apps pick most
   of this up with no per-app work.
2. **The shared list** (`ParamList`): the same adjust model for every
   list-driven app, plus the detent declaration. This is where the 0.005 to
   0.2 multipliers go away.
3. **App walk**: convert the remaining and bespoke apps in groups (instruments,
   effects, sequencing, library, utilities), a how-to guide for each as it
   goes (Atlas's is the template and its screenshot harness the method).
4. **A contract test suite** drives every installed app through the six
   questions in section 6 using real `Input` (as the Atlas harness does) and
   fails if: a ◄ ► press moves a value by something other than its detent; hold
   does not accelerate; hold-SELECT does not reset; a destructive row acts on
   a tap; F1 or F4 does nothing; the hint line differs from the routed
   controls; or a value, unit or focused label is truncated. New apps
   (including the testers' agent-built ones) must pass it before integration,
   and `template.rs` and `CLAUDE.md` get updated to teach it.
5. **Visual review** from the same harness for every app, before and after,
   so readability changes are seen, not assumed.

What I would not touch: the audio engine, presets, or app behaviour beyond
controls and layout.

## 10. Not decided here

Per-note pad pressure inside the synthesis engine; a gesture looper;
touchscreen gestures (the device might have a touchscreen; if it does, taps and
drags would be added to set-by-joystick and scrubbing, not replace them).

## 11. The PS5 profile (built)

This is set up for the test controller, whose left stick is broken: the left
stick is unused, the **right stick is the device's joystick**, and **L2 and R2
are unbound**. The
layout is the same in every context, so the controller never changes meaning
between Home, a list and a play app.

| DualSense | Does |
|---|---|
| D-pad ▲ ▼ | Browse (rows, apps, sounds) |
| D-pad ◄ ► | Lower / raise the value |
| Touchpad click | SELECT: tap to select, hold half a second to reset |
| PS button | Home |
| Right stick | The joystick. In a list: Y moves rows (up is down the list, as asked originally), X changes the value |
| R3 (play apps) | Joystick click: keep the pushed sound |
| L1 / R1 (play apps) | Peek and set a dial / Play-Menu flip |
| Options, Create (play apps) | F2, F3 |
| Square, Circle, Cross, Triangle | Pads 1, 5, 9, 13 (the left column), for quick tests |
| Outside play apps | L1 Home, R1 F4 (Mixer), R3 F2, L3 F3, and Options/Create plus the D-pad and shoulders still double as Retro's pad |

Retro's A/B/X/Y are no longer on the face buttons (they are the test pads now);
re-learn them in the Controller app if needed. The map your Controller app had
saved was moved aside to `saves/controller_map.json.bak-2026-10-04` so these
defaults apply; to get it back, rename it to `controller_map.json`.
