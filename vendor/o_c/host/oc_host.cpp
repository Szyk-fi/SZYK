// The "hardware" under the O&C firmware, on a computer.
//
// The firmware (../fw) is compiled unchanged except for a handful of files
// listed in ../PATCHES.md. Everything it expects of a Teensy 3.2 and the
// module around it lives here: pin levels, EEPROM, time, the CV inputs and
// DAC outputs, the OLED's frame, the random generator, and the two timer
// interrupts. The firmware's own setup() and loop() run on a thread of their
// own, blocking calls and all; its timer interrupts are run by the host's audio
// thread at the rates it asks for, so its CV and triggers keep sample time.

#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <thread>

#include <Arduino.h>
#include <EEPROM.h>

#include "OC_ADC.h"
#include "OC_DAC.h"
#include "OC_apps.h"
#include "OC_core.h"
#include "OC_gpio.h"
#include "OC_calibration.h"
#include "src/drivers/FreqMeasure/OC_FreqMeasure.h"

extern void setup();
extern void loop();
extern const OC::CalibrationData kCalibrationDefaults;

// ---- Arduino core -----------------------------------------------------------

SerialClass Serial;
FreqMeasureClass FreqMeasure;
EEPROMClass EEPROM;
extern "C" uint8_t oc_host_eeprom[8192] = {0};
extern "C" volatile uint32_t oc_host_cycle_counter = 0;

namespace {

using Clock = std::chrono::steady_clock;
const Clock::time_point g_epoch = Clock::now();

// Pins idle high: the buttons and triggers are pulled up and read low when active.
struct Pins {
  std::atomic<uint8_t> v[64];
  Pins() {
    for (auto &a : v) a = 1;
  }
} g_pins;
void (*g_handler[64])() = {nullptr};
int g_handler_mode[64] = {0};

void (*g_core_isr)() = nullptr;
void (*g_ui_isr)() = nullptr;
std::atomic<bool> g_core_begun{false};
std::atomic<bool> g_ui_begun{false};

uint32_t g_random = 0x2545F491u;

}  // namespace

extern "C" {

void oc_host_pin_mode(uint8_t, uint8_t) {}
int oc_host_pin_read(uint8_t pin) { return pin < 64 ? g_pins.v[pin].load(std::memory_order_relaxed) : 1; }
void oc_host_pin_write(uint8_t, uint8_t) {}
unsigned long oc_host_millis() {
  return (unsigned long)std::chrono::duration_cast<std::chrono::milliseconds>(Clock::now() - g_epoch).count();
}
void oc_host_delay(unsigned long ms) { std::this_thread::sleep_for(std::chrono::milliseconds(ms)); }
void oc_host_attach(uint8_t pin, void (*fn)(), int mode) {
  if (pin < 64) {
    g_handler[pin] = fn;
    g_handler_mode[pin] = mode;
  }
}
void oc_host_timer_begin(void (*fn)(), unsigned long microseconds) {
  // The firmware starts its core timer at 60 us and its UI timer at 1 ms.
  if (microseconds <= 100) {
    g_core_isr = fn;
    g_core_begun = true;
  } else {
    g_ui_isr = fn;
    g_ui_begun = true;
  }
}
uint32_t oc_host_random32() {
  g_random ^= g_random << 13;
  g_random ^= g_random >> 17;
  g_random ^= g_random << 5;
  return g_random;
}
void oc_host_random_seed(uint32_t seed) { g_random = seed ? seed : 0x2545F491u; }

}  // extern "C"

// ---- CV inputs: the ADC -----------------------------------------------------

namespace {
// Volts the host says are patched into each CV input.
std::atomic<int32_t> g_cv_millivolts[ADC_CHANNEL_LAST];
}

OC::ADC::CalibrationData *OC::ADC::calibration_data_ = nullptr;
uint32_t OC::ADC::raw_[ADC_CHANNEL_LAST];
uint32_t OC::ADC::smoothed_[ADC_CHANNEL_LAST];

