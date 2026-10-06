//! Shared parts for the Mutable Instruments module apps (Rings, Elements,
//! Marbles, Tides). Each of those wraps the module's real C++ DSP through
//! a bridge in `vendor/bridge`; what they have in common on the Rust side
//! lives here:
//! - `Controls`: a module's panel as a table of knobs and switches, shared
//!   with the audio thread, with modulation inputs on the mod bus and
//!   everything the play view (`PlayHost`) needs.
//! - `RateBridge`: runs a fixed-rate engine (Rings at 48 kHz, Elements and
//!   Marbles at 32 kHz) at the device's rate.
//! - `Keys`: pads and MIDI keys as note events, for the voices.
//! - `CvOuts`: a module's CV outputs patched to other apps' mod inputs.
//!
//! Not an app itself (no `create`), so the registry skips it.

use crate::app::Input;
use crate::modbus::ModBus;
use crate::util::{accelerate, AtomicF32};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// One knob or switch on a module's panel.
pub struct Spec {
    pub name: &'static str,
    /// Empty for a knob; the positions of a switch otherwise.
    pub choices: &'static [&'static str],
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// How a knob's value reads; `None` shows 0..1 knobs as a percentage.
    pub fmt: Option<fn(f32) -> String>,
    /// Registers "<App>: <name>" on the mod bus.
    pub modulated: bool,
}

