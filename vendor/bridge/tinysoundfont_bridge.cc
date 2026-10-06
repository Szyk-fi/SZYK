// TinySoundFont (vendor/tinysoundfont, MIT, (c) Bernhard Schelling) is a
// single header; this translation unit is the one place its implementation
// is compiled. The Rust side calls tsf_* directly through its C API.
#define TSF_IMPLEMENTATION
#include "../tinysoundfont/tsf.h"
