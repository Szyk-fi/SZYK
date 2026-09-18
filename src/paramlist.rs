//! A small, reusable scrollable settings list: one control moves the
//! selection, another edits the selected item's value. Built for
//! Plaits once its parameter count grew past what fits as individual
//! on-screen fields; any app with more than a handful of adjustable
//! values can reuse this instead of hand-rolling navigation.

use crate::display::FrameBuffer;
use crate::spleen_fonts::SPLEEN_8X16;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{CornerRadii, PrimitiveStyle, Rectangle, RoundedRectangle};
use embedded_graphics::text::Text;

/// This list's one shared accent -- every app's menu goes through
/// here, so this is the single place to change to shift the whole
/// device's palette at once, rather than each app tuning its own
/// green. Picked to match the UI mockup worked out separately.
pub const ACCENT: Rgb565 = Rgb565::new(11, 59, 15);
/// The selected row's highlight chip -- a dark, muted tint of
/// `ACCENT` rather than true alpha blending (Rgb565 has no alpha to
/// blend with), approximating the mockup's translucent highlight
/// over a near-black screen. Shared with the launcher's own selected-
/// row highlight (see `os.rs`'s `draw_launcher`) so the very first
/// screen someone sees matches every app's menu instead of using its
/// own leftover placeholder style.
pub const SELECTED_CHIP_BG: Rgb565 = Rgb565::new(3, 11, 3);

pub struct ParamList {
    pub selected: usize,
    scroll_top: usize,
    /// Accumulates raw ticks between actual moves -- see `navigate`.
    accum: i32,
}

impl ParamList {
    pub fn new() -> Self {
        Self {
            selected: 0,
            scroll_top: 0,
            accum: 0,
        }
    }

    /// Moves the selection by one row once `ticks_per_step` raw ticks
    /// have accumulated in the same direction -- deliberately a discrete
    /// step per some number of detents, not a scaled analog sweep (a
    /// magnitude-scaled version of this is what caused Engine/Chord Type
    /// to "fly" wildly from a single burst of MIDI ticks). `ticks_per_step`
    /// is the adjustable "list nav speed": 1 = a step per tick (fastest),
    /// higher = more deliberate/slower.
    pub fn navigate(&mut self, delta: i32, len: usize, ticks_per_step: i32) {
        if len == 0 || delta == 0 {
            return;
        }
        let ticks_per_step = ticks_per_step.max(1);
        // A direction reversal shouldn't require unwinding a stale
        // accumulation from the other direction first.
        if (delta > 0 && self.accum < 0) || (delta < 0 && self.accum > 0) {
            self.accum = 0;
        }
        self.accum += delta.signum();
        if self.accum.abs() >= ticks_per_step {
            if self.accum > 0 {
                self.selected = (self.selected + 1) % len;
            } else {
                self.selected = (self.selected + len - 1) % len;
            }
            self.accum = 0;
        }
    }

