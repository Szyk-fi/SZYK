//! Trio: three layered voices, each running its own Plaits engine, that
//! split up what you play. Inspired by the idea of Polyend's Synth (three
//! engine layers with chord playing and followers); the roles and layout
//! here are Portamax's own, and the sound is the real Plaits DSP that
//! Portamax already ports (plaits_ffi.rs).
//!
//! The pads play the scale (bottom-left = root, rising left to right, row
//! by row). Chord mode turns each pad into a diatonic chord built on that
//! degree. Each layer then takes a role:
//! - Full: plays everything you hold
//! - Bass: only the lowest note, an octave down
//! - Top: only the highest note
//! - Arp: arpeggiates what you hold, in time
//! - Off
//!
//! So one pad can be a pad chord on layer A, a bassline on B and an
//! arpeggio on C. Each layer has its engine, Harmonics/Timbre/Morph,
//! decay, level and octave; up to four voices per layer.

use crate::{
    app::music_scales::{self, ROOT_NAMES, SCALE_TYPES},
    app::play_kit::{self as kit, KitConfig, Knob, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    display::FrameBuffer,
    led_output::PadColor,
    mixer_bus::MixerBus,
    modbus::ModBus,
    paramlist::ParamList,
    plaits_ffi::{PlaitsParams, PlaitsVoice},
    spleen_fonts::{SPLEEN_6X12, SPLEEN_8X16},
    util::AtomicF32,
};
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, text::Text};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "Trio";
const LAYERS: usize = 3;
const VOICES: usize = 4;
/// Seconds a released voice keeps rendering (its decay tail).
const TAIL_S: f32 = 4.0;

// Own palette: three inks on deep plum.
const BG: Rgb565 = Rgb565::new(5, 4, 7);
const INK: Rgb565 = Rgb565::new(29, 54, 28);
const ACCENT: Rgb565 = Rgb565::new(31, 26, 20);
const DIM: Rgb565 = Rgb565::new(14, 22, 16);
const FAINT: Rgb565 = Rgb565::new(7, 8, 10);
const LAYER_INK: [Rgb565; 3] = [Rgb565::new(31, 34, 12), Rgb565::new(10, 48, 30), Rgb565::new(24, 30, 31)];

pub const ROLES: [&str; 5] = ["Full", "Bass", "Top", "Arp", "Off"];
const ROLE_FULL: usize = 0;
const ROLE_BASS: usize = 1;
const ROLE_TOP: usize = 2;
const ROLE_ARP: usize = 3;
const ROLE_OFF: usize = 4;

/// Chord shapes, as scale-degree offsets stacked on the played degree.
pub const CHORDS: [(&str, &[usize]); 6] = [("Single", &[0]), ("Triad", &[0, 2, 4]), ("Seventh", &[0, 2, 4, 6]), ("Sus", &[0, 3, 4]), ("Power", &[0, 4, 7]), ("Stack", &[0, 3, 6, 9])];
pub const ARP_RATES: [(&str, f32); 5] = [("1/4", 1.0), ("1/8", 0.5), ("1/8T", 1.0 / 3.0), ("1/16", 0.25), ("1/32", 0.125)];
pub const ARP_PATTERNS: [&str; 4] = ["Up", "Down", "Up/Down", "Random"];

struct LayerShared {
    role: AtomicUsize,
    engine: AtomicU32,
    harmonics: AtomicF32,
    timbre: AtomicF32,
    morph: AtomicF32,
    decay: AtomicF32,
    level: AtomicF32,
    octave: AtomicUsize, // 0..4 = -2..+2
    cv: [Arc<AtomicF32>; 2],
    active: AtomicUsize,
}

impl LayerShared {
    fn new(role: usize, engine: u32, octave: usize, cv: [Arc<AtomicF32>; 2]) -> Self {
        Self {
            role: AtomicUsize::new(role),
            engine: AtomicU32::new(engine),
            harmonics: AtomicF32::new(0.5),
            timbre: AtomicF32::new(0.5),
            morph: AtomicF32::new(0.5),
            decay: AtomicF32::new(0.6),
            level: AtomicF32::new(0.7),
            octave: AtomicUsize::new(octave),
            cv,
            active: AtomicUsize::new(0),
        }
    }
}

struct Shared {
    layers: [LayerShared; LAYERS],
    scale: AtomicUsize,
    root: AtomicUsize,
    chord: AtomicUsize,
    bpm: AtomicF32,
    arp_rate: AtomicUsize,
    arp_pattern: AtomicUsize,
    arp_octaves: AtomicUsize,
    focus: AtomicUsize,
    /// The note the arpeggio is on (255 = none), for display.
    arp_now: AtomicUsize,
    /// MIDI notes currently held (after chord expansion), bitset.
    held: [AtomicBool; 128],
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
}

