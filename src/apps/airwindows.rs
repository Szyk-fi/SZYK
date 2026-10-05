//! Airwindows: Chris Johnson's effect library (vendor/airwindows, MIT) --
//! every one of its ~520 plugins running its own, unmodified C++ DSP
//! through vendor/bridge/airwindows_bridge.cc.
//!
//! Nothing here is a model of an Airwindows effect: the bridge builds the
//! plugin's own class (a stand-in header replaces the VST2 SDK it was
//! written against), and the plugin's own `processReplacing` runs the
//! audio. Its parameter names, units and value text come from the plugin
//! itself too, so "Density" shows what Airwindows says Density is.
//!
//! Pick an input with Source, pick a Category (Airwindows' own grouping,
//! from its Airwindopedia) and step through the Effect. Every plugin
//! parameter is a 0..1 control; the play view puts the first six on dials
//! and all of them on the Controls pads.
//!
//! The audio bus is mono, so the plugin gets the same signal on both
//! channels; its stereo result goes to the speakers and, summed, onto the
//! bus for other apps to read.
//!
//! Real time: effects are built on the UI thread and handed to the audio
//! thread through a try_lock slot; the one they replace is handed back
//! the same way and dropped on the UI thread, so the callback never
//! allocates or frees.

use crate::{
    app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input},
    audio::AudioProcessor,
    audio_bus::{cycle_source, AudioBus, NO_SOURCE},
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_8X16, SPLEEN_6X12},
    util::AtomicF32,
};
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}, text::Text};
use std::ffi::{c_char, c_void, CStr};
use std::os::raw::c_int;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "Airwindows";

// Own palette: pale sky ink on deep night blue, after the Airwindows site.
const BG: Rgb565 = Rgb565::new(2, 6, 9);
const INK: Rgb565 = Rgb565::new(26, 54, 30);
const ACCENT: Rgb565 = Rgb565::new(10, 46, 26);
const DIM: Rgb565 = Rgb565::new(11, 26, 19);
const FAINT: Rgb565 = Rgb565::new(4, 10, 9);

unsafe extern "C" {
    fn aw_count() -> c_int;
    fn aw_id(i: c_int) -> *const c_char;
    fn aw_create(i: c_int, rate: f32) -> *mut c_void;
    fn aw_destroy(h: *mut c_void);
    fn aw_set_rate(h: *mut c_void, rate: f32);
    fn aw_param_count(h: *mut c_void) -> c_int;
    fn aw_param_name(h: *mut c_void, p: c_int, out: *mut c_char, n: c_int);
    fn aw_param_display(h: *mut c_void, p: c_int, out: *mut c_char, n: c_int);
    fn aw_param_label(h: *mut c_void, p: c_int, out: *mut c_char, n: c_int);
    fn aw_get(h: *mut c_void, p: c_int) -> f32;
    fn aw_set(h: *mut c_void, p: c_int, v: f32);
    fn aw_process(h: *mut c_void, in_l: *const f32, in_r: *const f32, out_l: *mut f32, out_r: *mut f32, n: c_int);
}

/// Airwindows' own grouping (vendor/airwindows/categories.txt, from its
/// Airwindopedia): `Category: Effect, Effect, ...`, best first.
const CATEGORIES_TXT: &str = include_str!("../../vendor/airwindows/categories.txt");

/// One plugin instance. The handle belongs to whoever holds the box.
struct Fx {
    h: *mut c_void,
    params: usize,
}

// The plugin has no thread affinity; a box only ever lives on one side at a time.
unsafe impl Send for Fx {}

impl Fx {
    fn new(index: usize, rate: f32) -> Option<Fx> {
        let h = unsafe { aw_create(index as c_int, rate) };
        if h.is_null() {
            return None;
        }
        let params = (unsafe { aw_param_count(h) }.max(0) as usize).min(MAX_PARAMS);
        Some(Fx { h, params })
    }

