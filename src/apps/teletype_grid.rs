//! Teletype's grid integration: the G.* ops, which turn a monome grid into
//! a panel of buttons, faders and XY pads that fire Teletype scripts, plus a
//! free LED layer scripts draw on. Part of the Teletype app (teletype.rs); it
//! plays on the shared grid (grid_kit.rs), so the Grid app's screen grid
//! works as well as a real one.
//!
//! The behaviour follows the module's: 256 buttons, 64 faders, 64 groups,
//! 8 XY pads on a 16 x 16 coordinate space; latching buttons toggle on a
//! press and momentary ones follow the key; coarse faders take the key's
//! position, fine ones step from their end keys (held, they repeat after
//! 700 ms every 40 ms) and spread their middle keys over 0..level; pressing
//! a second key on a fader while one is held slides to it; a control's
//! script and its group's script run after the key; pressed buttons and
//! fader fills light at 13; the LED layer (G.LED, G.REC...) is drawn over
//! the controls, where -1 dims and -2 brightens by 3 and -3 leaves alone;
//! G.DIM darkens everything; G.ROTATE 1 turns the grid 180 degrees. Written
//! from the Teletype grid studies and op reference, with the details the
//! docs leave out (rounding, clamping, the order things draw in) matched to
//! the firmware's behaviour. No firmware code is copied (it is GPL-2.0).
//!
//! Not here: grid control mode (the module's on-grid script editor) and
//! the grid visualiser on the module's screen.
//!
//! Lives on the audio thread inside the interpreter: fixed arrays, no
//! allocation. Keys come in from the UI thread through Teletype's edit
//! queue and the picture goes back as a 16 x 16 level map.

/// The module's grid coordinate space.
pub const DIM: usize = 16;
const D: i16 = DIM as i16;
pub const BUTTONS: usize = 256;
pub const FADERS: usize = 64;
pub const GROUPS: usize = 64;
pub const XYPADS: usize = 8;
/// LED layer levels below 0.
pub const LED_DIM: i16 = -1;
pub const LED_BRI: i16 = -2;
pub const LED_OFF: i16 = -3;
/// A pressed button, a fader's fill, an XY pad's point.
const ON: i16 = 13;
const MAX_HELD: usize = 10;
const HOLD_DELAY: u16 = 700;
const REPEAT: u16 = 40;
/// Fader slides move on a 25 ms tick.
const SLEW_MS: u8 = 25;
/// Scripts 1-8, M, I (as 0-9).
const SCRIPTS: i16 = 10;

/// Fader types: coarse horizontal/vertical bar, coarse dot, fine bar, fine
/// dot. Odd types are vertical.
const CH_BAR: i16 = 0;
const CV_BAR: i16 = 1;
const CH_DOT: i16 = 2;
const CV_DOT: i16 = 3;
const FH_BAR: i16 = 4;
const FV_BAR: i16 = 5;
const FH_DOT: i16 = 6;
const FV_DOT: i16 = 7;
const COARSE: i16 = CV_DOT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GOp {
    Rst,
    Clr,
    Dim,
    Rotate,
    Key,
    Grp,
    GrpEn,
    GrpRst,
    GrpSw,
    GrpSc,
    Grpi,
    Led,
    LedC,
    Rec,
    Rct,
    Btn,
    Gbt,
    Btx,
    Gbx,
    BtnEn,
    BtnX,
    BtnY,
    BtnV,
    BtnL,
    Btni,
    Btnx,
    Btny,
    Btnv,
    Btnl,
    BtnSw,
    BtnPr,
    GbtnV,
    GbtnL,
    GbtnC,
    GbtnI,
    GbtnW,
    GbtnH,
    GbtnX1,
    GbtnX2,
    GbtnY1,
    GbtnY2,
    Fdr,
    Gfd,
    Fdx,
    Gfx,
    FdrEn,
    FdrX,
    FdrY,
    FdrN,
    FdrV,
    FdrL,
    Fdri,
    Fdrx,
    Fdry,
    Fdrn,
    Fdrv,
    Fdrl,
    FdrPr,
    GfdrN,
    GfdrV,
    GfdrL,
    GfdrRn,
    Xyp,
    XypX,
    XypY,
}

impl GOp {
    /// (arguments, returns a value, can be set).
    pub const fn shape(self) -> (u8, bool, bool) {
        use GOp::*;
        match self {
            Rst | Clr => (0, false, false),
            Dim | Rotate | GrpRst | GrpSw | BtnSw => (1, false, false),
            Key | GbtnL | GfdrL | GfdrRn => (3, false, false),
            LedC | BtnPr | GbtnV | FdrPr | GfdrN | GfdrV => (2, false, false),
            Rec | Rct => (6, false, false),
            Xyp => (7, false, false),
            Btn | Fdr => (8, false, false),
            Gbt | Gfd => (9, false, false),
            Btx | Fdx => (10, false, false),
            Gbx | Gfx => (11, false, false),
            Grp | Btnx | Btny | Btnv | Btnl | Fdrx | Fdry | Fdrn | Fdrv | Fdrl => (0, true, true),
            Grpi | Btni | Fdri => (0, true, false),
            GrpEn | GrpSc | BtnEn | BtnX | BtnY | BtnV | BtnL | FdrEn | FdrX | FdrY | FdrN | FdrV | FdrL => (1, true, true),
            GbtnC | GbtnW | GbtnH | GbtnX1 | GbtnX2 | GbtnY1 | GbtnY2 | XypX | XypY => (1, true, false),
            Led => (2, true, true),
            GbtnI => (2, true, false),
        }
    }
}

/// What an op leaves behind: a value to push, scripts to run now (a
/// button's then its group's, for G.BTN.PR / G.FDR.PR) and scripts a key
/// fired (as bits 0-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Out {
    pub ret: Option<i16>,
    pub run: [i8; 2],
    pub mask: u16,
}

impl Out {
    const NONE: Out = Out { ret: None, run: [-1, -1], mask: 0 };
    fn ret(v: i16) -> Out {
        Out { ret: Some(v), ..Out::NONE }
    }
}

#[derive(Clone, Copy, Debug)]
struct Common {
    enabled: bool,
    group: i16,
    x: i16,
    y: i16,
    w: i16,
    h: i16,
    level: i16,
    script: i16,
}

impl Common {
    const INIT: Common = Common { enabled: false, group: 0, x: 0, y: 0, w: 1, h: 1, level: 5, script: -1 };
    fn within(&self, x: i16, y: i16) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug)]
struct Button {
    c: Common,
    latch: bool,
    state: bool,
}

#[derive(Clone, Copy, Debug)]
struct Fader {
    c: Common,
    kind: i16,
    value: i16,
    slide: bool,
    acc: i16,
    end: i16,
    delta: i16,
    up: bool,
}