impl Shared {
    fn held_notes(&self) -> Vec<u8> {
        (0..128u8).filter(|&n| self.held[n as usize].load(Ordering::Relaxed)).collect()
    }
}

// ------------------------------------------------------------ controls

const CONTROLS: [&str; 16] = ["Timbre", "Morph", "Harmonics", "Decay", "Layer", "Engine", "Role", "Level", "Octave", "Chord", "Scale", "Root", "Arp Rate", "Arp Pattern", "Tempo", "Arp Octaves"];
const C_TIMBRE: usize = 0;
const C_MORPH: usize = 1;
const C_HARM: usize = 2;
const C_DECAY: usize = 3;
const C_LAYER: usize = 4;
const C_ENGINE: usize = 5;
const C_ROLE: usize = 6;
const C_LEVEL: usize = 7;
const C_OCTAVE: usize = 8;
const C_CHORD: usize = 9;
const C_SCALE: usize = 10;
const C_ROOT: usize = 11;
const C_ARP_RATE: usize = 12;
const C_ARP_PAT: usize = 13;
const C_TEMPO: usize = 14;
const C_ARP_OCT: usize = 15;

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "trio",
        layers: vec![Layer::Native(0, "PLAY"), Layer::Controls, Layer::Moments],
        hero: vec![[C_TIMBRE, C_MORPH], [C_HARM, C_DECAY], [C_LAYER, C_ENGINE], [C_ROLE, C_LEVEL]],
        browse: Some(C_LAYER),
        routes: Routes { stick_x: Some(C_TIMBRE), stick_y: Some(C_MORPH), hand_l: Some(C_HARM), hand_r: Some(C_DECAY) },
        throws: Vec::new(),
        midi_to_pads: true,
        own_expression: false,
    }
}

pub struct TrioApp {
    p: Arc<Shared>,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    kit: PlayKit,
    prev_grid: [bool; 16],
    /// Notes each pad is holding (chord-expanded), to release exactly.
    pad_notes: [Vec<u8>; 16],
}

/// Pad (row-major, row 0 at the top) -> scale position (0 = bottom-left).
fn pad_position(pad: usize) -> usize {
    (3 - pad / 4) * 4 + pad % 4
}

