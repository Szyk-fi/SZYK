// The few host-side pieces Dexed's msfa engine expects its plugin to supply:
// the standard 12-tone tuning table (the same one Dexed's own tuning.cc builds
// for "Standard Tuning") and the trace hook, which does nothing here.

#include <cstdarg>
#include <memory>

#include "../msfa/tuning.h"

namespace {

struct StandardTuning : public TuningState {
  StandardTuning() {
    const int base = 50857777;  // (1 << 24) * (log(440) / log(2) - 69/12)
    const int step = (1 << 24) / 12;
    for (int n = 0; n < 128; ++n) table_[n] = base + step * n;
  }
  int32_t midinote_to_logfreq(int midinote) override {
    return table_[midinote < 0 ? 0 : (midinote > 127 ? 127 : midinote)];
  }
  int table_[128];
};

}  // namespace

std::shared_ptr<TuningState> createStandardTuning() { return std::make_shared<StandardTuning>(); }

void dexed_trace(const char*, const char*, ...) {}
