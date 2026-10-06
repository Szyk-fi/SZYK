#!/usr/bin/env python3
"""The host-compatibility edits every O&C-family firmware needs, applied to a fresh
copy of its source (see vendor/o_c*/PATCHES.md). Idempotent; reports what it did
and what it could not find.

    tools/oc_port_patches.py vendor/o_c_hemi/fw
"""
import re, sys, os

root = sys.argv[1]
done = []
missing = []

def path(p):
    return os.path.join(root, p)

def edit(p, fn, what):
    f = path(p)
    if not os.path.exists(f):
        missing.append(f'{p}: absent ({what})')
        return
    s = open(f).read()
    t = fn(s)
    if t is None:
        missing.append(f'{p}: pattern not found ({what})')
    elif t != s:
        open(f, 'w').write(t)
        done.append(f'{p}: {what}')

def sub(old, new, count=1):
    def fn(s):
        return s.replace(old, new, count) if old in s else (s if new in s else None)
    return fn

# clang won't initialise a member from a template it shadows.
for f in ['OC_calibration.h', 'OC_apps.ino', 'OC_apps.cpp', 'APP_SETTINGS.h']:
    if os.path.exists(path(f)):
        edit(f, lambda s: re.sub(r'(?<![:\w])FOURCC<', 'FourCC<', s), 'FOURCC -> FourCC alias')
for f in ['util/util_misc.h']:
    edit(f, lambda s: s if 'using FourCC' in s else s.replace('template <uint32_t a, uint32_t b>\nstruct TWOCC', "template <uint32_t a, uint32_t b, uint32_t c, uint32_t d>\nusing FourCC = FOURCC<a, b, c, d>;\n\ntemplate <uint32_t a, uint32_t b>\nstruct TWOCC", 1) if 'struct TWOCC' in s else None, 'FourCC alias')

# pow() isn't constexpr in clang.
for f in ['OC_calibration.ino', 'OC_calibration.cpp']:
    if os.path.exists(path(f)):
        edit(f, lambda s: s.replace('(uint16_t)((float)pow(2,OC::ADC::kAdcResolution)*0.6666667f)', '(uint16_t)(4096.0f * 0.6666667f)').replace('(uint16_t)((float)pow(2,OC::ADC::kAdcResolution)*1.0f)', '(uint16_t)(4096.0f * 1.0f)'), '_ADC_OFFSET literal')

# The Kinetis port-register pinMode.
def drop_pinmode(s):
    a = s.find('/* local copy of pinMode')
    b = s.find('#endif', a)
    if a < 0:
        return s
    return s[:a] + '// (The original defined a faster-slew pinMode here from the Kinetis port registers;\n// the host gets pinMode from host/Arduino.h.)\n\n' + s[b:]
if os.path.exists(path('OC_gpio.h')):
    edit('OC_gpio.h', drop_pinmode, 'dropped the Kinetis pinMode')

# EEPROM: bigger on the host (64-bit structs).
edit('OC_config.h', lambda s: re.sub(r'#define EEPROM_GLOBALSETTINGS_END\s+\d+', '#define EEPROM_GLOBALSETTINGS_END 2048', s) if 'EEPROM_GLOBALSETTINGS_END' in s else None, 'global settings region 2 KB')
edit('util/EEPROMStorage.h', lambda s: s.replace('static const size_t LENGTH = 2048;', 'static const size_t LENGTH = 8192;') if 'LENGTH' in s else None, 'EEPROM 8 KB')

# Cortex-M assembly in util_math.h.
def math_h(s):
    a = s.find('inline uint32_t USAT16(uint32_t value) __attribute__((always_inline));')
    b = s.find('template <typename T, T smoothing>')
    if a < 0 or b < 0:
        return s if 'plain C' in s else None
    return s[:a] + '''// Host: the Cortex-M4 USAT/UMULL instructions as plain C.
inline uint32_t USAT16(uint32_t value) __attribute__((always_inline));
inline uint32_t USAT16(uint32_t value) {
  return value > 65535u ? 65535u : value;
}

inline uint32_t USAT16(int32_t value) __attribute__((always_inline));
inline uint32_t USAT16(int32_t value) {
  return value < 0 ? 0u : (value > 65535 ? 65535u : (uint32_t)value);
}

static inline uint32_t multiply_u32xu32_rshift24(uint32_t a, uint32_t b) __attribute__((always_inline));
static inline uint32_t multiply_u32xu32_rshift24(uint32_t a, uint32_t b)
{
  return (uint32_t)(((uint64_t)a * b) >> 24);
}

static inline uint32_t multiply_u32xu32_rshift(uint32_t a, uint32_t b, uint32_t shift) __attribute__((always_inline));
static inline uint32_t multiply_u32xu32_rshift(uint32_t a, uint32_t b, uint32_t shift)
{
  return (uint32_t)(((uint64_t)a * b) >> shift);
}

''' + s[b:]
edit('util/util_math.h', math_h, 'asm -> plain C')

# size_t is 64 bits here.
edit('src/drivers/weegfx.cpp', lambda s: s.replace('void Graphics::print(uint32_t value, size_t width) {', 'void Graphics::print(uint32_t value, unsigned width) {') if 'size_t width' in s or 'unsigned width) {' in s else None, 'print width type')

print('\n'.join('patched  ' + d for d in done))
print('\n'.join('MISSING  ' + m for m in missing))
