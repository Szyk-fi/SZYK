// Hosts the real Mutable Instruments Beads (vendor/eurorack/beads, MIT,
// published by Emilie Gillet on 12 Oct 2026) the way the module's own
// firmware hosts it: beads/beads.cc's Process() and MonitorStateChange(),
// re-done here around this sim's audio callback instead of the module's
// codec interrupt. Everything that shapes the sound is Emilie's code,
// compiled unmodified: the Processor (granular player, delay controller,
// four recording media, wow and flutter, reverb, limiters), the Synth that
// takes over when nothing is patched, GainControl (the input AGC and the
// tape-mode soft clipping) and the pot laws in cv_reader.h (the detented
// attenurandomizers, the PITCH knob's snapping intervals).
//
// What is ours, and why:
//
// - Sample-rate conversion. The module runs its codec at the recording
//   medium's own rate: 48 kHz for digital, 32 kHz for vintage digital,
//   24 kHz for both tapes (ENABLE_HARDWARE_24K, processor.h). The codec's
//   converters do the anti-aliasing there. Here the host runs at its own
//   rate, so the codec's job is done by a windowed-sinc resampler in each
//   direction (Kaiser, 12 zero crossings, cut-off at 46% of the lower
//   rate). At 48 kHz in digital mode it is bypassed, sample for sample.
//
// - Three hardware headers. gain_control.h needs Codec::Frame from
//   drivers/codec.h, and cv_reader.h (for PotChannel) needs the ADC and
//   settings headers. Those pull in STM32 register definitions, so their
//   include guards are claimed here first and the handful of declarations
//   the DSP actually uses are given below, copied from the real headers.
//   gain_control.cc is then compiled as part of this file.
//
// - The read-out for the screen. The grains, the delay heads and the
//   recording buffer are private to the engine, so this file alone
//   includes the engine with `private` and `protected` defined as
//   `public`, and only ever reads. This changes access checks, not layout
//   (the Itanium C++ ABI lays members out in declaration order regardless
//   of access), so the objects are the same ones processor.cc built.
//
// Nothing else is modelled: the factory test, calibration, the buffer
// backup to flash and the LEDs aren't here; the app draws its own panel.

#include <algorithm>
#include <cassert>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <new>
#include <type_traits>
#include <utility>

#include "stmlib/stmlib.h"

// --- Stand-ins for the three hardware headers (see above). ---
#define BEADS_DRIVERS_CODEC_H_
#define BEADS_DRIVERS_CV_ADC_H_
#define BEADS_DRIVERS_POTS_ADC_H_
#define BEADS_SETTINGS_H_

namespace beads {

// drivers/codec.h
const size_t kMaxCodecBlockSize = 24;
class Codec {
 public:
  struct Frame {
    int32_t l;
    int32_t r;
  };
  enum Event { EVENT_NONE, EVENT_SEED, EVENT_FREEZE };
};

// drivers/cv_adc.h
enum CvAdcChannel {
  CV_ADC_CHANNEL_ASSIGN,
  CV_ADC_CHANNEL_DENSITY,
  CV_ADC_CHANNEL_PITCH,
  CV_ADC_CHANNEL_TIME,
  CV_ADC_CHANNEL_SIZE,
  CV_ADC_CHANNEL_SHAPE,
  CV_ADC_CHANNEL_LAST
};
class CvAdc {
 public:
  void Init() { }
  void Convert() { }
  uint16_t value(CvAdcChannel) const { return 0; }
  float float_value(CvAdcChannel) const { return 0.0f; }
  const uint16_t* values() { return NULL; }
};

// drivers/pots_adc.h
enum PotsAdcChannel {
  POTS_ADC_CHANNEL_FEEDBACK,
  POTS_ADC_CHANNEL_DRY_WET,
  POTS_ADC_CHANNEL_REVERB,
  POTS_ADC_CHANNEL_TIME_AMOUNT,
  POTS_ADC_CHANNEL_SIZE_AMOUNT,
  POTS_ADC_CHANNEL_SHAPE_AMOUNT,
  POTS_ADC_CHANNEL_PITCH_AMOUNT,
  POTS_ADC_CHANNEL_FIRST_DIRECT_CHANNEL,
  POTS_ADC_CHANNEL_TIME = POTS_ADC_CHANNEL_FIRST_DIRECT_CHANNEL,
  POTS_ADC_CHANNEL_DENSITY,
  POTS_ADC_CHANNEL_PITCH,
  POTS_ADC_CHANNEL_SIZE,
  POTS_ADC_CHANNEL_SHAPE,
  POTS_ADC_CHANNEL_LAST
};
class PotsAdc {
 public:
  void Init() { }
  void StartMuxedPotsConversion() { }
  void Convert() { }
  uint16_t value(PotsAdcChannel) const { return 0; }
  float float_value(PotsAdcChannel) const { return 0.0f; }
};

// settings.h (only what cv_reader.h names)
struct AdcCalibrationData {
  float scale;
  float offset;
};
struct PersistentData {
  AdcCalibrationData adc_calibration[CV_ADC_CHANNEL_LAST];
};
class Settings {
 public:
  const PersistentData& persistent_data() const { return persistent_data_; }
 private:
  PersistentData persistent_data_;
};

}  // namespace beads

