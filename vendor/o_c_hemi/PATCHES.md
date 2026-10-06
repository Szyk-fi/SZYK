# Changes to the vendored Hemisphere Suite

Same approach and replacements as `vendor/o_c/PATCHES.md` (shared stand-ins are in
`vendor/o_c/host`). Common edits are made by `tools/oc_port_patches.py`; the sketch's
`sketch.cpp` and `prototypes.h` by `tools/oc_sketch.py vendor/o_c_hemi o_c_REV.ino
APP_HEMISPHERE.ino OC_scales.h OC_strings.h braids_quantizer.h OC_patterns.h`.

- Replaced: `fw/OC_ADC.h` (adds the `Scan()` the host feeds), `fw/src/drivers/display.h`,
  the ADC/FreqMeasure stub headers; not included: `OC_DAC.cpp`, the display driver,
  `OC_ADC.cpp` (DMA scan), USB/bootloader files.
- Edited (the common set): `OC_calibration.h/.ino`, `OC_apps.ino`, `util/util_misc.h`,
  `util/util_math.h`, `util/EEPROMStorage.h`, `OC_config.h`, `OC_gpio.h`, `weegfx.cpp`.
- `fw/HemisphereApplet.h`, `hemisphere_config.h`, `HSLorenzGeneratorManager.h`,
  `HSRingBufferManager.h`: `#pragma once`, because the applets' sketch files are
  joined into one unit where the Arduino build includes headers once per file.
- Sketch order: the applets (`HEM_*`) before `APP_HEMISPHERE.ino`, whose table names
  their functions (the Arduino build's prototypes covered that).
- Host: `usbMIDI` is a stub that receives nothing and drops what is sent (no MIDI
  transport into the firmware yet).
