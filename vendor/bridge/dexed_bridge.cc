// C ABI around Dexed's msfa FM engine (vendor/dexed/msfa, Apache-2.0, (c)
// Pascal Gauthier and Google): sixteen Dx7Note voices, the DX7's LFO and the
// FmCore operator kernel, run the way Dexed's plugin does -- one LFO sample
// per 64-sample block, each live voice computed into a shared buffer, the sum
// scaled to a float. No synthesis of our own: this is only the part Dexed's
// plugin shell does around the engine (voice allocation, unpacking a
// 32-voice bank's packed voice into the engine's 156-byte layout, controllers).
//
// Only Dexed's "Modern" engine is here (msfa's FmCore); its Mark I and OPL
// engines are GPL and are not used.

#include <cstdint>
#include <cstring>
#include <memory>

#include "../dexed/msfa/controllers.h"
#include "../dexed/msfa/dx7note.h"
#include "../dexed/msfa/env.h"
#include "../dexed/msfa/exp2.h"
#include "../dexed/msfa/fm_core.h"
#include "../dexed/msfa/freqlut.h"
#include "../dexed/msfa/lfo.h"
#include "../dexed/msfa/pitchenv.h"
#include "../dexed/msfa/porta.h"
#include "../dexed/msfa/sin.h"
#include "../dexed/msfa/synth.h"
#include "../dexed/msfa/tuning.h"

namespace {

const int kVoices = 16;

struct Voice {
  Dx7Note* note = nullptr;
  int midi = -1;
  bool keydown = false;
  bool sustained = false;
  bool live = false;
  uint32_t seq = 0;
};

struct Handle {
  Voice voices[kVoices];
  uint8_t patch[156];
  Lfo lfo;
  Controllers ctrl;
  FmCore core;
  std::shared_ptr<TuningState> tuning;
  bool sustain = false;
  uint32_t next_seq = 1;
  double rate = 0;
  // Samples of the current 64-sample block not yet handed out.
  float block[N];
  int block_pos = N;
};

bool g_tables_ready = false;

void init_rate(double rate) {
  Freqlut::init(rate);
  Lfo::init(rate);
  PitchEnv::init(rate);
  Env::init_sr(rate);
  Porta::init_sr(rate);
}

// The DX7's own power-on voice: "INIT VOICE" -- operator 1 audible, the rest
// silent, algorithm 1.
void init_voice(uint8_t* d) {
  std::memset(d, 0, 156);
  for (int op = 0; op < 6; ++op) {
    uint8_t* o = d + op * 21;
    o[0] = o[1] = o[2] = o[3] = 99;     // EG rates
    o[4] = o[5] = o[6] = 99; o[7] = 0;  // EG levels
    o[8] = 39;                          // break point
    o[11] = o[12] = 0;                  // curves
    o[16] = op == 5 ? 99 : 0;           // output level (the last block is operator 1)
    o[18] = 1;                          // coarse
    o[20] = 7;                          // detune
  }
  d[126] = d[127] = d[128] = d[129] = 99;
  d[130] = d[131] = d[132] = d[133] = 50;
  d[137] = 35; d[140] = 0;              // LFO speed, amp mod depth
  d[142] = 0;
  d[144] = 24;                          // transpose: C3
  std::memcpy(d + 145, "INIT VOICE", 10);
  d[155] = 63;
}

int choose_voice(Handle* h, int midi) {
  // Same note again: reuse it. Else a silent voice, else the oldest released,
  // else the oldest held.
  for (int i = 0; i < kVoices; ++i)
    if (h->voices[i].live && h->voices[i].keydown && h->voices[i].midi == midi) return i;
  for (int i = 0; i < kVoices; ++i)
    if (!h->voices[i].live) return i;
  int best = 0, best_score = -1;
  for (int i = 0; i < kVoices; ++i) {
    int score = (h->voices[i].keydown ? 0 : 2);
    if (score > best_score || (score == best_score && h->voices[i].seq < h->voices[best].seq)) {
      best = i;
      best_score = score;
    }
  }
  return best;
}

void render_block(Handle* h) {
  AlignedBuf<int32_t, N> audio;
  for (int j = 0; j < N; ++j) {
    audio.get()[j] = 0;
    h->block[j] = 0;
  }
  h->ctrl.refresh();
  int32_t lfo_value = h->lfo.getsample();
  int32_t lfo_delay = h->lfo.getdelay();
  for (int i = 0; i < kVoices; ++i) {
    Voice& v = h->voices[i];
    if (!v.live) continue;
    v.note->compute(audio.get(), lfo_value, lfo_delay, &h->ctrl);
    for (int j = 0; j < N; ++j) {
      int32_t val = audio.get()[j] >> 4;
      int clip = val < -(1 << 24) ? 0x8000 : val >= (1 << 24) ? 0x7fff : val >> 9;
      float f = static_cast<float>(clip) / static_cast<float>(0x8000);
      h->block[j] += f > 1 ? 1 : (f < -1 ? -1 : f);
      audio.get()[j] = 0;
    }
    if (!v.keydown && !v.sustained && !v.note->isPlaying()) v.live = false;
  }
}

}  // namespace

