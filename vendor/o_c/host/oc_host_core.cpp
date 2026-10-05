// The "hardware" under the O&C firmware, on a computer -- the part shared by
// every instance.
//
// Several firmwares run at once (O&C 1-4): the firmware is compiled once per
// instance into its own namespace (build.rs), and everything it expects of a
// Teensy 3.2 and the module around it -- pin levels, EEPROM, the CV inputs and
// DAC outputs, the OLED's frame, the two timer interrupts -- is kept here in a
// table indexed by instance. The firmware itself calls these functions without
// an instance number; which instance it means is the calling thread's: a
// firmware thread is bound to its instance when it starts, and the host's audio
// thread binds itself to one for the length of each call into it.
//
// Each firmware's setup() and loop() run on a thread of their own, blocking
// calls and all; its timer interrupts are run by the host's audio thread at the
// rates it asks for, so its CV and triggers keep sample time.

#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <thread>

#include <Arduino.h>
#include <EEPROM.h>

SerialClass Serial;
EEPROMClass EEPROM;
extern "C" volatile uint32_t oc_host_cycle_counter = 0;

namespace {

constexpr int kInstances = 4;
constexpr int kEeprom = 8192;

using Clock = std::chrono::steady_clock;
const Clock::time_point g_epoch = Clock::now();

struct Instance {
  // Pins idle high: the buttons and triggers are pulled up and read low when active.
  std::atomic<uint8_t> pin[64];
  void (*handler[64])() = {nullptr};
  int handler_mode[64] = {0};
  void (*core_isr)() = nullptr;
  void (*ui_isr)() = nullptr;
  std::atomic<bool> core_begun{false};
  std::atomic<bool> ui_begun{false};
  std::atomic<int32_t> cv_mv[4];
  std::atomic<uint32_t> dac_code[4];
  uint8_t eeprom[kEeprom] = {0};
  uint8_t back[1024] = {0};
  uint8_t front[1024] = {0};
  std::mutex front_mutex;
  std::atomic<uint64_t> frames{0};
  Clock::time_point last_frame = Clock::now();
  uint32_t random = 0x2545F491u;
  std::atomic<bool> started{false};

  Instance() {
    for (auto &p : pin) p = 1;
    for (auto &c : cv_mv) c = 0;
    for (auto &d : dac_code) d = 0;
  }
};

Instance g_inst[kInstances];
thread_local int t_inst = 0;

Instance &me() { return g_inst[t_inst]; }

// A binding of the calling thread to an instance for a scope.
struct Bind {
  int previous;
  explicit Bind(int inst) : previous(t_inst) { t_inst = (inst >= 0 && inst < kInstances) ? inst : 0; }
  ~Bind() { t_inst = previous; }
};

}  // namespace

// Each instance's firmware, from its own translation units (see build.rs).
extern "C" {
void oc0_main();
void oc1_main();
void oc2_main();
void oc3_main();
const char *oc0_app_name();
const char *oc1_app_name();
const char *oc2_app_name();
const char *oc3_app_name();
}

// ---- What the firmware calls (the calling thread's instance) ------------------

extern "C" {

void oc_host_pin_mode(uint8_t, uint8_t) {}
int oc_host_pin_read(uint8_t pin) { return pin < 64 ? me().pin[pin].load(std::memory_order_relaxed) : 1; }
void oc_host_pin_write(uint8_t, uint8_t) {}
unsigned long oc_host_millis() {
  return (unsigned long)std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now() - g_epoch).count();
}
void oc_host_delay(unsigned long ms) { std::this_thread::sleep_for(std::chrono::milliseconds(ms)); }
void oc_host_attach(uint8_t pin, void (*fn)(), int mode) {
  if (pin < 64) {
    me().handler[pin] = fn;
    me().handler_mode[pin] = mode;
  }
}
void oc_host_timer_begin(void (*fn)(), unsigned long microseconds) {
  // The firmware starts its core timer at 60 us and its UI timer at 1 ms.
  Instance &m = me();
  if (microseconds <= 100) {
    m.core_isr = fn;
    m.core_begun = true;
  } else {
    m.ui_isr = fn;
    m.ui_begun = true;
  }
}
uint32_t oc_host_random32() {
  Instance &m = me();
  m.random ^= m.random << 13;
  m.random ^= m.random >> 17;
  m.random ^= m.random << 5;
  return m.random;
}
void oc_host_random_seed(uint32_t seed) { me().random = seed ? seed : 0x2545F491u; }

uint8_t *oc_host_eeprom_ptr() { return me().eeprom; }

int oc_host_cv_millivolts(int channel) { return (channel >= 0 && channel < 4) ? me().cv_mv[channel].load(std::memory_order_relaxed) : 0; }
void oc_host_dac_write(int channel, uint32_t code) {
  if (channel >= 0 && channel < 4) me().dac_code[channel].store(code, std::memory_order_relaxed);
}

