//! The grid: one shared monome-style grid that apps play on. It shows on
//! screen (the Grid app, `apps/grid.rs`) and on a real monome grid when one
//! is plugged in, both at once.
//!
//! A grid is keys that light. Each key sends a press and a release, and each
//! key's LED has 16 levels (monome's "varibright", 0 = off, 15 = full). The
//! grid has no behaviour of its own: the app holding it (its *focus*) reads
//! the presses and decides every LED, the way a monome grid is drawn entirely
//! by the script or module running it.
//!
//! Size: 8 rows x 16 columns by default (the size of a grid 128), and
//! anything from 1 x 1 up to 64 rows x 128 columns. An app draws a picture of
//! whatever size it has (Kria's is 16 x 8) and it sits at the top-left; on a
//! bigger grid the rest stays dark, on a smaller one the picture is cropped.
//!
//! Hardware, on the Mac: a real grid is reached the same way norns, Max and
//! everything else on a computer reach one, through monome's serialosc daemon
//! over OSC on UDP (port 12002 for discovery). We ask it for devices, point
//! the first grid at our port with prefix `/portamax`, then read
//! `/portamax/grid/key x y s` and send `/portamax/grid/led/level/map` in 8 x 8
//! quads. If serialosc isn't installed nothing answers and nothing else
//! changes; we ask again every two seconds, so a grid plugged in later is
//! found. The hardware shows an `offset` window of the grid, so a 16 x 8 grid
//! can look at any part of a 64 x 128 one.
//!
//! On the Portamax hardware there is no serialosc: a grid on the USB host port
//! speaks monome's serial protocol (mext) directly. That driver isn't written
//! yet; it would feed the same `Grid` the way `Client` does here.
//!
//! Threads: the UI thread (apps, in `tick` / `background_tick`) and the
//! serialosc thread share the grid through one mutex. The audio thread never
//! touches it: an app that wants grid presses in its sound reads them on the
//! UI thread and passes them on the way it passes pad presses.

use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub const MAX_ROWS: usize = 64;
pub const MAX_COLS: usize = 128;
pub const DEFAULT_ROWS: usize = 8;
pub const DEFAULT_COLS: usize = 16;
/// Presses waiting for the focused app before the oldest are dropped. A
/// frame is 16 ms; nobody presses 512 keys in one.
const MAX_KEYS: usize = 512;

/// A key going down or up, in grid coordinates (x = column, y = row, both
/// from the top-left, as monome counts them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub x: usize,
    pub y: usize,
    pub down: bool,
}

/// A real grid serialosc told us about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub kind: String,
    pub rows: usize,
    pub cols: usize,
}

/// An app's picture: LED levels 0-15, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leds {
    pub rows: usize,
    pub cols: usize,
    v: Vec<u8>,
}

impl Leds {
    pub fn new(rows: usize, cols: usize) -> Leds {
        Leds { rows, cols, v: vec![0; rows * cols] }
    }
    pub fn get(&self, x: usize, y: usize) -> u8 {
        if x < self.cols && y < self.rows {
            self.v[y * self.cols + x]
        } else {
            0
        }
    }
    /// Sets a level, clamped to 0-15; keys off the picture are ignored, so
    /// drawing code needn't check its own edges.
    pub fn set(&mut self, x: usize, y: usize, level: i32) {
        if x < self.cols && y < self.rows {
            self.v[y * self.cols + x] = level.clamp(0, 15) as u8;
        }
    }
    /// Brightens (or dims) a key, the way monome code does `buffer[i] += 4`.
    pub fn add(&mut self, x: usize, y: usize, d: i32) {
        let l = self.get(x, y) as i32;
        self.set(x, y, l + d);
    }
    pub fn fill_row(&mut self, y: usize, xs: std::ops::Range<usize>, level: i32) {
        for x in xs {
            self.set(x, y, level);
        }
    }
}

struct State {
    rows: usize,
    cols: usize,
    /// Size follows the plugged-in grid.
    follow: bool,
    leds: Vec<u8>,
    held: Vec<bool>,
    /// Bumped whenever the LEDs or the size change, so the screen and the
    /// hardware only redraw when there's something new.
    gen: u64,
    keys: VecDeque<Key>,
    clients: Vec<String>,
    focus: Option<String>,
    device: Option<Device>,
    /// Where the hardware's top-left key sits on the grid.
    offset: (usize, usize),
}