impl TrioApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let names = ["A", "B", "C"];
        let cv = |i: usize| [mods.register(format!("{APP_NAME}: {} Timbre", names[i])), mods.register(format!("{APP_NAME}: {} Morph", names[i]))];
        // A: string-machine pad chords; B: virtual-analog bass; C: an
        // FM arpeggio. (Engines are indices into plaits::ENGINE_NAMES.)
        let layers = [LayerShared::new(ROLE_FULL, 6, 2, cv(0)), LayerShared::new(ROLE_BASS, 8, 2, cv(1)), LayerShared::new(ROLE_ARP, 10, 3, cv(2))];
        Self {
            p: Arc::new(Shared {
                layers,
                scale: AtomicUsize::new(2),
                root: AtomicUsize::new(9),
                chord: AtomicUsize::new(1),
                bpm: AtomicF32::new(110.0),
                arp_rate: AtomicUsize::new(3),
                arp_pattern: AtomicUsize::new(0),
                arp_octaves: AtomicUsize::new(1),
                focus: AtomicUsize::new(0),
                arp_now: AtomicUsize::new(255),
                held: std::array::from_fn(|_| AtomicBool::new(false)),
                mix_level,
                ext_mix_level,
                output,
            }),
            list: ParamList::new(),
            nav,
            sensitivity,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            prev_grid: [false; 16],
            pad_notes: std::array::from_fn(|_| Vec::new()),
        }
    }

    fn focus(&self) -> usize {
        self.p.focus.load(Ordering::Relaxed).min(LAYERS - 1)
    }

    fn layer(&self) -> &LayerShared {
        &self.p.layers[self.focus()]
    }

    /// The notes one pad plays: its degree, chord-stacked in the scale.
    fn pad_chord(&self, pad: usize) -> Vec<u8> {
        let scale = self.p.scale.load(Ordering::Relaxed);
        let root = 48 + self.p.root.load(Ordering::Relaxed) as i32;
        let pos = pad_position(pad);
        let shape = CHORDS[self.p.chord.load(Ordering::Relaxed).min(CHORDS.len() - 1)].1;
        shape.iter().map(|o| (root + music_scales::degree(scale, pos + o)).clamp(0, 127) as u8).collect()
    }

    fn press(&mut self, pad: usize) {
        let notes = self.pad_chord(pad);
        for &n in &notes {
            self.p.held[n as usize].store(true, Ordering::Relaxed);
        }
        self.pad_notes[pad] = notes;
    }

    fn release(&mut self, pad: usize) {
        let notes = std::mem::take(&mut self.pad_notes[pad]);
        for n in notes {
            // another held pad may share the note
            if !self.pad_notes.iter().any(|v| v.contains(&n)) {
                self.p.held[n as usize].store(false, Ordering::Relaxed);
            }
        }
    }

    fn release_all(&mut self) {
        for pad in 0..16 {
            self.release(pad);
        }
        self.prev_grid = [false; 16];
    }

    fn handle_pads(&mut self, grid: &[bool; 16]) {
        for i in 0..16 {
            if grid[i] && !self.prev_grid[i] {
                self.press(i);
            } else if !grid[i] && self.prev_grid[i] {
                self.release(i);
            }
        }
        self.prev_grid = *grid;
    }

    fn value(&self, c: usize) -> String {
        let l = self.layer();
        let pct = |a: &AtomicF32| format!("{:.0}%", a.get() * 100.0);
        match c {
            C_TIMBRE => pct(&l.timbre),
            C_MORPH => pct(&l.morph),
            C_HARM => pct(&l.harmonics),
            C_DECAY => pct(&l.decay),
            C_LAYER => format!("{} · {}", ["A", "B", "C"][self.focus()], ROLES[l.role.load(Ordering::Relaxed).min(ROLES.len() - 1)]),
            C_ENGINE => crate::apps::plaits::ENGINE_NAMES[l.engine.load(Ordering::Relaxed) as usize % crate::apps::plaits::ENGINE_NAMES.len()].into(),
            C_ROLE => ROLES[l.role.load(Ordering::Relaxed).min(ROLES.len() - 1)].into(),
            C_LEVEL => pct(&l.level),
            C_OCTAVE => format!("{:+}", l.octave.load(Ordering::Relaxed) as i32 - 2),
            C_CHORD => CHORDS[self.p.chord.load(Ordering::Relaxed).min(CHORDS.len() - 1)].0.into(),
            C_SCALE => SCALE_TYPES[self.p.scale.load(Ordering::Relaxed) % SCALE_TYPES.len()].0.into(),
            C_ROOT => ROOT_NAMES[self.p.root.load(Ordering::Relaxed) % 12].into(),
            C_ARP_RATE => ARP_RATES[self.p.arp_rate.load(Ordering::Relaxed).min(ARP_RATES.len() - 1)].0.into(),
            C_ARP_PAT => ARP_PATTERNS[self.p.arp_pattern.load(Ordering::Relaxed).min(ARP_PATTERNS.len() - 1)].into(),
            C_TEMPO => format!("{:.0} BPM", self.p.bpm.get()),
            _ => format!("{}", self.p.arp_octaves.load(Ordering::Relaxed)),
        }
    }

    fn label(&self, c: usize) -> String {
        let names = crate::apps::plaits::engine_param_names(self.layer().engine.load(Ordering::Relaxed) as usize);
        match c {
            C_HARM => names[0].into(),
            C_TIMBRE => names[1].into(),
            C_MORPH => names[2].into(),
            _ => CONTROLS[c].into(),
        }
    }

    fn edit(&mut self, c: usize, d: i32) {
        if d == 0 {
            return;
        }
        let sens = self.sensitivity.get().max(0.01) * 10.0;
        let p = Arc::clone(&self.p);
        let l = &p.layers[self.focus()];
        let nudge = |a: &AtomicF32| a.set((a.get() + d as f32 * 0.01 * sens).clamp(0.0, 1.0));
        let cyc = |a: &AtomicUsize, n: usize| a.store((a.load(Ordering::Relaxed) as i32 + d.signum()).rem_euclid(n as i32) as usize, Ordering::Relaxed);
        let clampu = |a: &AtomicUsize, lo: usize, hi: usize| a.store((a.load(Ordering::Relaxed) as i32 + d.signum()).clamp(lo as i32, hi as i32) as usize, Ordering::Relaxed);
        match c {
            C_TIMBRE => nudge(&l.timbre),
            C_MORPH => nudge(&l.morph),
            C_HARM => nudge(&l.harmonics),
            C_DECAY => nudge(&l.decay),
            C_LEVEL => nudge(&l.level),
            C_LAYER => cyc(&p.focus, LAYERS),
            C_ENGINE => l.engine.store((l.engine.load(Ordering::Relaxed) as i32 + d.signum()).rem_euclid(crate::apps::plaits::ENGINE_NAMES.len() as i32) as u32, Ordering::Relaxed),
            C_ROLE => cyc(&l.role, ROLES.len()),
            C_OCTAVE => clampu(&l.octave, 0, 4),
            C_CHORD | C_SCALE | C_ROOT => {
                // changing the harmony mid-hold would strand notes
                self.release_all();
                match c {
                    C_CHORD => cyc(&p.chord, CHORDS.len()),
                    C_SCALE => cyc(&p.scale, SCALE_TYPES.len()),
                    _ => cyc(&p.root, 12),
                }
            }
            C_ARP_RATE => clampu(&p.arp_rate, 0, ARP_RATES.len() - 1),
            C_ARP_PAT => cyc(&p.arp_pattern, ARP_PATTERNS.len()),
            C_TEMPO => p.bpm.set((p.bpm.get() + d as f32).clamp(40.0, 240.0)),
            _ => clampu(&p.arp_octaves, 1, 3),
        }
    }

    /// Menu: the shared rows, then the focused layer's.
    fn rows(&self) -> Vec<(String, String, bool)> {
        let mut r = Vec::new();
        for c in [C_LAYER, C_CHORD, C_SCALE, C_ROOT, C_TEMPO, C_ARP_RATE, C_ARP_PAT, C_ARP_OCT, C_ROLE, C_ENGINE, C_HARM, C_TIMBRE, C_MORPH, C_DECAY, C_LEVEL, C_OCTAVE] {
            let indent = if matches!(c, C_ROLE | C_ENGINE | C_HARM | C_TIMBRE | C_MORPH | C_DECAY | C_LEVEL | C_OCTAVE) { "  " } else { "" };
            r.push((format!("{indent}{}", self.label(c)), self.value(c), false));
        }
        r
    }

    fn row_control(row: usize) -> usize {
        [C_LAYER, C_CHORD, C_SCALE, C_ROOT, C_TEMPO, C_ARP_RATE, C_ARP_PAT, C_ARP_OCT, C_ROLE, C_ENGINE, C_HARM, C_TIMBRE, C_MORPH, C_DECAY, C_LEVEL, C_OCTAVE][row.min(15)]
    }

    fn knob(&self, c: usize) -> Knob<'_> {
        let l = self.layer();
        match c {
            C_TIMBRE => Knob::F(&l.timbre, 0.0, 1.0),
            C_MORPH => Knob::F(&l.morph, 0.0, 1.0),
            C_HARM => Knob::F(&l.harmonics, 0.0, 1.0),
            C_DECAY => Knob::F(&l.decay, 0.0, 1.0),
            C_LEVEL => Knob::F(&l.level, 0.0, 1.0),
            C_ENGINE => Knob::U(&l.engine, crate::apps::plaits::ENGINE_NAMES.len() as u32),
            C_TEMPO => Knob::F(&self.p.bpm, 40.0, 240.0),
            _ => Knob::None,
        }
    }

    fn usize_ctl(&self, c: usize) -> Option<(&AtomicUsize, usize, usize)> {
        let l = self.layer();
        Some(match c {
            C_LAYER => (&self.p.focus, 0, LAYERS),
            C_ROLE => (&l.role, 0, ROLES.len()),
            C_OCTAVE => (&l.octave, 0, 5),
            C_CHORD => (&self.p.chord, 0, CHORDS.len()),
            C_SCALE => (&self.p.scale, 0, SCALE_TYPES.len()),
            C_ROOT => (&self.p.root, 0, 12),
            C_ARP_RATE => (&self.p.arp_rate, 0, ARP_RATES.len()),
            C_ARP_PAT => (&self.p.arp_pattern, 0, ARP_PATTERNS.len()),
            C_ARP_OCT => (&self.p.arp_octaves, 1, 4),
            _ => return None,
        })
    }
}

