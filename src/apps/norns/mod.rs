//! Norns: a cartridge that runs norns scripts (monome's Lua sound
//! computer platform) on Portamax.
//!
//! Scripts live on the SD card in `saves/norns/dust/code/<name>/<name>.lua`
//! (the same layout as norns' `dust/code`), plus the bundled ones in
//! `assets/norns/code/`. Pick one from the list and it runs with the norns
//! API: screen, encoders and keys, params, clock, metro, MIDI in, the
//! PolyPerc engine and softcut. Everything here is Portamax's own
//! implementation of the documented API (see host.rs, engine.rs,
//! lua/prelude.lua); no norns code is included, so scripts are
//! user-supplied, like ROMs for Retro.
//!
//! Grid: `grid.connect()` gives a script Portamax's grid (the Grid app on
//! screen, and a real monome grid when one is plugged in), with `g.key`,
//! `g:led`, `g:all`, `g:refresh`, `g:rotation` and `g.cols` / `g.rows`; the
//! script takes the grid when it connects. Play it from the Grid app.
//!
//! Arc: `arc.connect()` gives a script Portamax's arc (the Arc app on
//! screen, and a real monome arc when one is plugged in), with `a.delta`,
//! `a.key`, `a:led`, `a:all`, `a:segment` and `a:refresh`; connecting takes
//! the arc. Turn it from the Arc app.
//!
//! Not there (yet): other SuperCollider engines (scripts that ask for one
//! still run, with its sound commands ignored), crow, audio input into softcut, saving psets.
//!
//! Controls -- norns has three encoders and three keys:
//! - D-pad up/down (or knob 1) = E2, D-pad left/right (or knob 2) = E3;
//!   pad 16 held turns either into E1
//!   (pads 9/10 also nudge E1 down/up)
//! - SELECT = K2, hold SELECT = K3; pads 13/14/15 = K1/K2/K3, held
//! - pad 11 = Params page (knob 1 picks, knob 2 changes), pad 12 = back
//!   to the script list
//! - pads 1-8 play a C major scale into the script as MIDI notes
//!   (MIDI device 1), for scripts that take notes

pub mod engine;
pub mod host;
pub mod screen;

use crate::app::{App, Input};
use crate::audio::AudioProcessor;
use crate::audio_bus::AudioBus;
use crate::display::FrameBuffer;
use crate::led_output::PadColor;
use crate::mixer_bus::MixerBus;
use crate::modbus::ModBus;
use crate::util::AtomicF32;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use engine::Queue;
use host::{Event, HostOut};
use screen::{Screen, H, W};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};

const APP_NAME: &str = "Norns";
const BG: Rgb565 = Rgb565::new(1, 2, 1);
const INK: Rgb565 = Rgb565::new(26, 52, 26);
const DIM: Rgb565 = Rgb565::new(12, 24, 12);

/// Pads (0-based) and what they do.
const PAD_E1_DOWN: usize = 8;
const PAD_E1_UP: usize = 9;
const PAD_PARAMS: usize = 10;
const PAD_SELECT: usize = 11;
const PAD_K: [usize; 3] = [12, 13, 14];
const PAD_E1_SHIFT: usize = 15;
/// Pads 1-8: C major from middle C.
const NOTE_PADS: [u8; 8] = [60, 62, 64, 65, 67, 69, 71, 72];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Select,
    Play,
    Params,
}

#[derive(Clone, Debug)]
pub struct Script {
    pub name: String,
    pub path: PathBuf,
    /// Shipped with Portamax rather than on the SD card.
    #[allow(dead_code)] // Slint GUI list
    pub bundled: bool,
}

/// The two places scripts come from.
fn script_roots() -> Vec<(PathBuf, bool)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    vec![(root.join("saves/norns/dust/code"), false), (root.join("assets/norns/code"), true)]
}

/// Every `<dir>/<dir>.lua` (and other top-level .lua files in a script
/// folder, as `dir/file`) under the given roots.
pub fn discover(roots: &[(PathBuf, bool)]) -> Vec<Script> {
    let mut out = Vec::new();
    for (root, bundled) in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for dir in dirs {
            let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let main = dir.join(format!("{name}.lua"));
            if main.is_file() {
                out.push(Script { name: name.clone(), path: main, bundled: *bundled });
            }
            let mut others: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map(|e| e.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "lua") && p.file_stem().is_some_and(|s| s != name.as_str())).collect())
                .unwrap_or_default();
            others.sort();
            for p in others {
                let stem = p.file_stem().unwrap().to_string_lossy().to_string();
                out.push(Script { name: format!("{name}/{stem}"), path: p, bundled: *bundled });
            }
        }
    }
    out
}