impl Spec {
    pub const fn knob(name: &'static str, default: f32) -> Spec {
        Spec { name, choices: &[], min: 0.0, max: 1.0, default, fmt: None, modulated: true }
    }
    pub const fn range(name: &'static str, min: f32, max: f32, default: f32, fmt: fn(f32) -> String) -> Spec {
        Spec { name, choices: &[], min, max, default, fmt: Some(fmt), modulated: true }
    }
    pub const fn switch(name: &'static str, choices: &'static [&'static str], default: usize) -> Spec {
        Spec { name, choices, min: 0.0, max: (choices.len() - 1) as f32, default: default as f32, fmt: None, modulated: false }
    }
    pub fn is_switch(&self) -> bool {
        !self.choices.is_empty()
    }
}

/// A module's panel: one value per `Spec`, plus a mod-bus input for each
/// modulated knob. Shared with the audio thread through an `Arc`.
pub struct Controls {
    pub specs: &'static [Spec],
    values: Vec<AtomicF32>,
    ext: Vec<Option<Arc<AtomicF32>>>,
}

impl Controls {
    pub fn new(specs: &'static [Spec], app: &str, modbus: &ModBus) -> Controls {
        Controls {
            specs,
            values: specs.iter().map(|s| AtomicF32::new(s.default)).collect(),
            ext: specs.iter().map(|s| s.modulated.then(|| modbus.register(format!("{app}: {}", s.name)))).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// The knob's own setting, without modulation.
    pub fn raw(&self, i: usize) -> f32 {
        self.values[i].get()
    }

    /// The value the DSP uses: the knob plus its mod-bus input (a full
    /// 0..1 of modulation sweeps the whole range), clamped.
    pub fn get(&self, i: usize) -> f32 {
        let s = &self.specs[i];
        let ext = self.ext[i].as_ref().map_or(0.0, |e| e.get());
        (self.values[i].get() + ext * (s.max - s.min)).clamp(s.min, s.max)
    }

    /// A switch's position.
    pub fn choice(&self, i: usize) -> usize {
        self.values[i].get().round().max(0.0) as usize
    }

    pub fn set(&self, i: usize, v: f32) {
        let s = &self.specs[i];
        self.values[i].set(v.clamp(s.min, s.max));
    }

    pub fn text(&self, i: usize) -> String {
        let s = &self.specs[i];
        if s.is_switch() {
            return s.choices.get(self.choice(i)).copied().unwrap_or("?").to_string();
        }
        match s.fmt {
            Some(f) => f(self.raw(i)),
            None => format!("{:.0}%", self.raw(i) * 100.0),
        }
    }

    pub fn norm(&self, i: usize) -> f32 {
        let s = &self.specs[i];
        if s.max > s.min { (self.raw(i) - s.min) / (s.max - s.min) } else { 0.0 }
    }

    pub fn set_norm(&self, i: usize, v: f32) {
        let s = &self.specs[i];
        let x = s.min + v.clamp(0.0, 1.0) * (s.max - s.min);
        self.set(i, if s.is_switch() { x.round() } else { x });
    }

    /// One step of the D-pad or an encoder: switches move one position
    /// (wrapping), knobs by `sensitivity` of their range per detent.
    pub fn edit(&self, i: usize, delta: i32, sensitivity: f32) {
        if delta == 0 {
            return;
        }
        let s = &self.specs[i];
        if s.is_switch() {
            let n = s.choices.len() as i32;
            self.set(i, (self.choice(i) as i32 + delta.signum()).rem_euclid(n) as f32);
        } else {
            self.set(i, self.raw(i) + accelerate(delta) * sensitivity * (s.max - s.min) * 0.05);
        }
    }

    pub fn reset(&self, i: usize) {
        self.set(i, self.specs[i].default);
    }

    /// The mod-bus input names this panel registers, for the manifest test.
    #[allow(dead_code)]
    pub fn mod_input_names(&self, app: &str) -> Vec<String> {
        self.specs.iter().filter(|s| s.modulated).map(|s| format!("{app}: {}", s.name)).collect()
    }
}

/// Runs an engine with a fixed sample rate and block size at the device's
/// rate: engine blocks are rendered on demand and read back with linear
/// interpolation, and the mono input is resampled to the engine's rate
/// on the way in. Allocates only in `new`.
pub struct RateBridge {
    engine_rate: f32,
    block: usize,
    /// Engine frames rendered but not yet consumed.
    rendered: Vec<[f32; 2]>,
    next_frame: usize,
    prev: [f32; 2],
    next: [f32; 2],
    frac: f32,
    /// Device-rate input history, read at `in_pos`.
    input: VecDeque<f32>,
    in_pos: f32,
    engine_in: Vec<f32>,
    primed: bool,
}

impl RateBridge {
    pub fn new(engine_rate: f32, block: usize) -> RateBridge {
        RateBridge {
            engine_rate,
            block,
            rendered: vec![[0.0; 2]; block],
            next_frame: block,
            prev: [0.0; 2],
            next: [0.0; 2],
            frac: 0.0,
            input: VecDeque::with_capacity(8192),
            in_pos: 0.0,
            engine_in: vec![0.0; block],
            primed: false,
        }
    }

    /// How many device samples the input path is delayed by.
    fn latency(block: usize, in_step: f32) -> usize {
        (block as f32 * in_step).ceil() as usize + 2
    }

    /// Fills `out` (stereo frames at `device_rate`) from `render`, which
    /// receives one engine block of input and fills one block of output.
    /// `input` is mono at the device rate, as long as `out`.
    pub fn process(&mut self, input: &[f32], out: &mut [[f32; 2]], device_rate: f32, mut render: impl FnMut(&[f32], &mut [[f32; 2]])) {
        let step = self.engine_rate / device_rate;
        let in_step = device_rate / self.engine_rate;
        if !self.primed {
            // Blocks are rendered as soon as the first of their frames is
            // needed, so a block's input reaches up to one block into the
            // future. Delaying the input by that much means it has always
            // arrived by the time it's read.
            self.primed = true;
            for _ in 0..Self::latency(self.block, in_step) {
                self.input.push_back(0.0);
            }
        }
        for (j, o) in out.iter_mut().enumerate() {
            if self.input.len() < self.input.capacity() {
                self.input.push_back(input.get(j).copied().unwrap_or(0.0));
            }
            self.frac += step;
            while self.frac >= 1.0 {
                self.frac -= 1.0;
                if self.next_frame >= self.block {
                    for k in 0..self.block {
                        let i0 = self.in_pos.floor() as usize;
                        let f = self.in_pos - i0 as f32;
                        let a = self.input.get(i0).or(self.input.back()).copied().unwrap_or(0.0);
                        let b = self.input.get(i0 + 1).copied().unwrap_or(a);
                        self.engine_in[k] = a + (b - a) * f;
                        self.in_pos += in_step;
                    }
                    // Drop input already read, keeping one sample for
                    // interpolation.
                    while self.in_pos >= 1.0 && self.input.len() > 1 {
                        self.input.pop_front();
                        self.in_pos -= 1.0;
                    }
                    render(&self.engine_in, &mut self.rendered);
                    self.next_frame = 0;
                }
                self.prev = self.next;
                self.next = self.rendered[self.next_frame];
                self.next_frame += 1;
            }
            for c in 0..2 {
                o[c] = self.prev[c] + (self.next[c] - self.prev[c]) * self.frac;
            }
        }
    }
}

/// A note event from the pads or a keyboard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteEvent {
    pub note: u8,
    /// 0 for a release.
    pub velocity: u8,
}

/// Note events on their way from the UI thread to the audio thread. The
/// audio side only ever `try_lock`s, so a busy UI never stalls it.
#[derive(Default)]
pub struct NoteQueue(Mutex<VecDeque<NoteEvent>>);

impl NoteQueue {
    pub fn pop(&self) -> Option<NoteEvent> {
        self.0.try_lock().ok()?.pop_front()
    }
    fn push(&self, events: &[NoteEvent]) {
        if let Ok(mut q) = self.0.lock() {
            q.extend(events.iter().copied());
            while q.len() > 64 {
                q.pop_front();
            }
        }
    }
}

/// Turns the pads and the MIDI keys (which include notes other apps send
/// over the note bus) into note-on/off events and the set of held notes.
#[derive(Default)]
pub struct Keys {
    pub queue: Arc<NoteQueue>,
    prev_pads: [bool; 16],
    prev_midi: Vec<u8>,
    /// Held notes, oldest first.
    held: Vec<u8>,
}

impl Keys {
    /// `pad_note` gives each physical pad's note.
    pub fn update(&mut self, input: &Input, pads_play: bool, pad_note: impl Fn(usize) -> u8) {
        let mut events = Vec::new();
        for i in 0..16 {
            let now = pads_play && input.grid[i];
            if now != self.prev_pads[i] {
                events.push(NoteEvent { note: pad_note(i), velocity: if now { 100 } else { 0 } });
            }
            self.prev_pads[i] = now;
        }
        self.prev_midi.resize(128, 0);
        for k in 0..128 {
            let v = input.midi_keys.0[k];
            if (v > 0) != (self.prev_midi[k] > 0) {
                events.push(NoteEvent { note: k as u8, velocity: v });
            }
            self.prev_midi[k] = v;
        }
        if events.is_empty() {
            return;
        }
        for e in &events {
            self.held.retain(|&n| n != e.note);
            if e.velocity > 0 {
                self.held.push(e.note);
            }
        }
        self.queue.push(&events);
    }