impl PlayHost for TrioApp {
    fn kit_control_count(&self) -> usize {
        CONTROLS.len()
    }
    fn kit_label(&self, i: usize) -> String {
        self.label(i % CONTROLS.len())
    }
    fn kit_value(&self, i: usize) -> String {
        self.value(i % CONTROLS.len())
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        if let Some((a, lo, n)) = self.usize_ctl(i) {
            let span = (n - lo).max(2) - 1;
            return Some((a.load(Ordering::Relaxed).saturating_sub(lo)).min(span) as f32 / span as f32);
        }
        self.knob(i).norm()
    }
    fn kit_stepped(&self, i: usize) -> bool {
        self.usize_ctl(i).is_some() || i == C_ENGINE
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit(i, delta);
    }
    fn kit_reset(&mut self, i: usize) {
        let l = self.layer();
        match i {
            C_TIMBRE | C_MORPH | C_HARM => self.knob(i).set(0.5),
            C_DECAY => l.decay.set(0.6),
            C_LEVEL => l.level.set(0.7),
            C_OCTAVE => l.octave.store(2, Ordering::Relaxed),
            C_TEMPO => self.p.bpm.set(110.0),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let harmony = matches!(i, C_CHORD | C_SCALE | C_ROOT);
        let change = self.usize_ctl(i).map(|(a, lo, n)| {
            let span = (n - lo).max(2) - 1;
            let next = lo + (v.clamp(0.0, 1.0) * span as f32).round() as usize;
            (next, next != a.load(Ordering::Relaxed))
        });
        if let Some((next, differs)) = change {
            if harmony && differs {
                self.release_all();
            }
            if let Some((a, _, _)) = self.usize_ctl(i) {
                a.store(next, Ordering::Relaxed);
            }
        } else {
            self.knob(i).set(v);
        }
    }
    /// Pads are pitched: a MIDI note presses the pad whose root it is.
    fn kit_midi_pad(&self, note: u8) -> Option<usize> {
        (0..16).find(|&p| self.pad_chord(p).first() == Some(&note)).or_else(|| (0..16).find(|&p| self.pad_chord(p).first().is_some_and(|r| r % 12 == note % 12)))
    }
    fn kit_pad_label(&self, _layer: u8, pad: usize) -> String {
        let n = self.pad_chord(pad);
        let name = |m: u8| format!("{}{}", ROOT_NAMES[m as usize % 12], m as i32 / 12 - 1);
        match n.as_slice() {
            [one] => name(*one),
            [root, ..] => format!("{}{}", name(*root), if n.len() > 1 { "+" } else { "" }),
            [] => String::new(),
        }
    }
    fn kit_pad_color(&self, _layer: u8, pad: usize, _held: bool) -> PadColor {
        if !self.pad_notes[pad].is_empty() {
            PadColor::Green
        } else if pad_position(pad) % SCALE_TYPES[self.p.scale.load(Ordering::Relaxed) % SCALE_TYPES.len()].1.len() == 0 {
            PadColor::Blue // each root
        } else {
            PadColor::Off
        }
    }
    fn kit_line(&self) -> String {
        let roles: Vec<String> = self
            .p
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}:{}", ["A", "B", "C"][i], ROLES[l.role.load(Ordering::Relaxed).min(ROLES.len() - 1)]))
            .collect();
        format!("{}  {}", roles.join(" "), self.value(C_CHORD))
    }
}

