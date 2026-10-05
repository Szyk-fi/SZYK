// Game_Music_Emu (vendor/gme, LGPL-2.1, (c) Shay Green, Michael Pyne and
// others) is called directly through its own C API (gme.h) from
// src/apps/chip_player.rs. Its sources are compiled by the list in gme.sources
// and flags in gme.defines; this file only names what the library is.
extern "C" const char* portamax_gme_license() { return "LGPL-2.1 (see vendor/gme/LICENSE-LGPL-2.1)"; }
