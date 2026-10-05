// C ABI wrapper around Mutable Instruments' Braids macro oscillator (from
// vendor/eurorack/braids, MIT-licensed, (c) Emilie Gillet). No DSP of its
// own: it follows braids/braids.cc's RenderBlock -- the AD envelope that
// can move timbre, colour, pitch and the VCA, the macro oscillator itself,
// and the bit and sample-rate reduction -- one 24-sample block at a time.
//
// Braids runs at a fixed 96 kHz; the Rust side resamples to the device.
// Left out: the quantizer (the pads already pick notes), the module's
// per-unit "signature" waveshaper and its pitch drift, which exist to make
// two real modules sound slightly different from each other.

#include <cstdint>
#include <cstring>

#include "braids/envelope.h"
#include "braids/macro_oscillator.h"

namespace {

const uint16_t kBitMasks[] = {0xc000, 0xe000, 0xf000, 0xf800, 0xff00, 0xfff0, 0xffff};
const uint16_t kDecimation[] = {24, 12, 6, 4, 3, 2, 1};
const size_t kBlock = 24;

struct Handle {
  braids::MacroOscillator osc;
  braids::Envelope env;
  uint16_t gain_lp;
};

}  // namespace

extern "C" {

int braids_block_size() { return static_cast<int>(kBlock); }
int braids_shape_count() { return braids::MACRO_OSC_SHAPE_LAST_ACCESSIBLE_FROM_META + 1; }

void* braids_create() {
  Handle* h = new Handle();
  std::memset(&h->gain_lp, 0, sizeof(h->gain_lp));
  h->osc.Init();
  h->osc.set_shape(braids::MACRO_OSC_SHAPE_CSAW);
  h->env.Init();
  return h;
}

void braids_destroy(void* handle) { delete static_cast<Handle*>(handle); }

// pitch is in 1/128 semitones (MIDI note << 7); timbre and colour 0..1; the
// ad_* amounts are the firmware's 0..15 settings; ad_vca is 0 or 1;
// resolution and rate index the firmware's tables (0..6). One block of 24
// samples at 96 kHz, as -1..1.
void braids_render(void* handle, int shape, int pitch, float timbre, float color, int strike, int ad_attack, int ad_decay,
                   int ad_timbre, int ad_color, int ad_fm, int ad_vca, int resolution, int rate, float* out) {
  Handle* h = static_cast<Handle*>(handle);
  auto clampi = [](int v, int lo, int hi) { return v < lo ? lo : (v > hi ? hi : v); };
  shape = clampi(shape, 0, braids::MACRO_OSC_SHAPE_LAST_ACCESSIBLE_FROM_META);

  h->env.Update(clampi(ad_attack, 0, 15) * 8, clampi(ad_decay, 0, 15) * 8);
  uint32_t ad_value = h->env.Render();

  h->osc.set_shape(static_cast<braids::MacroOscillatorShape>(shape));
  int32_t params[2] = {static_cast<int32_t>(timbre * 32767.0f), static_cast<int32_t>(color * 32767.0f)};
  const int amounts[2] = {clampi(ad_timbre, 0, 15), clampi(ad_color, 0, 15)};
  for (int i = 0; i < 2; ++i) {
    params[i] += static_cast<int32_t>((ad_value * amounts[i]) >> 5);
    params[i] = clampi(params[i], 0, 32767);
  }
  h->osc.set_parameters(static_cast<int16_t>(params[0]), static_cast<int16_t>(params[1]));

  int32_t p = pitch + static_cast<int32_t>((ad_value * clampi(ad_fm, 0, 15)) >> 7);
  h->osc.set_pitch(static_cast<int16_t>(clampi(p, 0, 16383)));

  if (strike) {
    h->osc.Strike();
    h->env.Trigger(braids::ENV_SEGMENT_ATTACK);
  }

  uint8_t sync[kBlock];
  int16_t buffer[kBlock];
  std::memset(sync, 0, sizeof(sync));
  h->osc.Render(sync, buffer, kBlock);

  const size_t decimation = kDecimation[clampi(rate, 0, 6)];
  const uint16_t mask = kBitMasks[clampi(resolution, 0, 6)];
  const int32_t gain = ad_vca ? static_cast<int32_t>(ad_value) : 65535;
  int16_t held = 0;
  for (size_t i = 0; i < kBlock; ++i) {
    if ((i % decimation) == 0) held = buffer[i] & mask;
    int16_t sample = static_cast<int16_t>(held * h->gain_lp >> 16);
    h->gain_lp += (gain - h->gain_lp) >> 4;
    out[i] = sample / 32768.0f;
  }
}

}  // extern "C"
