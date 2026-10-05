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
