// C ABI wrapper around Mutable Instruments' Plaits voice (from
// vendor/eurorack/plaits, MIT-licensed, (c) Emilie Gillet), so it can be
// called from Rust via a plain extern "C" boundary. This file contains no
// synthesis code of its own -- it just adapts plaits::Voice::Render's
// signature to something FFI-friendly and always renders in
// kMaxBlockSize-sized chunks, since Voice's internal scratch buffers are
// sized for exactly that.

#include <cstdint>
#include <cstring>
#include <vector>

#include "plaits/dsp/voice.h"

namespace {

struct Handle {
  plaits::Voice voice;
  std::vector<uint8_t> scratch;
};

}  // namespace

extern "C" {

void* plaits_voice_create() {
  Handle* h = new Handle();
  h->scratch.resize(32 * 1024);
  stmlib::BufferAllocator allocator(h->scratch.data(), h->scratch.size());
  h->voice.Init(&allocator);
  return h;
}

void plaits_voice_destroy(void* handle) {
  delete static_cast<Handle*>(handle);
}

// Renders `size` samples (mono, the "out" channel) at Plaits' fixed
// internal 48kHz -- the Rust side resamples to the device's actual rate.
// `size` may be any length; internally chunked into kMaxBlockSize pieces.
void plaits_voice_render(
    void* handle,
    int engine,
    float note,
    float harmonics,
    float timbre,
    float morph,
    float decay,
    float lpg_colour,
    float trigger,
    float level,
    float* out,
    int size) {
  Handle* h = static_cast<Handle*>(handle);

  plaits::Patch patch;
  patch.note = note;
  patch.harmonics = harmonics;
  patch.timbre = timbre;
  patch.morph = morph;
  patch.frequency_modulation_amount = 0.0f;
  patch.timbre_modulation_amount = 0.0f;
  patch.morph_modulation_amount = 0.0f;
  patch.engine = engine;
  patch.decay = decay;
  patch.lpg_colour = lpg_colour;

  plaits::Modulations modulations;
  modulations.engine = 0.0f;
  modulations.note = 0.0f;
  modulations.frequency = 0.0f;
  modulations.harmonics = 0.0f;
  modulations.timbre = 0.0f;
  modulations.morph = 0.0f;
  modulations.trigger = trigger;
  modulations.level = level;
  modulations.frequency_patched = false;
  modulations.timbre_patched = false;
  modulations.morph_patched = false;
  modulations.trigger_patched = true;
  modulations.level_patched = false;

  int remaining = size;
  float* dst = out;
  while (remaining > 0) {
    int chunk = remaining > static_cast<int>(plaits::kMaxBlockSize)
        ? static_cast<int>(plaits::kMaxBlockSize)
        : remaining;
    plaits::Voice::Frame frames[plaits::kMaxBlockSize];
    h->voice.Render(patch, modulations, frames, chunk);
    for (int i = 0; i < chunk; i++) {
      dst[i] = frames[i].out / 32768.0f;
    }
    dst += chunk;
    remaining -= chunk;
  }
}

}  // extern "C"