    fn text(&self, f: unsafe extern "C" fn(*mut c_void, c_int, *mut c_char, c_int), p: usize) -> String {
        let mut buf = [0 as c_char; 96];
        unsafe { f(self.h, p as c_int, buf.as_mut_ptr(), buf.len() as c_int) };
        unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().trim().to_string()
    }
    fn name(&self, p: usize) -> String {
        self.text(aw_param_name, p)
    }
    fn display(&self, p: usize) -> String {
        self.text(aw_param_display, p)
    }
    fn label(&self, p: usize) -> String {
        self.text(aw_param_label, p)
    }
    fn get(&self, p: usize) -> f32 {
        unsafe { aw_get(self.h, p as c_int) }
    }
    fn set(&self, p: usize, v: f32) {
        if p < self.params {
            unsafe { aw_set(self.h, p as c_int, v.clamp(0.0, 1.0)) };
        }
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        unsafe { aw_destroy(self.h) };
    }
}

/// The most parameters any Airwindows effect has is 37.
const MAX_PARAMS: usize = 40;
const BLOCK: usize = 1024;

/// The effect registry, name by index, in the bridge's (alphabetical) order.
fn effect_names() -> Vec<String> {
    (0..unsafe { aw_count() }).map(|i| unsafe { CStr::from_ptr(aw_id(i)) }.to_string_lossy().into_owned()).collect()
}

/// `(category, effect indices)`: "All" (alphabetical) first, then each
/// Airwindows category in its own best-first order. Names the registry
/// doesn't have are skipped, so a trimmed library still lists cleanly.
fn categories(names: &[String]) -> Vec<(String, Vec<usize>)> {
    let mut out = vec![("All".to_string(), (0..names.len()).collect::<Vec<_>>())];
    for line in CATEGORIES_TXT.lines() {
        let Some((cat, list)) = line.split_once(": ") else { continue };
        let members: Vec<usize> = list.split(',').filter_map(|n| names.iter().position(|x| x == n.trim())).collect();
        if !members.is_empty() {
            out.push((cat.trim().to_string(), members));
        }
    }
    out
}

const C_EFFECT: usize = 0;
const C_CATEGORY: usize = 1;
/// The first plugin parameter's control index.
const C_P0: usize = 2;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "airwindows",
        layers: vec![Layer::Controls, Layer::Moments],
        hero: vec![[C_EFFECT, C_CATEGORY], [C_P0, C_P0 + 1], [C_P0 + 2, C_P0 + 3], [C_P0 + 4, C_P0 + 5]],
        browse: Some(C_EFFECT),
        // Most Airwindows effects lead with their main amount; the rest vary
        // too much for a fixed meaning, so the stick takes the first two
        // and the hands the next two (bind others by wiggling).
        routes: Routes { stick_x: Some(C_P0), stick_y: Some(C_P0 + 1), hand_l: Some(C_P0 + 2), hand_r: Some(C_P0 + 3) },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: false,
    }
}

/// What the UI and the audio thread share.
struct Shared {
    /// Registry index of the chosen effect.
    effect: AtomicUsize,
    values: Vec<AtomicF32>,
    cv: [Arc<AtomicF32>; 4],
    enabled: AtomicBool,
    source: AtomicUsize,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    in_peak: AtomicF32,
    out_peak: AtomicF32,
    /// A freshly built effect waiting for the audio thread.
    incoming: Mutex<Option<Box<Fx>>>,
    /// Effects the audio thread has finished with, to be dropped by the UI.
    retired: Mutex<Vec<Box<Fx>>>,
    rate: AtomicF32,
    /// Set by the audio thread when the effect produced NaN or a runaway;
    /// the UI answers by building it afresh (its state is poisoned for good).
    poisoned: AtomicBool,
}

impl Shared {
    fn value(&self, p: usize) -> f32 {
        let cv = if p < 4 { self.cv[p].get() } else { 0.0 };
        (self.values[p].get() + cv).clamp(0.0, 1.0)
    }
}

pub struct AirwindowsApp {
    p: Arc<Shared>,
    bus: Arc<AudioBus>,
    own: usize,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
    names: Vec<String>,
    cats: Vec<(String, Vec<usize>)>,
    category: usize,
    /// The UI's own copy of the chosen effect: it answers name/unit/value
    /// text questions without touching the one on the audio thread.
    twin: Mutex<Fx>,
    defaults: Vec<f32>,
}

