//! Arc: the on-screen monome arc.
//!
//! Shows the shared arc (`arc_kit`) full screen, four (or two) rings of 64
//! LEDs at their levels, and plays it: drag around a ring with the mouse (or
//! a finger) to turn it, a full circle being 1024 steps like the hardware's;
//! click inside a ring to push it. Knob 1 turns the selected ring and knob 2
//! the next one (the D-pad's up/down picks which); the pads push and turn
//! rings: the top row pushes rings 1-4, the second row turns them
//! anticlockwise and the third clockwise while held. Whatever app holds the
//! arc (a norns script, for now) gets the turns and draws the rings, here and
//! on a real monome arc at once.
//!
//! The menu (R1) sets two or four encoders (or Follow hardware), which app
//! plays the arc (SELECT does that too) and how many steps a knob click or a
//! held pad turns.

use crate::{
    app::play_kit::{KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input, SlintExtra},
    display::{FrameBuffer, HEIGHT, WIDTH},
    led_output::PadColor,
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16},
    util::AtomicF32,
};
use super::arc_kit::{self, ArcHub, Snapshot, LEDS};
use super::kids_kit;
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{Circle, PrimitiveStyle, Rectangle}, text::Text};
use std::f32::consts::TAU;
use std::sync::Arc;

const APP_NAME: &str = "Arc";
const PAGE: u8 = 0;

// Own palette: anodised aluminium, warm-white LEDs.
const BG: Rgb565 = Rgb565::new(3, 6, 4);
const LED_OFF: Rgb565 = Rgb565::new(6, 12, 7);
const LED_FULL: Rgb565 = Rgb565::new(31, 60, 24);
const INK: Rgb565 = Rgb565::new(22, 44, 20);
const DIM: Rgb565 = Rgb565::new(11, 22, 10);
const SELECTED: Rgb565 = Rgb565::new(8, 46, 31);
const HARDWARE: Rgb565 = Rgb565::new(31, 34, 4);
const HELD: Rgb565 = Rgb565::new(10, 52, 31);

/// Steps in a full turn, as a real arc counts them.
#[allow(dead_code)] // not used by the main binary
const STEPS_PER_TURN: f32 = 1024.0;

const C_ENCODERS: usize = 0;
const C_FOLLOW: usize = 1;
const C_PLAYS: usize = 2;
const C_HARDWARE: usize = 3;
const C_STEP: usize = 4;
const N_CONTROLS: usize = 5;

const TOP: i32 = 26;
const BOTTOM: i32 = HEIGHT as i32 - 20;

/// Ring `n` of `count`: centre and radius in pixels.
fn ring_at(n: usize, count: usize) -> (f32, f32, f32) {
    let count = count.max(1);
    let w = WIDTH as f32 / count as f32;
    let h = (BOTTOM - TOP) as f32;
    let r = (w.min(h) * 0.5 - 14.0).max(8.0);
    (w * (n as f32 + 0.5), TOP as f32 + h * 0.5, r)
}

/// LED `i`'s position: LED 0 at the top, going clockwise.
fn led_at(cx: f32, cy: f32, r: f32, i: usize) -> (f32, f32) {
    let a = i as f32 / LEDS as f32 * TAU;
    (cx + r * a.sin(), cy - r * a.cos())
}

fn level_color(l: u8) -> Rgb565 {
    let t = (l.min(15) as f32 / 15.0).sqrt();
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Rgb565::new(mix(LED_OFF.r(), LED_FULL.r()), mix(LED_OFF.g(), LED_FULL.g()), mix(LED_OFF.b(), LED_FULL.b()))
}

fn onoff(b: bool) -> String {
    if b { "On" } else { "Off" }.into()
}

/// What the pointer is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)] // not used by the main binary
enum Pointer {
    /// Turning ring `n`: the angle last seen and the part-step left over.
    Turn { n: usize, angle: f32, rest: f32 },
    /// Holding ring `n`'s push.
    Push(usize),
}

