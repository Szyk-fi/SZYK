// C ABI wrapper around Mutable Instruments' Marbles random sampler (from
// vendor/eurorack/marbles, MIT-licensed, (c) Emilie Gillet). It drives
// marbles::TGenerator and marbles::XYGenerator from their internal clock,
// the way marbles/marbles.cc's Process() does with nothing patched into
// the clock inputs. The tables below (the six factory scales, the deja vu
// loop lengths and the Y divider ratios) are copied from
// marbles/settings.cc and marbles/marbles.cc, under the same license;
// those files can't be compiled here because they also drive the
// module's flash and hardware.
//
// Marbles runs at 32 kHz in blocks of marbles::kBlockSize (5).

#include <cstdint>
#include <cstring>

#include "marbles/random/random_generator.h"
#include "marbles/random/random_stream.h"
#include "marbles/random/t_generator.h"
#include "marbles/random/x_y_generator.h"
#include "stmlib/dsp/hysteresis_quantizer.h"

using namespace marbles;

namespace {

const size_t kBlock = 5;  // marbles/io_buffer.h kBlockSize
const float kRate = 32000.0f;  // marbles/marbles.cc kSampleRate

const Scale preset_scales[6] = {
  // C major
  { 1.0f, 12, { { 0.0000f, 255 }, { 0.0833f, 16 }, { 0.1667f, 96 }, { 0.2500f, 24 }, { 0.3333f, 128 }, { 0.4167f, 64 },
                { 0.5000f, 8 }, { 0.5833f, 192 }, { 0.6667f, 16 }, { 0.7500f, 96 }, { 0.8333f, 24 }, { 0.9167f, 128 } } },
  // C minor
  { 1.0f, 12, { { 0.0000f, 255 }, { 0.0833f, 16 }, { 0.1667f, 96 }, { 0.2500f, 128 }, { 0.3333f, 8 }, { 0.4167f, 64 },
                { 0.5000f, 4 }, { 0.5833f, 192 }, { 0.6667f, 96 }, { 0.7500f, 16 }, { 0.8333f, 128 }, { 0.9167f, 16 } } },
  // Pentatonic
  { 1.0f, 12, { { 0.0000f, 255 }, { 0.0833f, 4 }, { 0.1667f, 96 }, { 0.2500f, 4 }, { 0.3333f, 4 }, { 0.4167f, 140 },
                { 0.5000f, 4 }, { 0.5833f, 192 }, { 0.6667f, 4 }, { 0.7500f, 96 }, { 0.8333f, 4 }, { 0.9167f, 4 } } },
  // Pelog
  { 1.0f, 7, { { 0.0000f, 255 }, { 0.1275f, 128 }, { 0.2625f, 32 }, { 0.4600f, 8 }, { 0.5883f, 192 }, { 0.7067f, 64 },
               { 0.8817f, 16 } } },
  // Raag Bhairav That
  { 1.0f, 12, { { 0.0000f, 255 }, { 0.0752f, 128 }, { 0.1699f, 4 }, { 0.2630f, 4 }, { 0.3219f, 128 }, { 0.4150f, 64 },
                { 0.4918f, 4 }, { 0.5850f, 192 }, { 0.6601f, 64 }, { 0.7549f, 4 }, { 0.8479f, 4 }, { 0.9069f, 64 } } },
  // Raag Shri
  { 1.0f, 12, { { 0.0000f, 255 }, { 0.0752f, 4 }, { 0.1699f, 128 }, { 0.2630f, 64 }, { 0.3219f, 4 }, { 0.4150f, 128 },
                { 0.4918f, 4 }, { 0.5850f, 192 }, { 0.6601f, 4 }, { 0.7549f, 64 }, { 0.8479f, 128 }, { 0.9069f, 4 } } },
};

const Ratio y_divider_ratios[] = {
  { 1, 64 }, { 1, 48 }, { 1, 32 }, { 1, 24 }, { 1, 16 }, { 1, 12 },
  { 1, 8 }, { 1, 6 }, { 1, 4 }, { 1, 3 }, { 1, 2 }, { 1, 1 },
};

const int loop_length[] = {
  1,
  2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
  3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
  4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
  5, 5, 5, 5,
  6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
  7, 7,
  8, 8, 8, 8, 8, 8, 8, 8, 8,
  10, 10, 10,
  12, 12, 12, 12, 12, 12, 12,
  14, 14,
  16
};

struct Handle {
  RandomGenerator random_generator;
  RandomStream random_stream;
  TGenerator t_generator;
  XYGenerator xy_generator;
  stmlib::HysteresisQuantizer2 deja_vu_length_quantizer;
  float ramp_buffer[kBlock * 4];
  bool gates[kBlock * 2];
  float voltages[kBlock * 4];
  stmlib::GateFlags no_clock[kBlock];
};

}  // namespace