impl AirwindowsApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let names = effect_names();
        let cats = categories(&names);
        let output = bus.register(APP_NAME);
        let own = bus.index_of(APP_NAME).unwrap_or(NO_SOURCE);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        // Airwindows' own favourite first: the top of its Dynamics list is
        // a sensible, audible place to start; fall back to the first effect.
        let first = cats.iter().find(|c| c.0 == "Ambience").and_then(|c| c.1.first().copied()).unwrap_or(0);
        let twin = Fx::new(first, 48_000.0).expect("an Airwindows effect");
        let mut app = Self {
            p: Arc::new(Shared {
                effect: AtomicUsize::new(first),
                values: (0..MAX_PARAMS).map(|_| AtomicF32::new(0.0)).collect(),
                cv: std::array::from_fn(|i| mods.register(format!("{APP_NAME}: Param {}", i + 1))),
                enabled: AtomicBool::new(true),
                source: AtomicUsize::new(NO_SOURCE),
                mix_level,
                ext_mix_level,
                output,
                in_peak: AtomicF32::new(0.0),
                out_peak: AtomicF32::new(0.0),
                incoming: Mutex::new(None),
                retired: Mutex::new(Vec::new()),
                rate: AtomicF32::new(48_000.0),
                poisoned: AtomicBool::new(false),
            }),
            bus,
            own,
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            names,
            cats,
            category: 0,
            twin: Mutex::new(twin),
            defaults: Vec::new(),
        };
        app.category = app.cats.iter().position(|c| c.0 == "Ambience").unwrap_or(0);
        app.adopt_defaults();
        app
    }

    /// Reads the twin's startup values as this effect's defaults and applies them.
    fn adopt_defaults(&mut self) {
        let twin = self.twin.lock().unwrap();
        self.defaults = (0..twin.params).map(|p| twin.get(p)).collect();
        for (i, v) in self.p.values.iter().enumerate() {
            v.set(self.defaults.get(i).copied().unwrap_or(0.0));
        }
    }

    fn effect(&self) -> usize {
        self.p.effect.load(Ordering::Relaxed)
    }

    fn effect_name(&self) -> &str {
        self.names.get(self.effect()).map_or("", |s| s.as_str())
    }

    fn params(&self) -> usize {
        self.twin.lock().unwrap().params
    }

    /// Switches effect: a new instance for the twin and one for the audio
    /// thread, both at the plugin's own default settings.
    fn select_effect(&mut self, index: usize) {
        if index != self.effect() {
            self.rebuild(index);
        }
    }

    /// Builds the effect afresh even if it is the one already chosen.
    fn rebuild(&mut self, index: usize) {
        if index >= self.names.len() {
            return;
        }
        let rate = self.p.rate.get();
        let (Some(twin), Some(live)) = (Fx::new(index, rate), Fx::new(index, rate)) else { return };
        *self.twin.lock().unwrap() = twin;
        self.p.effect.store(index, Ordering::Relaxed);
        self.adopt_defaults();
        if let Ok(mut slot) = self.p.incoming.lock() {
            // A swap the audio thread never collected is simply replaced.
            *slot = Some(Box::new(live));
        }
    }

    fn members(&self) -> &[usize] {
        &self.cats[self.category.min(self.cats.len() - 1)].1
    }

    /// Steps the effect within the category, wrapping.
    fn step_effect(&mut self, d: i32) {
        let members = self.members().to_vec();
        let pos = members.iter().position(|&e| e == self.effect());
        let next = match pos {
            Some(p) => members[(p as i32 + d).rem_euclid(members.len() as i32) as usize],
            None => members[0],
        };
        self.select_effect(next);
    }

    fn step_category(&mut self, d: i32) {
        self.category = (self.category as i32 + d).rem_euclid(self.cats.len() as i32) as usize;
        if !self.members().contains(&self.effect()) {
            let first = self.members()[0];
            self.select_effect(first);
        }
    }

    fn drain_retired(&mut self) {
        if self.p.poisoned.swap(false, Ordering::Relaxed) {
            // Keep the player's settings: only the plugin's internal state is bad.
            let kept: Vec<f32> = self.p.values.iter().map(|v| v.get()).collect();
            self.rebuild(self.effect());
            for (v, k) in self.p.values.iter().zip(kept) {
                v.set(k);
            }
        }
        if let Ok(mut r) = self.p.retired.try_lock() {
            r.clear();
        }
    }

    fn param_text(&self, p: usize) -> (String, String) {
        let twin = self.twin.lock().unwrap();
        if p >= twin.params {
            return (String::new(), String::new());
        }
        twin.set(p, self.p.value(p));
        let (name, display, label) = (twin.name(p), twin.display(p), twin.label(p));
        (name, if label.is_empty() { display } else { format!("{display} {label}") })
    }

    fn edit_param(&mut self, p: usize, d: i32) {
        if p >= self.params() || d == 0 {
            return;
        }
        let step = 0.01 * self.sensitivity.get().max(0.01) * 10.0;
        let a = &self.p.values[p];
        a.set((a.get() + d as f32 * step).clamp(0.0, 1.0));
    }

    fn source_row(&self) -> String {
        self.bus.source_name(self.p.source.load(Ordering::Relaxed))
    }

    fn step_source(&self, d: i32) {
        let cur = self.p.source.load(Ordering::Relaxed);
        let mut s = cycle_source(cur, d.signum(), self.bus.len());
        if s == self.own {
            s = cycle_source(s, d.signum(), self.bus.len());
        }
        self.p.source.store(s, Ordering::Relaxed);
    }

    /// Menu rows: Source, Category, Effect, then the plugin's parameters.
    fn rows(&self) -> Vec<(String, String, bool)> {
        let mut r = vec![
            ("Source".to_string(), self.source_row(), false),
            ("Category".to_string(), self.cats[self.category].0.clone(), false),
            ("Effect".to_string(), self.effect_name().to_string(), false),
        ];
        for p in 0..self.params() {
            let (n, v) = self.param_text(p);
            r.push((n, v, false));
        }
        r
    }

    fn edit_row(&mut self, row: usize, d: i32) {
        if d == 0 {
            return;
        }
        match row {
            0 => self.step_source(d),
            1 => self.step_category(d.signum()),
            2 => self.step_effect(d.signum()),
            r => self.edit_param(r - 3, d),
        }
    }
}

