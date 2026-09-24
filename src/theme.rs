//! The device's shared theme colors -- accent (the list highlight,
//! every bespoke panel's active-state color, the bottom bar, ...) and
//! screen background -- read everywhere the live UI currently reads
//! a hardcoded green/near-black, so picking a new color in Settings
//! recolors the whole UI at once instead of just one screen. Stored
//! as HSV, not RGB -- that's what Settings' color wheel (hue +
//! saturation) and brightness control actually edit, so storing it
//! any other way would mean round-tripping through an RGB<->HSV
//! conversion on every knob tick for no reason.

use std::sync::atomic::{AtomicU32, Ordering};

/// The default green (`#5CF07A`) this build's accent originally
/// shipped with, expressed as the HSV triple Settings' wheel edits
/// (converts back to `#5CF07A` to within +/-1 per channel from float
/// rounding).
pub const ACCENT_DEFAULT_HUE: u32 = 132;
pub const ACCENT_DEFAULT_SAT: u32 = 62;
pub const ACCENT_DEFAULT_VAL: u32 = 94;

/// The default near-black screen background (`#0B100C`) this build
/// originally shipped with, same hue family as the default accent
/// (a dark, barely-saturated tint of the same green) so the two still
/// look deliberately paired if neither is ever touched.
pub const BG_DEFAULT_HUE: u32 = 132;
pub const BG_DEFAULT_SAT: u32 = 31;
pub const BG_DEFAULT_VAL: u32 = 6;

pub struct ThemeColor {
    hue: AtomicU32,
    sat: AtomicU32,
    val: AtomicU32,
}

impl ThemeColor {
    pub fn new(default_hue: u32, default_sat: u32, default_val: u32) -> Self {
        Self {
            hue: AtomicU32::new(default_hue),
            sat: AtomicU32::new(default_sat),
            val: AtomicU32::new(default_val),
        }
    }

    /// `(hue 0..360, saturation 0..100, value/brightness 0..100)`.
    pub fn hsv(&self) -> (u32, u32, u32) {
        (self.hue.load(Ordering::Relaxed), self.sat.load(Ordering::Relaxed), self.val.load(Ordering::Relaxed))
    }

    pub fn set_hue(&self, h: u32) {
        self.hue.store(h % 360, Ordering::Relaxed);
    }
    pub fn set_sat(&self, s: u32) {
        self.sat.store(s.min(100), Ordering::Relaxed);
    }
    pub fn set_val(&self, v: u32) {
        self.val.store(v.min(100), Ordering::Relaxed);
    }

    /// Sets hue/saturation together from a point on the wheel --
    /// `dx`/`dy` are pixel offsets from the wheel's center, `radius`
    /// the wheel's real on-screen radius in the same units, so a
    /// click anywhere outside the ring still clamps to full
    /// saturation instead of doing nothing.
    pub fn set_from_wheel(&self, dx: f32, dy: f32, radius: f32) {
        if radius <= 0.0 {
            return;
        }
        // Matches `@conic-gradient`'s own convention (0deg at the
        // top, increasing clockwise) so the marker this same state
        // drives lands exactly where you clicked -- see
        // `wheel_marker` for the inverse of this.
        let mut hue_deg = dx.atan2(-dy).to_degrees();
        if hue_deg < 0.0 {
            hue_deg += 360.0;
        }
        let sat = ((dx * dx + dy * dy).sqrt() / radius * 100.0).clamp(0.0, 100.0);
        self.set_hue(hue_deg.round() as u32);
        self.set_sat(sat.round() as u32);
    }

    /// The wheel marker's pixel offset from center for the current
    /// hue/saturation -- the exact inverse of `set_from_wheel`, so
    /// the dot always lands back where a click would reproduce it.
    pub fn wheel_marker(&self, radius: f32) -> (f32, f32) {
        let (h, s, _) = self.hsv();
        let theta = (h as f32).to_radians();
        let r = (s as f32 / 100.0) * radius;
        (r * theta.sin(), -r * theta.cos())
    }

    pub fn rgb(&self) -> (u8, u8, u8) {
        let (h, s, v) = self.hsv();
        hsv_to_rgb(h as f32, s as f32 / 100.0, v as f32 / 100.0)
    }

    pub fn hex(&self) -> String {
        let (r, g, b) = self.rgb();
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

/// Standard HSV -> RGB (`h` 0..360, `s`/`v` 0..1).
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime.rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h_prime as u32 % 6 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_red_green_blue_convert_exactly() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), (255, 0, 0));
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), (0, 255, 0));
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), (0, 0, 255));
    }

    #[test]
    fn zero_saturation_is_gray_regardless_of_hue() {
        assert_eq!(hsv_to_rgb(200.0, 0.0, 0.5), hsv_to_rgb(10.0, 0.0, 0.5));
    }

    #[test]
    fn the_default_accent_hsv_reproduces_the_original_green_within_rounding() {
        let (r, g, b) = hsv_to_rgb(ACCENT_DEFAULT_HUE as f32, ACCENT_DEFAULT_SAT as f32 / 100.0, ACCENT_DEFAULT_VAL as f32 / 100.0);
        assert!(r.abs_diff(0x5C) <= 1 && g.abs_diff(0xF0) <= 1 && b.abs_diff(0x7A) <= 1, "got #{r:02X}{g:02X}{b:02X}, expected close to #5CF07A");
    }

    #[test]
    fn the_default_background_hsv_reproduces_the_original_near_black_within_rounding() {
        let (r, g, b) = hsv_to_rgb(BG_DEFAULT_HUE as f32, BG_DEFAULT_SAT as f32 / 100.0, BG_DEFAULT_VAL as f32 / 100.0);
        assert!(r.abs_diff(0x0B) <= 1 && g.abs_diff(0x10) <= 1 && b.abs_diff(0x0C) <= 1, "got #{r:02X}{g:02X}{b:02X}, expected close to #0B100C");
    }

    #[test]
    fn set_hue_wraps_instead_of_going_out_of_range() {
        let c = ThemeColor::new(ACCENT_DEFAULT_HUE, ACCENT_DEFAULT_SAT, ACCENT_DEFAULT_VAL);
        c.set_hue(370);
        assert_eq!(c.hsv().0, 10);
    }

    #[test]
    fn wheel_round_trips_through_a_click_and_back_to_the_same_marker_position() {
        let c = ThemeColor::new(ACCENT_DEFAULT_HUE, ACCENT_DEFAULT_SAT, ACCENT_DEFAULT_VAL);
        let radius = 70.0;
        let (dx, dy) = (35.0, -20.0);
        c.set_from_wheel(dx, dy, radius);
        let (mx, my) = c.wheel_marker(radius);
        assert!((mx - dx).abs() < 0.5 && (my - dy).abs() < 0.5, "expected marker near ({dx}, {dy}), got ({mx}, {my})");
    }

    #[test]
    fn clicking_dead_center_zeroes_saturation() {
        let c = ThemeColor::new(ACCENT_DEFAULT_HUE, ACCENT_DEFAULT_SAT, ACCENT_DEFAULT_VAL);
        c.set_from_wheel(0.0, 0.0, 70.0);
        assert_eq!(c.hsv().1, 0);
    }
}