void OC::ADC::Init(CalibrationData *calibration_data) {
  calibration_data_ = calibration_data;
  for (int i = 0; i < ADC_CHANNEL_LAST; ++i) {
    // 0 V reads as the calibrated offset, so value() starts at zero.
    raw_[i] = smoothed_[i] = (uint32_t)calibration_data->offset[i] << kAdcSmoothBits;
  }
}

void OC::ADC::Scan_DMA() {
  // The module's input range is +-5 V over 12 bits around the offset
  // (full scale 4096 counts = 10 V), inverted by its input stage.
  for (int i = 0; i < ADC_CHANNEL_LAST; ++i) {
    int32_t counts = (int32_t)((int64_t)g_cv_millivolts[i].load(std::memory_order_relaxed) * 4096 / 10000);
    int32_t raw = (int32_t)calibration_data_->offset[i] - counts;
    if (raw < 0) raw = 0;
    if (raw > 4095) raw = 4095;
    update((ADC_CHANNEL)i, (uint32_t)raw << (kAdcScanResolution - kAdcResolution));
  }
}

void OC::ADC::CalibratePitch(int32_t, int32_t) {
  // The ADC calibration screen is not reachable on the host (inputs are exact).
  calibration_data_->pitch_cv_scale = kDefaultPitchCVScale;
}

// ---- CV outputs: the DAC ----------------------------------------------------

namespace {
std::atomic<uint32_t> g_dac_code[DAC_CHANNEL_LAST];
}

extern "C" void oc_host_dac_write(int channel, uint32_t code) { g_dac_code[channel].store(code, std::memory_order_relaxed); }
void set8565_CHA(uint32_t data) { oc_host_dac_write(0, data); }
void set8565_CHB(uint32_t data) { oc_host_dac_write(1, data); }
void set8565_CHC(uint32_t data) { oc_host_dac_write(2, data); }
void set8565_CHD(uint32_t data) { oc_host_dac_write(3, data); }
void SPI_init() {}

OC::DAC::CalibrationData *OC::DAC::calibration_data_ = nullptr;
uint32_t OC::DAC::values_[DAC_CHANNEL_LAST];
uint16_t OC::DAC::history_[DAC_CHANNEL_LAST][OC::DAC::kHistoryDepth];
volatile size_t OC::DAC::history_tail_ = 0;
uint8_t OC::DAC::DAC_scaling[DAC_CHANNEL_LAST];

namespace {
// Per-channel calibration rows chosen by the autotuner (OC_autotune), kept
// beside the defaults so a channel can switch between them.
uint16_t g_default_cal[DAC_CHANNEL_LAST][OCTAVES + 1];
uint16_t g_auto_cal[DAC_CHANNEL_LAST][OCTAVES + 1];
bool g_auto_in_use[DAC_CHANNEL_LAST];
bool g_auto_has_data[DAC_CHANNEL_LAST];
}  // namespace

void OC::DAC::Init(CalibrationData *calibration_data) {
  calibration_data_ = calibration_data;
  for (int c = 0; c < DAC_CHANNEL_LAST; ++c) {
    memcpy(g_default_cal[c], calibration_data->calibrated_octaves[c], sizeof(g_default_cal[c]));
    memcpy(g_auto_cal[c], g_default_cal[c], sizeof(g_auto_cal[c]));
    g_auto_in_use[c] = false;
    g_auto_has_data[c] = false;
    DAC_scaling[c] = VOLTAGE_SCALING_1V_PER_OCT;
    values_[c] = calibration_data->calibrated_octaves[c][kOctaveZero];
  }
  history_tail_ = 0;
  memset(history_, 0, sizeof(history_));
}

uint8_t OC::DAC::calibration_data_used(uint8_t channel_id) {
  return channel_id < DAC_CHANNEL_LAST && g_auto_in_use[channel_id];
}

