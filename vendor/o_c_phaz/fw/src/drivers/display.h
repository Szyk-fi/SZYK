// Host build of the display interface: the same weegfx::Graphics drawing into
// a 128x64 one-bit page-format frame (the SH1106's layout: eight pages of 128
// bytes, bit 0 the top row of a page), published for the host to show. The
// SPI/DMA transfer to the panel, and its frame ring, are not needed.
// See vendor/o_c/PATCHES.md.
#ifndef DRIVERS_DISPLAY_H_
#define DRIVERS_DISPLAY_H_

#include "weegfx.h"
#include "../../util/util_debugpins.h"

struct SH1106_128x64_Driver {
  static constexpr uint8_t kDefaultOffset = 2;
  static constexpr size_t kFrameSize = 128 * 64 / 8;
};

namespace display {

void Init();
void AdjustOffset(uint8_t offset);
static inline void SetFlipMode(bool) {}
static inline void Flush() {}
static inline void Update() {}

// Frame handoff: begin_frame() returns the frame to draw into (throttling the
// caller to a display-like rate), end_frame() publishes it.
uint8_t *begin_frame();
void end_frame();

};

extern weegfx::Graphics graphics;

#define GRAPHICS_BEGIN_FRAME(wait) \
do { \
  uint8_t *frame = display::begin_frame(); \
  if (frame) { \
    graphics.Begin(frame, weegfx::CLEAR_FRAME_ENABLE); \
    do {} while(0)

#define GRAPHICS_END_FRAME() \
    graphics.End(); \
    display::end_frame(); \
  } \
} while (0)

#endif // DRIVERS_DISPLAY_H_