    #[allow(dead_code)]
    pub fn pop(&self) -> Option<NoteEvent> {
        self.queue.pop()
    }

    pub fn held(&self) -> &[u8] {
        &self.held
    }
}

/// A module's CV outputs, each patchable to any app's mod input (the same
/// App / Input rows as Turing Machine's outputs).
pub struct CvOuts {
    #[allow(dead_code)]
    pub names: &'static [&'static str],
    /// 0 = not patched, else mod-bus index + 1.
    targets: Vec<AtomicUsize>,
}

impl CvOuts {
    pub fn new(names: &'static [&'static str]) -> CvOuts {
        CvOuts { names, targets: names.iter().map(|_| AtomicUsize::new(0)).collect() }
    }

    pub fn target(&self, i: usize) -> usize {
        self.targets[i].load(Ordering::Relaxed)
    }

    pub fn any(&self) -> bool {
        self.targets.iter().any(|t| t.load(Ordering::Relaxed) > 0)
    }

    pub fn app_label(&self, bus: &ModBus, i: usize) -> String {
        crate::modbus::Patch::app_label(bus, self.target(i))
    }

    pub fn input_label(&self, bus: &ModBus, i: usize) -> String {
        crate::modbus::Patch::input_label(bus, self.target(i))
    }

    pub fn step_app(&self, bus: &ModBus, i: usize, d: i32) {
        self.targets[i].store(crate::modbus::Patch::step_app(bus, self.target(i), d), Ordering::Relaxed);
    }

    pub fn step_input(&self, bus: &ModBus, i: usize, d: i32) {
        self.targets[i].store(crate::modbus::Patch::step_input(bus, self.target(i), d), Ordering::Relaxed);
    }

    pub fn clear(&self, i: usize) {
        self.targets[i].store(0, Ordering::Relaxed);
    }

