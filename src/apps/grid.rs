//! Grid: the on-screen monome grid.
//!
//! Shows the shared grid (`grid_kit`) full screen, every key at its LED
//! level, and plays it: a mouse click (or touch) on a key presses it, and the
//! 16 pads press a 4 x 4 window of keys that the D-pad moves around the grid
//! a window at a time. Whatever app holds the grid (Kria, for now) gets the
//! presses and draws the LEDs, here and on a real monome grid at once.
//!
//! The menu (R1) sets the size: 8 x 16 by default (a grid 128), presets for
//! every monome size and beyond, or any rows 1-64 x columns 1-128. With
//! Follow hardware on, plugging in a grid sets the size to match. It also
//! picks which app plays the grid (SELECT does that too), and where on a big
//! grid the hardware's window sits.

use crate::{
    app::play_kit::{KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input, SlintExtra},
    display::{FrameBuffer, HEIGHT, WIDTH},
    led_output::PadColor,
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16},
    util::AtomicF32,
};
use super::grid_kit::{self, Grid, Snapshot};
use super::kids_kit;
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}, text::Text};
use std::sync::Arc;

const APP_NAME: &str = "Grid";
const PAGE: u8 = 0;

// Own palette: graphite case, warm-white varibright keys.
const BG: Rgb565 = Rgb565::new(2, 4, 2);
const KEY_OFF: Rgb565 = Rgb565::new(5, 10, 5);
const KEY_FULL: Rgb565 = Rgb565::new(31, 60, 24);
const INK: Rgb565 = Rgb565::new(22, 44, 20);
const DIM: Rgb565 = Rgb565::new(11, 22, 10);
const PADS: Rgb565 = Rgb565::new(8, 46, 31);
const HARDWARE: Rgb565 = Rgb565::new(31, 34, 4);
const HELD: Rgb565 = Rgb565::new(10, 52, 31);

/// Size presets, rows x columns: monome's 64, 128, 256 and 512, then
/// doubling up to the largest grid this supports.
const SIZES: [(usize, usize); 8] = [(8, 8), (8, 16), (16, 16), (16, 32), (32, 32), (32, 64), (64, 64), (64, 128)];

const C_SIZE: usize = 0;
const C_ROWS: usize = 1;
const C_COLS: usize = 2;
const C_FOLLOW: usize = 3;
const C_PLAYS: usize = 4;
const C_HARDWARE: usize = 5;
const C_HW_X: usize = 6;
const C_HW_Y: usize = 7;
const C_PADS_X: usize = 8;
const C_PADS_Y: usize = 9;
const N_CONTROLS: usize = 10;

/// Grid area on screen: below the title line, above the hint line.
const TOP: i32 = 26;
const BOTTOM: i32 = HEIGHT as i32 - 20;
const SIDE: i32 = 8;

/// Where the grid sits on screen: top-left corner and key pitch in pixels.
fn layout(rows: usize, cols: usize) -> (i32, i32, i32) {
    let (w, h) = (WIDTH as i32 - 2 * SIDE, BOTTOM - TOP);
    let cell = (w / cols.max(1) as i32).min(h / rows.max(1) as i32).max(1);
    let x0 = SIDE + (w - cell * cols as i32) / 2;
    let y0 = TOP + (h - cell * rows as i32) / 2;
    (x0, y0, cell)
}

/// A varibright level as a colour: unlit keys still show, like the
/// silicone of a real grid. The square root spreads the low levels apart
/// (an LED's 2 vs 4 looks far more different than a linear fade shows).
fn level_color(l: u8) -> Rgb565 {
    let t = (l.min(15) as f32 / 15.0).sqrt();
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Rgb565::new(mix(KEY_OFF.r(), KEY_FULL.r()), mix(KEY_OFF.g(), KEY_FULL.g()), mix(KEY_OFF.b(), KEY_FULL.b()))
}

fn outline(f: &mut FrameBuffer, x: i32, y: i32, w: i32, h: i32, c: Rgb565) {
    Rectangle::new(Point::new(x - 2, y - 2), Size::new((w + 3).max(1) as u32, (h + 3).max(1) as u32))
        .into_styled(PrimitiveStyle::with_stroke(c, 1))
        .draw(f)
        .ok();
}

