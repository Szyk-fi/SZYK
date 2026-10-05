# Phazerville Suite firmware (vendor/o_c_phaz)

`fw/` is the Phazerville Suite (the `T32` branch), a large fork of the Hemisphere
Suite with many more applets, from https://github.com/djphazer/O_C-Phazerville
(`software/src`, at commit 182644d, August 2026), by djphazer and contributors, on
top of the Hemisphere Suite (Chysn), Ornaments & Crimes (Patrick Dowling, Max
Stadler and others) and Mutable Instruments code (Olivier Gillet).

**Licensing.** No top-level licence file; 195 source files carry an MIT header and
about 74 carry none, so as with `vendor/o_c` they are here on the understanding that
they ship under their authors' MIT terms, not on a stated grant. **Left out because
they are GPL-3.0:** `OC_DAC.cpp`, `src/drivers/SH1106_128x64_driver.cpp`, and the
Grids data (`grids_resources.h`, `grids2_resources.h`, derived from Mutable
Instruments' GPL Grids), which means the **DrumMap applet is removed**. The DAC and
display are written by Portamax (`host/oc_host_fw.cpp`, MIT).