#[derive(Clone, Copy, Debug)]
struct XyPad {
    c: Common,
    vx: i16,
    vy: i16,
}

#[derive(Clone, Copy, Debug)]
struct Group {
    enabled: bool,
    script: i16,
    min: i16,
    max: i16,
}

#[derive(Clone, Copy, Debug, Default)]
struct Held {
    /// 0 free, 1 waiting for the hold delay, 2 repeating.
    used: u8,
    x: i16,
    y: i16,
    left: u16,
}

const BUTTON: Button = Button { c: Common::INIT, latch: false, state: false };
const FADER: Fader = Fader { c: Common::INIT, kind: CH_BAR, value: 0, slide: false, acc: 0, end: 0, delta: 1, up: false };
const XYPAD: XyPad = XyPad { c: Common::INIT, vx: 0, vy: 0 };
const GROUP: Group = Group { enabled: true, script: -1, min: 0, max: 16383 };

pub struct TtGrid {
    btn: [Button; BUTTONS],
    fdr: [Fader; FADERS],
    xyp: [XyPad; XYPADS],
    grp: [Group; GROUPS],
    /// The LED layer, [x][y].
    leds: [[i16; DIM]; DIM],
    rotate: bool,
    dim: i16,
    current: i16,
    latest_group: i16,
    latest_button: i16,
    latest_fader: i16,
    held: [Held; MAX_HELD],
    slew: u8,
    /// The grid's size as the module sees it (at most 16 x 16).
    cols: i16,
    rows: i16,
    /// The picture changed since it was last drawn.
    pub dirty: bool,
}

impl Default for TtGrid {
    fn default() -> Self {
        TtGrid::new()
    }
}

/// `a..b` scaled onto `x..y`, rounded the module's way.
fn scale(a: i32, b: i32, x: i32, y: i32, v: i32) -> i32 {
    if a == b {
        return x;
    }
    let r = (v - a) * (y - x) * 2 / (b - a);
    (r / 2) + (r & 1) + x
}

fn level(l: i16) -> i16 {
    l.clamp(LED_OFF, 15)
}

/// Keeps a control on the 16 x 16 space: a negative corner shrinks it, a
/// corner off the far edge refuses it.
fn clamp_area(mut x: i16, mut y: i16, mut w: i16, mut h: i16) -> Option<(i16, i16, i16, i16)> {
    if x < 0 {
        w = w.saturating_add(x);
        x = 0;
    } else if x >= D {
        return None;
    }
    if w.saturating_add(x) > D {
        w = D - x;
    }
    if y < 0 {
        h = h.saturating_add(y);
        y = 0;
    } else if y >= D {
        return None;
    }
    if h.saturating_add(y) > D {
        h = D - y;
    }
    Some((x, y, w, h))
}

fn script(s: i16) -> i16 {
    if (0..SCRIPTS).contains(&s) {
        s
    } else {
        -1
    }
}

fn fader_clamp_level(l: i16, kind: i16, w: i16, h: i16) -> i16 {
    if kind > COARSE {
        let size = if kind == FH_BAR || kind == FH_DOT { w } else { h } as i32;
        let max = ((size - 2) << 4) - 1;
        if l < 0 || size < 3 {
            return 0;
        }
        (l as i32).min(max) as i16
    } else {
        level(l)
    }
}

impl TtGrid {
    pub fn new() -> TtGrid {
        TtGrid {
            btn: [BUTTON; BUTTONS],
            fdr: [FADER; FADERS],
            xyp: [XYPAD; XYPADS],
            grp: [GROUP; GROUPS],
            leds: [[LED_OFF; DIM]; DIM],
            rotate: false,
            dim: 0,
            current: 0,
            latest_group: 0,
            latest_button: 0,
            latest_fader: 0,
            held: [Held::default(); MAX_HELD],
            slew: 0,
            cols: D,
            rows: 8,
            dirty: true,
        }
    }

    /// Everything back to a fresh scene's grid (G.RST, and loading a scene).
    /// The grid's size stays.
    pub fn reset(&mut self) {
        let (cols, rows) = (self.cols, self.rows);
        *self = TtGrid::new();
        self.cols = cols;
        self.rows = rows;
    }

    /// The real (or screen) grid's size; the module only ever sees 16 x 16.
    pub fn set_size(&mut self, cols: usize, rows: usize) {
        let (c, r) = (cols.clamp(1, DIM) as i16, rows.clamp(1, DIM) as i16);
        if (c, r) != (self.cols, self.rows) {
            self.cols = c;
            self.rows = r;
            self.held = [Held::default(); MAX_HELD];
            self.dirty = true;
        }
    }

    fn group_ok(g: i16) -> bool {
        (0..GROUPS as i16).contains(&g)
    }

    fn fader_max(&self, i: usize) -> i16 {
        let f = &self.fdr[i];
        match f.kind {
            CH_BAR | CH_DOT => f.c.w - 1,
            CV_BAR | CV_DOT => f.c.h - 1,
            _ => f.c.level,
        }
    }

    /// A fader's value in its group's range.
    fn fader_scaled(&self, i: usize) -> i16 {
        let f = &self.fdr[i];
        let g = &self.grp[f.c.group as usize];
        scale(0, self.fader_max(i) as i32, g.min as i32, g.max as i32, f.value as i32) as i16
    }

    fn set_fader_scaled(&mut self, i: usize, v: i16) {
        let g = self.grp[self.fdr[i].c.group as usize];
        let v = v.max(g.min).min(g.max);
        let max = self.fader_max(i);
        self.fdr[i].value = scale(g.min as i32, g.max as i32, 0, max as i32, v as i32) as i16;
    }

    fn set_fader_n(&mut self, i: usize, v: i16) {
        let max = self.fader_max(i);
        self.fdr[i].value = v.min(max).max(0);
    }

    fn set_fader_level(&mut self, i: usize, l: i16) {
        let f = &mut self.fdr[i];
        let l = fader_clamp_level(l, f.kind, f.c.w, f.c.h);
        if f.kind > COARSE {
            f.value = scale(0, f.c.level as i32, 0, l as i32, f.value as i32) as i16;
        }
        f.c.level = l;
    }