impl PlayHost for AirwindowsApp {
    fn kit_control_count(&self) -> usize {
        C_P0 + self.params()
    }
    fn kit_label(&self, i: usize) -> String {
        match i {
            C_EFFECT => "Effect".into(),
            C_CATEGORY => "Category".into(),
            _ => self.param_text(i - C_P0).0,
        }
    }
    fn kit_value(&self, i: usize) -> String {
        match i {
            C_EFFECT => self.effect_name().to_string(),
            C_CATEGORY => self.cats[self.category].0.clone(),
            _ => self.param_text(i - C_P0).1,
        }
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        match i {
            C_EFFECT => {
                let m = self.members();
                let pos = m.iter().position(|&e| e == self.effect()).unwrap_or(0);
                Some(if m.len() > 1 { pos as f32 / (m.len() - 1) as f32 } else { 0.0 })
            }
            C_CATEGORY => Some(self.category as f32 / (self.cats.len() - 1).max(1) as f32),
            _ if i - C_P0 < self.params() => Some(self.p.values[i - C_P0].get()),
            _ => None,
        }
    }
    fn kit_stepped(&self, i: usize) -> bool {
        i < C_P0
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match i {
            C_EFFECT => self.step_effect(delta.signum()),
            C_CATEGORY => self.step_category(delta.signum()),
            _ => self.edit_param(i - C_P0, delta),
        }
    }
    fn kit_reset(&mut self, i: usize) {
        if i >= C_P0 {
            let p = i - C_P0;
            if let Some(&d) = self.defaults.get(p) {
                self.p.values[p].set(d);
            }
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i {
            C_EFFECT => {
                let m = self.members().to_vec();
                if let Some(&e) = m.get((v * (m.len() - 1) as f32).round() as usize) {
                    self.select_effect(e);
                }
            }
            C_CATEGORY => {
                let c = (v * (self.cats.len() - 1) as f32).round() as usize;
                if c != self.category {
                    self.step_category(c as i32 - self.category as i32);
                }
            }
            _ if i - C_P0 < self.params() => self.p.values[i - C_P0].set(v),
            _ => {}
        }
    }
    fn kit_line(&self) -> String {
        if self.p.source.load(Ordering::Relaxed) == NO_SOURCE {
            "R1: pick a Source".into()
        } else if !self.p.enabled.load(Ordering::Relaxed) {
            "Bypassed".into()
        } else {
            format!("{}  {}", self.effect_name(), self.cats[self.category].0)
        }
    }
}

impl App for AirwindowsApp {
    fn play_surface(&self) -> bool {
        true
    }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        (!self.kit.menu).then(|| self.kit.column(self))
    }
    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(self.kit.layer_label())
    }
    fn toggle_grid_mode(&mut self) {
        self.kit.next_layer();
    }
    fn grid_led_overlay(&self) -> [crate::led_output::PadColor; 16] {
        self.kit.led_overlay(self)
    }
    fn tick(&mut self, input: &Input) {
        self.drain_retired();
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let input = &step.input;
        let rows = 3 + self.params();
        self.list.navigate_input(input, rows, self.nav.get() as i32);
        self.edit_row(self.list.selected, input.knob2);
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if !self.kit.menu {
            if let Some(col) = self.play_column() {
                let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
                kit::draw::column(f, &col, 16, 40, 350, 285, pal);
            }
        } else {
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw(f, 16, 44, 24, r.len(), &r);
        }
        let ink = MonoTextStyle::new(&SPLEEN_6X12, INK);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        let big = MonoTextStyle::new(&SPLEEN_8X16, ACCENT);
        // Right panel: the effect, its category and how hard it is working.
        Text::new(self.effect_name(), Point::new(384, 60), big).draw(f).ok();
        Text::new(&self.cats[self.category].0, Point::new(384, 80), dim).draw(f).ok();
        let meter = |f: &mut FrameBuffer, y: i32, label: &str, v: f32| {
            Text::new(label, Point::new(384, y + 9), ink).draw(f).ok();
            Rectangle::new(Point::new(420, y), Size::new(200, 10)).into_styled(PrimitiveStyle::with_stroke(DIM, 1)).draw(f).ok();
            let w = (v.clamp(0.0, 1.0).sqrt() * 198.0) as u32;
            Rectangle::new(Point::new(421, y + 1), Size::new(w, 8)).into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(f).ok();
        };
        meter(f, 100, "IN", self.p.in_peak.get());
        meter(f, 118, "OUT", self.p.out_peak.get());
        // Every parameter, in the plugin's own words, as far as there's room.
        for p in 0..self.params().min(11) {
            let (n, v) = self.param_text(p);
            let y = 150 + p as i32 * 15;
            Text::new(&n.chars().take(16).collect::<String>(), Point::new(384, y), ink).draw(f).ok();
            Text::new(&v.chars().take(9).collect::<String>(), Point::new(560, y), dim).draw(f).ok();
            let x = (self.p.values[p].get() * 40.0) as u32;
            Rectangle::new(Point::new(486, y - 7), Size::new(x.max(1), 4)).into_styled(PrimitiveStyle::with_fill(DIM)).draw(f).ok();
        }
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn transport_action(&self) -> Option<&'static str> {
        Some(if self.p.enabled.load(Ordering::Relaxed) { "BYPASS" } else { "ENABLE" })
    }
    fn toggle_running(&mut self) {
        self.p.enabled.fetch_xor(true, Ordering::Relaxed);
    }
    fn needs_background_audio(&self) -> bool {
        self.p.source.load(Ordering::Relaxed) != NO_SOURCE
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if self.taken {
            return None;
        }
        self.taken = true;
        let first = Fx::new(self.effect(), self.p.rate.get())?;
        Some(Box::new(Processor {
            p: Arc::clone(&self.p),
            bus: Arc::clone(&self.bus),
            fx: Some(Box::new(first)),
            stash: None,
            last: [f32::NAN; MAX_PARAMS],
            input: Vec::with_capacity(4096),
            l: vec![0.0; BLOCK],
            r: vec![0.0; BLOCK],
            wet_l: vec![0.0; BLOCK],
            wet_r: vec![0.0; BLOCK],
            bypass: 0.0,
        }))
    }
}