pub struct GridApp {
    g: Arc<Grid>,
    kit: PlayKit,
    list: ParamList,
    nav: Arc<AtomicF32>,
    /// Top-left key of the pads' 4 x 4 window.
    win: (usize, usize),
    pads_down: [bool; 16],
    /// The key under the pointer while it's pressed.
    pointer: Option<(usize, usize)>,
}

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "grid",
        layers: vec![Layer::Native(PAGE, "GRID")],
        hero: Vec::new(),
        browse: None,
        routes: Routes { stick_x: None, stick_y: None, hand_l: None, hand_r: None },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: true,
    }
}

fn onoff(b: bool) -> String {
    if b { "On" } else { "Off" }.into()
}

impl GridApp {
    pub fn new(g: Arc<Grid>, nav: Arc<AtomicF32>) -> Self {
        Self { g, kit: PlayKit::new(kit_config(), !cfg!(test)), list: ParamList::new(), nav, win: (0, 0), pads_down: [false; 16], pointer: None }
    }

    /// Keeps the pads' window on the grid.
    fn clamp_window(&mut self) {
        let (rows, cols) = self.g.size();
        self.win = (self.win.0.min(cols.saturating_sub(4)), self.win.1.min(rows.saturating_sub(4)));
    }

    fn move_window(&mut self, dx: i32, dy: i32) {
        self.win = ((self.win.0 as i32 + dx).max(0) as usize, (self.win.1 as i32 + dy).max(0) as usize);
        self.clamp_window();
    }

    fn set_size(&mut self, rows: usize, cols: usize) {
        self.release_pads();
        self.g.set_size(rows, cols);
        self.clamp_window();
    }

    /// The next app (or previous, for -1) that can play the grid.
    fn cycle_focus(&mut self, d: i32) {
        let clients = self.g.clients();
        if clients.is_empty() {
            return;
        }
        self.release_pads();
        let cur = self.g.focus().and_then(|f| clients.iter().position(|c| *c == f)).unwrap_or(0);
        let n = clients.len() as i32;
        let next = ((cur as i32 + d) % n + n) % n;
        self.g.set_focus(&clients[next as usize]);
    }

    /// Lets go of whatever the pads and pointer hold, so nothing sticks
    /// down when the window moves or the grid changes under them.
    fn release_pads(&mut self) {
        for pad in 0..16 {
            if self.pads_down[pad] {
                let (x, y) = self.pad_key(pad);
                self.g.press(x, y, false);
            }
        }
        self.pads_down = [false; 16];
        if let Some((x, y)) = self.pointer.take() {
            self.g.press(x, y, false);
        }
    }

    /// Pads count row-major from the top-left, like grid keys.
    fn pad_key(&self, pad: usize) -> (usize, usize) {
        (self.win.0 + pad % 4, self.win.1 + pad / 4)
    }

