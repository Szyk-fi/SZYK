//! Orca: the livecoding sequencer language by Hundredrabbits, running its
//! own simulation (vendor/orca, MIT: sim.c, gbuffer.c, vmio.c from the C
//! implementation, Orca-c) -- every operator, every rule, the real thing.
//!
//! Orca is a grid of letters. Each letter is an operator; they read their
//! neighbours, write to the cells beside them, and every tick of the clock the
//! whole grid runs once. `A` adds, `D` delays, `E` moves east, `R` is random,
//! `T` plays a track, `:` sends a note, `*` is a bang that wakes lowercase
//! operators. There is no keyboard on this device, so editing is by pad.
//!
//! - **D-pad** moves the cursor: up and down by row, left and right by column
//!   (hold left or right to travel faster).
//! - **Pads** type a glyph at the cursor. **F2** turns the pages: operators A-P,
//!   operators Q-Z with the special glyphs, then the values 0-F, G-V, and W-Z.
//!   The right-hand panel shows what each pad types.
//! - **SELECT** erases the cell under the cursor. **R1** opens the menu:
//!   choose a Pattern (the bundled examples and anything you save), BPM,
//!   where the notes go, the built-in voice, and Save.
//!
//! The pads' other pages are the kit's usual **Controls** and **Moments**.
//!
//! Time comes from the device's shared transport: F3 starts and stops it with
//! every other sequencer, at its tempo, a tick every sixteenth note (Orca's
//! own rate of four ticks a beat). Notes (`:` and `%`) are sent to whichever
//! app the Route names, or to the built-in voice -- Orca makes no sound of its
//! own, so that voice is only a plain, polyphonic default so a patch can be
//! heard with nothing routed. MIDI control change and pitch bend glyphs are
//! run but have nowhere to go here, and the OSC and UDP glyphs are ignored.
//!
//! The grid runs on the audio thread, sample-locked to the clock; edits reach
//! it through a queue, and the screen shows a copy it publishes after each tick.

use super::kids_kit;
use crate::{
    app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input, SlintExtra},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    clock::Clock,
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::ModBus,
    note_bus::{NoteBus, NoteOut, NoteRoute},
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12},
    util::AtomicF32,
};
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}, text::Text};
use std::collections::VecDeque;
use std::ffi::{c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "Orca";

/// Where patterns live: the bundled `examples/` and anything saved.
const ORCA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/orca");

/// The grid is fixed at 64 x 24: it fits the screen at 6 x 12 pixels a cell,
/// and every bundled example is smaller.
const W: usize = 64;
const H: usize = 24;
const TICKS_PER_BEAT: f64 = 4.0;

// Own palette: black glass, phosphor green, operators in white.
const BG: Rgb565 = Rgb565::new(0, 1, 0);
const INK: Rgb565 = Rgb565::new(10, 52, 14);
const OPERATOR: Rgb565 = Rgb565::new(30, 61, 30);
const ACCENT: Rgb565 = Rgb565::new(31, 44, 4);
const DIM: Rgb565 = Rgb565::new(4, 18, 6);
const FAINT: Rgb565 = Rgb565::new(2, 8, 3);

#[repr(C)]
struct Oevent {
    raw: [u8; 38],
}

#[repr(C)]
struct OeventList {
    buffer: *mut Oevent,
    count: usize,
    capacity: usize,
}

unsafe extern "C" {
    fn orca_run(gbuffer: *mut c_char, mbuffer: *mut u8, height: usize, width: usize, tick: usize, events: *mut OeventList, seed: usize);
    fn mbuffer_clear(mbuffer: *mut u8, height: usize, width: usize);
    fn oevent_list_init(l: *mut OeventList);
    fn oevent_list_deinit(l: *mut OeventList);
    fn oevent_list_clear(l: *mut OeventList);
    fn oevent_list_alloc_item(l: *mut OeventList) -> *mut Oevent;
}

/// The simulation's event list, grown once up front so a tick on the audio
/// thread never reallocates it (more than `RESERVE` events in one tick is
/// not a thing a 64 x 24 grid can produce).
struct Events {
    list: OeventList,
    _anchor: *mut c_void,
}

const RESERVE: usize = 1024;

impl Events {
    fn new() -> Events {
        let mut list = OeventList { buffer: std::ptr::null_mut(), count: 0, capacity: 0 };
        unsafe {
            oevent_list_init(&mut list);
            for _ in 0..RESERVE {
                oevent_list_alloc_item(&mut list);
            }
            oevent_list_clear(&mut list);
        }
        Events { list, _anchor: std::ptr::null_mut() }
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        unsafe { oevent_list_deinit(&mut self.list) };
    }
}

// Only the audio thread touches it once it is built.
unsafe impl Send for Events {}

/// One note-on from the simulation.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NoteEvent {
    channel: u8,
    note: u8,
    velocity: u8,
    /// Length in ticks (0 is "as short as it gets").
    duration: u8,
    mono: bool,
}

/// The simulation: the grid, its marks, and the tick count.
struct Engine {
    grid: Vec<u8>,
    marks: Vec<u8>,
    tick: usize,
    events: Events,
}

impl Engine {
    fn new() -> Engine {
        Engine { grid: vec![b'.'; W * H], marks: vec![0; W * H], tick: 0, events: Events::new() }
    }

