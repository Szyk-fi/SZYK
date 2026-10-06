# Changes to the vendored O&C firmware

The firmware is compiled as published except for the changes below, each of
which only adapts it to run on a computer. The hardware under it is replaced by
`host/` (see `host/oc_host.cpp`).

Replaced (same public interface, no hardware):
- `fw/OC_ADC.h`: the ADC with its DMA scan and the Teensy ADC library. The
  interface, smoothing and pitch scaling are the original's; the host writes
  each input's voltage. `OC_ADC.cpp` is not included; `host/oc_host.cpp` has its
  functions.
- `fw/src/drivers/display.h`: the SH1106 display with its DMA frame ring. The
  same `weegfx::Graphics` draws into a 128x64 page-format frame the host reads.
  The driver, `display.cpp`, `framebuffer.h` and `page_display_driver.h` are not
  included.
- `fw/src/drivers/ADC/OC_util_ADC.h`, `fw/src/drivers/FreqMeasure/OC_FreqMeasure.h`:
  empty and stub headers for the Teensy libraries (the frequency counter in the
  References app never measures anything on the host).
- `OC_DAC.cpp`: not included (GPL-3.0); rewritten in `host/oc_host.cpp`.

Edited:
- `fw/OC_gpio.h`: dropped the local `pinMode` that writes Kinetis port registers.
- `fw/OC_calibration.h`, `fw/OC_apps.ino`, `fw/util/util_misc.h`: `FOURCC = FourCC<...>`, with
  `FourCC` an alias of the template -- clang won't initialise a member from a
  template of the same name.
- `fw/OC_calibration.ino`: `_ADC_OFFSET` as a literal (clang's `pow()` isn't
  constexpr).
- `fw/OC_config.h`, `fw/util/EEPROMStorage.h`: the EEPROM is 8 KB and the
  global-settings region 2 KB, because the settings structs are larger with
  64-bit pointers than the Teensy's 850 bytes allow.
- `fw/src/drivers/weegfx.cpp`: `print(uint32_t, size_t)` -> `unsigned` to match
  its declaration where `size_t` is 64 bits.
- `fw/util/util_math.h`: `USAT16` and the 32x32 multiplies as plain C instead of
  Cortex-M4 assembly.

Added:
- `sketch.cpp`: the Arduino sketch's `.ino` files as one translation unit, with
  `prototypes.h` standing in for the prototypes the Arduino build generates
  (made by `vendor/o_c/tools/oc_prototypes.py`; two lines naming types defined in the
  sketch were removed by hand).
- `host/Arduino.h`, `host/EEPROM.h`, `host/arm_math.h`: the parts of the Arduino
  and Teensy cores the firmware calls.

Several modules and firmwares: `build.rs` builds each firmware variant (`O_C_VARIANTS`:
stock here, `vendor/o_c_hemi`, `vendor/o_c_phaz`) as one shared library, with the host core
(`host/oc_host_core.cpp`) compiled into it. The firmwares keep their state in globals, so
the runtime (`src/apps/oc_firmware.rs`) loads a private temporary *copy* of the library per
module -- each copy has its own globals, which is the isolation, with no namespaces and
no changes to the firmware. `host/oc_host_core.cpp` holds the one instance's pins, EEPROM,
CV, DAC and frame; each variant's `host/oc_host_fw.cpp` defines the members of that
firmware's own classes (ADC, DAC, display) and its entry points. A module is stopped by
`ocfw_stop`, which makes the firmware's blocking calls throw out of its thread, then the
copy is unloaded and deleted. The libraries are built without RTTI, as on the Teensy.
