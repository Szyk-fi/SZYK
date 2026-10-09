//! Grid Pads: the grid as XY pads for any app.
//!
//! After pad by Meng Qi (monome community), which split a grid into 4 x 4
//! XY pads sending MIDI. Here the grid is cut into 4 x 4 pads (eight on a
//! 8 x 16 grid) and each pad's X and Y (0 to 1, Y rising upwards) go to any
//! app's mod inputs, routed in the menu. A press moves the point there,
//! gliding if Glide is set; a pad shows a cross through its point.
//!
//! It plays the shared grid (grid_kit.rs): the Grid app's screen grid or a
//! real monome. Pick "Pads" in the Grid app (SELECT) if another app has it.

use crate::{
    app::play_kit::{KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input, SlintExtra},
    display::{FrameBuffer, WIDTH},
    modbus::{ModBus, Patch},
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16},
    util::AtomicF32,
};
use super::grid_kit::{self, Grid, Leds};
use super::kids_kit;
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}, text::Text};
use std::sync::Arc;
use std::time::Instant;

const APP_NAME: &str = "Pads";
pub const PADS: usize = 8;
const SIDE: usize = 4;
const OUTS: usize = PADS * 2;

const BG: Rgb565 = Rgb565::new(3, 4, 6);
const INK: Rgb565 = Rgb565::new(24, 46, 28);
const DIM: Rgb565 = Rgb565::new(10, 20, 12);
const ACCENT: Rgb565 = Rgb565::new(31, 30, 8);
const POINT: Rgb565 = Rgb565::new(8, 50, 31);

const C_GLIDE: usize = 0;
const C_ROUTES: usize = 1;
const N_CONTROLS: usize = C_ROUTES + OUTS;

fn out_name(o: usize) -> String {
    format!("Pad {} {}", o / 2 + 1, if o % 2 == 0 { "X" } else { "Y" })
}

#[derive(Clone, Copy, Debug)]
struct Pad {
    /// Where it is and where it's going, 0..1.
    x: f32,
    y: f32,
    tx: f32,
    ty: f32,
}

pub struct GridPadsApp {
    g: Arc<Grid>,
    mods: Arc<ModBus>,
    kit: PlayKit,
    list: ParamList,
    nav: Arc<AtomicF32>,
    pads: [Pad; PADS],
    routes: [usize; OUTS],
    /// Seconds to glide to a new point (0 = jump).
    glide: f32,
    last: Instant,
}

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "grid_pads",
        layers: vec![Layer::Native(0, "PADS")],
        hero: Vec::new(),
        browse: None,
        routes: Routes { stick_x: None, stick_y: None, hand_l: None, hand_r: None },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: true,
    }
}

/// Which pad a grid key is in, and where in it (0..1 across, 0..1 up).
fn locate(cols: usize, x: usize, y: usize) -> Option<(usize, f32, f32)> {
    let across = (cols / SIDE).max(1);
    let p = (y / SIDE) * across + x / SIDE;
    if x / SIDE >= across || p >= PADS {
        return None;
    }
    let fx = (x % SIDE) as f32 / (SIDE - 1) as f32;
    let fy = (SIDE - 1 - y % SIDE) as f32 / (SIDE - 1) as f32;
    Some((p, fx, fy))
}

impl GridPadsApp {
    pub fn new(g: Arc<Grid>, mods: Arc<ModBus>, nav: Arc<AtomicF32>) -> Self {
        g.register(APP_NAME);
        let pad = Pad { x: 0.5, y: 0.5, tx: 0.5, ty: 0.5 };
        Self { g, mods, kit: PlayKit::new(kit_config(), !cfg!(test)), list: ParamList::new(), nav, pads: [pad; PADS], routes: [0; OUTS], glide: 0.0, last: Instant::now() }
    }

    fn set_point(&mut self, p: usize, x: f32, y: f32) {
        let pad = &mut self.pads[p];
        pad.tx = x;
        pad.ty = y;
        if self.glide <= 0.0 {
            pad.x = x;
            pad.y = y;
        }
    }

    /// Presses in, points glided, the grid drawn, values sent on.
    fn frame(&mut self) {
        let dt = self.last.elapsed().as_secs_f32().min(0.1);
        self.last = Instant::now();
        let (rows, cols) = self.g.size();
        if self.g.focus().as_deref() == Some(APP_NAME) {
            for k in self.g.keys(APP_NAME) {
                if k.down {
                    if let Some((p, x, y)) = locate(cols, k.x, k.y) {
                        self.set_point(p, x, y);
                    }
                }
            }
        }
        let step = if self.glide > 0.0 { (dt / self.glide).min(1.0) } else { 1.0 };
        for pad in self.pads.iter_mut() {
            pad.x += (pad.tx - pad.x) * step;
            pad.y += (pad.ty - pad.y) * step;
        }
        if self.g.focus().as_deref() == Some(APP_NAME) {
            self.g.show(APP_NAME, &self.leds(rows, cols));
        }
        for o in 0..OUTS {
            if self.routes[o] > 0 {
                if let Some(h) = self.mods.get(self.routes[o] - 1) {
                    let pad = &self.pads[o / 2];
                    h.set(if o % 2 == 0 { pad.x } else { pad.y });
                }
            }
        }
    }