    /// Writes output `i`'s value (0..1 or -1..1) into its patched input.
    pub fn send(&self, bus: &ModBus, i: usize, value: f32) {
        let t = self.target(i);
        if t > 0 {
            if let Some(h) = bus.get(t - 1) {
                h.set(value);
            }
        }
    }
}

/// The note a physical pad plays: chromatic from C3, bottom-left up,
/// shifted by octaves.
pub fn pad_note(pad: usize, octave: i32) -> u8 {
    (48 + crate::app::play_kit::pad_rank(pad) as i32 + octave * 12).clamp(0, 127) as u8
}

pub fn semitones(v: f32) -> String {
    format!("{v:+.0} st")
}

pub fn octaves(v: f32) -> String {
    format!("{:+}", v.round() as i32)
}

// ------------------------------------------------------------ app shell

use crate::app::play_kit::{self as kit, KitConfig, PlayHost, PlayKit};
use crate::app::App;
use crate::audio::AudioProcessor;
use crate::display::{FrameBuffer, HEIGHT, WIDTH};
use crate::paramlist::ParamList;
use crate::spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;

/// What a module app supplies; `MiApp` does the rest (play view, menu,
/// screen, pads as keys).
pub trait Module: 'static {
    const NAME: &'static str;
    /// Background, title ink, accent, dim -- every app has its own.
    const PALETTE: [Rgb565; 4];
    fn controls(&self) -> &Controls;
    fn kit_config(&self) -> KitConfig;
    /// Whether the KEYS layer exists (the pads play notes).
    fn plays_notes(&self) -> bool {
        true
    }
    /// Octave shift for the pads, if the panel has one.
    fn octave(&self) -> i32 {
        0
    }
    /// Menu rows after the panel's controls: (name, value).
    fn extra_rows(&self) -> Vec<(String, String)> {
        Vec::new()
    }
    fn edit_extra(&mut self, _i: usize, _delta: i32) {}
    fn reset_extra(&mut self, _i: usize) {}
    /// Once per frame, after the keys have been read.
    fn update(&mut self, _keys: &Keys) {}
    fn processor(&mut self, notes: Arc<NoteQueue>) -> Box<dyn AudioProcessor>;
    /// The play view's status line.
    fn line(&self, keys: &Keys) -> String {
        keys.held().iter().rev().take(4).map(|&n| crate::util::note_name(n as i32)).collect::<Vec<_>>().join(" ")
    }
    /// Bars for the right-hand panel: (label, 0..1 or -1..1).
    fn meters(&self) -> Vec<(String, f32)> {
        Vec::new()
    }
    fn background(&mut self) {}
    fn needs_background_audio(&self) -> bool {
        false
    }
    fn running(&self) -> Option<bool> {
        None
    }
    fn toggle_running(&mut self) {}
    fn hint(&self) -> &'static str {
        "L/R: dial (SELECT: next)   F2: pads   R1: menu"
    }
    /// The panel's headline: the model, mode or what's playing.
    #[allow(dead_code)] // Slint GUI only
    fn title(&self) -> String {
        Self::NAME.to_string()
    }
}

pub struct MiApp<M: Module> {
    pub m: M,
    pub kit: PlayKit,
    pub keys: Keys,
    list: ParamList,
    sensitivity: Arc<AtomicF32>,
    nav_speed: Arc<AtomicF32>,
}

impl<M: Module> MiApp<M> {
    pub fn new(m: M, sensitivity: Arc<AtomicF32>, nav_speed: Arc<AtomicF32>) -> Self {
        let kit = PlayKit::new(m.kit_config(), !cfg!(test));
        MiApp { m, kit, keys: Keys::default(), list: ParamList::new(), sensitivity, nav_speed }
    }

    fn rows(&self) -> Vec<(String, String)> {
        let c = self.m.controls();
        (0..c.len()).map(|i| (c.specs[i].name.to_string(), c.text(i))).chain(self.m.extra_rows()).collect()
    }

    fn edit_row(&mut self, i: usize, delta: i32) {
        let n = self.m.controls().len();
        if i < n {
            self.m.controls().edit(i, delta, self.sensitivity.get());
        } else {
            self.m.edit_extra(i - n, delta);
        }
    }