#define private public
#define protected public
#include "beads/dsp/processor.h"
#include "beads/dsp/synth.h"
#include "beads/gain_control.h"
#include "beads/cv_reader.h"
#undef private
#undef protected

#include "beads/gain_control.cc"

namespace beads {
// Declared in cv_reader.h, defined nowhere in the published tree.
PotChannel::PotChannel() { }
PotChannel::~PotChannel() { }
}  // namespace beads

namespace {

using namespace beads;
using stmlib::GateFlags;

const int kBlockRate = 4000;                   // beads.cc
const int kTransitionBlocks = kBlockRate / 4;  // beads.cc: codec_transition_frames
const float kSynthModeDelay = 10.0f;           // beads.cc: MonitorStateChange
const int kOverviewSize = 256;
const int kMaxGrains = 32;
const int kScopeSize = 256;

// ---------------------------------------------------------------------------
// Streaming windowed-sinc resampler, stereo. Stands in for the codec's own
// converters when the module's rate isn't the host's (see the top comment).

class Resampler {
 public:
  // Room for a 4:1 ratio (a 96 kHz host feeding the 24 kHz tape media).
  enum { kHistory = 256, kTableRes = 128, kTableSize = 8192, kZeroCrossings = 12 };

  void Init(float in_rate, float out_rate) {
    step_ = double(in_rate) / double(out_rate);
    bypass_ = std::fabs(step_ - 1.0) < 1e-9;
    const double fc = 0.46 * std::min(1.0, 1.0 / step_);  // cycles per input sample
    half_ = kZeroCrossings / (2.0 * fc);
    const double beta = 7.0;
    const double i0_beta = BesselI0(beta);
    int n = int(half_ * kTableRes) + 2;
    if (n > kTableSize) n = kTableSize;
    table_size_ = n;
    for (int i = 0; i < n; ++i) {
      const double x = double(i) / kTableRes;
      double s = x == 0.0 ? 1.0 : std::sin(M_PI * 2.0 * fc * x) / (M_PI * 2.0 * fc * x);
      const double r = x / half_;
      const double w = r >= 1.0 ? 0.0 : BesselI0(beta * std::sqrt(1.0 - r * r)) / i0_beta;
      table_[i] = float(2.0 * fc * s * w);
    }
    count_ = 0;
    t_ = 0.0;
    std::fill(&l_[0], &l_[kHistory], 0.0f);
    std::fill(&r_[0], &r_[kHistory], 0.0f);
  }

  // Latency in output samples.
  float latency() const { return bypass_ ? 0.0f : float(half_ / step_); }

  template<typename Sink>
  inline void Push(float l, float r, Sink& sink) {
    if (bypass_) {
      sink.Push(l, r);
      return;
    }
    l_[count_ & (kHistory - 1)] = l;
    r_[count_ & (kHistory - 1)] = r;
    ++count_;
    const double newest = double(count_ - 1);
    while (t_ + half_ <= newest) {
      const int64_t lo = int64_t(std::ceil(t_ - half_));
      const int64_t hi = int64_t(std::floor(t_ + half_));
      float sl = 0.0f;
      float sr = 0.0f;
      for (int64_t k = std::max<int64_t>(lo, std::max<int64_t>(0, count_ - kHistory)); k <= hi; ++k) {
        const float w = Kernel(float(std::fabs(t_ - double(k))));
        sl += w * l_[k & (kHistory - 1)];
        sr += w * r_[k & (kHistory - 1)];
      }
      sink.Push(sl, sr);
      t_ += step_;
    }
  }

 private:
  static double BesselI0(double x) {
    double sum = 1.0, term = 1.0;
    for (int k = 1; k < 32; ++k) {
      term *= (x / (2.0 * k)) * (x / (2.0 * k));
      sum += term;
    }
    return sum;
  }

  inline float Kernel(float x) const {
    const float p = x * kTableRes;
    const int i = int(p);
    if (i + 1 >= table_size_) return 0.0f;
    const float f = p - float(i);
    return table_[i] + (table_[i + 1] - table_[i]) * f;
  }

  double step_;
  double half_;
  double t_;
  bool bypass_;
  int64_t count_;
  int table_size_;
  float table_[kTableSize];
  float l_[kHistory];
  float r_[kHistory];
};

template<int N>
struct Fifo {
  FloatFrame buf[N];
  int read;
  int size;
  void Clear() { read = size = 0; }
  inline void Push(float l, float r) {
    if (size >= N) return;  // can't happen at sane block sizes; drop rather than corrupt
    FloatFrame& f = buf[(read + size) % N];
    f.l = l;
    f.r = r;
    ++size;
  }
  inline FloatFrame Pop() {
    FloatFrame f = buf[read];
    read = (read + 1) % N;
    --size;
    return f;
  }
};

}  // namespace

