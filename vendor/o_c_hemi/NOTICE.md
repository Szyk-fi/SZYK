# Hemisphere Suite firmware (vendor/o_c_hemi)

`fw/` is the Hemisphere Suite, a fork of Ornaments & Crimes with two-applet
"hemispheres", from https://github.com/Chysn/O_C-HemisphereSuite
(`software/o_c_REV`, at commit 893deeb, June 2022), by Chysn (Jason Justian) and
contributors, on top of the O&C firmware by Patrick Dowling, Max Stadler and others
and code from Mutable Instruments (Olivier Gillet).

**Licensing.** The same situation as `vendor/o_c`: there is no top-level licence file;
133 source files carry an MIT header and about 41 carry none. They are included on
the understanding that they ship together under the same authors' MIT terms, not on a
stated grant. Upstream's `OC_DAC.cpp` and `SH1106_128x64_driver.cpp` (GPL-3.0) and
the Teensy-only hardware files are not included; the DAC and display interfaces are
implemented by `host/oc_host_fw.cpp` (Portamax's own, MIT). No GPL code is linked.
