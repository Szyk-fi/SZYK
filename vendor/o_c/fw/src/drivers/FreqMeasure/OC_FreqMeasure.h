// Host stand-in for the Teensy FreqMeasure library, which times pulses on a
// pin with a hardware timer. APP_REFS uses it for its "auto" tuning reference;
// on the host nothing ever measures, so `available()` is always false.
#pragma once
#include <stdint.h>
struct FreqMeasureClass {
  void begin() {}
  void end() {}
  bool available() { return false; }
  uint32_t read() { return 0; }
  float countToFrequency(uint32_t) { return 0.0f; }
};
extern FreqMeasureClass FreqMeasure;