struct Running {
    tx: Sender<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
    name: String,
}

pub struct NornsApp {
    scripts: Vec<Script>,
    selected: usize,
    mode: Mode,
    queue: Arc<Queue>,
    out: Arc<Mutex<HostOut>>,
    running: Option<Running>,
    /// Keys to release next frame (from knob presses).
    pending_release: Vec<i32>,
    prev_grid: [bool; 16],
    param_sel: usize,
    /// The 128x64 picture currently shown (script frame, or our menus).
    shown: Vec<u8>,
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    peak: Arc<AtomicF32>,
    roots: Vec<(PathBuf, bool)>,
    /// Where the script's MIDI notes go (set from Portal's Notes page).
    note_out: Option<crate::note_bus::NoteOut>,
    /// The shared grid a script reaches with `grid.connect()`.
    grid: Arc<crate::apps::grid_kit::Grid>,
    /// The shared arc, `arc.connect()`.
    arc: Arc<crate::apps::arc_kit::ArcHub>,
}

impl NornsApp {
    pub fn new(_sensitivity: Arc<AtomicF32>, _nav_speed: Arc<AtomicF32>, modbus: Arc<ModBus>, _audio_bus: Arc<AudioBus>, mixer_bus: Arc<MixerBus>) -> Self {
        let (mix_level, ext_mix_level) = mixer_bus.register(APP_NAME, &modbus);
        let roots = script_roots();
        let mut app = Self {
            scripts: discover(&roots),
            selected: 0,
            mode: Mode::Select,
            queue: Arc::new(Queue::default()),
            out: Arc::new(Mutex::new(HostOut::default())),
            running: None,
            pending_release: Vec::new(),
            prev_grid: [false; 16],
            param_sel: 0,
            shown: vec![0; W * H],
            mix_level,
            ext_mix_level,
            peak: Arc::new(AtomicF32::new(0.0)),
            roots,
            note_out: None,
            grid: crate::apps::grid_kit::grid(),
            arc: crate::apps::arc_kit::arc(),
        };
        // Boot straight into a script (like norns resuming its last one).
        if !cfg!(test) {
            if let Ok(name) = std::env::var("PORTAMAX_NORNS_SCRIPT") {
                if let Some(i) = app.scripts.iter().position(|s| s.name == name) {
                    app.selected = i;
                    app.load(i);
                    // give it a moment to draw its first screen
                    let t0 = std::time::Instant::now();
                    while app.out.lock().unwrap().frames == 0 && t0.elapsed() < std::time::Duration::from_millis(500) {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
            }
        }
        app
    }

    /// Lets a script's MIDI notes play other apps (see note_bus.rs).
    pub fn with_notes(mut self, bus: Option<Arc<crate::note_bus::NoteBus>>) -> Self {
        if let Some(bus) = bus {
            self.note_out = Some(bus.register_source_routed("Norns", crate::note_bus::NONE));
        }
        self
    }

    #[cfg(test)]
    fn with_roots(mut self, roots: Vec<(PathBuf, bool)>) -> Self {
        self.scripts = discover(&roots);
        self.roots = roots;
        self
    }

    #[allow(dead_code)] // tests, Slint GUI
    pub fn mode(&self) -> Mode {
        self.mode
    }

    fn send(&self, e: Event) {
        if let Some(r) = &self.running {
            let _ = r.tx.send(e);
        }
    }

    fn stop(&mut self) {
        if let Some(mut r) = self.running.take() {
            let _ = r.tx.send(Event::Quit);
            if let Some(t) = r.thread.take() {
                let _ = t.join();
            }
        }
        self.mode = Mode::Select;
        self.scripts = discover(&self.roots);
    }

    fn load(&mut self, i: usize) {
        self.stop();
        let Some(s) = self.scripts.get(i).cloned() else { return };
        let (tx, rx) = channel();
        let queue = Arc::clone(&self.queue);
        let out = Arc::clone(&self.out);
        {
            let mut o = out.lock().unwrap();
            *o = HostOut::default();
        }
        let code_dir = s.path.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or_default();
        let path = s.path.clone();
        let grid = Arc::clone(&self.grid);
        let arc = Arc::clone(&self.arc);
        let thread = std::thread::Builder::new().name(format!("norns:{}", s.name)).spawn(move || host::run(path, code_dir, queue, out, rx, grid, arc)).ok();
        self.running = Some(Running { tx, thread, name: s.name });
        self.mode = Mode::Play;
    }

    pub fn script_name(&self) -> String {
        self.running.as_ref().map_or_else(String::new, |r| r.name.clone())
    }

    fn key(&mut self, n: i32, z: i32) {
        self.send(Event::Key(n, z));
    }

    fn render_select(&self) -> Vec<u8> {
        let mut s = Screen::new();
        s.level(4);
        s.move_to(0.0, 8.0);
        s.text("SELECT");
        if self.scripts.is_empty() {
            s.level(15);
            s.move_to(0.0, 28.0);
            s.text("no scripts found");
            s.level(4);
            s.move_to(0.0, 40.0);
            s.text("copy them to SD:");
            s.move_to(0.0, 50.0);
            s.text("norns/dust/code/");
            return s.px;
        }
        let first = self.selected.saturating_sub(2).min(self.scripts.len().saturating_sub(5));
        for (row, i) in (first..self.scripts.len()).take(5).enumerate() {
            s.level(if i == self.selected { 15 } else { 4 });
            s.move_to(0.0, 20.0 + row as f32 * 10.0);
            s.text(&self.scripts[i].name);
        }
        s.px
    }

    fn render_params(&self, rows: &[(String, String, String, usize)]) -> Vec<u8> {
        let mut s = Screen::new();
        s.level(4);
        s.move_to(0.0, 8.0);
        s.text("PARAMS");
        let sel = self.param_sel.min(rows.len().saturating_sub(1));
        let first = sel.saturating_sub(2).min(rows.len().saturating_sub(5));
        for (row, i) in (first..rows.len()).take(5).enumerate() {
            let (kind, name, value, _) = &rows[i];
            let y = 20.0 + row as f32 * 10.0;
            let header = kind == "separator" || kind == "group";
            s.level(if i == sel { 15 } else if header { 2 } else { 4 });
            s.move_to(if header { 0.0 } else { 4.0 }, y);
            s.text(&if header { format!("{} {}", if kind == "group" { ">" } else { "-" }, name.to_uppercase()) } else { name.clone() });
            if !header {
                s.move_to(127.0, y);
                s.text_right(value);
            }
        }
        s.px
    }

    /// The picture for this frame.
    fn refresh(&mut self) {
        self.shown = match self.mode {
            Mode::Select => self.render_select(),
            Mode::Params => {
                let rows = self.out.lock().unwrap().params.clone();
                self.render_params(&rows)
            }
            Mode::Play => {
                let o = self.out.lock().unwrap();
                let mut px = if o.frame.len() == W * H { o.frame.clone() } else { vec![0; W * H] };
                if let Some(e) = &o.error {
                    // like norns: the error on screen over the script
                    let mut s = Screen::from_pixels(std::mem::take(&mut px));
                    s.level(0);
                    s.rect(0.0, 44.0, 128.0, 20.0);
                    s.fill();
                    s.level(15);
                    s.move_to(0.0, 52.0);
                    s.text("error:");
                    s.move_to(0.0, 61.0);
                    s.text(&e.chars().take(25).collect::<String>());
                    px = s.px;
                }
                px
            }
        };
    }

    /// The norns frame as RGBA, white-on-black like the OLED.
    #[allow(dead_code)] // Slint GUI only
    pub fn frame_rgba(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(W * H * 4);
        for &l in &self.shown {
            let c = (l as f32 / 15.0 * 255.0) as u8;
            v.extend_from_slice(&[c, c, c, 255]);
        }
        v
    }

    pub fn status(&self) -> String {
        let o = self.out.lock().unwrap();
        match self.mode {
            Mode::Select => format!("{} scripts", self.scripts.len()),
            _ => {
                let last = o.log.last().cloned().unwrap_or_default();
                format!("{}  engine: {}  {}", self.script_name(), if o.engine.is_empty() { "-" } else { &o.engine }, last)
            }
        }
    }

    fn handle_pads(&mut self, grid: &[bool; 16]) {
        for i in 0..16 {
            let (now, was) = (grid[i], self.prev_grid[i]);
            if now == was {
                continue;
            }
            if let Some(k) = PAD_K.iter().position(|&p| p == i) {
                self.key(k as i32 + 1, now as i32);
            } else if i < NOTE_PADS.len() {
                let n = NOTE_PADS[i];
                self.send(Event::Midi(if now { vec![0x90, n, 100] } else { vec![0x80, n, 0] }));
            } else if now {
                match i {
                    PAD_E1_DOWN => self.send(Event::Enc(1, -1)),
                    PAD_E1_UP => self.send(Event::Enc(1, 1)),
                    PAD_PARAMS => self.toggle_params(),
                    PAD_SELECT => self.stop(),
                    _ => {}
                }
            }
        }
        self.prev_grid = *grid;
    }

    fn toggle_params(&mut self) {
        if self.running.is_none() {
            return;
        }
        self.mode = if self.mode == Mode::Params { Mode::Play } else { Mode::Params };
        self.send(Event::WantParams(self.mode == Mode::Params));
    }
}

impl Drop for NornsApp {
    fn drop(&mut self) {
        self.stop();
    }
}

impl App for NornsApp {
    fn tick(&mut self, input: &Input) {
        for k in std::mem::take(&mut self.pending_release) {
            self.key(k, 0);
        }
        match self.mode {
            Mode::Select => {
                let d = (input.knob1 + input.knob2).signum() + if input.nav_down { 1 } else { 0 } - if input.nav_up { 1 } else { 0 };
                if !self.scripts.is_empty() {
                    self.selected = (self.selected as i32 + d).clamp(0, self.scripts.len() as i32 - 1) as usize;
                }
                if input.knob1_press || input.knob2_press || input.nav_select {
                    self.load(self.selected);
                }
                self.prev_grid = input.grid;
            }
            Mode::Play => {
                let shift = input.grid[PAD_E1_SHIFT];
                // No encoders on the device: D-pad up/down is E2 (up turns
                // it clockwise) and left/right arrives in knob2 as E3.
                let e2 = input.knob1 - input.navigation_steps;
                if e2 != 0 {
                    self.send(Event::Enc(if shift { 1 } else { 2 }, e2));
                }
                if input.knob2 != 0 {
                    self.send(Event::Enc(if shift { 1 } else { 3 }, input.knob2));
                }
                for (pressed, k) in [(input.knob1_press, 2), (input.knob2_press, 3)] {
                    if pressed {
                        self.key(k, 1);
                        self.pending_release.push(k);
                    }
                }
                self.handle_pads(&input.grid);
            }
            Mode::Params => {
                let n = self.out.lock().unwrap().params.len();
                if n > 0 {
                    let d = input.knob1.signum() + input.navigation_steps.signum();
                    self.param_sel = (self.param_sel as i32 + d).clamp(0, n as i32 - 1) as usize;
                }
                if input.knob2 != 0 {
                    let idx = self.out.lock().unwrap().params.get(self.param_sel).map(|r| r.3);
                    if let Some(idx) = idx {
                        self.send(Event::ParamDelta(idx, input.knob2));
                    }
                }
                if input.knob1_press {
                    self.toggle_params();
                }
                self.handle_pads(&input.grid);
            }
        }
        self.refresh();
    }

    fn on_exit(&mut self) {
        // Scripts keep running (like norns) until another is picked; only
        // let go of held keys.
        for (k, &p) in PAD_K.iter().enumerate() {
            if self.prev_grid[p] {
                self.key(k as i32 + 1, 0);
            }
        }
        self.prev_grid = [false; 16];
    }

    fn draw(&mut self, fb: &mut FrameBuffer) {
        fb.clear(BG).ok();
        let style = MonoTextStyle::new(&crate::spleen_fonts::SPLEEN_8X16, INK);
        Text::new(APP_NAME, Point::new(16, 24), style).draw(fb).ok();
        // the 128x64 screen at 4x
        let (ox, oy) = (64, 40);
        Rectangle::new(Point::new(ox - 2, oy - 2), Size::new(516, 260)).into_styled(PrimitiveStyle::with_stroke(DIM, 1)).draw(fb).ok();
        for y in 0..H {
            for x in 0..W {
                let l = self.shown[y * W + x];
                if l == 0 {
                    continue;
                }
                let c = Rgb565::new((l as u32 * 31 / 15) as u8, (l as u32 * 63 / 15) as u8, (l as u32 * 31 / 15) as u8);
                Rectangle::new(Point::new(ox + x as i32 * 4, oy + y as i32 * 4), Size::new(4, 4)).into_styled(PrimitiveStyle::with_fill(c)).draw(fb).ok();
            }
        }
        let small = MonoTextStyle::new(&crate::spleen_fonts::SPLEEN_6X12, DIM);
        Text::new(&self.status(), Point::new(16, 316), small).draw(fb).ok();
        Text::new("U/D E2  L/R E3  SELECT K2  pads 13-15 K1-3  pad 11 PARAMS  pad 12 list", Point::new(16, 334), small).draw(fb).ok();
    }

    fn audio_processor(&mut self) -> Option<Box<dyn AudioProcessor>> {
        let mut p = engine::Processor::new(Arc::clone(&self.queue), Arc::clone(&self.mix_level), Arc::clone(&self.ext_mix_level), Arc::clone(&self.peak));
        if let Some(out) = self.note_out.take() {
            p.notes = out;
        }
        Some(Box::new(p))
    }

    /// A script makes sound off screen too (it's a running program).
    fn needs_background_audio(&self) -> bool {
        self.running.is_some()
    }

    fn running(&self) -> Option<bool> {
        Some(self.running.is_some())
    }

    /// F3: stop the script (back to the list), or start the selected one.
    fn toggle_running(&mut self) {
        if self.running.is_some() {
            self.stop();
        } else {
            self.load(self.selected);
        }
    }

    fn grid_mode_label(&self) -> Option<&'static str> {
        Some(match self.mode {
            Mode::Select => "SCRIPTS",
            Mode::Play => "SCREEN",
            Mode::Params => "PARAMS",
        })
    }

    fn toggle_grid_mode(&mut self) {
        self.toggle_params();
    }

    fn grid_led_overlay(&self) -> [PadColor; 16] {
        let mut c = [PadColor::Off; 16];
        if self.mode == Mode::Select {
            return c;
        }
        for p in c.iter_mut().take(NOTE_PADS.len()) {
            *p = PadColor::Blue;
        }
        c[PAD_E1_DOWN] = PadColor::Yellow;
        c[PAD_E1_UP] = PadColor::Yellow;
        c[PAD_PARAMS] = if self.mode == Mode::Params { PadColor::Green } else { PadColor::Yellow };
        c[PAD_SELECT] = PadColor::Red;
        for p in PAD_K {
            c[p] = PadColor::Green;
        }
        c[PAD_E1_SHIFT] = PadColor::Yellow;
        c
    }

    fn slint_rows(&self) -> Vec<(String, String, bool)> {
        match self.mode {
            Mode::Select => self.scripts.iter().map(|s| (s.name.clone(), if s.bundled { "built in".into() } else { "SD card".into() }, false)).collect(),
            Mode::Params => self.out.lock().unwrap().params.iter().map(|(k, n, v, _)| (n.clone(), v.clone(), k == "separator" || k == "group")).collect(),
            Mode::Play => vec![(self.script_name(), String::new(), true)],
        }
    }

    fn slint_selected(&self) -> usize {
        match self.mode {
            Mode::Select => self.selected,
            Mode::Params => self.param_sel,
            Mode::Play => 0,
        }
    }

    fn slint_extra(&mut self) -> crate::app::SlintExtra {
        self.refresh();
        crate::app::SlintExtra::Norns(crate::app::NornsExtra {
            frame_rgba: self.frame_rgba(),
            title: match self.mode {
                Mode::Select => "SELECT".into(),
                _ => self.script_name(),
            },
            mode: format!("{:?}", self.mode).to_uppercase(),
            status: self.status(),
            peak: self.peak.get().clamp(0.0, 1.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn app_with(dir: &std::path::Path) -> NornsApp {
        NornsApp::new(
            Arc::new(AtomicF32::new(0.1)),
            Arc::new(AtomicF32::new(3.0)),
            Arc::new(ModBus::new()),
            Arc::new(AudioBus::new()),
            Arc::new(MixerBus::new()),
        )
        .with_roots(vec![(dir.to_path_buf(), false)])
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pmx-norns-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_script(root: &std::path::Path, name: &str, src: &str) {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("{name}.lua")), src).unwrap();
    }

    fn wait_until(mut f: impl FnMut() -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(3) {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn the_bundled_script_is_listed() {
        let names: Vec<String> = discover(&script_roots()).into_iter().filter(|s| s.bundled).map(|s| s.name).collect();
        assert!(names.contains(&"tidepool".to_string()), "{names:?}");
    }

    #[test]
    fn a_script_draws_reacts_to_encoders_and_clocks_notes_into_polyperc() {
        let root = tmp("basic");
        write_script(
            &root,
            "probe",
            r#"
engine.name = 'PolyPerc'
local MusicUtil = require "musicutil"
count = 0
turned = 0
function init()
  params:add_control("cut", "cutoff", controlspec.new(50, 5000, 'exp', 0, 800, 'hz'))
  params:set_action("cut", function(x) engine.cutoff(x) end)
  params:add_number("n", "n", 0, 8, 3)
  params:default()
  clock.run(function()
    while true do
      clock.sync(1/4)
      count = count + 1
      engine.hz(MusicUtil.note_num_to_freq(60 + count % 8))
      redraw()
    end
  end)
end
function enc(n, d) turned = turned + d * n redraw() end
function redraw()
  screen.clear()
  screen.level(15)
  screen.move(0, 10)
  screen.text("turned " .. turned)
  screen.rect(0, 20, count % 100, 4)
  screen.fill()
  screen.update()
end
"#,
        );
        let mut a = app_with(&root);
        let mut p = a.audio_processor().unwrap();
        assert_eq!(a.scripts.len(), 1);
        a.tick(&Input { knob1_press: true, ..Input::default() });
        assert_eq!(a.mode(), Mode::Play);
        assert!(wait_until(|| a.out.lock().unwrap().frames > 2), "clocked redraws arrive: {:?}", a.out.lock().unwrap().log);
        assert_eq!(a.out.lock().unwrap().engine, "PolyPerc");
        a.tick(&Input { knob2: 2, ..Input::default() }); // E3 +2 -> turned 6
        assert!(wait_until(|| {
            a.tick(&Input::default());
            a.shown.iter().filter(|&&l| l > 0).count() > 20
        }));
        // the notes reach the engine
        let mut buf = vec![0.0f32; 1024];
        let mut energy = 0.0;
        for _ in 0..50 {
            buf.fill(0.0);
            p.process(&mut buf, 2, 48_000.0);
            energy += buf.iter().map(|x| x * x).sum::<f32>();
            std::thread::sleep(Duration::from_millis(4));
        }
        assert!(energy > 0.1, "PolyPerc sounded ({energy})");
        // params page lists and edits params
        a.tick(&Input { grid: { let mut g = [false; 16]; g[PAD_PARAMS] = true; g }, ..Input::default() });
        assert_eq!(a.mode(), Mode::Params);
        assert!(wait_until(|| a.out.lock().unwrap().params.iter().any(|r| r.1 == "cutoff")));
        assert!(a.out.lock().unwrap().params.iter().any(|r| r.1 == "tempo"), "system clock params are there");
        a.stop();
        assert_eq!(a.mode(), Mode::Select);
        assert!(a.out.lock().unwrap().error.is_none(), "{:?}", a.out.lock().unwrap().log);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_script_plays_the_grid_and_lets_go_of_it_when_it_stops() {
        let root = tmp("grid");
        write_script(
            &root,
            "gridprobe",
            r#"
local g = grid.connect()
function init()
  g:all(0)
  g:led(2, 3, 9)
  g:refresh()
  redraw()
end
g.key = function(x, y, z)
  if z == 1 then g:led(x, y, 15) g:refresh() end
end
function redraw()
  screen.clear()
  screen.move(0, 10)
  screen.text(g.cols .. "x" .. g.rows .. " " .. g.name)
  screen.update()
end
"#,
        );
        let mut a = app_with(&root);
        let grid = Arc::clone(&a.grid);
        grid.register("Other");
        a.tick(&Input { knob1_press: true, ..Input::default() });
        let lit = |x: usize, y: usize| {
            let s = grid.snapshot();
            s.leds[y * s.cols + x]
        };
        assert!(wait_until(|| lit(1, 2) == 9), "g:led(2, 3, 9) lights key (1, 2): {:?}", a.out.lock().unwrap().log);
        assert_eq!(grid.focus().as_deref(), Some(host::GRID_CLIENT), "connecting takes the grid");
        grid.press(5, 4, true);
        assert!(wait_until(|| lit(5, 4) == 15), "g.key arrives 1-based and lights the key");
        grid.press(5, 4, false);
        a.stop();
        assert!(wait_until(|| grid.snapshot().leds.iter().all(|&l| l == 0)), "a stopped script leaves the grid dark");
        assert!(a.out.lock().unwrap().error.is_none(), "{:?}", a.out.lock().unwrap().log);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_script_turns_the_arc_and_draws_its_rings() {
        let root = tmp("arc");
        write_script(
            &root,
            "arcprobe",
            r#"
local a = arc.connect()
local pos = {0, 0, 0, 0}
local pushed = 0
local function draw()
  a:all(0)
  for n = 1, 4 do a:led(n, pos[n] + 1, 15) end
  a:segment(4, 0, math.pi, 5 + pushed)
  a:refresh()
end
function init() draw() end
a.delta = function(n, d) pos[n] = (pos[n] + d) % 64; draw() end
a.key = function(n, z) pushed = z * 10; draw() end
"#,
        );
        let mut a = app_with(&root);
        let arc = Arc::clone(&a.arc);
        arc.register("Other");
        a.tick(&Input { knob1_press: true, ..Input::default() });
        let led = |n: usize, i: usize| arc.snapshot().leds[n][i];
        assert!(wait_until(|| led(0, 0) == 15), "a:led(1, 1, 15) lights ring 1's top LED: {:?}", a.out.lock().unwrap().log);
        assert_eq!(arc.focus().as_deref(), Some(host::GRID_CLIENT), "connecting takes the arc");
        assert_eq!((led(3, 10), led(3, 40)), (5, 0), "a half-ring segment on ring 4");
        arc.turn(1, 3);
        assert!(wait_until(|| led(1, 3) == 15), "a.delta(2, 3) moves ring 2's light");
        arc.turn(1, -5);
        assert!(wait_until(|| led(1, 62) == 15), "and back past the top");
        arc.key(3, true);
        assert!(wait_until(|| led(3, 10) == 15), "a.key arrives");
        a.stop();
        assert!(wait_until(|| arc.snapshot().leds.iter().flatten().all(|&l| l == 0)), "a stopped script leaves the arc dark");
        assert!(a.out.lock().unwrap().error.is_none(), "{:?}", a.out.lock().unwrap().log);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The bundled grid and arc scripts light their hardware and take
    /// presses and turns without a Lua error, at a small and a big size.
    #[test]
    fn the_grid_and_arc_scripts_play_their_hardware() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/norns/code");
        let grid_scripts = ["gridsteps", "bouncers", "automata", "plinko", "charge", "isogrid", "strumharp", "quickhands", "blocks"];
        let arc_scripts = ["arcarp", "shoals", "scrubber"];
        for (name, size) in grid_scripts.iter().flat_map(|n| [(*n, (8, 16)), (*n, (16, 16))]).chain(arc_scripts.iter().map(|n| (*n, (8, 16)))) {
            let mut a = app_with(&root);
            a.grid.set_size(size.0, size.1);
            a.selected = a.scripts.iter().position(|s| s.name == name).unwrap();
            a.tick(&Input { knob1_press: true, ..Input::default() });
            let grid = Arc::clone(&a.grid);
            let arc = Arc::clone(&a.arc);
            let is_arc = arc_scripts.contains(&name);
            let lit = || if is_arc { arc.snapshot().leds.iter().flatten().any(|&l| l > 0) } else { grid.snapshot().leds.iter().any(|&l| l > 0) };
            assert!(wait_until(lit), "{name} {size:?}: lights its {}: {:?}", if is_arc { "arc" } else { "grid" }, a.out.lock().unwrap().log);
            for k in 0..12usize {
                if is_arc {
                    arc.turn(k % 4, if k % 2 == 0 { 40 } else { -25 });
                    arc.key(k % 4, k % 3 == 0);
                } else {
                    let (x, y) = ((k * 5) % size.1, (k * 3) % size.0);
                    grid.press(x, y, true);
                    std::thread::sleep(Duration::from_millis(5));
                    grid.press(x, y, false);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            std::thread::sleep(Duration::from_millis(100));
            let o = a.out.lock().unwrap();
            assert!(o.error.is_none(), "{name} {size:?}: {:?}\n{:?}", o.error, o.log);
            drop(o);
            a.stop();
        }
    }

    /// Writes the Grid / Arc app's screen with each bundled grid and arc
    /// script running to the folder in PORTAMAX_NORNS_GRID_SHOT.
    #[test]
    #[ignore = "writes screenshots to the folder in PORTAMAX_NORNS_GRID_SHOT"]
    fn grid_and_arc_script_screenshots() {
        let Ok(dir) = std::env::var("PORTAMAX_NORNS_GRID_SHOT") else { return };
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/norns/code");
        let save = |fb: &crate::display::FrameBuffer, name: &str| {
            let mut out = b"P6\n640 360\n255\n".to_vec();
            out.extend(fb.buffer().iter().flat_map(|px| [(px >> 16) as u8, (px >> 8) as u8, *px as u8]));
            std::fs::write(std::path::Path::new(&dir).join(format!("{name}.ppm")), out).unwrap();
        };
        for name in ["bouncers", "automata", "plinko", "charge", "isogrid", "strumharp", "quickhands", "blocks", "shoals", "scrubber", "arcarp"] {
            let mut a = app_with(&root);
            a.selected = a.scripts.iter().position(|s| s.name == name).unwrap();
            a.tick(&Input { knob1_press: true, ..Input::default() });
            std::thread::sleep(Duration::from_millis(2500));
            let mut fb = crate::display::FrameBuffer::new();
            if ["shoals", "scrubber", "arcarp"].contains(&name) {
                crate::apps::arc::ArcApp::new(Arc::clone(&a.arc), Arc::new(AtomicF32::new(3.0))).draw(&mut fb);
            } else {
                crate::apps::grid::GridApp::new(Arc::clone(&a.grid), Arc::new(AtomicF32::new(3.0))).draw(&mut fb);
            }
            save(&fb, name);
            a.stop();
        }
    }

    #[test]
    fn a_script_error_shows_on_screen_instead_of_crashing() {
        let root = tmp("err");
        write_script(&root, "broken", "function init() local x = nil; x.y = 1 end");
        let mut a = app_with(&root);
        a.tick(&Input { knob1_press: true, ..Input::default() });
        assert!(wait_until(|| a.out.lock().unwrap().error.is_some()));
        a.tick(&Input::default());
        assert!(a.shown.iter().any(|&l| l == 15), "error text drawn");
        a.stop();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Every bundled script must start, draw, take encoders, keys and
    /// pad notes without a Lua error, and make sound.
    #[test]
    fn every_bundled_script_runs_and_plays() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/norns/code");
        let mut names: Vec<String> = discover(&[(root.clone(), true)]).into_iter().map(|s| s.name).collect();
        assert!(names.len() >= 121, "tidepool plus the 120 others: {names:?}");
        // PORTAMAX_NORNS_ONLY=a,b,c checks just those scripts (for writing
        // new ones without waiting on all of them).
        if let Ok(only) = std::env::var("PORTAMAX_NORNS_ONLY") {
            let want: Vec<&str> = only.split(',').map(str::trim).collect();
            names.retain(|n| want.contains(&n.as_str()));
            assert_eq!(names.len(), want.len(), "unknown script in PORTAMAX_NORNS_ONLY: {want:?} vs {names:?}");
        }
        let mut silent = Vec::new();
        for name in &names {
            let mut a = app_with(&root);
            a.selected = a.scripts.iter().position(|s| &s.name == name).unwrap();
            a.tick(&Input { knob1_press: true, ..Input::default() });
            let mut p = a.audio_processor().unwrap();
            assert!(wait_until(|| a.out.lock().unwrap().frames > 2), "{name}: no frames: {:?}", a.out.lock().unwrap().log);
            let pad = |i: usize| Input { grid: std::array::from_fn(|g| g == i), ..Input::default() };
            for input in [
                Input { knob1: 1, ..Input::default() },
                Input { knob2: -1, ..Input::default() },
                pad(PAD_K[2]),
                Input::default(),
                pad(PAD_K[2]),
                Input::default(),
                pad(PAD_K[1]),
                Input::default(),
                pad(0),
                Input::default(),
                pad(4),
                Input::default(),
            ] {
                a.tick(&input);
                std::thread::sleep(Duration::from_millis(15));
            }
            let mut energy = 0.0;
            let mut buf = vec![0.0f32; 1024];
            for _ in 0..150 {
                buf.fill(0.0);
                p.process(&mut buf, 2, 48_000.0);
                assert!(buf.iter().all(|x| x.is_finite()), "{name}: non-finite audio");
                energy += buf.iter().map(|x| x * x).sum::<f32>();
                std::thread::sleep(Duration::from_millis(5));
            }
            let o = a.out.lock().unwrap();
            assert!(o.error.is_none(), "{name}: {:?}\n{:?}", o.error, o.log);
            drop(o);
            if energy < 0.01 {
                silent.push(name.clone());
            }
            a.stop();
        }
        assert!(silent.is_empty(), "these scripts made no sound: {silent:?}");
    }

    /// The real test of compatibility: awake, if a copy is present (it is
    /// user-supplied, not bundled). Set PORTAMAX_NORNS_AWAKE to its folder.
    #[test]
    fn awake_runs_if_available() {
        let Some(dir) = std::env::var_os("PORTAMAX_NORNS_AWAKE") else { return };
        let dir = PathBuf::from(dir);
        let root = dir.parent().unwrap().to_path_buf();
        let mut a = app_with(&root);
        let i = a.scripts.iter().position(|s| s.name == "awake").expect("awake listed");
        a.selected = i;
        a.tick(&Input { knob1_press: true, ..Input::default() });
        let mut p = a.audio_processor().unwrap();
        assert!(wait_until(|| a.out.lock().unwrap().frames > 3), "{:?}", a.out.lock().unwrap().log);
        let mut energy = 0.0;
        let mut buf = vec![0.0f32; 1024];
        for _ in 0..80 {
            buf.fill(0.0);
            p.process(&mut buf, 2, 48_000.0);
            energy += buf.iter().map(|x| x * x).sum::<f32>();
            std::thread::sleep(Duration::from_millis(5));
        }
        // E1 to SOUND mode, then E2 changes cutoff
        a.tick(&Input { knob1: 2, grid: { let mut g = [false; 16]; g[PAD_E1_SHIFT] = true; g }, ..Input::default() });
        a.tick(&Input { knob1: 3, ..Input::default() });
        std::thread::sleep(Duration::from_millis(50));
        let o = a.out.lock().unwrap();
        assert!(o.error.is_none(), "{:?}", o.log);
        assert!(energy > 0.05, "awake played ({energy})");
        drop(o);
        a.stop();
    }
}

/// Builds the app from the shared services (see `AppContext` and
/// registry.rs) -- the one entry point the app registry needs, so this
/// file can be dropped in or removed without editing anything else.
pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(NornsApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()).with_notes(ctx.try_get()))
}
