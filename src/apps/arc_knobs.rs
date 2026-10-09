//! Arc Knobs: the arc's rings as knobs for any app.
//!
//! After knobs and quadknobs by MonsieurBon (monome community), which made
//! an arc into MIDI CC knobs and paged them to double or quadruple them.
//! Here each ring is a knob from 0 to 1 sent to any app's mod input (the
//! menu routes each one), in four banks, so an arc 4 is sixteen knobs. A
//! whole turn of a ring sweeps a knob end to end (fine control), pushing a
//! ring moves to the next bank. A knob is drawn as a 300-degree sweep like a
//! real one, with the bank shown as a dot at the bottom of every ring.
//!
//! It plays the shared arc (arc_kit.rs): the Arc app on screen, or a real
//! arc. Pick "Knobs" in the Arc app (SELECT) if another app holds the arc.

use crate::{
    app::play_kit::{KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input, SlintExtra},
    display::{FrameBuffer, WIDTH},
    modbus::{ModBus, Patch},
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16},
    util::AtomicF32,
};
use super::arc_kit::{self, ArcHub, Rings, LEDS, MAX_ENCODERS};
use super::kids_kit;
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}, text::Text};
use std::sync::Arc;

const APP_NAME: &str = "Knobs";
pub const BANKS: usize = 4;
const KNOBS: usize = BANKS * MAX_ENCODERS;
/// Steps for a knob's whole range (one turn of a real arc).
const STEPS_FULL: f32 = 1024.0;
/// The sweep: LEDs from about seven o'clock round to five o'clock.
const SWEEP_START: usize = 40;
const SWEEP_LEN: usize = 49;

const BG: Rgb565 = Rgb565::new(2, 5, 4);
const INK: Rgb565 = Rgb565::new(24, 48, 22);
const DIM: Rgb565 = Rgb565::new(10, 20, 10);
const ACCENT: Rgb565 = Rgb565::new(31, 40, 6);
const BAR: Rgb565 = Rgb565::new(8, 40, 26);

const C_BANK: usize = 0;
const C_ROUTES: usize = 1;
const N_CONTROLS: usize = C_ROUTES + KNOBS;

fn knob_name(k: usize) -> String {
    format!("Knob {}{}", k % MAX_ENCODERS + 1, (b'A' + (k / MAX_ENCODERS) as u8) as char)
}

/// One ring drawn as a knob at `v` (0..1), with the bank's dot.
pub fn ring_for(v: f32, bank: usize) -> [u8; LEDS] {
    let mut r = [0u8; LEDS];
    let lit = (v.clamp(0.0, 1.0) * (SWEEP_LEN - 1) as f32).round() as usize;
    for k in 0..SWEEP_LEN {
        r[(SWEEP_START + k) % LEDS] = if k < lit { 5 } else if k == lit { 15 } else { 1 };
    }
    // The bank: one of four dots in the gap at the bottom.
    r[30 + bank.min(BANKS - 1)] = 9;
    r
}

pub struct ArcKnobsApp {
    a: Arc<ArcHub>,
    mods: Arc<ModBus>,
    kit: PlayKit,
    list: ParamList,
    nav: Arc<AtomicF32>,
    values: [f32; KNOBS],
    /// 0 = not routed, else mod input index + 1.
    routes: [usize; KNOBS],
    bank: usize,
    /// The knob the menu's knobs turn when the arc isn't there.
    sel: usize,
    shown: Option<Rings>,
}

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "arc_knobs",
        layers: vec![Layer::Native(0, "KNOBS")],
        hero: Vec::new(),
        browse: None,
        routes: Routes { stick_x: None, stick_y: None, hand_l: None, hand_r: None },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: true,
    }
}

impl ArcKnobsApp {
    pub fn new(a: Arc<ArcHub>, mods: Arc<ModBus>, nav: Arc<AtomicF32>) -> Self {
        a.register(APP_NAME);
        Self { a, mods, kit: PlayKit::new(kit_config(), !cfg!(test)), list: ParamList::new(), nav, values: [0.5; KNOBS], routes: [0; KNOBS], bank: 0, sel: 0, shown: None }
    }

    fn knob(&self, ring: usize) -> usize {
        self.bank * MAX_ENCODERS + ring
    }

    /// Turns from the arc, the rings drawn back, the values sent on.
    fn frame(&mut self) {
        if self.a.focus().as_deref() == Some(APP_NAME) {
            for e in self.a.events(APP_NAME) {
                match e {
                    arc_kit::Event::Delta { n, d } => {
                        let k = self.knob(n);
                        self.values[k] = (self.values[k] + d as f32 / STEPS_FULL).clamp(0.0, 1.0);
                    }
                    arc_kit::Event::Key { down: true, .. } => self.bank = (self.bank + 1) % BANKS,
                    _ => {}
                }
            }
            let rings: Rings = std::array::from_fn(|n| ring_for(self.values[self.knob(n)], self.bank));
            // Shown every frame: focus may have come back with the rings dark.
            self.a.show(APP_NAME, &rings);
            self.shown = Some(rings);
        }
        for k in 0..KNOBS {
            if self.routes[k] > 0 {
                if let Some(h) = self.mods.get(self.routes[k] - 1) {
                    h.set(self.values[k]);
                }
            }
        }
    }

    fn text(&self, i: usize) -> (String, String) {
        match i {
            C_BANK => ("Bank".into(), ((b'A' + self.bank as u8) as char).to_string()),
            // One row per knob so every routing reads at a glance.
            _ => (knob_name(i - C_ROUTES), Patch::label(&self.mods, self.routes[i - C_ROUTES])),
        }
    }