/// What the screen needs to draw the grid.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub rows: usize,
    pub cols: usize,
    pub leds: Vec<u8>,
    pub held: Vec<bool>,
    pub focus: Option<String>,
    pub device: Option<Device>,
    pub offset: (usize, usize),
}

pub struct Grid {
    s: Mutex<State>,
}

impl Default for Grid {
    fn default() -> Self {
        Grid::new()
    }
}

impl Grid {
    pub fn new() -> Grid {
        let (rows, cols) = (DEFAULT_ROWS, DEFAULT_COLS);
        Grid {
            s: Mutex::new(State {
                rows,
                cols,
                follow: true,
                leds: vec![0; rows * cols],
                held: vec![false; rows * cols],
                gen: 1,
                keys: VecDeque::new(),
                clients: Vec::new(),
                focus: None,
                device: None,
                offset: (0, 0),
            }),
        }
    }

    fn st(&self) -> MutexGuard<'_, State> {
        // A panic elsewhere while holding the lock leaves plain data behind,
        // still fine to use.
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// (rows, columns).
    pub fn size(&self) -> (usize, usize) {
        let s = self.st();
        (s.rows, s.cols)
    }

    /// Resizes the grid (clamped to 1x1 .. 64x128). Everything lit or held
    /// is cleared: the focused app redraws on its next frame.
    pub fn set_size(&self, rows: usize, cols: usize) {
        let mut s = self.st();
        Self::resize(&mut s, rows, cols);
    }

    fn resize(s: &mut State, rows: usize, cols: usize) {
        let (rows, cols) = (rows.clamp(1, MAX_ROWS), cols.clamp(1, MAX_COLS));
        if (rows, cols) == (s.rows, s.cols) {
            return;
        }
        s.rows = rows;
        s.cols = cols;
        s.leds = vec![0; rows * cols];
        s.held = vec![false; rows * cols];
        s.keys.clear();
        s.offset = (s.offset.0.min(cols - 1), s.offset.1.min(rows - 1));
        s.gen += 1;
    }

    pub fn follows_device(&self) -> bool {
        self.st().follow
    }

    /// Size follows the plugged-in grid (from now, and whenever one arrives).
    pub fn set_follow(&self, on: bool) {
        let mut s = self.st();
        s.follow = on;
        if let (true, Some(d)) = (on, s.device.clone()) {
            Self::resize(&mut s, d.rows, d.cols);
            s.offset = (0, 0);
        }
    }

    /// An app that can play the grid. The first one to ask gets it.
    pub fn register(&self, name: &str) {
        let mut s = self.st();
        if !s.clients.iter().any(|c| c == name) {
            s.clients.push(name.to_string());
        }
        if s.focus.is_none() {
            s.focus = Some(name.to_string());
        }
    }

    pub fn clients(&self) -> Vec<String> {
        self.st().clients.clone()
    }

    pub fn focus(&self) -> Option<String> {
        self.st().focus.clone()
    }

    /// Hands the grid to another app. The old app's picture goes dark and
    /// keys still held are forgotten, so the new app never sees a release
    /// for a press it didn't get.
    pub fn set_focus(&self, name: &str) {
        let mut s = self.st();
        if s.focus.as_deref() == Some(name) || !s.clients.iter().any(|c| c == name) {
            return;
        }
        s.focus = Some(name.to_string());
        s.keys.clear();
        s.held.iter_mut().for_each(|h| *h = false);
        s.leds.iter_mut().for_each(|l| *l = 0);
        s.gen += 1;
    }

    /// A key on the grid went down or up (from the screen, or the hardware
    /// after its offset). A repeat of the state it's already in is ignored.
    pub fn press(&self, x: usize, y: usize, down: bool) {
        let mut s = self.st();
        if x >= s.cols || y >= s.rows {
            return;
        }
        let i = y * s.cols + x;
        if s.held[i] == down {
            return;
        }
        s.held[i] = down;
        if s.keys.len() >= MAX_KEYS {
            s.keys.pop_front();
        }
        s.keys.push_back(Key { x, y, down });
        s.gen += 1;
    }