impl App for TrioApp {
    fn play_surface(&self) -> bool {
        true
    }
    fn supports_pad_lock(&self) -> bool {
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
    fn grid_led_overlay(&self) -> [PadColor; 16] {
        self.kit.led_overlay(self)
    }
    fn tick(&mut self, input: &Input) {
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        let input = &step.input;
        // pads play on PLAY (and in the menu); kit layers hand us an empty grid
        self.handle_pads(&input.grid);
        self.list.navigate_input(input, 16, self.nav.get() as i32);
        self.edit(Self::row_control(self.list.selected), input.knob2);
    }
    fn on_exit(&mut self) {
        self.release_all();
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
        let small = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        for (i, l) in self.p.layers.iter().enumerate() {
            let y = 60 + i as i32 * 80;
            let ink = if i == self.focus() { LAYER_INK[i] } else { DIM };
            let big = MonoTextStyle::new(&SPLEEN_8X16, ink);
            let engine = crate::apps::plaits::ENGINE_NAMES[l.engine.load(Ordering::Relaxed) as usize % crate::apps::plaits::ENGINE_NAMES.len()];
            Text::new(&format!("{}  {}", ["A", "B", "C"][i], ROLES[l.role.load(Ordering::Relaxed).min(ROLES.len() - 1)]), Point::new(392, y), big).draw(f).ok();
            Text::new(engine, Point::new(392, y + 18), MonoTextStyle::new(&SPLEEN_6X12, ink)).draw(f).ok();
            let act = l.active.load(Ordering::Relaxed);
            for v in 0..VOICES {
                let c = if v < act { LAYER_INK[i] } else { FAINT };
                embedded_graphics::primitives::Rectangle::new(Point::new(392 + v as i32 * 14, y + 26), Size::new(10, 10))
                    .into_styled(embedded_graphics::primitives::PrimitiveStyle::with_fill(c))
                    .draw(f)
                    .ok();
            }
        }
        Text::new("pads: scale/chords   D-pad: layer   F2: pads   R1: menu", Point::new(16, 340), small).draw(f).ok();
    }
    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        self.rows()
    }
    fn slint_selected(&self) -> usize {
        self.list.selected
    }
    fn slint_scale_info(&self) -> Option<music_scales::ScaleInfo> {
        Some(music_scales::ScaleInfo::new(self.p.scale.load(Ordering::Relaxed), self.p.root.load(Ordering::Relaxed) as i32))
    }
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        Some(Box::new(Processor::new(Arc::clone(&self.p))))
    }
}

