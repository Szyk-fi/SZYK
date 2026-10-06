// C ABI over Airwindows' effects (vendor/airwindows, MIT, (c) Chris
// Johnson). No DSP of its own: every effect's processReplacing() runs
// untouched; this builds one from the generated registry (see build.rs),
// reads its parameter names/units/values the way a VST host does, and
// feeds it audio.

#include <cstring>

#include "audioeffectx.h"

struct AwEntry {
  const char* id;
  AudioEffect* (*make)(audioMasterCallback);
};
extern const AwEntry aw_registry[];
extern const int aw_registry_len;

extern "C" {

int aw_count() { return aw_registry_len; }

// The effect's folder name, which is also its Airwindows name.
const char* aw_id(int i) {
  return (i >= 0 && i < aw_registry_len) ? aw_registry[i].id : "";
}

void* aw_create(int i, float sample_rate) {
  if (i < 0 || i >= aw_registry_len) return nullptr;
  AudioEffect* e = aw_registry[i].make(nullptr);
  if (e) e->setSampleRate(sample_rate);
  return e;
}

// Effects read the rate every block, so it can follow the device.
void aw_set_rate(void* h, float sample_rate) { if (h) static_cast<AudioEffect*>(h)->setSampleRate(sample_rate); }

void aw_destroy(void* h) { delete static_cast<AudioEffect*>(h); }

int aw_param_count(void* h) {
  return h ? static_cast<AudioEffect*>(h)->getNumParameters() : 0;
}

static void text(void (AudioEffect::*get)(VstInt32, char*), void* h, int p, char* out, int n) {
  if (n <= 0) return;
  char buf[128] = {0};
  if (h) (static_cast<AudioEffect*>(h)->*get)(p, buf);
  std::strncpy(out, buf, n - 1);
  out[n - 1] = 0;
}

void aw_param_name(void* h, int p, char* out, int n) { text(&AudioEffect::getParameterName, h, p, out, n); }
void aw_param_display(void* h, int p, char* out, int n) { text(&AudioEffect::getParameterDisplay, h, p, out, n); }
void aw_param_label(void* h, int p, char* out, int n) { text(&AudioEffect::getParameterLabel, h, p, out, n); }
float aw_get(void* h, int p) { return h ? static_cast<AudioEffect*>(h)->getParameter(p) : 0.0f; }
void aw_set(void* h, int p, float v) { if (h) static_cast<AudioEffect*>(h)->setParameter(p, v); }

// Stereo in, stereo out; the buffers may not alias.
void aw_process(void* h, const float* in_l, const float* in_r, float* out_l, float* out_r, int n) {
  AudioEffect* e = static_cast<AudioEffect*>(h);
  float* in[2] = {const_cast<float*>(in_l), const_cast<float*>(in_r)};
  float* out[2] = {out_l, out_r};
  e->processReplacing(in, out, n);
}

}  // extern "C"
