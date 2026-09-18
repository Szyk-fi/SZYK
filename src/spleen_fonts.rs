//! Custom bitmap fonts, replacing `embedded_graphics::mono_font::ascii`'s
//! generic built-ins with [Spleen](https://github.com/fcambus/spleen), a
//! hand-designed monospace bitmap font family (BSD-2-Clause, Copyright
//! (c) 2018-2026 Frederic Cambus -- see `fonts/LICENSE-spleen`).
//!
//! This is a deliberate choice over rendering a real vector font (e.g.
//! rasterizing a TTF at runtime with a crate like `ab_glyph`): the real
//! target hardware (an STM32N6) will draw this UI the exact same way
//! this sim does -- blitting a fixed pre-rendered glyph bitmap per
//! character, no font rasterizer in the loop. A runtime-rasterized
//! vector font would look better on this desktop build but wouldn't
//! reflect what the real firmware can actually do; a hand-designed
//! bitmap font (rather than embedded-graphics's plain, generic ascii
//! set) is the version of "looks nice" available within that
//! constraint.
//!
//! `fonts/spleen_src/convert.py` did the one-time BDF -> raw-bitmap
//! conversion (see its own comments for the exact packing this format
//! expects); `fonts/raw/*.raw` are its output, embedded here via
//! `include_bytes!`. Re-run it (pointing at a different Spleen `.bdf`
//! size from https://github.com/fcambus/spleen) to add another size.

use embedded_graphics::geometry::Size;
use embedded_graphics::image::ImageRaw;
use embedded_graphics::mono_font::mapping::StrGlyphMapping;
use embedded_graphics::mono_font::{DecorationDimensions, MonoFont};

/// Every size shares this mapping: glyph index 0 = U+0020 (space)
/// through index 94 = U+007E (`~`), the full printable-ASCII range --
/// matches exactly the 95 glyphs `convert.py` packs into each raw
/// file, one row wide. Anything outside that range (a stray non-ASCII
/// character) falls back to glyph 0 (space) rather than panicking or
/// drawing garbage.
static MAPPING: StrGlyphMapping = StrGlyphMapping::new("\0\u{20}\u{7e}", 0);

/// Replaces `FONT_6X10`/`FONT_6X13` -- dim labels, hints, captions.
pub const SPLEEN_6X12: MonoFont = MonoFont {
    image: ImageRaw::new(include_bytes!("../fonts/raw/spleen_6x12.raw"), 570),
    glyph_mapping: &MAPPING,
    character_size: Size::new(6, 12),
    character_spacing: 0,
    baseline: 8, // FONT_ASCENT (9) - 1, same convention the built-in fonts use
    underline: DecorationDimensions::new(8 + 2, 1),
    strikethrough: DecorationDimensions::new(12 / 2, 1),
};

/// Replaces `FONT_9X15`/`FONT_9X18` -- the shared `ParamList` menu
/// every app uses, and the launcher's app list.
pub const SPLEEN_8X16: MonoFont = MonoFont {
    image: ImageRaw::new(include_bytes!("../fonts/raw/spleen_8x16.raw"), 760),
    glyph_mapping: &MAPPING,
    character_size: Size::new(8, 16),
    character_spacing: 0,
    baseline: 11, // FONT_ASCENT (12) - 1
    underline: DecorationDimensions::new(11 + 2, 1),
    strikethrough: DecorationDimensions::new(16 / 2, 1),
};

/// Replaces `FONT_10X20` -- each app's title. (Spleen's 12x24 was the
/// first size tried here and fit better proportionally, but 16x32 --
/// tried on Voltage, then swept everywhere once it held up -- reads
/// more like an actual wordmark against the smaller menu text; the
/// 12x24 raw file/BDF are still under `fonts/` if this needs to come
/// back down a size.)
pub const SPLEEN_16X32: MonoFont = MonoFont {
    image: ImageRaw::new(include_bytes!("../fonts/raw/spleen_16x32.raw"), 1520),
    glyph_mapping: &MAPPING,
    character_size: Size::new(16, 32),
    character_spacing: 0,
    baseline: 25, // FONT_ASCENT (26) - 1
    underline: DecorationDimensions::new(25 + 2, 1),
    strikethrough: DecorationDimensions::new(32 / 2, 1),
};