void OC::DAC::set_auto_channel_calibration_data(uint8_t channel_id) {
  if (channel_id >= DAC_CHANNEL_LAST) return;
  memcpy(calibration_data_->calibrated_octaves[channel_id], g_auto_cal[channel_id], sizeof(g_auto_cal[channel_id]));
  g_auto_in_use[channel_id] = true;
}

void OC::DAC::set_default_channel_calibration_data(uint8_t channel_id) {
  if (channel_id >= DAC_CHANNEL_LAST) return;
  memcpy(calibration_data_->calibrated_octaves[channel_id], g_default_cal[channel_id], sizeof(g_default_cal[channel_id]));
  g_auto_in_use[channel_id] = false;
}

void OC::DAC::update_auto_channel_calibration_data(uint8_t channel_id, int8_t octave, uint32_t pitch_data) {
  if (channel_id >= DAC_CHANNEL_LAST) return;
  int index = kOctaveZero + octave;
  if (index < 0 || index > OCTAVES) return;
  g_auto_cal[channel_id][index] = (uint16_t)USAT16(pitch_data);
  g_auto_has_data[channel_id] = true;
}

void OC::DAC::reset_auto_channel_calibration_data(uint8_t channel_id) {
  if (channel_id >= DAC_CHANNEL_LAST) return;
  memcpy(g_auto_cal[channel_id], g_default_cal[channel_id], sizeof(g_auto_cal[channel_id]));
  g_auto_has_data[channel_id] = false;
  if (g_auto_in_use[channel_id]) set_default_channel_calibration_data(channel_id);
}

void OC::DAC::reset_all_auto_channel_calibration_data() {
  for (uint8_t c = 0; c < DAC_CHANNEL_LAST; ++c) reset_auto_channel_calibration_data(c);
}

void OC::DAC::choose_calibration_data() {
  for (uint8_t c = 0; c < DAC_CHANNEL_LAST; ++c) {
    if (g_auto_has_data[c]) set_auto_channel_calibration_data(c);
    else set_default_channel_calibration_data(c);
  }
}

void OC::DAC::set_scaling(uint8_t scaling, uint8_t channel_id) {
  if (channel_id < DAC_CHANNEL_LAST && scaling < VOLTAGE_SCALING_LAST) DAC_scaling[channel_id] = scaling;
}

void OC::DAC::restore_scaling(uint32_t scaling) {
  for (int c = 0; c < DAC_CHANNEL_LAST; ++c) {
    uint8_t s = (scaling >> (8 * c)) & 0xff;
    DAC_scaling[c] = s < VOLTAGE_SCALING_LAST ? s : VOLTAGE_SCALING_1V_PER_OCT;
  }
}

uint8_t OC::DAC::get_voltage_scaling(uint8_t channel_id) {
  return channel_id < DAC_CHANNEL_LAST ? DAC_scaling[channel_id] : (uint8_t)VOLTAGE_SCALING_1V_PER_OCT;
}

uint32_t OC::DAC::store_scaling() {
  uint32_t packed = 0;
  for (int c = 0; c < DAC_CHANNEL_LAST; ++c) packed |= (uint32_t)DAC_scaling[c] << (8 * c);
  return packed;
}

void OC::DAC::set_Vbias(uint32_t) {}
void OC::DAC::init_Vbias() {}

// ---- Display ----------------------------------------------------------------

weegfx::Graphics graphics;

namespace {
uint8_t g_back[1024];
uint8_t g_front[1024];
std::mutex g_front_mutex;
std::atomic<uint64_t> g_frames{0};
Clock::time_point g_last_frame = Clock::now();
}  // namespace

void display::Init() { graphics.Init(); }
void display::AdjustOffset(uint8_t) {}

uint8_t *display::begin_frame() {
  // A panel refreshes at a few hundred hertz at most. Screens that draw in a
  // tight loop (the splash, calibration) are held to that.
  auto since = Clock::now() - g_last_frame;
  if (since < std::chrono::milliseconds(4)) std::this_thread::sleep_for(std::chrono::milliseconds(4) - since);
  g_last_frame = Clock::now();
  return g_back;
}