    fn leds(&self, rows: usize, cols: usize) -> Leds {
        let mut l = Leds::new(rows, cols);
        let across = (cols / SIDE).max(1);
        for (p, pad) in self.pads.iter().enumerate() {
            let (px, py) = ((p % across) * SIDE, (p / across) * SIDE);
            if px + SIDE > cols || py + SIDE > rows {
                continue;
            }
            let kx = px + (pad.x * (SIDE - 1) as f32).round() as usize;
            let ky = py + SIDE - 1 - (pad.y * (SIDE - 1) as f32).round() as usize;
            for i in 0..SIDE {
                // a faint pad, a cross through its point, the point bright
                for j in 0..SIDE {
                    l.set(px + i, py + j, if (p % across + p / across) % 2 == 0 { 1 } else { 0 });
                }
                l.set(px + i, ky, 5);
                l.set(kx, py + i, 5);
            }
            l.set(kx, ky, 15);
        }
        l
    }

    fn text(&self, i: usize) -> (String, String) {
        match i {
            C_GLIDE => ("Glide".into(), if self.glide <= 0.0 { "off".into() } else { format!("{:.0} ms", self.glide * 1000.0) }),
            // One row per output so every pad's routing reads at a glance.
            _ => (out_name(i - C_ROUTES), Patch::label(&self.mods, self.routes[i - C_ROUTES])),
        }
    }

    /// Steps an output through None and then every mod input, app by app.
    fn step_route(&self, route: usize, d: i32) -> usize {
        let apps = self.mods.apps();
        let all: Vec<usize> = apps.iter().flat_map(|(_, v)| v.iter().map(|i| i + 1)).collect();
        let at = if route == 0 { 0 } else { all.iter().position(|r| *r == route).map_or(0, |p| p + 1) };
        let next = (at as i32 + d.signum()).rem_euclid(all.len() as i32 + 1) as usize;
        if next == 0 { 0 } else { all[next - 1] }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        (0..N_CONTROLS).map(|i| {
            let (a, b) = self.text(i);
            (a, b, false)
        }).collect()
    }

    fn control(&mut self, i: usize, d: i32) {
        if d == 0 {
            return;
        }
        match i {
            C_GLIDE => self.glide = (self.glide + d as f32 * 0.05).clamp(0.0, 4.0),
            _ => {
                let o = i - C_ROUTES;
                self.routes[o] = self.step_route(self.routes[o], d);
            }
        }
    }
}

impl PlayHost for GridPadsApp {
    fn kit_control_count(&self) -> usize {
        N_CONTROLS
    }
    fn kit_label(&self, i: usize) -> String {
        self.text(i).0
    }
    fn kit_value(&self, i: usize) -> String {
        self.text(i).1
    }
    fn kit_norm(&self, _i: usize) -> Option<f32> {
        None
    }
    fn kit_stepped(&self, _i: usize) -> bool {
        true
    }
    fn kit_pads_play(&self, _layer: u8) -> bool {
        false
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.control(i, delta);
    }
    fn kit_reset(&mut self, i: usize) {
        match i {
            C_GLIDE => self.glide = 0.0,
            _ => self.routes[i - C_ROUTES] = 0,
        }
    }
    fn kit_set_norm(&mut self, _i: usize, _v: f32) {}
    fn kit_line(&self) -> String {
        format!("{} routed", self.routes.iter().filter(|&&r| r > 0).count())
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        format!("{},{}", pad % 4 + 1, 4 - pad / 4)
    }
}

