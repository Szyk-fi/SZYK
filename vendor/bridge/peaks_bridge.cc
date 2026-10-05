// C ABI wrapper around Mutable Instruments' Peaks (from vendor/eurorack/peaks,
// MIT-licensed, (c) Emilie Gillet): the two processors of the module, each one
// of twelve functions (envelope, LFOs, bass/snare/FM drums, hi-hat, pulse
// shaper and randomizer, bouncing ball, mini sequencer, number station) with
// four parameters, driven the way peaks/peaks.cc drives them: a gate input
// turned into rising/falling flags per sample, parameters as 16-bit pots, and
// 16-bit samples out. Nothing here is DSP of our own.
//
// Peaks runs at 48 kHz; the Rust side resamples to the device.

#include <cstdint>

#include "peaks/processors.h"

namespace {

struct Handle {
  peaks::Processors processors[2];
  peaks::GateFlags gate[2];
  int function[2];
};

}  // namespace

extern "C" {

int peaks_function_count() { return static_cast<int>(peaks::PROCESSOR_FUNCTION_LAST); }

void* peaks_create() {
  Handle* h = new Handle();
  for (int i = 0; i < 2; ++i) {
    h->processors[i].Init(static_cast<uint8_t>(i));
    h->processors[i].set_control_mode(peaks::CONTROL_MODE_FULL);
    h->gate[i] = peaks::GATE_FLAG_LOW;
    h->function[i] = static_cast<int>(h->processors[i].function());
  }
  return h;
}

void peaks_destroy(void* handle) { delete static_cast<Handle*>(handle); }

// Sets processor `ch` (0 or 1) to `function` and its four 16-bit parameters.
void peaks_set(void* handle, int ch, int function, uint16_t p0, uint16_t p1, uint16_t p2, uint16_t p3) {
  Handle* h = static_cast<Handle*>(handle);
  peaks::Processors& p = h->processors[ch & 1];
  if (function < 0 || function >= static_cast<int>(peaks::PROCESSOR_FUNCTION_LAST)) return;
  if (function != h->function[ch & 1]) {
    p.set_function(static_cast<peaks::ProcessorFunction>(function));
    h->function[ch & 1] = static_cast<int>(p.function());
  }
  p.set_parameter(0, p0);
  p.set_parameter(1, p1);
  p.set_parameter(2, p2);
  p.set_parameter(3, p3);
}

// The function processor `ch` is actually running: the snare and hi-hat swap
// between themselves according to their parameters, as on the module.
int peaks_function(void* handle, int ch) {
  return static_cast<int>(static_cast<Handle*>(handle)->processors[ch & 1].function());
}

// Renders `n` samples of processor `ch` with the gate input held at `gate`.
void peaks_render(void* handle, int ch, int gate, int16_t* out, int n) {
  Handle* h = static_cast<Handle*>(handle);
  peaks::Processors& p = h->processors[ch & 1];
  for (int i = 0; i < n; ++i) {
    h->gate[ch & 1] = peaks::ExtractGateFlags(h->gate[ch & 1], gate != 0);
    int16_t s = 0;
    p.Process(&h->gate[ch & 1], &s, 1);
    out[i] = s;
  }
}

}  // extern "C"
