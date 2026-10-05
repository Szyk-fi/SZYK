// A just-enough stand-in for Steinberg's VST2 AudioEffectX, so Airwindows'
// plugins (MIT, (c) Chris Johnson) compile unchanged as plain DSP classes.
// Nothing here processes audio: every effect's own processReplacing() does
// that. This only gives them the base class, constants and string helpers
// their constructors and parameter-display code call, and a sample rate.
//
// Differences from the real SDK, deliberately: the name/label length limits
// are larger (the real kVstMaxParamStrLen is 8, which truncated names such
// as "Dry/Wet" in hosts), and there is no host callback at all.
#pragma once
#define __audioeffect__

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <set>
#include <string>

typedef int32_t VstInt32;
typedef intptr_t VstIntPtr;
typedef void* audioMasterCallback;

enum VstPlugCategory {
  kPlugCategUnknown = 0, kPlugCategEffect, kPlugCategSynth, kPlugCategAnalysis,
  kPlugCategMastering, kPlugCategSpacializer, kPlugCategRoomFx, kPlugSurroundFx,
  kPlugCategRestoration, kPlugCategOfflineProcess, kPlugCategShell, kPlugCategGenerator
};

const int kVstMaxProgNameLen = 64;
const int kVstMaxParamStrLen = 48;
const int kVstMaxProductStrLen = 64;
const int kVstMaxVendorStrLen = 64;
const int kVstMaxEffectNameLen = 64;

inline char* vst_strncpy(char* dst, const char* src, size_t maxLen) {
  char* d = dst;
  while (maxLen && *src) { *d++ = *src++; --maxLen; }
  *d = 0;
  return dst;
}

class AudioEffect {
 public:
  AudioEffect(audioMasterCallback, VstInt32 programs, VstInt32 params)
      : sampleRate_(44100.0f), numParams_(params), numPrograms_(programs) {}
  virtual ~AudioEffect() {}
  // Some plugins never initialise a member or two (PunchyGuitar's gateL and
  // gateR) and rely on `new` handing back zeroed memory, as a host's fresh
  // heap usually does. Make that deterministic instead of lucky.
  static void* operator new(size_t n) { void* p = calloc(1, n); if (!p) abort(); return p; }
  static void operator delete(void* p) { free(p); }
  float getSampleRate() { return sampleRate_; }
  void setSampleRate(float r) { sampleRate_ = r; }
  VstInt32 getNumParameters() const { return numParams_; }
  virtual float getParameter(VstInt32) { return 0.0f; }
  virtual void setParameter(VstInt32, float) {}
  virtual void getParameterName(VstInt32, char* t) { *t = 0; }
  virtual void getParameterDisplay(VstInt32, char* t) { *t = 0; }
  virtual void getParameterLabel(VstInt32, char* t) { *t = 0; }
  virtual void processReplacing(float**, float**, VstInt32) {}
  virtual bool getEffectName(char* n) { *n = 0; return false; }
  void setNumInputs(VstInt32) {}
  void setNumOutputs(VstInt32) {}
  void setUniqueID(VstInt32) {}
  void canProcessReplacing(bool = true) {}
  void canDoubleReplacing(bool = true) {}
  void programsAreChunks(bool = true) {}
  void canMono(bool = true) {}
  void isSynth(bool = true) {}
 private:
  float sampleRate_;
  VstInt32 numParams_, numPrograms_;
};

class AudioEffectX : public AudioEffect {
 public:
  AudioEffectX(audioMasterCallback m, VstInt32 programs, VstInt32 params) : AudioEffect(m, programs, params) {}
};

inline void float2string(float value, char* text, VstInt32 maxLen) {
  snprintf(text, maxLen, "%.2f", value);
}
inline void int2string(VstInt32 value, char* text, VstInt32 maxLen) {
  snprintf(text, maxLen, "%d", (int)value);
}
inline void dB2string(float value, char* text, VstInt32 maxLen) {
  if (value <= 0.0f) vst_strncpy(text, "-inf", maxLen);
  else snprintf(text, maxLen, "%.2f", 20.0 * log10((double)value));
}