extern "C" {

// Mirrored by `Controls` in src/apps/beads.rs; keep the two in step.
struct BeadsControls {
  float pots[POTS_ADC_CHANNEL_LAST];  // raw 0..1, in PotsAdcChannel order
  float time_offset;    // added to TIME/SIZE/SHAPE after the pot law, then clamped
  float size_offset;
  float shape_offset;
  float pitch_offset;   // semitones, added to PITCH
  float density_cv;     // DENSITY CV, -1..1 (the module's CV range)
  float mix_offset[3];  // feedback, dry/wet, reverb: the ASSIGN CV's place
  int32_t media;        // Processor::Media: 0 dirty tape .. 3 digital
  int32_t gain;         // settings.gain: 0 = automatic, 1..255 = manual
  int32_t has_left;     // a cable in IN L
  int32_t has_right;    // a cable in IN R (without one, R is normalled to L: mono)
  int32_t seed_button;  // SeedButtonState
  int32_t seed_patched; // a cable in SEED
  int32_t seed_gate;    // its level this block
  int32_t freeze_button;
  int32_t freeze_gate;
};

struct BeadsGrainView {
  float age;    // 0 = at the record head, 1 = the far end of the buffer
  float env;    // the grain's envelope right now, 0..1
  float pan;    // 0 = left, 1 = right
  float speed;  // playback speed (1 = original pitch; synth mode: Hz)
};

struct BeadsView {
  int32_t mode;           // 0 granular, 1 delay, 2 synth, 3 codec restarting
  int32_t media;
  int32_t mono;
  int32_t frozen;
  float codec_rate;
  float buffer_seconds;
  float recorded;         // how much of the buffer has been recorded since the restart, 0..1
  float overwrite_reserve;
  float vu;               // the module's VU LED, 0..1
  int32_t agc_adjusting;
  float synth_countdown;  // seconds until the synth takes over, -1 if not counting
  int32_t seed_led;
  int32_t num_grains;
  BeadsGrainView grains[kMaxGrains];
  float overview[kOverviewSize];  // buffer peak by age, 0 = newest
  float scope[kScopeSize];        // recent output (every 4th sample), oldest first
};

}  // extern "C"

namespace {

struct Host {
  uint8_t block_1[512 * 1024];
  uint8_t block_2[256 * 1024];
  uint16_t reverb_buffer[32768];

  Processor processor;
  Synth synth;
  GainControl gain_control;
  PotChannel pots[POTS_ADC_CHANNEL_LAST];

  bool started;
  bool synth_mode;
  int transition_blocks;
  float no_input_time;
  float host_rate;
  float codec_rate;
  int block_size;

  Resampler down;
  Resampler up;
  Fifo<4096> codec_in;
  Fifo<8192> host_out;

  bool seed_delay[256];
  int seed_delay_write;
  GateFlags previous_seed;
  bool seed_led;

