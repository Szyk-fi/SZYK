//! The proposed landscape panel is 800x480. Existing app raster artwork uses
//! a 640x360 logical canvas, fitted uniformly into 800x450 with 15px bars.
//! Keeping that coordinate contract preserves games, instruments and embedded
//! artwork without distorting circles, changing hit tests or cropping controls.
//! Slint panels lay out at native panel size independently of this raster path.

use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::Pixel;
use minifb::{Window, WindowOptions};

pub const PANEL_WIDTH: usize = 800;
pub const PANEL_HEIGHT: usize = 480;
/// Logical raster canvas; do not confuse with the physical panel dimensions.
pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 360;

pub struct FrameBuffer {
    pixels: Vec<u32>,
    panel: Vec<u32>,
}

impl FrameBuffer {
    pub fn new() -> Self {
        Self {
            pixels: vec![0; WIDTH * HEIGHT],
            panel: vec![0; PANEL_WIDTH * PANEL_HEIGHT],
        }
    }

    /// Uniform nearest-neighbour fit, with a persistent output allocation.
    pub fn panel_buffer(&mut self) -> &[u32] {
        const FIT_HEIGHT: usize = HEIGHT * PANEL_WIDTH / WIDTH;
        const TOP: usize = (PANEL_HEIGHT - FIT_HEIGHT) / 2;
        self.panel.fill(0);
        for y in 0..FIT_HEIGHT {
            for x in 0..PANEL_WIDTH {
                self.panel[(y + TOP) * PANEL_WIDTH + x] =
                    self.pixels[(y * WIDTH / PANEL_WIDTH) * WIDTH + x * WIDTH / PANEL_WIDTH];
            }
        }
        &self.panel
    }

    pub fn buffer(&self) -> &[u32] {
        &self.pixels
    }
}

impl OriginDimensions for FrameBuffer {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl DrawTarget for FrameBuffer {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x < 0 || point.y < 0 {
                continue;
            }
            let (x, y) = (point.x as usize, point.y as usize);
            if x >= WIDTH || y >= HEIGHT {
                continue;
            }
            // Widen Rgb565's 5/6/5 bits into minifb's 0x00RRGGBB buffer.
            let r = (color.r() as u32) << 3;
            let g = (color.g() as u32) << 2;
            let b = (color.b() as u32) << 3;
            self.pixels[y * WIDTH + x] = (r << 16) | (g << 8) | b;
        }
        Ok(())
    }
}

pub fn open_window() -> Window {
    let mut window = Window::new(
        "ROVE OS (sim) \u{2014} 800x480 landscape",
        PANEL_WIDTH,
        PANEL_HEIGHT,
        WindowOptions { resize: true, scale_mode: minifb::ScaleMode::AspectRatioStretch, ..WindowOptions::default() },
    )
    .expect("failed to open simulator window");
    // Without this, update_with_buffer spins as fast as the OS will let it —
    // pins a core for no reason since this is just a UI, not the audio path.
    window.set_target_fps(60);
    window
}

#[cfg(test)]
mod panel_tests {
    use super::*;
    #[test]
    fn panel_preserves_raster_proportions_edges_and_clears_bars() {
        let mut fb = FrameBuffer::new();
        fb.pixels.fill(0x123456);
        fb.pixels[0] = 0xff0000;
        fb.pixels[WIDTH * HEIGHT - 1] = 0x00ff00;
        let panel = fb.panel_buffer();
        assert_eq!(panel.len(), 800 * 480);
        assert!(panel[..15 * 800].iter().all(|p| *p == 0));
        assert!(panel[465 * 800..].iter().all(|p| *p == 0));
        assert_eq!(panel[15 * 800], 0xff0000);
        assert_eq!(panel[465 * 800 - 1], 0x00ff00);
        assert_eq!(panel[240 * 800 + 400], 0x123456);
    }
}
