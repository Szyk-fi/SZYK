//! The script host: one thread per running script, owning its Lua state
//! the way norns' own event loop does. It turns encoder/key/MIDI events
//! into calls to the script's `enc` / `key` / MIDI handlers, runs clocks
//! and metros (`_px_tick`, ~1 kHz, so `clock.sync` lands within about a
//! millisecond), and publishes the screen whenever the script calls
//! `screen.update()`. Sound leaves through the command queue to the
//! audio processor (engine.rs).

use super::engine::{Cmd, Queue};
use super::screen::Screen;
use crate::apps::arc_kit::{self, ArcHub, Rings};
use crate::apps::grid_kit::{Grid, Leds};
use mlua::{Function, Lua, MultiValue, Table, Value, Variadic};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PRELUDE: &str = include_str!("lua/prelude.lua");
const MUSICUTIL: &str = include_str!("lua/musicutil.lua");

/// Engines this cartridge can run. Others fall back to PolyPerc with a
/// warning (the script still runs; its engine commands are ignored).
pub const ENGINES: &[&str] = &["PolyPerc"];

pub enum Event {
    Enc(i32, i32),
    Key(i32, i32),
    Midi(Vec<u8>),
    ParamDelta(usize, i32),
    /// The Params page is open: keep the row list fresh.
    WantParams(bool),
    Quit,
}

/// What the host shows the UI.
#[derive(Default)]
pub struct HostOut {
    pub frame: Vec<u8>,
    /// Bumped on every `screen.update()`.
    pub frames: u64,
    pub log: Vec<String>,
    pub error: Option<String>,
    pub engine: String,
    /// Params page rows: (kind, name, value, param index).
    pub params: Vec<(String, String, String, usize)>,
    pub running: bool,
}

impl HostOut {
    fn say(&mut self, s: String) {
        eprintln!("norns: {s}");
        self.log.push(s);
        if self.log.len() > 50 {
            self.log.remove(0);
        }
    }
}

/// The grid as a script sees it: vport 1 is Portamax's grid (the screen
/// one, and a real grid when one is plugged in). The script draws into
/// `leds` and `g:refresh()` shows it; presses come back through
/// `_px_grid_key`. `g:rotation(r)` turns the script's view a quarter turn
/// at a time, so a script made for a vertical grid still fits.
///
/// The arc the same way: vport 1 is Portamax's arc, `a:refresh()` shows
/// `rings`, turns and pushes come back through `_px_arc_delta` / `_px_arc_key`.
pub struct GridHost {
    grid: Arc<Grid>,
    leds: Leds,
    rot: u8,
    connected: bool,
    arc: Arc<ArcHub>,
    rings: Rings,
    arc_connected: bool,
}

/// The name the grid knows a running script by.
pub const GRID_CLIENT: &str = "Norns";

impl GridHost {
    pub fn new(grid: Arc<Grid>, arc: Arc<ArcHub>) -> GridHost {
        let (rows, cols) = grid.size();
        GridHost { grid, leds: Leds::new(rows, cols), rot: 0, connected: false, arc, rings: [[0; arc_kit::LEDS]; arc_kit::MAX_ENCODERS], arc_connected: false }
    }

    /// A 1-based ring number -> its index, if the arc has it.
    fn ring(&self, n: f64) -> Option<usize> {
        let i = n as i64 - 1;
        (0..self.arc.encoders() as i64).contains(&i).then_some(i as usize)
    }

    fn arc_name(&self) -> String {
        match self.arc.device() {
            Some(d) => format!("{} {}", d.kind, d.id),
            None => "portamax arc".into(),
        }
    }

    /// (columns, rows) as the script sees them.
    fn dims(&self) -> (usize, usize) {
        let (r, c) = (self.leds.rows, self.leds.cols);
        if self.rot % 2 == 1 { (r, c) } else { (c, r) }
    }

    /// Script (0-based) -> grid coordinates.
    fn to_grid(&self, x: usize, y: usize) -> Option<(usize, usize)> {
        let (sc, sr) = self.dims();
        if x >= sc || y >= sr {
            return None;
        }
        let (c, r) = (self.leds.cols, self.leds.rows);
        Some(match self.rot {
            1 => (c - 1 - y, x),
            2 => (c - 1 - x, r - 1 - y),
            3 => (y, r - 1 - x),
            _ => (x, y),
        })
    }