// ------------------------------------------------------------------ DSP

struct VoiceState {
    voice: PlaitsVoice,
    note: u8,
    gate: bool,
    /// Seconds since release (rendering stops after TAIL_S).
    since_off: f32,
    age: u64,
}

struct LayerDsp {
    voices: Vec<VoiceState>,
    buf: Vec<f32>,
    clock: u64,
}

impl LayerDsp {
    fn new() -> Self {
        Self {
            voices: (0..VOICES).map(|_| VoiceState { voice: PlaitsVoice::new(), note: 0, gate: false, since_off: TAIL_S, age: 0 }).collect(),
            buf: vec![0.0; 4096],
            clock: 0,
        }
    }
    fn note_on(&mut self, n: u8) {
        if self.voices.iter().any(|v| v.gate && v.note == n) {
            return;
        }
        self.clock += 1;
        let i = self
            .voices
            .iter()
            .position(|v| !v.gate && v.since_off >= TAIL_S)
            .or_else(|| self.voices.iter().enumerate().filter(|(_, v)| !v.gate).max_by(|a, b| a.1.since_off.total_cmp(&b.1.since_off)).map(|(i, _)| i))
            .unwrap_or_else(|| (0..VOICES).min_by_key(|&i| self.voices[i].age).unwrap_or(0));
        let v = &mut self.voices[i];
        v.note = n;
        v.gate = true;
        v.since_off = 0.0;
        v.age = self.clock;
    }
    fn note_off(&mut self, n: u8) {
        for v in self.voices.iter_mut().filter(|v| v.gate && v.note == n) {
            v.gate = false;
            v.since_off = 0.0;
        }
    }
    fn all_off(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.gate) {
            v.gate = false;
            v.since_off = 0.0;
        }
    }
    /// The notes this layer should be holding now (not used by Arp).
    fn sync(&mut self, want: &[u8]) {
        let playing: Vec<u8> = self.voices.iter().filter(|v| v.gate).map(|v| v.note).collect();
        for n in playing.iter().filter(|n| !want.contains(n)) {
            self.note_off(*n);
        }
        for n in want.iter().filter(|n| !playing.contains(n)) {
            self.note_on(*n);
        }
    }
}

struct Processor {
    p: Arc<Shared>,
    layers: Vec<LayerDsp>,
    /// Beats since the arp started.
    beat: f64,
    arp_step: i64,
    arp_index: usize,
    arp_dir: i32,
    arp_note: Option<u8>,
    held_before: Vec<u8>,
    rng: u32,
    mix: Vec<f32>,
}

impl Processor {
    fn new(p: Arc<Shared>) -> Self {
        Self { p, layers: (0..LAYERS).map(|_| LayerDsp::new()).collect(), beat: 0.0, arp_step: -1, arp_index: 0, arp_dir: 1, arp_note: None, held_before: Vec::new(), rng: 0x2545_f491, mix: vec![0.0; 4096] }
    }

    fn rand(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as usize % n.max(1)
    }

    /// The arpeggio's note list: held notes across the octave range.
    fn arp_pool(&self, held: &[u8]) -> Vec<u8> {
        let oct = self.p.arp_octaves.load(Ordering::Relaxed).clamp(1, 3);
        let mut v = Vec::with_capacity(held.len() * oct);
        for o in 0..oct {
            for &n in held {
                let m = n as usize + o * 12;
                if m < 128 {
                    v.push(m as u8);
                }
            }
        }
        v
    }

