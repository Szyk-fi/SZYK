// C ABI wrapper around Mutable Instruments' Tides (2018) poly slope
// generator (from vendor/eurorack/tides2, MIT-licensed, (c) Emilie
// Gillet). It calls tides::PolySlopeGenerator::Render the way
// tides2/tides.cc's Process() does with nothing patched into the clock
// input: frequency from the FREQUENCY knob, the TRIG input from the
// player's gate.
//
// The generator works in cycles per sample, so unlike the other
// bridges it runs at whatever rate the device does -- the Rust side
// passes frequency / sample rate.

#include <cstdint>

#include "stmlib/utils/gate_flags.h"
#include "tides2/poly_slope_generator.h"

namespace {

const size_t kMaxBlock = 64;

struct Handle {
  tides::PolySlopeGenerator generator;
  tides::PolySlopeGenerator::OutputSample out[kMaxBlock];
  stmlib::GateFlags gate[kMaxBlock];
  stmlib::GateFlags previous_gate;
  int previous_output_mode;
};

}  // namespace

extern "C" {

int tides_max_block() { return static_cast<int>(kMaxBlock); }

void* tides_create() {
  Handle* h = new Handle();
  h->generator.Init();
  h->previous_gate = stmlib::GATE_FLAG_LOW;
  h->previous_output_mode = -1;
  return h;
}

void tides_destroy(void* handle) { delete static_cast<Handle*>(handle); }

// ramp_mode: 0 AD, 1 looping, 2 AR. output_mode: 0 gates, 1 amplitude,
// 2 slope/phase, 3 frequency. range: 0 control, 1 audio.
// `frequency` is cycles per sample. `gate` holds the TRIG input for the
// whole block (rising/falling edges land on its first sample), or -1
// for unpatched. `out` receives n frames of the four outputs in volts,
// as the firmware sends them to its DACs.
void tides_render(
    void* handle,
    int ramp_mode,
    int output_mode,
    int range,
    float frequency,
    float slope,
    float shape,
    float smoothness,
    float shift,
    int gate,
    float* out,
    int n) {
  Handle* h = static_cast<Handle*>(handle);
  size_t size = n > static_cast<int>(kMaxBlock) ? kMaxBlock : static_cast<size_t>(n);
  if (output_mode != h->previous_output_mode) {
    // tides.cc resets the generator's filters on a mode change.
    h->generator.Reset();
    h->previous_output_mode = output_mode;
  }
  const stmlib::GateFlags* gate_flags = NULL;
  if (gate >= 0) {
    for (size_t i = 0; i < size; ++i) {
      h->previous_gate = stmlib::ExtractGateFlags(h->previous_gate, gate != 0);
      h->gate[i] = h->previous_gate;
    }
    gate_flags = h->gate;
  } else {
    for (size_t i = 0; i < size; ++i) h->gate[i] = stmlib::GATE_FLAG_LOW;
    gate_flags = h->gate;
  }
  h->generator.Render(
      tides::RampMode(ramp_mode),
      tides::OutputMode(output_mode),
      tides::Range(range),
      frequency,
      slope,
      shape,
      smoothness,
      shift,
      gate_flags,
      NULL,
      h->out,
      size);
  for (size_t i = 0; i < size; ++i) {
    for (int c = 0; c < 4; ++c) out[i * 4 + c] = h->out[i].channel[c];
  }
}

}  // extern "C"