pub struct ArcApp {
    a: Arc<ArcHub>,
    kit: PlayKit,
    list: ParamList,
    nav: Arc<AtomicF32>,
    /// The ring knob 1 turns (knob 2 turns the next).
    sel: usize,
    /// Steps per knob click or held-pad frame.
    step: i32,
    pads_down: [bool; 16],
    pointer: Option<Pointer>,
}

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "arc",
        layers: vec![Layer::Native(PAGE, "ARC")],
        hero: Vec::new(),
        browse: None,
        routes: Routes { stick_x: None, stick_y: None, hand_l: None, hand_r: None },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: true,
    }
}

#[allow(dead_code)] // not used by the main binary
impl ArcApp {
    pub fn new(a: Arc<ArcHub>, nav: Arc<AtomicF32>) -> Self {
        Self { a, kit: PlayKit::new(kit_config(), !cfg!(test)), list: ParamList::new(), nav, sel: 0, step: 8, pads_down: [false; 16], pointer: None }
    }

    /// The next app (or previous) that can play the arc.
    fn cycle_focus(&mut self, d: i32) {
        let clients = self.a.clients();
        if clients.is_empty() {
            return;
        }
        self.release();
        let cur = self.a.focus().and_then(|f| clients.iter().position(|c| *c == f)).unwrap_or(0);
        let n = clients.len() as i32;
        self.a.set_focus(&clients[(((cur as i32 + d) % n + n) % n) as usize]);
    }

    /// Lets go of every push the pads and pointer hold.
    fn release(&mut self) {
        for n in 0..arc_kit::MAX_ENCODERS {
            if self.pads_down[n] {
                self.a.key(n, false);
            }
        }
        self.pads_down = [false; 16];
        if let Some(Pointer::Push(n)) = self.pointer.take() {
            self.a.key(n, false);
        }
    }

