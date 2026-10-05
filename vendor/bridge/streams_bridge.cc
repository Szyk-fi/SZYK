// C ABI wrapper around Mutable Instruments' Streams (from
// vendor/eurorack/streams, MIT-licensed, (c) Emilie Gillet): the module's two
// processors, each an envelope, vactrol, follower, compressor, filter
// controller or Lorenz generator, driven as streams/streams.cc drives them: an
// audio sample and an excite sample in, a gain and a filter frequency out (the
// 16-bit values the firmware sends its DAC and PWM). The module's VCA and
// low-pass filter are analogue circuits; this is the digital part that decides
// what they do.
//
// Streams samples at 31.09 kHz; the Rust side resamples.

#include <cstdint>

#include "streams/processor.h"

namespace {

struct Handle {
  streams::Processor processors[2];
  int function[2];
  int alternate[2];
};

}  // namespace

extern "C" {

int streams_function_count() { return static_cast<int>(streams::PROCESSOR_FUNCTION_LAST); }

void* streams_create() {
  Handle* h = new Handle();
  for (int i = 0; i < 2; ++i) {
    h->processors[i].Init(static_cast<uint8_t>(i));
    h->processors[i].Configure();
    h->function[i] = static_cast<int>(h->processors[i].function());
    h->alternate[i] = 0;
  }
  return h;
}

void streams_destroy(void* handle) { delete static_cast<Handle*>(handle); }

// Sets processor `ch` to `function` with its two 16-bit parameters.
void streams_set(void* handle, int ch, int function, int alternate, uint16_t p0, uint16_t p1) {
  Handle* h = static_cast<Handle*>(handle);
  streams::Processor& p = h->processors[ch & 1];
  if (function < 0 || function >= static_cast<int>(streams::PROCESSOR_FUNCTION_LAST)) return;
  if (function != h->function[ch & 1]) {
    p.set_function(static_cast<streams::ProcessorFunction>(function));
    h->function[ch & 1] = function;
  }
  if (alternate != h->alternate[ch & 1]) {
    p.set_alternate(alternate != 0);
    h->alternate[ch & 1] = alternate;
  }
  p.set_parameter(0, p0);
  p.set_parameter(1, p1);
  p.Configure();
}

// Renders `n` samples of processor `ch`. `audio` and `excite` are 16-bit
// samples; `gain` and `frequency` receive the 16-bit control values.
void streams_process(void* handle, int ch, const int16_t* audio, const int16_t* excite, uint16_t* gain, uint16_t* frequency, int n) {
  Handle* h = static_cast<Handle*>(handle);
  streams::Processor& p = h->processors[ch & 1];
  for (int i = 0; i < n; ++i) {
    uint16_t g = 0;
    uint16_t f = 65535;
    p.Process(audio[i], excite[i], &g, &f);
    gain[i] = g;
    frequency[i] = f;
  }
}

}  // extern "C"
