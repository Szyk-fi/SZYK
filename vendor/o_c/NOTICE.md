# Ornaments & Crimes firmware (vendor/o_c)

`fw/` is the firmware of the O&C eurorack module from
https://github.com/mxmxmx/O_C (`software/o_c_REV`, at commit cfbffab), by
Patrick Dowling, Max Stadler (mxmxmx), Tim Churches and others, with code from
Mutable Instruments (Olivier Gillet: the Peaks, Frames, Braids and Streams
pieces it builds on) and Teensy audio-library helpers (PJRC). Most source files
carry an MIT licence header and are used under it.

**Licensing you should know about.** The project has no top-level licence file,
and about 35 of its files -- mostly core headers and `.cpp` files such as
`OC_core.h`, `OC_ui.cpp` and `OC_scales.cpp` -- carry no licence header at all.
They are by the same authors and ship together with the MIT-headed files, but
nothing in the repository states their terms, so they are included here on that
understanding and not on a licence grant. If you plan to redistribute Portamax
widely, ask the maintainers or drop `vendor/o_c`.

Two upstream files are **not** included because they are GPL-3.0:
`OC_DAC.cpp` (the DAC8565 driver) and `src/drivers/SH1106_128x64_driver.cpp`
(the display driver). The firmware's DAC and display interfaces are
implemented from their headers' declarations in `host/oc_host.cpp` instead,
written without reference to the GPL sources.

`host/` is Portamax's own code (MIT, like the rest of Portamax): the stand-in
for the Teensy 3.2 and the module around it.

## Licensing review (2026-10-05)

Checked for all three O&C-family firmwares (`vendor/o_c`, `o_c_hemi`, `o_c_phaz`):

- **No upstream licence file.** GitHub reports no licence for `mxmxmx/O_C`,
  `Chysn/O_C-HemisphereSuite` or `djphazer/O_C-Phazerville`. File headers are the only
  grant: 76 / 133 / 197 files carry the standard MIT text; 40 / 39 / 69 carry none.
  Hemisphere's own `SegmentDisplay.h` says "MIT License, see HemisphereSuite License
  info on GitHub", and Phazerville's README says code is "generally considered MIT
  licensed" except where headers say otherwise -- so MIT is clearly the intent, but
  the unlabelled files have no explicit grant.
- **The maintainers say the bundle is GPLv3 as shipped.** https://ornament-and-cri.me/licensing/
  states that `SH1106_128x64_driver.cpp` (derived from GPL3 code) makes the whole
  firmware bundle "implicitly subject to" GPLv3; Phazerville's README adds that "some
  GPLv3 bits" are included. The GPL pieces are `OC_DAC.cpp`, the SH1106 driver, and
  the Grids rhythm data (`grids_resources.h`, `grids2_resources.h`, from Mutable
  Instruments' GPL Grids). **None of them is in this repository**, and Phazerville's
  DrumMap applet (which needs the Grids data) is removed. A search of all three trees
  finds no GPL text, and `host/` was compared against the upstream GPL files: the only
  shared line is `calibration_data_ = calibration_data;`. Whether the remaining
  MIT-headed files are still "bundle-derived" in the maintainers' sense is theirs to
  say; Portamax's position is that it ships only MIT-granted code plus its own.
- **Third-party pieces and their notices (kept in the file headers):** Mutable
  Instruments code (MIT, Olivier/Emilie Gillet), `gfx_font_6x8.h` (Tinusaur, MIT),
  `dspinst.h` (PJRC, MIT), `extern/fastapprox` in Phazerville (Paul Mineiro, 3-clause
  BSD -- needs its notice in binary distributions too). Applet comments crediting
  other modules ("inspired by", "concept by") are ideas, not code.
- **What remains a risk:** the unlabelled files, and the maintainers' GPL claim. Before
  distributing builds beyond a small beta, ask the maintainers (the ornament-and-cri.me
  site lists contacts) to confirm the MIT-headed files may be used on their own, or
  drop the O&C apps from the release. Binary distributions must carry the MIT and BSD
  notices (the repository's `vendor/*/NOTICE.md` and the file headers).