// A panel refreshes at a few hundred hertz at most. Screens that draw in a
// tight loop (the splash, calibration) are held to that.
uint8_t *oc_host_begin_frame() {
  Instance &m = me();
  auto since = Clock::now() - m.last_frame;
  if (since < std::chrono::milliseconds(4)) std::this_thread::sleep_for(std::chrono::milliseconds(4) - since);
  m.last_frame = Clock::now();
  return m.back;
}

void oc_host_end_frame() {
  Instance &m = me();
  std::lock_guard<std::mutex> lock(m.front_mutex);
  memcpy(m.front, m.back, sizeof(m.front));
  m.frames.fetch_add(1, std::memory_order_release);
}

}  // extern "C"

// ---- The host's side ----------------------------------------------------------

extern "C" {

// Starts an instance's firmware: setup(), then loop() forever, on a thread of its own.
void oc_start(int inst) {
  if (inst < 0 || inst >= kInstances) return;
  if (g_inst[inst].started.exchange(true)) return;
  std::thread([inst] {
    t_inst = inst;
    switch (inst) {
      case 0: oc0_main(); break;
      case 1: oc1_main(); break;
      case 2: oc2_main(); break;
      default: oc3_main(); break;
    }
  }).detach();
}

int oc_started(int inst) { return inst >= 0 && inst < kInstances && g_inst[inst].started.load() ? 1 : 0; }

// Whether the firmware has started its timers (so the host should run them).
int oc_timers_running(int inst) {
  if (inst < 0 || inst >= kInstances) return 0;
  return (g_inst[inst].core_begun.load() && g_inst[inst].ui_begun.load()) ? 1 : 0;
}

// Runs `core_ticks` of the 16.666 kHz interrupt and `ui_ticks` of the 1 kHz
// one, interleaved the way they'd fall in real time.
void oc_run_isrs(int inst, int core_ticks, int ui_ticks) {
  if (inst < 0 || inst >= kInstances) return;
  Bind bind(inst);
  Instance &m = g_inst[inst];
  if (!m.core_begun.load() || !m.core_isr) return;
  int ui_done = 0;
  for (int i = 0; i < core_ticks; ++i) {
    m.core_isr();
    if (m.ui_isr && m.ui_begun.load()) {
      int due = (int)(((int64_t)(i + 1) * ui_ticks) / (core_ticks > 0 ? core_ticks : 1));
      while (ui_done < due) {
        m.ui_isr();
        ++ui_done;
      }
    }
  }
}

// A pin's level, as the module's wiring would present it. Triggers are active
// low (their interrupts fire on the falling edge), and so are the buttons.
void oc_set_pin(int inst, int pin, int level) {
  if (inst < 0 || inst >= kInstances || pin < 0 || pin >= 64) return;
  Bind bind(inst);
  Instance &m = g_inst[inst];
  uint8_t old = m.pin[pin].exchange(level ? 1 : 0);
  if (old != (level ? 1 : 0) && m.handler[pin]) {
    bool falling = old == 1 && !level;
    bool rising = old == 0 && level;
    int mode = m.handler_mode[pin];
    if ((mode == FALLING && falling) || (mode == RISING && rising) || mode == CHANGE) m.handler[pin]();
  }
}

void oc_set_cv_millivolts(int inst, int channel, int millivolts) {
  if (inst >= 0 && inst < kInstances && channel >= 0 && channel < 4) g_inst[inst].cv_mv[channel].store(millivolts, std::memory_order_relaxed);
}

// The DAC channel's output in millivolts, with the default calibration: the
// firmware's table puts octave i at code i * 6553.5 + 4890 (approximately), and
// octave index 3 is 0 V, so the output spans about -3.7 V to +6.3 V.
int oc_dac_millivolts(int inst, int channel) {
  if (inst < 0 || inst >= kInstances || channel < 0 || channel >= 4) return 0;
  double code = g_inst[inst].dac_code[channel].load(std::memory_order_relaxed);
  return (int)((code - 4890.0) * 1000.0 / 6553.5 - 3.0 * 1000.0);
}

// Copies the latest published frame (1024 bytes) and returns how many frames
// have been published so far.
uint64_t oc_frame(int inst, uint8_t *out) {
  if (inst < 0 || inst >= kInstances) return 0;
  Instance &m = g_inst[inst];
  std::lock_guard<std::mutex> lock(m.front_mutex);
  memcpy(out, m.front, sizeof(m.front));
  return m.frames.load(std::memory_order_acquire);
}

void oc_eeprom_read(int inst, uint8_t *out) {
  if (inst >= 0 && inst < kInstances) memcpy(out, g_inst[inst].eeprom, kEeprom);
}
void oc_eeprom_write(int inst, const uint8_t *in) {
  if (inst >= 0 && inst < kInstances) memcpy(g_inst[inst].eeprom, in, kEeprom);
}

const char *oc_app_name(int inst) {
  switch (inst) {
    case 0: return oc0_app_name();
    case 1: return oc1_app_name();
    case 2: return oc2_app_name();
    case 3: return oc3_app_name();
    default: return "";
  }
}

}  // extern "C"