impl App for GridPadsApp {
    fn play_surface(&self) -> bool {
        true
    }
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }
    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
    }
    fn needs_background_audio(&self) -> bool {
        self.g.focus().as_deref() == Some(APP_NAME) || self.routes.iter().any(|&r| r > 0)
    }
    fn background_tick(&mut self) {
        self.frame();
    }
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        if step.menu {
            let i = &step.input;
            self.list.navigate_input(i, N_CONTROLS, self.nav.get() as i32);
            let sel = self.list.selected.min(N_CONTROLS - 1);
            self.kit_edit(sel, i.knob2);
            if i.knob2_press {
                self.kit_reset(sel);
            }
            return;
        }
        if step.native.is_none() {
            return;
        }
        // The device's own 16 pads are pad 1, as a 4 x 4 grid.
        for pad in 0..16 {
            if step.input.grid[pad] {
                self.set_point(0, (pad % 4) as f32 / 3.0, (3 - pad / 4) as f32 / 3.0);
            }
        }
        if input.knob1_press {
            self.g.set_focus(APP_NAME);
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if self.kit.menu {
            Text::new("Grid Pads", Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(f).ok();
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 64, 24, 11, &r, BG, DIM, ACCENT);
            return;
        }
        let title = MonoTextStyle::new(&SPLEEN_8X16, INK);
        Text::new("GRID PADS", Point::new(8, 18), title).draw(f).ok();
        let holds = if self.g.focus().as_deref() == Some(APP_NAME) { "grid: these pads".to_string() } else { format!("grid: {} (SELECT takes it)", self.g.focus().unwrap_or_else(|| "nobody".into())) };
        Text::new(&holds, Point::new(200, 18), title).draw(f).ok();
        let size = 130;
        let gap = (WIDTH as i32 - 4 * size) / 5;
        for (p, pad) in self.pads.iter().enumerate() {
            let x = gap + (p % 4) as i32 * (size + gap);
            let y = 36 + (p / 4) as i32 * (size + 26);
            Rectangle::new(Point::new(x, y), Size::new(size as u32, size as u32)).into_styled(PrimitiveStyle::with_stroke(DIM, 1)).draw(f).ok();
            let (px, py) = (x + (pad.x * (size - 1) as f32) as i32, y + ((1.0 - pad.y) * (size - 1) as f32) as i32);
            Rectangle::new(Point::new(x, py), Size::new(size as u32, 1)).into_styled(PrimitiveStyle::with_fill(DIM)).draw(f).ok();
            Rectangle::new(Point::new(px, y), Size::new(1, size as u32)).into_styled(PrimitiveStyle::with_fill(DIM)).draw(f).ok();
            Rectangle::new(Point::new(px - 4, py - 4), Size::new(9, 9)).into_styled(PrimitiveStyle::with_fill(POINT)).draw(f).ok();
            let routed = self.routes[2 * p] > 0 || self.routes[2 * p + 1] > 0;
            let label = format!("{} {:.2} {:.2}", p + 1, pad.x, pad.y);
            Text::new(&label, Point::new(x, y + size + 14), MonoTextStyle::new(&SPLEEN_6X12, if routed { INK } else { DIM })).draw(f).ok();
        }
    }
    fn slint_extra(&mut self) -> SlintExtra {
        let mut fb = FrameBuffer::new();
        self.draw(&mut fb);
        kids_kit::screen_extra(&fb)
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn App> {
    Box::new(GridPadsApp::new(grid_kit::grid(), ctx.get(), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_moves_its_pads_point_and_the_values_reach_mod_inputs() {
        let g = Arc::new(Grid::new());
        let mods = Arc::new(ModBus::new());
        let pan = mods.register("Synth: Pan");
        let tone = mods.register("Synth: Tone");
        let mut app = GridPadsApp::new(Arc::clone(&g), Arc::clone(&mods), Arc::new(AtomicF32::new(3.0)));
        // Pad 6 (second row, second pad) X to pan, Y to tone.
        let o = 2 * 5;
        app.control(C_ROUTES + o, 1);
        app.control(C_ROUTES + o + 1, 1);
        app.control(C_ROUTES + o + 1, 1);
        // Its bottom-right key: X 1, Y 0.
        g.press(7, 7, true);
        g.press(7, 7, false);
        app.frame();
        assert_eq!((pan.get(), tone.get()), (1.0, 0.0));
        // Top-left, a third of the way across from there in Y terms.
        g.press(4, 5, true);
        app.frame();
        assert_eq!(pan.get(), 0.0);
        assert!((tone.get() - 2.0 / 3.0).abs() < 1e-6);
        let s = g.snapshot();
        assert_eq!(s.leds[5 * 16 + 4], 15, "the point lights");
        assert_eq!(s.leds[5 * 16 + 7], 5, "and its cross");
        // Gliding gets there over time, not at once.
        app.glide = 0.5;
        g.press(4, 5, false);
        g.press(7, 4, true);
        app.frame();
        assert!(pan.get() < 0.5);
    }

    #[test]
    fn keys_outside_the_pads_are_ignored() {
        assert_eq!(locate(16, 15, 7).map(|p| p.0), Some(7));
        assert_eq!(locate(16, 3, 9), None, "a third row of pads is past eight");
        assert_eq!(locate(6, 5, 0), None, "a pad that doesn't fit");
    }
}