    fn text(&self, i: usize) -> (String, String) {
        let (rows, cols) = self.g.size();
        let (ox, oy) = self.g.offset();
        let dev = self.g.device();
        match i {
            C_SIZE => ("Size".into(), format!("{rows} x {cols}{}", match SIZES.iter().position(|&s| s == (rows, cols)) {
                Some(_) => "",
                None => " (custom)",
            })),
            C_ROWS => ("Rows".into(), rows.to_string()),
            C_COLS => ("Columns".into(), cols.to_string()),
            C_FOLLOW => ("Follow hardware".into(), onoff(self.g.follows_device())),
            C_PLAYS => ("Plays".into(), self.g.focus().unwrap_or_else(|| "nothing yet".into())),
            C_HARDWARE => ("Hardware".into(), match &dev {
                Some(d) => format!("{} {} ({} x {})", d.kind, d.id, d.rows, d.cols),
                None => "none (serialosc)".into(),
            }),
            C_HW_X => ("Hardware column".into(), (ox + 1).to_string()),
            C_HW_Y => ("Hardware row".into(), (oy + 1).to_string()),
            C_PADS_X => ("Pads column".into(), (self.win.0 + 1).to_string()),
            C_PADS_Y => ("Pads row".into(), (self.win.1 + 1).to_string()),
            _ => (String::new(), String::new()),
        }
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
        let (rows, cols) = self.g.size();
        let (ox, oy) = self.g.offset();
        let step = |v: usize, max: usize| (v as i32 + d).clamp(1, max as i32) as usize;
        match i {
            C_SIZE => {
                // From the current size to the next preset up or down by area.
                let area = rows * cols;
                let last = SIZES.len() - 1;
                let next = match SIZES.iter().position(|&s| s == (rows, cols)) {
                    Some(k) => (k as i32 + d.signum()).clamp(0, last as i32) as usize,
                    None if d > 0 => SIZES.iter().position(|&(r, c)| r * c > area).unwrap_or(last),
                    None => SIZES.iter().rposition(|&(r, c)| r * c < area).unwrap_or(0),
                };
                let (r, c) = SIZES[next];
                self.set_size(r, c);
            }
            C_ROWS => self.set_size(step(rows, grid_kit::MAX_ROWS), cols),
            C_COLS => self.set_size(rows, step(cols, grid_kit::MAX_COLS)),
            C_FOLLOW => {
                self.release_pads();
                self.g.set_follow(d > 0);
                self.clamp_window();
            }
            C_PLAYS => self.cycle_focus(d.signum()),
            C_HW_X => self.g.set_offset((ox as i32 + d).max(0) as usize, oy),
            C_HW_Y => self.g.set_offset(ox, (oy as i32 + d).max(0) as usize),
            C_PADS_X => {
                self.release_pads();
                self.move_window(d, 0);
            }
            C_PADS_Y => {
                self.release_pads();
                self.move_window(0, d);
            }
            _ => {}
        }
    }

    fn draw_grid(&self, f: &mut FrameBuffer, s: &Snapshot) {
        let (x0, y0, cell) = layout(s.rows, s.cols);
        // A gap between keys while they're big enough to have one.
        let gap = if cell >= 16 { cell / 8 } else if cell >= 4 { 1 } else { 0 };
        let key = (cell - gap).max(1) as u32;
        for y in 0..s.rows {
            for x in 0..s.cols {
                let i = y * s.cols + x;
                let c = if s.held[i] && s.leds[i] == 0 { HELD } else { level_color(s.leds[i]) };
                let r = Rectangle::new(Point::new(x0 + x as i32 * cell, y0 + y as i32 * cell), Size::new(key, key));
                f.fill_solid(&r, c).ok();
                // A held key that's also lit gets a ring, so the press shows.
                if s.held[i] && s.leds[i] != 0 && cell >= 6 {
                    r.into_styled(PrimitiveStyle::with_stroke(HELD, 1)).draw(f).ok();
                }
            }
        }
        if let Some(d) = &s.device {
            let (ox, oy) = s.offset;
            let (w, h) = (d.cols.min(s.cols - ox), d.rows.min(s.rows - oy));
            outline(f, x0 + ox as i32 * cell, y0 + oy as i32 * cell, w as i32 * cell, h as i32 * cell, HARDWARE);
        }
        let (w, h) = (4.min(s.cols), 4.min(s.rows));
        outline(f, x0 + self.win.0 as i32 * cell, y0 + self.win.1 as i32 * cell, w as i32 * cell, h as i32 * cell, PADS);
    }
}

impl PlayHost for GridApp {
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
            C_SIZE | C_ROWS | C_COLS => self.set_size(grid_kit::DEFAULT_ROWS, grid_kit::DEFAULT_COLS),
            C_HW_X | C_HW_Y => self.g.set_offset(0, 0),
            C_PADS_X | C_PADS_Y => {
                self.release_pads();
                self.win = (0, 0);
            }
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, _i: usize, _v: f32) {}
    fn kit_line(&self) -> String {
        let (rows, cols) = self.g.size();
        format!("{rows} x {cols} -> {}", self.g.focus().unwrap_or_else(|| "nothing".into()))
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        let (x, y) = self.pad_key(pad);
        format!("{},{}", x + 1, y + 1)
    }
    /// The LED under each pad, in the four colours a pad has.
    fn kit_pad_color(&self, _layer: u8, pad: usize, held: bool) -> PadColor {
        if held {
            return PadColor::Red;
        }
        let s = self.g.snapshot();
        let (x, y) = self.pad_key(pad);
        if x >= s.cols || y >= s.rows {
            return PadColor::Off;
        }
        match s.leds[y * s.cols + x] {
            0 => PadColor::Off,
            1..=5 => PadColor::Blue,
            6..=10 => PadColor::Green,
            _ => PadColor::Yellow,
        }
    }
}