    /// A key on the hardware, in its own coordinates.
    pub fn device_press(&self, x: usize, y: usize, down: bool) {
        let (ox, oy) = self.st().offset;
        self.press(x + ox, y + oy, down);
    }

    /// The presses waiting for `name`, if it holds the grid; nothing for
    /// anyone else.
    pub fn keys(&self, name: &str) -> Vec<Key> {
        let mut s = self.st();
        if s.focus.as_deref() != Some(name) {
            return Vec::new();
        }
        s.keys.drain(..).collect()
    }

    /// `name`'s picture, if it holds the grid. Returns whether it was shown.
    pub fn show(&self, name: &str, leds: &Leds) -> bool {
        let mut s = self.st();
        if s.focus.as_deref() != Some(name) {
            return false;
        }
        let (rows, cols) = (s.rows, s.cols);
        let mut changed = false;
        for y in 0..rows {
            for x in 0..cols {
                let l = leds.get(x, y);
                let cell = &mut s.leds[y * cols + x];
                if *cell != l {
                    *cell = l;
                    changed = true;
                }
            }
        }
        if changed {
            s.gen += 1;
        }
        true
    }

    pub fn snapshot(&self) -> Snapshot {
        let s = self.st();
        Snapshot {
            rows: s.rows,
            cols: s.cols,
            leds: s.leds.clone(),
            held: s.held.clone(),
            focus: s.focus.clone(),
            device: s.device.clone(),
            offset: s.offset,
        }
    }

    pub fn device(&self) -> Option<Device> {
        self.st().device.clone()
    }

    /// A grid arrived (or changed size), or went away.
    pub fn set_device(&self, d: Option<Device>) {
        let mut s = self.st();
        if let (true, Some(d)) = (s.follow, d.as_ref()) {
            Self::resize(&mut s, d.rows, d.cols);
            s.offset = (0, 0);
        }
        s.device = d;
        s.gen += 1;
    }

    pub fn offset(&self) -> (usize, usize) {
        self.st().offset
    }

    /// Moves the hardware's window over the grid (kept on the grid).
    pub fn set_offset(&self, x: usize, y: usize) {
        let mut s = self.st();
        s.offset = (x.min(s.cols - 1), y.min(s.rows - 1));
        s.gen += 1;
    }

    /// The hardware's view: its own size, levels from under its window.
    fn hardware_frame(&self) -> Option<(usize, usize, Vec<u8>, u64)> {
        let s = self.st();
        let d = s.device.as_ref()?;
        let (ox, oy) = s.offset;
        let mut v = vec![0u8; d.rows * d.cols];
        for y in 0..d.rows {
            for x in 0..d.cols {
                let (gx, gy) = (x + ox, y + oy);
                if gx < s.cols && gy < s.rows {
                    v[y * d.cols + x] = s.leds[gy * s.cols + gx];
                }
            }
        }
        Some((d.rows, d.cols, v, s.gen))
    }
}

/// The one grid. Opening it the first time also starts looking for a real
/// grid through serialosc.
#[cfg(not(test))]
pub fn grid() -> Arc<Grid> {
    static GRID: std::sync::OnceLock<Arc<Grid>> = std::sync::OnceLock::new();
    Arc::clone(GRID.get_or_init(|| {
        let g = Arc::new(Grid::new());
        serialosc::start(Arc::clone(&g));
        g
    }))
}

/// Under test each test thread gets its own grid (tests run in parallel)
/// and nothing goes looking for hardware.
#[cfg(test)]
pub fn grid() -> Arc<Grid> {
    thread_local! {
        static GRID: Arc<Grid> = Arc::new(Grid::new());
    }
    GRID.with(Arc::clone)
}

/// Just enough OSC 1.0 for serialosc: messages with int, float and string
/// arguments. (serialosc sends no bundles; other types are skipped over.)
pub mod osc {
    #[derive(Clone, Debug, PartialEq)]
    pub enum Arg {
        I(i32),
        F(f32),
        S(String),
    }

    fn put_str(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(s.as_bytes());
        // At least one NUL, then pad to a multiple of four bytes.
        out.push(0);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }

    pub fn encode(addr: &str, args: &[Arg]) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + args.len() * 5);
        put_str(&mut out, addr);
        let tags: String = std::iter::once(',')
            .chain(args.iter().map(|a| match a {
                Arg::I(_) => 'i',
                Arg::F(_) => 'f',
                Arg::S(_) => 's',
            }))
            .collect();
        put_str(&mut out, &tags);
        for a in args {
            match a {
                Arg::I(v) => out.extend_from_slice(&v.to_be_bytes()),
                Arg::F(v) => out.extend_from_slice(&v.to_bits().to_be_bytes()),
                Arg::S(s) => put_str(&mut out, s),
            }
        }
        out
    }

    fn get_str(b: &[u8], at: &mut usize) -> Option<String> {
        let rest = b.get(*at..)?;
        let end = rest.iter().position(|&c| c == 0)?;
        let s = std::str::from_utf8(&rest[..end]).ok()?.to_string();
        *at += (end + 4) & !3;
        Some(s)
    }

    fn get4(b: &[u8], at: &mut usize) -> Option<[u8; 4]> {
        let v: [u8; 4] = b.get(*at..*at + 4)?.try_into().ok()?;
        *at += 4;
        Some(v)
    }

    pub fn decode(b: &[u8]) -> Option<(String, Vec<Arg>)> {
        let mut at = 0;
        let addr = get_str(b, &mut at)?;
        if !addr.starts_with('/') {
            return None;
        }
        if at >= b.len() {
            return Some((addr, Vec::new()));
        }
        let tags = get_str(b, &mut at)?;
        let mut args = Vec::new();
        for t in tags.strip_prefix(',')?.chars() {
            match t {
                'i' => args.push(Arg::I(i32::from_be_bytes(get4(b, &mut at)?))),
                'f' => args.push(Arg::F(f32::from_bits(u32::from_be_bytes(get4(b, &mut at)?)))),
                's' | 'S' => args.push(Arg::S(get_str(b, &mut at)?)),
                'h' | 't' | 'd' => at += 8,
                'b' => {
                    let n = i32::from_be_bytes(get4(b, &mut at)?).max(0) as usize;
                    at += (n + 3) & !3;
                }
                'c' | 'r' | 'm' => at += 4,
                // True, false, nil, infinitum: no data.
                'T' | 'F' | 'N' | 'I' => {}
                _ => return None,
            }
        }
        Some((addr, args))
    }
}

/// The serialosc client: finds a grid, reads its keys, lights its LEDs.
pub mod serialosc {
    use super::osc::{self, Arg};
    use super::*;

    /// serialosc's discovery port.
    #[cfg_attr(test, allow(dead_code))] // tests talk to a stand-in instead
    pub const PORT: u16 = 12002;
    /// Our OSC address prefix on the grid.
    pub const PREFIX: &str = "/portamax";
    const HOST: &str = "127.0.0.1";
    const ASK_EVERY: Duration = Duration::from_secs(2);
    /// LEDs go out at most this often (a grid refreshes at about 60 Hz).
    const LED_EVERY: Duration = Duration::from_millis(10);

    #[cfg_attr(test, allow(dead_code))] // tests never go looking for hardware
    pub fn start(grid: Arc<Grid>) {
        let spawned = std::thread::Builder::new().name("serialosc".into()).spawn(move || {
            let Ok(mut c) = Client::new(grid, SocketAddr::from(([127, 0, 0, 1], PORT))) else { return };
            loop {
                c.poll();
            }
        });
        // No thread, no hardware grid; the screen grid still works.
        drop(spawned);
    }

    /// A monome's size from the name serialosc gives it, until the grid
    /// answers `/sys/info` with its real size.
    fn size_for(kind: &str) -> (usize, usize) {
        let k = kind.to_ascii_lowercase();
        if k.contains("64") && !k.contains("256") {
            (8, 8)
        } else if k.contains("256") || k.contains("zero") {
            (16, 16)
        } else if k.contains("512") {
            (16, 32)
        } else {
            (8, 16)
        }
    }

    pub struct Client {
        grid: Arc<Grid>,
        sock: UdpSocket,
        serialosc: SocketAddr,
        dev: Option<SocketAddr>,
        dev_id: Option<String>,
        asked: Option<Instant>,
        sent_gen: u64,
        sent_at: Instant,
    }

