#ifndef OC_ADC_H_
#define OC_ADC_H_

// Host build of the O&C ADC interface. The module reads four CV inputs through
// the Teensy's ADC with DMA; here the host writes each input's voltage and
// Scan_DMA() folds it in with the same smoothing, offset and pitch scaling, so
// the apps see numbers shaped exactly as on the module. See vendor/o_c/PATCHES.md.

#include "OC_config.h"
#include "OC_options.h"

#include <stdint.h>
#include <string.h>

enum ADC_CHANNEL {
  ADC_CHANNEL_1,
  ADC_CHANNEL_2,
  ADC_CHANNEL_3,
  ADC_CHANNEL_4,
  ADC_CHANNEL_LAST,
};

namespace OC {

class ADC {
public:
  static constexpr uint8_t kAdcResolution = 12;
  static constexpr uint32_t kAdcSmoothing = 4;
  static constexpr uint32_t kAdcSmoothBits = 8; // fractional bits for smoothing
  static constexpr uint16_t kDefaultPitchCVScale = SEMITONES << 7;
  static constexpr uint8_t kAdcScanResolution = 16;
  static constexpr uint32_t kAdcValueShift = kAdcSmoothBits;

  struct CalibrationData {
    uint16_t offset[ADC_CHANNEL_LAST];
    uint16_t pitch_cv_scale;
    int16_t pitch_cv_offset;
  };

  static void Init(CalibrationData *calibration_data);
  static void Init_DMA() {}
  static void DMA_ISR() {}
  static void Scan_DMA();

  template <ADC_CHANNEL channel>
  static int32_t value() {
    return calibration_data_->offset[channel] - (smoothed_[channel] >> kAdcValueShift);
  }

  static int32_t value(ADC_CHANNEL channel) {
    return calibration_data_->offset[channel] - (smoothed_[channel] >> kAdcValueShift);
  }

  static uint32_t raw_value(ADC_CHANNEL channel) {
    return raw_[channel] >> kAdcValueShift;
  }

  static uint32_t smoothed_raw_value(ADC_CHANNEL channel) {
    return smoothed_[channel] >> kAdcValueShift;
  }

  static int32_t pitch_value(ADC_CHANNEL channel) {
    return (value(channel) * calibration_data_->pitch_cv_scale) >> 12;
  }

  static int32_t raw_pitch_value(ADC_CHANNEL channel) {
    int32_t value = calibration_data_->offset[channel] - raw_value(channel);
    return (value * calibration_data_->pitch_cv_scale) >> 12;
  }

  static void CalibratePitch(int32_t c2, int32_t c4);

private:
  static void update(ADC_CHANNEL channel, uint32_t value) {
    value = (value >> (kAdcScanResolution - kAdcResolution)) << kAdcSmoothBits;
    raw_[channel] = value;
    value = (smoothed_[channel] * (kAdcSmoothing - 1) + value) / kAdcSmoothing;
    smoothed_[channel] = value;
  }

  static CalibrationData *calibration_data_;
  static uint32_t raw_[ADC_CHANNEL_LAST];
  static uint32_t smoothed_[ADC_CHANNEL_LAST];
};

};

#endif // OC_ADC_H_
