# Legibility and accessibility on the Portamax screen

The UI is designed for the panel the device actually has, not a desktop
window. Every number below comes from that panel.

## The screen

- **Panel:** AMS317PN01, a 3.17" AMOLED. It is 360×640 portrait,
  rotated to landscape, so the UI is 640×360 (`src/display.rs`).
- **Diagonal:** √(640² + 360²) = 734 px over 3.17" ≈ **232 PPI**.
- **One UI pixel = 0.11 mm.** The sim's 640×360 window is the physical
  pixel grid. There is no scaling between the design and the glass.
- **Viewing distance:** a handheld instrument is held at roughly
  30–40 cm, about the same as a phone.

## Type

These are the rules. `font-size` in the Slint files is the em size in
physical pixels.

| Use | Minimum | Physical em | Notes |
|---|---|---|---|
| Any text at all | **12 px** | 1.3 mm | Hint lines, axis labels, captions. Nothing smaller anywhere. |
| Menu rows, dial labels and values | 12–14 px | 1.3–1.5 mm | |
| Primary values (preset, engine, BPM) | ≥ 15 px | ≥ 1.65 mm | |
| Titles | 20–28 px | 2.2–3 mm | |

Why 12 px is the floor:
- At 12 px the cap height is about 0.9 mm, which subtends about 9–10
  arcminutes at 30 cm.
- Human-factors guidance (ISO 9241-303, ANSI/HFES 100) puts the minimum
  for occasional reading at roughly 9 arcmin and the preference at
  16 or more.
- So 12 px is a floor for short labels, not a size for anything you read
  continuously. Anything the player reads while playing is 14 px or
  larger.

Before this pass the GUI used 7, 8, 9, 10 and 11 px text in about 220
places. At 232 PPI, 8 px is a 0.9 mm em with a 0.6 mm cap height, too
small to read at arm's length. All of them are now at least 12 px.

The framebuffer UI (the device binary, `embedded_graphics`) already uses
Spleen 6×12 as its smallest font, a 12 px cell, so it meets the same
floor.

## Contrast

- **Text on its background meets WCAG 2.1 AA, 4.5:1.**
- Secondary text is drawn at no less than 72% of the app's ink colour.
  On the dark palettes that keeps it above 4.5:1. It used to go down to
  35%, which was about 2:1. The rule is applied to every `color:` that
  uses `with-alpha` in the Slint files.
- Decorative strokes, grid lines and inactive bars may be fainter,
  because they aren't text.
- Every app picks its own palette. When adding one, check its ink and
  dim colours against its background at 4.5:1.

## Layout

- **Content area:** 284 px high, 4 px padding, between the 40 px title
  and the 36 px F-button bar.
- **Clipping:** the content area clips, so nothing can overlap the
  F-button labels.
- **Panels** must fit 280 px at these type sizes. Atlas and the grid
  panel were re-laid-out for this:
  - Ledger now shows 3 tracks × 10 rows.
  - The grid rows are 15 px.
- **Menus** are windowed around the selection, so long lists scroll and
  never run off the screen.
- **Hint lines:** one line each (two at most), at 12 px.

## Controls

The device has no encoders or scroll wheels. Every action is reachable
with the D-pad, SELECT, L1/R1 and F1–F4:

| Control | What it does |
|---|---|
| D-pad up/down | Move or browse |
| D-pad left/right | Change the value / turn the focused dial |
| SELECT | Select, or move the focus to the next dial |
| Hold SELECT (0.5 s) | Reset |

- Physical buttons give tactile feedback and work with gloves. Nothing
  depends on touch, though the sim's mouse clicks stand in for it.
- **The focused dial is shown three ways:** a filled dot, an accent
  arc, and its name flashed in the status line. Colour is never the only
  cue.
- **Pad LEDs** use colour plus brightness (off / dim / lit / held), so
  the states stay distinguishable for colour-blind players.

## Not done yet

- **No large-text setting.** A Settings toggle that multiplies every
  size by 1.25 would need the panels to reflow. It's worth doing once the
  layouts settle.
- **No screen-reader or haptic feedback.** The device has no speaker
  channel or haptics reserved for UI speech.
- **Some collection screens' two-line hint footers** lose their last
  words at 12 px. They should be shortened.
