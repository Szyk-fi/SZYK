# Push 2 as the Portamax / ROVE test surface

Launch ROVE normally (`cargo run` or `cargo run --example slint_home_live`).
Native Push 2 support is the default: it finds a Push 2 connected before or
after launch and uses its User MIDI port. No separate Push app or launcher is needed.
The Push display shows parameter names, values and navigation across the full
960×160 display. The app's screen image is never rendered on Push.

When applying this reviewed branch to an existing checkout, run
`bash tools/apply-push2-default.command` once from this checkout. It checks the
complete patch before applying, preserves the existing branch/index/local edits,
and does not launch a separate Push process. Its optional argument is the target
checkout; the default is Max's existing Modulade/portamax-sim folder.
Quit Ableton Live while using Portamax's display. If Live is running, it can
own the display or change the hardware mode underneath this host.

The display needs libusb 1.0. On this Mac it is already installed. On another
Mac, install it with `brew install libusb`; on Linux use your distribution's
libusb package and grant the current user USB access to vendor 2982/product 1967.
Missing display access leaves MIDI working and prints a diagnostic. macOS and
Linux have a native display backend; Windows currently has MIDI support only.

## Knobs and screen buttons

| Control | Action |
| --- | --- |
| Eight encoders above the display | Edit eight parameters of the visible app |
| Page left/right, Swing encoder or lower display buttons 5/6 | Previous/next bank of eight; Shift jumps eight banks |
| Touch an encoder | Highlight its label and current value |
| Upper display buttons 1–8 | Select/highlight a parameter; open a group in menu apps |
| Device | Return from keyboard settings to parameters |
| Lower display button 2 | Arm the visible instrument, or disarm the armed instrument |
| Shift + lower display button 2, or Shift + Note | Choose the MIDI Arm target without changing the app being edited |
| Lower display buttons 1, 3, 4 | Home/Settings, transport, Mixer |
| Lower display button 7 or Layout | Cycle pad layouts |
| Lower display button 8 or Scale | Open/close keyboard settings |
| Setup | Existing Home/Settings shortcut |
| Browse | Return to the launcher |
| Mix | Open Mixer |
| Play | Toggle the visible app's own transport, when it has one |
| Arrow keys | Existing menu navigation and value editing |
| Select | Existing select/open action |
| Master encoder | Master volume |
| Tempo encoder | Shared clock tempo; Shift changes in 0.1 BPM steps |

Keyboard settings use encoder 1 for root, 2 for scale, 3 for octave and 4
for the monome grid bank. Changes release held Push notes before remapping.
The header shows the parameter range, total and page number. Paging wraps in
both directions, including the final partial bank. The focused parameter's
complete label and value appear below the eight columns. The current bank is
remembered for each app during the session, including across keyboard settings. Upper buttons
select parameters rather than acting as nonexistent encoder push switches.

Parameter names use 16-pixel text and wrap over two lines. Short values use
24-pixel text; longer strings use 20- or 16-pixel text to stay within their
column. The group labels, page header and button hints stay compact.

On the launcher, encoder 1 browses; upper display button 1 opens the selected
app. The on-computer launcher shows the current selection.

## All 64 pads

| Layout | What the pads do |
| --- | --- |
| App pads | Original lower-left 4×4 surface, with the app's actual LED overlay |
| Chromatic 64 | 64 consecutive MIDI notes, bottom-left first, ascending across each row |
| Fourths | Chromatic rows five semitones apart; overlapping notes share a release correctly |
| In Key | Scale notes only; rows overlap at three scale degrees |
| Drums 64 | 64 consecutive MIDI notes, coloured in groups of sixteen |
| Monome Grid | 8×8 window of the existing shared monome grid, with actual app LEDs |

Note jumps straight to Fourths. Session returns to App pads. Octave ± changes
the register. Octave names use C4 = MIDI 60. Roots are blue, held pads red; pads outside MIDI 0–127 go dark.
Hold Shift for maximum pad velocity. Other strikes keep their own velocity.
The touch strip supplies pitch bend (or CC1 modulation if the device was
configured that way). Pressure supplies Portamax's existing aggregate aftertouch
and per-pad firmness; independent per-voice pressure is not part of its Input API.

The full keyboard feeds MIDI note numbers without wrapping onto sixteen pads.
It works with apps that consume keyboard notes; the app's own voice count and
musical behavior still apply. A utility app does not gain a synth voice.
For sample/drum apps, notes trigger whichever sounds those apps already map.

## Play one app while editing another

In ROVE, choose any installed MIDI instrument in the **MIDI ARM** selector
below the console. One instrument is armed at a time, independently of the
app being edited. The selector remains visible even on full-screen apps and
lists instruments without opening them all. For example, arm Bebot, then open
Beads: the keyboard and pads keep playing Bebot while the controls edit Beads.
Beads' input-source setting chooses which audio it processes.

The red **DISARM** button stops keyboard/pad input. **MIDI disarmed** in the
selector does the same. **Follow screen** restores automatic routing; **ARM**
arms the visible instrument. Choosing another instrument reassigns the arm.