    /// Steps a knob's route through None and then every mod input, app by app.
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
            C_BANK => self.bank = (self.bank as i32 + d).rem_euclid(BANKS as i32) as usize,
            _ => {
                let k = i - C_ROUTES;
                self.routes[k] = self.step_route(self.routes[k], d);
            }
        }
    }
}

impl PlayHost for ArcKnobsApp {
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
            C_BANK => self.bank = 0,
            _ => self.routes[i - C_ROUTES] = 0,
        }
    }
    fn kit_set_norm(&mut self, _i: usize, _v: f32) {}
    fn kit_line(&self) -> String {
        format!("bank {}", (b'A' + self.bank as u8) as char)
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        if pad < KNOBS { knob_name(pad) } else { String::new() }
    }
}

impl App for ArcKnobsApp {
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
        // Keeps sending while it holds the arc or feeds an app.
        self.a.focus().as_deref() == Some(APP_NAME) || self.routes.iter().any(|&r| r > 0)
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
        // Without an arc: a pad picks a knob, the knobs turn it.
        for pad in 0..16 {
            if step.input.grid[pad] {
                self.sel = pad;
                self.bank = pad / MAX_ENCODERS;
            }
        }
        let d = input.knob1 * 16 + input.knob2 * 2 + input.navigation_steps * -16;
        if d != 0 {
            self.values[self.sel] = (self.values[self.sel] + d as f32 / STEPS_FULL).clamp(0.0, 1.0);
        }
        if input.knob1_press {
            self.a.set_focus(APP_NAME);
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if self.kit.menu {
            Text::new("Arc Knobs", Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, INK)).draw(f).ok();
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 64, 24, 11, &r, BG, DIM, ACCENT);
            return;
        }
        let title = MonoTextStyle::new(&SPLEEN_8X16, INK);
        let small = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        Text::new("ARC KNOBS", Point::new(8, 18), title).draw(f).ok();
        let holds = if self.a.focus().as_deref() == Some(APP_NAME) { "arc: these knobs".to_string() } else { format!("arc: {} (SELECT takes it)", self.a.focus().unwrap_or_else(|| "nobody".into())) };
        Text::new(&holds, Point::new(200, 18), title).draw(f).ok();
        // Four banks of four, the live bank bright.
        let (x0, y0, cw, ch) = (16, 40, (WIDTH as i32 - 32) / 4, 68);
        for b in 0..BANKS {
            for n in 0..MAX_ENCODERS {
                let k = b * MAX_ENCODERS + n;
                let (x, y) = (x0 + n as i32 * cw, y0 + b as i32 * ch);
                let live = b == self.bank;
                let ink = if live { INK } else { DIM };
                let name = MonoTextStyle::new(&SPLEEN_8X16, if k == self.sel { ACCENT } else { ink });
                Text::new(&knob_name(k), Point::new(x, y + 12), name).draw(f).ok();
                let w = cw - 16;
                Rectangle::new(Point::new(x, y + 20), Size::new(w as u32, 10)).into_styled(PrimitiveStyle::with_stroke(DIM, 1)).draw(f).ok();
                let fill = (self.values[k] * (w - 2) as f32) as u32;
                if fill > 0 {
                    Rectangle::new(Point::new(x + 1, y + 21), Size::new(fill, 8)).into_styled(PrimitiveStyle::with_fill(if live { BAR } else { DIM })).draw(f).ok();
                }
                let to = if self.routes[k] == 0 { "not routed".to_string() } else { Patch::label(&self.mods, self.routes[k]) };
                Text::new(&to, Point::new(x, y + 44), MonoTextStyle::new(&SPLEEN_6X12, ink)).draw(f).ok();
            }
        }
        Text::new("turn a ring: its knob   push: next bank   pads + knobs: without an arc   R1: routes", Point::new(8, 354), small).draw(f).ok();
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
    Box::new(ArcKnobsApp::new(arc_kit::arc(), ctx.get(), ctx.named("nav_speed")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rings_turn_knobs_that_feed_mod_inputs_in_four_banks() {
        let a = Arc::new(ArcHub::new());
        let mods = Arc::new(ModBus::new());
        let cutoff = mods.register("Synth: Cutoff");
        let res = mods.register("Synth: Res");
        let mut app = ArcKnobsApp::new(Arc::clone(&a), Arc::clone(&mods), Arc::new(AtomicF32::new(3.0)));
        // Knob 1A to the cutoff, knob 2B to the resonance.
        app.control(C_ROUTES, 1);
        app.control(C_ROUTES + 5, 1);
        app.control(C_ROUTES + 5, 1);
        assert_eq!(app.text(C_ROUTES + 5).1, "Synth > Res");
        a.turn(0, -256);
        app.frame();
        assert!((cutoff.get() - 0.25).abs() < 1e-3, "{}", cutoff.get());
        // Push: bank B, where ring 2 is knob 2B.
        a.key(3, true);
        a.key(3, false);
        app.frame();
        a.turn(1, 512);
        app.frame();
        assert!((res.get() - 1.0).abs() < 1e-3);
        assert!((cutoff.get() - 0.25).abs() < 1e-3, "bank A's knob stays");
        let s = a.snapshot();
        assert_eq!((s.leds[1][31], s.leds[1][30]), (9, 0), "the bank's dot");
        assert_eq!(s.leds[1][(SWEEP_START + SWEEP_LEN - 1) % LEDS], 15, "a full knob");
        assert!(app.needs_background_audio());
    }
}