  float overview_abs[kOverviewSize];
  int scan_pos;
  int scan_bucket;
  float scan_acc;
  int scan_length;
  float recorded_seconds;  // since the codec (re)started, while not frozen
  int last_seed_button;
  bool last_seed_patched;
  float scope[kScopeSize];   // the last output, decimated, for synth mode
  int scope_write;
  int scope_phase;
};

void StartCodec(Host* h, bool synth_mode, bool reset_gain_control) {
  // beads.cc: ConfigureAndStartCodec.
  const int sr = synth_mode ? 48000 : int(h->processor.sample_rate());
  h->synth_mode = synth_mode;
  h->codec_rate = float(sr);
  h->block_size = sr / kBlockRate;
  const Processor::Media media = h->processor.media();
  h->gain_control.Init(
      float(sr),
      media == Processor::MEDIA_DIRTY_TAPE
          ? GainControl::SOFT_CLIPPING_MODE_ON
          : (media == Processor::MEDIA_CLEAN_TAPE
                ? GainControl::SOFT_CLIPPING_MODE_DYNAMIC
                : GainControl::SOFT_CLIPPING_MODE_OFF),
      reset_gain_control);
  h->transition_blocks = kTransitionBlocks;
  std::fill(&h->seed_delay[0], &h->seed_delay[256], false);
  h->seed_delay_write = 0;
  h->previous_seed = stmlib::GATE_FLAG_LOW;

  // The host side of a codec restart: new converters, empty queues.
  h->down.Init(h->host_rate, h->codec_rate);
  h->up.Init(h->codec_rate, h->host_rate);
  h->codec_in.Clear();
  h->host_out.Clear();
  const int prime = int(std::ceil(
      h->block_size * h->host_rate / h->codec_rate +
      h->down.latency() * h->host_rate / h->codec_rate +
      h->up.latency())) + 8;
  for (int i = 0; i < prime; ++i) h->host_out.Push(0.0f, 0.0f);

  std::fill(&h->overview_abs[0], &h->overview_abs[kOverviewSize], 0.0f);
  h->scan_pos = 0;
  h->scan_bucket = 0;
  h->scan_acc = 0.0f;
  h->scan_length = 0;
  h->recorded_seconds = 0.0f;
  h->started = true;
}

void MonitorStateChange(Host* h, const BeadsControls& c, float elapsed) {
  // beads.cc: MonitorStateChange. "No input" there is the normalization
  // probe finding nothing in either IN jack; here, no source picked.
  const bool no_input = !c.has_left && !c.has_right;
  if (no_input) {
    if (!h->synth_mode) {
      h->no_input_time += elapsed;
      if (h->no_input_time >= kSynthModeDelay) {
        StartCodec(h, true, true);
      }
    }
  } else {
    h->no_input_time = 0.0f;
    int m = c.media;
    CONSTRAIN(m, 0, 3);
    const Processor::Media media = Processor::Media(m);
    // normalization_detector.h: mono when either input is normalled.
    const bool mono = !c.has_left || !c.has_right;
    const bool media_changed = media != h->processor.media();
    const bool mono_changed = mono != h->processor.mono();
    if (!h->started || h->synth_mode || mono_changed || media_changed) {
      const bool was_synth = h->synth_mode;
      h->processor.Configure(media, mono);
      StartCodec(h, false, mono_changed || was_synth || !h->started);
    }
  }
}

void RunBlock(Host* h, const BeadsControls& c, const FloatFrame* in, FloatFrame* out, size_t size) {
  // beads.cc: Process, minus the LEDs and the normalization probe.
  Codec::Frame frames[kMaxCodecBlockSize];
  FloatFrame float_in[kMaxCodecBlockSize];
  bool freeze[kMaxCodecBlockSize];
  GateFlags seed[kMaxCodecBlockSize];

  for (int i = 0; i < POTS_ADC_CHANNEL_LAST; ++i) {
    float v = c.pots[i];
    CONSTRAIN(v, 0.0f, 1.0f);
    h->pots[i].Process(uint16_t(v * 65535.0f));
  }

  const bool freeze_now = c.freeze_button || c.freeze_gate;
  std::fill(&freeze[0], &freeze[size], freeze_now);
  if (!freeze_now && !h->transition_blocks) {
    h->recorded_seconds += float(size) / h->codec_rate;
  }

  GateFlags* seed_input = NULL;
  bool seed_block = false;
  if (c.seed_patched) {
    for (size_t i = 0; i < size; ++i) {
      // Delay the seed input by 2.5 ms, as the module does.
      h->seed_delay[h->seed_delay_write] = c.seed_gate != 0;
      const int delayed = (h->seed_delay_write - int(size) * 10 + 256 * 4) % 256;
      h->seed_delay_write = (h->seed_delay_write + 1) % 256;
      const bool has_seed = h->seed_delay[delayed];
      h->previous_seed = seed[i] = stmlib::ExtractGateFlags(h->previous_seed, has_seed);
      seed_block |= has_seed;
    }
    seed_input = seed;
  } else {
    h->previous_seed = stmlib::GATE_FLAG_LOW;
  }
  h->seed_led = seed_block && !h->transition_blocks;

  for (size_t i = 0; i < size; ++i) {
    float l = in[i].l;
    float r = in[i].r;
    CONSTRAIN(l, -1.0f, 0.999999f);
    CONSTRAIN(r, -1.0f, 0.999999f);
    frames[i].l = int32_t(l * 2147483648.0f);
    frames[i].r = int32_t(r * 2147483648.0f);
  }
  int gain = c.gain;
  CONSTRAIN(gain, 0, 255);
  h->gain_control.Process(
      !h->transition_blocks,
      uint8_t(gain),
      !c.has_left,
      !c.has_right,
      frames,
      float_in,
      size);
  if (h->transition_blocks) {
    --h->transition_blocks;
    for (size_t i = 0; i < size; ++i) out[i].l = out[i].r = 0.0f;
    return;
  }

  Parameters p;
  p.time.base = h->pots[POTS_ADC_CHANNEL_TIME].value() + c.time_offset;
  p.size.base = h->pots[POTS_ADC_CHANNEL_SIZE].value() + c.size_offset;
  p.shape.base = h->pots[POTS_ADC_CHANNEL_SHAPE].value() + c.shape_offset;
  CONSTRAIN(p.time.base, 0.0f, 1.0f);
  CONSTRAIN(p.size.base, 0.0f, 1.0f);
  CONSTRAIN(p.shape.base, 0.0f, 1.0f);
  p.pitch.base = h->pots[POTS_ADC_CHANNEL_PITCH].value() + c.pitch_offset;
  p.time.attenuverter = h->pots[POTS_ADC_CHANNEL_TIME_AMOUNT].value();
  p.size.attenuverter = h->pots[POTS_ADC_CHANNEL_SIZE_AMOUNT].value();
  p.shape.attenuverter = h->pots[POTS_ADC_CHANNEL_SHAPE_AMOUNT].value();
  p.pitch.attenuverter = h->pots[POTS_ADC_CHANNEL_PITCH_AMOUNT].value();
  // The sim's modulation adds onto the knobs (see the app), so the
  // attenurandomizers always act as on a module with no CV patched:
  // their internal-randomness half.
  p.time.has_cv = p.size.has_cv = p.shape.has_cv = p.pitch.has_cv = false;
  p.time.cv = p.size.cv = p.shape.cv = p.pitch.cv = 0.0f;
  p.monophonic_mode = false;

  ModulatedParameter d;
  d.base = h->pots[POTS_ADC_CHANNEL_DENSITY].value();
  d.cv = c.density_cv;
  CONSTRAIN(d.cv, -1.0f, 1.0f);

  float mix[3];
  for (int i = 0; i < 3; ++i) {
    mix[i] = h->pots[POTS_ADC_CHANNEL_FEEDBACK + i].value() * 1.005f + c.mix_offset[i];
    CONSTRAIN(mix[i], 0.0f, 1.0f);
  }
  Processor::MixParameters m;
  m.feedback = mix[0];
  m.dry_wet = mix[1];
  m.reverb_amount = mix[2];
  m.output_gate = false;

  int s = c.seed_button;
  CONSTRAIN(s, 0, 2);
  const SeedButtonState seed_button_state = SeedButtonState(s);

  if (h->synth_mode) {
    h->synth.Render(seed_button_state, seed_input, freeze, Synth::Resolution(h->processor.media()), d, p, m, out, size);
  } else {
    h->processor.Process(seed_button_state, seed_input, freeze, d, p, m, float_in, out, size);
  }
  for (size_t i = 0; i < size; ++i) {
    CONSTRAIN(out[i].l, -1.0f, 0.999999f);
    CONSTRAIN(out[i].r, -1.0f, 0.999999f);
  }
}

struct CodecSink {
  Host* h;
  inline void Push(float l, float r) { h->codec_in.Push(l, r); }
};

struct HostSink {
  Host* h;
  inline void Push(float l, float r) { h->host_out.Push(l, r); }
};

// Calls f with the granular processor the current medium uses.
template<typename F>
void WithActiveProcessor(Processor& p, F f) {
  switch (p.media_) {
    case Processor::MEDIA_DIRTY_TAPE: f(p.processor_dirty_tape_); break;
    case Processor::MEDIA_CLEAN_TAPE: f(p.processor_clean_tape_); break;
    case Processor::MEDIA_VINTAGE_DIGITAL: f(p.processor_vintage_digital_); break;
    default: f(p.processor_digital_); break;
  }
}

template<typename F>
void WithActiveDelayLine(Processor& p, F f) {
  WithActiveProcessor(p, [&](auto& gp) {
    if (p.mono_) {
      f(gp.mono_delay_line_);
    } else {
      f(gp.stereo_delay_line_);
    }
  });
}

// A slice of the recording buffer's peak overview, read on the audio
// thread so it never races the writes. A full sweep takes about 24 calls.
template<typename DL>
void Scan(Host* h, const DL& dl) {
  const int length = DL::MAX_DELAY;
  if (length != h->scan_length) {
    h->scan_length = length;
    h->scan_pos = 0;
    h->scan_bucket = 0;
    h->scan_acc = 0.0f;
  }
  // The two buckets just behind the record head change every call, so they
  // are re-read in full first; the sweep then carries on round the rest.
  const int head_bucket = int(int64_t((dl.write_ptr_ + 1) % length) * kOverviewSize / length);
  for (int b = head_bucket; b < head_bucket + 2; ++b) {
    const int bucket = b % kOverviewSize;
    const int from = int(int64_t(bucket) * length / kOverviewSize);
    const int to = int(int64_t(bucket + 1) * length / kOverviewSize);
    float peak = 0.0f;
    for (int pos = from; pos < to; ++pos) {
      const FloatFrame f = dl.Load(pos);
      peak = std::max(peak, std::max(std::fabs(f.l), std::fabs(f.r)));
    }
    h->overview_abs[bucket] = peak;
  }
  const int per_call = std::max(4096, length / 24);
  for (int i = 0; i < per_call; ++i) {
    const int bucket = int(int64_t(h->scan_pos) * kOverviewSize / length);
    if (bucket != h->scan_bucket) {
      h->overview_abs[h->scan_bucket] = h->scan_acc;
      h->scan_bucket = bucket;
      h->scan_acc = 0.0f;
    }
    const FloatFrame f = dl.Load(h->scan_pos);
    h->scan_acc = std::max(h->scan_acc, std::max(std::fabs(f.l), std::fabs(f.r)));
    if (++h->scan_pos >= length) h->scan_pos = 0;
  }
}

float GrainEnvelopeValue(const GrainEnvelope& e) {
  if (e.stage_ >= GrainEnvelope::STAGE_DONE) return 0.0f;
  if (e.stage_ == GrainEnvelope::STAGE_SUSTAIN) return 1.0f;
  const bool attack = e.stage_ == GrainEnvelope::STAGE_ATTACK;
  const float t = attack ? e.phase_ : 1.0f - e.phase_;
  const float base = attack ? t : (t * t) * (t * t);
  return base + (stmlib::SmoothStep(t) - base) * e.smoothness_;
}

float StealingValue(const VoiceStealingEnvelope& e) {
  float v = e.value_;
  CONSTRAIN(v, 0.0f, 1.0f);
  return v;
}

// PlayHead::Render's envelope, without advancing it.
float HeadEnvelope(const PlayHead& head, float shape, bool slave) {
  const float phase = head.phase_;
  if (slave) {
    const float slave_envelopes[6] = { 0.0f, 0.0f, 0.0f, PlayHead::Triangle(phase, 0.5f), 0.0f, 0.0f };
    return stmlib::Interpolate(slave_envelopes, shape, 5.0f);
  }
  const float pw[6] = { 0.0f, 0.001f, 0.01f, 0.5f, 0.75f, 0.999f };
  if (shape < 0.2f) {
    const float square = std::min(100.0f * PlayHead::Triangle(phase, 0.5f), 1.0f);
    const float triangle = PlayHead::Triangle(phase, pw[1]);
    return square + (triangle - square) * shape * 5.0f;
  }
  return PlayHead::Triangle(phase, stmlib::Interpolate(pw, shape, 5.0f));
}


// delay_controller.cc's kRatio table (it is file-local there).
const float kDelayRatios[22] = {
  1.0f / 16.0f, 1.0f / 16.0f, 1.0f / 8.0f, 1.0f / 8.0f, 1.0f / 4.0f,
  1.0f / 4.0f, 1.0f / 2.0f, 1.0f / 2.0f, 1.0f, 1.0f,
  1.0f, 1.0f, 1.0f / 2.0f, 1.0f / 3.0f, 1.0f / 4.0f,
  1.0f / 6.0f, 1.0f / 8.0f, 1.0f / 10.0f, 1.0f / 12.0f, 1.0f / 16.0f,
  1.0f / 16.0f, 1.0f / 16.0f,
};

// Unfrozen delay mode: the controller works the taps out sample by sample
// and hands them straight to the multitap delay, so they are never stored.
// This redoes DelayController::RenderInternal's unfrozen branch from the
// controller's own state (clock intervals and cross-fade, the smoothed
// TIME multiplier, the filtered DENSITY), for the screen only. Left out:
// DENSITY's CV as FM, and the slow wobble on the odd taps' level.
int DelayTaps(Host* h, const DelayController& dc, BeadsGrainView* out, int max_out) {
  const bool synchronized = !(h->last_seed_button == SEED_BUTTON_STATE_LOCKED && !h->last_seed_patched);
  const float density = dc.density_;
  float tap_ratio[2], tap_gain[2];
  if (synchronized) {
    float ratio = density * 20.0f;
    int ratio_integral = int(ratio);
    float ratio_fractional = ratio - float(ratio_integral);
    if (ratio_integral > 20) ratio_integral = 20;
    ratio_fractional -= 0.5f;
    ratio_fractional *= density < 0.5f ? 4.0f : 1.5f;
    ratio_fractional += 0.5f;
    CONSTRAIN(ratio_fractional, 0.0f, 1.0f);
    const int flip = ratio_integral & 1;
    tap_ratio[flip] = kDelayRatios[ratio_integral];
    tap_ratio[1 - flip] = kDelayRatios[ratio_integral + 1];
    tap_gain[flip] = 1.0f - ratio_fractional;
    tap_gain[1 - flip] = ratio_fractional;
  } else {
    tap_ratio[0] = 1.0f;
    tap_ratio[1] = 0.77f;
    float fade_to_odd = (density - 0.5f) * 25.0f;
    CONSTRAIN(fade_to_odd, 0.0f, 1.0f);
    tap_gain[0] = 1.0f - fade_to_odd * 0.2f;
    tap_gain[1] = fade_to_odd * 0.8f;
  }
  float multiplier = dc.multiplier_lp_ * 15.0f + 1.0f;
  CONSTRAIN(multiplier, 1.0f, 16.0f);
  const int multiplier_integral = int(multiplier);
  float multiplier_fractional = 0.5f + 8.0f * (multiplier - float(multiplier_integral) - 0.5f);
  CONSTRAIN(multiplier_fractional, 0.0f, 1.0f);
  const float max_delay = float(std::max(dc.max_delay_, 1));
  int n = 0;
  for (int interval = 0; interval < 2; ++interval) {
    for (int ratio = 0; ratio < 2; ++ratio) {
      for (int tap = 0; tap < 2 && n < max_out; ++tap) {
        float delay_time = dc.clock_.interval(interval) * tap_ratio[ratio] * float(multiplier_integral + tap);
        CONSTRAIN(delay_time, 68.0f, max_delay);
        const float fade = tap ? multiplier_fractional : 1.0f - multiplier_fractional;
        const float gain = tap_gain[ratio] * fade * dc.clock_.gain(interval);
        if (gain < 0.02f) continue;
        BeadsGrainView& gv = out[n++];
        gv.age = delay_time / max_delay;
        gv.env = gain;
        gv.pan = 0.5f;
        gv.speed = 1.0f;
      }
    }
  }
  return n;
}
}  // namespace