struct Processor {
    p: Arc<Shared>,
    bus: Arc<AudioBus>,
    fx: Option<Box<Fx>>,
    /// A finished effect the retired slot had no room for yet.
    stash: Option<Box<Fx>>,
    /// The last value pushed to the plugin per parameter, so only changes are sent.
    last: [f32; MAX_PARAMS],
    input: Vec<f32>,
    l: Vec<f32>,
    r: Vec<f32>,
    wet_l: Vec<f32>,
    wet_r: Vec<f32>,
    /// 1 = fully bypassed (smoothed over 10 ms).
    bypass: f32,
}

impl Processor {
    /// Takes a newly built effect if one is waiting, without blocking.
    fn collect_new_effect(&mut self) {
        if let Some(old) = self.stash.take() {
            match self.p.retired.try_lock() {
                Ok(mut r) => r.push(old),
                Err(_) => self.stash = Some(old),
            }
        }
        if self.stash.is_some() {
            return;
        }
        let Ok(mut slot) = self.p.incoming.try_lock() else { return };
        if let Some(new) = slot.take() {
            self.stash = self.fx.replace(new);
            self.last = [f32::NAN; MAX_PARAMS];
        }
    }
}

fn copy_bus(bus: &AudioBus, idx: usize, into: &mut Vec<f32>) {
    into.clear();
    if idx == NO_SOURCE {
        return;
    }
    if let Some(b) = bus.get(idx) {
        if let Ok(b) = b.try_lock() {
            into.extend_from_slice(&b);
        }
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.0);
        if channels == 0 || !rate.is_finite() || rate < 1000.0 {
            return;
        }
        self.p.rate.set(rate);
        self.collect_new_effect();
        copy_bus(&self.bus, self.p.source.load(Ordering::Relaxed), &mut self.input);
        let Some(fx) = self.fx.as_ref() else { return };
        unsafe { aw_set_rate(fx.h, rate) };
        for pi in 0..fx.params {
            let v = self.p.value(pi);
            if self.last[pi] != v {
                fx.set(pi, v);
                self.last[pi] = v;
            }
        }
        let level = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0);
        let target_bypass = if self.p.enabled.load(Ordering::Relaxed) { 0.0 } else { 1.0 };
        let step = 1.0 / (rate * 0.01);
        let frames = out.len() / channels;
        let (mut in_peak, mut out_peak) = (0.0f32, 0.0f32);
        // The bus buffer keeps its capacity between blocks, so refilling it doesn't allocate.
        let mut bus = self.p.output.try_lock().ok();
        if let Some(b) = bus.as_mut() {
            b.clear();
        }
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(BLOCK);
            for k in 0..n {
                let x = self.input.get(done + k).copied().filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-4.0, 4.0);
                self.l[k] = x;
                self.r[k] = x;
            }
            unsafe { aw_process(fx.h, self.l.as_ptr(), self.r.as_ptr(), self.wet_l.as_mut_ptr(), self.wet_r.as_mut_ptr(), n as c_int) };
            // A plugin that blows up (NaN, runaway) must not take the speakers with it.
            let sane = self.wet_l[..n].iter().chain(self.wet_r[..n].iter()).all(|v| v.is_finite() && v.abs() < 64.0);
            if !sane {
                self.p.poisoned.store(true, Ordering::Relaxed);
            }
            for k in 0..n {
                let (dry, wl, wr) = (self.l[k], if sane { self.wet_l[k] } else { 0.0 }, if sane { self.wet_r[k] } else { 0.0 });
                self.bypass += (target_bypass - self.bypass).clamp(-step, step);
                let (yl, yr) = ((wl * (1.0 - self.bypass) + dry * self.bypass) * level, (wr * (1.0 - self.bypass) + dry * self.bypass) * level);
                in_peak = in_peak.max(dry.abs());
                out_peak = out_peak.max(yl.abs().max(yr.abs()));
                let frame = &mut out[(done + k) * channels..(done + k + 1) * channels];
                match frame {
                    [a, b, ..] => {
                        *a = yl;
                        *b = yr;
                    }
                    [a] => *a = (yl + yr) * 0.5,
                    [] => {}
                }
                if let Some(b) = bus.as_mut() {
                    b.push((yl + yr) * 0.5);
                }
            }
            done += n;
        }
        self.p.in_peak.set(in_peak);
        self.p.out_peak.set(out_peak);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (AirwindowsApp, Arc<Mutex<Vec<f32>>>) {
        let bus = Arc::new(AudioBus::new());
        let src = bus.register("Tone");
        let a = AirwindowsApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), bus, Arc::new(MixerBus::new()));
        (a, src)
    }

    fn run(p: &mut Box<dyn AudioProcessor>, src: &Arc<Mutex<Vec<f32>>>, amp: f32, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for b in 0..blocks {
            *src.lock().unwrap() = (0..256).map(|n| (std::f32::consts::TAU * 220.0 * (n + b * 256) as f32 / 48_000.0).sin() * amp).collect();
            let mut out = vec![0.0; 512];
            p.process(&mut out, 2, 48_000.0);
            assert!(out.iter().all(|v| v.is_finite()), "finite output");
            all.extend(out.iter().step_by(2));
        }
        all
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn index_of(a: &AirwindowsApp, name: &str) -> usize {
        a.names.iter().position(|n| n == name).unwrap_or_else(|| panic!("{name} is in the library"))
    }

    #[test]
    fn the_whole_library_is_there_with_airwindows_own_categories() {
        let (a, _) = fixture();
        assert!(a.names.len() >= 500, "{} effects", a.names.len());
        assert!(a.cats.len() > 15 && a.cats[0].0 == "All");
        let reverb = a.cats.iter().find(|c| c.0 == "Reverb").expect("a Reverb category");
        assert!(reverb.1.iter().any(|&i| a.names[i] == "kCathedral"));
    }

    #[test]
    fn every_effect_builds_names_its_parameters_and_stays_sane_on_noise() {
        let names = effect_names();
        let noise: Vec<f32> = (0..BLOCK).map(|i| (((i * 7919 + 13) % 2000) as f32 / 1000.0 - 1.0) * 0.5).collect();
        let mut loud = 0;
        let mut broken: Vec<String> = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let fx = Fx::new(i, 48_000.0).unwrap_or_else(|| panic!("{name} builds"));
            // A few (ClipOnly) are simply "on", with nothing to set.
            for p in 0..fx.params {
                assert!(!fx.name(p).is_empty(), "{name} parameter {p} has a name");
                let v = fx.get(p);
                assert!((0.0..=1.0).contains(&v), "{name} parameter {p} starts in range ({v})");
            }
            let (mut ol, mut or) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
            for _ in 0..4 {
                unsafe { aw_process(fx.h, noise.as_ptr(), noise.as_ptr(), ol.as_mut_ptr(), or.as_mut_ptr(), BLOCK as c_int) };
            }
            if !ol.iter().chain(or.iter()).all(|v| v.is_finite()) {
                broken.push(name.clone());
                continue;
            }
            if ol.iter().chain(or.iter()).any(|v| v.abs() > 16.0) {
                loud += 1;
            }
        }
        eprintln!("non-finite on noise: {broken:?}");
        assert!(broken.is_empty(), "non-finite on noise: {broken:?}");
        assert!(loud < 10, "{loud} effects exceed +24 dB on half-scale noise at default settings");
    }

    #[test]
    fn an_effect_changes_the_signal_it_is_given() {
        let (mut a, src) = fixture();
        a.select_effect(index_of(&a, "Density"));
        a.p.source.store(0, Ordering::Relaxed);
        let mut p = a.audio_processor().unwrap();
        let wet = run(&mut p, &src, 0.5, 100);
        assert!(rms(&wet[wet.len() - 4800..]) > 0.05, "something comes out");
        // Density's Dry/Wet at 0 hands the input back.
        let dry_param = (0..a.params()).find(|&q| a.param_text(q).0.contains("Dry/Wet")).expect("Density has Dry/Wet");
        a.p.values[dry_param].set(0.0);
        a.p.values[dry_param].set(0.0);
        let dry = run(&mut p, &src, 0.5, 100);
        let expect = 0.5 * std::f32::consts::FRAC_1_SQRT_2;
        assert!((rms(&dry[dry.len() - 4800..]) - expect).abs() < 0.03, "fully dry is the input ({})", rms(&dry[dry.len() - 4800..]));
    }

    #[test]
    fn switching_effects_while_running_hands_over_cleanly_and_frees_off_the_audio_thread() {
        let (mut a, src) = fixture();
        a.p.source.store(0, Ordering::Relaxed);
        let mut p = a.audio_processor().unwrap();
        run(&mut p, &src, 0.5, 10);
        let to = index_of(&a, "Density");
        a.select_effect(to);
        run(&mut p, &src, 0.5, 4);
        assert!(!a.p.retired.lock().unwrap().is_empty(), "the replaced effect was handed back, not dropped in the callback");
        a.tick(&Input::default());
        assert!(a.p.retired.lock().unwrap().is_empty(), "the UI frees it");
        assert_eq!(a.effect_name(), "Density");
    }

    #[test]
    fn category_and_effect_step_and_wrap_and_defaults_come_from_the_plugin() {
        let (mut a, _) = fixture();
        a.step_category(1);
        let cat_len = a.members().len();
        let start = a.effect();
        for _ in 0..cat_len {
            a.step_effect(1);
        }
        assert_eq!(a.effect(), start, "a full lap of the category returns to the start");
        let name = a.effect_name().to_string();
        let defaults = a.defaults.clone();
        a.edit_param(0, 5);
        a.kit_reset(C_P0);
        assert_eq!(a.p.values[0].get(), defaults[0], "reset restores {name}'s own default");
    }

    #[test]
    fn a_poisoned_effect_is_replaced_and_keeps_the_players_settings() {
        let (mut a, _) = fixture();
        let _p = a.audio_processor().unwrap();
        a.p.values[0].set(0.9);
        a.p.poisoned.store(true, Ordering::Relaxed);
        a.tick(&Input::default());
        assert!(!a.p.poisoned.load(Ordering::Relaxed));
        assert!(a.p.incoming.lock().unwrap().is_some(), "a fresh instance is queued for the audio thread");
        assert_eq!(a.p.values[0].get(), 0.9);
    }

    #[test]
    fn unpatched_and_bypassed_behave() {
        let (mut a, src) = fixture();
        let mut p = a.audio_processor().unwrap();
        // Airwindows adds its own ~1e-17 "dither" so filters never denormalise: inaudible, not silence.
        assert!(run(&mut p, &src, 0.9, 10).iter().all(|v| v.abs() < 1e-6), "no source: silent");
        a.p.source.store(0, Ordering::Relaxed);
        a.toggle_running();
        let x = run(&mut p, &src, 0.5, 50);
        assert!((rms(&x[x.len() - 4800..]) - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 0.02, "bypassed: dry");
    }

    #[test]
    fn opens_on_the_play_view_and_r1_opens_the_menu() {
        let (mut a, _) = fixture();
        assert!(a.play_column().is_some());
        a.tick(&Input { shoulder_press: [false, true], ..Default::default() });
        assert!(a.play_column().is_none());
        assert!(a.rows().len() > 3, "the menu lists the plugin's parameters");
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(AirwindowsApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}