    fn next_arp(&mut self, pool: &[u8]) -> Option<u8> {
        if pool.is_empty() {
            return None;
        }
        let n = pool.len();
        let i = match self.p.arp_pattern.load(Ordering::Relaxed) {
            0 => {
                self.arp_index = (self.arp_index + 1) % n;
                self.arp_index
            }
            1 => {
                self.arp_index = (self.arp_index + n - 1) % n;
                self.arp_index
            }
            2 => {
                if n == 1 {
                    0
                } else {
                    let next = self.arp_index as i32 + self.arp_dir;
                    if next < 0 || next >= n as i32 {
                        self.arp_dir = -self.arp_dir;
                    }
                    self.arp_index = (self.arp_index as i32 + self.arp_dir).clamp(0, n as i32 - 1) as usize;
                    self.arp_index
                }
            }
            _ => self.rand(n),
        };
        pool.get(i).copied()
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        if channels == 0 || rate < 1000.0 {
            return;
        }
        let frames = out.len() / channels;
        if self.mix.len() < frames {
            return; // larger than any real block; stay silent rather than allocate
        }
        let held = self.p.held_notes();
        // what each role wants
        let lowest = held.first().copied();
        let highest = held.last().copied();
        // arp clock (block-rate is enough at these divisions: 5 ms jitter max)
        let bpm = self.p.bpm.get().clamp(40.0, 240.0);
        let div = ARP_RATES[self.p.arp_rate.load(Ordering::Relaxed).min(ARP_RATES.len() - 1)].1 as f64;
        if held.is_empty() {
            self.beat = 0.0;
            self.arp_step = -1;
            self.arp_note = None;
        } else {
            if self.held_before.is_empty() {
                // first key: start the arp on the beat, from the bottom
                self.arp_index = usize::MAX;
            }
            let step = (self.beat / div).floor() as i64;
            if step != self.arp_step {
                self.arp_step = step;
                let pool = self.arp_pool(&held);
                if self.arp_index >= pool.len() && self.arp_index != usize::MAX {
                    self.arp_index %= pool.len().max(1);
                }
                if self.arp_index == usize::MAX {
                    self.arp_index = if self.p.arp_pattern.load(Ordering::Relaxed) == 1 { 0 } else { pool.len() - 1 };
                }
                self.arp_note = self.next_arp(&pool);
                self.p.arp_now.store(self.arp_note.map_or(255, |n| n as usize), Ordering::Relaxed);
                // retrigger: the arp layers re-strike on every step
                for (li, l) in self.layers.iter_mut().enumerate() {
                    if self.p.layers[li].role.load(Ordering::Relaxed) == ROLE_ARP {
                        l.all_off();
                    }
                }
            }
            self.beat += (bpm as f64 / 60.0) * frames as f64 / rate as f64;
        }
        self.held_before = held.clone();
        let level = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0);
        self.mix[..frames].fill(0.0);
        let dt = frames as f32 / rate;
        for li in 0..LAYERS {
            let ls = &self.p.layers[li];
            let role = ls.role.load(Ordering::Relaxed);
            let oct = (ls.octave.load(Ordering::Relaxed) as i32 - 2) * 12;
            let shift = |n: u8, extra: i32| (n as i32 + oct + extra).clamp(0, 127) as u8;
            let want: Vec<u8> = match role {
                ROLE_FULL => held.iter().map(|&n| shift(n, 0)).collect(),
                ROLE_BASS => lowest.map(|n| shift(n, -12)).into_iter().collect(),
                ROLE_TOP => highest.map(|n| shift(n, 0)).into_iter().collect(),
                ROLE_ARP => self.arp_note.map(|n| shift(n, 0)).into_iter().collect(),
                _ => Vec::new(),
            };
            let l = &mut self.layers[li];
            if role == ROLE_ARP && !want.is_empty() {
                // a struck arp note: after the retrigger's all_off, start it
                if !l.voices.iter().any(|v| v.gate) {
                    l.note_on(want[0]);
                }
            } else {
                l.sync(&want);
            }
            let params = |note: u8, gate: bool| PlaitsParams {
                engine: ls.engine.load(Ordering::Relaxed) as i32,
                note: note as f32,
                harmonics: ls.harmonics.get(),
                timbre: (ls.timbre.get() + ls.cv[0].get()).clamp(0.0, 1.0),
                morph: (ls.morph.get() + ls.cv[1].get()).clamp(0.0, 1.0),
                decay: ls.decay.get(),
                lpg_colour: 0.5,
                trigger: gate,
            };
            let gain = ls.level.get() * 0.35;
            let mut active = 0;
            for v in l.voices.iter_mut() {
                if !v.gate && v.since_off >= TAIL_S {
                    continue;
                }
                if v.gate {
                    active += 1;
                } else {
                    v.since_off += dt;
                }
                if role == ROLE_OFF && !v.gate {
                    continue;
                }
                v.voice.render(&mut l.buf[..frames], rate, &params(v.note, v.gate));
                for (m, s) in self.mix[..frames].iter_mut().zip(&l.buf[..frames]) {
                    *m += s * gain;
                }
            }
            ls.active.store(active, Ordering::Relaxed);
        }
        for (frame, m) in out.chunks_mut(channels).zip(&self.mix[..frames]) {
            let y = (m * level).tanh();
            for c in frame.iter_mut() {
                *c += y;
            }
        }
        if let Ok(mut b) = self.p.output.try_lock() {
            b.clear();
            b.extend(self.mix[..frames].iter().map(|m| (m * level).tanh()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> TrioApp {
        TrioApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
    }

    fn pads(on: &[usize]) -> Input {
        Input { grid: std::array::from_fn(|k| on.contains(&k)), ..Default::default() }
    }

    fn render(p: &mut Box<dyn AudioProcessor>, blocks: usize, a: &TrioApp) -> (f32, Vec<usize>) {
        let mut e = 0.0;
        let mut act = vec![0; LAYERS];
        for _ in 0..blocks {
            let mut out = vec![0.0f32; 512];
            p.process(&mut out, 2, 48_000.0);
            assert!(out.iter().all(|v| v.is_finite()));
            e += out.iter().map(|v| v * v).sum::<f32>();
            for (i, l) in a.p.layers.iter().enumerate() {
                act[i] = act[i].max(l.active.load(Ordering::Relaxed));
            }
        }
        (e, act)
    }

    #[test]
    fn a_pad_plays_a_triad_and_the_roles_split_it() {
        let mut a = app();
        let bottom_left = 12;
        let chord = a.pad_chord(bottom_left);
        assert_eq!(chord.len(), 3, "Triad mode: three notes");
        // A minor (root A, minor scale): A C E
        assert_eq!(chord.iter().map(|n| n % 12).collect::<Vec<_>>(), vec![9, 0, 4]);
        let mut p = a.audio_processor().unwrap();
        a.tick(&pads(&[bottom_left]));
        let (e, act) = render(&mut p, 60, &a);
        assert!(e > 0.1, "it sounds ({e})");
        assert_eq!(act, vec![3, 1, 1], "Full holds the triad, Bass one note, Arp one at a time");
        a.tick(&Input::default());
        render(&mut p, 5, &a);
        assert!(a.p.layers.iter().all(|l| l.active.load(Ordering::Relaxed) == 0), "release lets go of everything");
    }

    #[test]
    fn the_arp_walks_the_held_notes_in_time() {
        let mut a = app();
        for l in &a.p.layers {
            l.role.store(ROLE_OFF, Ordering::Relaxed);
        }
        a.p.layers[2].role.store(ROLE_ARP, Ordering::Relaxed);
        a.p.bpm.set(240.0); // sixteenths every 62.5 ms
        let mut p = a.audio_processor().unwrap();
        a.tick(&pads(&[12]));
        let mut notes = std::collections::BTreeSet::new();
        for _ in 0..120 {
            render(&mut p, 1, &a);
            let n = a.p.arp_now.load(Ordering::Relaxed);
            if n < 128 {
                notes.insert(n % 12);
            }
        }
        assert_eq!(notes.len(), 3, "all three chord tones arpeggiated: {notes:?}");
    }

    #[test]
    fn chord_and_scale_changes_dont_strand_notes() {
        let mut a = app();
        a.tick(&pads(&[12, 13]));
        assert!(!a.p.held_notes().is_empty());
        a.kit.menu = true;
        a.list.selected = 1; // Chord row
        a.tick(&Input { knob2: 1, grid: pads(&[12, 13]).grid, ..Default::default() });
        // the change released everything; still-held pads re-strike on their next press
        assert!(a.p.held_notes().len() <= 8);
        a.tick(&Input { knob2: 1, ..Default::default() });
        assert!(a.p.held_notes().is_empty(), "nothing left hanging");
    }

    #[test]
    fn opens_on_play_and_knob_1_turns_timbre_of_the_focused_layer() {
        let mut a = app();
        assert!(a.play_column().is_some());
        let before = a.p.layers[0].timbre.get();
        a.tick(&Input { knob1: 3, ..Default::default() });
        assert!(a.p.layers[0].timbre.get() > before);
        a.tick(&Input { navigation_steps: -1, ..Default::default() });
        assert_eq!(a.focus(), 1, "D-pad up focuses layer B");
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(TrioApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()))
}