On Push, open an instrument and press **lower display button 2** to arm it.
Browse to any other app; all eight encoders edit that app while the pads keep
playing the armed instrument. The display names both, for example
`EDIT: Beads | MIDI/PADS: Bebot [ARM]`. Lower button 2 lights red when armed;
press it again to disarm. It is independent of the app's F2 pad-layer function.

To choose a different playing app from any screen, hold Shift and press lower
button 2 (or Note). Encoder 1 scrolls; upper display buttons 1–8 select the app
in that column. Select confirms the highlighted app, Page buttons browse eight
targets at a time, and Device cancels. Choosing a target keeps the edit screen
and its parameter bank. The first two choices are **Follow screen** and
**MIDI disarmed**; every available instrument follows.

The framebuffer simulator has the same routing: F5 opens the MIDI Arm chooser,
Up/Down or encoder 1 browses, Enter selects and Escape cancels. F6 toggles the
arm; F2 remains the app's pad-layout action where present.

Keyboard notes, app pads, velocities, pressure, bend and modulation follow the
playing target. Joystick and menu controls stay with the edit screen. Held
notes survive navigation through other apps and the launcher; note-off reaches
the original instrument. Explicitly changing the target releases the old owner
and waits for held controls to be released before the new owner can play them.
The target's App-pad overlay drives both ROVE and Push pad LEDs.

In Grid mode, select the intended client in Portamax's Grid app first. An 8×16
script needs two horizontal banks: select bank 1 or 2 with the fourth encoder
on the keyboard-settings page. Releases go to their original coordinates before
changing a bank. The Push and an actual monome can share the existing hub;
avoid pressing the same logical key from both simultaneously.

## How edits reach apps

Grouped apps expose every menu leaf, including closed groups, channel controls,
modulator slots and envelope settings. Names, values and edits share the same
native selection identities. Rev2 includes both layers and all sequence pages;
Chop includes Edit, Sample and FX controls. Skins provides voice selection,
lane settings and a held step's locks. Context controls such as voice or pad
selection choose which sound those settings edit. Read-only TimbreMap status
does not occupy an encoder.

The lazy app wrapper forwards controller catalogs and edits. Direct edits may
select the corresponding native menu row, but never tick an app a second time.
Other apps use their current menu rows, with at most one queued encoder edit
per normal app tick. Upper buttons select rows; Select performs the app's native
select/open action. Page changes discard old pending turns. Catalog changes
discard pending turns whose labels have changed, so they cannot edit a different
parameter accidentally.

MIDI input and output are restricted to one Push port. The generic MIDI/LED
paths exclude it while the native driver is running. USB transfers and port
reconnection run on a worker outside audio and UI callbacks. `PORTAMAX_PUSH2=0`
restores the previous generic controller mapping. A normal exit clears LEDs
and returns Push to Live MIDI mode and channel pressure.

Mappings and keyboard settings currently last for the running session. There
is no arbitrary MIDI-learn button remapping or new 64-step sequencer UI here;
Grid mode operates the existing grid apps.

## Validation

Focused routing, protocol and real-app tests are in `src/push2.rs`. The optional
`connected_push_accepts_native_display_frames` test opens the real hardware
and checks whether complete USB frames were accepted. It cannot verify the physical
LCD appearance or substitute for playing the pads and listening.

Hardware reference: [Ableton Push 2 MIDI and Display Interface Manual](https://github.com/Ableton/push-interface/blob/main/doc/AbletonPush2MIDIDisplayInterface.asc).

Physical operation is **unverified**: the attached Push is visible in IOUSB,
but this process could not access its MIDI ports or claim the USB display.
Launch normally from Terminal for the hardware check.

Validation on 2026-10-15: the permitted regression run passed 1,290 tests
(20 ignored); the two existing serialosc UDP tests and the previously stalled
Norns bundled-script test were excluded. Tests cover every registered MIDI
instrument being armable without opening it, independent controls, held notes,
reassignment, disarm, external MIDI note-off and hidden-instrument lifecycle.
A native Push integration test plays real Bebot DSP while changing Beads
parameters, then verifies note release. The full Slint launcher builds; its
headless selector test invokes the real callbacks, plays a cold hidden Bebot
and verifies Disarm and Follow screen. ROVE and native Push previews were
rendered and visually inspected.
Applying to Max's existing checkout was blocked by this session's filesystem
sandbox; its 119 existing modified/untracked files were confirmed unchanged.
The one-time installer performs the same patch check and apply outside that
sandbox. A temporary-checkout test confirmed installation, repeat-install
detection and preservation of all 119 user files and the staged index. A clean
installation was also compared against every changed and added source file.

Rendered native display payload previews: [parameters](push2-previews/push2-parameters.png)
and [keyboard settings](push2-previews/push2-keyboard.png), plus
[longer parameter names](push2-previews/push2-long-labels.png),
[MIDI Arm](push2-previews/push2-midi-arm.png) and the
[instrument picker](push2-previews/push2-midi-arm-picker.png).
The [ROVE selector](push2-previews/rove-bebot-keys-beads-controls.png) uses the
same routing as Push.
