// C ABI wrapper around Mutable Instruments' Rings resonator (from
// vendor/eurorack/rings, MIT-licensed, (c) Emilie Gillet), so Rust can
// drive it through a plain extern "C" boundary. No DSP of its own: it
// fills rings::Patch and rings::PerformanceState the way the firmware's
// CvScaler does (rings/cv_scaler.cc) and calls rings::Part::Process, or
// rings::StringSynthPart::Process for the "Disastrous Peace" easter egg,
// exactly as rings/rings.cc does.
//
// Rings runs at a fixed 48 kHz in blocks of rings::kMaxBlockSize (24).
// The Rust side resamples when the device runs at another rate.

#include <cstdint>
#include <cstring>

#include "rings/dsp/dsp.h"
#include "rings/dsp/part.h"
#include "rings/dsp/patch.h"
#include "rings/dsp/performance_state.h"
#include "rings/dsp/string_synth_part.h"

namespace {

struct Handle {
  rings::Part part;
  rings::StringSynthPart string_synth;
  // Same size as the firmware's (rings/rings.cc).
  uint16_t reverb_buffer[32768];
  int polyphony;
  int model;
  int fx;
};

}  // namespace

extern "C" {

int rings_block_size() { return static_cast<int>(rings::kMaxBlockSize); }

void* rings_create() {
  Handle* h = new Handle();
  std::memset(h->reverb_buffer, 0, sizeof(h->reverb_buffer));
  h->part.Init(h->reverb_buffer);
  h->string_synth.Init(h->reverb_buffer);
  h->polyphony = 1;
  h->model = 0;
  h->fx = 0;
  h->part.set_polyphony(1);
  h->part.set_model(rings::RESONATOR_MODEL_MODAL);
  h->string_synth.set_polyphony(1);
  h->string_synth.set_fx(rings::FX_FORMANT);
  return h;
}

void rings_destroy(void* handle) { delete static_cast<Handle*>(handle); }

// model: 0..5 are rings::ResonatorModel (modal, sympathetic strings,
// inharmonic string, FM voice, quantized sympathetic strings, string and
// reverb); 6 is the easter egg string synth, with `fx` picking its
// rings::FxType. `in` is the exciter input (silence for the internal
// exciter). `n` must be <= rings_block_size().
void rings_render(
    void* handle,
    int model,
    int fx,
    int polyphony,
    float structure,
    float brightness,
    float damping,
    float position,
    float note,
    float fm,
    int strum,
    int internal_exciter,
    const float* in,
    float* out,
    float* aux,
    int n) {
  Handle* h = static_cast<Handle*>(handle);
  if (polyphony != h->polyphony) {
    h->polyphony = polyphony;
    h->part.set_polyphony(polyphony);
    h->string_synth.set_polyphony(polyphony);
  }
  if (model >= 0 && model < rings::RESONATOR_MODEL_LAST) {
    h->part.set_model(static_cast<rings::ResonatorModel>(model));
  }
  if (fx != h->fx && fx >= 0 && fx < rings::FX_LAST) {
    h->fx = fx;
    h->string_synth.set_fx(static_cast<rings::FxType>(fx));
  }

  rings::Patch patch;
  patch.structure = structure;
  patch.brightness = brightness;
  patch.damping = damping;
  patch.position = position;

  rings::PerformanceState performance;
  performance.strum = strum != 0;
  performance.internal_exciter = internal_exciter != 0;
  performance.internal_strum = false;
  performance.internal_note = false;
  performance.tonic = 0.0f;
  performance.note = note;
  performance.fm = fm;
  // The firmware derives the chord from STRUCTURE with hysteresis
  // (rings/cv_scaler.cc); kNumChords there is 11.
  int chord = static_cast<int>(structure * 10.0f + 0.5f);
  performance.chord = chord < 0 ? 0 : (chord > 10 ? 10 : chord);

  float input[rings::kMaxBlockSize];
  size_t size = n > static_cast<int>(rings::kMaxBlockSize) ? rings::kMaxBlockSize : static_cast<size_t>(n);
  for (size_t i = 0; i < size; ++i) input[i] = in ? in[i] : 0.0f;

  if (model == 6) {
    h->string_synth.Process(performance, patch, input, out, aux, size);
  } else {
    h->part.Process(performance, patch, input, out, aux, size);
  }
}

}  // extern "C"
