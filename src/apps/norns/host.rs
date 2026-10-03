//! The script host: one thread per running script, owning its Lua state
//! the way norns' own event loop does. It turns encoder/key/MIDI events
//! into calls to the script's `enc` / `key` / MIDI handlers, runs clocks
//! and metros (`_px_tick`, ~1 kHz, so `clock.sync` lands within about a
//! millisecond), and publishes the screen whenever the script calls
//! `screen.update()`. Sound leaves through the command queue to the
//! audio processor (engine.rs).

use super::engine::{Cmd, Queue};
use super::screen::Screen;
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
pub fn setup(lua: &Lua, script: &Path, code_dir: &Path, queue: Arc<Queue>, out: Arc<Mutex<HostOut>>, screen: Rc<RefCell<Screen>>) -> mlua::Result<()> {
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
pub fn run(script: PathBuf, code_dir: PathBuf, queue: Arc<Queue>, out: Arc<Mutex<HostOut>>, rx: Receiver<Event>) {
    let lua = Lua::new();
    let screen = Rc::new(RefCell::new(Screen::new()));
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

    if let Err(e) = setup(&lua, &script, &code_dir, Arc::clone(&queue), Arc::clone(&out), Rc::clone(&screen)) {
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
    queue.push(Cmd::Reset);
    out.lock().unwrap().running = false;
}

fn wait_for_quit(rx: &Receiver<Event>) {
    while let Ok(e) = rx.recv() {
        if matches!(e, Event::Quit) {
            return;
        }
    }
}
