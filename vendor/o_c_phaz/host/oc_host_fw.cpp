// The part of the host layer that has to live inside each instance's firmware
// namespace, because it defines members of the firmware's own classes
// (OC::ADC, OC::DAC) and reads its objects. Compiled once per instance (the
// build wraps this file in `namespace ocN { ... }`); everything stateful goes
// through the instance-indexed functions in oc_host_core.cpp.

#include <cstdint>
#include <cstring>

#include <Arduino.h>
#include <EEPROM.h>

#include "OC_ADC.h"
#include "OC_DAC.h"
#include "OC_apps.h"
#include "OC_core.h"
#include "OC_gpio.h"
#include "OC_calibration.h"
#include "src/drivers/FreqMeasure/OC_FreqMeasure.h"
#include "OC_digital_inputs.h"

extern void setup();
extern void loop();

extern "C" {
int oc_host_cv_millivolts(int channel);
void oc_host_dac_write(int channel, uint32_t code);
uint8_t *oc_host_begin_frame();
void oc_host_end_frame();
}

// ---- Stateless Arduino-core object, one per instance (the EEPROM and Serial are shared) --

FreqMeasureClass FreqMeasure;
ADC_CHANNEL ADC_CHANNEL_1 = 0, ADC_CHANNEL_2 = 1, ADC_CHANNEL_3 = 2, ADC_CHANNEL_4 = 3;
DAC_CHANNEL DAC_CHANNEL_A = 0, DAC_CHANNEL_B = 1, DAC_CHANNEL_C = 2, DAC_CHANNEL_D = 3;

// ---- CV inputs: the ADC -----------------------------------------------------

OC::ADC::CalibrationData *OC::ADC::calibration_data_ = nullptr;
uint32_t OC::ADC::raw_[ADC_CHANNEL_LAST];
uint32_t OC::ADC::smoothed_[ADC_CHANNEL_LAST];

void OC::ADC::Init(CalibrationData *calibration_data, bool) {
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
    int32_t counts = (int32_t)((int64_t)oc_host_cv_millivolts(i) * 4096 / 10000);
    int32_t raw = (int32_t)calibration_data_->offset[i] - counts;
    if (raw < 0) raw = 0;
    if (raw > 4095) raw = 4095;
    update_raw((ADC_CHANNEL)i, (uint32_t)raw << (kAdcScanResolution - kAdcResolution));
  }
}

void OC::ADC::CalibratePitch(int32_t, int32_t) {
  // The ADC calibration screen is not reachable on the host (inputs are exact).
  calibration_data_->pitch_cv_scale = kDefaultPitchCVScale;
}

// ---- CV outputs: the DAC ----------------------------------------------------

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

void OC::DAC::Init(CalibrationData *calibration_data, bool) {
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
    uint8_t s = (scaling >> (c * 32 / DAC_CHANNEL_COUNT)) & 0x0f;
    DAC_scaling[c] = s < VOLTAGE_SCALING_LAST ? s : VOLTAGE_SCALING_1V_PER_OCT;
  }
}

uint8_t OC::DAC::get_voltage_scaling(uint8_t channel_id) {
  return channel_id < DAC_CHANNEL_LAST ? DAC_scaling[channel_id] : (uint8_t)VOLTAGE_SCALING_1V_PER_OCT;
}

uint32_t OC::DAC::store_scaling() {
  uint32_t packed = 0;
  for (int c = 0; c < DAC_CHANNEL_LAST; ++c) packed |= (uint32_t)DAC_scaling[c] << (c * 32 / DAC_CHANNEL_COUNT);
  return packed;
}

void OC::ADC::Init_DMA() {}

// The settings app's "flash upgrade" reboots into the Teensy loader; nothing to do here.
extern "C" void _reboot_Teensyduino_() {}

void OC::DAC::set_Vbias(uint32_t) {}
void OC::DAC::init_Vbias() {}

// ---- Display ----------------------------------------------------------------

weegfx::Graphics graphics;

void display::Init() {}
void display::AdjustOffset(uint8_t) {}
uint8_t *display::begin_frame() { return oc_host_begin_frame(); }
void display::end_frame() { oc_host_end_frame(); }

// ---- This firmware's entry points for the host -----------------------------

extern "C" {
void ocfw_main() {
  setup();
  loop();
}
const char *ocfw_current_app_name() { return OC::apps::current_app ? OC::apps::current_app->name : ""; }
}