    /// Runs one tick; the notes it played are appended to `out`.
    fn step(&mut self, out: &mut Vec<NoteEvent>) {
        unsafe {
            mbuffer_clear(self.marks.as_mut_ptr(), H, W);
            oevent_list_clear(&mut self.events.list);
            orca_run(self.grid.as_mut_ptr() as *mut c_char, self.marks.as_mut_ptr(), H, W, self.tick, &mut self.events.list, 1);
        }
        self.tick += 1;
        for i in 0..self.events.list.count {
            let e = unsafe { &*self.events.list.buffer.add(i) };
            // Oevent_midi_note: type 0, channel, octave, note, velocity, duration:7 | mono:1.
            if e.raw[0] == 0 {
                let note = (12 * e.raw[2] as usize + e.raw[3] as usize).min(127) as u8;
                if e.raw[1] <= 15 && out.len() < out.capacity() {
                    out.push(NoteEvent { channel: e.raw[1], note, velocity: e.raw[4].clamp(1, 127), duration: e.raw[5] & 0x7f, mono: e.raw[5] & 0x80 != 0 });
                }
            }
        }
    }
}

/// Pattern text -> grid: lines of glyphs, cropped to the grid and padded with
/// `.`; anything that isn't a glyph Orca knows becomes `.`.
fn parse_pattern(text: &str) -> Vec<u8> {
    let mut grid = vec![b'.'; W * H];
    for (y, line) in text.lines().take(H).enumerate() {
        for (x, c) in line.bytes().take(W).enumerate() {
            if is_glyph(c) {
                grid[y * W + x] = c;
            }
        }
    }
    grid
}

fn is_glyph(c: u8) -> bool {
    c.is_ascii_alphanumeric() || b"!#%*.:;=?".contains(&c)
}

/// Grid -> text, trailing empty rows and columns trimmed so a saved pattern
/// stays small and Orca itself loads it back as the same size it was.
fn pattern_text(grid: &[u8]) -> String {
    let rows = (0..H).rev().find(|&y| grid[y * W..(y + 1) * W].iter().any(|&c| c != b'.')).map_or(1, |y| y + 1);
    let cols = (0..W).rev().find(|&x| (0..rows).any(|y| grid[y * W + x] != b'.')).map_or(1, |x| x + 1);
    (0..rows).map(|y| String::from_utf8_lossy(&grid[y * W..y * W + cols]).into_owned()).collect::<Vec<_>>().join("\n") + "\n"
}

/// What a pad types on each page, in reading order (top row first).
const PAGES: [(&str, [u8; 16]); 5] = [
    ("OPS A-P", *b"ABCDEFGHIJKLMNOP"),
    ("OPS Q-Z", *b"QRSTUVWXYZ*#:!?%"),
    ("0-F", *b"0123456789abcdef"),
    ("G-V", *b"ghijklmnopqrstuv"),
    ("W-Z . = ;", *b"wxyz.=;         ")
];

fn kit_config() -> KitConfig {
    KitConfig {
        app_id: "orca",
        layers: vec![
            Layer::Native(0, PAGES[0].0),
            Layer::Native(1, PAGES[1].0),
            Layer::Native(2, PAGES[2].0),
            Layer::Native(3, PAGES[3].0),
            Layer::Native(4, PAGES[4].0),
            Layer::Controls,
            Layer::Moments,
        ],
        // The D-pad is the cursor, so there are no dials to turn with it.
        hero: Vec::new(),
        browse: None,
        routes: Routes { stick_x: None, stick_y: None, hand_l: None, hand_r: None },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: true,
    }
}

const C_PATTERN: usize = 0;
const C_BPM: usize = 1;
const C_ROUTE: usize = 2;
const C_WAVE: usize = 3;
const C_RELEASE: usize = 4;
const C_LEVEL: usize = 5;
const C_SAVE: usize = 6;
const N_CONTROLS: usize = 7;

const WAVES: [&str; 4] = ["Sine", "Triangle", "Saw", "Square"];

enum Edit {
    Set(usize, u8),
    Load(Vec<u8>),
}

/// What the screen shows: the grid and its marks as of the last tick.
struct View {
    grid: Vec<u8>,
    marks: Vec<u8>,
    tick: u64,
}

struct Shared {
    edits: Mutex<VecDeque<Edit>>,
    view: Mutex<View>,
    wave: AtomicUsize,
    /// 0..1 positions.
    release: AtomicF32,
    level: AtomicF32,
    cv: [Arc<AtomicF32>; 2],
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    /// Notes played by the last tick, newest last, as MIDI numbers (display).
    recent: AtomicU64,
    notes_played: AtomicU32,
    clock: Arc<Clock>,
}

pub struct OrcaApp {
    p: Arc<Shared>,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
    patterns: Vec<(String, Option<PathBuf>)>,
    pattern: usize,
    dir: PathBuf,
    cursor: (usize, usize),
    pads_down: [bool; 16],
    route: NoteRoute,
    note_out: Option<NoteOut>,
    status: String,
}