    fn reset_row(&mut self, i: usize) {
        let n = self.m.controls().len();
        if i < n {
            self.m.controls().reset(i);
        } else {
            self.m.reset_extra(i - n);
        }
    }
}

impl<M: Module> PlayHost for MiApp<M> {
    fn kit_control_count(&self) -> usize {
        self.m.controls().len()
    }
    fn kit_label(&self, i: usize) -> String {
        self.m.controls().specs[i].name.to_string()
    }
    fn kit_value(&self, i: usize) -> String {
        self.m.controls().text(i)
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        Some(self.m.controls().norm(i))
    }
    fn kit_stepped(&self, i: usize) -> bool {
        self.m.controls().specs[i].is_switch()
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.m.controls().edit(i, delta, self.sensitivity.get());
    }
    fn kit_reset(&mut self, i: usize) {
        self.m.controls().reset(i);
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        self.m.controls().set_norm(i, v);
    }
    fn kit_line(&self) -> String {
        self.m.line(&self.keys)
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        crate::util::note_name(pad_note(pad, self.m.octave()) as i32)
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, held: bool) -> crate::led_output::PadColor {
        use crate::led_output::PadColor;
        if held {
            PadColor::Green
        } else if pad_note(pad, self.m.octave()) % 12 == 0 {
            PadColor::Blue
        } else {
            PadColor::Off
        }
    }
}