    /// Draws `rows` (name, value) starting at (x, y), `row_h` apart,
    /// showing at most `visible` at once and scrolling to keep the
    /// selection on screen.
    pub fn draw(
        &mut self,
        fb: &mut FrameBuffer,
        x: i32,
        y: i32,
        row_h: i32,
        visible: usize,
        rows: &[(String, String)],
    ) {
        if rows.is_empty() {
            return;
        }
        if self.selected >= rows.len() {
            self.selected = rows.len() - 1;
        }
        if self.selected < self.scroll_top {
            self.scroll_top = self.selected;
        } else if self.selected >= self.scroll_top + visible {
            self.scroll_top = self.selected + 1 - visible;
        }

        let accent = MonoTextStyle::new(&SPLEEN_8X16, ACCENT);
        let dim = MonoTextStyle::new(&SPLEEN_8X16, Rgb565::new(18, 36, 18));
        let end = (self.scroll_top + visible).min(rows.len());

        for (row_i, idx) in (self.scroll_top..end).enumerate() {
            let (name, value) = &rows[idx];
            let yy = y + row_i as i32 * row_h;
            let marker = if idx == self.selected { ">" } else { " " };
            let text = format!("{marker} {name}: {value}");

            if idx == self.selected {
                // A snug highlight chip behind just this row's text
                // (sized from the font's own fixed advance, not a
                // fixed guess), rather than only recoloring the text
                // -- matches the selected-row treatment in the UI
                // mockup this palette was pulled from.
                let text_w = text.chars().count() as u32 * SPLEEN_8X16.character_size.width;
                let chip = Rectangle::new(Point::new(x - 3, yy - 13), Size::new(text_w + 6, 19));
                RoundedRectangle::new(chip, CornerRadii::new(Size::new(3, 3)))
                    .into_styled(PrimitiveStyle::with_fill(SELECTED_CHIP_BG))
                    .draw(fb)
                    .ok();
            }

            let style = if idx == self.selected { accent } else { dim };
            Text::new(&text, Point::new(x, yy), style).draw(fb).ok();
        }

        if self.scroll_top > 0 {
            Text::new("^ more", Point::new(x, y - row_h), dim).draw(fb).ok();
        }
        if end < rows.len() {
            let yy = y + visible as i32 * row_h;
            Text::new("v more", Point::new(x, yy), dim).draw(fb).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The selected row's highlight chip must actually draw something
    /// (not silently no-op) and must not panic for the edge cases most
    /// likely to break a "size the chip from the text" calculation: an
    /// empty-ish row, the very first row (chip_y going slightly
    /// negative), and a very long row.
    #[test]
    fn selected_row_chip_draws_without_panicking() {
        let mut list = ParamList::new();
        let mut fb = FrameBuffer::new();
        let rows = vec![
            ("A".to_string(), String::new()),
            ("A Reasonably Long Parameter Name".to_string(), "a fairly long value string too".to_string()),
        ];
        list.draw(&mut fb, 16, 44, 20, 2, &rows); // selected defaults to row 0
        let lit = fb.buffer().iter().filter(|&&p| p != 0).count();
        assert!(lit > 0, "expected the chip + row text to draw something, got a blank frame");

        list.selected = 1;
        let mut fb2 = FrameBuffer::new();
        list.draw(&mut fb2, 16, 44, 20, 2, &rows); // must not panic on the long row
    }

    fn lit_y_bounds(fb: &FrameBuffer) -> (i32, i32) {
        let mut min_y = i32::MAX;
        let mut max_y = i32::MIN;
        for (i, &p) in fb.buffer().iter().enumerate() {
            if p != 0 {
                let py = (i / crate::display::WIDTH) as i32;
                min_y = min_y.min(py);
                max_y = max_y.max(py);
            }
        }
        (min_y, max_y)
    }

    /// At `SPLEEN_8X16` (the "slightly larger" text every app's menu
    /// now uses), a row's text must not grow tall enough to spill
    /// into the row below it at the row height every app switched to
    /// (24px -- see the `list.draw(fb, 16, 44, 24, 10, ...)` call
    /// sites this font bump required across the whole app).
    #[test]
    fn taller_font_rows_do_not_vertically_collide_at_the_common_row_height() {
        let style = MonoTextStyle::new(&SPLEEN_8X16, Rgb565::new(18, 36, 18));
        let row_h = 24;
        let y0 = 44;

        let mut fb0 = FrameBuffer::new();
        Text::new("> Long Parameter Name: a fairly long value", Point::new(16, y0), style).draw(&mut fb0).ok();
        let (_, row0_bottom) = lit_y_bounds(&fb0);

        let mut fb1 = FrameBuffer::new();
        Text::new("    Another Row: 12.34", Point::new(16, y0 + row_h), style).draw(&mut fb1).ok();
        let (row1_top, _) = lit_y_bounds(&fb1);

        assert!(row0_bottom < row1_top, "row 0's text (bottom={row0_bottom}) would overlap row 1's text (top={row1_top}) at row_h={row_h}");
    }

    /// Settings uses a tighter row height (19px, for its longer,
    /// 12-visible-row list) than the 24px every other app's menu
    /// uses -- must not collide either.
    #[test]
    fn taller_font_rows_do_not_collide_at_settings_tighter_row_height() {
        let style = MonoTextStyle::new(&SPLEEN_8X16, Rgb565::new(18, 36, 18));
        let row_h = 19;
        let y0 = 55;

        let mut fb0 = FrameBuffer::new();
        Text::new("> Long Parameter Name: a fairly long value", Point::new(20, y0), style).draw(&mut fb0).ok();
        let (_, row0_bottom) = lit_y_bounds(&fb0);

        let mut fb1 = FrameBuffer::new();
        Text::new("    Another Row: 12.34", Point::new(20, y0 + row_h), style).draw(&mut fb1).ok();
        let (row1_top, _) = lit_y_bounds(&fb1);

        assert!(row0_bottom < row1_top, "row 0's text (bottom={row0_bottom}) would overlap row 1's text (top={row1_top}) at row_h={row_h}");
    }
}
