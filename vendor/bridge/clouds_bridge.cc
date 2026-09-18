// C ABI wrapper around Mutable Instruments' Clouds granular processor
// (from vendor/eurorack/clouds, MIT-licensed, (c) Emilie Gillet), so it
// can be called from Rust via a plain extern "C" boundary. This file
// contains no DSP code of its own -- it just adapts
// clouds::GranularProcessor's interface to something FFI-friendly.
//
// Buffer sizes (118784 + 65536-128 bytes) and the Process()-then-
// Prepare() call order come straight from the real firmware
// (clouds/clouds.cc) and Mutable's own standalone test harness
// (clouds/test/clouds_test.cc) -- not a guess. granular.overlap/
// window_shape aren't set here because GranularProcessor derives them
// itself from density/texture inside Prepare().

#include <cstdint>
#include <vector>

#include "clouds/dsp/granular_processor.h"

namespace {

const size_t kLargeBufferSize = 118784;
const size_t kSmallBufferSize = 65536 - 128;

struct Handle {
  clouds::GranularProcessor processor;
  std::vector<uint8_t> large_buffer;
  std::vector<uint8_t> small_buffer;
};

}  // namespace

extern "C" {

void* clouds_processor_create() {
  Handle* h = new Handle();
  h->large_buffer.resize(kLargeBufferSize);
  h->small_buffer.resize(kSmallBufferSize);
  h->processor.Init(
      h->large_buffer.data(), h->large_buffer.size(),
      h->small_buffer.data(), h->small_buffer.size());
  h->processor.set_num_channels(2);
  h->processor.set_low_fidelity(false);
  h->processor.set_playback_mode(clouds::PLAYBACK_MODE_GRANULAR);
  h->processor.Prepare();
  return h;
}

void clouds_processor_destroy(void* handle) {
  delete static_cast<Handle*>(handle);
}

// `in`/`out` are interleaved stereo float (L, R, L, R, ...), -1..1
// range, `num_frames` frames long, at Clouds' fixed internal 32kHz --
// the Rust side resamples to the device's actual rate. `num_frames` may
// be any length; internally chunked into kMaxBlockSize (32) pieces,
// since GranularProcessor's own scratch buffers are sized for exactly
// that.
void clouds_processor_render(
    void* handle,
    int playback_mode,
    float position,
    float size,
    float pitch,
    float density,
    float texture,
    float dry_wet,
    float stereo_spread,
    float feedback,
    float reverb,
    int freeze,
    int trigger,
    const float* in_interleaved,
    float* out_interleaved,
    int num_frames) {
  Handle* h = static_cast<Handle*>(handle);

  clouds::PlaybackMode mode = static_cast<clouds::PlaybackMode>(
      playback_mode < 0 || playback_mode >= clouds::PLAYBACK_MODE_LAST
          ? 0
          : playback_mode);
  h->processor.set_playback_mode(mode);

  clouds::Parameters* p = h->processor.mutable_parameters();
  p->position = position;
  p->size = size;
  p->pitch = pitch;
  p->density = density;
  p->texture = texture;
  p->dry_wet = dry_wet;
  p->stereo_spread = stereo_spread;
  p->feedback = feedback;
  p->reverb = reverb;
  p->freeze = freeze != 0;
  p->trigger = trigger != 0;
  p->gate = trigger != 0;

  int remaining = num_frames;
  const float* src = in_interleaved;
  float* dst = out_interleaved;
  while (remaining > 0) {
    int chunk = remaining > static_cast<int>(clouds::kMaxBlockSize)
        ? static_cast<int>(clouds::kMaxBlockSize)
        : remaining;
    clouds::ShortFrame in_frames[clouds::kMaxBlockSize];
    clouds::ShortFrame out_frames[clouds::kMaxBlockSize];
    for (int i = 0; i < chunk; i++) {
      in_frames[i].l = static_cast<short>(src[i * 2 + 0] * 32767.0f);
      in_frames[i].r = static_cast<short>(src[i * 2 + 1] * 32767.0f);
    }
    h->processor.Process(in_frames, out_frames, chunk);
    h->processor.Prepare();
    for (int i = 0; i < chunk; i++) {
      dst[i * 2 + 0] = out_frames[i].l / 32768.0f;
      dst[i * 2 + 1] = out_frames[i].r / 32768.0f;
    }
    src += chunk * 2;
    dst += chunk * 2;
    remaining -= chunk;
  }
}

}  // extern "C"
