// Host stand-in for the Teensy/Arduino core: just the parts the O&C firmware
// touches. Pins are simulated levels the host sets; time comes from the host.
#pragma once
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>
#include <math.h>
#include <algorithm>

#define F_CPU 120000000
// Teensy audio library helpers (extern/dspinst.h) have a portable branch for the
// Cortex-M0+ (KINETISL); take it instead of the M4 assembly.
#define KINETISL
#define FASTRUN
#define PROGMEM
#define INPUT 0
#define OUTPUT 1
#define INPUT_PULLUP 2
#define INPUT_PULLDOWN 3
#define OUTPUT_OPENDRAIN 4
#define HIGH 1
#define LOW 0
#define FALLING 2
#define RISING 3
#define CHANGE 4

using std::min;
using std::max;

typedef uint8_t byte;
typedef bool boolean;

// Cortex-M exclusive-access and barrier intrinsics (util_sync.h): a
// compare-and-swap against the value the paired load saw is the same contract.
static inline void __DMB() { __sync_synchronize(); }
static inline void __CLREX() {}
static thread_local volatile uint32_t *oc_exclusive_address = nullptr;
static thread_local uint32_t oc_exclusive_value = 0;
static inline uint32_t __LDREXW(volatile uint32_t *p) {
  oc_exclusive_address = p;
  oc_exclusive_value = __atomic_load_n(p, __ATOMIC_SEQ_CST);
  return oc_exclusive_value;
}
static inline uint32_t __STREXW(uint32_t v, volatile uint32_t *p) {
  uint32_t expected = oc_exclusive_value;
  bool ok = oc_exclusive_address == p && __atomic_compare_exchange_n(p, &expected, v, false, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
  return ok ? 0 : 1;
}

extern "C" {
void oc_host_pin_mode(uint8_t pin, uint8_t mode);
int oc_host_pin_read(uint8_t pin);
void oc_host_pin_write(uint8_t pin, uint8_t value);
unsigned long oc_host_millis();
void oc_host_attach(uint8_t pin, void (*fn)(), int mode);
void oc_host_timer_begin(void (*fn)(), unsigned long microseconds);
void oc_host_delay(unsigned long ms);
}

static inline void pinMode(uint8_t pin, uint8_t mode) { oc_host_pin_mode(pin, mode); }
static inline int digitalReadFast(uint8_t pin) { return oc_host_pin_read(pin); }
static inline int digitalRead(uint8_t pin) { return oc_host_pin_read(pin); }
static inline void digitalWriteFast(uint8_t pin, uint8_t v) { oc_host_pin_write(pin, v); }
static inline void digitalWrite(uint8_t pin, uint8_t v) { oc_host_pin_write(pin, v); }
static inline unsigned long millis() { return oc_host_millis(); }
static inline unsigned long micros() { return oc_host_millis() * 1000UL; }
static inline void delay(unsigned long ms) { oc_host_delay(ms); }
static inline void delayMicroseconds(unsigned int) {}
static inline void attachInterrupt(uint8_t pin, void (*fn)(), int mode) { oc_host_attach(pin, fn, mode); }
static inline void noInterrupts() {}
static inline void interrupts() {}
#define __disable_irq() do {} while (0)
#define __enable_irq() do {} while (0)

// The timers are driven by the host (from its audio thread, at the rates the
// firmware asks for), so these only tell it which function wants which rate.
class IntervalTimer {
 public:
  bool begin(void (*fn)(), unsigned long microseconds) { oc_host_timer_begin(fn, microseconds); return true; }
  void priority(int) {}
  void end() {}
};

// Arduino's random(): a small deterministic generator is plenty here.
extern "C" uint32_t oc_host_random32();
extern "C" void oc_host_random_seed(uint32_t seed);
static inline long random(long max) { return max <= 0 ? 0 : (long)(oc_host_random32() % (uint32_t)max); }
static inline long random(long min, long max) { return min >= max ? min : min + random(max - min); }
static inline void randomSeed(unsigned long seed) { oc_host_random_seed((uint32_t)seed); }

// Teensy's elapsedMillis: an unsigned that reads as the milliseconds since it was set.
class elapsedMillis {
 public:
  elapsedMillis() { ms_ = millis(); }
  elapsedMillis(unsigned long v) { ms_ = millis() - v; }
  operator unsigned long() const { return millis() - ms_; }
  elapsedMillis &operator=(unsigned long v) { ms_ = millis() - v; return *this; }
  elapsedMillis &operator-=(unsigned long v) { ms_ += v; return *this; }
  elapsedMillis &operator+=(unsigned long v) { ms_ -= v; return *this; }
 private:
  unsigned long ms_;
};

// Serial output goes nowhere.
struct SerialClass {
  void begin(unsigned long) {}
  void print(const char *) {}
  void println(const char * = "") {}
  void printf(const char *, ...) {}
  operator bool() const { return false; }
};
extern SerialClass Serial;

#define NVIC_SET_PRIORITY(a, b) do {} while (0)

// The Cortex-M4 cycle counter, used only for the firmware's own profiling.
extern "C" volatile uint32_t oc_host_cycle_counter;
#define ARM_DWT_CYCCNT oc_host_cycle_counter
#define ARM_DEMCR oc_host_cycle_counter
#define ARM_DEMCR_TRCENA 0
#define ARM_DWT_CTRL oc_host_cycle_counter
#define ARM_DWT_CTRL_CYCCNTENA 0
#define IRQ_PORTB 0

#ifndef constrain
#define constrain(x, lo, hi) ((x) < (lo) ? (lo) : ((x) > (hi) ? (hi) : (x)))
#endif

// The module's USB-MIDI device port. The host has no MIDI transport into the
// firmware yet, so nothing is ever received and what the firmware sends is dropped.
struct UsbMidiStub {
  bool read(int = 0) { return false; }
  uint8_t getType() { return 0; }
  uint8_t getChannel() { return 0; }
  uint8_t getData1() { return 0; }
  uint8_t getData2() { return 0; }
  uint8_t* getSysExArray() { static uint8_t none[8]; return none; }
  void sendNoteOn(uint8_t, uint8_t, uint8_t, uint8_t = 0) {}
  void sendNoteOff(uint8_t, uint8_t, uint8_t, uint8_t = 0) {}
  void sendControlChange(uint8_t, uint8_t, uint8_t, uint8_t = 0) {}
  void sendAfterTouch(uint8_t, uint8_t, uint8_t = 0) {}
  void sendPitchBend(int, uint8_t, uint8_t = 0) {}
  void sendSysEx(uint32_t, const uint8_t*, bool = false, uint8_t = 0) {}
  void send_now() {}
};
static UsbMidiStub usbMIDI;