    fn text(&self, i: usize) -> (String, String) {
        match i {
            C_ENCODERS => ("Encoders".into(), self.a.encoders().to_string()),
            C_FOLLOW => ("Follow hardware".into(), onoff(self.a.follows_device())),
            C_PLAYS => ("Plays".into(), self.a.focus().unwrap_or_else(|| "nothing yet".into())),
            C_HARDWARE => ("Hardware".into(), match self.a.device() {
                Some(d) => format!("{} {} ({} encoders)", d.kind, d.id, d.encoders),
                None => "none: plug in an arc".into(),
            }),
            C_STEP => ("Steps per click".into(), self.step.to_string()),
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
        match i {
            C_ENCODERS => {
                self.release();
                self.a.set_encoders(if d > 0 { 4 } else { 2 });
                self.sel = self.sel.min(self.a.encoders() - 1);
            }
            C_FOLLOW => {
                self.release();
                self.a.set_follow(d > 0);
                self.sel = self.sel.min(self.a.encoders() - 1);
            }
            C_PLAYS => self.cycle_focus(d.signum()),
            C_STEP => self.step = (self.step + d).clamp(1, 64),
            _ => {}
        }
    }

    /// The ring under a screen point, and where on it: the angle from the
    /// top (clockwise, radians) and whether it's inside the LEDs.
    fn hit(&self, x: f32, y: f32) -> Option<(usize, f32, bool)> {
        let count = self.a.encoders();
        for n in 0..count {
            let (cx, cy, r) = ring_at(n, count);
            let (dx, dy) = (x - cx, y - cy);
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= r + 14.0 {
                return Some((n, dx.atan2(-dy).rem_euclid(TAU), dist < r * 0.55));
            }
        }
        None
    }

    fn draw_rings(&self, f: &mut FrameBuffer, s: &Snapshot) {
        for n in 0..s.encoders {
            let (cx, cy, r) = ring_at(n, s.encoders);
            let dot = if r > 60.0 { 7 } else { 5 };
            for i in 0..LEDS {
                let (x, y) = led_at(cx, cy, r, i);
                let c = level_color(s.leds[n][i]);
                Rectangle::new(Point::new(x as i32 - dot / 2, y as i32 - dot / 2), Size::new(dot as u32, dot as u32)).into_styled(PrimitiveStyle::with_fill(c)).draw(f).ok();
            }
            // The knob in the middle: lit while pushed, outlined when a
            // knob turns it.
            let knob = (r * 0.5) as u32 * 2;
            let centre = Point::new(cx as i32, cy as i32);
            let fill = if s.held[n] { HELD } else { BG };
            Circle::with_center(centre, knob).into_styled(PrimitiveStyle::with_fill(fill)).draw(f).ok();
            let pair = n == self.sel || (s.encoders > 1 && n == (self.sel + 1) % s.encoders);
            Circle::with_center(centre, knob).into_styled(PrimitiveStyle::with_stroke(if pair { SELECTED } else { DIM }, 2)).draw(f).ok();
            let label = MonoTextStyle::new(&SPLEEN_8X16, if pair { SELECTED } else { DIM });
            Text::new(&(n + 1).to_string(), Point::new(cx as i32 - 4, cy as i32 + 5), label).draw(f).ok();
        }
    }
}

impl PlayHost for ArcApp {
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
            C_ENCODERS => self.control(C_ENCODERS, 1),
            C_STEP => self.step = 8,
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, _i: usize, _v: f32) {}
    fn kit_line(&self) -> String {
        format!("arc {} -> {}", self.a.encoders(), self.a.focus().unwrap_or_else(|| "nothing".into()))
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        let n = pad % 4 + 1;
        match pad / 4 {
            0 => format!("push {n}"),
            1 => format!("{n} <"),
            2 => format!("{n} >"),
            _ => String::new(),
        }
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, held: bool) -> PadColor {
        if pad / 4 >= 3 || pad % 4 >= self.a.encoders() {
            return PadColor::Off;
        }
        if held {
            PadColor::Red
        } else if pad / 4 == 0 {
            PadColor::Yellow
        } else {
            PadColor::Blue
        }
    }
}

