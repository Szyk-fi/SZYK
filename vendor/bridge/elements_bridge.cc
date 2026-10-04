// C ABI wrapper around Mutable Instruments' Elements modal voice (from
// vendor/eurorack/elements, MIT-licensed, (c) Emilie Gillet). No DSP of
// its own: it writes the panel's values into elements::Patch the way the
// firmware's CvScaler does (elements/cv_scaler.cc) and calls
// elements::Part::Process, as elements/elements.cc does.
//
// Elements runs at a fixed 32 kHz in blocks of elements::kMaxBlockSize
// (16). The Rust side resamples to the device rate.

#include <cstdint>
#include <cstring>

#include "elements/dsp/dsp.h"
#include "elements/dsp/part.h"

namespace {

struct Handle {
  elements::Part part;
  // Same size as the firmware's (elements/elements.cc).
  uint16_t reverb_buffer[32768];
};

}  // namespace

extern "C" {

int elements_block_size() { return static_cast<int>(elements::kMaxBlockSize); }

// `seed` plays the part of the module's serial number, which the
// firmware hashes into a few per-unit character settings (Part::Seed).
void* elements_create(uint32_t seed) {
  Handle* h = new Handle();
  std::memset(h->reverb_buffer, 0, sizeof(h->reverb_buffer));
  h->part.Init(h->reverb_buffer);
  uint32_t s[3] = { seed, seed * 2654435761u, seed ^ 0x5bd1e995u };
  h->part.Seed(s, 3);
  return h;
}

void elements_destroy(void* handle) { delete static_cast<Handle*>(handle); }

// The 14 panel controls in elements::Patch's order (0..1 each, `space`
// 0..2 like the firmware's SPACE knob), followed by the performance
// state. model: 0 modal, 1 string, 2 strings (elements::ResonatorModel);
// 3 is the "Ominous" easter-egg voice. `n` <= elements_block_size().
void elements_render(
    void* handle,
    const float* panel,
    int model,
    int gate,
    float note,
    float modulation,
    float strength,
    const float* blow_in,
    const float* strike_in,
    float* main_out,
    float* aux_out,
    int n) {
  Handle* h = static_cast<Handle*>(handle);
  elements::Patch* p = h->part.mutable_patch();
  p->exciter_envelope_shape = panel[0];
  p->exciter_bow_level = panel[1];
  p->exciter_bow_timbre = panel[2];
  p->exciter_blow_level = panel[3];
  p->exciter_blow_meta = panel[4];
  p->exciter_blow_timbre = panel[5];
  p->exciter_strike_level = panel[6];
  p->exciter_strike_meta = panel[7];
  p->exciter_strike_timbre = panel[8];
  p->resonator_geometry = panel[9];
  p->resonator_brightness = panel[10];
  p->resonator_damping = panel[11];
  p->resonator_position = panel[12];
  p->space = panel[13];

  h->part.set_easter_egg(model == 3);
  if (model >= 0 && model <= 2) {
    h->part.set_resonator_model(static_cast<elements::ResonatorModel>(model));
  }

  elements::PerformanceState state;
  state.gate = gate != 0;
  state.note = note;
  state.modulation = modulation;
  state.strength = strength;

  float silence[elements::kMaxBlockSize] = { 0 };
  size_t size = n > static_cast<int>(elements::kMaxBlockSize) ? elements::kMaxBlockSize : static_cast<size_t>(n);
  h->part.Process(state, blow_in ? blow_in : silence, strike_in ? strike_in : silence, main_out, aux_out, size);
}

}  // extern "C"
