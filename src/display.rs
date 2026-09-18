//! Simulated framebuffer standing in for the target panel: AMS317PN01,
//! 360x640 native portrait, rotated 90° by the firmware driver for
//! landscape use. The sim just runs at the rotated 640x360 logical
//! resolution directly — physical rotation is a driver-level concern, not
//! something the desktop architecture validation needs to reproduce.
//!
//! Pixel format is a placeholder (Rgb565, the common choice for small
//! SPI/QSPI panels this size) — the AMS317PN01's driver IC wasn't
//! confirmed when this was picked, so revisit once it is.

use embedded_graphics::pixelcolor::{Rgb565, RgbColor};
use embedded_graphics::prelude::*;
use embedded_graphics::Pixel;
use minifb::{Window, WindowOptions};

pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 360;

pub struct FrameBuffer {
    pixels: Vec<u32>,
}

impl FrameBuffer {
    pub fn new() -> Self {
        Self {
            pixels: vec![0; WIDTH * HEIGHT],
        }
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
        "Portamax OS (sim) \u{2014} AMS317PN01 640x360 landscape",
        WIDTH,
        HEIGHT,
        WindowOptions::default(),
    )
    .expect("failed to open simulator window");
    // Without this, update_with_buffer spins as fast as the OS will let it —
    // pins a core for no reason since this is just a UI, not the audio path.
    window.set_target_fps(60);
    window
}