impl App for ArcApp {
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
        self.release();
    }
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let count = self.a.encoders();
        self.sel = self.sel.min(count - 1);
        if step.menu {
            let i = &step.input;
            self.list.navigate_input(i, N_CONTROLS, self.nav.get() as i32);
            let sel = self.list.selected.min(N_CONTROLS - 1);
            self.kit_edit(sel, i.knob2);
            if i.knob2_press {
                self.kit_reset(sel);
            }
            self.release();
            return;
        }
        if step.native.is_none() {
            self.release();
            return;
        }
        if input.navigation_steps != 0 {
            self.sel = (self.sel as i32 + input.navigation_steps).rem_euclid(count as i32) as usize;
        }
        // Raw knobs: the kit would turn a dial with them.
        self.a.turn(self.sel, input.knob1 * self.step);
        if count > 1 {
            self.a.turn((self.sel + 1) % count, input.knob2 * self.step);
        }
        for pad in 0..16 {
            let down = step.input.grid[pad];
            let n = pad % 4;
            if n < count {
                match pad / 4 {
                    0 if down != self.pads_down[pad] => self.a.key(n, down),
                    1 if down => self.a.turn(n, -self.step),
                    2 if down => self.a.turn(n, self.step),
                    _ => {}
                }
            }
            self.pads_down[pad] = down;
        }
        if input.knob1_press {
            self.cycle_focus(1);
        }
    }
    /// A click or touch, in screen pixels; negative when it lets go.
    /// Dragging round a ring turns it; inside the LEDs pushes it.
    fn slint_pointer_pick(&mut self, x: f32, y: f32) {
        if x < 0.0 || y < 0.0 || self.kit.menu {
            if let Some(Pointer::Push(n)) = self.pointer.take() {
                self.a.key(n, false);
            }
            self.pointer = None;
            return;
        }
        match self.pointer {
            None => match self.hit(x, y) {
                Some((n, _, true)) => {
                    self.a.key(n, true);
                    self.pointer = Some(Pointer::Push(n));
                }
                Some((n, angle, false)) => self.pointer = Some(Pointer::Turn { n, angle, rest: 0.0 }),
                None => {}
            },
            Some(Pointer::Turn { n, angle, rest }) => {
                let (cx, cy, _) = ring_at(n, self.a.encoders());
                let now = (x - cx).atan2(-(y - cy)).rem_euclid(TAU);
                // The short way round, so crossing the top isn't a whole turn.
                let mut d = now - angle;
                if d > TAU / 2.0 {
                    d -= TAU;
                } else if d < -TAU / 2.0 {
                    d += TAU;
                }
                let steps = d / TAU * STEPS_PER_TURN + rest;
                let whole = steps.trunc();
                self.a.turn(n, whole as i32);
                self.pointer = Some(Pointer::Turn { n, angle: now, rest: steps - whole });
            }
            Some(Pointer::Push(_)) => {}
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if self.kit.menu {
            Text::new(APP_NAME, Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(f).ok();
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 64, 24, 11, &r, BG, DIM, SELECTED);
            return;
        }
        let s = self.a.snapshot();
        let title = MonoTextStyle::new(&SPLEEN_8X16, INK);
        let small = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        Text::new(&format!("ARC {}", s.encoders), Point::new(8, 18), title).draw(f).ok();
        let plays = format!("plays: {}", s.focus.as_deref().unwrap_or("nothing yet"));
        Text::new(&plays, Point::new(200, 18), title).draw(f).ok();
        let hw = match &s.device {
            Some(d) => format!("{} ({})", d.kind, d.id),
            None => "no hardware".into(),
        };
        let hw_x = WIDTH as i32 - 8 - 8 * hw.chars().count() as i32;
        Text::new(&hw, Point::new(hw_x, 18), MonoTextStyle::new(&SPLEEN_8X16, if s.device.is_some() { HARDWARE } else { DIM })).draw(f).ok();
        self.draw_rings(f, &s);
        Text::new("drag a ring: turn   click its middle: push   knobs: the blue pair   SELECT: next app", Point::new(8, HEIGHT as i32 - 6), small).draw(f).ok();
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
    Box::new(ArcApp::new(arc_kit::arc(), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::arc_kit::Event;

    fn app() -> (ArcApp, Arc<ArcHub>) {
        let a = Arc::new(ArcHub::new());
        a.register("Script");
        (ArcApp::new(Arc::clone(&a), Arc::new(AtomicF32::new(3.0))), a)
    }

    #[test]
    fn dragging_round_a_ring_turns_it_a_1024th_of_a_turn_a_step() {
        let (mut app, a) = app();
        let (cx, cy, r) = ring_at(2, 4);
        // From the top, a quarter turn clockwise in small moves, across
        // nothing but ring 3.
        for k in 0..=16 {
            let t = k as f32 / 16.0 * TAU / 4.0;
            app.slint_pointer_pick(cx + r * t.sin(), cy - r * t.cos());
        }
        app.slint_pointer_pick(-1.0, -1.0);
        let total: i32 = a.events("Script").iter().map(|e| match e {
            Event::Delta { n: 2, d } => *d,
            other => panic!("{other:?}"),
        }).sum();
        assert!((255..=256).contains(&total), "{total}");
        // Back the other way, crossing the top: the short way round.
        for t in [0.1f32, -0.1] {
            app.slint_pointer_pick(cx + r * t.sin(), cy - r * t.cos());
        }
        app.slint_pointer_pick(-1.0, -1.0);
        let back: i32 = a.events("Script").iter().map(|e| if let Event::Delta { d, .. } = e { *d } else { 0 }).sum();
        assert!((-33..=-32).contains(&back), "{back}");
    }

    #[test]
    fn clicking_a_rings_middle_pushes_it_and_letting_go_releases() {
        let (mut app, a) = app();
        let (cx, cy, _) = ring_at(0, 4);
        app.slint_pointer_pick(cx, cy);
        app.slint_pointer_pick(cx + 1.0, cy);
        app.slint_pointer_pick(-1.0, -1.0);
        assert_eq!(a.events("Script"), [Event::Key { n: 0, down: true }, Event::Key { n: 0, down: false }]);
    }

    #[test]
    fn knobs_turn_the_selected_pair_and_pads_push_and_turn() {
        let (mut app, a) = app();
        app.tick(&Input { navigation_steps: 3, ..Default::default() });
        app.tick(&Input { knob1: 1, knob2: -2, ..Default::default() });
        assert_eq!(a.events("Script"), [Event::Delta { n: 3, d: 8 }, Event::Delta { n: 0, d: -16 }], "ring 4, and ring 1 after it");
        // Pad 2 pushes ring 2, pad 7 turns ring 3 back, pad 9 ring 1 on.
        let pads = |held: &[usize]| Input { grid: std::array::from_fn(|i| held.contains(&i)), ..Default::default() };
        app.tick(&pads(&[1, 6, 8]));
        app.tick(&pads(&[1, 6, 8]));
        app.tick(&Input::default());
        assert_eq!(
            a.events("Script"),
            [Event::Key { n: 1, down: true }, Event::Delta { n: 2, d: -8 }, Event::Delta { n: 0, d: 8 }, Event::Delta { n: 2, d: -8 }, Event::Delta { n: 0, d: 8 }, Event::Key { n: 1, down: false }]
        );
    }

    #[test]
    fn rings_draw_their_levels_with_two_or_four_encoders() {
        let (mut app, a) = app();
        for n in [4, 2] {
            a.set_encoders(n);
            let mut r = [[0u8; LEDS]; arc_kit::MAX_ENCODERS];
            r[1][16] = 15;
            a.show("Script", &r);
            let mut fb = FrameBuffer::new();
            app.draw(&mut fb);
            let (cx, cy, rad) = ring_at(1, n);
            let px = |i: usize| {
                let (x, y) = led_at(cx, cy, rad, i);
                fb.buffer()[y as usize * WIDTH + x as usize]
            };
            assert_ne!(px(16), px(17), "{n} rings: the lit LED is brighter than its neighbour");
        }
        assert!(matches!(app.slint_extra(), SlintExtra::Screen(_)));
    }

    /// Writes the Arc app's screen to the folder in PORTAMAX_ARC_SHOT.
    #[test]
    #[ignore = "writes screenshots to the folder in PORTAMAX_ARC_SHOT"]
    fn screenshot() {
        let Ok(dir) = std::env::var("PORTAMAX_ARC_SHOT") else { return };
        let (mut app, a) = app();
        let mut r = [[0u8; LEDS]; arc_kit::MAX_ENCODERS];
        let step = std::f64::consts::TAU / 64.0;
        for (n, v) in [0.3f64, 0.55, 0.8, 0.15].iter().enumerate() {
            arc_kit::draw::segment(&mut r[n], 0.0, v * 63.0 * step, 6);
            r[n][(v * 63.0) as usize] = 15;
        }
        a.show("Script", &r);
        a.key(2, true);
        let mut fb = FrameBuffer::new();
        app.draw(&mut fb);
        let mut out = b"P6\n640 360\n255\n".to_vec();
        out.extend(fb.buffer().iter().flat_map(|px| [(px >> 16) as u8, (px >> 8) as u8, *px as u8]));
        std::fs::write(std::path::Path::new(&dir).join("arc.ppm"), out).unwrap();
    }
}
