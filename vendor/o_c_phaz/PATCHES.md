# Changes to the vendored Phazerville Suite

See `vendor/o_c/PATCHES.md` for the approach; stand-ins shared by all the O&C-family
firmwares are in `vendor/o_c/host` (Arduino core, `elapsedMicros`, mixed-type `min`/`max`,
`avr/pgmspace.h`, `arm_math`, a `usbMIDI` stub). Built as C++17 (`cxx_std` in `build.rs`) with
`USB_MIDI` and the T32 environment's apps (Calibr8or, Scenes, Pong, Piqued).

Removed: `OC_DAC.cpp`, the SH1106 driver, `display.cpp`, `framebuffer.h`,
`page_display_driver.h` (GPL or hardware); `OC_ADC.cpp`, `usb_name.c`, `*_ADC` library
sources, `FreqMeasureCapture.h`; Grids resources and the `DrumMap` applet
(`hemisphere_config.h`, `applets/DrumMap.h`); the project's `.xcf` images.

Replaced: `OC_ADC.h` (Teensy ADC/DMA members and the SCA table dropped, a run-time
`update_raw` added), `src/drivers/display.h`, the ADC and FreqMeasure stub headers.

Edited: `OC_gpio.h` (Kinetis `pinMode`), `util/util_math.h`, `util/EEPROMStorage.h` (8 KB),
`OC_config.h`, `OC_calibration.h` (constexpr `pow`), `OC_menus.cpp` (constexpr `cosf`
table built at start-up), `OC_scales.h/.cpp` (Scala file load/save need the SD card; behind
`OC_HOST_SDCARD`), `OC_debug.cpp` (RAM screen), `OC_core.cpp` (`FreeRam`, and the
deferred-task queue gets a mutex because the ISR and loop are real threads here),
`Main.cpp` (serial screen-capture block off), `weegfx.cpp` (`print` width type),
`applets/EnvSeq.h` (an enum cast).
