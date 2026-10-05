// C ABI wrapper around Mutable Instruments' Stages segment generator (from
// vendor/eurorack/stages, MIT-licensed, (c) Emilie Gillet). No DSP of its
// own: a "group" is configured with segment types and loops and given each
// segment's two parameters exactly as stages/stages.cc does for the module's
// sliders and pots, then rendered. One segment gives the module's single
// functions (LFOs, oscillators, decay envelope, pulse and gate generators,
// sample and hold, portamento, delay); several give multi-segment envelopes
// and, in the right shape, the step sequencer.
//
// Stages runs at 31.25 kHz in blocks of 8; the Rust side resamples.

#include <cstdint>
#include <cstring>

#include "stages/segment_generator.h"
#include "stmlib/utils/gate_flags.h"

namespace {

const int kBlock = 8;

struct Handle {
  stages::SegmentGenerator generator;
  stmlib::GateFlags previous_gate;
  stmlib::HysteresisQuantizer2 step_quantizer;
};

}  // namespace

extern "C" {

int stages_block_size() { return kBlock; }
float stages_sample_rate() { return stages::kSampleRate; }

void* stages_create() {
  Handle* h = new Handle();
  h->generator.Init(&h->step_quantizer);
  h->previous_gate = stmlib::GATE_FLAG_LOW;
  return h;
}

void stages_destroy(void* handle) { delete static_cast<Handle*>(handle); }

// types: 0 ramp, 1 step, 2 hold, 3 alt. loops: 0/1. n is 1..8.
void stages_configure(void* handle, int has_trigger, const int* types, const int* loops, int n) {
  Handle* h = static_cast<Handle*>(handle);
  if (n < 1) n = 1;
  if (n > 8) n = 8;
  stages::segment::Configuration config[8];
  for (int i = 0; i < n; ++i) {
    config[i].type = static_cast<stages::segment::Type>(types[i] & 3);
    config[i].loop = loops[i] != 0;
  }
  h->generator.Configure(has_trigger != 0, config, n);
}

void stages_set_segment(void* handle, int index, float primary, float secondary) {
  static_cast<Handle*>(handle)->generator.set_segment_parameters(index, primary, secondary);
}

// One block of up to 8 samples: `gate` is the gate input for the whole
// block; value, phase and segment receive one entry per sample. Returns
// whether the group is idle (its first segment active).
int stages_process(void* handle, int gate, float* value, float* phase, int* segment, int n) {
  Handle* h = static_cast<Handle*>(handle);
  if (n > kBlock) n = kBlock;
  stmlib::GateFlags flags[kBlock];
  for (int i = 0; i < n; ++i) {
    h->previous_gate = stmlib::ExtractGateFlags(h->previous_gate, gate != 0);
    flags[i] = h->previous_gate;
  }
  stages::SegmentGenerator::Output out[kBlock];
  bool idle = h->generator.Process(flags, out, n);
  for (int i = 0; i < n; ++i) {
    value[i] = out[i].value;
    phase[i] = out[i].phase;
    segment[i] = out[i].segment;
  }
  return idle ? 1 : 0;
}

}  // extern "C"
