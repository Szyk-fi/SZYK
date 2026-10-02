//! The norns screen: 128x64 pixels, 16 brightness levels, drawn with a
//! small path API (move/line/rect/circle/arc, then stroke or fill) and
//! text. Scripts draw into a back buffer; `update()` publishes it.
//!
//! Own implementation of the documented `screen` API, rasterised without
//! anti-aliasing (most scripts call `screen.aa(0)` anyway).

use embedded_graphics::mono_font::ascii::FONT_5X8;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::{Baseline, Text};
use std::f32::consts::TAU;

pub const W: usize = 128;
pub const H: usize = 64;
/// Advance of one character of the built-in font, px.
pub const CHAR_W: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    Close,
}

pub struct Screen {
    /// Back buffer, one level (0..15) per pixel.
    pub px: Vec<u8>,
    level: u8,
    line_width: f32,
    cur: (f32, f32),
    /// Start of the current sub-path (for close).
    start: (f32, f32),
    path: Vec<Seg>,
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen {
    pub fn new() -> Self {
        Self { px: vec![0; W * H], level: 15, line_width: 1.0, cur: (0.0, 0.0), start: (0.0, 0.0), path: Vec::new() }
    }

    /// Draw over an existing picture.
    pub fn from_pixels(px: Vec<u8>) -> Self {
        let mut s = Self::new();
        if px.len() == W * H {
            s.px = px;
        }
        s
    }

    pub fn clear(&mut self) {
        self.px.fill(0);
        self.path.clear();
    }

    pub fn level(&mut self, l: i32) {
        self.level = l.clamp(0, 15) as u8;
    }

    pub fn line_width(&mut self, w: f32) {
        self.line_width = w.max(0.5);
    }

    pub fn move_to(&mut self, x: f32, y: f32) {
        self.cur = (x, y);
        self.start = (x, y);
        self.path.push(Seg::Move(x, y));
    }

    pub fn move_rel(&mut self, dx: f32, dy: f32) {
        self.move_to(self.cur.0 + dx, self.cur.1 + dy);
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        if self.path.is_empty() {
            self.path.push(Seg::Move(self.cur.0, self.cur.1));
            self.start = self.cur;
        }
        self.path.push(Seg::Line(x, y));
        self.cur = (x, y);
    }

    pub fn line_rel(&mut self, dx: f32, dy: f32) {
        self.line_to(self.cur.0 + dx, self.cur.1 + dy);
    }

    pub fn close(&mut self) {
        self.path.push(Seg::Close);
        self.cur = self.start;
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.move_to(x, y);
        self.line_to(x + w, y);
        self.line_to(x + w, y + h);
        self.line_to(x, y + h);
        self.close();
    }

    /// An arc around (cx, cy), angles in radians, as line segments. Like
    /// cairo, it connects from the current point when a path is open.
    pub fn arc(&mut self, cx: f32, cy: f32, r: f32, a1: f32, a2: f32) {
        let mut a2 = a2;
        while a2 < a1 {
            a2 += TAU;
        }
        let n = ((a2 - a1).abs() * r.max(1.0) / 2.0).ceil().clamp(4.0, 128.0) as usize;
        let p0 = (cx + r * a1.cos(), cy + r * a1.sin());
        if self.path.is_empty() {
            self.move_to(p0.0, p0.1);
        } else {
            self.line_to(p0.0, p0.1);
        }
        for i in 1..=n {
            let a = a1 + (a2 - a1) * i as f32 / n as f32;
            self.line_to(cx + r * a.cos(), cy + r * a.sin());
        }
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32) {
        self.move_to(cx + r, cy);
        self.arc(cx, cy, r, 0.0, TAU);
        self.close();
    }

    /// Cubic Bezier from the current point.
    pub fn curve(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x3: f32, y3: f32) {
        let (x0, y0) = self.cur;
        for i in 1..=16 {
            let t = i as f32 / 16.0;
            let u = 1.0 - t;
            let x = u * u * u * x0 + 3.0 * u * u * t * x1 + 3.0 * u * t * t * x2 + t * t * t * x3;
            let y = u * u * u * y0 + 3.0 * u * u * t * y1 + 3.0 * u * t * t * y2 + t * t * t * y3;
            self.line_to(x, y);
        }
    }

    pub fn curve_rel(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x3: f32, y3: f32) {
        let (x, y) = self.cur;
        self.curve(x + x1, y + y1, x + x2, y + y2, x + x3, y + y3);
    }

    pub fn pixel(&mut self, x: f32, y: f32) {
        self.rect(x.floor(), y.floor(), 1.0, 1.0);
    }

    fn put(&mut self, x: i32, y: i32) {
        if (0..W as i32).contains(&x) && (0..H as i32).contains(&y) {
            self.px[y as usize * W + x as usize] = self.level;
        }
    }

    /// One straight segment, covering the pixels the line passes over
    /// (end excluded, the way a 1-px cairo stroke lands).
    fn segment(&mut self, (x0, y0): (f32, f32), (x1, y1): (f32, f32)) {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = dx.abs().max(dy.abs());
        let w = self.line_width.round().max(1.0) as i32;
        // offset across the line for widths > 1
        let horiz = dx.abs() >= dy.abs();
        let steps = len.round() as i32;
        for s in 0..steps.max(if len > 0.0 { 1 } else { 0 }) {
            let t = s as f32 / len.max(1.0);
            let x = (x0 + dx * t).floor() as i32;
            let y = (y0 + dy * t).floor() as i32;
            for o in 0..w {
                let off = o - (w - 1) / 2;
                if horiz {
                    self.put(x, y + off - if w > 1 { 0 } else { 0 });
                } else {
                    self.put(x + off, y);
                }
            }
        }
    }

    fn polylines(&self) -> Vec<Vec<(f32, f32)>> {
        let mut out: Vec<Vec<(f32, f32)>> = Vec::new();
        let mut start = (0.0, 0.0);
        for s in &self.path {
            match *s {
                Seg::Move(x, y) => {
                    out.push(vec![(x, y)]);
                    start = (x, y);
                }
                Seg::Line(x, y) => {
                    if out.is_empty() {
                        out.push(vec![start]);
                    }
                    out.last_mut().unwrap().push((x, y));
                }
                Seg::Close => {
                    if let Some(p) = out.last_mut() {
                        p.push(p[0]);
                    }
                }
            }
        }
        out
    }

    pub fn stroke(&mut self) {
        for poly in self.polylines() {
            for w in poly.windows(2) {
                self.segment(w[0], w[1]);
            }
        }
        self.path.clear();
    }

    /// Even-odd scanline fill of the whole path, sampled at pixel centres.
    pub fn fill(&mut self) {
        let polys = self.polylines();
        for y in 0..H {
            let sy = y as f32 + 0.5;
            let mut xs: Vec<f32> = Vec::new();
            for poly in &polys {
                let n = poly.len();
                for i in 0..n {
                    let (a, b) = (poly[i], poly[(i + 1) % n]);
                    if (a.1 <= sy && b.1 > sy) || (b.1 <= sy && a.1 > sy) {
                        xs.push(a.0 + (sy - a.1) / (b.1 - a.1) * (b.0 - a.0));
                    }
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in xs.chunks(2) {
                if let [a, b] = pair {
                    let x0 = (a - 0.5).ceil() as i32;
                    let x1 = (b - 0.5).ceil() as i32;
                    for x in x0..x1 {
                        self.put(x, y as i32);
                    }
                }
            }
        }
        self.path.clear();
    }

    pub fn text_width(s: &str) -> f32 {
        s.chars().count() as f32 * CHAR_W
    }

    /// Text with its baseline at the current point; the point advances.
    pub fn text(&mut self, s: &str) {
        let (x, y) = self.cur;
        self.draw_text(x, y, s);
        self.cur.0 += Self::text_width(s);
    }

    pub fn text_right(&mut self, s: &str) {
        let (x, y) = self.cur;
        self.draw_text(x - Self::text_width(s), y, s);
    }

    pub fn text_center(&mut self, s: &str) {
        let (x, y) = self.cur;
        self.draw_text(x - Self::text_width(s) / 2.0, y, s);
    }

    fn draw_text(&mut self, x: f32, y: f32, s: &str) {
        let mut target = Mask { screen: self };
        let style = MonoTextStyle::new(&FONT_5X8, BinaryColor::On);
        Text::with_baseline(s, Point::new(x.round() as i32, y.round() as i32), style, Baseline::Alphabetic).draw(&mut target).ok();
    }
}

/// Lets embedded-graphics' font renderer paint at the current level.
struct Mask<'a> {
    screen: &'a mut Screen,
}

impl OriginDimensions for Mask<'_> {
    fn size(&self) -> Size {
        Size::new(W as u32, H as u32)
    }
}

impl DrawTarget for Mask<'_> {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;
    fn draw_iter<I: IntoIterator<Item = Pixel<BinaryColor>>>(&mut self, pixels: I) -> Result<(), Self::Error> {
        for Pixel(p, c) in pixels {
            if c == BinaryColor::On {
                self.screen.put(p.x, p.y);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &Screen) -> usize {
        s.px.iter().filter(|&&p| p > 0).count()
    }

    #[test]
    fn a_relative_line_covers_its_length() {
        let mut s = Screen::new();
        s.level(15);
        s.move_to(32.0, 30.0);
        s.line_rel(4.0, 0.0);
        s.stroke();
        assert_eq!(lit(&s), 4);
        assert_eq!(s.px[30 * W + 32], 15);
    }

    #[test]
    fn rect_fill_and_circle_fill() {
        let mut s = Screen::new();
        s.level(7);
        s.rect(10.0, 10.0, 4.0, 3.0);
        s.fill();
        assert_eq!(lit(&s), 12);
        s.clear();
        s.circle(64.0, 32.0, 10.0);
        s.fill();
        let n = lit(&s) as f32;
        assert!((n - std::f32::consts::PI * 100.0).abs() < 25.0, "{n}");
    }

    #[test]
    fn text_draws_and_advances() {
        let mut s = Screen::new();
        s.move_to(0.0, 10.0);
        s.text("STEP");
        assert!(lit(&s) > 10);
        assert!(s.px[..11 * W].iter().any(|&p| p > 0));
        assert!(s.px[11 * W..].iter().all(|&p| p == 0), "nothing below the baseline for caps");
    }
}
