#ifndef OC_GPIO_H_
#define OC_GPIO_H_

#include "OC_options.h"

// These pins can be remapped for flip mode.
extern uint8_t TR1, TR2, TR3, TR4;
extern uint8_t encL1, encL2, butL, encR1, encR2, butR;
extern uint8_t but_top, but_bot, but_mid, but_top2, but_bot2;

static constexpr uint8_t OLED_DC = 6, OLED_RST = 7, OLED_CS = 8;
static constexpr uint8_t DAC_CS = 10, DAC_RST = 9;
static constexpr uint8_t OC_GPIO_DEBUG_PIN1 = 24, OC_GPIO_DEBUG_PIN2 = 25;
static constexpr bool DAC_20Vpp = false;
static constexpr bool DAC_is_inverted = false;

#ifdef NORTHERNLIGHT
static constexpr bool NorthernLightModular = true;
#else
static constexpr bool NorthernLightModular = false;
#endif

// OLED CS is active low
#define OLED_CS_ACTIVE LOW
#define OLED_CS_INACTIVE HIGH

#define OC_GPIO_BUTTON_PINMODE INPUT_PULLUP
#define OC_GPIO_TRx_PINMODE INPUT_PULLUP
#define OC_GPIO_ENC_PINMODE INPUT_PULLUP

// (The original defined a faster-slew pinMode here from the Kinetis port registers;
// the host gets pinMode from host/Arduino.h.)

namespace OC {
  void SetFlipMode(bool flip_180);
}

#endif // OC_GPIO_H_
