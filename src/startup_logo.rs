//! Startup-sequence logos -- pre-thresholded 1bpp bitmaps, same
//! reasoning as `spleen_fonts.rs`: real firmware blits a fixed
//! pre-rendered bitmap, not a rasterized SVG, so the conversion from
//! the source vector artwork (`assets/svg_src/*.svg`, via
//! `assets/svg_src/convert_*.py`) happens once, here, not at runtime.

use crate::display::FrameBuffer;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::Pixel;

/// A single pre-converted 1bpp bitmap logo.
pub struct Logo {
    pub width: i32,
    pub height: i32,
    bytes_per_row: i32,
    bits: &'static [u8],
}

impl Logo {
    /// Draws the logo with its top-left corner at `(x, y)` in
    /// `color`; unlit pixels are left untouched (the caller's own
    /// background shows through), same "transparent everywhere else"
    /// behavior text glyphs already have.
    pub fn draw(&self, fb: &mut FrameBuffer, x: i32, y: i32, color: Rgb565) {
        let mut pixels = Vec::new();
        for row in 0..self.height {
            for col in 0..self.width {
                let byte_i = (row * self.bytes_per_row + col / 8) as usize;
                let bit_i = 7 - (col % 8);
                let lit = self.bits.get(byte_i).map(|b| (b >> bit_i) & 1 != 0).unwrap_or(false);
                if lit {
                    pixels.push(Pixel(Point::new(x + col, y + row), color));
                }
            }
        }
        fb.draw_iter(pixels).ok();
    }

    /// Draws itself centered on the real screen.
    pub fn draw_centered(&self, fb: &mut FrameBuffer, color: Rgb565) {
        let x = (crate::display::WIDTH as i32 - self.width) / 2;
        let y = (crate::display::HEIGHT as i32 - self.height) / 2;
        self.draw(fb, x, y, color);
    }
}

/// The parent brand, shown first in the boot sequence (see os.rs).
pub const SZYK: Logo = Logo {
    width: 400,
    height: 94,
    bytes_per_row: (400 + 7) / 8,
    bits: include_bytes!("../assets/raw/szyk_logo.raw"),
};

/// The product/device name, shown second in the boot sequence.
pub const MX1: Logo = Logo {
    width: 400,
    height: 155,
    bytes_per_row: (400 + 7) / 8,
    bits: include_bytes!("../assets/raw/mx1_logo.raw"),
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display;

    /// Each logo's declared `width`/`height` must actually match its
    /// embedded raw file's real size -- if someone reruns a
    /// `convert_*.py` script at a different target size and forgets
    /// to update the matching `Logo` here, every row would read from
    /// the wrong offset (the classic "stride mismatch" bug: the image
    /// wouldn't just be cropped, it'd visibly skew/tear diagonally),
    /// not just fail to compile.
    #[test]
    fn logo_dimensions_match_their_embedded_raw_files() {
        for (name, logo) in [("SZYK", &SZYK), ("MX1", &MX1)] {
            assert_eq!(
                logo.bits.len(),
                (logo.bytes_per_row * logo.height) as usize,
                "{name}'s width/height don't match its .raw file's actual size -- re-check them against the convert script's last output"
            );
        }
    }

    /// Drawing must actually paint something (not silently no-op),
    /// centered on the real screen size it's meant for, and must not
    /// panic when partially or fully off-screen (dragging/testing at
    /// an odd position shouldn't be able to crash the boot screen).
    #[test]
    fn each_logo_draws_something_when_centered_and_does_not_panic_off_screen() {
        for logo in [&SZYK, &MX1] {
            let mut fb = FrameBuffer::new();
            logo.draw_centered(&mut fb, Rgb565::WHITE);
            let lit = fb.buffer().iter().filter(|&&p| p != 0).count();
            assert!(lit > 500, "expected the centered logo to paint a substantial number of pixels, got {lit}");

            let mut fb2 = FrameBuffer::new();
            logo.draw(&mut fb2, -200, -200, Rgb565::WHITE); // mostly off the top-left
            logo.draw(&mut fb2, display::WIDTH as i32 - 50, display::HEIGHT as i32 - 50, Rgb565::WHITE); // mostly off the bottom-right
        }
    }
}