void display::end_frame() {
  std::lock_guard<std::mutex> lock(g_front_mutex);
  memcpy(g_front, g_back, sizeof(g_front));
  g_frames.fetch_add(1, std::memory_order_release);
}

// ---- The host's side --------------------------------------------------------

namespace {
std::atomic<bool> g_started{false};
std::atomic<bool> g_running_ok{true};
}  // namespace

extern "C" {

// Starts the firmware: setup(), then loop() forever, on a thread of its own.
void oc_start() {
  if (g_started.exchange(true)) return;
  std::thread([] {
    setup();
    loop();
  }).detach();
}

int oc_started() { return g_started.load() ? 1 : 0; }

// Whether the firmware has started its timers (so the host should run them).
int oc_timers_running() { return (g_core_begun.load() && g_ui_begun.load()) ? 1 : 0; }

// Runs `core_ticks` of the 16.666 kHz interrupt and `ui_ticks` of the 1 kHz
// one, interleaved the way they'd fall in real time.
void oc_run_isrs(int core_ticks, int ui_ticks) {
  if (!g_core_begun.load() || !g_core_isr) return;
  int ui_done = 0;
  for (int i = 0; i < core_ticks; ++i) {
    g_core_isr();
    if (g_ui_isr && g_ui_begun.load()) {
      int due = (int)(((int64_t)(i + 1) * ui_ticks) / (core_ticks > 0 ? core_ticks : 1));
      while (ui_done < due) {
        g_ui_isr();
        ++ui_done;
      }
    }
  }
}

// A pin's level, as the module's wiring would present it. Triggers are active
// low (their interrupts fire on the falling edge), and so are the buttons.
void oc_set_pin(int pin, int level) {
  if (pin < 0 || pin >= 64) return;
  uint8_t old = g_pins.v[pin].exchange(level ? 1 : 0);
  if (old != (level ? 1 : 0) && g_handler[pin]) {
    bool falling = old == 1 && !level;
    bool rising = old == 0 && level;
    int mode = g_handler_mode[pin];
    if ((mode == FALLING && falling) || (mode == RISING && rising) || mode == CHANGE) g_handler[pin]();
  }
}

void oc_set_cv_millivolts(int channel, int millivolts) {
  if (channel >= 0 && channel < ADC_CHANNEL_LAST) g_cv_millivolts[channel].store(millivolts, std::memory_order_relaxed);
}

// The DAC channel's output in millivolts, with the default calibration: the
// firmware's table puts octave i at code i * 6553.5 + 4890 (approximately), and
// octave index 3 is 0 V, so the output spans about -3.7 V to +6.3 V.
int oc_dac_millivolts(int channel) {
  if (channel < 0 || channel >= DAC_CHANNEL_LAST) return 0;
  double code = g_dac_code[channel].load(std::memory_order_relaxed);
  return (int)((code - 4890.0) * 1000.0 / 6553.5 - OC::DAC::kOctaveZero * 1000.0);
}

uint32_t oc_dac_code(int channel) { return (channel >= 0 && channel < DAC_CHANNEL_LAST) ? g_dac_code[channel].load(std::memory_order_relaxed) : 0; }

// Copies the latest published frame (1024 bytes) and returns how many frames
// have been published so far.
uint64_t oc_frame(uint8_t *out) {
  std::lock_guard<std::mutex> lock(g_front_mutex);
  memcpy(out, g_front, sizeof(g_front));
  return g_frames.load(std::memory_order_acquire);
}

void oc_eeprom_read(uint8_t *out) { memcpy(out, oc_host_eeprom, 8192); }
void oc_eeprom_write(const uint8_t *in) { memcpy(oc_host_eeprom, in, 8192); }

const char *oc_app_name() {
  return OC::apps::current_app ? OC::apps::current_app->name : "";
}

}  // extern "C"