    impl Client {
        pub fn new(grid: Arc<Grid>, serialosc: SocketAddr) -> std::io::Result<Client> {
            let sock = UdpSocket::bind((HOST, 0))?;
            sock.set_read_timeout(Some(Duration::from_millis(5)))?;
            Ok(Client { grid, sock, serialosc, dev: None, dev_id: None, asked: None, sent_gen: u64::MAX, sent_at: Instant::now() })
        }

        pub fn port(&self) -> u16 {
            self.sock.local_addr().map(|a| a.port()).unwrap_or(0)
        }

        fn send(&self, to: SocketAddr, addr: &str, args: &[Arg]) {
            // UDP to a port nobody listens on just vanishes; nothing to do.
            let _ = self.sock.send_to(&osc::encode(addr, args), to);
        }

        fn me(&self) -> [Arg; 2] {
            [Arg::S(HOST.into()), Arg::I(self.port() as i32)]
        }

        /// serialosc only tells us about one add/remove per `notify`, so it
        /// is asked again after each.
        fn notify(&self) {
            self.send(self.serialosc, "/serialosc/notify", &self.me());
        }

        /// One turn: ask for a grid if there isn't one, take in one message
        /// (waiting up to 5 ms), and send the LEDs if they changed.
        pub fn poll(&mut self) {
            if self.dev.is_none() && self.asked.map_or(true, |t| t.elapsed() >= ASK_EVERY) {
                self.send(self.serialosc, "/serialosc/list", &self.me());
                self.notify();
                self.asked = Some(Instant::now());
            }
            let mut buf = [0u8; 2048];
            if let Ok((n, _)) = self.sock.recv_from(&mut buf) {
                if let Some((addr, args)) = osc::decode(&buf[..n]) {
                    self.handle(&addr, &args);
                }
            }
            self.leds();
        }

        fn handle(&mut self, addr: &str, args: &[Arg]) {
            match (addr, args) {
                ("/serialosc/device", [Arg::S(id), Arg::S(kind), Arg::I(port), ..]) => {
                    // The first grid wins; an arc isn't a grid.
                    if self.dev.is_none() && !kind.to_ascii_lowercase().contains("arc") {
                        self.attach(id, kind, *port);
                    }
                }
                ("/serialosc/add", _) => {
                    self.asked = None;
                    self.notify();
                }
                ("/serialosc/remove", [Arg::S(id), ..]) => {
                    if self.dev_id.as_deref() == Some(id.as_str()) {
                        self.dev = None;
                        self.dev_id = None;
                        self.grid.set_device(None);
                    }
                    self.asked = None;
                    self.notify();
                }
                ("/sys/size", [Arg::I(x), Arg::I(y), ..]) => {
                    if let Some(d) = self.grid.device() {
                        let (cols, rows) = ((*x).clamp(1, MAX_COLS as i32) as usize, (*y).clamp(1, MAX_ROWS as i32) as usize);
                        self.grid.set_device(Some(Device { rows, cols, ..d }));
                        self.sent_gen = u64::MAX;
                    }
                }
                (a, [Arg::I(x), Arg::I(y), Arg::I(s), ..]) if a.strip_prefix(PREFIX) == Some("/grid/key") => {
                    if *x >= 0 && *y >= 0 {
                        self.grid.device_press(*x as usize, *y as usize, *s != 0);
                    }
                }
                _ => {}
            }
        }

        fn attach(&mut self, id: &str, kind: &str, port: i32) {
            let Ok(port) = u16::try_from(port) else { return };
            let dev = SocketAddr::from(([127, 0, 0, 1], port));
            self.send(dev, "/sys/port", &[Arg::I(self.port() as i32)]);
            self.send(dev, "/sys/host", &[Arg::S(HOST.into())]);
            self.send(dev, "/sys/prefix", &[Arg::S(PREFIX.into())]);
            self.send(dev, "/sys/info", &self.me());
            let (rows, cols) = size_for(kind);
            self.dev = Some(dev);
            self.dev_id = Some(id.to_string());
            self.grid.set_device(Some(Device { id: id.into(), kind: kind.into(), rows, cols }));
            self.sent_gen = u64::MAX;
        }