extern "C" {

int marbles_block_size() { return static_cast<int>(kBlock); }
float marbles_sample_rate() { return kRate; }

void* marbles_create(uint32_t seed) {
  Handle* h = new Handle();
  h->random_generator.Init(seed ? seed : 1);
  h->random_stream.Init(&h->random_generator);
  h->t_generator.Init(&h->random_stream, kRate);
  h->xy_generator.Init(&h->random_stream, kRate);
  for (int i = 0; i < 6; ++i) h->xy_generator.LoadScale(i, preset_scales[i]);
  h->deja_vu_length_quantizer.Init(sizeof(loop_length) / sizeof(int), 0.25f, false);
  std::memset(h->no_clock, 0, sizeof(h->no_clock));
  return h;
}

void marbles_destroy(void* handle) { delete static_cast<Handle*>(handle); }

// One block of kBlock samples. `knobs` holds the panel's knobs, 0..1:
//   0 T rate, 1 T bias, 2 T jitter, 3 deja vu, 4 length,
//   5 X spread, 6 X bias, 7 X steps, 8 T pulse width, 9 Y divider.
// `modes`: 0 T model (0..6), 1 T range (0..2), 2 X range (0..2),
//   3 X scale (0..5), 4 deja vu on (0 off, 1 on, 2 locked),
//   5 X control mode (0..2).
// Out: `gates` = kBlock frames of t1, t2, t3; `cv` = kBlock frames of
// x1, x2, x3, y in volts.
void marbles_render(void* handle, const float* knobs, const int* modes, int* gates_out, float* cv_out) {
  Handle* h = static_cast<Handle*>(handle);

  // The firmware's deadband around 12 o'clock on DEJA VU (marbles.cc).
  float deja_vu = knobs[3];
  if (deja_vu < 0.47f) {
    deja_vu *= 1.06382978723f;
  } else if (deja_vu > 0.53f) {
    deja_vu = 0.5f + (deja_vu - 0.53f) * 1.06382978723f;
  } else {
    deja_vu = 0.5f;
  }
  int deja_vu_mode = modes[4];
  float dv = deja_vu_mode == 2 ? 0.5f : (deja_vu_mode == 1 ? deja_vu : 0.0f);
  int length = h->deja_vu_length_quantizer.Lookup(loop_length, knobs[4]);

  Ramps ramps;
  ramps.master = &h->ramp_buffer[0];
  ramps.external = &h->ramp_buffer[kBlock];
  ramps.slave[0] = &h->ramp_buffer[kBlock * 2];
  ramps.slave[1] = &h->ramp_buffer[kBlock * 3];

  h->t_generator.set_model(TGeneratorModel(modes[0]));
  h->t_generator.set_range(TGeneratorRange(modes[1]));
  // T RATE: the firmware's CvReader maps the pot to pot * 120 - 60
  // semitones (cv_reader.cc, channel_settings_).
  h->t_generator.set_rate((knobs[0] - 0.5f) * 120.0f);
  h->t_generator.set_bias(knobs[1]);
  h->t_generator.set_jitter(knobs[2]);
  h->t_generator.set_deja_vu(dv);
  h->t_generator.set_length(length);
  h->t_generator.set_pulse_width_mean(knobs[8]);
  h->t_generator.set_pulse_width_std(0.0f);
  h->t_generator.Process(false, h->no_clock, ramps, h->gates, kBlock);

  GroupSettings x;
  x.control_mode = ControlMode(modes[5]);
  x.voltage_range = VoltageRange(modes[2] % 3);
  x.register_mode = false;
  x.register_value = 0.0f;
  x.spread = knobs[5];
  x.bias = knobs[6];
  x.steps = knobs[7];
  x.deja_vu = dv;
  x.scale_index = modes[3];
  x.length = length;
  x.ratio.p = 1;
  x.ratio.q = 1;

  // Y's settings as the firmware's defaults (settings.cc): spread and
  // bias at noon, no quantization, full range, divider from the knob.
  GroupSettings y;
  y.control_mode = CONTROL_MODE_IDENTICAL;
  y.voltage_range = VOLTAGE_RANGE_FULL;
  y.register_mode = false;
  y.register_value = 0.0f;
  y.spread = 0.5f;
  y.bias = 0.5f;
  y.steps = 0.0f;
  y.deja_vu = 0.0f;
  y.length = 1;
  int div = static_cast<int>(knobs[9] * 11.99f);
  y.ratio = y_divider_ratios[div < 0 ? 0 : (div > 11 ? 11 : div)];
  y.scale_index = modes[3];

  h->xy_generator.Process(CLOCK_SOURCE_INTERNAL_T1_T2_T3, x, y, h->no_clock, ramps, h->voltages, kBlock);

  // Output order as the firmware wires its DACs and gate jacks: the
  // generator's voltages come out as x1, x2, x3, y per frame, and
  // T2 is the master ramp's square (marbles.cc).
  for (size_t i = 0; i < kBlock; ++i) {
    gates_out[i * 3 + 0] = h->gates[i * 2 + 0];
    gates_out[i * 3 + 1] = ramps.master[i] < 0.5f;
    gates_out[i * 3 + 2] = h->gates[i * 2 + 1];
    for (int c = 0; c < 4; ++c) cv_out[i * 4 + c] = h->voltages[i * 4 + c];
  }
}

}  // extern "C"