impl App for GridApp {
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
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        self.kit.led_overlay(self)
    }
    fn on_exit(&mut self) {
        self.release_pads();
    }
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        // The hardware may have resized the grid since last frame.
        self.clamp_window();
        if step.menu {
            let i = &step.input;
            self.list.navigate_input(i, N_CONTROLS, self.nav.get() as i32);
            let sel = self.list.selected.min(N_CONTROLS - 1);
            self.kit_edit(sel, i.knob2);
            if i.knob2_press {
                self.kit_reset(sel);
            }
            self.release_pads();
            return;
        }
        if step.native.is_none() {
            self.release_pads();
            return;
        }
        // The D-pad moves the pads' window a whole window at a time.
        if input.nav_x != 0 || input.navigation_steps != 0 {
            self.release_pads();
            self.move_window(input.nav_x * 4, input.navigation_steps * 4);
        }
        for pad in 0..16 {
            let down = step.input.grid[pad];
            if down != self.pads_down[pad] {
                let (x, y) = self.pad_key(pad);
                self.g.press(x, y, down);
                self.pads_down[pad] = down;
            }
        }
        if input.knob1_press {
            self.cycle_focus(1);
        }
    }
    /// A click or touch on the screen, in screen pixels; negative when it
    /// lets go. Sliding across keys presses each in turn.
    fn slint_pointer_pick(&mut self, x: f32, y: f32) {
        let up = |a: &mut Self| {
            if let Some((kx, ky)) = a.pointer.take() {
                a.g.press(kx, ky, false);
            }
        };
        if x < 0.0 || y < 0.0 || self.kit.menu {
            up(self);
            return;
        }
        let (rows, cols) = self.g.size();
        let (x0, y0, cell) = layout(rows, cols);
        let (kx, ky) = (((x - x0 as f32) / cell as f32).floor(), ((y - y0 as f32) / cell as f32).floor());
        if kx < 0.0 || ky < 0.0 || kx >= cols as f32 || ky >= rows as f32 {
            up(self);
            return;
        }
        let key = (kx as usize, ky as usize);
        if self.pointer != Some(key) {
            up(self);
            self.g.press(key.0, key.1, true);
            self.pointer = Some(key);
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if self.kit.menu {
            Text::new(APP_NAME, Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(f).ok();
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 64, 24, 11, &r, BG, DIM, PADS);
            return;
        }
        let s = self.g.snapshot();
        let title = MonoTextStyle::new(&SPLEEN_8X16, INK);
        let small = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        Text::new(&format!("GRID {} x {}", s.rows, s.cols), Point::new(SIDE, 18), title).draw(f).ok();
        let plays = format!("plays: {}", s.focus.as_deref().unwrap_or("nothing yet"));
        Text::new(&plays, Point::new(200, 18), title).draw(f).ok();
        let hw = match &s.device {
            Some(d) => format!("{} ({} x {})", d.kind, d.rows, d.cols),
            None => "no hardware".into(),
        };
        let hw_x = WIDTH as i32 - SIDE - 8 * hw.chars().count() as i32;
        Text::new(&hw, Point::new(hw_x, 18), MonoTextStyle::new(&SPLEEN_8X16, if s.device.is_some() { HARDWARE } else { DIM })).draw(f).ok();
        self.draw_grid(f, &s);
        Text::new("pads: the blue window   D-pad: move it   SELECT: next app   R1: size", Point::new(SIDE, HEIGHT as i32 - 6), small).draw(f).ok();
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
    Box::new(GridApp::new(grid_kit::grid(), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::grid_kit::{Key, Leds};

    fn app() -> (GridApp, Arc<Grid>) {
        let g = Arc::new(Grid::new());
        g.register("Seq");
        (GridApp::new(Arc::clone(&g), Arc::new(AtomicF32::new(3.0))), g)
    }

    /// The screen pixel at the middle of key (x, y).
    fn key_center(g: &Grid, x: usize, y: usize) -> (f32, f32) {
        let (rows, cols) = g.size();
        let (x0, y0, cell) = layout(rows, cols);
        ((x0 + x as i32 * cell + cell / 2) as f32, (y0 + y as i32 * cell + cell / 2) as f32)
    }

    #[test]
    fn a_click_on_the_screen_presses_that_key_and_letting_go_releases_it() {
        let (mut a, g) = app();
        let (px, py) = key_center(&g, 13, 6);
        a.slint_pointer_pick(px, py);
        a.slint_pointer_pick(px + 1.0, py); // same key: no second press
        a.slint_pointer_pick(-1.0, -1.0);
        assert_eq!(g.keys("Seq"), vec![Key { x: 13, y: 6, down: true }, Key { x: 13, y: 6, down: false }]);
        // Clicks between the grid and the screen edge press nothing.
        a.slint_pointer_pick(1.0, 1.0);
        assert!(g.keys("Seq").is_empty());
    }

    #[test]
    fn the_pads_press_a_window_the_dpad_moves_a_window_at_a_time() {
        let (mut a, g) = app();
        a.tick(&Input { nav_x: 1, navigation_steps: 1, ..Default::default() });
        assert_eq!(a.win, (4, 4));
        a.tick(&Input { nav_x: 9, ..Default::default() });
        assert_eq!(a.win, (12, 4), "kept on the 16 x 8 grid");
        // Pad 6 is the second row's second pad.
        a.tick(&Input { grid: std::array::from_fn(|i| i == 5), ..Default::default() });
        a.tick(&Input::default());
        assert_eq!(g.keys("Seq"), vec![Key { x: 13, y: 5, down: true }, Key { x: 13, y: 5, down: false }]);
    }

    #[test]
    fn the_size_steps_through_presets_up_to_64_by_128_or_any_size_by_hand() {
        let (mut a, g) = app();
        assert_eq!(g.size(), (8, 16));
        a.control(C_SIZE, 1);
        assert_eq!(g.size(), (16, 16));
        for _ in 0..10 {
            a.control(C_SIZE, 1);
        }
        assert_eq!(g.size(), (64, 128));
        a.control(C_ROWS, -3);
        assert_eq!(g.size(), (61, 128));
        assert!(a.text(C_SIZE).1.contains("custom"));
        a.control(C_SIZE, -1);
        assert_eq!(g.size(), (64, 64), "the next preset down by size");
        a.kit_reset(C_SIZE);
        assert_eq!(g.size(), (8, 16));
    }

    #[test]
    fn select_hands_the_grid_to_the_next_app() {
        let (mut a, g) = app();
        g.register("Teletype");
        a.tick(&Input { knob1_press: true, ..Default::default() });
        assert_eq!(g.focus().as_deref(), Some("Teletype"));
        a.control(C_PLAYS, 1);
        assert_eq!(g.focus().as_deref(), Some("Seq"));
    }

    #[test]
    fn lit_keys_draw_brighter_at_every_size() {
        let (mut a, g) = app();
        for (rows, cols) in [(8, 16), (64, 128), (1, 1), (37, 101)] {
            g.set_size(rows, cols);
            let mut l = Leds::new(rows, cols);
            l.set(0, 0, 15);
            g.show("Seq", &l);
            let mut fb = FrameBuffer::new();
            a.draw(&mut fb);
            let (x0, y0, cell) = layout(rows, cols);
            let px = |x: i32, y: i32| fb.buffer()[(y as usize) * WIDTH + x as usize];
            let lit = px(x0 + cell / 2 - (cell > 1) as i32, y0 + cell / 2 - (cell > 1) as i32);
            assert_ne!(lit, px(2, HEIGHT as i32 / 2), "{rows} x {cols}: the lit key shows");
            if cols > 1 {
                assert_ne!(lit, px(x0 + cell + cell / 2 - (cell > 1) as i32, y0 + cell / 2 - (cell > 1) as i32), "{rows} x {cols}: brighter than an unlit key");
            }
        }
        assert!(matches!(a.slint_extra(), SlintExtra::Screen(_)));
    }
}