impl OrcaApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        Self::with_dir(Path::new(ORCA_DIR), sensitivity, nav, mods, bus, mixer)
    }

    pub fn with_dir(dir: &Path, sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let (route, out) = NoteRoute::new(None, APP_NAME, "orca", true);
        let mut app = Self {
            p: Arc::new(Shared {
                edits: Mutex::new(VecDeque::new()),
                view: Mutex::new(View { grid: vec![b'.'; W * H], marks: vec![0; W * H], tick: 0 }),
                wave: AtomicUsize::new(1),
                release: AtomicF32::new(0.35),
                level: AtomicF32::new(0.7),
                cv: [mods.register(format!("{APP_NAME}: Release")), mods.register(format!("{APP_NAME}: Level"))],
                mix_level,
                ext_mix_level,
                output,
                recent: AtomicU64::new(0),
                notes_played: AtomicU32::new(0),
                clock: Clock::shared(),
            }),
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            patterns: Vec::new(),
            pattern: 0,
            dir: dir.to_path_buf(),
            cursor: (0, 0),
            pads_down: [false; 16],
            route,
            note_out: Some(out),
            status: String::new(),
        };
        app.rescan();
        app.load_pattern(1.min(app.patterns.len() - 1));
        app
    }

    /// Follows `clock` instead of the process-wide transport (tests).
    pub fn following(mut self, clock: Arc<Clock>) -> Self {
        Arc::get_mut(&mut self.p).expect("set the clock before anything else holds Orca's state").clock = clock;
        self
    }

    /// Lets Orca play other apps (see note_bus.rs).
    pub fn with_notes(mut self, bus: Option<Arc<NoteBus>>) -> Self {
        let (route, out) = NoteRoute::new(bus, APP_NAME, "orca", true);
        self.route = route;
        self.note_out = Some(out);
        self
    }

    /// "(empty)", then every `.orca` file under the folder, one level deep.
    fn rescan(&mut self) {
        let mut found: Vec<(String, PathBuf)> = Vec::new();
        let mut take = |d: &Path, prefix: &str| {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_file() && p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("orca")) {
                    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    found.push((format!("{prefix}{stem}"), p));
                }
            }
        };
        take(&self.dir, "");
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                take(&p, &format!("{name}/"));
                for sub in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                    let sp = sub.path();
                    if sp.is_dir() {
                        let sname = sp.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        take(&sp, &format!("{name}/{sname}/"));
                    }
                }
            }
        }
        found.sort_by_key(|(n, _)| n.to_lowercase());
        self.patterns = std::iter::once(("(empty)".to_string(), None)).chain(found.into_iter().map(|(n, p)| (n, Some(p)))).collect();
    }

    fn load_pattern(&mut self, index: usize) {
        let Some((name, path)) = self.patterns.get(index).cloned() else { return };
        let grid = match path {
            None => vec![b'.'; W * H],
            Some(p) => match std::fs::read_to_string(&p) {
                Ok(text) => parse_pattern(&text),
                Err(e) => {
                    self.status = format!("Could not read {name}: {e}");
                    return;
                }
            },
        };
        self.pattern = index;
        self.status.clear();
        self.cursor = (0, 0);
        if let Ok(mut q) = self.p.edits.lock() {
            q.push_back(Edit::Load(grid));
        }
    }

    fn step_pattern(&mut self, d: i32) {
        let n = self.patterns.len() as i32;
        if n > 0 && d != 0 {
            self.load_pattern((self.pattern as i32 + d.signum()).rem_euclid(n) as usize);
        }
    }

    fn put(&mut self, x: usize, y: usize, g: u8) {
        if x < W && y < H && is_glyph(g) {
            if let Ok(mut q) = self.p.edits.lock() {
                q.push_back(Edit::Set(y * W + x, g));
            }
        }
    }

    /// Writes the grid to `orca/saved/NN.orca`.
    fn save(&mut self) {
        let grid = self.p.view.lock().map(|v| v.grid.clone()).unwrap_or_default();
        let dir = self.dir.join("saved");
        if std::fs::create_dir_all(&dir).is_err() {
            self.status = "Could not create the saved folder".into();
            return;
        }
        let n = (1..1000).find(|n| !dir.join(format!("{n:02}.orca")).exists()).unwrap_or(999);
        let path = dir.join(format!("{n:02}.orca"));
        match std::fs::write(&path, pattern_text(&grid)) {
            Ok(()) => {
                self.status = format!("Saved saved/{n:02}");
                self.rescan();
                if let Some(i) = self.patterns.iter().position(|(_, p)| p.as_ref() == Some(&path)) {
                    self.pattern = i;
                }
            }
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    fn bpm(&self) -> f32 {
        self.p.clock.bpm()
    }

    fn text(&self, i: usize) -> (String, String) {
        match i {
            C_PATTERN => ("Pattern".into(), format!("{} {}/{}", self.patterns.get(self.pattern).map_or("?", |p| p.0.as_str()), self.pattern + 1, self.patterns.len())),
            C_BPM => ("BPM".into(), format!("{:.0}", self.bpm())),
            C_ROUTE => ("Plays".into(), self.route.label()),
            C_WAVE => ("Voice".into(), WAVES[self.p.wave.load(Ordering::Relaxed).min(3)].into()),
            C_RELEASE => ("Release".into(), format!("{:.0} ms", release_ms(self.p.release.get()))),
            C_LEVEL => ("Level".into(), format!("{:.0}%", self.p.level.get() * 100.0)),
            _ => ("Save".into(), "press to save as a new pattern".into()),
        }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        (0..N_CONTROLS).map(|i| {
            let (n, v) = self.text(i);
            (n, v, false)
        }).collect()
    }

    fn edit_continuous(&mut self, i: usize, d: i32) {
        let step = d as f32 * 0.01 * self.sensitivity.get().max(0.01) * 10.0;
        match i {
            C_BPM => self.p.clock.set_bpm((self.bpm() + d as f32).clamp(30.0, 300.0)),
            C_RELEASE => self.p.release.set((self.p.release.get() + step).clamp(0.0, 1.0)),
            C_LEVEL => self.p.level.set((self.p.level.get() + step).clamp(0.0, 1.0)),
            _ => {}
        }
    }
}

/// The built-in voice's release, 10 ms to 3 s, exponentially.
fn release_ms(p: f32) -> f32 {
    10.0 * 300f32.powf(p.clamp(0.0, 1.0))
}

impl PlayHost for OrcaApp {
    fn kit_control_count(&self) -> usize {
        N_CONTROLS
    }
    fn kit_label(&self, i: usize) -> String {
        self.text(i.min(N_CONTROLS - 1)).0
    }
    fn kit_value(&self, i: usize) -> String {
        self.text(i.min(N_CONTROLS - 1)).1
    }
    fn kit_norm(&self, i: usize) -> Option<f32> {
        Some(match i {
            C_PATTERN => if self.patterns.len() > 1 { self.pattern as f32 / (self.patterns.len() - 1) as f32 } else { 0.0 },
            C_BPM => (self.bpm() - 30.0) / 270.0,
            C_WAVE => self.p.wave.load(Ordering::Relaxed) as f32 / 3.0,
            C_RELEASE => self.p.release.get(),
            C_LEVEL => self.p.level.get(),
            _ => return None,
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        matches!(i, C_PATTERN | C_ROUTE | C_WAVE | C_SAVE)
    }
    fn kit_pads_play(&self, _layer: u8) -> bool {
        false
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        match i {
            C_PATTERN => self.step_pattern(delta),
            C_ROUTE => {
                if delta != 0 {
                    self.route.step(delta);
                }
            }
            C_WAVE => {
                if delta != 0 {
                    self.p.wave.store((self.p.wave.load(Ordering::Relaxed) as i32 + delta.signum()).rem_euclid(4) as usize, Ordering::Relaxed);
                }
            }
            C_SAVE => {
                if delta != 0 {
                    self.save();
                }
            }
            _ => self.edit_continuous(i, delta),
        }
    }
    fn kit_reset(&mut self, i: usize) {
        match i {
            C_BPM => self.p.clock.set_bpm(120.0),
            C_ROUTE => self.route.reset(),
            C_WAVE => self.p.wave.store(1, Ordering::Relaxed),
            C_RELEASE => self.p.release.set(0.35),
            C_LEVEL => self.p.level.set(0.7),
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i {
            C_PATTERN => {
                if self.patterns.len() > 1 {
                    let t = (v * (self.patterns.len() - 1) as f32).round() as usize;
                    if t != self.pattern {
                        self.load_pattern(t);
                    }
                }
            }
            C_BPM => self.p.clock.set_bpm(30.0 + v * 270.0),
            C_WAVE => self.p.wave.store((v * 3.0).round() as usize, Ordering::Relaxed),
            C_RELEASE => self.p.release.set(v),
            C_LEVEL => self.p.level.set(v),
            _ => {}
        }
    }
    fn kit_line(&self) -> String {
        if self.status.is_empty() {
            format!("tick {}", self.p.view.lock().map_or(0, |v| v.tick))
        } else {
            self.status.clone()
        }
    }
}

impl App for OrcaApp {
    fn play_surface(&self) -> bool {
        true
    }
    fn wants_fullscreen(&self) -> bool {
        true
    }
    fn play_column(&self) -> Option<crate::app::PlayColumn> {
        None
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
        Some(self.p.clock.running())
    }
    fn toggle_running(&mut self) {
        self.p.clock.toggle();
    }
    fn needs_background_audio(&self) -> bool {
        self.p.clock.running()
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
            self.pads_down = [false; 16];
            return;
        }
        let Some(page) = step.native else {
            self.pads_down = [false; 16];
            return;
        };
        // The cursor: the raw D-pad, since the kit would turn a dial with it.
        let (x, y) = self.cursor;
        let dy = input.navigation_steps;
        let dx = input.nav_x;
        self.cursor = ((x as i32 + dx).clamp(0, W as i32 - 1) as usize, (y as i32 + dy).clamp(0, H as i32 - 1) as usize);
        let (x, y) = self.cursor;
        for pad in 0..16 {
            if step.input.grid[pad] && !self.pads_down[pad] {
                // A blank pad types nothing (put ignores what isn't a glyph).
                self.put(x, y, PAGES[(page as usize).min(PAGES.len() - 1)].1[pad]);
            }
            self.pads_down[pad] = step.input.grid[pad];
        }
        if input.knob1_press {
            self.put(x, y, b'.');
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if self.kit.menu {
            Text::new(APP_NAME, Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, OPERATOR)).draw(f).ok();
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 44, 24, r.len(), &r, BG, DIM, ACCENT);
            if !self.status.is_empty() {
                Text::new(&self.status, Point::new(16, 330), MonoTextStyle::new(&SPLEEN_6X12, ACCENT)).draw(f).ok();
            }
            return;
        }
        if matches!(self.kit.layer(), Layer::Controls | Layer::Moments) {
            let col = self.kit.column(self);
            let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
            kit::draw::column(f, &col, 16, 40, 350, 280, pal);
        } else {
            self.draw_grid(f);
        }
        self.draw_side(f);
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
    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        if self.taken {
            return None;
        }
        self.taken = true;
        Some(Box::new(Processor {
            p: Arc::clone(&self.p),
            engine: Engine::new(),
            notes: self.note_out.take().unwrap_or_else(NoteOut::detached),
            events: Vec::with_capacity(RESERVE),
            voices: [Voice::default(); VOICES],
            next_voice: 0,
            next_beat: 0.0,
            epoch: u64::MAX,
            was_running: false,
            mono: [None; 16],
        }))
    }
}

impl OrcaApp {
    fn draw_grid(&self, f: &mut FrameBuffer) {
        let Ok(view) = self.p.view.lock() else { return };
        let (x0, y0) = (6, 22);
        let mut one = [0u8; 4];
        for y in 0..H {
            for x in 0..W {
                let g = view.grid[y * W + x];
                let m = view.marks[y * W + x];
                let (px, py) = (x0 + x as i32 * 6, y0 + y as i32 * 12);
                let here = (x, y) == self.cursor;
                if here {
                    Rectangle::new(Point::new(px, py), Size::new(6, 12)).into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(f).ok();
                }
                if g == b'.' {
                    if here || m & 0b11 != 0 {
                        continue;
                    }
                    // an empty cell is a single dim dot
                    Rectangle::new(Point::new(px + 2, py + 7), Size::new(1, 1)).into_styled(PrimitiveStyle::with_fill(if x % 8 == 0 && y % 8 == 0 { INK } else { FAINT })).draw(f).ok();
                    continue;
                }
                let colour = if here {
                    BG
                } else if m & 0b0100 != 0 {
                    ACCENT // a locked or sleeping cell
                } else if g.is_ascii_uppercase() || g == b'*' || b"#:!?%=;".contains(&g) {
                    OPERATOR
                } else if m & 0b11 != 0 {
                    ACCENT
                } else {
                    INK
                };
                let s = (g as char).encode_utf8(&mut one);
                Text::new(s, Point::new(px, py + 9), MonoTextStyle::new(&SPLEEN_6X12, colour)).draw(f).ok();
            }
        }
    }

    fn draw_side(&self, f: &mut FrameBuffer) {
        let (ink, dim, acc) = (MonoTextStyle::new(&SPLEEN_6X12, OPERATOR), MonoTextStyle::new(&SPLEEN_6X12, DIM), MonoTextStyle::new(&SPLEEN_6X12, ACCENT));
        let x = 398;
        Text::new("ORCA", Point::new(x, 16), MonoTextStyle::new(&SPLEEN_6X12, OPERATOR)).draw(f).ok();
        let tick = self.p.view.lock().map_or(0, |v| v.tick);
        let state = if self.p.clock.running() { "playing" } else { "stopped (F3)" };
        Text::new(&format!("{:.0} bpm  tick {tick}  {state}", self.bpm()), Point::new(x + 40, 16), dim).draw(f).ok();
        let name = self.patterns.get(self.pattern).map_or("", |p| p.0.as_str());
        Text::new(&name.chars().take(38).collect::<String>(), Point::new(x, 36), acc).draw(f).ok();
        Text::new(&format!("plays: {}", self.route.label().chars().take(28).collect::<String>()), Point::new(x, 52), dim).draw(f).ok();
        Text::new(&format!("cursor {},{}", self.cursor.0, self.cursor.1), Point::new(x, 68), dim).draw(f).ok();
        // What each pad types on this page.
        let page = match self.kit.layer() {
            Layer::Native(id, _) => id as usize,
            _ => 0,
        };
        let (title, glyphs) = PAGES[page.min(PAGES.len() - 1)];
        Text::new(&format!("pads: {title}"), Point::new(x, 92), ink).draw(f).ok();
        for (i, g) in glyphs.iter().enumerate() {
            let (cx, cy) = (x + 8 + (i as i32 % 4) * 40, 104 + (i as i32 / 4) * 34);
            Rectangle::new(Point::new(cx, cy), Size::new(34, 28)).into_styled(PrimitiveStyle::with_stroke(DIM, 1)).draw(f).ok();
            let c = *g as char;
            if c != ' ' {
                let mut buf = [0u8; 4];
                Text::new(c.encode_utf8(&mut buf), Point::new(cx + 14, cy + 18), if g.is_ascii_uppercase() { ink } else { acc }).draw(f).ok();
            }
        }
        // The notes just sent.
        let recent = self.p.recent.load(Ordering::Relaxed);
        let names: Vec<String> = (0..4).rev().filter_map(|k| {
            let n = ((recent >> (k * 8)) & 0xff) as u8;
            (n != 0).then(|| crate::util::note_name(n as i32 - 1))
        }).collect();
        Text::new(&format!("notes {}", names.join(" ")), Point::new(x, 254), dim).draw(f).ok();
        Text::new(&format!("{} sent", self.p.notes_played.load(Ordering::Relaxed)), Point::new(x, 268), dim).draw(f).ok();
        Text::new("D-pad: cursor", Point::new(x, 292), dim).draw(f).ok();
        Text::new("pads: type  SELECT: erase", Point::new(x, 306), dim).draw(f).ok();
        Text::new("F2: pad page  R1: menu", Point::new(x, 320), dim).draw(f).ok();
    }
}

const VOICES: usize = 8;

/// The built-in voice: a plain polyphonic oscillator with a short attack and
/// a release you set, so a patch is audible with nothing routed.
#[derive(Clone, Copy, Default)]
struct Voice {
    freq: f32,
    phase: f32,
    amp: f32,
    gain: f32,
    gate_samples: i64,
    gate: bool,
    active: bool,
    note: u8,
}

fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

impl Voice {
    fn next(&mut self, wave: usize, rate: f32, attack: f32, release: f32) -> f32 {
        if !self.active {
            return 0.0;
        }
        let dt = (self.freq / rate).clamp(0.0, 0.45);
        let s = match wave {
            0 => (self.phase * std::f32::consts::TAU).sin(),
            1 => 4.0 * (self.phase - 0.5).abs() - 1.0,
            2 => 2.0 * self.phase - 1.0 - poly_blep(self.phase, dt),
            _ => {
                let naive = if self.phase < 0.5 { 1.0 } else { -1.0 };
                naive + poly_blep(self.phase, dt) - poly_blep((self.phase + 0.5) % 1.0, dt)
            }
        };
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        if self.gate {
            self.amp = (self.amp + attack).min(1.0);
            self.gate_samples -= 1;
            if self.gate_samples <= 0 {
                self.gate = false;
            }
        } else {
            self.amp *= release;
            if self.amp < 1e-4 {
                self.active = false;
            }
        }
        s * self.amp * self.gain
    }
}

struct Processor {
    p: Arc<Shared>,
    engine: Engine,
    notes: NoteOut,
    events: Vec<NoteEvent>,
    voices: [Voice; VOICES],
    next_voice: usize,
    /// The beat the next tick falls on, and the transport epoch it belongs to.
    next_beat: f64,
    epoch: u64,
    was_running: bool,
    /// The mono note sounding on each channel.
    mono: [Option<u8>; 16],
}

impl Processor {
    fn apply_edits(&mut self) -> bool {
        let Ok(mut q) = self.p.edits.try_lock() else { return false };
        let mut changed = false;
        while let Some(e) = q.pop_front() {
            match e {
                Edit::Set(i, g) => {
                    if i < self.engine.grid.len() {
                        self.engine.grid[i] = g;
                    }
                }
                Edit::Load(g) => {
                    if g.len() == self.engine.grid.len() {
                        self.engine.grid.copy_from_slice(&g);
                        self.engine.tick = 0;
                    }
                    // The old pattern's notes must not hang over into the new one.
                    self.notes.all_off();
                    for v in self.voices.iter_mut() {
                        v.gate = false;
                    }
                }
            }
            changed = true;
        }
        changed
    }

    fn publish(&self) {
        if let Ok(mut v) = self.p.view.try_lock() {
            v.grid.copy_from_slice(&self.engine.grid);
            v.marks.copy_from_slice(&self.engine.marks);
            v.tick = self.engine.tick as u64;
        }
    }

    /// Sends one tick's notes to wherever the route says.
    fn play(&mut self, events: &[NoteEvent], tick_samples: f64, rate: f32) {
        for e in events {
            // Orca's length is in ticks; 0 is as short as it gets (a blip).
            let gate = if e.duration == 0 { (rate * 0.02) as u32 } else { (e.duration as f64 * tick_samples) as u32 }.max(1);
            if e.mono {
                if let Some(prev) = self.mono[e.channel as usize].take() {
                    self.notes.note_off(prev);
                }
                self.mono[e.channel as usize] = Some(e.note);
            }
            if self.notes.internal() {
                // The same note again takes over its own voice.
                let slot = self.voices.iter().position(|v| v.active && v.note == e.note).unwrap_or(self.next_voice);
                self.next_voice = (slot + 1) % VOICES;
                self.voices[slot] = Voice { freq: 440.0 * 2f32.powf((e.note as f32 - 69.0) / 12.0), phase: 0.0, amp: 0.0, gain: e.velocity as f32 / 127.0, gate_samples: gate as i64, gate: true, active: true, note: e.note };
            } else {
                self.notes.trigger(e.note, e.velocity, gate);
            }
            let r = self.p.recent.load(Ordering::Relaxed);
            self.p.recent.store((r << 8) | (e.note as u64 + 1), Ordering::Relaxed);
            self.p.notes_played.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl AudioProcessor for Processor {
    fn process(&mut self, out: &mut [f32], channels: usize, rate: f32) {
        out.fill(0.0);
        if channels == 0 || !rate.is_finite() || rate < 1000.0 {
            return;
        }
        let frames = out.len() / channels;
        let mut dirty = self.apply_edits();
        let snap = self.p.clock.snap();
        let running = snap.running;
        if running && (!self.was_running || snap.epoch != self.epoch) {
            // A (re)start begins the grid again from tick 0 on the next tick boundary.
            if snap.epoch != self.epoch {
                self.engine.tick = 0;
                self.next_beat = (snap.beat * TICKS_PER_BEAT - 1e-9).ceil().max(0.0) / TICKS_PER_BEAT;
                self.epoch = snap.epoch;
            } else {
                self.next_beat = (snap.beat * TICKS_PER_BEAT - 1e-9).ceil().max(0.0) / TICKS_PER_BEAT;
            }
        }
        if !running && self.was_running {
            self.notes.all_off();
            for v in self.voices.iter_mut() {
                v.gate = false;
            }
            self.mono = [None; 16];
        }
        self.was_running = running;

        let bps = snap.beats_per_sample(rate);
        let tick_samples = rate as f64 * 60.0 / snap.bpm.max(1.0) as f64 / TICKS_PER_BEAT;
        let wave = self.p.wave.load(Ordering::Relaxed).min(3);
        let release_s = release_ms((self.p.release.get() + self.p.cv[0].get()).clamp(0.0, 1.0)) / 1000.0;
        let release = (-1.0 / (release_s * rate).max(1.0)).exp();
        let attack = 1.0 / (0.003 * rate);
        let level = (self.p.level.get() + self.p.cv[1].get()).clamp(0.0, 1.0);
        let master = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0) * level * level * 0.5;
        let shared = Arc::clone(&self.p);
        let mut bus = shared.output.try_lock().ok();
        if let Some(b) = bus.as_mut() {
            b.clear();
        }
        let mut done = 0usize;
        while done < frames {
            let mut chunk = frames - done;
            if running && bps > 0.0 {
                let until = (self.next_beat - snap.beat) / bps - done as f64;
                if until <= 0.0 {
                    self.events.clear();
                    self.engine.step(&mut self.events);
                    let events = std::mem::take(&mut self.events);
                    self.play(&events, tick_samples, rate);
                    self.events = events;
                    self.next_beat += 1.0 / TICKS_PER_BEAT;
                    dirty = true;
                    continue;
                }
                chunk = chunk.min(until.ceil().max(1.0) as usize);
            }
            self.notes.advance(chunk as u32);
            for k in 0..chunk {
                let mut s = 0.0;
                for v in self.voices.iter_mut() {
                    s += v.next(wave, rate, attack, release);
                }
                let s = s * master;
                for o in out[(done + k) * channels..(done + k + 1) * channels].iter_mut() {
                    *o = s;
                }
                if let Some(b) = bus.as_mut() {
                    b.push(s);
                }
            }
            done += chunk;
        }
        if dirty {
            self.publish();
        }
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(OrcaApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()).following(Clock::shared()).with_notes(ctx.try_get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(dir: &Path) -> (OrcaApp, Arc<Clock>) {
        let clock = Arc::new(Clock::new());
        clock.set_bpm(120.0);
        let a = OrcaApp::with_dir(dir, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new())).following(Arc::clone(&clock));
        (a, clock)
    }

    fn dir_with(files: &[(&str, &str)]) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("portamax-orca-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        dir
    }

    /// Runs `blocks` blocks of 512 frames, advancing the clock like the mix engine does.
    fn run(p: &mut Box<dyn AudioProcessor>, clock: &Clock, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for _ in 0..blocks {
            let mut buf = vec![0.0f32; 512 * 2];
            p.process(&mut buf, 2, 48_000.0);
            clock.end_block(512, 48_000.0);
            assert!(buf.iter().all(|v| v.is_finite()));
            all.extend(buf.iter().step_by(2));
        }
        all
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn hz(x: &[f32]) -> f32 {
        let crossings = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        crossings as f32 / (x.len() as f32 / 48_000.0)
    }

    #[test]
    #[ignore = "writes a screenshot to the scratch path in PORTAMAX_ORCA_SHOT"]
    fn screenshot() {
        let Ok(path) = std::env::var("PORTAMAX_ORCA_SHOT") else { return };
        let (mut a, clock) = app(Path::new(ORCA_DIR));
        let want = std::env::var("PORTAMAX_ORCA_PATTERN").unwrap_or_else(|_| "examples/basics/_midi".into());
        let i = a.patterns.iter().position(|(n, _)| *n == want).unwrap();
        a.load_pattern(i);
        let mut p = a.audio_processor().unwrap();
        a.toggle_running();
        run(&mut p, &clock, 150);
        a.tick(&Input { nav_x: 5, navigation_steps: 3, ..Default::default() });
        let mut fb = FrameBuffer::new();
        a.draw(&mut fb);
        let bytes: Vec<u8> = fb.buffer().iter().flat_map(|px| [(px >> 16) as u8, (px >> 8) as u8, *px as u8]).collect();
        let mut out = format!("P6\n640 360\n255\n").into_bytes();
        out.extend(bytes);
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn the_real_simulation_runs_its_operators() {
        let mut e = Engine::new();
        // 1A2: A adds its two neighbours and writes the sum below itself.
        e.grid[W + 1] = b'1';
        e.grid[W + 2] = b'A';
        e.grid[W + 3] = b'2';
        let mut notes = Vec::with_capacity(8);
        e.step(&mut notes);
        assert_eq!(e.grid[2 * W + 2], b'3', "1 + 2 = 3, one row down");
        assert!(notes.is_empty());
    }

    #[test]
    fn a_midi_glyph_banged_makes_the_note_it_spells() {
        let mut e = Engine::new();
        // :03C with a bang under the colon (channel 0, octave 3, note C): MIDI 36.
        // The bang goes below, not beside: the simulation runs the grid in reading
        // order and a bang erases itself when it runs, so one to the left would be
        // gone before the operator it was meant to wake.
        for (i, c) in b":03C".iter().enumerate() {
            e.grid[W + 1 + i] = *c;
        }
        e.grid[2 * W + 1] = b'*';
        let mut notes = Vec::with_capacity(8);
        e.step(&mut notes);
        assert_eq!(notes.len(), 1);
        assert_eq!((notes[0].note, notes[0].channel, notes[0].velocity), (36, 0, 127));
        // A lowercase letter is a sharp: c is C#.
        let mut e = Engine::new();
        for (i, c) in b":03c".iter().enumerate() {
            e.grid[W + 1 + i] = *c;
        }
        e.grid[2 * W + 1] = b'*';
        e.step(&mut notes);
        assert_eq!(notes.last().unwrap().note, 37);
    }

    #[test]
    fn patterns_parse_save_and_round_trip() {
        let g = parse_pattern("1A2\n.#x?\n");
        assert_eq!(&g[..3], b"1A2");
        assert_eq!(g[W + 1], b'#');
        assert_eq!(g[W + 3], b'?');
        assert_eq!(pattern_text(&g), "1A2.\n.#x?\n");
        let junk = parse_pattern("a~b\u{e9}c");
        assert_eq!(&junk[..6], b"a.b..c", "an unknown character is an empty cell (the accent is two bytes)");
    }

    #[test]
    fn it_lists_the_bundled_examples_and_loads_one() {
        let dir = Path::new(ORCA_DIR);
        let (a, _) = app(dir);
        assert!(a.patterns.len() > 20, "{} patterns", a.patterns.len());
        assert_eq!(a.patterns[0].0, "(empty)");
        assert!(a.patterns.iter().any(|(n, _)| n == "examples/basics/a"));
    }

    #[test]
    fn a_pad_types_at_the_cursor_and_select_erases_and_the_dpad_moves() {
        let dir = dir_with(&[]);
        let (mut a, clock) = app(&dir);
        let mut p = a.audio_processor().unwrap();
        // Page 0: pad 0 is A. Move right twice and down once, then type.
        a.tick(&Input { nav_x: 2, navigation_steps: 1, ..Default::default() });
        assert_eq!(a.cursor, (2, 1));
        a.tick(&Input { grid: std::array::from_fn(|i| i == 0), ..Default::default() });
        a.tick(&Input::default());
        run(&mut p, &clock, 1);
        assert_eq!(a.p.view.lock().unwrap().grid[W + 2], b'A');
        a.tick(&Input { knob1_press: true, ..Default::default() });
        run(&mut p, &clock, 1);
        assert_eq!(a.p.view.lock().unwrap().grid[W + 2], b'.', "SELECT erases");
        // F2 to the next page: pad 0 is Q there.
        a.toggle_grid_mode();
        a.tick(&Input { grid: std::array::from_fn(|i| i == 0), ..Default::default() });
        run(&mut p, &clock, 1);
        assert_eq!(a.p.view.lock().unwrap().grid[W + 2], b'Q');
    }

    #[test]
    fn the_transport_runs_a_patch_and_its_note_sounds_at_pitch_on_the_beat() {
        // A clock (8D1) bangs the colon beside it every 8th tick: channel 0, octave 5, note A = MIDI 69 =
        // 440 Hz, held for 4 ticks.
        let dir = dir_with(&[("p.orca", ".8D1\n...:05A.4\n")]);
        let (mut a, clock) = app(&dir);
        assert_eq!(a.patterns[a.pattern].0, "p", "the first pattern is loaded on start");
        a.p.wave.store(0, Ordering::Relaxed); // sine, so the pitch is easy to count
        a.p.release.set(0.0);
        let mut p = a.audio_processor().unwrap();
        // Stopped: the grid doesn't run and nothing sounds.
        assert!(rms(&run(&mut p, &clock, 40)) < 1e-6);
        a.toggle_running();
        let out = run(&mut p, &clock, 80);
        assert!(rms(&out) > 0.02, "it plays ({})", rms(&out));
        // At 120 bpm a tick is 6000 samples: the first tick has fired, the grid advanced.
        let tick = a.p.view.lock().unwrap().tick;
        assert!((5..=8).contains(&tick), "80 blocks of 512 at 120 bpm is about 6.8 ticks, got {tick}");
        let steady = &out[2000..14000];
        assert!((hz(steady) - 440.0).abs() < 15.0, "A5... A4 at 440 Hz, got {}", hz(steady));
        a.toggle_running();
        let after = run(&mut p, &clock, 60);
        assert!(rms(&after[after.len() - 4800..]) < 0.001, "stopping releases the notes");
    }

    #[test]
    fn notes_can_be_routed_to_another_app_through_the_note_bus() {
        let dir = dir_with(&[("p.orca", ".1D1\n...:05A\n")]);
        let clock = Arc::new(Clock::new());
        clock.set_bpm(120.0);
        let bus = Arc::new(NoteBus::new());
        let inbox = bus.register_instrument("synth", "Test Synth").unwrap();
        let mut a = OrcaApp::with_dir(&dir, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::new(ModBus::new()), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()))
            .following(Arc::clone(&clock))
            .with_notes(Some(Arc::clone(&bus)));
        a.kit_edit(C_ROUTE, 1); // own sound -> none
        a.kit_edit(C_ROUTE, 1); // none -> Test Synth
        assert_eq!(a.route.label(), "Test Synth");
        let mut p = a.audio_processor().unwrap();
        a.toggle_running();
        let mut view = crate::note_bus::NoteView::default();
        let mut seen = false;
        for _ in 0..80 {
            run(&mut p, &clock, 1);
            inbox.poll(&mut view);
            seen |= view.keys[69] > 0;
        }
        assert!(seen, "A4 (69) reached the other app's inbox");
    }

    #[test]
    fn saving_writes_a_pattern_that_loads_back() {
        let dir = dir_with(&[]);
        let (mut a, clock) = app(&dir);
        let mut p = a.audio_processor().unwrap();
        a.tick(&Input { grid: std::array::from_fn(|i| i == 3), ..Default::default() }); // D
        a.tick(&Input::default());
        run(&mut p, &clock, 1);
        a.kit_edit(C_SAVE, 1);
        assert!(dir.join("saved/01.orca").exists(), "{}", a.status);
        assert_eq!(std::fs::read_to_string(dir.join("saved/01.orca")).unwrap(), "D\n");
        assert!(a.patterns.iter().any(|(n, _)| n == "saved/01"));
    }

    #[test]
    fn its_controls_are_mod_inputs_and_it_opens_on_the_grid() {
        let dir = dir_with(&[]);
        let modbus = Arc::new(ModBus::new());
        let a = OrcaApp::with_dir(&dir, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::clone(&modbus), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()));
        for name in ["Orca: Release", "Orca: Level", "Mixer: Orca Level"] {
            assert!(modbus.index_of(name).is_some(), "{name}");
        }
        assert_eq!(a.kit.layer_label(), "OPS A-P");
        assert!(a.wants_fullscreen());
    }
}