extern "C" {

int beads_controls_size() { return int(sizeof(BeadsControls)); }
int beads_view_size() { return int(sizeof(BeadsView)); }

void* beads_create(float host_rate) {
  Host* h = new (std::nothrow) Host();
  if (!h) return NULL;
  std::memset(h->block_1, 0, sizeof(h->block_1));
  std::memset(h->block_2, 0, sizeof(h->block_2));
  std::memset(h->reverb_buffer, 0, sizeof(h->reverb_buffer));
  uint8_t* buffers[3] = { h->block_1, h->block_1 + 256 * 1024, h->block_2 };
  h->processor.Init(buffers, h->reverb_buffer);
  h->synth.Init(h->reverb_buffer);
  h->gain_control.Init(48000.0f, GainControl::SOFT_CLIPPING_MODE_OFF, true);
  // cv_reader.cc: CvReader::Init.
  for (int i = 0; i < POTS_ADC_CHANNEL_LAST; ++i) {
    PotChannel::Law law = PotChannel::LAW_NORMAL;
    if (i >= POTS_ADC_CHANNEL_TIME_AMOUNT && i <= POTS_ADC_CHANNEL_PITCH_AMOUNT) {
      law = PotChannel::LAW_ATTENUVERTER;
    } else if (i == POTS_ADC_CHANNEL_PITCH) {
      law = PotChannel::LAW_DETUNE;
    }
    h->pots[i].Init(law, 0.05f, i == POTS_ADC_CHANNEL_TIME ? 0.001f : 0.0f);
  }
  h->started = false;
  h->synth_mode = false;
  h->no_input_time = 0.0f;
  h->host_rate = host_rate > 0.0f ? host_rate : 48000.0f;
  h->seed_led = false;
  // The module boots with the digital medium in stereo.
  h->processor.Configure(Processor::MEDIA_DIGITAL, false);
  StartCodec(h, false, true);
  h->started = false;  // the first render configures for real
  return h;
}

void beads_destroy(void* handle) { delete static_cast<Host*>(handle); }

void beads_render(
    void* handle,
    const BeadsControls* controls,
    const float* in_l,
    const float* in_r,
    float* out_l,
    float* out_r,
    int n,
    float host_rate) {
  Host* h = static_cast<Host*>(handle);
  const BeadsControls& c = *controls;
  h->last_seed_button = c.seed_button;
  h->last_seed_patched = c.seed_patched != 0;
  if (host_rate > 0.0f && std::fabs(host_rate - h->host_rate) > 0.5f) {
    h->host_rate = host_rate;
    StartCodec(h, h->synth_mode, false);
  }

  int done = 0;
  while (done < n) {
    const int chunk = std::min(256, n - done);
    MonitorStateChange(h, c, float(chunk) / h->host_rate);

    CodecSink codec_sink = { h };
    for (int i = 0; i < chunk; ++i) {
      const float l = in_l ? in_l[done + i] : 0.0f;
      const float r = in_r ? in_r[done + i] : l;
      h->down.Push(l, r, codec_sink);
    }

    HostSink host_sink = { h };
    FloatFrame block_in[kMaxCodecBlockSize];
    FloatFrame block_out[kMaxCodecBlockSize];
    const int size = h->block_size;
    while (h->codec_in.size >= size) {
      for (int i = 0; i < size; ++i) block_in[i] = h->codec_in.Pop();
      RunBlock(h, c, block_in, block_out, size_t(size));
      for (int i = 0; i < size; ++i) h->up.Push(block_out[i].l, block_out[i].r, host_sink);
    }

    for (int i = 0; i < chunk; ++i) {
      float l = 0.0f, r = 0.0f;
      if (h->host_out.size > 0) {
        const FloatFrame f = h->host_out.Pop();
        l = f.l;
        r = f.r;
      }
      out_l[done + i] = l;
      out_r[done + i] = r;
      if (++h->scope_phase >= 4) {
        h->scope_phase = 0;
        h->scope[h->scope_write] = 0.5f * (l + r);
        h->scope_write = (h->scope_write + 1) % kScopeSize;
      }
    }
    done += chunk;
  }

  if (!h->synth_mode) {
    WithActiveDelayLine(h->processor, [&](const auto& dl) { Scan(h, dl); });
  }
}

void beads_view(void* handle, BeadsView* v) {
  Host* h = static_cast<Host*>(handle);
  Processor& p = h->processor;
  std::memset(v, 0, sizeof(*v));
  v->media = int32_t(p.media_);
  v->mono = p.mono_;
  v->codec_rate = h->codec_rate;
  v->vu = float(h->gain_control.vu_meter()) / 255.0f;
  v->agc_adjusting = h->gain_control.adjusting();
  v->seed_led = h->seed_led;
  v->synth_countdown = (!h->synth_mode && h->no_input_time > 0.0f)
      ? std::max(0.0f, kSynthModeDelay - h->no_input_time)
      : -1.0f;
  v->overwrite_reserve = h->synth_mode ? 1.0f : p.overwrite_reserve();
  for (int i = 0; i < kScopeSize; ++i) {
    v->scope[i] = h->scope[(h->scope_write + i) % kScopeSize];
  }

  if (h->synth_mode) {
    v->mode = 2;
    WavetableGranularPlayer& player = h->synth.wavetable_granular_player_;
    int n = 0;
    for (int i = 0; i < WavetableGranularPlayer::NUM_GRAINS && n < kMaxGrains; ++i) {
      const WavetableGrain& g = player.grain_[i];
      if (g.done()) continue;
      BeadsGrainView& gv = v->grains[n++];
      gv.age = g.phase_;
      gv.env = GrainEnvelopeValue(g.envelope_) * StealingValue(g.voice_stealing_envelope_);
      gv.pan = 0.5f;
      gv.speed = g.frequency_ * 48000.0f;
    }
    v->num_grains = n;
  } else {
    const bool delay_mode = p.delay_mode_transition_ > 0.0f;
    v->mode = h->transition_blocks ? 3 : (delay_mode ? 1 : 0);
    const float internal_rate = p.sample_rate() / float(p.ratio());

    WithActiveProcessor(p, [&](auto& gp) {
      auto fill = [&](const auto& dl) {
        typedef typename std::decay<decltype(dl)>::type DL;
        const int length = DL::MAX_DELAY;
        v->buffer_seconds = float(length) / internal_rate;
        v->frozen = dl.frozen();
        v->recorded = std::min(1.0f, h->recorded_seconds / v->buffer_seconds);
        const int wp = dl.write_ptr_;
        for (int k = 0; k < kOverviewSize; ++k) {
          const int64_t pos = (int64_t(wp) + int64_t(k) * length / kOverviewSize) % length;
          v->overview[k] = h->overview_abs[int(pos * kOverviewSize / length)];
        }
        int n = 0;
        if (!delay_mode) {
          for (int i = 0; i < DelayGranularPlayer::NUM_GRAINS && n < kMaxGrains; ++i) {
            const DelayGrain& g = gp.granular_player_.grain_[i];
            if (g.done()) continue;
            BeadsGrainView& gv = v->grains[n++];
            gv.age = float(g.delay_ >> 10) / float(length);
            gv.env = GrainEnvelopeValue(g.envelope_) * StealingValue(g.voice_stealing_envelope_);
            const float left = g.gain_[0][0] + g.gain_[0][1];
            const float right = g.gain_[1][0] + g.gain_[1][1];
            gv.pan = (left + right) > 0.0f ? right / (left + right) : 0.5f;
            gv.speed = float(g.speed_) / 1024.0f;
          }
        } else if (dl.frozen()) {
          // Frozen delay: the controller's play heads slice the buffer.
          const DelayController& dc = p.delay_controller_;
          const float max_delay = float(std::max(dc.max_delay_, 1));
          for (int i = 0; i < DelayController::NUM_HEADS && n < kMaxGrains; ++i) {
            for (int slave = 0; slave < 2 && n < kMaxGrains; ++slave) {
              const PlayHead& head = slave ? dc.slave_play_head_[i] : dc.play_head_[i];
              if (head.done()) continue;
              BeadsGrainView& gv = v->grains[n++];
              gv.age = float(head.start_ + head.remaining_) / max_delay;
              gv.env = head.gain_ * HeadEnvelope(head, dc.shape_, slave != 0);
              gv.pan = 0.5f;
              gv.speed = 1.0f;
            }
          }
        } else {
          n = DelayTaps(h, p.delay_controller_, v->grains, kMaxGrains);
        }
        v->num_grains = n;
      };
      if (p.mono_) fill(gp.mono_delay_line_); else fill(gp.stereo_delay_line_);
    });
  }
  for (int i = 0; i < v->num_grains; ++i) {
    CONSTRAIN(v->grains[i].age, 0.0f, 1.0f);
    CONSTRAIN(v->grains[i].env, 0.0f, 1.0f);
  }
}

}  // extern "C"
