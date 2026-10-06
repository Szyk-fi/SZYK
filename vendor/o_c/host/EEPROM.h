// Host EEPROM: a 2 KB array the host can save and restore (Teensy 3.2 has 2 KB).
#pragma once
#include <stdint.h>
#include <stddef.h>

// Each firmware instance has its own EEPROM; the current one is the calling thread's.
extern "C" uint8_t *oc_host_eeprom_ptr();
#define oc_host_eeprom (oc_host_eeprom_ptr())

struct EERef {
  EERef(size_t i) : index(i) {}
  operator uint8_t() const { return oc_host_eeprom[index]; }
  EERef& operator=(uint8_t v) { oc_host_eeprom[index] = v; return *this; }
  EERef& update(uint8_t v) { if (oc_host_eeprom[index] != v) oc_host_eeprom[index] = v; return *this; }
  size_t index;
};

struct EEPtr {
  EEPtr(size_t i) : index(i) {}
  operator size_t() const { return index; }
  EEPtr& operator++() { ++index; return *this; }
  EEPtr operator++(int) { return EEPtr(index++); }
  EERef operator*() const { return EERef(index); }
  size_t index;
};

struct EEPROMClass {
  EERef operator[](size_t i) { return EERef(i); }
  uint8_t read(int i) { return oc_host_eeprom[i]; }
  void write(int i, uint8_t v) { oc_host_eeprom[i] = v; }
  void update(int i, uint8_t v) { oc_host_eeprom[i] = v; }
  size_t length() { return 8192; }
};
extern EEPROMClass EEPROM;
