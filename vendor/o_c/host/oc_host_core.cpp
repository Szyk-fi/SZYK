// The "hardware" under an O&C-family firmware, on a computer.
//
// Each firmware (stock O&C, Hemisphere Suite, Phazerville...) is built as a
// shared library that includes this file, and the host loads a fresh copy of
// the library for each module, so a module's firmware and everything in here
// -- pin levels, EEPROM, the CV inputs and DAC outputs, the OLED's frame, the
// two timer interrupts -- are that module's alone (the firmware keeps its state
// in globals, so two copies in one image would collide).
//
// The firmware's setup() and loop() run on a thread of their own, blocking
// calls and all; its timer interrupts are run by the host's audio thread at the
// rates it asks for, so its CV and triggers keep sample time. To stop a
// firmware, the host asks its thread to leave: wherever the firmware waits or
// draws it passes a checkpoint, which unwinds the thread.

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

constexpr int kEeprom = 8192;

using Clock = std::chrono::steady_clock;
const Clock::time_point g_epoch = Clock::now();

// What a stopped firmware thread unwinds with (see oc_host_checkpoint).
struct StopFirmware {};

struct Hardware {
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
  std::atomic<bool> stop{false};
  std::thread thread;

  Hardware() {
    for (auto &p : pin) p = 1;
    for (auto &c : cv_mv) c = 0;
    for (auto &d : dac_code) d = 0;
  }
};

// One firmware per image: the host loads each firmware as its own library, a
// fresh copy per module, so these globals are that module's alone.
Hardware g;

// Called wherever the firmware waits or draws: once a stop is asked for, the
// firmware thread leaves through here, however deep in its loop it is.
inline void checkpoint() {
  if (g.stop.load(std::memory_order_relaxed)) throw StopFirmware{};
}

}  // namespace

// This firmware's entry points (the variant's oc_host_fw.cpp).
extern "C" {
void ocfw_main();
const char *ocfw_current_app_name();
}

// ---- What the firmware calls (the calling thread's instance) ------------------

extern "C" {

void oc_host_pin_mode(uint8_t, uint8_t) {}
int oc_host_pin_read(uint8_t pin) { return pin < 64 ? g.pin[pin].load(std::memory_order_relaxed) : 1; }
void oc_host_pin_write(uint8_t, uint8_t) {}
unsigned long oc_host_millis() {
  return (unsigned long)std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now() - g_epoch).count();
}
void oc_host_delay(unsigned long ms) {
  checkpoint();
  std::this_thread::sleep_for(std::chrono::milliseconds(ms));
  checkpoint();
}
void oc_host_attach(uint8_t pin, void (*fn)(), int mode) {
  if (pin < 64) {
    g.handler[pin] = fn;
    g.handler_mode[pin] = mode;
  }
}
void oc_host_timer_begin(void (*fn)(), unsigned long microseconds) {
  // The firmware starts its core timer at 60 us and its UI timer at 1 ms.
  Hardware &m = g;
  if (microseconds <= 100) {
    m.core_isr = fn;
    m.core_begun = true;
  } else {
    m.ui_isr = fn;
    m.ui_begun = true;
  }
}
uint32_t oc_host_random32() {
  Hardware &m = g;
  m.random ^= m.random << 13;
  m.random ^= m.random >> 17;
  m.random ^= m.random << 5;
  return m.random;
}
void oc_host_random_seed(uint32_t seed) { g.random = seed ? seed : 0x2545F491u; }

uint8_t *oc_host_eeprom_ptr() { return g.eeprom; }

int oc_host_cv_millivolts(int channel) { return (channel >= 0 && channel < 4) ? g.cv_mv[channel].load(std::memory_order_relaxed) : 0; }
void oc_host_dac_write(int channel, uint32_t code) {
  if (channel >= 0 && channel < 4) g.dac_code[channel].store(code, std::memory_order_relaxed);
}

