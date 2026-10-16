# 800 × 480 landscape display

The proposed panel is 5:3. The shared Slint device shell now lays out an
800 × 480 screen, with 40px title, 24px hint and 48px function bar: the
ordinary app body is 800 × 368. Full-screen apps receive 800 × 432 above a persistent 48px touchscreen action bar. Boot logos alone occupy the complete panel.
The 1460 × 656 enclosure preview has a wider and taller screen opening,
vertically centered pad deck, 50% larger camera lens and relocated shoulder/sensor controls. Function actions live in the touchscreen; there is no physical F-button row. Smaller
windows show a centered 5:3 panel without the surrounding enclosure; larger
windows center the enclosure. The native simulator window resizes with
aspect-preserving scaling.

Shared parameter rails show five 44px rows, with selection and ± editing
through the existing app navigation contract. Windowing preserves access to
long menus. Launcher entries are 44px; categories scroll. Dedicated rails
forward the same callbacks. Rev2 has larger actions and section tabs and a
repositioned keyboard. Each app retains its own palette and visual identity.
The special illustrated style remains confined to the existing ten Kids apps.

## Raster content and visualizers

`display::PANEL_WIDTH` / `PANEL_HEIGHT` are 800 / 480. `display::WIDTH` /
`HEIGHT` remain the 640 × 360 **logical app drawing canvas**. Changing those
constants blindly would break existing artwork, pixel-buffer indexing and
pointer contracts throughout the app library.

The native simulator presents that canvas at 800 × 450, uniformly scaled by
1.25, with 15px black bars above and below. Slint raster panels similarly use
`image-fit: contain`, with source dimensions carried from `ScreenExtra` and
the inverse transform used for touch coordinates. Retro retains its source
video dimensions and contain-fit. Slint vector panels lay out using the new
available space; raster artwork is not stretched, cropped or relabeled as
native 800 × 480 artwork. The website mockup uses the same 800 × 480 opening
and proportional 640 × 360 drawing adapter.

This is an intentional compatibility path for all existing pixel-drawn apps,
not a claim that each app's artwork has been redrawn at 800 × 480.

## Touch and hardware limits

The desktop Slint integration currently forwards one pointer gesture per
TouchArea. This change does not implement the FT5426G I²C driver or contact-ID
routing, and does not establish ten-finger performance support. Physical
multitouch, touch calibration, display timings and RGB/backlight wiring must
be validated on target hardware. Existing pads, MIDI and desktop keyboard
remain available. The simulator's RGB565 raster format also remains unchanged;
this work does not claim to configure an RGB888 MCU framebuffer.

## Verification

`cargo check --example slint_home_live`

`cargo test --bin portamax-sim`

`cargo run --example slint_home_live -- --render-display OUTPUT_DIRECTORY`

The display renderer enumerates all 116 installed manifests and captures actual app
play/menu states at 800×480, 1000×600, 1460×656 and 1600×900. It also captures
the launcher and startup logos and writes coverage.csv. Use
`PORTAMAX_RENDER_APP` with comma-separated app names/IDs for a smaller run.
Rendering does not prove every state fits; inspect captures, particularly
expanded menus, long labels, media browsers and loaded games. No synthesized
placeholder screen is substituted for app output.

Console refinement: enclosure 1420×560 (60 wider, 40 shorter), with 87px pads
(previously 78px), a centered pad deck, and recessed SZYK speaker perforations.
The display remains 800×480; these are preview coordinates, not enclosure millimetres.

Touchscreen gestures: tap an app to open it; drag the app rows vertically to
browse without launching. The launcher scrollbar also supports click/drag
seeking. Shared play knobs and parameter rows increase on upward drags and
decrease on downward drags; Rev2 retains its direct knob drags. The browser
preview follows the same gesture conventions. SZYK grille now uses 94 larger
6.5px holes; left slots removed, camera up 10px and hand sensors down 10px.

Live touch repair: launcher gestures live on a persistent list surface rather
than repeated rows, surviving model replacement during a press. Open
Application is tappable. Unchanged parameter-name models are retained to
preserve drags. Browser input queues pointer down/up edges so fast taps are
not lost between frames. SZYK grille uses rounded slots rather than holes.

Shared app control coverage: play-column pads emit down/up/cancel gates;
pad-layer chip triggers the existing F2 action; Menu triggers the R1 edge;
on-screen stick and hand controls update the same expression values as the
console. These bindings exist in both ordinary and full-screen play-column
placements. New-app pad widgets now emit release/cancel too. Browser play
kit exposes the equivalent controls, menu row editing and a Play return.
Regression captures include Atlas, Bloom and Plaits at all four display sizes.