        fn leds(&mut self) {
            let Some(dev) = self.dev else { return };
            if self.sent_at.elapsed() < LED_EVERY {
                return;
            }
            let Some((rows, cols, v, gen)) = self.grid.hardware_frame() else { return };
            if gen == self.sent_gen {
                return;
            }
            let addr = format!("{PREFIX}/grid/led/level/map");
            for qy in (0..rows).step_by(8) {
                for qx in (0..cols).step_by(8) {
                    let mut args = Vec::with_capacity(66);
                    args.push(Arg::I(qx as i32));
                    args.push(Arg::I(qy as i32));
                    for y in qy..qy + 8 {
                        for x in qx..qx + 8 {
                            let l = if x < cols && y < rows { v[y * cols + x] } else { 0 };
                            args.push(Arg::I(l as i32));
                        }
                    }
                    self.send(dev, &addr, &args);
                }
            }
            self.sent_gen = gen;
            self.sent_at = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::osc::{decode, encode, Arg};
    use super::*;

    #[test]
    fn the_grid_opens_at_8_by_16_and_sizes_up_to_64_by_128() {
        let g = Grid::new();
        assert_eq!(g.size(), (8, 16));
        g.set_size(64, 128);
        assert_eq!(g.size(), (64, 128));
        g.set_size(500, 500);
        assert_eq!(g.size(), (MAX_ROWS, MAX_COLS));
        g.set_size(0, 0);
        assert_eq!(g.size(), (1, 1));
    }

    #[test]
    fn keys_and_leds_belong_to_the_app_holding_the_grid() {
        let g = Grid::new();
        g.register("Kria");
        g.register("Teletype");
        assert_eq!(g.focus().as_deref(), Some("Kria"));
        g.press(3, 2, true);
        g.press(3, 2, true); // a repeat isn't a second press
        g.press(3, 2, false);
        g.press(99, 0, true); // off the grid
        assert!(g.keys("Teletype").is_empty());
        assert_eq!(g.keys("Kria"), vec![Key { x: 3, y: 2, down: true }, Key { x: 3, y: 2, down: false }]);

        let mut l = Leds::new(8, 16);
        l.set(15, 7, 12);
        l.add(15, 7, 9); // clamps at 15
        assert!(!g.show("Teletype", &l));
        assert!(g.show("Kria", &l));
        assert_eq!(g.snapshot().leds[7 * 16 + 15], 15);

        // Handing it over darkens the old picture and forgets held keys.
        g.press(0, 0, true);
        g.set_focus("Teletype");
        let s = g.snapshot();
        assert!(s.leds.iter().all(|&v| v == 0) && s.held.iter().all(|&h| !h));
        assert!(g.keys("Teletype").is_empty());
    }

    #[test]
    fn a_smaller_picture_sits_top_left_and_a_bigger_one_is_cropped() {
        let g = Grid::new();
        g.register("a");
        g.set_size(16, 16);
        let mut l = Leds::new(8, 32);
        l.set(0, 0, 4);
        l.set(31, 7, 8);
        g.show("a", &l);
        let s = g.snapshot();
        assert_eq!(s.leds[0], 4);
        assert_eq!(s.leds.iter().filter(|&&v| v != 0).count(), 1);
    }

    #[test]
    fn the_hardware_sees_a_window_of_the_grid_and_presses_land_under_it() {
        let g = Grid::new();
        g.register("a");
        g.set_follow(false);
        g.set_size(64, 128);
        g.set_device(Some(Device { id: "m1".into(), kind: "monome 128".into(), rows: 8, cols: 16 }));
        assert_eq!(g.size(), (64, 128), "not following, so the size stays");
        g.set_offset(16, 8);
        let mut l = Leds::new(64, 128);
        l.set(16, 8, 7);
        g.show("a", &l);
        let (rows, cols, v, _) = g.hardware_frame().unwrap();
        assert_eq!((rows, cols, v[0]), (8, 16, 7));
        g.device_press(1, 1, true);
        assert_eq!(g.keys("a"), vec![Key { x: 17, y: 9, down: true }]);
        // Following snaps the grid to the hardware.
        g.set_follow(true);
        assert_eq!((g.size(), g.offset()), ((8, 16), (0, 0)));
    }

    #[test]
    fn osc_messages_round_trip() {
        let m = encode("/portamax/grid/key", &[Arg::I(3), Arg::I(-1), Arg::S("abcd".into()), Arg::F(0.5)]);
        assert_eq!(m.len() % 4, 0);
        assert_eq!(&m[..20], b"/portamax/grid/key\0\0");
        assert_eq!(decode(&m), Some(("/portamax/grid/key".into(), vec![Arg::I(3), Arg::I(-1), Arg::S("abcd".into()), Arg::F(0.5)])));
        assert_eq!(decode(b"not osc\0"), None);
    }

    /// The whole serialosc conversation over real UDP on loopback, with
    /// two sockets playing serialosc and the grid it hands us.
    #[test]
    fn a_grid_found_through_serialosc_plays_and_lights() {
        let sosc = UdpSocket::bind("127.0.0.1:0").unwrap();
        let dev = UdpSocket::bind("127.0.0.1:0").unwrap();
        for s in [&sosc, &dev] {
            s.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        }
        let g = Arc::new(Grid::new());
        g.register("a");
        let mut c = serialosc::Client::new(Arc::clone(&g), sosc.local_addr().unwrap()).unwrap();
        let recv = |s: &UdpSocket| {
            let mut b = [0u8; 2048];
            let (n, from) = s.recv_from(&mut b).expect("a message");
            (decode(&b[..n]).unwrap(), from)
        };

        c.poll();
        let ((addr, args), from) = recv(&sosc);
        assert_eq!(addr, "/serialosc/list");
        assert_eq!(args, vec![Arg::S("127.0.0.1".into()), Arg::I(c.port() as i32)]);
        let ((addr, _), _) = recv(&sosc);
        assert_eq!(addr, "/serialosc/notify");

        let port = dev.local_addr().unwrap().port() as i32;
        sosc.send_to(&encode("/serialosc/device", &[Arg::S("m0000123".into()), Arg::S("monome 128".into()), Arg::I(port)]), from).unwrap();
        c.poll();
        let mut got = Vec::new();
        for _ in 0..4 {
            got.push(recv(&dev).0);
        }
        assert_eq!(got[0], ("/sys/port".into(), vec![Arg::I(c.port() as i32)]));
        assert_eq!(got[2], ("/sys/prefix".into(), vec![Arg::S("/portamax".into())]));
        assert_eq!(got[3].0, "/sys/info");
        assert_eq!(g.device().map(|d| (d.rows, d.cols)), Some((8, 16)));

        // It is really a 256: the size follows.
        dev.send_to(&encode("/sys/size", &[Arg::I(16), Arg::I(16)]), from).unwrap();
        dev.send_to(&encode("/portamax/grid/key", &[Arg::I(5), Arg::I(9), Arg::I(1)]), from).unwrap();
        for _ in 0..20 {
            c.poll();
        }
        assert_eq!(g.size(), (16, 16));
        assert_eq!(g.keys("a"), vec![Key { x: 5, y: 9, down: true }]);

        // LEDs go out as four 8x8 quads.
        let mut l = Leds::new(16, 16);
        l.set(9, 10, 11);
        g.show("a", &l);
        std::thread::sleep(Duration::from_millis(15));
        // Drain the frame sent before the picture changed, if any.
        let mut quads = std::collections::HashMap::new();
        for _ in 0..40 {
            c.poll();
            dev.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
            let mut b = [0u8; 2048];
            while let Ok((n, _)) = dev.recv_from(&mut b) {
                if let Some((addr, args)) = decode(&b[..n]) {
                    if addr == "/portamax/grid/led/level/map" {
                        if let [Arg::I(x), Arg::I(y), rest @ ..] = args.as_slice() {
                            quads.insert((*x, *y), rest.to_vec());
                        }
                    }
                }
            }
            if quads.get(&(8, 8)).map_or(false, |q| q[2 * 8 + 1] == Arg::I(11)) {
                break;
            }
        }
        assert_eq!(quads.len(), 4);
        assert_eq!(quads[&(8, 8)].len(), 64);
        assert_eq!(quads[&(8, 8)][2 * 8 + 1], Arg::I(11), "key (9,10) is quad (8,8) row 2 column 1");

        // Unplugged: the grid forgets it.
        sosc.send_to(&encode("/serialosc/remove", &[Arg::S("m0000123".into()), Arg::S("monome 128".into()), Arg::I(port)]), from).unwrap();
        for _ in 0..10 {
            c.poll();
        }
        assert_eq!(g.device(), None);
    }
}