    /// Moves a control's corner (G.BTN.X, G.FDR.Y...), keeping its far corner.
    fn move_common(c: &mut Common, x: Option<i16>, y: Option<i16>) {
        if let Some((x, y, w, h)) = clamp_area(x.unwrap_or(c.x), y.unwrap_or(c.y), c.w, c.h) {
            c.x = x;
            c.y = y;
            c.w = w;
            c.h = h;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn init_button(&mut self, group: i16, i: i16, x: i16, y: i16, w: i16, h: i16, latch: i16, lvl: i16, s: i16) {
        if !Self::group_ok(group) || !(0..BUTTONS as i16).contains(&i) {
            return;
        }
        let Some((x, y, w, h)) = clamp_area(x, y, w, h) else { return };
        if w <= 0 || h <= 0 {
            return;
        }
        let b = &mut self.btn[i as usize];
        b.c = Common { enabled: true, group, x, y, w, h, level: level(lvl), script: script(s) };
        b.latch = latch != 0;
        if !b.latch {
            b.state = false;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn init_fader(&mut self, group: i16, i: i16, x: i16, y: i16, w: i16, h: i16, kind: i16, lvl: i16, s: i16) {
        if !Self::group_ok(group) || !(0..FADERS as i16).contains(&i) {
            return;
        }
        let Some((x, y, w, h)) = clamp_area(x, y, w, h) else { return };
        if w <= 0 || h <= 0 {
            return;
        }
        let kind = if (CH_BAR..=FV_DOT).contains(&kind) { kind } else { CH_BAR };
        let f = &mut self.fdr[i as usize];
        f.c = Common { enabled: true, group, x, y, w, h, level: fader_clamp_level(lvl, kind, w, h), script: script(s) };
        f.kind = kind;
    }

    fn rectangle(&mut self, x: i16, y: i16, w: i16, h: i16, fill: i16, border: i16) {
        let (x, y, w, h) = (x as i32, y as i32, w as i32, h as i32);
        let d = DIM as i32;
        for col in (x + 1).max(0)..(x + w - 1).min(d) {
            for row in (y + 1).max(0)..(y + h - 1).min(d) {
                self.leds[col as usize][row as usize] = fill;
            }
        }
        for row in [y, y + h - 1] {
            if (0..d).contains(&row) {
                for col in x.max(0)..(x + w).min(d) {
                    self.leds[col as usize][row as usize] = border;
                }
            }
        }
        for col in [x, x + w - 1] {
            if (0..d).contains(&col) {
                for row in y.max(0)..(y + h).min(d) {
                    self.leds[col as usize][row as usize] = border;
                }
            }
        }
        self.dirty = true;
    }

    /// The buttons of a group that are on.
    fn on_in(&self, g: i16) -> impl Iterator<Item = (usize, &Button)> {
        self.btn.iter().enumerate().filter(move |(_, b)| b.c.group == g && b.state)
    }

    /// A control's script, then its group's.
    fn run_pair(&self, c: &Common) -> [i8; 2] {
        [c.script as i8, self.grp[c.group as usize].script as i8]
    }

    /// Runs one G op. `a` holds its arguments in the order they're written,
    /// `v` the value being set.
    pub fn op(&mut self, op: GOp, set: bool, a: &[i16], v: i16) -> Out {
        use GOp::*;
        let arg = |k: usize| a.get(k).copied().unwrap_or(0);
        let bi = |i: i16| (0..BUTTONS as i16).contains(&i).then_some(i as usize);
        let fi = |i: i16| (0..FADERS as i16).contains(&i).then_some(i as usize);
        let xi = |i: i16| (0..XYPADS as i16).contains(&i).then_some(i as usize);
        // The module tests group ops with `> 64` (so 64 slips through to
        // nothing); here a group past the end is simply out.
        let gi = |g: i16| Self::group_ok(g).then_some(g as usize);
        let lb = self.latest_button as usize;
        let lf = self.latest_fader as usize;
        let mut out = Out::NONE;
        match op {
            Rst => self.reset(),
            Clr => self.leds = [[LED_OFF; DIM]; DIM],
            Dim => self.dim = arg(0).clamp(0, 14),
            Rotate => {
                self.rotate = arg(0) != 0;
                self.held = [Held::default(); MAX_HELD];
            }
            Key => {
                let (x, y) = (arg(0), arg(1));
                if (0..D).contains(&x) && (0..D).contains(&y) {
                    out.mask = self.key_at(x, y, arg(2) != 0);
                }
            }
            Grp => {
                if set {
                    if Self::group_ok(v) {
                        self.current = v;
                    }
                } else {
                    out = Out::ret(self.current);
                }
            }
            GrpEn => match (set, gi(arg(0))) {
                (true, Some(g)) => self.grp[g].enabled = v != 0,
                (true, None) => {}
                (false, g) => out = Out::ret(g.map_or(0, |g| self.grp[g].enabled as i16)),
            },
            GrpRst => {
                if let Some(g) = gi(arg(0)) {
                    let gg = g as i16;
                    self.grp[g] = GROUP;
                    self.btn.iter_mut().filter(|b| b.c.group == gg).for_each(|b| *b = BUTTON);
                    self.fdr.iter_mut().filter(|f| f.c.group == gg).for_each(|f| *f = FADER);
                    self.xyp.iter_mut().filter(|p| p.c.group == gg).for_each(|p| *p = XYPAD);
                }
            }
            GrpSw => {
                if let Some(g) = gi(arg(0)) {
                    self.grp.iter_mut().for_each(|x| x.enabled = false);
                    self.grp[g].enabled = true;
                }
            }
            GrpSc => match (set, gi(arg(0))) {
                (true, Some(g)) => self.grp[g].script = script(v.wrapping_sub(1)),
                (true, None) => {}
                (false, g) => out = Out::ret(g.map_or(-1, |g| self.grp[g].script + 1)),
            },
            Grpi => out = Out::ret(self.latest_group),
            Led => {
                let (x, y) = (arg(0), arg(1));
                let on = (0..D).contains(&x) && (0..D).contains(&y);
                if set {
                    if on {
                        self.leds[x as usize][y as usize] = level(v);
                    }
                } else {
                    out = Out::ret(if on { self.leds[x as usize][y as usize] } else { LED_OFF });
                }
            }
            LedC => {
                let (x, y) = (arg(0), arg(1));
                if (0..D).contains(&x) && (0..D).contains(&y) {
                    self.leds[x as usize][y as usize] = LED_OFF;
                }
            }
            Rec => self.rectangle(arg(0), arg(1), arg(2), arg(3), level(arg(4)), level(arg(5))),
            Rct => {
                let (x1, y1) = (arg(0), arg(1));
                let (w, h) = (arg(2).wrapping_sub(x1).wrapping_add(1), arg(3).wrapping_sub(y1).wrapping_add(1));
                self.rectangle(x1, y1, w, h, level(arg(4)), level(arg(5)));
            }
            Btn | Gbt => {
                let (g, a) = if op == Gbt { (arg(0), &a[1.min(a.len())..]) } else { (self.current, a) };
                let arg = |k: usize| a.get(k).copied().unwrap_or(0);
                self.init_button(g, arg(0), arg(1), arg(2), arg(3), arg(4), arg(5), arg(6), arg(7).wrapping_sub(1));
            }
            Btx | Gbx => {
                let (g, a) = if op == Gbx { (arg(0), &a[1.min(a.len())..]) } else { (self.current, a) };
                let arg = |k: usize| a.get(k).copied().unwrap_or(0);
                let (cx, cy) = (arg(8), arg(9));
                if cx > 0 && cy > 0 {
                    let (cx, cy) = (cx.min(D), cy.min(D));
                    let (w, h) = (arg(3), arg(4));
                    for y in 0..cy {
                        for x in 0..cx {
                            let i = arg(0).wrapping_add(y * cx + x);
                            self.init_button(g, i, arg(1).wrapping_add(w.wrapping_mul(x)), arg(2).wrapping_add(h.wrapping_mul(y)), w, h, arg(5), arg(6), arg(7).wrapping_sub(1));
                        }
                    }
                }
            }
            BtnEn | BtnX | BtnY | BtnV | BtnL => {
                let i = bi(arg(0));
                if set {
                    if let Some(i) = i {
                        let b = &mut self.btn[i];
                        match op {
                            BtnEn => b.c.enabled = v != 0,
                            BtnX => Self::move_common(&mut b.c, Some(v), None),
                            BtnY => Self::move_common(&mut b.c, None, Some(v)),
                            BtnV => b.state = v != 0,
                            _ => b.c.level = level(v),
                        }
                    }
                } else {
                    out = Out::ret(i.map_or(0, |i| {
                        let b = &self.btn[i];
                        match op {
                            BtnEn => b.c.enabled as i16,
                            BtnX => b.c.x,
                            BtnY => b.c.y,
                            BtnV => b.state as i16,
                            _ => b.c.level,
                        }
                    }));
                }
            }
            Btni => out = Out::ret(self.latest_button),
            Btnx | Btny | Btnv | Btnl => {
                let b = &mut self.btn[lb];
                if set {
                    match op {
                        Btnx => Self::move_common(&mut b.c, Some(v), None),
                        Btny => Self::move_common(&mut b.c, None, Some(v)),
                        Btnv => b.state = v != 0,
                        _ => b.c.level = level(v),
                    }
                } else {
                    out = Out::ret(match op {
                        Btnx => b.c.x,
                        Btny => b.c.y,
                        Btnv => b.state as i16,
                        _ => b.c.level,
                    });
                }
            }
            BtnSw => {
                if let Some(id) = bi(arg(0)) {
                    let g = self.btn[id].c.group;
                    self.btn.iter_mut().filter(|b| b.c.group == g).for_each(|b| b.state = false);
                    self.btn[id].state = true;
                }
            }
            BtnPr => {
                if let Some(i) = bi(arg(0)) {
                    let b = &mut self.btn[i];
                    b.state = if b.latch { !b.state } else { arg(1) != 0 };
                    let c = b.c;
                    self.latest_button = i as i16;
                    self.latest_group = c.group;
                    out.run = self.run_pair(&c);
                }
            }
            GbtnV => {
                let (g, on) = (arg(0), arg(1) != 0);
                if gi(g).is_some() {
                    self.btn.iter_mut().filter(|b| b.c.group == g).for_each(|b| b.state = on);
                }
            }
            GbtnL => {
                let (g, odd, even) = (arg(0), level(arg(1)), level(arg(2)));
                if gi(g).is_some() {
                    let mut is_odd = false;
                    for b in self.btn.iter_mut().filter(|b| b.c.group == g) {
                        b.c.level = if is_odd { odd } else { even };
                        is_odd = !is_odd;
                    }
                }
            }
            GbtnC => out = Out::ret(if gi(arg(0)).is_some() { self.on_in(arg(0)).count() as i16 } else { 0 }),
            GbtnI => {
                let id = if gi(arg(0)).is_some() { self.on_in(arg(0)).nth(arg(1).max(0) as usize).filter(|_| arg(1) >= 0).map_or(-1, |(i, _)| i as i16) } else { -1 };
                out = Out::ret(id);
            }
            GbtnW | GbtnH | GbtnX1 | GbtnX2 | GbtnY1 | GbtnY2 => {
                let g = arg(0);
                let none = if matches!(op, GbtnW | GbtnH) { 0 } else { -1 };
                let horizontal = matches!(op, GbtnW | GbtnX1 | GbtnX2);
                let mut span: Option<(i16, i16)> = None;
                if gi(g).is_some() {
                    for (_, b) in self.on_in(g) {
                        let p = if horizontal { b.c.x } else { b.c.y };
                        span = Some(span.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
                    }
                }
                out = Out::ret(span.map_or(none, |(lo, hi)| match op {
                    GbtnW | GbtnH => hi - lo + 1,
                    GbtnX1 | GbtnY1 => lo,
                    _ => hi,
                }));
            }
            Fdr | Gfd => {
                let (g, a) = if op == Gfd { (arg(0), &a[1.min(a.len())..]) } else { (self.current, a) };
                let arg = |k: usize| a.get(k).copied().unwrap_or(0);
                self.init_fader(g, arg(0), arg(1), arg(2), arg(3), arg(4), arg(5), arg(6), arg(7).wrapping_sub(1));
            }
            Fdx | Gfx => {
                let (g, a) = if op == Gfx { (arg(0), &a[1.min(a.len())..]) } else { (self.current, a) };
                let arg = |k: usize| a.get(k).copied().unwrap_or(0);
                let (cx, cy) = (arg(8), arg(9));
                if cx > 0 && cy > 0 {
                    let (cx, cy) = (cx.min(D), cy.min(D));
                    let (w, h) = (arg(3), arg(4));
                    for y in 0..cy {
                        for x in 0..cx {
                            let i = arg(0).wrapping_add(y * cx + x);
                            self.init_fader(g, i, arg(1).wrapping_add(w.wrapping_mul(x)), arg(2).wrapping_add(h.wrapping_mul(y)), w, h, arg(5), arg(6), arg(7).wrapping_sub(1));
                        }
                    }
                }
            }
            FdrEn | FdrX | FdrY | FdrN | FdrV | FdrL => {
                let i = fi(arg(0));
                if set {
                    if let Some(i) = i {
                        self.fader_set(op, i, v);
                    }
                } else {
                    out = Out::ret(i.map_or(0, |i| self.fader_get(op, i)));
                }
            }
            Fdri => out = Out::ret(self.latest_fader),
            Fdrx | Fdry | Fdrn | Fdrv | Fdrl => {
                let op = match op {
                    Fdrx => FdrX,
                    Fdry => FdrY,
                    Fdrn => FdrN,
                    Fdrv => FdrV,
                    _ => FdrL,
                };
                if set {
                    self.fader_set(op, lf, v);
                } else {
                    out = Out::ret(self.fader_get(op, lf));
                }
            }
            FdrPr => {
                if let Some(i) = fi(arg(0)) {
                    self.set_fader_n(i, arg(1).wrapping_sub(1));
                    self.latest_fader = i as i16;
                    let c = self.fdr[i].c;
                    self.latest_group = c.group;
                    out.run = self.run_pair(&c);
                }
            }
            GfdrN | GfdrV => {
                let g = arg(0);
                if let Some(gu) = gi(g) {
                    let gr = self.grp[gu];
                    let n = arg(1);
                    for i in 0..FADERS {
                        if self.fdr[i].c.group == g {
                            let max = self.fader_max(i);
                            self.fdr[i].value = if op == GfdrN {
                                n.max(0).min(max)
                            } else {
                                let v = n.max(gr.min).min(gr.max);
                                scale(gr.min as i32, gr.max as i32, 0, max as i32, v as i32) as i16
                            };
                        }
                    }
                }
            }
            GfdrL => {
                let g = arg(0);
                if gi(g).is_some() {
                    let mut is_odd = false;
                    for i in 0..FADERS {
                        if self.fdr[i].c.group == g {
                            self.set_fader_level(i, if is_odd { arg(1) } else { arg(2) });
                            is_odd = !is_odd;
                        }
                    }
                }
            }
            GfdrRn => {
                if let Some(g) = gi(arg(0)) {
                    self.grp[g].min = arg(1);
                    self.grp[g].max = arg(2);
                }
            }
            Xyp => {
                if let (Some(i), Some((x, y, w, h))) = (xi(arg(0)), clamp_area(arg(1), arg(2), arg(3), arg(4))) {
                    self.xyp[i] = XyPad { c: Common { enabled: true, group: self.current, x, y, w, h, level: level(arg(5)), script: script(arg(6).wrapping_sub(1)) }, vx: 0, vy: 0 };
                }
            }
            XypX | XypY => out = Out::ret(xi(arg(0)).map_or(0, |i| if op == XypX { self.xyp[i].vx } else { self.xyp[i].vy })),
        }
        if out.ret.is_none() || set {
            // Anything that isn't a plain read may have changed the picture.
            self.dirty = true;
        }
        out
    }

    fn fader_get(&self, op: GOp, i: usize) -> i16 {
        let f = &self.fdr[i];
        match op {
            GOp::FdrEn => f.c.enabled as i16,
            GOp::FdrX => f.c.x,
            GOp::FdrY => f.c.y,
            GOp::FdrN => f.value,
            GOp::FdrV => self.fader_scaled(i),
            _ => f.c.level,
        }
    }

    fn fader_set(&mut self, op: GOp, i: usize, v: i16) {
        match op {
            GOp::FdrEn => self.fdr[i].c.enabled = v != 0,
            GOp::FdrX => {
Self::move_common(&mut self.fdr[i].c, Some(v), None)
            }
            GOp::FdrY => {
Self::move_common(&mut self.fdr[i].c, None, Some(v))
            }
            GOp::FdrN => self.set_fader_n(i, v),
            GOp::FdrV => self.set_fader_scaled(i, v),
            _ => self.set_fader_level(i, v),
        }
    }

    /// A key on the grid, in the grid's own coordinates (G.ROTATE turns it
    /// round first). Returns the scripts to run, as bits 0-9.
    pub fn key(&mut self, x: usize, y: usize, z: bool) -> u16 {
        let (x, y) = (x.min(DIM) as i16, y.min(DIM) as i16);
        let (x, y) = if self.rotate { (self.cols - x - 1, self.rows - y - 1) } else { (x, y) };
        if !(0..D).contains(&x) || !(0..D).contains(&y) {
            return 0;
        }
        self.key_at(x, y, z)
    }

    /// A key in the module's coordinates: from the grid after rotation, or
    /// from G.KEY as it is.
    fn key_at(&mut self, x: i16, y: i16, z: bool) -> u16 {
        // Held keys: for fine faders' repeat and for slides.
        if z {
            if let Some(h) = self.held.iter_mut().find(|h| h.used == 0 || (h.x, h.y) == (x, y)) {
                *h = Held { used: 1, x, y, left: HOLD_DELAY };
            }
        } else {
            for h in self.held.iter_mut().filter(|h| (h.x, h.y) == (x, y)) {
                h.used = 0;
            }
        }
        let mut mask = 0u16;
        let mut refresh = false;
        let fire = |mask: &mut u16, s: i16| {
            if s >= 0 {
                *mask |= 1 << s;
            }
        };

        if z {
            for i in 0..XYPADS {
                let c = self.xyp[i].c;
                if c.enabled && self.grp[c.group as usize].enabled && c.within(x, y) {
                    self.xyp[i].vx = x - c.x;
                    self.xyp[i].vy = y - c.y;
                    fire(&mut mask, c.script);
                    self.latest_group = c.group;
                    fire(&mut mask, self.grp[c.group as usize].script);
                    refresh = true;
                }
            }
            for i in 0..FADERS {
                let c = self.fdr[i].c;
                if !(c.enabled && self.grp[c.group as usize].enabled && c.within(x, y)) {
                    continue;
                }
                let kind = self.fdr[i].kind;
                let vertical = kind & 1 == 1;
                // Another key already held on this fader (not in line with
                // this one) makes this press a slide to it.
                let mut held = self.held.iter().find(|h| h.used > 0 && if vertical { h.y != y } else { h.x != x } && c.within(h.x, h.y)).copied();
                let f = &mut self.fdr[i];
                let level = c.level as i32;
                let fine = |pos: i32, span: i32| {
                    let v = (((pos << 1) + 1) * level) / (span - 2).max(1);
                    ((v >> 1) + (v & 1)) as i16
                };
                let slide_to = |f: &mut Fader, end: i16, delta: i16| {
                    f.slide = true;
                    f.acc = 0;
                    f.end = end;
                    f.delta = delta.max(1);
                    f.up = end > f.value;
                };
                match kind {
                    CH_BAR | CH_DOT | CV_BAR | CV_DOT => {
                        let v = if vertical { c.h + c.y - y - 1 } else { x - c.x };
                        if held.is_none() {
                            f.slide = false;
                            f.value = v;
                        } else {
                            slide_to(f, v, 16);
                        }
                    }
                    _ => {
                        let (pos, first, last, span) = if vertical {
                            // Vertical fine faders count up from the bottom.
                            (c.h + c.y - y - 2, y == c.y + c.h - 1, y == c.y, c.h)
                        } else {
                            (x - c.x - 1, x == c.x, x == c.x + c.w - 1, c.w)
                        };
                        if let Some(h) = held {
                            let edge = if vertical { h.y == c.y || h.y == c.y + c.h - 1 } else { h.x == c.x || h.x == c.x + c.w - 1 };
                            if edge {
                                held = None;
                            }
                        }
                        if held.is_none() {
                            f.slide = false;
                            if first {
                                f.value = (f.value - 1).max(0);
                            } else if last {
                                if f.value < c.level {
                                    f.value += 1;
                                }
                            } else {
                                f.value = fine(pos as i32, span as i32);
                            }
                        } else {
                            let end = if first {
                                0
                            } else if last {
                                c.level
                            } else {
                                fine(pos as i32, span as i32)
                            };
                            let delta = if level > 0 { (((span as i32 - 2) << 4) / level) as i16 } else { 1 };
                            slide_to(f, end, delta);
                        }
                    }
                }
                fire(&mut mask, c.script);
                self.latest_fader = i as i16;
                self.latest_group = c.group;
                fire(&mut mask, self.grp[c.group as usize].script);
                refresh = true;
            }
        }

        for i in 0..BUTTONS {
            let c = self.btn[i].c;
            if !(c.enabled && self.grp[c.group as usize].enabled && c.within(x, y)) {
                continue;
            }
            let b = &mut self.btn[i];
            if b.latch {
                if z {
                    b.state = !b.state;
                    fire(&mut mask, c.script);
                }
            } else {
                b.state = z;
                fire(&mut mask, c.script);
            }
            self.latest_button = i as i16;
            self.latest_group = c.group;
            fire(&mut mask, self.grp[c.group as usize].script);
            refresh = true;
        }
        if refresh {
            self.dirty = true;
        }
        mask
    }

    /// A key held past the hold delay: fine faders' end keys keep stepping.
    fn repeat(&mut self, x: i16, y: i16) -> u16 {
        let mut mask = 0u16;
        for i in 0..FADERS {
            let c = self.fdr[i].c;
            if !(c.enabled && self.grp[c.group as usize].enabled && c.within(x, y)) {
                continue;
            }
            let f = &mut self.fdr[i];
            let (down, up) = match f.kind {
                FH_BAR | FH_DOT => (x == c.x, x == c.x + c.w - 1),
                FV_BAR | FV_DOT => (y == c.y + c.h - 1, y == c.y),
                _ => (false, false),
            };
            if down {
                f.value = (f.value - 1).max(0);
            } else if up {
                if f.value < c.level {
                    f.value += 1;
                }
            } else {
                continue;
            }
            for s in self.run_pair(&c) {
                if s >= 0 {
                    mask |= 1 << s;
                }
            }
            self.latest_fader = i as i16;
            self.latest_group = c.group;
            self.dirty = true;
        }
        mask
    }

    /// Faders sliding toward a key: a step whenever their delay has passed.
    fn slide(&mut self) -> u16 {
        let mut mask = 0u16;
        for i in 0..FADERS {
            let f = &mut self.fdr[i];
            if !f.slide {
                continue;
            }
            f.acc += 1;
            if f.acc < f.delta {
                continue;
            }
            f.acc = 0;
            f.value += if f.up { 1 } else { -1 };
            if (f.up && f.value >= f.end) || (!f.up && f.value <= f.end) {
                f.value = f.end;
                f.slide = false;
            }
            let c = f.c;
            self.latest_fader = i as i16;
            self.latest_group = c.group;
            for s in self.run_pair(&c) {
                if s >= 0 {
                    mask |= 1 << s;
                }
            }
            self.dirty = true;
        }
        mask
    }

    /// One millisecond: held keys repeat, faders slide. Returns the scripts
    /// to run, as bits 0-9.
    pub fn tick_ms(&mut self) -> u16 {
        let mut mask = 0;
        for k in 0..MAX_HELD {
            let h = self.held[k];
            if h.used == 0 {
                continue;
            }
            if h.left > 1 {
                self.held[k].left -= 1;
                continue;
            }
            self.held[k].used = 2;
            self.held[k].left = REPEAT;
            mask |= self.repeat(h.x, h.y);
        }
        self.slew += 1;
        if self.slew >= SLEW_MS {
            self.slew = 0;
            mask |= self.slide();
        }
        mask
    }

    /// The picture: controls, then the LED layer, then G.DIM, then
    /// G.ROTATE. `out` is 16 x 16 levels, row-major.
    pub fn render(&self, out: &mut [u8; DIM * DIM]) {
        let (sx, sy) = (self.cols, self.rows);
        out.fill(0);
        let mut fill = |x: i16, y: i16, w: i16, h: i16, l: i16| {
            if l == LED_OFF {
                return;
            }
            for yy in y.max(0)..(y.saturating_add(h)).min(sy) {
                for xx in x.max(0)..(x.saturating_add(w)).min(sx) {
                    let p = &mut out[yy as usize * DIM + xx as usize];
                    *p = match l {
                        LED_DIM => p.saturating_sub(3),
                        LED_BRI => (*p + 3).min(15),
                        l => l.clamp(0, 15) as u8,
                    };
                }
            }
        };
        let on = |c: &Common| c.enabled && self.grp[c.group as usize].enabled;

        for p in self.xyp.iter().filter(|p| on(&p.c)) {
            if p.vx != 0 || p.vy != 0 {
                let c = p.c;
                let (x, y) = (c.x + p.vx, c.y + p.vy);
                fill(c.x, y, c.w, 1, c.level);
                fill(x, c.y, 1, c.h, c.level);
                fill(x, y, 1, 1, ON);
            }
        }

        for f in self.fdr.iter().filter(|f| on(&f.c)) {
            let c = f.c;
            let v = f.value;
            // A fine fader's value in sixteenths of a key.
            let fine = |span: i16| -> (i16, i16) {
                if c.level <= 0 {
                    return (0, 0);
                }
                let fl = ((v as i32 * (span as i32 - 2)) << 5) / c.level as i32;
                let fl = ((fl >> 1) + (fl & 1)) as i16;
                (fl >> 4, fl & 15)
            };
            match f.kind {
                CH_BAR => {
                    fill(c.x, c.y, v + 1, c.h, ON);
                    fill(c.x + v + 1, c.y, c.w - v - 1, c.h, c.level);
                }
                CH_DOT => {
                    fill(c.x, c.y, c.w, c.h, c.level);
                    fill(c.x + v, c.y, 1, c.h, ON);
                }
                CV_BAR => {
                    fill(c.x, c.y, c.w, c.h - v - 1, c.level);
                    fill(c.x, c.y + c.h - v - 1, c.w, v + 1, ON);
                }
                CV_DOT => {
                    fill(c.x, c.y, c.w, c.h, c.level);
                    fill(c.x, c.y + c.h - v - 1, c.w, 1, ON);
                }
                FH_BAR | FH_DOT => {
                    let (full, part) = fine(c.w);
                    if f.kind == FH_BAR {
                        fill(c.x + 1, c.y, full, c.h, 15);
                        if part > 0 {
                            fill(c.x + full + 1, c.y, 1, c.h, part);
                        }
                    } else if part > 0 {
                        let l = if c.w - 1 >= c.level { ON } else { part.max(3) };
                        fill(c.x + full + 1, c.y, 1, c.h, l);
                    } else if full > 0 {
                        fill(c.x + full, c.y, 1, c.h, 15);
                    }
                    fill(c.x, c.y, 1, c.h, ON);
                    fill(c.x + c.w - 1, c.y, 1, c.h, ON);
                }
                _ => {
                    let (full, part) = fine(c.h);
                    if f.kind == FV_BAR {
                        fill(c.x, c.y + c.h - 1 - full, c.w, full, 15);
                        if part > 0 {
                            fill(c.x, c.y + c.h - 2 - full, c.w, 1, part);
                        }
                    } else if part > 0 {
                        let l = if c.h - 1 >= c.level { ON } else { part.max(3) };
                        fill(c.x, c.y + c.h - full - 2, c.w, 1, l);
                    } else if full > 0 {
                        fill(c.x, c.y + c.h - full - 1, c.w, 1, 15);
                    }
                    fill(c.x, c.y + c.h - 1, c.w, 1, ON);
                    fill(c.x, c.y, c.w, 1, ON);
                }
            }
        }

        for b in self.btn.iter().filter(|b| on(&b.c)) {
            fill(b.c.x, b.c.y, b.c.w, b.c.h, if b.state { ON } else { b.c.level });
        }

        for y in 0..sy {
            for x in 0..sx {
                let p = &mut out[y as usize * DIM + x as usize];
                match self.leds[x as usize][y as usize] {
                    l if l >= 0 => *p = l as u8,
                    LED_DIM => *p = p.saturating_sub(3),
                    LED_BRI => *p = (*p + 3).min(15),
                    _ => {}
                }
                *p = p.saturating_sub(self.dim as u8);
            }
        }

        if self.rotate {
            let (sx, sy) = (sx as usize, sy as usize);
            for k in 0..(sx * sy) / 2 {
                let (x, y) = (k % sx, k / sx);
                out.swap(y * DIM + x, (sy - 1 - y) * DIM + (sx - 1 - x));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use GOp::*;

    fn op(g: &mut TtGrid, o: GOp, a: &[i16]) -> Option<i16> {
        g.op(o, false, a, 0).ret
    }
    fn set(g: &mut TtGrid, o: GOp, a: &[i16], v: i16) {
        g.op(o, true, a, v);
    }
    fn pic(g: &TtGrid) -> [u8; DIM * DIM] {
        let mut out = [0; DIM * DIM];
        g.render(&mut out);
        out
    }
    fn at(p: &[u8; DIM * DIM], x: usize, y: usize) -> u8 {
        p[y * DIM + x]
    }

    #[test]
    fn buttons_latch_or_follow_the_key_and_fire_their_scripts() {
        let mut g = TtGrid::new();
        // G.BTN id x y w h latch level script
        op(&mut g, Btn, &[0, 2, 1, 2, 1, 1, 4, 3]);
        op(&mut g, Btn, &[1, 0, 0, 1, 1, 0, 6, 0]);
        set(&mut g, GrpSc, &[0], 9);
        let p = pic(&g);
        assert_eq!((at(&p, 2, 1), at(&p, 3, 1), at(&p, 0, 0), at(&p, 4, 1)), (4, 4, 6, 0));
        // A latching button toggles on the press only; script 3 and the
        // group's script M (8) run.
        assert_eq!(g.key(3, 1, true), 1 << 2 | 1 << 8);
        assert_eq!(g.key(3, 1, false), 1 << 8, "a release of a latching key still fires the group");
        assert_eq!(op(&mut g, BtnV, &[0]), Some(1));
        assert_eq!(at(&pic(&g), 2, 1), ON as u8);
        g.key(2, 1, true);
        assert_eq!(op(&mut g, BtnV, &[0]), Some(0));
        // A momentary one follows the key.
        g.key(0, 0, true);
        assert_eq!((op(&mut g, Btnv, &[]), op(&mut g, Btni, &[])), (Some(1), Some(1)));
        g.key(0, 0, false);
        assert_eq!(op(&mut g, Btnv, &[]), Some(0));
        // A disabled group's controls neither draw nor play.
        set(&mut g, GrpEn, &[0], 0);
        assert_eq!(g.key(0, 0, true), 0);
        assert_eq!(pic(&g), [0; DIM * DIM]);
    }

    #[test]
    fn button_blocks_and_group_queries() {
        let mut g = TtGrid::new();
        // G.BTX id x y w h latch level script countx county: 4 x 2 latching.
        op(&mut g, Btx, &[10, 0, 2, 1, 1, 1, 3, 0, 4, 2]);
        g.key(1, 2, true);
        g.key(3, 3, true);
        assert_eq!(op(&mut g, GbtnC, &[0]), Some(2));
        assert_eq!(op(&mut g, GbtnI, &[0, 1]), Some(17));
        assert_eq!((op(&mut g, GbtnX1, &[0]), op(&mut g, GbtnX2, &[0]), op(&mut g, GbtnW, &[0])), (Some(1), Some(3), Some(3)));
        assert_eq!((op(&mut g, GbtnY1, &[0]), op(&mut g, GbtnH, &[0])), (Some(2), Some(2)));
        op(&mut g, BtnSw, &[12]);
        assert_eq!((op(&mut g, GbtnC, &[0]), op(&mut g, BtnV, &[12])), (Some(1), Some(1)));
        // A control off the edge is cropped, one past it refused.
        op(&mut g, Btn, &[0, 14, 0, 4, 1, 0, 5, 0]);
        op(&mut g, Btn, &[1, 16, 0, 1, 1, 0, 5, 0]);
        assert_eq!((op(&mut g, BtnEn, &[0]), op(&mut g, BtnEn, &[1])), (Some(1), Some(0)));
        g.key(15, 0, true);
        assert_eq!(op(&mut g, Btni, &[]), Some(0));
    }

    #[test]
    fn coarse_faders_take_the_key_and_scale_to_their_group() {
        let mut g = TtGrid::new();
        // G.FDR id x y w h type level script: an 8-key horizontal bar.
        op(&mut g, Fdr, &[0, 0, 0, 8, 1, 0, 3, 2]);
        assert_eq!(g.key(5, 0, true), 1 << 1);
        assert_eq!(op(&mut g, FdrN, &[0]), Some(5));
        assert_eq!(op(&mut g, FdrV, &[0]), Some(scale(0, 7, 0, 16383, 5) as i16));
        let p = pic(&g);
        assert_eq!((at(&p, 0, 0), at(&p, 5, 0), at(&p, 6, 0)), (ON as u8, ON as u8, 3));
        op(&mut g, GfdrRn, &[0, 0, 70]);
        assert_eq!(op(&mut g, FdrV, &[0]), Some(50));
        set(&mut g, FdrV, &[0], 10);
        assert_eq!(op(&mut g, FdrN, &[0]), Some(1));
        // A vertical one counts up from the bottom.
        op(&mut g, Fdr, &[1, 10, 0, 1, 8, 1, 3, 0]);
        g.key(10, 6, true);
        assert_eq!((op(&mut g, FdrN, &[1]), op(&mut g, Fdri, &[])), (Some(1), Some(1)));
    }

    #[test]
    fn fine_faders_step_from_their_ends_and_repeat_while_held() {
        let mut g = TtGrid::new();
        // A fine horizontal bar 8 keys wide with 0..60.
        op(&mut g, Fdr, &[0, 0, 0, 8, 1, 4, 60, 0]);
        g.key(7, 0, true);
        g.key(7, 0, false);
        assert_eq!(op(&mut g, FdrN, &[0]), Some(1));
        // Middle keys spread 0..60 over six keys: key 3 is the third.
        g.key(3, 0, true);
        g.key(3, 0, false);
        assert_eq!(op(&mut g, FdrN, &[0]), Some(25));
        // Held, the top end repeats after 700 ms every 40 ms.
        g.key(7, 0, true);
        for _ in 0..699 {
            g.tick_ms();
        }
        assert_eq!(op(&mut g, FdrN, &[0]), Some(26));
        g.tick_ms();
        assert_eq!(op(&mut g, FdrN, &[0]), Some(27));
        for _ in 0..80 {
            g.tick_ms();
        }
        assert_eq!(op(&mut g, FdrN, &[0]), Some(29));
        g.key(7, 0, false);
        for _ in 0..200 {
            g.tick_ms();
        }
        assert_eq!(op(&mut g, FdrN, &[0]), Some(29));
        // The level clamps to what the keys can show.
        set(&mut g, FdrL, &[0], 500);
        assert_eq!(op(&mut g, FdrL, &[0]), Some((6 << 4) - 1));
    }

    #[test]
    fn a_second_key_slides_a_fader_there() {
        let mut g = TtGrid::new();
        op(&mut g, Fdr, &[0, 0, 0, 8, 1, 0, 3, 0]);
        g.key(1, 0, true);
        g.key(4, 0, true);
        assert_eq!(op(&mut g, FdrN, &[0]), Some(1), "slides, doesn't jump");
        // A step per 16 ticks of 25 ms.
        for _ in 0..400 {
            g.tick_ms();
        }
        assert_eq!(op(&mut g, FdrN, &[0]), Some(2));
        for _ in 0..800 {
            g.tick_ms();
        }
        assert_eq!(op(&mut g, FdrN, &[0]), Some(4));
    }

    #[test]
    fn the_led_layer_draws_over_the_controls_then_dim_and_rotate() {
        let mut g = TtGrid::new();
        op(&mut g, Btn, &[0, 0, 0, 2, 1, 0, 9, 0]);
        set(&mut g, Led, &[0, 0], LED_DIM);
        set(&mut g, Led, &[1, 0], LED_BRI);
        set(&mut g, Led, &[5, 5], 7);
        assert_eq!(op(&mut g, Led, &[5, 5]), Some(7));
        assert_eq!(op(&mut g, Led, &[20, 5]), Some(LED_OFF));
        let p = pic(&g);
        assert_eq!((at(&p, 0, 0), at(&p, 1, 0), at(&p, 5, 5)), (6, 12, 7));
        op(&mut g, Dim, &[4]);
        let p = pic(&g);
        assert_eq!((at(&p, 0, 0), at(&p, 5, 5), at(&p, 6, 6)), (2, 3, 0));
        op(&mut g, Dim, &[0]);
        // A 16 x 8 grid turned round.
        op(&mut g, Rotate, &[1]);
        let p = pic(&g);
        assert_eq!((at(&p, 15, 7), at(&p, 14, 7), at(&p, 10, 2)), (6, 12, 7));
        // Keys turn round too: the far corner is button 0.
        g.key(15, 7, true);
        assert_eq!(op(&mut g, Btnv, &[]), Some(1));
        op(&mut g, Clr, &[]);
        assert_eq!(op(&mut g, Led, &[5, 5]), Some(LED_OFF));
    }

    #[test]
    fn rectangles_have_a_border_and_a_fill() {
        let mut g = TtGrid::new();
        op(&mut g, Rec, &[1, 1, 4, 3, 2, 9]);
        let p = pic(&g);
        let row = |y: usize| (0..6).map(|x| at(&p, x, y)).collect::<Vec<_>>();
        assert_eq!(row(0), [0, 0, 0, 0, 0, 0]);
        assert_eq!(row(1), [0, 9, 9, 9, 9, 0]);
        assert_eq!(row(2), [0, 9, 2, 2, 9, 0]);
        assert_eq!(row(3), [0, 9, 9, 9, 9, 0]);
        // G.RCT takes corners.
        op(&mut g, Clr, &[]);
        op(&mut g, Rct, &[2, 0, 2, 5, 0, 5]);
        let p = pic(&g);
        assert_eq!((at(&p, 2, 0), at(&p, 2, 5), at(&p, 3, 0)), (5, 5, 0));
    }

    #[test]
    fn xy_pads_and_press_ops() {
        let mut g = TtGrid::new();
        op(&mut g, Xyp, &[0, 8, 0, 8, 8, 3, 4]);
        assert_eq!(g.key(10, 5, true), 1 << 3);
        assert_eq!((op(&mut g, XypX, &[0]), op(&mut g, XypY, &[0])), (Some(2), Some(5)));
        let p = pic(&g);
        assert_eq!((at(&p, 10, 5), at(&p, 8, 5), at(&p, 10, 0), at(&p, 9, 4)), (ON as u8, 3, 3, 0));
        // G.BTN.PR presses from a script and names what to run.
        op(&mut g, Btn, &[7, 0, 0, 1, 1, 1, 5, 2]);
        set(&mut g, GrpSc, &[0], 10);
        let out = g.op(BtnPr, false, &[7, 1], 0);
        assert_eq!((out.run, op(&mut g, BtnV, &[7])), ([1, 9], Some(1)));
        op(&mut g, Fdr, &[3, 0, 1, 8, 1, 0, 5, 0]);
        let out = g.op(FdrPr, false, &[3, 4], 0);
        assert_eq!((out.run, op(&mut g, FdrN, &[3]), op(&mut g, Fdri, &[])), ([-1, 9], Some(3), Some(3)));
        // G.KEY is a key press that ignores rotation.
        op(&mut g, Rotate, &[1]);
        g.op(Key, false, &[0, 0, 1], 0);
        assert_eq!(op(&mut g, BtnV, &[7]), Some(0));
        op(&mut g, Rst, &[]);
        assert_eq!((op(&mut g, BtnEn, &[7]), op(&mut g, GrpSc, &[0])), (Some(0), Some(0)));
    }
}