// A panel refreshes at a few hundred hertz at most. Screens that draw in a
// tight loop (the splash, calibration) are held to that.
uint8_t *oc_host_begin_frame() {
  checkpoint();
  Hardware &m = g;
  auto since = Clock::now() - m.last_frame;
  if (since < std::chrono::milliseconds(4)) std::this_thread::sleep_for(std::chrono::milliseconds(4) - since);
  m.last_frame = Clock::now();
  return m.back;
}

void oc_host_end_frame() {
  Hardware &m = g;
  std::lock_guard<std::mutex> lock(m.front_mutex);
  memcpy(m.front, m.back, sizeof(m.front));
  m.frames.fetch_add(1, std::memory_order_release);
}

}  // extern "C"

// ---- The host's side ----------------------------------------------------------

extern "C" {

// Starts the firmware: setup(), then loop() forever, on a thread of its own.
void ocfw_start() {
  if (g.started.exchange(true)) return;
  g.thread = std::thread([] {
    try {
      ocfw_main();
    } catch (const StopFirmware &) {
    }
  });
}

// Asks the firmware thread to leave and waits for it. The host stops running
// the interrupts first.
void ocfw_stop() {
  g.stop = true;
  if (g.thread.joinable()) g.thread.join();
}

// Whether the firmware has started its timers (so the host should run them).
int ocfw_timers_running() { return (g.core_begun.load() && g.ui_begun.load()) ? 1 : 0; }

// Runs `core_ticks` of the 16.666 kHz interrupt and `ui_ticks` of the 1 kHz
// one, interleaved the way they'd fall in real time.
void ocfw_run_isrs(int core_ticks, int ui_ticks) {
  if (!g.core_begun.load() || !g.core_isr || g.stop.load()) return;
  int ui_done = 0;
  for (int i = 0; i < core_ticks; ++i) {
    g.core_isr();
    if (g.ui_isr && g.ui_begun.load()) {
      int due = (int)(((int64_t)(i + 1) * ui_ticks) / (core_ticks > 0 ? core_ticks : 1));
      while (ui_done < due) {
        g.ui_isr();
        ++ui_done;
      }
    }
  }
}

// A pin's level, as the module's wiring would present it. Triggers are active
// low (their interrupts fire on the falling edge), and so are the buttons.
void ocfw_set_pin(int pin, int level) {
  if (pin < 0 || pin >= 64) return;
  uint8_t old = g.pin[pin].exchange(level ? 1 : 0);
  if (old != (level ? 1 : 0) && g.handler[pin]) {
    bool falling = old == 1 && !level;
    bool rising = old == 0 && level;
    int mode = g.handler_mode[pin];
    if ((mode == FALLING && falling) || (mode == RISING && rising) || mode == CHANGE) g.handler[pin]();
  }
}

void ocfw_set_cv_millivolts(int channel, int millivolts) {
  if (channel >= 0 && channel < 4) g.cv_mv[channel].store(millivolts, std::memory_order_relaxed);
}

// The DAC channel's output in millivolts, with the default calibration: the
// firmware's table puts octave i at code i * 6553.5 + 4890 (approximately), and
// octave index 3 is 0 V, so the output spans about -3.7 V to +6.3 V.
int ocfw_dac_millivolts(int channel) {
  if (channel < 0 || channel >= 4) return 0;
  double code = g.dac_code[channel].load(std::memory_order_relaxed);
  return (int)((code - 4890.0) * 1000.0 / 6553.5 - 3.0 * 1000.0);
}

// Copies the latest published frame (1024 bytes) and returns how many frames
// have been published so far.
uint64_t ocfw_frame(uint8_t *out) {
  std::lock_guard<std::mutex> lock(g.front_mutex);
  memcpy(out, g.front, sizeof(g.front));
  return g.frames.load(std::memory_order_acquire);
}

void ocfw_eeprom_read(uint8_t *out) { memcpy(out, g.eeprom, kEeprom); }
void ocfw_eeprom_write(const uint8_t *in) { memcpy(g.eeprom, in, kEeprom); }

const char *ocfw_app_name() { return ocfw_current_app_name(); }

}  // extern "C"
