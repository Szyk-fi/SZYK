# DX7 presets

Drop Yamaha DX7 patch files (`.syx`) here and Cascade will pick them
up on startup, the same way `samples/` works for the Sequencer app.

Organize them into subfolders, one per bank/collection -- each
subfolder becomes its own named **bank** in Cascade's menu (its name
is just the folder name). Any `.syx` files left loose directly in
`dx7_presets/` (not inside a subfolder) are grouped into a synthetic
**(root)** bank. For example:

```
dx7_presets/
  Factory ROM 1-2/
    bank1.syx
    bank2.syx
  My Patches/
    favorites.syx
  loose_voice.syx        <- goes into the "(root)" bank
```

Supported formats:
- **32-voice bulk bank dumps** (the format almost all patch libraries
  online are distributed as) -- every voice in the file is imported.
- **Single voice dumps** (155-byte unpacked format) -- one voice per file.

Cascade's own synthesis engine is a deliberately simplified 6-operator
FM model (8 algorithms instead of the real DX7's 32, one shared
envelope instead of six independent ones -- see cascade.rs's module
doc comment for the full list of simplifications), so an imported
patch is a **best-effort approximation**, not a byte-exact recreation
of how it sounded on real hardware:

- Algorithm is mapped to the closest of Cascade's 8 by number
  (`dx7_algorithm % 8`), not by matching the actual operator routing.
- Feedback, operator Ratio, and operator Level map directly.
- The shared envelope is derived from the patch's Operator 1 (the
  primary carrier)'s own envelope -- the other five operators'
  envelope shapes are discarded.

In the launcher, open Cascade and expand the **Presets** group at the
top of its menu: turn the **Bank** leaf's knob to pick a folder, then
the **Preset** leaf's knob to browse patches within it, and press to
load.