    /// Grid -> script (0-based) coordinates.
    fn from_grid(&self, gx: usize, gy: usize) -> (usize, usize) {
        let (c, r) = (self.leds.cols, self.leds.rows);
        match self.rot {
            1 => (gy, c - 1 - gx),
            2 => (c - 1 - gx, r - 1 - gy),
            3 => (r - 1 - gy, gx),
            _ => (gx, gy),
        }
    }

    /// Follows a change of grid size (the Grid app, or a grid plugged in).
    /// True when it changed.
    fn follow_size(&mut self) -> bool {
        let (rows, cols) = self.grid.size();
        if (rows, cols) == (self.leds.rows, self.leds.cols) {
            return false;
        }
        self.leds = Leds::new(rows, cols);
        true
    }

    fn name(&self) -> String {
        match self.grid.device() {
            Some(d) => format!("{} {}", d.kind, d.id),
            None => "portamax grid".into(),
        }
    }
}

fn nums(args: &Variadic<Value>) -> Vec<f32> {
    args.iter()
        .filter_map(|v| match v {
            Value::Integer(i) => Some(*i as f32),
            Value::Number(n) => Some(*n as f32),
            Value::Boolean(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        })
        .collect()
}

fn f(v: Option<f32>) -> f32 {
    v.unwrap_or(0.0)
}

/// Builds the norns environment and loads `script` (not yet `init()`ed).
pub fn setup(lua: &Lua, script: &Path, code_dir: &Path, queue: Arc<Queue>, out: Arc<Mutex<HostOut>>, screen: Rc<RefCell<Screen>>, grid: Rc<RefCell<GridHost>>) -> mlua::Result<()> {
    let g = lua.globals();
    let px = lua.create_table()?;
    let start = Instant::now();
    px.set("now", lua.create_function(move |_, ()| Ok(start.elapsed().as_secs_f64()))?)?;
    {
        let out = Arc::clone(&out);
        px.set("print", lua.create_function(move |_, s: String| {
            out.lock().unwrap().say(s);
            Ok(())
        })?)?;
    }
    {
        let out = Arc::clone(&out);
        px.set("error", lua.create_function(move |_, s: String| {
            let mut o = out.lock().unwrap();
            o.say(format!("error: {s}"));
            o.error = Some(s);
            Ok(())
        })?)?;
    }
    for (name, kind) in [("engine", 0), ("softcut", 1), ("audio", 2)] {
        let q = Arc::clone(&queue);
        px.set(name, lua.create_function(move |_, (cmd, args): (String, Variadic<Value>)| {
            let a = nums(&args);
            q.push(match kind {
                0 => Cmd::Engine(cmd, a),
                1 => Cmd::Softcut(cmd, a),
                _ => Cmd::Audio(cmd, a),
            });
            Ok(())
        })?)?;
    }
    // MIDI out: notes go on the note bus, to whichever app Norns is
    // routed to (Portal's Notes page); everything else is accepted and
    // ignored so scripts that send it keep running.
    {
        let q = Arc::clone(&queue);
        px.set("midi_out", lua.create_function(move |_, (_port, data): (Value, Value)| {
            if let Value::Table(t) = data {
                let bytes: Vec<u8> = t.sequence_values::<f64>().flatten().map(|v| v.clamp(0.0, 255.0) as u8).collect();
                if bytes.first().is_some_and(|s| matches!(s & 0xF0, 0x80 | 0x90)) {
                    q.push(Cmd::Midi(bytes));
                }
            }
            Ok(())
        })?)?;
    }
    // grid: _px.grid_* back the vport methods in prelude.lua.
    {
        let gh = Rc::clone(&grid);
        px.set("grid_connect", lua.create_function(move |_, ()| {
            let mut g = gh.borrow_mut();
            g.connected = true;
            g.follow_size();
            g.grid.register(GRID_CLIENT);
            // A script that asks for the grid gets it, as on norns.
            g.grid.set_focus(GRID_CLIENT);
            let (c, r) = g.dims();
            Ok((c, r, g.name()))
        })?)?;
        let gh = Rc::clone(&grid);
        px.set("grid_led", lua.create_function(move |_, (x, y, l): (f64, f64, Option<f64>)| {
            let mut g = gh.borrow_mut();
            if x >= 1.0 && y >= 1.0 {
                if let Some((gx, gy)) = g.to_grid(x as usize - 1, y as usize - 1) {
                    g.leds.set(gx, gy, l.unwrap_or(0.0).floor() as i32);
                }
            }
            Ok(())
        })?)?;
        let gh = Rc::clone(&grid);
        px.set("grid_all", lua.create_function(move |_, l: Option<f64>| {
            let mut g = gh.borrow_mut();
            let (r, c) = (g.leds.rows, g.leds.cols);
            let l = l.unwrap_or(0.0).floor() as i32;
            for y in 0..r {
                g.leds.fill_row(y, 0..c, l);
            }
            Ok(())
        })?)?;
        let gh = Rc::clone(&grid);
        px.set("grid_refresh", lua.create_function(move |_, ()| {
            let g = gh.borrow();
            if g.connected {
                g.grid.show(GRID_CLIENT, &g.leds);
            }
            Ok(())
        })?)?;
        let gh = Rc::clone(&grid);
        px.set("grid_rotation", lua.create_function(move |_, r: Option<f64>| {
            let mut g = gh.borrow_mut();
            g.rot = (r.unwrap_or(0.0) as i64).rem_euclid(4) as u8;
            let (c, r) = g.dims();
            Ok((c, r))
        })?)?;
    }
    // arc: _px.arc_* back the vport methods in prelude.lua.
    {
        let gh = Rc::clone(&grid);
        px.set("arc_connect", lua.create_function(move |_, ()| {
            let mut g = gh.borrow_mut();
            g.arc_connected = true;
            g.arc.register(GRID_CLIENT);
            g.arc.set_focus(GRID_CLIENT);
            Ok((g.arc.encoders(), g.arc_name()))
        })?)?;
        let gh = Rc::clone(&grid);
        px.set("arc_led", lua.create_function(move |_, (n, x, l): (f64, f64, Option<f64>)| {
            let mut g = gh.borrow_mut();
            if let Some(r) = g.ring(n) {
                // LED 1 is at the top; 65 is 1 again, as on norns.
                let i = (x as i64 - 1).rem_euclid(arc_kit::LEDS as i64) as usize;
                g.rings[r][i] = l.unwrap_or(0.0).clamp(0.0, 15.0) as u8;
            }
            Ok(())
        })?)?;
        let gh = Rc::clone(&grid);
        px.set("arc_all", lua.create_function(move |_, l: Option<f64>| {
            let mut g = gh.borrow_mut();
            let l = l.unwrap_or(0.0).clamp(0.0, 15.0) as u8;
            g.rings = [[l; arc_kit::LEDS]; arc_kit::MAX_ENCODERS];
            Ok(())
        })?)?;
        let gh = Rc::clone(&grid);
        px.set("arc_segment", lua.create_function(move |_, (n, a1, a2, l): (f64, f64, f64, Option<f64>)| {
            let mut g = gh.borrow_mut();
            if let Some(r) = g.ring(n) {
                arc_kit::draw::segment(&mut g.rings[r], a1, a2, l.unwrap_or(0.0).clamp(0.0, 15.0) as u8);
            }
            Ok(())
        })?)?;
        let gh = Rc::clone(&grid);
        px.set("arc_refresh", lua.create_function(move |_, ()| {
            let g = gh.borrow();
            if g.arc_connected {
                g.arc.show(GRID_CLIENT, &g.rings);
            }
            Ok(())
        })?)?;
    }
    let dirs = lua.create_table()?;
    let script_dir = script.parent().unwrap_or(Path::new(".")).to_path_buf();
    for (i, d) in [script_dir.clone(), code_dir.to_path_buf(), code_dir.parent().unwrap_or(code_dir).to_path_buf()].iter().enumerate() {
        dirs.set(i + 1, d.to_string_lossy().to_string())?;
    }
    px.set("search_dirs", dirs)?;
    g.set("_px", px)?;

    // screen.*
    let scr = lua.create_table()?;
    macro_rules! sfn {
        ($name:literal, $args:ty, |$s:ident, $a:ident| $body:expr) => {{
            let screen = Rc::clone(&screen);
            scr.set($name, lua.create_function(move |_, $a: $args| {
                let mut $s = screen.borrow_mut();
                $body;
                Ok(())
            })?)?;
        }};
    }
    sfn!("clear", (), |s, _a| s.clear());
    sfn!("level", Option<f32>, |s, a| s.level(f(a) as i32));
    sfn!("line_width", Option<f32>, |s, a| s.line_width(f(a)));
    sfn!("move", (Option<f32>, Option<f32>), |s, a| s.move_to(f(a.0), f(a.1)));
    sfn!("move_rel", (Option<f32>, Option<f32>), |s, a| s.move_rel(f(a.0), f(a.1)));
    sfn!("line", (Option<f32>, Option<f32>), |s, a| s.line_to(f(a.0), f(a.1)));
    sfn!("line_rel", (Option<f32>, Option<f32>), |s, a| s.line_rel(f(a.0), f(a.1)));
    sfn!("rect", (Option<f32>, Option<f32>, Option<f32>, Option<f32>), |s, a| s.rect(f(a.0), f(a.1), f(a.2), f(a.3)));
    sfn!("circle", (Option<f32>, Option<f32>, Option<f32>), |s, a| s.circle(f(a.0), f(a.1), f(a.2)));
    sfn!("arc", (Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>), |s, a| s.arc(f(a.0), f(a.1), f(a.2), f(a.3), f(a.4)));
    sfn!("curve", (Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>), |s, a| s.curve(f(a.0), f(a.1), f(a.2), f(a.3), f(a.4), f(a.5)));
    sfn!("curve_rel", (Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>), |s, a| s.curve_rel(f(a.0), f(a.1), f(a.2), f(a.3), f(a.4), f(a.5)));
    sfn!("close", (), |s, _a| s.close());
    sfn!("stroke", (), |s, _a| s.stroke());
    sfn!("fill", (), |s, _a| s.fill());
    sfn!("pixel", (Option<f32>, Option<f32>), |s, a| s.pixel(f(a.0), f(a.1)));
    sfn!("text", Value, |s, a| s.text(&value_text(&a)));
    sfn!("text_right", Value, |s, a| s.text_right(&value_text(&a)));
    sfn!("text_center", Value, |s, a| s.text_center(&value_text(&a)));
    {
        let screen = Rc::clone(&screen);
        let out = Arc::clone(&out);
        scr.set("update", lua.create_function(move |_, ()| {
            let s = screen.borrow();
            let mut o = out.lock().unwrap();
            o.frame.clear();
            o.frame.extend_from_slice(&s.px);
            o.frames += 1;
            Ok(())
        })?)?;
    }
    g.set("screen", scr)?;

    lua.load(PRELUDE).set_name("portamax/prelude.lua").exec()?;

    // require "musicutil" & co.
    let package: Table = g.get("package")?;
    let preload: Table = package.get("preload")?;
    let mu: Function = lua.load(MUSICUTIL).set_name("portamax/musicutil.lua").into_function()?;
    preload.set("musicutil", mu.clone())?;
    preload.set("lib/musicutil", mu)?;
    for (name, global) in [("util", "util"), ("controlspec", "controlspec"), ("paramset", "paramset"), ("tabutil", "tab")] {
        let v: Value = g.get(global)?;
        let getter = lua.create_function(move |_, _: MultiValue| Ok(v.clone()))?;
        preload.set(name, getter.clone())?;
        preload.set(format!("lib/{name}"), getter)?;
    }
    let sd = script_dir.to_string_lossy();
    let cd = code_dir.to_string_lossy();
    let path: String = package.get("path")?;
    package.set("path", format!("{sd}/?.lua;{sd}/lib/?.lua;{cd}/?.lua;{path}"))?;

    let sys: Function = g.get("_px_system_params")?;
    sys.call::<()>(())?;

    let src = std::fs::read_to_string(script).map_err(mlua::Error::external)?;
    lua.load(&src).set_name(script.file_name().map_or("script".into(), |n| n.to_string_lossy().to_string())).exec()?;
    Ok(())
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string_lossy().to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                format!("{n}")
            }
        }
        Value::Boolean(b) => b.to_string(),
        Value::Nil => "nil".into(),
        _ => String::new(),
    }
}