impl<M: Module> App for MiApp<M> {
    fn instrument_settings(&self) -> Vec<crate::app::Setting> {
        crate::app::play_kit::settings_of(self)
    }
    fn adjust_setting(&mut self, index: usize, delta: i32) {
        crate::app::play_kit::adjust_in(self, index, delta)
    }
    fn supports_pad_lock(&self) -> bool {
        true
    }
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
    fn running(&self) -> Option<bool> {
        self.m.running()
    }
    fn toggle_running(&mut self) {
        self.m.toggle_running();
    }
    fn needs_background_audio(&self) -> bool {
        self.m.needs_background_audio()
    }
    fn background_tick(&mut self) {
        self.m.background();
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows().into_iter().map(|(a, b)| (a, b, false)).collect()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_windowed_rows(&mut self, visible: usize) -> (Vec<(String, String, bool)>, usize, bool, bool) {
        let rows = self.slint_rows();
        if rows.is_empty() || visible == 0 {
            return (rows, 0, false, false);
        }
        let (start, end) = self.list.centered_scroll_window(visible, rows.len());
        (rows[start..end].to_vec(), self.list.selected - start, start > 0, end < rows.len())
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        let meters = self.m.meters();
        let mut cells = Vec::new();
        for (label, v) in &meters {
            cells.push(label.clone());
            cells.push(format!("{:+.0}%", v * 100.0).trim_start_matches('+').to_string());
        }
        crate::app::SlintExtra::Grid(crate::app::GridExtra {
            caption: format!("MUTABLE INSTRUMENTS / {}", M::NAME.to_uppercase()),
            title: self.m.title(),
            cells,
            col_x: vec![0.0, 150.0],
            highlight: -1,
            footer: self.m.line(&self.keys),
            meter: -1.0,
        })
    }

    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let octave = self.m.octave();
        let pads_play = self.m.plays_notes() && step.native.is_some();
        // Notes from a keyboard or the note bus play at their own pitch,
        // whatever the pads are doing.
        let mut keyed = step.input;
        keyed.midi_keys = input.midi_keys;
        self.keys.update(&keyed, pads_play, |p| pad_note(p, octave));
        self.m.update(&self.keys);

        if step.menu {
            let input = &step.input;
            let n = self.rows().len();
            self.list.navigate_input(input, n, self.nav_speed.get() as i32);
            let sel = self.list.selected.min(n.saturating_sub(1));
            self.edit_row(sel, input.knob2);
            if input.knob2_press {
                self.reset_row(sel);
            }
        }
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(self.m.processor(Arc::clone(&self.keys.queue)))
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        let [bg, ink, accent, dim] = M::PALETTE;
        Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEIGHT as u32)).into_styled(PrimitiveStyle::with_fill(bg)).draw(fb).ok();
        Text::new(M::NAME, Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, ink)).draw(fb).ok();
        let small = MonoTextStyle::new(&SPLEEN_6X12, dim);
        if self.kit.menu {
            let rows = self.rows();
            self.list.draw_themed(fb, 16, 44, 24, 10, &rows, bg, dim, accent);
        } else if let Some(col) = self.play_column() {
            let pal = kit::draw::Palette { bg, ink, accent, dim, faint: dim };
            kit::draw::column(fb, &col, 16, 40, 350, 280, pal);
        }
        // Right: the module's outputs as bars.
        let meters = self.m.meters();
        let ink_style = MonoTextStyle::new(&SPLEEN_6X12, ink);
        for (k, (label, v)) in meters.iter().enumerate().take(12) {
            let y = 60 + k as i32 * 22;
            Text::new(label, Point::new(390, y + 9), ink_style).draw(fb).ok();
            Rectangle::new(Point::new(470, y), Size::new(150, 10)).into_styled(PrimitiveStyle::with_stroke(dim, 1)).draw(fb).ok();
            let v = v.clamp(-1.0, 1.0);
            let (x0, w) = if v >= 0.0 { (470, (v * 150.0) as i32) } else { (470 + 75 + (v * 75.0) as i32, (-v * 75.0) as i32) };
            if w > 0 {
                Rectangle::new(Point::new(x0, y + 1), Size::new(w as u32, 8)).into_styled(PrimitiveStyle::with_fill(accent)).draw(fb).ok();
            }
        }
        let hint = if self.kit.menu { "U/D: browse   L/R: change   hold SELECT: reset   R1: play" } else { self.m.hint() };
        Text::new(hint, Point::new(16, 337), small).draw(fb).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static SPECS: [Spec; 2] = [Spec::knob("Tone", 0.5), Spec::switch("Mode", &["A", "B", "C"], 1)];

    #[test]
    fn controls_edit_wrap_and_modulate() {
        let bus = ModBus::new();
        let c = Controls::new(&SPECS, "Test", &bus);
        assert_eq!(c.text(0), "50%");
        assert_eq!(c.text(1), "B");
        c.edit(1, 1, 0.1);
        c.edit(1, 1, 0.1);
        assert_eq!(c.text(1), "A", "switches wrap");
        let idx = bus.index_of("Test: Tone").expect("knobs are mod inputs");
        bus.get(idx).unwrap().set(0.25);
        assert!((c.get(0) - 0.75).abs() < 1e-6, "modulation adds to the knob");
        assert!((c.raw(0) - 0.5).abs() < 1e-6, "without moving it");
        assert!(bus.index_of("Test: Mode").is_none(), "switches aren't");
    }

    #[test]
    fn the_rate_bridge_keeps_time_and_passes_input() {
        // A 32 kHz "engine" that echoes its input and counts blocks.
        let mut b = RateBridge::new(32_000.0, 16);
        let mut blocks = 0;
        let input: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut out = vec![[0.0f32; 2]; 4800];
        b.process(&input, &mut out, 48_000.0, |inp, o| {
            blocks += 1;
            for (k, x) in inp.iter().enumerate() {
                o[k] = [*x, *x];
            }
        });
        // 0.1 s at 32 kHz is 3200 engine samples: 200 blocks.
        assert!((199..=201).contains(&blocks), "{blocks}");
        // The echo is the input, one engine block late.
        let lat = RateBridge::latency(16, 1.5);
        let err: f32 = (100..4700).map(|i| (out[i][0] - input[i - lat]).abs()).fold(0.0, f32::max);
        assert!(err < 0.05, "input survives the round trip: {err}");
    }

    #[test]
    fn keys_track_pads_and_midi() {
        let mut k = Keys::default();
        let mut input = Input::default();
        input.grid[12] = true;
        k.update(&input, true, |p| pad_note(p, 0));
        assert_eq!(k.pop(), Some(NoteEvent { note: 48, velocity: 100 }), "bottom-left pad is C3");
        input.midi_keys.0[64] = 90;
        k.update(&input, true, |p| pad_note(p, 0));
        assert_eq!(k.pop(), Some(NoteEvent { note: 64, velocity: 90 }));
        assert_eq!(k.held(), &[48, 64]);
        input.grid[12] = false;
        k.update(&input, true, |p| pad_note(p, 0));
        assert_eq!(k.pop(), Some(NoteEvent { note: 48, velocity: 0 }));
        assert_eq!(k.held(), &[64]);
    }
}