extern "C" {

void* dexed_create(double rate) {
  if (!g_tables_ready) {
    Exp2::init();
    Tanh::init();
    Sin::init();
    g_tables_ready = true;
  }
  init_rate(rate);
  Handle* h = new Handle();
  h->rate = rate;
  h->tuning = createStandardTuning();
  for (int i = 0; i < kVoices; ++i) h->voices[i].note = new Dx7Note(h->tuning, nullptr);
  init_voice(h->patch);
  h->lfo.reset(h->patch + 137);
  h->ctrl.values_[kControllerPitch] = 0x2000;
  h->ctrl.values_[kControllerPitchRangeUp] = 2;
  h->ctrl.values_[kControllerPitchRangeDn] = 2;
  h->ctrl.values_[kControllerPitchStep] = 0;
  h->ctrl.masterTune = 0;
  h->ctrl.modwheel_cc = h->ctrl.breath_cc = h->ctrl.foot_cc = h->ctrl.aftertouch_cc = 0;
  h->ctrl.portamento_enable_cc = false;
  h->ctrl.portamento_cc = 0;
  h->ctrl.mpeEnabled = false;
  h->ctrl.core = &h->core;
  // Dexed's defaults: the mod wheel adds vibrato at half depth.
  h->ctrl.wheel.parseConfig("50 1 0 0");
  h->ctrl.foot.parseConfig("0 0 0 0");
  h->ctrl.breath.parseConfig("0 0 0 0");
  h->ctrl.at.parseConfig("0 0 0 0");
  h->ctrl.refresh();
  return h;
}

void dexed_destroy(void* handle) {
  Handle* h = static_cast<Handle*>(handle);
  for (int i = 0; i < kVoices; ++i) delete h->voices[i].note;
  delete h;
}

void dexed_set_rate(void* handle, double rate) {
  Handle* h = static_cast<Handle*>(handle);
  if (rate != h->rate) {
    h->rate = rate;
    init_rate(rate);
  }
}

// One voice in the DX7's 128-byte packed (32-voice bank) layout, unpacked
// into the engine's 156-byte layout (the format Dexed itself unpacks to).
void dexed_load_packed_voice(void* handle, const uint8_t* b) {
  Handle* h = static_cast<Handle*>(handle);
  uint8_t d[156];
  std::memset(d, 0, sizeof(d));
  for (int op = 0; op < 6; ++op) {
    const uint8_t* s = b + op * 17;
    uint8_t* o = d + op * 21;
    std::memcpy(o, s, 11);
    o[11] = s[11] & 3;
    o[12] = (s[11] >> 2) & 3;
    o[13] = s[12] & 7;
    o[20] = (s[12] >> 3) & 15;
    o[14] = s[13] & 3;
    o[15] = (s[13] >> 2) & 7;
    o[16] = s[14];
    o[17] = s[15] & 1;
    o[18] = (s[15] >> 1) & 31;
    o[19] = s[16];
  }
  std::memcpy(d + 126, b + 102, 8);  // pitch EG
  d[134] = b[110] & 31;              // algorithm
  d[135] = b[111] & 7;               // feedback
  d[136] = (b[111] >> 3) & 1;        // oscillator key sync
  std::memcpy(d + 137, b + 112, 4);  // LFO speed, delay, pitch depth, amp depth
  d[141] = b[116] & 1;               // LFO key sync
  d[142] = (b[116] >> 1) & 7;        // LFO waveform
  d[143] = (b[116] >> 4) & 7;        // LFO pitch mod sensitivity
  d[144] = b[117];                   // transpose
  std::memcpy(d + 145, b + 118, 10);
  d[155] = 63;
  std::memcpy(h->patch, d, sizeof(d));
  h->lfo.reset(h->patch + 137);
}

// A single voice already in the unpacked 155-byte layout (the single-voice
// SysEx dump).
void dexed_load_unpacked_voice(void* handle, const uint8_t* d155) {
  Handle* h = static_cast<Handle*>(handle);
  std::memcpy(h->patch, d155, 155);
  h->patch[155] = 63;
  h->lfo.reset(h->patch + 137);
}

void dexed_note_on(void* handle, int midi, int velocity) {
  Handle* h = static_cast<Handle*>(handle);
  int pitch = midi + h->patch[144] - 24;
  if (pitch < 0 || pitch > 127) return;
  bool any_down = false;
  for (int i = 0; i < kVoices; ++i) any_down = any_down || h->voices[i].keydown;
  if (!any_down) h->lfo.keydown();
  int i = choose_voice(h, midi);
  Voice& v = h->voices[i];
  bool stolen = v.note->isPlaying();
  v.midi = midi;
  v.keydown = true;
  v.sustained = h->sustain;
  v.live = true;
  v.seq = h->next_seq++;
  v.note->init(h->patch, pitch, velocity > 127 ? 127 : velocity, 1, &h->ctrl);
  if (h->patch[136] && !stolen) v.note->oscSync();
}

void dexed_note_off(void* handle, int midi) {
  Handle* h = static_cast<Handle*>(handle);
  for (int i = 0; i < kVoices; ++i) {
    Voice& v = h->voices[i];
    if (v.live && v.keydown && v.midi == midi) {
      v.keydown = false;
      if (h->sustain) v.sustained = true;
      else v.note->keyup();
      return;
    }
  }
}

void dexed_sustain(void* handle, int on) {
  Handle* h = static_cast<Handle*>(handle);
  h->sustain = on != 0;
  if (!h->sustain) {
    for (int i = 0; i < kVoices; ++i) {
      Voice& v = h->voices[i];
      if (v.live && v.sustained && !v.keydown) {
        v.sustained = false;
        v.note->keyup();
      }
    }
  }
}

void dexed_all_off(void* handle) {
  Handle* h = static_cast<Handle*>(handle);
  for (int i = 0; i < kVoices; ++i) {
    Voice& v = h->voices[i];
    if (v.live) {
      v.keydown = false;
      v.sustained = false;
      v.note->keyup();
    }
  }
}

// pitch_bend 0..16383 (8192 centre); the others 0..127.
void dexed_controllers(void* handle, int pitch_bend, int mod_wheel, int breath, int foot, int aftertouch, int bend_range) {
  Handle* h = static_cast<Handle*>(handle);
  h->ctrl.values_[kControllerPitch] = pitch_bend < 0 ? 0 : (pitch_bend > 16383 ? 16383 : pitch_bend);
  h->ctrl.values_[kControllerPitchRangeUp] = bend_range;
  h->ctrl.values_[kControllerPitchRangeDn] = bend_range;
  h->ctrl.modwheel_cc = mod_wheel;
  h->ctrl.breath_cc = breath;
  h->ctrl.foot_cc = foot;
  h->ctrl.aftertouch_cc = aftertouch;
}

// Mono output, any length; blocks are rendered 64 samples at a time and
// carried over between calls.
void dexed_render(void* handle, float* out, int n) {
  Handle* h = static_cast<Handle*>(handle);
  for (int i = 0; i < n; ++i) {
    if (h->block_pos >= N) {
      render_block(h);
      h->block_pos = 0;
    }
    out[i] = h->block[h->block_pos++];
  }
}

int dexed_active_voices(void* handle) {
  Handle* h = static_cast<Handle*>(handle);
  int n = 0;
  for (int i = 0; i < kVoices; ++i) n += h->voices[i].live ? 1 : 0;
  return n;
}

}  // extern "C"