/// Calls global `name` if the script defined it.
fn call_global(lua: &Lua, name: &str, args: impl mlua::IntoLuaMulti) -> mlua::Result<()> {
    if let Ok(func) = lua.globals().get::<Function>(name) {
        func.call::<()>(args)?;
    }
    Ok(())
}

fn midi_in(lua: &Lua, data: Vec<u8>) -> mlua::Result<()> {
    let midi: Table = lua.globals().get("midi")?;
    let vports: Table = midi.get("vports")?;
    let dev: Table = vports.get(1)?;
    if let Ok(ev) = dev.get::<Function>("event") {
        let t = lua.create_table()?;
        for (i, b) in data.iter().enumerate() {
            t.set(i + 1, *b)?;
        }
        ev.call::<()>(t)?;
    }
    Ok(())
}

fn params_rows(lua: &Lua) -> mlua::Result<Vec<(String, String, String, usize)>> {
    let list: Function = lua.globals().get("_px_params_list")?;
    let rows: Table = list.call(())?;
    let mut out = Vec::new();
    for row in rows.sequence_values::<Table>() {
        let row = row?;
        out.push((row.get(1)?, row.get(2)?, row.get(3)?, row.get::<usize>(4)?));
    }
    Ok(out)
}

/// Runs a script until `Quit` (or the UI goes away), then its `cleanup`.
pub fn run(script: PathBuf, code_dir: PathBuf, queue: Arc<Queue>, out: Arc<Mutex<HostOut>>, rx: Receiver<Event>, grid: Arc<Grid>, arc: Arc<ArcHub>) {
    let lua = Lua::new();
    let screen = Rc::new(RefCell::new(Screen::new()));
    let gh = Rc::new(RefCell::new(GridHost::new(grid, arc)));
    queue.push(Cmd::Reset);
    {
        let mut o = out.lock().unwrap();
        o.running = true;
        o.error = None;
        o.frame = vec![0; super::screen::W * super::screen::H];
    }
    let fail = |e: mlua::Error| {
        let mut o = out.lock().unwrap();
        let msg = e.to_string();
        let short = msg.lines().next().unwrap_or("").to_string();
        o.say(format!("error: {msg}"));
        o.error = Some(short);
    };

    if let Err(e) = setup(&lua, &script, &code_dir, Arc::clone(&queue), Arc::clone(&out), Rc::clone(&screen), Rc::clone(&gh)) {
        fail(e);
        wait_for_quit(&rx);
        out.lock().unwrap().running = false;
        return;
    }
    // engine.name
    let engine: Option<String> = lua.globals().get::<Table>("engine").ok().and_then(|t| t.raw_get::<Option<String>>("name").ok().flatten());
    {
        let mut o = out.lock().unwrap();
        match &engine {
            Some(n) if ENGINES.contains(&n.as_str()) => o.engine = n.clone(),
            Some(n) => {
                o.engine = format!("{n}?");
                o.say(format!("engine '{n}' isn't on Portamax yet; its commands are ignored"));
            }
            None => o.engine = "none".into(),
        }
    }
    // norns draws the script's screen once it has started
    if let Err(e) = call_global(&lua, "init", ()).and_then(|_| call_global(&lua, "redraw", ())) {
        fail(e);
    }
    let start = Instant::now();
    let mut want_params = false;
    let mut last_params = Instant::now() - Duration::from_secs(1);
    let tick: Option<Function> = lua.globals().get("_px_tick").ok();
    loop {
        let mut events = Vec::new();
        match rx.recv_timeout(Duration::from_millis(1)) {
            Ok(e) => events.push(e),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        let mut quit = false;
        for e in events {
            let r = match e {
                Event::Enc(n, d) => call_global(&lua, "enc", (n, d)),
                Event::Key(n, z) => call_global(&lua, "key", (n, z)),
                Event::Midi(data) => midi_in(&lua, data),
                Event::ParamDelta(i, d) => lua.globals().get::<Function>("_px_params_delta").and_then(|f| f.call::<()>((i, d))),
                Event::WantParams(w) => {
                    want_params = w;
                    last_params = Instant::now() - Duration::from_secs(1);
                    Ok(())
                }
                Event::Quit => {
                    quit = true;
                    Ok(())
                }
            };
            if let Err(e) = r {
                fail(e);
            }
        }
        if quit {
            break;
        }
        if let Err(e) = grid_turn(&lua, &gh) {
            fail(e);
        }
        if let Err(e) = arc_turn(&lua, &gh) {
            fail(e);
        }
        if let Some(t) = &tick {
            if let Err(e) = t.call::<()>(start.elapsed().as_secs_f64()) {
                fail(e);
            }
        }
        if want_params && last_params.elapsed() > Duration::from_millis(80) {
            last_params = Instant::now();
            match params_rows(&lua) {
                Ok(rows) => out.lock().unwrap().params = rows,
                Err(e) => fail(e),
            }
        }
    }
    let _ = call_global(&lua, "cleanup", ());
    // The script is gone: its picture goes dark.
    {
        let g = gh.borrow();
        if g.connected {
            g.grid.show(GRID_CLIENT, &Leds::new(g.leds.rows, g.leds.cols));
        }
        if g.arc_connected {
            g.arc.show(GRID_CLIENT, &[[0; arc_kit::LEDS]; arc_kit::MAX_ENCODERS]);
        }
    }
    queue.push(Cmd::Reset);
    out.lock().unwrap().running = false;
}

/// Grid presses into the script's `g.key`, and a new grid size into its
/// `g.cols` / `g.rows`.
fn grid_turn(lua: &Lua, gh: &Rc<RefCell<GridHost>>) -> mlua::Result<()> {
    if !gh.borrow().connected {
        return Ok(());
    }
    if gh.borrow_mut().follow_size() {
        let (c, r) = gh.borrow().dims();
        if let Ok(f) = lua.globals().get::<Function>("_px_grid_resize") {
            f.call::<()>((c, r))?;
        }
    }
    let keys = {
        let g = gh.borrow();
        g.grid.keys(GRID_CLIENT).into_iter().map(|k| {
            let (x, y) = g.from_grid(k.x, k.y);
            (x + 1, y + 1, k.down as i32)
        }).collect::<Vec<_>>()
    };
    if keys.is_empty() {
        return Ok(());
    }
    let f: Function = lua.globals().get("_px_grid_key")?;
    for k in keys {
        f.call::<()>(k)?;
    }
    Ok(())
}

/// Arc turns into the script's `a.delta`, pushes into `a.key` (1-based).
fn arc_turn(lua: &Lua, gh: &Rc<RefCell<GridHost>>) -> mlua::Result<()> {
    let events = {
        let g = gh.borrow();
        if !g.arc_connected {
            return Ok(());
        }
        g.arc.events(GRID_CLIENT)
    };
    for e in events {
        match e {
            arc_kit::Event::Delta { n, d } => lua.globals().get::<Function>("_px_arc_delta")?.call::<()>((n + 1, d))?,
            arc_kit::Event::Key { n, down } => lua.globals().get::<Function>("_px_arc_key")?.call::<()>((n + 1, down as i32))?,
        }
    }
    Ok(())
}

fn wait_for_quit(rx: &Receiver<Event>) {
    while let Ok(e) = rx.recv() {
        if matches!(e, Event::Quit) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rotated_script_maps_every_key_and_led_both_ways() {
        let grid = Arc::new(Grid::new());
        grid.set_size(8, 16);
        let mut gh = GridHost::new(grid, Arc::new(ArcHub::new()));
        for rot in 0..4u8 {
            gh.rot = rot;
            let (c, r) = gh.dims();
            assert_eq!((c, r), if rot % 2 == 1 { (8, 16) } else { (16, 8) }, "rotation {rot}");
            let mut seen = std::collections::HashSet::new();
            for y in 0..r {
                for x in 0..c {
                    let (gx, gy) = gh.to_grid(x, y).expect("on the grid");
                    assert!(gx < 16 && gy < 8);
                    assert!(seen.insert((gx, gy)), "each key once");
                    assert_eq!(gh.from_grid(gx, gy), (x, y), "rotation {rot}: a press comes back where it was drawn");
                }
            }
            assert!(gh.to_grid(c, 0).is_none());
        }
        gh.rot = 1;
        assert_eq!(gh.to_grid(0, 0), Some((15, 0)), "a quarter turn: the script's top-left is the grid's top-right");
    }
}
