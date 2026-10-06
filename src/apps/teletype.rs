//! Teletype: monome's algorithmic ecosystem, the Eurorack module you program
//! in a tiny prefix language, running as a Portamax app.
//!
//! A scene is eight trigger scripts (1-8), a metronome script (M) and an
//! init script (I), six lines each, plus four 64-step number patterns. A
//! script runs when its trigger fires; M runs every M milliseconds. Lines
//! are commands like `CV 1 N ADD 48 P.NEXT` or `EVERY 4: TR.P 2`, evaluated
//! right to left: numbers push, operators pop their arguments and push a
//! result, and an operator standing first on a line with one value too many
//! *sets* instead of gets (`X 5` stores 5; `X` reads it). `:` ends a PRE
//! (`IF`, `L`, `EVERY`, `DEL`, `PROB`...), `;` separates sub-commands.
//!
//! The interpreter is written here from the Teletype manual and op reference
//! (monome.org/docs/teletype), with the rules that the manual leaves vague
//! checked against the firmware's behaviour: the set-if-first-with-an-extra-
//! value rule, EVERY/SKIP/OTHER counting per line, the L loop's mutable I,
//! W's 10000-iteration cap, DEL's 64 slots, an 8-deep SCRIPT call limit,
//! P.NEXT/P.PREV wrapping, the integer rounding of QT/SCALE/AVG, DIV and
//! MOD by zero giving 0, N/V/VV's volt tables. No firmware code is copied
//! (the firmware is GPL-2.0; Portamax is MIT).
//!
//! **Implemented**: variables A B C D X Y Z T, I J K, O, DRUNK, FLIP, R, TIME,
//! LAST; maths, comparison, logic and bitwise ops, RAND/RRAND/TOSS, N V VV,
//! BPM, ER; CV, CV.SET, CV.SLEW, CV.OFF, TR, TR.P, TR.TIME, TR.TOG, TR.POL,
//! IN, PARAM (+.SCALE), M, M.ACT, M.RESET; the PREs IF ELIF ELSE L W EVERY
//! SKIP OTHER PROB DEL S; DEL.CLR, S.ALL S.POP S.CLR S.L, SCRIPT/$, BREAK,
//! KILL, SYNC; the pattern ops P/PN with .N .L .WRAP .START .END .I .HERE
//! .NEXT .PREV .INS .RM .PUSH .POP .MIN .MAX .RND .REV .ROT .SHUF .+ .- .+W
//! .-W; and Kria's ops, as Teletype has them for Ansible: KR.PAT, KR.POS,
//! KR.L.ST, KR.L.LEN, KR.RES, KR.CV, KR.MUTE, KR.TMUTE, KR.CLK, KR.PG,
//! KR.CUE, KR.DIR, KR.DUR, KR.PERIOD, KR.SCALE, KR.PRE (see kria.rs; they
//! read 0 and do nothing when there is no Kria). And the grid ops, all of
//! them: G.RST/CLR/DIM/ROTATE/KEY, groups (G.GRP...), the LED layer (G.LED,
//! G.REC, G.RCT), buttons (G.BTN, G.BTX, G.GBT, G.GBX and their queries),
//! faders (G.FDR... with all eight types, hold-repeat and slides) and XY
//! pads (G.XYP); see teletype_grid.rs. Scenes load and save in the module's own text format (the files its
//! USB stick reads and writes: `#1`..`#8`, `#M`, `#I`, `#P`).
//!
//! **Not implemented**: the other I2C/expander ops (Ansible's other apps, Just Friends, ER-301,
//! TXo, crow, Disting...), the FADER expander's ops, grid control mode, MIDI ops, Q, the turtle,
//! CHAOS, functions ($F, $L, $S), SCENE ops, DEL.X/R/G/B, P.MAP, SCALE0,
//! EXP, the rotation ops, hex/binary literals, comment lines and
//! SCRIPT.POL. A scene line that uses one does not load (it shows as an
//! error on that line) rather than running wrongly.
//!
//! **ER**: Euclidean rhythms come from a Bresenham spread that puts a hit on
//! step 0, which gives the same rhythms as Teletype's tables; for some fills
//! the firmware's tables may be a rotation of these (not checked on hardware).
//!
//! **On Portamax** (there are no jacks):
//! - Trigger inputs 1-8 are pads 1-8 on the SCRIPTS page (9 and 10 run M and
//!   I) and the mod inputs `Teletype: Trig 1`..`8` (a rising edge past 0.5
//!   fires the script, so any app's gate or LFO can drive one).
//! - IN is the mod input `Teletype: IN` plus the left hand sensor; PARAM is
//!   the Param knob in the menu plus `Teletype: PARAM` and the right hand.
//! - The four CV and four TR outputs can each be routed to any app's mod
//!   input (menu rows), CV as 0..1 for 0..10 V and TR as 0 or 1.
//! - TR n also plays a note at CV n's pitch for as long as it is high
//!   (`N 60` = 5 V = middle C, as the manual has it), to the built-in voice
//!   or to an app the Plays row names, so a scene is audible with nothing
//!   patched. Set Plays to none to use the outputs only as CV.
//! - Time is Teletype's own millisecond clock, not the device tempo (M is in
//!   ms, like the module; `M BPM 120` gives 500 ms). F3 starts and stops the
//!   metro: it runs while F3 is on *and* M.ACT is on. On the module a scene
//!   starts playing as soon as it loads; here, like every Portamax app,
//!   nothing plays until you start it (the pads and trigger inputs still
//!   run scripts at any time).
//!
//! - The grid is the shared grid (the Grid app's screen grid, or a real
//!   monome grid): Teletype is one of its apps, and a scene whose scripts
//!   use a G op takes the grid when it loads. Its 16 x 16 space sits at the
//!   grid's top-left. Grid presses run scripts whether or not F3 is on.
//!
//! Editing is by pad, a word at a time: the D-pad moves along the line and
//! between lines, F2 turns the pad pages (numbers, variables, I/O, maths,
//! logic, flow, patterns, the tracker), a pad inserts its word at the caret,
//! SELECT deletes the word before it. A line that would not run on the module
//! (wrong number of arguments, a PRE without `:`...) is marked and skipped.
//!
//! The interpreter runs on the audio thread, a millisecond at a time, and
//! never allocates there: lines are fixed arrays of words, edits arrive
//! through a queue, and the screen reads a copy published after each change.

use super::kids_kit;
use crate::{
    app::play_kit::{self as kit, KitConfig, Layer, PlayHost, PlayKit, Routes},
    app::{App, Input, SlintExtra},
    audio::AudioProcessor,
    audio_bus::AudioBus,
    display::FrameBuffer,
    mixer_bus::MixerBus,
    modbus::{ModBus, Patch},
    note_bus::{NoteBus, NoteOut, NoteRoute},
    paramlist::ParamList,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12, SPLEEN_8X16},
    util::AtomicF32,
};
use super::grid_kit::{self, Grid, Leds};
use super::kria::{self, KrOp};
use super::teletype_grid::{self as tg, GOp, TtGrid};
use embedded_graphics::{mono_font::MonoTextStyle, pixelcolor::Rgb565, prelude::*, primitives::{PrimitiveStyle, Rectangle}, text::Text};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    Arc, Mutex,
};

const APP_NAME: &str = "Teletype";

/// Bundled scenes in `scenes/`, saved ones in `saved/`.
const TT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/teletype");

/// The module's own limits.
const LINES: usize = 6;
const WORDS: usize = 16;
const SCRIPTS: usize = 10; // 1-8, M (8), I (9)
const METRO: usize = 8;
const INIT: usize = 9;
/// J and K for commands run from the live line or the pads.
const LIVE: usize = 10;
const PAT_LEN: usize = 64;
const DELAYS: usize = 64;
const STACK_OPS: usize = 16;
const EXEC_DEPTH: u8 = 8;
const WHILE_DEPTH: u16 = 10_000;
const METRO_MIN: i16 = 25;
const CV_MAX: i16 = 16383;

// Own palette: the module's amber-on-black OLED.
const BG: Rgb565 = Rgb565::new(1, 2, 1);
const INK: Rgb565 = Rgb565::new(28, 44, 10);
const BRIGHT: Rgb565 = Rgb565::new(31, 58, 18);
const ACCENT: Rgb565 = Rgb565::new(10, 50, 28);
const DIM: Rgb565 = Rgb565::new(12, 20, 5);
const FAINT: Rgb565 = Rgb565::new(5, 9, 3);
const ERR: Rgb565 = Rgb565::new(31, 14, 8);

// ---------------------------------------------------------------- the language

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PatOp {
    Val,
    Len,
    Wrap,
    Start,
    End,
    Idx,
    Here,
    Next,
    Prev,
    Ins,
    Rm,
    Push,
    Pop,
    Min,
    Max,
    Rnd,
    Rev,
    Rot,
    Shuf,
    Add,
    Sub,
    AddW,
    SubW,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// A B C D X Y Z T.
    Var(u8),
    I,
    J,
    K,
    O,
    OInc,
    OMin,
    OMax,
    OWrap,
    Drunk,
    DrunkMin,
    DrunkMax,
    DrunkWrap,
    Flip,
    Time,
    TimeAct,
    Last,
    R,
    RMin,
    RMax,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Abs,
    Sgn,
    Min,
    Max,
    Lim,
    Wrap,
    Qt,
    Avg,
    Scale,
    Eq,
    Ne,
    Lt,
    Gt,
    Lte,
    Gte,
    Inr,
    Outr,
    Inri,
    Outri,
    And,
    Or,
    Ez,
    Nz,
    Lsh,
    Rsh,
    BAnd,
    BOr,
    BXor,
    BNot,
    BSet,
    BGet,
    BClr,
    BTog,
    Tern,
    Rand,
    RRand,
    Toss,
    N,
    V,
    VV,
    Bpm,
    Er,
    Cv,
    CvSet,
    CvSlew,
    CvOff,
    Tr,
    TrP,
    TrTime,
    TrTog,
    TrPol,
    In,
    Param,
    InScale,
    ParamScale,
    M,
    MAct,
    MReset,
    DelClr,
    SAll,
    SPop,
    SClr,
    SL,
    Script,
    Break,
    Kill,
    Sync,
    PNum,
    /// A pattern op on the working pattern (P.N).
    P(PatOp),
    /// The same op with the pattern number as its first argument (PN...).
    PN(PatOp),
    /// Kria's ops, as Teletype has them for Ansible.
    Kr(KrOp),
    /// The grid ops (teletype_grid.rs).
    G(GOp),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Md {
    If,
    Elif,
    Else,
    L,
    W,
    Every,
    Skip,
    Other,
    Prob,
    Del,
    S,
}

impl Md {
    fn params(self) -> usize {
        match self {
            Md::Else | Md::Other | Md::S => 0,
            Md::L => 2,
            _ => 1,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Md::If => "IF",
            Md::Elif => "ELIF",
            Md::Else => "ELSE",
            Md::L => "L",
            Md::W => "W",
            Md::Every => "EVERY",
            Md::Skip => "SKIP",
            Md::Other => "OTHER",
            Md::Prob => "PROB",
            Md::Del => "DEL",
            Md::S => "S",
        }
    }
    const ALL: [Md; 11] = [Md::If, Md::Elif, Md::Else, Md::L, Md::W, Md::Every, Md::Skip, Md::Other, Md::Prob, Md::Del, Md::S];
}

/// One word of a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok {
    Num(i16),
    Op(Op),
    Mod(Md),
    /// `:`, between a PRE and the command it runs.
    Pre,
    /// `;`, between sub-commands.
    Sub,
}

/// An operator's shape: how many values it takes, whether it gives one back,
/// and whether it can be set (first on a line, with one value more).
struct OpDef {
    op: Op,
    name: &'static str,
    params: u8,
    ret: bool,
    set: bool,
}

const fn g(op: Op, name: &'static str, params: u8) -> OpDef {
    OpDef { op, name, params, ret: true, set: false }
}
const fn gs(op: Op, name: &'static str, params: u8) -> OpDef {
    OpDef { op, name, params, ret: true, set: true }
}
const fn act(op: Op, name: &'static str, params: u8) -> OpDef {
    OpDef { op, name, params, ret: false, set: false }
}

/// Every word the interpreter knows. The first entry for an op is the name it
/// is written back with; later ones are aliases (`+` for ADD...).
static OPS: &[OpDef] = &[
    gs(Op::Var(0), "A", 0),
    gs(Op::Var(1), "B", 0),
    gs(Op::Var(2), "C", 0),
    gs(Op::Var(3), "D", 0),
    gs(Op::Var(4), "X", 0),
    gs(Op::Var(5), "Y", 0),
    gs(Op::Var(6), "Z", 0),
    gs(Op::Var(7), "T", 0),
    gs(Op::I, "I", 0),
    gs(Op::J, "J", 0),
    gs(Op::K, "K", 0),
    gs(Op::O, "O", 0),
    gs(Op::OInc, "O.INC", 0),
    gs(Op::OMin, "O.MIN", 0),
    gs(Op::OMax, "O.MAX", 0),
    gs(Op::OWrap, "O.WRAP", 0),
    gs(Op::Drunk, "DRUNK", 0),
    gs(Op::DrunkMin, "DRUNK.MIN", 0),
    gs(Op::DrunkMax, "DRUNK.MAX", 0),
    gs(Op::DrunkWrap, "DRUNK.WRAP", 0),
    gs(Op::Flip, "FLIP", 0),
    gs(Op::Time, "TIME", 0),
    gs(Op::TimeAct, "TIME.ACT", 0),
    g(Op::Last, "LAST", 1),
    g(Op::R, "R", 0),
    gs(Op::RMin, "R.MIN", 0),
    gs(Op::RMax, "R.MAX", 0),
    g(Op::Add, "ADD", 2),
    g(Op::Add, "+", 2),
    g(Op::Sub, "SUB", 2),
    g(Op::Sub, "-", 2),
    g(Op::Mul, "MUL", 2),
    g(Op::Mul, "*", 2),
    g(Op::Div, "DIV", 2),
    g(Op::Div, "/", 2),
    g(Op::Mod, "MOD", 2),
    g(Op::Mod, "%", 2),
    g(Op::Abs, "ABS", 1),
    g(Op::Sgn, "SGN", 1),
    g(Op::Min, "MIN", 2),
    g(Op::Max, "MAX", 2),
    g(Op::Lim, "LIM", 3),
    g(Op::Wrap, "WRAP", 3),
    g(Op::Wrap, "WRP", 3),
    g(Op::Qt, "QT", 2),
    g(Op::Avg, "AVG", 2),
    g(Op::Scale, "SCALE", 5),
    g(Op::Scale, "SCL", 5),
    g(Op::Eq, "EQ", 2),
    g(Op::Eq, "==", 2),
    g(Op::Ne, "NE", 2),
    g(Op::Ne, "!=", 2),
    g(Op::Lt, "LT", 2),
    g(Op::Lt, "<", 2),
    g(Op::Gt, "GT", 2),
    g(Op::Gt, ">", 2),
    g(Op::Lte, "LTE", 2),
    g(Op::Lte, "<=", 2),
    g(Op::Gte, "GTE", 2),
    g(Op::Gte, ">=", 2),
    g(Op::Inr, "INR", 3),
    g(Op::Inr, "><", 3),
    g(Op::Outr, "OUTR", 3),
    g(Op::Outr, "<>", 3),
    g(Op::Inri, "INRI", 3),
    g(Op::Inri, ">=<", 3),
    g(Op::Outri, "OUTRI", 3),
    g(Op::Outri, "<=>", 3),
    g(Op::And, "AND", 2),
    g(Op::And, "&&", 2),
    g(Op::Or, "OR", 2),
    g(Op::Or, "||", 2),
    g(Op::Ez, "EZ", 1),
    g(Op::Ez, "!", 1),
    g(Op::Nz, "NZ", 1),
    g(Op::Lsh, "LSH", 2),
    g(Op::Lsh, "<<", 2),
    g(Op::Rsh, "RSH", 2),
    g(Op::Rsh, ">>", 2),
    g(Op::BAnd, "&", 2),
    g(Op::BOr, "|", 2),
    g(Op::BXor, "^", 2),
    g(Op::BNot, "~", 1),
    g(Op::BSet, "BSET", 2),
    g(Op::BGet, "BGET", 2),
    g(Op::BClr, "BCLR", 2),
    g(Op::BTog, "BTOG", 2),
    g(Op::Tern, "?", 3),
    g(Op::Rand, "RAND", 1),
    g(Op::Rand, "RND", 1),
    g(Op::RRand, "RRAND", 2),
    g(Op::RRand, "RRND", 2),
    g(Op::Toss, "TOSS", 0),
    g(Op::N, "N", 1),
    g(Op::V, "V", 1),
    g(Op::VV, "VV", 1),
    g(Op::Bpm, "BPM", 1),
    g(Op::Er, "ER", 3),
    gs(Op::Cv, "CV", 1),
    act(Op::CvSet, "CV.SET", 2),
    gs(Op::CvSlew, "CV.SLEW", 1),
    gs(Op::CvOff, "CV.OFF", 1),
    gs(Op::Tr, "TR", 1),
    act(Op::TrP, "TR.P", 1),
    act(Op::TrP, "TR.PULSE", 1),
    gs(Op::TrTime, "TR.TIME", 1),
    act(Op::TrTog, "TR.TOG", 1),
    gs(Op::TrPol, "TR.POL", 1),
    g(Op::In, "IN", 0),
    g(Op::Param, "PARAM", 0),
    g(Op::Param, "PRM", 0),
    act(Op::InScale, "IN.SCALE", 2),
    act(Op::ParamScale, "PARAM.SCALE", 2),
    gs(Op::M, "M", 0),
    gs(Op::MAct, "M.ACT", 0),
    act(Op::MReset, "M.RESET", 0),
    act(Op::DelClr, "DEL.CLR", 0),
    act(Op::SAll, "S.ALL", 0),
    act(Op::SPop, "S.POP", 0),
    act(Op::SClr, "S.CLR", 0),
    g(Op::SL, "S.L", 0),
    gs(Op::Script, "SCRIPT", 0),
    gs(Op::Script, "$", 0),
    act(Op::Break, "BREAK", 0),
    act(Op::Break, "BRK", 0),
    act(Op::Kill, "KILL", 0),
    act(Op::Sync, "SYNC", 1),
    gs(Op::PNum, "P.N", 0),
    gs(Op::P(PatOp::Val), "P", 1),
    gs(Op::PN(PatOp::Val), "PN", 2),
    gs(Op::P(PatOp::Len), "P.L", 0),
    gs(Op::PN(PatOp::Len), "PN.L", 1),
    gs(Op::P(PatOp::Wrap), "P.WRAP", 0),
    gs(Op::PN(PatOp::Wrap), "PN.WRAP", 1),
    gs(Op::P(PatOp::Start), "P.START", 0),
    gs(Op::PN(PatOp::Start), "PN.START", 1),
    gs(Op::P(PatOp::End), "P.END", 0),
    gs(Op::PN(PatOp::End), "PN.END", 1),
    gs(Op::P(PatOp::Idx), "P.I", 0),
    gs(Op::PN(PatOp::Idx), "PN.I", 1),
    gs(Op::P(PatOp::Here), "P.HERE", 0),
    gs(Op::PN(PatOp::Here), "PN.HERE", 1),
    gs(Op::P(PatOp::Next), "P.NEXT", 0),
    gs(Op::PN(PatOp::Next), "PN.NEXT", 1),
    gs(Op::P(PatOp::Prev), "P.PREV", 0),
    gs(Op::PN(PatOp::Prev), "PN.PREV", 1),
    act(Op::P(PatOp::Ins), "P.INS", 2),
    act(Op::PN(PatOp::Ins), "PN.INS", 3),
    act(Op::P(PatOp::Rm), "P.RM", 1),
    act(Op::PN(PatOp::Rm), "PN.RM", 2),
    act(Op::P(PatOp::Push), "P.PUSH", 1),
    act(Op::PN(PatOp::Push), "PN.PUSH", 2),
    g(Op::P(PatOp::Pop), "P.POP", 0),
    g(Op::PN(PatOp::Pop), "PN.POP", 1),
    g(Op::P(PatOp::Min), "P.MIN", 0),
    g(Op::PN(PatOp::Min), "PN.MIN", 1),
    g(Op::P(PatOp::Max), "P.MAX", 0),
    g(Op::PN(PatOp::Max), "PN.MAX", 1),
    g(Op::P(PatOp::Rnd), "P.RND", 0),
    g(Op::PN(PatOp::Rnd), "PN.RND", 1),
    act(Op::P(PatOp::Rev), "P.REV", 0),
    act(Op::PN(PatOp::Rev), "PN.REV", 1),
    act(Op::P(PatOp::Rot), "P.ROT", 1),
    act(Op::PN(PatOp::Rot), "PN.ROT", 2),
    act(Op::P(PatOp::Shuf), "P.SHUF", 0),
    act(Op::PN(PatOp::Shuf), "PN.SHUF", 1),
    act(Op::P(PatOp::Add), "P.+", 2),
    act(Op::PN(PatOp::Add), "PN.+", 3),
    act(Op::P(PatOp::Sub), "P.-", 2),
    act(Op::PN(PatOp::Sub), "PN.-", 3),
    act(Op::P(PatOp::AddW), "P.+W", 4),
    act(Op::PN(PatOp::AddW), "PN.+W", 5),
    act(Op::P(PatOp::SubW), "P.-W", 4),
    act(Op::PN(PatOp::SubW), "PN.-W", 5),
    kr(KrOp::Pre, "KR.PRE"),
    kr(KrOp::Period, "KR.PERIOD"),
    kr(KrOp::Pat, "KR.PAT"),
    kr(KrOp::Scale, "KR.SCALE"),
    kr(KrOp::Pos, "KR.POS"),
    kr(KrOp::LSt, "KR.L.ST"),
    kr(KrOp::LLen, "KR.L.LEN"),
    kr(KrOp::Res, "KR.RES"),
    kr(KrOp::Cv, "KR.CV"),
    kr(KrOp::Mute, "KR.MUTE"),
    kr(KrOp::TMute, "KR.TMUTE"),
    kr(KrOp::Clk, "KR.CLK"),
    kr(KrOp::Pg, "KR.PG"),
    kr(KrOp::Cue, "KR.CUE"),
    kr(KrOp::Dir, "KR.DIR"),
    kr(KrOp::Dur, "KR.DUR"),
    gr(GOp::Rst, "G.RST"),
    gr(GOp::Clr, "G.CLR"),
    gr(GOp::Dim, "G.DIM"),
    gr(GOp::Rotate, "G.ROTATE"),
    gr(GOp::Key, "G.KEY"),
    gr(GOp::Grp, "G.GRP"),
    gr(GOp::GrpEn, "G.GRP.EN"),
    gr(GOp::GrpRst, "G.GRP.RST"),
    gr(GOp::GrpSw, "G.GRP.SW"),
    gr(GOp::GrpSc, "G.GRP.SC"),
    gr(GOp::Grpi, "G.GRPI"),
    gr(GOp::Led, "G.LED"),
    gr(GOp::LedC, "G.LED.C"),
    gr(GOp::Rec, "G.REC"),
    gr(GOp::Rct, "G.RCT"),
    gr(GOp::Btn, "G.BTN"),
    gr(GOp::Gbt, "G.GBT"),
    gr(GOp::Btx, "G.BTX"),
    gr(GOp::Gbx, "G.GBX"),
    gr(GOp::BtnEn, "G.BTN.EN"),
    gr(GOp::BtnX, "G.BTN.X"),
    gr(GOp::BtnY, "G.BTN.Y"),
    gr(GOp::BtnV, "G.BTN.V"),
    gr(GOp::BtnL, "G.BTN.L"),
    gr(GOp::Btni, "G.BTNI"),
    gr(GOp::Btnx, "G.BTNX"),
    gr(GOp::Btny, "G.BTNY"),
    gr(GOp::Btnv, "G.BTNV"),
    gr(GOp::Btnl, "G.BTNL"),
    gr(GOp::BtnSw, "G.BTN.SW"),
    gr(GOp::BtnPr, "G.BTN.PR"),
    gr(GOp::GbtnV, "G.GBTN.V"),
    gr(GOp::GbtnL, "G.GBTN.L"),
    gr(GOp::GbtnC, "G.GBTN.C"),
    gr(GOp::GbtnI, "G.GBTN.I"),
    gr(GOp::GbtnW, "G.GBTN.W"),
    gr(GOp::GbtnH, "G.GBTN.H"),
    gr(GOp::GbtnX1, "G.GBTN.X1"),
    gr(GOp::GbtnX2, "G.GBTN.X2"),
    gr(GOp::GbtnY1, "G.GBTN.Y1"),
    gr(GOp::GbtnY2, "G.GBTN.Y2"),
    gr(GOp::Fdr, "G.FDR"),
    gr(GOp::Gfd, "G.GFD"),
    gr(GOp::Fdx, "G.FDX"),
    gr(GOp::Gfx, "G.GFX"),
    gr(GOp::FdrEn, "G.FDR.EN"),
    gr(GOp::FdrX, "G.FDR.X"),
    gr(GOp::FdrY, "G.FDR.Y"),
    gr(GOp::FdrN, "G.FDR.N"),
    gr(GOp::FdrV, "G.FDR.V"),
    gr(GOp::FdrL, "G.FDR.L"),
    gr(GOp::Fdri, "G.FDRI"),
    gr(GOp::Fdrx, "G.FDRX"),
    gr(GOp::Fdry, "G.FDRY"),
    gr(GOp::Fdrn, "G.FDRN"),
    gr(GOp::Fdrv, "G.FDRV"),
    gr(GOp::Fdrl, "G.FDRL"),
    gr(GOp::FdrPr, "G.FDR.PR"),
    gr(GOp::GfdrN, "G.GFDR.N"),
    gr(GOp::GfdrV, "G.GFDR.V"),
    gr(GOp::GfdrL, "G.GFDR.L"),
    gr(GOp::GfdrRn, "G.GFDR.RN"),
    gr(GOp::Xyp, "G.XYP"),
    gr(GOp::XypX, "G.XYP.X"),
    gr(GOp::XypY, "G.XYP.Y"),
];

/// A Kria op, shaped as Kria's link describes it.
const fn kr(op: KrOp, name: &'static str) -> OpDef {
    let (params, ret, set) = kria::Link::shape(op);
    OpDef { op: Op::Kr(op), name, params, ret, set }
}

/// A grid op, shaped as teletype_grid.rs describes it.
const fn gr(op: GOp, name: &'static str) -> OpDef {
    let (params, ret, set) = op.shape();
    OpDef { op: Op::G(op), name, params, ret, set }
}

/// Does a line use the grid?
fn uses_grid(toks: &[Tok]) -> bool {
    toks.iter().any(|t| matches!(t, Tok::Op(Op::G(_))))
}

fn def(op: Op) -> &'static OpDef {
    OPS.iter().find(|d| d.op == op).expect("every op has a table entry")
}

/// One word of text -> a token.
fn parse_word(w: &str) -> Option<Tok> {
    let up = w.to_ascii_uppercase();
    match up.as_str() {
        ":" => return Some(Tok::Pre),
        ";" => return Some(Tok::Sub),
        _ => {}
    }
    if let Some(m) = Md::ALL.iter().find(|m| m.name() == up) {
        return Some(Tok::Mod(*m));
    }
    if let Some(d) = OPS.iter().find(|d| d.name == up) {
        return Some(Tok::Op(d.op));
    }
    let digits = up.strip_prefix('-').unwrap_or(&up);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        // Teletype numbers are 16-bit; a bigger one is clamped, like typing it on the module.
        let v: i64 = up.parse().unwrap_or(0);
        return Some(Tok::Num(v.clamp(i16::MIN as i64, i16::MAX as i64) as i16));
    }
    None
}

/// A line of text -> its words. `IF X: TR.P 1` and `IF X : TR.P 1` read the same.
fn parse_line(text: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    for raw in text.split_whitespace() {
        let mut w = raw;
        let mut trailing = Vec::new();
        // A `:` or `;` stuck to the end of a word is its own word.
        while w.len() > 1 && (w.ends_with(':') || w.ends_with(';')) {
            trailing.push(if w.ends_with(':') { Tok::Pre } else { Tok::Sub });
            w = &w[..w.len() - 1];
        }
        out.push(parse_word(w).ok_or_else(|| format!("unknown word {w}"))?);
        out.extend(trailing.into_iter().rev());
    }
    if out.len() > WORDS {
        return Err(format!("{} words (16 max)", out.len()));
    }
    Ok(out)
}

fn tok_text(t: Tok) -> String {
    match t {
        Tok::Num(v) => v.to_string(),
        Tok::Op(op) => def(op).name.to_string(),
        Tok::Mod(m) => m.name().to_string(),
        Tok::Pre => ":".into(),
        Tok::Sub => ";".into(),
    }
}

/// Words -> text the module writes: `IF X: TR.P 1; CV 1 0`.
fn line_text(toks: &[Tok]) -> String {
    let mut s = String::new();
    for (i, t) in toks.iter().enumerate() {
        if i > 0 && !matches!(t, Tok::Pre | Tok::Sub) {
            s.push(' ');
        }
        s.push_str(&tok_text(*t));
    }
    s
}

/// Checks a command the way the module does before it accepts a line:
/// every operator has its arguments, a PRE comes first and ends in `:`,
/// and no sub-command leaves more than one value over.
fn validate(toks: &[Tok]) -> Result<(), String> {
    if toks.is_empty() {
        return Ok(());
    }
    let sep = toks.iter().position(|t| *t == Tok::Pre);
    if toks.iter().filter(|t| **t == Tok::Pre).count() > 1 {
        return Err("only one : per line".into());
    }
    match (toks[0], sep) {
        (Tok::Mod(_), None) => return Err("a PRE needs a :".into()),
        (_, Some(_)) if !matches!(toks[0], Tok::Mod(_)) => return Err(": only after a PRE".into()),
        _ => {}
    }
    if toks.iter().skip(1).any(|t| matches!(t, Tok::Mod(_))) {
        return Err("a PRE must come first".into());
    }
    if let (Tok::Mod(m), Some(s)) = (toks[0], sep) {
        let depth = check_sub(&toks[1..s], false)?;
        if depth != m.params() {
            return Err(format!("{} takes {}", m.name(), m.params()));
        }
        let post = &toks[s + 1..];
        if post.is_empty() {
            return Err(format!("nothing after {}:", m.name()));
        }
        for sub in post.split(|t| *t == Tok::Sub) {
            if check_sub(sub, true)? > 1 {
                return Err("extra value".into());
            }
        }
        return Ok(());
    }
    for sub in toks.split(|t| *t == Tok::Sub) {
        if check_sub(sub, true)? > 1 {
            return Err("extra value".into());
        }
    }
    Ok(())
}

/// Stack depth a sub-command leaves, right to left.
fn check_sub(toks: &[Tok], can_set: bool) -> Result<usize, String> {
    let mut depth = 0usize;
    for (i, t) in toks.iter().enumerate().rev() {
        match t {
            Tok::Num(_) => depth += 1,
            Tok::Op(op) => {
                let d = def(*op);
                let p = d.params as usize;
                if can_set && i == 0 && d.set && depth >= p + 1 {
                    depth -= p + 1;
                } else {
                    if depth < p {
                        return Err(format!("{} needs {p}", d.name));
                    }
                    depth -= p;
                    if d.ret {
                        depth += 1;
                    }
                }
            }
            Tok::Sub | Tok::Pre | Tok::Mod(_) => return Err("misplaced word".into()),
        }
    }
    Ok(depth)
}

/// A line as the audio thread holds it: a fixed array, so copying one into a
/// delay or the stack never allocates.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Line {
    t: [Tok; WORDS],
    n: u8,
    ok: bool,
}

impl Default for Line {
    fn default() -> Self {
        Line { t: [Tok::Num(0); WORDS], n: 0, ok: true }
    }
}

impl Line {
    fn from(toks: &[Tok]) -> Line {
        let mut l = Line::default();
        let n = toks.len().min(WORDS);
        l.t[..n].copy_from_slice(&toks[..n]);
        l.n = n as u8;
        l.ok = toks.len() <= WORDS && validate(toks).is_ok();
        l
    }
    fn words(&self) -> &[Tok] {
        &self.t[..self.n as usize]
    }
}

// ---------------------------------------------------------------- the machine

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pattern {
    v: [i16; PAT_LEN],
    len: i16,
    wrap: bool,
    start: i16,
    end: i16,
    idx: i16,
}

impl Default for Pattern {
    fn default() -> Self {
        Pattern { v: [0; PAT_LEN], len: 0, wrap: true, start: 0, end: 63, idx: 0 }
    }
}

impl Pattern {
    /// A negative index counts back from the length; past the end is the last step.
    fn norm(&self, idx: i16) -> usize {
        let mut i = idx as i32;
        let len = self.len as i32;
        if i < 0 {
            i = if i < -len { 0 } else { len + i };
        }
        i.clamp(0, PAT_LEN as i32 - 1) as usize
    }
    fn next(&mut self) {
        if self.idx == self.len - 1 || self.idx == self.end {
            if self.wrap {
                self.idx = self.start;
            }
        } else {
            self.idx += 1;
        }
        if self.idx > self.len || self.idx < 0 || self.idx >= PAT_LEN as i16 {
            self.idx = 0;
        }
    }
    fn prev(&mut self) {
        if self.idx == 0 || self.idx == self.start {
            if self.wrap {
                self.idx = if self.end < self.len { self.end } else { self.len - 1 };
            }
        } else {
            self.idx -= 1;
        }
        self.idx = self.idx.clamp(0, PAT_LEN as i16 - 1);
    }
    /// START..END in order, inside the pattern.
    fn span(&self) -> (usize, usize) {
        let a = self.start.clamp(0, 63) as usize;
        let b = self.end.clamp(0, 63) as usize;
        (a.min(b), a.max(b))
    }
}

/// WRAP: into lo..hi inclusive.
fn wrap(i: i32, a: i32, b: i32) -> i32 {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    (i - lo).rem_euclid(hi - lo + 1) + lo
}

/// Volts in Teletype's 0..16383 = 0..10 V scale.
fn volts(v: i32) -> i32 {
    (v as f32 * 1638.4).round() as i32
}
fn note_volts(n: i32) -> i32 {
    let n = n.clamp(-127, 127);
    let v = ((n.abs() * 16384) as f32 / 120.0).round() as i32;
    if n < 0 { -v } else { v }
}

/// Euclidean rhythm: is `step` a hit when `fill` hits are spread over `len`?
fn euclid(fill: i32, len: i32, step: i32) -> bool {
    if len < 1 || fill < 1 {
        return false;
    }
    let len = len.min(32);
    let fill = fill.min(len);
    let s = step.rem_euclid(len);
    (s * fill) % len < fill
}

#[derive(Clone, Copy, Default)]
struct CvOut {
    target: i16,
    cur: f32,
    step: f32,
    left: i32,
    slew: i16,
    off: i16,
}

impl CvOut {
    fn out(&self) -> i16 {
        (self.cur.round() as i32 + self.off as i32).clamp(0, CV_MAX as i32) as i16
    }
    /// Where the output is heading: what a note started now should play,
    /// since a script usually sets CV and pulses TR in the same breath.
    fn destination(&self) -> i16 {
        (self.target as i32 + self.off as i32).clamp(0, CV_MAX as i32) as i16
    }
}

#[derive(Clone, Copy)]
struct TrOut {
    on: bool,
    pol: bool,
    time: i16,
    timer: i32,
}

/// A command waiting to run: a delay (counting down) or one on the stack.
#[derive(Clone, Copy, Default)]
struct Pending {
    line: Line,
    ms: i32,
    script: u8,
    i: i16,
}

#[derive(Clone, Copy)]
struct Vars {
    v: [i16; 8],
    o: i16,
    o_inc: i16,
    o_min: i16,
    o_max: i16,
    o_wrap: bool,
    drunk: i16,
    drunk_min: i16,
    drunk_max: i16,
    drunk_wrap: bool,
    flip: bool,
    time: i16,
    time_act: bool,
    r_min: i16,
    r_max: i16,
    m: i16,
    m_act: bool,
    p_n: i16,
    in_range: (i16, i16),
    param_range: (i16, i16),
}

impl Default for Vars {
    fn default() -> Self {
        Vars {
            v: [1, 2, 3, 4, 0, 0, 0, 0],
            o: 0,
            o_inc: 1,
            o_min: 0,
            o_max: 63,
            o_wrap: true,
            drunk: 0,
            drunk_min: 0,
            drunk_max: 255,
            drunk_wrap: false,
            flip: false,
            time: 0,
            time_act: true,
            r_min: 0,
            r_max: CV_MAX,
            m: 1000,
            m_act: true,
            p_n: 0,
            in_range: (0, CV_MAX),
            param_range: (0, CV_MAX),
        }
    }
}

/// Per-script-call state, as the module keeps it on its exec stack.
#[derive(Clone, Copy)]
struct Frame {
    script: u8,
    line: u8,
    i: i16,
    if_else: bool,
    breaking: bool,
    while_continue: bool,
    while_depth: u16,
}

impl Frame {
    fn top(script: usize, i: i16) -> Frame {
        Frame { script: script as u8, line: 0, i, if_else: true, breaking: false, while_continue: false, while_depth: 0 }
    }
}

struct Stack {
    v: [i16; 16],
    n: usize,
}

impl Stack {
    fn new() -> Stack {
        Stack { v: [0; 16], n: 0 }
    }
    fn push(&mut self, x: i16) {
        if self.n < self.v.len() {
            self.v[self.n] = x;
            self.n += 1;
        }
    }
    fn pop(&mut self) -> i16 {
        if self.n == 0 {
            return 0;
        }
        self.n -= 1;
        self.v[self.n]
    }
}

/// A scene's content: what loading one replaces.
#[derive(Clone)]
struct Scene {
    scripts: [[Line; LINES]; SCRIPTS],
    pats: [Pattern; 4],
}

impl Default for Scene {
    fn default() -> Self {
        Scene { scripts: [[Line::default(); LINES]; SCRIPTS], pats: [Pattern::default(); 4] }
    }
}

/// The interpreter and everything it drives. Lives on the audio thread.
struct Engine {
    scripts: [[Line; LINES]; SCRIPTS],
    pats: [Pattern; 4],
    vars: Vars,
    jk: [[i16; 2]; SCRIPTS + 1],
    every: [[(i16, i16); LINES]; SCRIPTS],
    every_last: bool,
    delays: [Pending; DELAYS],
    stack: [Pending; STACK_OPS],
    stack_n: usize,
    cv: [CvOut; 4],
    tr: [TrOut; 4],
    last_run: [u32; SCRIPTS],
    fired: [u32; SCRIPTS],
    now: u32,
    metro_acc: i32,
    depth: u8,
    rng: u32,
    /// Kria, as Teletype reaches Ansible.
    kria: Arc<kria::Link>,
    /// IN and PARAM before scaling, 0..16383.
    in_raw: i16,
    param_raw: i16,
    /// The device transport (F3). The metro runs while this and M.ACT are
    /// both on: on the module M.ACT alone decides, but on Portamax nothing
    /// plays until you start it.
    transport: bool,
    /// The grid ops' buttons, faders and LED layer.
    grid: Box<TtGrid>,
}

impl Engine {
    fn new() -> Engine {
        let mut e = Engine {
            scripts: [[Line::default(); LINES]; SCRIPTS],
            pats: [Pattern::default(); 4],
            vars: Vars::default(),
            jk: [[0; 2]; SCRIPTS + 1],
            every: [[(0, 1); LINES]; SCRIPTS],
            every_last: false,
            delays: [Pending::default(); DELAYS],
            stack: [Pending::default(); STACK_OPS],
            stack_n: 0,
            cv: [CvOut { slew: 1, ..Default::default() }; 4],
            tr: [TrOut { on: false, pol: true, time: 100, timer: 0 }; 4],
            last_run: [0; SCRIPTS],
            fired: [0; SCRIPTS],
            now: 0,
            metro_acc: 0,
            depth: 0,
            rng: 0x2545_f491,
            kria: kria::link(),
            in_raw: 0,
            param_raw: 0,
            transport: true,
            grid: Box::new(TtGrid::new()),
        };
        e.reset_state();
        e
    }

    /// A fresh scene's state: everything but its scripts and patterns.
    fn reset_state(&mut self) {
        self.vars = Vars::default();
        self.jk = [[0; 2]; SCRIPTS + 1];
        self.every = [[(0, 1); LINES]; SCRIPTS];
        self.every_last = false;
        self.delays = [Pending::default(); DELAYS];
        self.stack_n = 0;
        self.metro_acc = 0;
        for c in self.cv.iter_mut() {
            *c = CvOut { slew: 1, ..Default::default() };
        }
        for t in self.tr.iter_mut() {
            *t = TrOut { on: false, pol: true, time: 100, timer: 0 };
        }
        self.grid.reset();
    }

    /// Runs the scripts a grid key fired (bits 0-9), in order.
    fn run_mask(&mut self, mask: u16) {
        for s in 0..SCRIPTS {
            if mask & (1 << s) != 0 {
                self.run_script(s, None);
            }
        }
    }

    /// Loads a scene the way the module does: fresh state, then the init script.
    fn load(&mut self, s: &Scene) {
        self.scripts = s.scripts;
        self.pats = s.pats;
        self.reset_state();
        self.run_script(INIT, None);
    }

    /// 0..32767, a fresh draw.
    fn rand(&mut self) -> i32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 17) as i32
    }

    fn rrand(&mut self, a: i32, b: i32) -> i32 {
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        lo + self.rand() % (hi - lo + 1)
    }

    fn run_script(&mut self, script: usize, parent: Option<&Frame>) {
        if script >= SCRIPTS || self.depth >= EXEC_DEPTH {
            return;
        }
        self.depth += 1;
        let mut fr = Frame::top(script, parent.map_or(0, |p| p.i));
        if let Some(p) = parent {
            fr.if_else = p.if_else;
        }
        for l in 0..LINES {
            let line = self.scripts[script][l];
            if line.n == 0 || !line.ok {
                continue;
            }
            if fr.breaking {
                break;
            }
            fr.line = l as u8;
            loop {
                self.process(line.words(), &mut fr);
                if !(fr.while_continue && !fr.breaking) {
                    break;
                }
            }
        }
        self.last_run[script] = self.now;
        self.fired[script] = self.fired[script].wrapping_add(1);
        self.depth -= 1;
    }

    /// Runs one command from outside a script (a pad, the live line).
    fn run_live(&mut self, line: &Line) {
        if line.ok && line.n > 0 {
            let mut fr = Frame::top(LIVE, 0);
            self.process(line.words(), &mut fr);
        }
    }

    fn run_pending(&mut self, p: Pending) {
        let mut fr = Frame::top(p.script as usize, p.i);
        self.process(p.line.words(), &mut fr);
    }

    /// One command: sub-commands left to right, each right to left.
    fn process(&mut self, toks: &[Tok], fr: &mut Frame) {
        let sep = toks.iter().position(|t| *t == Tok::Pre);
        let end = sep.unwrap_or(toks.len());
        let mut start = 0;
        while start < end && !fr.breaking {
            let stop = toks[start..end].iter().position(|t| *t == Tok::Sub).map_or(end, |p| start + p);
            if stop > start {
                self.sub(toks, start, stop, sep, fr);
            }
            start = stop + 1;
        }
    }

    fn sub(&mut self, toks: &[Tok], start: usize, stop: usize, sep: Option<usize>, fr: &mut Frame) {
        let mut st = Stack::new();
        for idx in (start..stop).rev() {
            match toks[idx] {
                Tok::Num(v) => st.push(v),
                Tok::Op(op) => {
                    let d = def(op);
                    let set = idx == start && d.set && st.n > d.params as usize;
                    self.op(op, set, &mut st, fr);
                }
                Tok::Mod(m) => {
                    let post = sep.map_or(&toks[toks.len()..], |s| &toks[s + 1..]);
                    self.pre(m, &mut st, post, fr);
                }
                Tok::Pre | Tok::Sub => {}
            }
        }
    }

    fn pre(&mut self, m: Md, st: &mut Stack, post: &[Tok], fr: &mut Frame) {
        match m {
            Md::If => {
                let a = st.pop();
                fr.if_else = false;
                if a != 0 {
                    fr.if_else = true;
                    self.process(post, fr);
                }
            }
            Md::Elif => {
                let a = st.pop();
                if !fr.if_else && a != 0 {
                    fr.if_else = true;
                    self.process(post, fr);
                }
            }
            Md::Else => {
                if !fr.if_else {
                    fr.if_else = true;
                    self.process(post, fr);
                }
            }
            Md::L => {
                let a = st.pop();
                let b = st.pop();
                // I is the loop's counter but the loop runs a fixed number of
                // times: a command that changes I bends the values, not the count.
                fr.i = a;
                if a < b {
                    for _ in a as i32..=b as i32 {
                        self.process(post, fr);
                        if fr.breaking {
                            break;
                        }
                        fr.i = fr.i.wrapping_add(1);
                    }
                    if !fr.breaking {
                        fr.i = fr.i.wrapping_sub(1);
                    }
                } else {
                    for _ in (b as i32..=a as i32).rev() {
                        if fr.breaking {
                            break;
                        }
                        self.process(post, fr);
                        fr.i = fr.i.wrapping_sub(1);
                    }
                    if !fr.breaking {
                        fr.i = fr.i.wrapping_add(1);
                    }
                }
            }
            Md::W => {
                let a = st.pop();
                if a != 0 {
                    self.process(post, fr);
                    fr.while_depth += 1;
                    fr.while_continue = fr.while_depth < WHILE_DEPTH;
                } else {
                    fr.while_continue = false;
                }
            }
            Md::Every | Md::Skip => {
                let m_ = st.pop();
                let (s, l) = (fr.script as usize, fr.line as usize);
                if s >= SCRIPTS {
                    return;
                }
                let e = &mut self.every[s][l];
                e.1 = if m_ < 0 { m_.wrapping_neg() } else if m_ == 0 { 1 } else { m_ };
                e.0 %= e.1;
                e.0 = (e.0 + 1) % e.1;
                let now = if m == Md::Every { e.0 == 0 } else { e.0 != 0 };
                self.every_last = now;
                if now {
                    self.process(post, fr);
                }
            }
            Md::Other => {
                if !self.every_last {
                    self.process(post, fr);
                }
            }
            Md::Prob => {
                let a = st.pop() as i32;
                if self.rand() % 100 < a {
                    self.process(post, fr);
                }
            }
            Md::Del => {
                let ms = st.pop().max(1) as i32;
                if let Some(slot) = self.delays.iter_mut().find(|d| d.ms == 0) {
                    *slot = Pending { line: Line::from(post), ms, script: fr.script, i: fr.i };
                }
            }
            Md::S => {
                if self.stack_n < STACK_OPS {
                    self.stack[self.stack_n] = Pending { line: Line::from(post), ms: 0, script: fr.script, i: fr.i };
                    self.stack_n += 1;
                }
            }
        }
    }

    fn op(&mut self, op: Op, set: bool, st: &mut Stack, fr: &mut Frame) {
        let b = |x: bool| x as i16;
        let jk = (fr.script as usize).min(SCRIPTS);
        // `get`/`put` for the plain variables: set stores, get pushes.
        macro_rules! var {
            ($field:expr) => {{
                if set {
                    $field = st.pop();
                } else {
                    st.push($field);
                }
            }};
        }
        macro_rules! flag {
            ($field:expr) => {{
                if set {
                    $field = st.pop() != 0;
                } else {
                    st.push(b($field));
                }
            }};
        }
        match op {
            Op::Var(k) => var!(self.vars.v[k as usize]),
            Op::I => var!(fr.i),
            Op::J => var!(self.jk[jk][0]),
            Op::K => var!(self.jk[jk][1]),
            Op::O => {
                if set {
                    self.vars.o = st.pop();
                } else {
                    let v = &mut self.vars;
                    let out = v.o;
                    v.o = v.o.wrapping_add(v.o_inc);
                    if v.o_wrap {
                        if v.o < v.o_min {
                            v.o = v.o_max;
                        } else if v.o > v.o_max {
                            v.o = v.o_min;
                        }
                    } else {
                        v.o = v.o.clamp(v.o_min.min(v.o_max), v.o_max.max(v.o_min));
                    }
                    st.push(out);
                }
            }
            Op::OInc => var!(self.vars.o_inc),
            Op::OMin => var!(self.vars.o_min),
            Op::OMax => var!(self.vars.o_max),
            Op::OWrap => flag!(self.vars.o_wrap),
            Op::Drunk => {
                if set {
                    self.vars.drunk = st.pop();
                } else {
                    let step = (self.rand() % 3 - 1) as i16;
                    let v = &mut self.vars;
                    v.drunk = v.drunk.wrapping_add(step);
                    if v.drunk_wrap {
                        if v.drunk < v.drunk_min {
                            v.drunk = v.drunk_max;
                        } else if v.drunk > v.drunk_max {
                            v.drunk = v.drunk_min;
                        }
                    } else {
                        v.drunk = v.drunk.clamp(v.drunk_min.min(v.drunk_max), v.drunk_max.max(v.drunk_min));
                    }
                    st.push(v.drunk);
                }
            }
            Op::DrunkMin => var!(self.vars.drunk_min),
            Op::DrunkMax => var!(self.vars.drunk_max),
            Op::DrunkWrap => flag!(self.vars.drunk_wrap),
            Op::Flip => {
                if set {
                    self.vars.flip = st.pop() != 0;
                } else {
                    self.vars.flip = !self.vars.flip;
                    st.push(b(self.vars.flip));
                }
            }
            Op::Time => var!(self.vars.time),
            Op::TimeAct => flag!(self.vars.time_act),
            Op::Last => {
                let s = st.pop() as i32 - 1;
                let v = if (0..SCRIPTS as i32).contains(&s) { self.now.wrapping_sub(self.last_run[s as usize]).min(32767) as i16 } else { 0 };
                st.push(v);
            }
            Op::R => {
                let (lo, hi) = (self.vars.r_min as i32, self.vars.r_max as i32);
                let v = self.rrand(lo, hi);
                st.push(v as i16);
            }
            Op::RMin => var!(self.vars.r_min),
            Op::RMax => var!(self.vars.r_max),
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::Min | Op::Max | Op::Eq | Op::Ne | Op::Lt | Op::Gt | Op::Lte | Op::Gte | Op::And | Op::Or | Op::Lsh | Op::Rsh | Op::BAnd | Op::BOr | Op::BXor | Op::BSet | Op::BGet | Op::BClr | Op::BTog | Op::Qt | Op::Avg => {
                let x = st.pop();
                let y = st.pop();
                let (xi, yi) = (x as i32, y as i32);
                let r: i16 = match op {
                    Op::Add => x.wrapping_add(y),
                    Op::Sub => x.wrapping_sub(y),
                    Op::Mul => x.wrapping_mul(y),
                    Op::Div => if y != 0 { x.wrapping_div(y) } else { 0 },
                    Op::Mod => if y != 0 { x.wrapping_rem(y) } else { 0 },
                    Op::Min => x.min(y),
                    Op::Max => x.max(y),
                    Op::Eq => b(x == y),
                    Op::Ne => b(x != y),
                    Op::Lt => b(x < y),
                    Op::Gt => b(x > y),
                    Op::Lte => b(x <= y),
                    Op::Gte => b(x >= y),
                    Op::And => b(x != 0 && y != 0),
                    Op::Or => b(x != 0 || y != 0),
                    // A negative shift goes the other way.
                    Op::Lsh => if yi >= 0 { (xi << yi.min(31)) as i16 } else { (xi >> (-yi).min(31)) as i16 },
                    Op::Rsh => if yi >= 0 { (xi >> yi.min(31)) as i16 } else { (xi << (-yi).min(31)) as i16 },
                    Op::BAnd => x & y,
                    Op::BOr => x | y,
                    Op::BXor => x ^ y,
                    Op::BSet => x | (1i16.wrapping_shl(y.clamp(0, 15) as u32)),
                    Op::BGet => (x >> y.clamp(0, 15)) & 1,
                    Op::BClr => x & !(1i16.wrapping_shl(y.clamp(0, 15) as u32)),
                    Op::BTog => x ^ (1i16.wrapping_shl(y.clamp(0, 15) as u32)),
                    // The nearer multiple of y, the upper one on a tie; QT by 0 is 0.
                    Op::Qt => {
                        if yi == 0 {
                            0
                        } else {
                            let c = xi / yi;
                            let (d, e) = (c * yi, (c + 1) * yi);
                            (if (xi - d).abs() < (xi - e).abs() { d } else { e }) as i16
                        }
                    }
                    // Rounds half up, as the module does.
                    Op::Avg => {
                        let mut r = (xi * 2 + yi * 2) / 2;
                        if r % 2 != 0 {
                            r += 1;
                        }
                        (r / 2) as i16
                    }
                    _ => 0,
                };
                st.push(r);
            }
            Op::Abs => {
                let x = st.pop();
                st.push(x.wrapping_abs());
            }
            Op::Sgn => {
                let x = st.pop();
                st.push(x.signum());
            }
            Op::Ez => {
                let x = st.pop();
                st.push(b(x == 0));
            }
            Op::Nz => {
                let x = st.pop();
                st.push(b(x != 0));
            }
            Op::BNot => {
                let x = st.pop();
                st.push(!x);
            }
            Op::Lim => {
                let (i, lo, hi) = (st.pop(), st.pop(), st.pop());
                st.push(if i < lo { lo } else if i > hi { hi } else { i });
            }
            Op::Wrap => {
                let (i, lo, hi) = (st.pop() as i32, st.pop() as i32, st.pop() as i32);
                st.push(wrap(i, lo, hi) as i16);
            }
            Op::Scale => {
                let (a, b_, x, y, i) = (st.pop() as i32, st.pop() as i32, st.pop() as i32, st.pop() as i32, st.pop() as i32);
                if b_ == a {
                    st.push(0);
                } else {
                    // Half-up rounding through a doubled quotient.
                    let r = (i - a) * (y - x) * 2 / (b_ - a);
                    st.push((r / 2 + (r & 1) + x) as i16);
                }
            }
            Op::Inr | Op::Outr | Op::Inri | Op::Outri => {
                let (l, x, h) = (st.pop(), st.pop(), st.pop());
                st.push(b(match op {
                    Op::Inr => l < x && x < h,
                    Op::Outr => x < l || x > h,
                    Op::Inri => l <= x && x <= h,
                    _ => x <= l || x >= h,
                }));
            }
            Op::Tern => {
                let (c, y, z) = (st.pop(), st.pop(), st.pop());
                st.push(if c != 0 { y } else { z });
            }
            Op::Rand => {
                let a = st.pop() as i32;
                let r = self.rand();
                let v = if a < 0 { -(r % (1 - a)) } else if a == 32767 { r } else { r % (a + 1) };
                st.push(v as i16);
            }
            Op::RRand => {
                let (a, b_) = (st.pop() as i32, st.pop() as i32);
                let v = self.rrand(a, b_);
                st.push(v as i16);
            }
            Op::Toss => {
                let v = self.rand() & 1;
                st.push(v as i16);
            }
            Op::N => {
                let n = st.pop() as i32;
                st.push(note_volts(n) as i16);
            }
            Op::V => {
                let a = (st.pop() as i32).clamp(-10, 10);
                st.push((a.signum() * volts(a.abs())) as i16);
            }
            Op::VV => {
                let a = st.pop() as i32;
                let (sign, a) = (if a < 0 { -1 } else { 1 }, a.abs().min(1000));
                st.push((sign * (volts(a / 100) + ((a % 100) as f32 * 16.384).round() as i32)) as i16);
            }
            Op::Bpm => {
                let a = (st.pop() as i32).clamp(2, 1000);
                st.push((60_000.0 / a as f32).round() as i16);
            }
            Op::Er => {
                let (f, l, s) = (st.pop() as i32, st.pop() as i32, st.pop() as i32);
                st.push(b(euclid(f, l, s)));
            }
            Op::Cv | Op::CvSlew | Op::CvOff | Op::CvSet => {
                let k = st.pop() as i32 - 1;
                let Some(c) = (0..4).contains(&k).then(|| &mut self.cv[k as usize]) else {
                    if set || op == Op::CvSet {
                        st.pop();
                    } else {
                        st.push(0);
                    }
                    return;
                };
                match op {
                    Op::Cv if set => {
                        c.target = st.pop().clamp(0, CV_MAX);
                        c.left = c.slew.max(1) as i32;
                        c.step = (c.target as f32 - c.cur) / c.left as f32;
                    }
                    Op::Cv => st.push(c.target),
                    Op::CvSet => {
                        c.target = st.pop().clamp(0, CV_MAX);
                        c.cur = c.target as f32;
                        c.left = 0;
                    }
                    Op::CvSlew if set => c.slew = st.pop().max(1),
                    Op::CvSlew => st.push(c.slew),
                    Op::CvOff if set => c.off = st.pop(),
                    _ => st.push(c.off),
                }
            }
            Op::Tr | Op::TrP | Op::TrTime | Op::TrTog | Op::TrPol => {
                let k = st.pop() as i32 - 1;
                let Some(t) = (0..4).contains(&k).then(|| &mut self.tr[k as usize]) else {
                    if set {
                        st.pop();
                    } else if matches!(op, Op::Tr | Op::TrTime | Op::TrPol) {
                        st.push(0);
                    }
                    return;
                };
                match op {
                    Op::Tr if set => {
                        t.on = st.pop() != 0;
                        t.timer = 0;
                    }
                    Op::Tr => st.push(b(t.on)),
                    Op::TrP => {
                        t.on = t.pol;
                        t.timer = t.time.max(1) as i32;
                    }
                    Op::TrTime if set => t.time = st.pop().max(0),
                    Op::TrTime => st.push(t.time),
                    Op::TrTog => {
                        t.on = !t.on;
                        t.timer = 0;
                    }
                    Op::TrPol if set => t.pol = st.pop() != 0,
                    _ => st.push(b(t.pol)),
                }
            }
            Op::In => {
                let (lo, hi) = self.vars.in_range;
                st.push(scale_raw(self.in_raw, lo, hi));
            }
            Op::Param => {
                let (lo, hi) = self.vars.param_range;
                st.push(scale_raw(self.param_raw, lo, hi));
            }
            Op::InScale => {
                let (lo, hi) = (st.pop(), st.pop());
                self.vars.in_range = (lo, hi);
            }
            Op::ParamScale => {
                let (lo, hi) = (st.pop(), st.pop());
                self.vars.param_range = (lo, hi);
            }
            Op::M => {
                if set {
                    self.vars.m = st.pop().max(METRO_MIN);
                } else {
                    st.push(self.vars.m);
                }
            }
            Op::MAct => flag!(self.vars.m_act),
            Op::MReset => self.metro_acc = 0,
            Op::DelClr => self.delays = [Pending::default(); DELAYS],
            Op::SAll => {
                while self.stack_n > 0 {
                    self.stack_n -= 1;
                    let p = self.stack[self.stack_n];
                    self.run_pending(p);
                }
            }
            Op::SPop => {
                if self.stack_n > 0 {
                    self.stack_n -= 1;
                    let p = self.stack[self.stack_n];
                    self.run_pending(p);
                }
            }
            Op::SClr => self.stack_n = 0,
            Op::SL => st.push(self.stack_n as i16),
            Op::Script => {
                if set {
                    let s = st.pop() as i32 - 1;
                    if (0..SCRIPTS as i32).contains(&s) {
                        let parent = *fr;
                        self.run_script(s as usize, Some(&parent));
                    }
                } else {
                    let s = fr.script as i16 + 1;
                    st.push(if s as usize > SCRIPTS { 0 } else { s });
                }
            }
            Op::Break => fr.breaking = true,
            Op::Kill => {
                self.stack_n = 0;
                self.vars.m_act = false;
                self.delays = [Pending::default(); DELAYS];
                for t in self.tr.iter_mut() {
                    if t.timer > 0 {
                        t.timer = 0;
                        t.on = !t.pol;
                    }
                }
                for c in self.cv.iter_mut() {
                    c.cur = c.target as f32;
                    c.left = 0;
                }
            }
            Op::Sync => {
                let n = st.pop();
                self.every_last = false;
                for row in self.every.iter_mut() {
                    for e in row.iter_mut() {
                        if e.1 == 0 {
                            e.1 = 1;
                        }
                        let mut c = n as i32;
                        while c < 0 {
                            c += e.1 as i32;
                        }
                        e.0 = c as i16;
                    }
                }
            }
            Op::PNum => {
                if set {
                    self.vars.p_n = st.pop().clamp(0, 3);
                } else {
                    st.push(self.vars.p_n);
                }
            }
            Op::P(p) => {
                let pn = self.vars.p_n.clamp(0, 3) as usize;
                self.pattern_op(pn, p, set, st);
            }
            Op::PN(p) => {
                let pn = st.pop().clamp(0, 3) as usize;
                self.pattern_op(pn, p, set, st);
            }
            Op::Kr(k) => {
                let (params, ret, _) = kria::Link::shape(k);
                let mut args = [0i16; 2];
                for a in args.iter_mut().take(params as usize) {
                    *a = st.pop();
                }
                let args = &args[..params as usize];
                if set {
                    let v = st.pop();
                    self.kria.set(k, args, v);
                } else if ret {
                    st.push(self.kria.get(k, args));
                } else {
                    self.kria.set(k, args, 0);
                }
            }
            Op::G(g) => {
                let (params, ret, _) = g.shape();
                let mut args = [0i16; 11];
                for a in args.iter_mut().take(params as usize) {
                    *a = st.pop();
                }
                let v = if set { st.pop() } else { 0 };
                let out = self.grid.op(g, set, &args[..params as usize], v);
                if let (Some(r), false, true) = (out.ret, set, ret) {
                    st.push(r);
                }
                let parent = *fr;
                for s in out.run {
                    if s >= 0 {
                        self.run_script(s as usize, Some(&parent));
                    }
                }
                self.run_mask(out.mask);
            }
        }
    }

    fn pattern_op(&mut self, pn: usize, op: PatOp, set: bool, st: &mut Stack) {
        let r = self.rand();
        let pat = &mut self.pats[pn];
        match op {
            PatOp::Val => {
                let i = pat.norm(st.pop());
                if set {
                    pat.v[i] = st.pop();
                } else {
                    st.push(pat.v[i]);
                }
            }
            PatOp::Len => {
                if set {
                    pat.len = st.pop().clamp(0, PAT_LEN as i16);
                } else {
                    st.push(pat.len);
                }
            }
            PatOp::Wrap => {
                if set {
                    pat.wrap = st.pop() != 0;
                } else {
                    st.push(pat.wrap as i16);
                }
            }
            PatOp::Start => {
                if set {
                    pat.start = st.pop().clamp(0, 63);
                } else {
                    st.push(pat.start);
                }
            }
            PatOp::End => {
                if set {
                    pat.end = st.pop().clamp(0, 63);
                } else {
                    st.push(pat.end);
                }
            }
            PatOp::Idx => {
                if set {
                    let i = pat.norm(st.pop()) as i16;
                    pat.idx = i.min(pat.len);
                } else {
                    st.push(pat.idx);
                }
            }
            PatOp::Here => {
                let i = pat.idx.clamp(0, 63) as usize;
                if set {
                    pat.v[i] = st.pop();
                } else {
                    st.push(pat.v[i]);
                }
            }
            PatOp::Next | PatOp::Prev => {
                if op == PatOp::Next {
                    pat.next();
                } else {
                    pat.prev();
                }
                let i = pat.idx.clamp(0, 63) as usize;
                if set {
                    pat.v[i] = st.pop();
                } else {
                    st.push(pat.v[i]);
                }
            }
            PatOp::Ins => {
                let i = pat.norm(st.pop());
                let v = st.pop();
                let len = pat.len.clamp(0, PAT_LEN as i16) as usize;
                if i <= len {
                    let top = len.min(PAT_LEN - 1);
                    pat.v.copy_within(i..top, i + 1);
                    pat.v[i] = v;
                    pat.len = (len + 1).min(PAT_LEN) as i16;
                }
            }
            PatOp::Rm => {
                let i = pat.norm(st.pop());
                let len = pat.len.max(0) as usize;
                if i < len {
                    pat.v.copy_within(i + 1..len, i);
                    pat.len -= 1;
                }
            }
            PatOp::Push => {
                let v = st.pop();
                if (pat.len as usize) < PAT_LEN {
                    pat.v[pat.len.max(0) as usize] = v;
                    pat.len += 1;
                }
            }
            PatOp::Pop => {
                if pat.len > 0 {
                    pat.len -= 1;
                    st.push(pat.v[pat.len as usize]);
                } else {
                    st.push(0);
                }
            }
            PatOp::Min | PatOp::Max => {
                let (a, z) = pat.span();
                let mut best = a;
                for i in a..=z {
                    if (op == PatOp::Min && pat.v[i] < pat.v[best]) || (op == PatOp::Max && pat.v[i] > pat.v[best]) {
                        best = i;
                    }
                }
                st.push(best as i16);
            }
            PatOp::Rnd => {
                let (a, z) = pat.span();
                st.push(pat.v[a + r as usize % (z - a + 1)]);
            }
            PatOp::Rev => {
                let (a, z) = pat.span();
                pat.v[a..=z].reverse();
            }
            PatOp::Rot => {
                let n = st.pop() as i32;
                let (a, z) = pat.span();
                let len = (z - a + 1) as i32;
                let k = n.rem_euclid(len) as usize;
                pat.v[a..=z].rotate_right(k);
            }
            PatOp::Shuf => {
                let (a, z) = pat.span();
                let mut seed = r as u32 | 1;
                for i in (a + 1..=z).rev() {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let j = a + (seed as usize % (i - a + 1));
                    pat.v.swap(i, j);
                }
            }
            PatOp::Add | PatOp::Sub | PatOp::AddW | PatOp::SubW => {
                let i = pat.norm(st.pop());
                let d = st.pop();
                let mut v = if matches!(op, PatOp::Add | PatOp::AddW) { pat.v[i].wrapping_add(d) } else { pat.v[i].wrapping_sub(d) };
                if matches!(op, PatOp::AddW | PatOp::SubW) {
                    let (lo, hi) = (st.pop() as i32, st.pop() as i32);
                    v = wrap(v as i32, lo, hi) as i16;
                }
                pat.v[i] = v;
            }
        }
    }

    /// One millisecond of the module's clock: delays, pulses, slews, the metro.
    fn ms(&mut self) {
        self.now = self.now.wrapping_add(1);
        if self.vars.time_act {
            self.vars.time = if self.vars.time >= i16::MAX { 0 } else { self.vars.time + 1 };
        }
        for k in 0..DELAYS {
            if self.delays[k].ms > 0 {
                self.delays[k].ms -= 1;
                if self.delays[k].ms == 0 {
                    let p = self.delays[k];
                    self.run_pending(p);
                }
            }
        }
        for t in self.tr.iter_mut() {
            if t.timer > 0 {
                t.timer -= 1;
                if t.timer == 0 {
                    t.on = !t.pol;
                }
            }
        }
        for c in self.cv.iter_mut() {
            if c.left > 0 {
                c.cur += c.step;
                c.left -= 1;
                if c.left == 0 {
                    c.cur = c.target as f32;
                }
            }
        }
        let fired = self.grid.tick_ms();
        self.run_mask(fired);
        if self.vars.m_act && self.transport {
            self.metro_acc += 1;
            if self.metro_acc >= self.vars.m.max(METRO_MIN) as i32 {
                self.metro_acc = 0;
                self.run_script(METRO, None);
            }
        }
    }
}

fn scale_raw(raw: i16, lo: i16, hi: i16) -> i16 {
    (lo as i32 + (raw.clamp(0, CV_MAX) as i32 * (hi as i32 - lo as i32)) / CV_MAX as i32) as i16
}

/// CV in Teletype units -> MIDI note: `N 60` is 5 V, middle C.
fn cv_note(v: i16) -> u8 {
    ((v as f32 * 120.0 / 16384.0).round() as i32).clamp(0, 127) as u8
}

// ---------------------------------------------------------------- scene files

/// A scene as text: the scripts as words (so a bad line can still be shown
/// and fixed) plus the patterns.
#[derive(Clone, Debug, PartialEq)]
struct SceneText {
    scripts: Vec<Vec<Vec<Tok>>>,
    pats: [Pattern; 4],
    /// Lines that did not load, as "script line: why".
    errors: Vec<String>,
}

impl Default for SceneText {
    fn default() -> Self {
        SceneText { scripts: vec![vec![Vec::new(); LINES]; SCRIPTS], pats: [Pattern::default(); 4], errors: Vec::new() }
    }
}

fn script_name(s: usize) -> String {
    match s {
        METRO => "M".into(),
        INIT => "I".into(),
        _ => format!("{}", s + 1),
    }
}

impl SceneText {
    /// The module's scene file: a description, then `#1`..`#8`, `#M`, `#I`
    /// with one command a line, then `#P` (lengths, wraps, starts, ends,
    /// then 64 rows of four values, tab separated). `#G` (grid) is skipped.
    fn parse(text: &str) -> SceneText {
        let mut s = SceneText::default();
        let mut section: Option<String> = None;
        let mut rows: Vec<Vec<i16>> = Vec::new();
        let mut filled = [0usize; SCRIPTS];
        for raw in text.lines() {
            let line = raw.trim_end_matches('\r');
            if let Some(tag) = line.strip_prefix('#').filter(|t| t.len() == 1) {
                section = Some(tag.to_ascii_uppercase());
                continue;
            }
            let Some(sec) = section.as_deref() else { continue };
            let script = match sec {
                "M" => Some(METRO),
                "I" => Some(INIT),
                d if d.len() == 1 && (b'1'..=b'8').contains(&d.as_bytes()[0]) => Some((d.as_bytes()[0] - b'1') as usize),
                _ => None,
            };
            if let Some(si) = script {
                if line.trim().is_empty() {
                    continue;
                }
                let li = filled[si];
                if li >= LINES {
                    s.errors.push(format!("{}: more than 6 lines", script_name(si)));
                    continue;
                }
                filled[si] += 1;
                match parse_line(line) {
                    Ok(t) => {
                        if let Err(e) = validate(&t) {
                            s.errors.push(format!("{}.{}: {e}", script_name(si), li + 1));
                        }
                        s.scripts[si][li] = t;
                    }
                    Err(e) => {
                        s.errors.push(format!("{}.{}: {e}", script_name(si), li + 1));
                        // The line stays empty but keeps its place, so the
                        // rest of the script keeps its line numbers.
                    }
                }
            } else if sec == "P" {
                let nums: Vec<i16> = line.split_whitespace().filter_map(|w| w.parse::<i32>().ok()).map(|v| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16).collect();
                if nums.len() >= 4 {
                    rows.push(nums);
                }
            }
        }
        for (r, row) in rows.iter().enumerate() {
            for p in 0..4 {
                let v = row[p];
                let pat = &mut s.pats[p];
                match r {
                    0 => pat.len = v.clamp(0, PAT_LEN as i16),
                    1 => pat.wrap = v != 0,
                    2 => pat.start = v.clamp(0, 63),
                    3 => pat.end = v.clamp(0, 63),
                    _ if r - 4 < PAT_LEN => pat.v[r - 4] = v,
                    _ => {}
                }
            }
        }
        s
    }

    fn write(&self, description: &str) -> String {
        let mut out = format!("{description}\n\n");
        for si in (0..8).chain([METRO, INIT]) {
            out.push_str(&format!("#{}\n", script_name(si)));
            for l in self.scripts[si].iter().filter(|l| !l.is_empty()) {
                out.push_str(&line_text(l));
                out.push('\n');
            }
            out.push('\n');
        }
        out.push_str("#P\n");
        let row = |f: &dyn Fn(&Pattern) -> i16| self.pats.iter().map(|p| f(p).to_string()).collect::<Vec<_>>().join("\t") + "\n";
        out.push_str(&row(&|p| p.len));
        out.push_str(&row(&|p| p.wrap as i16));
        out.push_str(&row(&|p| p.start));
        out.push_str(&row(&|p| p.end));
        out.push('\n');
        for i in 0..PAT_LEN {
            out.push_str(&row(&|p| p.v[i]));
        }
        out
    }

    fn compiled(&self) -> Scene {
        let mut sc = Scene { pats: self.pats, ..Default::default() };
        for s in 0..SCRIPTS {
            for l in 0..LINES {
                sc.scripts[s][l] = Line::from(&self.scripts[s][l]);
            }
        }
        sc
    }
}

// ---------------------------------------------------------------- pads

/// What a pad does on an edit page.
#[derive(Clone, Copy, PartialEq)]
enum Pad {
    Word(&'static str),
    Digit(u8),
    /// Negates the number before the caret, or types SUB.
    Minus,
    Back,
    Fire(u8),
    PrevScript,
    NextScript,
    RunLine,
    ClearLine,
    /// Tracker: zero the cell, set the length to this row, insert, remove.
    Zero,
    LenHere,
    Ins,
    Rm,
    None,
}

use Pad::{Digit as Dg, Word as W_};

const P_TRACKER: usize = 11;

const PAGES: [(&str, [Pad; 16]); 12] = [
    ("SCRIPTS", [Pad::Fire(0), Pad::Fire(1), Pad::Fire(2), Pad::Fire(3), Pad::Fire(4), Pad::Fire(5), Pad::Fire(6), Pad::Fire(7), Pad::Fire(8), Pad::Fire(9), Pad::PrevScript, Pad::NextScript, Pad::RunLine, Pad::ClearLine, Pad::Back, Pad::None]),
    ("NUMBERS", [Dg(1), Dg(2), Dg(3), Pad::Minus, Dg(4), Dg(5), Dg(6), W_(":"), Dg(7), Dg(8), Dg(9), W_(";"), Pad::Back, Dg(0), W_("ADD"), W_("I")]),
    ("VARS", [W_("A"), W_("B"), W_("C"), W_("D"), W_("X"), W_("Y"), W_("Z"), W_("T"), W_("I"), W_("J"), W_("K"), W_("O"), W_("DRUNK"), W_("FLIP"), W_("R"), W_("TIME")]),
    ("I/O", [W_("CV"), W_("TR"), W_("TR.P"), W_("TR.TOG"), W_("CV.SLEW"), W_("CV.SET"), W_("TR.TIME"), W_("CV.OFF"), W_("IN"), W_("PARAM"), W_("M"), W_("M.ACT"), W_("N"), W_("V"), W_("VV"), W_("BPM")]),
    ("MATH", [W_("ADD"), W_("SUB"), W_("MUL"), W_("DIV"), W_("MOD"), W_("MIN"), W_("MAX"), W_("LIM"), W_("WRAP"), W_("QT"), W_("AVG"), W_("SCALE"), W_("RAND"), W_("RRAND"), W_("TOSS"), W_("ABS")]),
    ("LOGIC", [W_("EQ"), W_("NE"), W_("LT"), W_("GT"), W_("LTE"), W_("GTE"), W_("AND"), W_("OR"), W_("EZ"), W_("NZ"), W_("?"), W_("ER"), W_("INR"), W_("OUTR"), W_("LSH"), W_("RSH")]),
    ("FLOW", [W_("IF"), W_("ELIF"), W_("ELSE"), W_("L"), W_("W"), W_("EVERY"), W_("SKIP"), W_("OTHER"), W_("PROB"), W_("DEL"), W_("S"), W_("S.ALL"), W_("$"), W_("BREAK"), W_("KILL"), W_("SYNC")]),
    ("PATTERN", [W_("P"), W_("P.N"), W_("P.L"), W_("P.I"), W_("P.HERE"), W_("P.NEXT"), W_("P.PREV"), W_("P.START"), W_("P.END"), W_("P.WRAP"), W_("P.INS"), W_("P.RM"), W_("P.PUSH"), W_("P.POP"), W_("PN"), W_("P.RND")]),
    ("KRIA", [W_("KR.PAT"), W_("KR.POS"), W_("KR.L.ST"), W_("KR.L.LEN"), W_("KR.RES"), W_("KR.CV"), W_("KR.MUTE"), W_("KR.TMUTE"), W_("KR.CLK"), W_("KR.PG"), W_("KR.CUE"), W_("KR.DIR"), W_("KR.DUR"), W_("KR.PERIOD"), W_("KR.SCALE"), W_("KR.PRE")]),
    ("GRID", [W_("G.BTN"), W_("G.BTX"), W_("G.FDR"), W_("G.FDX"), W_("G.LED"), W_("G.REC"), W_("G.CLR"), W_("G.RST"), W_("G.BTNV"), W_("G.BTNI"), W_("G.FDRN"), W_("G.FDRV"), W_("G.FDRI"), W_("G.GRP"), W_("G.DIM"), W_("G.KEY")]),
    ("GRID+", [W_("G.BTN.V"), W_("G.BTN.L"), W_("G.BTN.EN"), W_("G.BTN.SW"), W_("G.GBTN.C"), W_("G.GBTN.I"), W_("G.FDR.N"), W_("G.FDR.V"), W_("G.FDR.L"), W_("G.GFDR.RN"), W_("G.GRP.EN"), W_("G.GRP.SC"), W_("G.GRPI"), W_("G.ROTATE"), W_("G.RCT"), W_("G.XYP")]),
    ("TRACKER", [Dg(1), Dg(2), Dg(3), Pad::Minus, Dg(4), Dg(5), Dg(6), Pad::Zero, Dg(7), Dg(8), Dg(9), Pad::LenHere, Pad::Back, Dg(0), Pad::Ins, Pad::Rm]),
];

fn pad_label(p: Pad) -> String {
    match p {
        Pad::Word(w) => w.to_string(),
        Pad::Digit(d) => d.to_string(),
        Pad::Minus => "-".into(),
        Pad::Back => "DEL<".into(),
        Pad::Fire(s) => format!("RUN {}", script_name(s as usize)),
        Pad::PrevScript => "EDIT<".into(),
        Pad::NextScript => "EDIT>".into(),
        Pad::RunLine => "DO LN".into(),
        Pad::ClearLine => "CLR LN".into(),
        Pad::Zero => "ZERO".into(),
        Pad::LenHere => "LEN".into(),
        Pad::Ins => "INS".into(),
        Pad::Rm => "RM".into(),
        Pad::None => String::new(),
    }
}

fn kit_config() -> KitConfig {
    let mut layers: Vec<Layer> = PAGES.iter().enumerate().map(|(i, (name, _))| Layer::Native(i as u8, name)).collect();
    layers.push(Layer::Controls);
    layers.push(Layer::Moments);
    KitConfig {
        app_id: "teletype",
        layers,
        // The D-pad is the caret, so there are no dials to turn with it.
        hero: Vec::new(),
        browse: None,
        routes: Routes { stick_x: None, stick_y: None, hand_l: None, hand_r: None },
        throws: Vec::new(),
        midi_to_pads: false,
        own_expression: true,
    }
}

// ---------------------------------------------------------------- the app

const C_SCENE: usize = 1;
const C_METRO: usize = 2;
const C_METRO_ON: usize = 3;
const C_PARAM: usize = 4;
const C_ROUTE: usize = 0;
const C_WAVE: usize = 5;
const C_RELEASE: usize = 6;
const C_LEVEL: usize = 7;
const C_SAVE: usize = 8;
/// Then two rows (app, input) for each of CV 1-4 and TR 1-4.
const C_OUTS: usize = 9;
const N_OUTS: usize = 8;
const N_CONTROLS: usize = C_OUTS + 2 * N_OUTS;

const WAVES: [&str; 4] = ["Sine", "Triangle", "Saw", "Square"];

enum Edit {
    Line(usize, usize, Line),
    Fire(usize),
    Run(Line),
    Cell(usize, usize, i16),
    PatLen(usize, i16),
    PatIns(usize, usize),
    PatRm(usize, usize),
    Load(Box<Scene>),
    Metro(i16),
    MetroAct(bool),
    /// A key on the grid: x, y, down.
    GridKey(usize, usize, bool),
}

/// What the screen shows, published by the audio thread after a change.
#[derive(Clone)]
struct View {
    vars: [i16; 8],
    m: i16,
    m_act: bool,
    time: i16,
    p_n: i16,
    pats: [Pattern; 4],
    cv: [i16; 4],
    tr: [bool; 4],
    fired: [u32; SCRIPTS],
    delays: usize,
    stack: usize,
}

impl Default for View {
    fn default() -> Self {
        View { vars: Vars::default().v, m: 1000, m_act: true, time: 0, p_n: 0, pats: [Pattern::default(); 4], cv: [0; 4], tr: [false; 4], fired: [0; SCRIPTS], delays: 0, stack: 0 }
    }
}

struct Shared {
    edits: Mutex<VecDeque<Edit>>,
    view: Mutex<View>,
    wave: AtomicUsize,
    release: AtomicF32,
    level: AtomicF32,
    param: AtomicF32,
    /// Hands: left adds to IN, right to PARAM.
    hands: [AtomicF32; 2],
    trig_in: [Arc<AtomicF32>; 8],
    in_cv: Arc<AtomicF32>,
    param_cv: Arc<AtomicF32>,
    /// CV 1-4 then TR 1-4: 0 = not routed, else modbus target + 1.
    outs: [AtomicUsize; N_OUTS],
    mix_level: Arc<AtomicF32>,
    ext_mix_level: Arc<AtomicF32>,
    output: Arc<Mutex<Vec<f32>>>,
    notes_played: AtomicU32,
    running: AtomicBool,
    /// The grid's size (columns, rows), from the UI thread.
    grid_size: [AtomicUsize; 2],
    /// The grid ops' picture (16 x 16 levels) and a count of redraws.
    grid_out: Mutex<([u8; tg::DIM * tg::DIM], u64)>,
}

pub struct TeletypeApp {
    p: Arc<Shared>,
    mods: Arc<ModBus>,
    list: ParamList,
    nav: Arc<AtomicF32>,
    sensitivity: Arc<AtomicF32>,
    taken: bool,
    kit: PlayKit,
    scenes: Vec<(String, Option<PathBuf>)>,
    scene: usize,
    dir: PathBuf,
    text: SceneText,
    /// The script being edited, the line, and the caret (a word index).
    script: usize,
    line: usize,
    caret: usize,
    /// True while digits extend the number before the caret.
    typing: bool,
    /// Tracker cursor: pattern, row.
    cell: (usize, usize),
    cell_typing: bool,
    pads_down: [bool; 16],
    route: NoteRoute,
    note_out: Option<NoteOut>,
    status: String,
    grid: Arc<Grid>,
    /// The last picture shown on the grid, and which redraw it was.
    grid_leds: Leds,
    grid_gen: u64,
}

impl TeletypeApp {
    pub fn new(sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        Self::with_dir(Path::new(TT_DIR), sensitivity, nav, mods, bus, mixer)
    }

    pub fn with_dir(dir: &Path, sensitivity: Arc<AtomicF32>, nav: Arc<AtomicF32>, mods: Arc<ModBus>, bus: Arc<AudioBus>, mixer: Arc<MixerBus>) -> Self {
        let output = bus.register(APP_NAME);
        let (mix_level, ext_mix_level) = mixer.register(APP_NAME, &mods);
        let (route, out) = NoteRoute::new(None, APP_NAME, "teletype", true);
        let mut app = Self {
            p: Arc::new(Shared {
                edits: Mutex::new(VecDeque::new()),
                view: Mutex::new(View::default()),
                wave: AtomicUsize::new(1),
                release: AtomicF32::new(0.3),
                level: AtomicF32::new(0.7),
                param: AtomicF32::new(0.0),
                hands: [AtomicF32::new(0.0), AtomicF32::new(0.0)],
                trig_in: std::array::from_fn(|i| mods.register(format!("{APP_NAME}: Trig {}", i + 1))),
                in_cv: mods.register(format!("{APP_NAME}: IN")),
                param_cv: mods.register(format!("{APP_NAME}: PARAM")),
                outs: std::array::from_fn(|_| AtomicUsize::new(0)),
                mix_level,
                ext_mix_level,
                output,
                notes_played: AtomicU32::new(0),
                running: AtomicBool::new(false),
                grid_size: [AtomicUsize::new(grid_kit::DEFAULT_COLS), AtomicUsize::new(grid_kit::DEFAULT_ROWS)],
                grid_out: Mutex::new(([0; tg::DIM * tg::DIM], 0)),
            }),
            mods,
            list: ParamList::new(),
            nav,
            sensitivity,
            taken: false,
            kit: PlayKit::new(kit_config(), !cfg!(test)),
            scenes: Vec::new(),
            scene: 0,
            dir: dir.to_path_buf(),
            text: SceneText::default(),
            script: 0,
            line: 0,
            caret: 0,
            typing: false,
            cell: (0, 0),
            cell_typing: false,
            pads_down: [false; 16],
            route,
            note_out: Some(out),
            status: String::new(),
            grid: grid_kit::grid(),
            grid_leds: Leds::new(tg::DIM, tg::DIM),
            grid_gen: 0,
        };
        app.grid.register(APP_NAME);
        app.rescan();
        app.load_scene(1.min(app.scenes.len() - 1));
        app
    }

    /// Lets Teletype play other apps (see note_bus.rs).
    pub fn with_notes(mut self, bus: Option<Arc<NoteBus>>) -> Self {
        let (route, out) = NoteRoute::new(bus, APP_NAME, "teletype", true);
        self.route = route;
        self.note_out = Some(out);
        self
    }

    /// "(empty)", the bundled scenes, then the saved ones.
    fn rescan(&mut self) {
        let mut found = Vec::new();
        for sub in ["scenes", "saved"] {
            let mut here: Vec<(String, PathBuf)> = std::fs::read_dir(self.dir.join(sub))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("txt")))
                .map(|p| {
                    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    (if sub == "saved" { format!("saved/{stem}") } else { stem }, p)
                })
                .collect();
            here.sort_by_key(|(n, _)| n.to_lowercase());
            found.extend(here);
        }
        self.scenes = std::iter::once(("(empty)".to_string(), None)).chain(found.into_iter().map(|(n, p)| (n, Some(p)))).collect();
    }

    fn load_scene(&mut self, index: usize) {
        let Some((name, path)) = self.scenes.get(index).cloned() else { return };
        let text = match path {
            None => SceneText::default(),
            Some(p) => match std::fs::read_to_string(&p) {
                Ok(t) => SceneText::parse(&t),
                Err(e) => {
                    self.status = format!("Could not read {name}: {e}");
                    return;
                }
            },
        };
        self.status = match text.errors.first() {
            Some(e) if text.errors.len() > 1 => format!("{e} (+{} more)", text.errors.len() - 1),
            Some(e) => e.clone(),
            None => String::new(),
        };
        self.send(Edit::Load(Box::new(text.compiled())));
        // A scene that plays the grid takes it, as plugging a grid into the
        // module would.
        if text.scripts.iter().flatten().any(|l| uses_grid(l)) {
            self.grid.set_focus(APP_NAME);
        }
        self.text = text;
        self.scene = index;
        self.script = 0;
        self.line = 0;
        self.caret = 0;
        self.typing = false;
    }

    fn step_scene(&mut self, d: i32) {
        let n = self.scenes.len() as i32;
        if n > 0 && d != 0 {
            self.load_scene((self.scene as i32 + d.signum()).rem_euclid(n) as usize);
        }
    }

    /// Writes the scene, as the module would to its USB stick, to `saved/NN.txt`.
    fn save(&mut self) {
        let mut text = self.text.clone();
        if let Ok(v) = self.p.view.lock() {
            // The patterns as the scripts have left them, like the module saves.
            text.pats = v.pats;
        }
        let dir = self.dir.join("saved");
        if std::fs::create_dir_all(&dir).is_err() {
            self.status = "Could not create the saved folder".into();
            return;
        }
        let n = (1..1000).find(|n| !dir.join(format!("{n:02}.txt")).exists()).unwrap_or(999);
        let path = dir.join(format!("{n:02}.txt"));
        match std::fs::write(&path, text.write("Portamax Teletype scene")) {
            Ok(()) => {
                self.status = format!("Saved saved/{n:02}");
                self.rescan();
                if let Some(i) = self.scenes.iter().position(|(_, p)| p.as_ref() == Some(&path)) {
                    self.scene = i;
                }
            }
            Err(e) => self.status = format!("Could not save: {e}"),
        }
    }

    fn send(&self, e: Edit) {
        if let Ok(mut q) = self.p.edits.lock() {
            q.push_back(e);
        }
    }

    fn cur(&self) -> &Vec<Tok> {
        &self.text.scripts[self.script][self.line]
    }

    /// The edited line has changed: hand it to the interpreter.
    fn commit(&mut self) {
        let l = Line::from(self.cur());
        self.send(Edit::Line(self.script, self.line, l));
    }

    fn insert(&mut self, t: Tok) {
        if self.cur().len() >= WORDS {
            self.status = "Line full (16 words)".into();
            return;
        }
        let c = self.caret.min(self.cur().len());
        self.text.scripts[self.script][self.line].insert(c, t);
        self.caret = c + 1;
        self.commit();
    }

    fn backspace(&mut self) {
        let c = self.caret.min(self.cur().len());
        if c > 0 {
            self.text.scripts[self.script][self.line].remove(c - 1);
            self.caret = c - 1;
            self.commit();
        }
        self.typing = false;
    }

    fn digit(&mut self, d: u8) {
        let c = self.caret.min(self.cur().len());
        if self.typing && c > 0 {
            if let Tok::Num(v) = self.cur()[c - 1] {
                let neg = v < 0;
                let n = (v as i32).abs() * 10 + d as i32;
                let n = if neg { -n } else { n };
                self.text.scripts[self.script][self.line][c - 1] = Tok::Num(n.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
                self.commit();
                return;
            }
        }
        self.insert(Tok::Num(d as i16));
        self.typing = true;
    }

    fn minus(&mut self) {
        let c = self.caret.min(self.cur().len());
        if c > 0 {
            if let Tok::Num(v) = self.cur()[c - 1] {
                self.text.scripts[self.script][self.line][c - 1] = Tok::Num(v.wrapping_neg());
                self.commit();
                return;
            }
        }
        self.insert(Tok::Op(Op::Sub));
    }

    fn line_error(&self, s: usize, l: usize) -> Option<String> {
        let t = &self.text.scripts[s][l];
        validate(t).err()
    }

    fn pad(&mut self, pad: Pad, tracker: bool) {
        if tracker {
            self.tracker_pad(pad);
            return;
        }
        let was_typing = self.typing;
        self.typing = false;
        match pad {
            Pad::Word(w) => {
                if let Some(t) = parse_word(w) {
                    self.insert(t);
                }
            }
            Pad::Digit(d) => {
                self.typing = was_typing;
                self.digit(d);
            }
            Pad::Minus => self.minus(),
            Pad::Back => self.backspace(),
            Pad::Fire(s) => self.send(Edit::Fire(s as usize)),
            Pad::PrevScript | Pad::NextScript => {
                let d = if pad == Pad::NextScript { 1 } else { SCRIPTS - 1 };
                self.script = (self.script + d) % SCRIPTS;
                self.line = 0;
                self.caret = self.cur().len();
            }
            Pad::RunLine => {
                let l = Line::from(self.cur());
                match self.line_error(self.script, self.line) {
                    Some(e) => self.status = format!("Not run: {e}"),
                    None => self.send(Edit::Run(l)),
                }
            }
            Pad::ClearLine => {
                self.text.scripts[self.script][self.line].clear();
                self.caret = 0;
                self.commit();
            }
            _ => {}
        }
    }

    fn tracker_pad(&mut self, pad: Pad) {
        let (p, r) = self.cell;
        let cur = self.p.view.lock().map_or(0, |v| v.pats[p].v[r]);
        let typing = self.cell_typing;
        self.cell_typing = false;
        match pad {
            Pad::Digit(d) => {
                let v = if typing { (cur as i32).abs() * 10 + d as i32 } else { d as i32 };
                let v = if typing && cur < 0 { -v } else { v };
                self.send(Edit::Cell(p, r, v.clamp(i16::MIN as i32, i16::MAX as i32) as i16));
                self.cell_typing = true;
            }
            Pad::Minus => {
                self.send(Edit::Cell(p, r, cur.wrapping_neg()));
                self.cell_typing = typing;
            }
            Pad::Back => self.send(Edit::Cell(p, r, cur / 10)),
            Pad::Zero => self.send(Edit::Cell(p, r, 0)),
            Pad::LenHere => self.send(Edit::PatLen(p, r as i16 + 1)),
            Pad::Ins => self.send(Edit::PatIns(p, r)),
            Pad::Rm => self.send(Edit::PatRm(p, r)),
            _ => {}
        }
    }

    fn view(&self) -> View {
        self.p.view.lock().map(|v| v.clone()).unwrap_or_default()
    }

    fn text(&self, i: usize) -> (String, String) {
        match i {
            C_SCENE => ("Scene".into(), format!("{} {}/{}", self.scenes.get(self.scene).map_or("?", |s| s.0.as_str()), self.scene + 1, self.scenes.len())),
            C_METRO => ("Metro (M)".into(), format!("{} ms", self.view().m)),
            C_METRO_ON => ("Metro on (M.ACT)".into(), if self.view().m_act { "on" } else { "off" }.into()),
            C_PARAM => ("Param knob".into(), format!("{}", (self.p.param.get() * CV_MAX as f32) as i32)),
            C_ROUTE => ("Plays".into(), self.route.label()),
            C_WAVE => ("Voice".into(), WAVES[self.p.wave.load(Ordering::Relaxed).min(3)].into()),
            C_RELEASE => ("Release".into(), format!("{:.0} ms", release_ms(self.p.release.get()))),
            C_LEVEL => ("Level".into(), format!("{:.0}%", self.p.level.get() * 100.0)),
            C_SAVE => ("Save".into(), "press to save as a new scene".into()),
            _ => {
                let k = (i - C_OUTS) / 2;
                let name = if k < 4 { format!("CV {}", k + 1) } else { format!("TR {}", k - 3) };
                let r = self.p.outs[k].load(Ordering::Relaxed);
                if (i - C_OUTS) % 2 == 0 {
                    (format!("{name} to"), Patch::app_label(&self.mods, r))
                } else {
                    (format!("{name} input"), Patch::input_label(&self.mods, r))
                }
            }
        }
    }

    fn rows(&self) -> Vec<(String, String, bool)> {
        let mut r = Vec::new();
        for i in 0..N_CONTROLS {
            let (n, v) = self.text(i);
            r.push((n, v, false));
            if i == 0 {
                // The instrument it plays, dialled in right under Plays.
                r.extend(self.route.settings().into_iter().map(|s| (format!("  {}", s.label), s.value, false)));
            }
        }
        r
    }

    fn edit_control(&mut self, i: usize, d: i32) {
        if d == 0 {
            return;
        }
        let step = d as f32 * 0.01 * self.sensitivity.get().max(0.01) * 10.0;
        match i {
            C_SCENE => self.step_scene(d),
            C_METRO => {
                let m = self.view().m as i32;
                let by = if m >= 1000 { 50 } else if m >= 200 { 10 } else { 1 };
                self.send(Edit::Metro((m + d * by).clamp(METRO_MIN as i32, 32767) as i16));
            }
            C_METRO_ON => self.send(Edit::MetroAct(d > 0)),
            C_PARAM => self.p.param.set((self.p.param.get() + step).clamp(0.0, 1.0)),
            C_ROUTE => self.route.step(d),
            C_WAVE => self.p.wave.store((self.p.wave.load(Ordering::Relaxed) as i32 + d.signum()).rem_euclid(4) as usize, Ordering::Relaxed),
            C_RELEASE => self.p.release.set((self.p.release.get() + step).clamp(0.0, 1.0)),
            C_LEVEL => self.p.level.set((self.p.level.get() + step).clamp(0.0, 1.0)),
            C_SAVE => self.save(),
            _ => {
                let k = (i - C_OUTS) / 2;
                let r = self.p.outs[k].load(Ordering::Relaxed);
                let next = if (i - C_OUTS) % 2 == 0 { Patch::step_app(&self.mods, r, d) } else { Patch::step_input(&self.mods, r, d) };
                self.p.outs[k].store(next, Ordering::Relaxed);
            }
        }
    }
}

/// The built-in voice's release, 10 ms to 3 s, exponentially.
fn release_ms(p: f32) -> f32 {
    10.0 * 300f32.powf(p.clamp(0.0, 1.0))
}

impl PlayHost for TeletypeApp {
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
            C_SCENE => if self.scenes.len() > 1 { self.scene as f32 / (self.scenes.len() - 1) as f32 } else { 0.0 },
            C_METRO => (self.view().m as f32 - 25.0) / 1975.0,
            C_PARAM => self.p.param.get(),
            C_WAVE => self.p.wave.load(Ordering::Relaxed) as f32 / 3.0,
            C_RELEASE => self.p.release.get(),
            C_LEVEL => self.p.level.get(),
            _ => return None,
        })
    }
    fn kit_stepped(&self, i: usize) -> bool {
        !matches!(i, C_PARAM | C_RELEASE | C_LEVEL)
    }
    fn kit_pads_play(&self, _layer: u8) -> bool {
        false
    }
    fn kit_edit(&mut self, i: usize, delta: i32) {
        self.edit_control(i, delta);
    }
    fn kit_reset(&mut self, i: usize) {
        match i {
            C_METRO => self.send(Edit::Metro(1000)),
            C_PARAM => self.p.param.set(0.0),
            C_ROUTE => self.route.reset(),
            C_WAVE => self.p.wave.store(1, Ordering::Relaxed),
            C_RELEASE => self.p.release.set(0.3),
            C_LEVEL => self.p.level.set(0.7),
            i if i >= C_OUTS => {
                let k = (i - C_OUTS) / 2;
                self.p.outs[k].store(0, Ordering::Relaxed);
            }
            _ => {}
        }
    }
    fn kit_set_norm(&mut self, i: usize, v: f32) {
        let v = v.clamp(0.0, 1.0);
        match i {
            C_SCENE => {
                if self.scenes.len() > 1 {
                    let t = (v * (self.scenes.len() - 1) as f32).round() as usize;
                    if t != self.scene {
                        self.load_scene(t);
                    }
                }
            }
            C_METRO => self.send(Edit::Metro((25.0 + v * 1975.0) as i16)),
            C_PARAM => self.p.param.set(v),
            C_WAVE => self.p.wave.store((v * 3.0).round() as usize, Ordering::Relaxed),
            C_RELEASE => self.p.release.set(v),
            C_LEVEL => self.p.level.set(v),
            _ => {}
        }
    }
    fn kit_line(&self) -> String {
        if self.status.is_empty() {
            format!("script {}", script_name(self.script))
        } else {
            self.status.clone()
        }
    }
}

impl App for TeletypeApp {
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
        Some(self.p.running.load(Ordering::Relaxed))
    }
    /// F3 starts and stops the metro (with M.ACT, which scripts control).
    fn toggle_running(&mut self) {
        self.p.running.fetch_xor(true, Ordering::Relaxed);
    }
    fn needs_background_audio(&self) -> bool {
        // The metro keeps the scene playing when another app is on screen,
        // and a mod input can fire a script at any time.
        self.p.running.load(Ordering::Relaxed) || self.mods.requested(APP_NAME) || self.grid.focus().as_deref() == Some(APP_NAME)
    }
    fn background_tick(&mut self) {
        self.grid_frame();
    }
    fn tick(&mut self, input: &Input) {
        self.p.hands[0].set(input.hands[0]);
        self.p.hands[1].set(input.hands[1]);
        let mut play = std::mem::take(&mut self.kit);
        let step = play.tick(self, input);
        self.kit = play;
        if step.menu {
            let i = &step.input;
            let n = self.route.settings().len();
            self.list.navigate_input(i, N_CONTROLS + n, self.nav.get() as i32);
            let sel = self.list.selected.min(N_CONTROLS + n - 1);
            match crate::app::play_kit::menu_row(sel, n) {
                crate::app::play_kit::MenuRow::Setting(j) => {
                    if i.knob2 != 0 {
                        self.route.adjust(j, i.knob2.signum());
                    }
                }
                crate::app::play_kit::MenuRow::Control(c) => {
                    self.kit_edit(c, i.knob2);
                    if i.knob2_press {
                        self.kit_reset(c);
                    }
                }
            }
            self.pads_down = [false; 16];
            return;
        }
        let Some(page) = step.native else {
            self.pads_down = [false; 16];
            return;
        };
        let page = (page as usize).min(PAGES.len() - 1);
        let tracker = page == P_TRACKER;
        // The caret: the raw D-pad, since the kit would turn a dial with it.
        let (dx, dy) = (input.nav_x, input.navigation_steps);
        if tracker {
            if dx != 0 || dy != 0 {
                self.cell_typing = false;
                self.cell = ((self.cell.0 as i32 + dx).clamp(0, 3) as usize, (self.cell.1 as i32 + dy).clamp(0, PAT_LEN as i32 - 1) as usize);
            }
        } else {
            if dy != 0 {
                self.line = (self.line as i32 + dy).clamp(0, LINES as i32 - 1) as usize;
                self.caret = self.cur().len();
                self.typing = false;
            }
            if dx != 0 {
                self.caret = (self.caret as i32 + dx).clamp(0, self.cur().len() as i32) as usize;
                self.typing = false;
            }
        }
        for pad in 0..16 {
            if step.input.grid[pad] && !self.pads_down[pad] {
                self.pad(PAGES[page].1[pad], tracker);
            }
            self.pads_down[pad] = step.input.grid[pad];
        }
        if input.knob1_press {
            if tracker {
                self.tracker_pad(Pad::Zero);
            } else {
                self.backspace();
            }
        }
    }
    fn draw(&mut self, f: &mut FrameBuffer) {
        f.clear(BG).ok();
        if self.kit.menu {
            Text::new(APP_NAME, Point::new(16, 30), MonoTextStyle::new(&SPLEEN_16X32, BRIGHT)).draw(f).ok();
            let r: Vec<(String, String)> = self.rows().into_iter().map(|(a, b, _)| (a, b)).collect();
            self.list.draw_themed(f, 16, 44, 24, 11, &r, BG, DIM, ACCENT);
            if !self.status.is_empty() {
                Text::new(&self.status, Point::new(16, 330), MonoTextStyle::new(&SPLEEN_6X12, ACCENT)).draw(f).ok();
            }
            return;
        }
        let view = self.view();
        let page = match self.kit.layer() {
            Layer::Native(id, _) => Some(id as usize),
            _ => None,
        };
        match page {
            None => {
                let col = self.kit.column(self);
                let pal = kit::draw::Palette { bg: BG, ink: INK, accent: ACCENT, dim: DIM, faint: FAINT };
                kit::draw::column(f, &col, 16, 40, 350, 280, pal);
            }
            Some(P_TRACKER) => self.draw_tracker(f, &view),
            Some(_) => self.draw_editor(f, &view),
        }
        self.draw_side(f, &view, page);
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
            mods: Arc::clone(&self.mods),
            eng: Engine::new(),
            notes: self.note_out.take().unwrap_or_else(NoteOut::detached),
            voices: [Voice::default(); 4],
            sounding: [None; 4],
            prev_tr: [false; 4],
            trig_prev: [false; 8],
            ms_acc: 0.0,
        }))
    }
}

impl TeletypeApp {
    /// Passes the grid's presses to the interpreter and shows its picture.
    /// Runs every frame, on screen or not, so a scene plays from the Grid
    /// app's screen or a real grid while anything else is showing.
    fn grid_frame(&mut self) {
        let (rows, cols) = self.grid.size();
        self.p.grid_size[0].store(cols, Ordering::Relaxed);
        self.p.grid_size[1].store(rows, Ordering::Relaxed);
        if self.grid.focus().as_deref() != Some(APP_NAME) {
            return;
        }
        for k in self.grid.keys(APP_NAME) {
            if k.x < tg::DIM && k.y < tg::DIM {
                self.send(Edit::GridKey(k.x, k.y, k.down));
            }
        }
        if let Ok(g) = self.p.grid_out.lock() {
            if g.1 != self.grid_gen {
                self.grid_gen = g.1;
                for y in 0..tg::DIM {
                    for x in 0..tg::DIM {
                        self.grid_leds.set(x, y, g.0[y * tg::DIM + x] as i32);
                    }
                }
            }
        }
        self.grid.show(APP_NAME, &self.grid_leds);
    }

    fn draw_editor(&self, f: &mut FrameBuffer, view: &View) {
        let big = MonoTextStyle::new(&SPLEEN_8X16, BRIGHT);
        let dim = MonoTextStyle::new(&SPLEEN_8X16, DIM);
        let small = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        let title = match self.script {
            METRO => format!("METRO  {} ms {}", view.m, if view.m_act { "on" } else { "off" }),
            INIT => "INIT".to_string(),
            s => format!("SCRIPT {}", s + 1),
        };
        Text::new(&title, Point::new(8, 34), big).draw(f).ok();
        let (x0, width) = (28, 384);
        for l in 0..LINES {
            let y = 48 + l as i32 * 24;
            let toks = &self.text.scripts[self.script][l];
            let bad = validate(toks).is_err();
            if l == self.line {
                Rectangle::new(Point::new(4, y), Size::new(412, 22)).into_styled(PrimitiveStyle::with_fill(FAINT)).draw(f).ok();
            }
            Text::new(&format!("{}", l + 1), Point::new(10, y + 16), if bad { MonoTextStyle::new(&SPLEEN_8X16, ERR) } else { dim }).draw(f).ok();
            // Word positions, so the caret can sit between two of them.
            let mut xs = Vec::with_capacity(toks.len() + 1);
            let mut x = 0;
            for (i, t) in toks.iter().enumerate() {
                if i > 0 && !matches!(t, Tok::Pre | Tok::Sub) {
                    x += 8;
                }
                xs.push(x);
                x += tok_text(*t).len() as i32 * 8;
            }
            xs.push(x + 2);
            let caret_x = if l == self.line { xs[self.caret.min(toks.len())] } else { 0 };
            let scroll = (caret_x - width + 16).max(0);
            for (i, t) in toks.iter().enumerate() {
                let tx = x0 + xs[i] - scroll;
                if tx < x0 - 4 || tx > x0 + width {
                    continue;
                }
                let colour = if bad { ERR } else if matches!(t, Tok::Mod(_) | Tok::Pre | Tok::Sub) { ACCENT } else if matches!(t, Tok::Num(_)) { INK } else { BRIGHT };
                Text::new(&tok_text(*t), Point::new(tx, y + 16), MonoTextStyle::new(&SPLEEN_8X16, colour)).draw(f).ok();
            }
            if l == self.line {
                let cx = x0 + caret_x - scroll - 4;
                Rectangle::new(Point::new(cx, y + 3), Size::new(2, 16)).into_styled(PrimitiveStyle::with_fill(ACCENT)).draw(f).ok();
            }
        }
        let msg = match self.line_error(self.script, self.line) {
            Some(e) => (format!("line {}: {e}", self.line + 1), ERR),
            None => (self.status.clone(), DIM),
        };
        Text::new(&msg.0.chars().take(68).collect::<String>(), Point::new(8, 206), MonoTextStyle::new(&SPLEEN_6X12, msg.1)).draw(f).ok();
        // A light per script that flips each time the script runs.
        for s in 0..SCRIPTS {
            let x = 8 + s as i32 * 40;
            let lit = view.fired[s] % 2 == 1;
            let colour = if s == self.script { ACCENT } else { DIM };
            Rectangle::new(Point::new(x, 220), Size::new(34, 22)).into_styled(PrimitiveStyle::with_stroke(colour, 1)).draw(f).ok();
            if lit {
                Rectangle::new(Point::new(x + 2, 222), Size::new(30, 18)).into_styled(PrimitiveStyle::with_fill(FAINT)).draw(f).ok();
            }
            Text::new(&script_name(s), Point::new(x + 13, 236), MonoTextStyle::new(&SPLEEN_6X12, if s == self.script { BRIGHT } else { INK })).draw(f).ok();
        }
        Text::new(&format!("delays {}  stack {}", view.delays, view.stack), Point::new(8, 262), small).draw(f).ok();
        Text::new("D-pad: caret and line   pads: type a word   SELECT: delete", Point::new(8, 300), small).draw(f).ok();
        Text::new("F2: pad page   R1: menu   F3: metro on/off", Point::new(8, 314), small).draw(f).ok();
        Text::new("SCRIPTS page: pads 1-8 run scripts, EDIT< > picks one", Point::new(8, 328), small).draw(f).ok();
    }

    fn draw_tracker(&self, f: &mut FrameBuffer, view: &View) {
        let big = MonoTextStyle::new(&SPLEEN_8X16, BRIGHT);
        Text::new(&format!("PATTERNS   P.N {}", view.p_n), Point::new(8, 34), big).draw(f).ok();
        let first = (self.cell.1 as i32 - 7).clamp(0, PAT_LEN as i32 - 16) as usize;
        for c in 0..4 {
            let x = 48 + c as i32 * 92;
            let pat = &view.pats[c];
            Text::new(&format!("{c}  L{}", pat.len), Point::new(x, 52), MonoTextStyle::new(&SPLEEN_6X12, if c as i16 == view.p_n { ACCENT } else { DIM })).draw(f).ok();
            for k in 0..16 {
                let r = first + k;
                let y = 58 + k as i32 * 16;
                if c == 0 {
                    Text::new(&format!("{r:2}"), Point::new(8, y + 12), MonoTextStyle::new(&SPLEEN_8X16, FAINT)).draw(f).ok();
                }
                if (c, r) == self.cell {
                    Rectangle::new(Point::new(x - 4, y), Size::new(84, 16)).into_styled(PrimitiveStyle::with_fill(FAINT)).draw(f).ok();
                }
                let inside = (r as i16) < pat.len;
                let colour = if !inside { DIM } else if r as i16 == pat.idx { BRIGHT } else { INK };
                let mark = if inside && r as i16 == pat.idx { ">" } else { " " };
                let edge = if r as i16 == pat.start { "[" } else if r as i16 == pat.end { "]" } else { " " };
                Text::new(&format!("{mark}{:>6}{edge}", pat.v[r]), Point::new(x, y + 12), MonoTextStyle::new(&SPLEEN_8X16, colour)).draw(f).ok();
            }
        }
        Text::new("D-pad: cell   digits type   LEN: length to here   SELECT: zero", Point::new(8, 328), MonoTextStyle::new(&SPLEEN_6X12, DIM)).draw(f).ok();
    }

    fn draw_side(&self, f: &mut FrameBuffer, view: &View, page: Option<usize>) {
        let x = 430;
        let ink = MonoTextStyle::new(&SPLEEN_6X12, INK);
        let dim = MonoTextStyle::new(&SPLEEN_6X12, DIM);
        let acc = MonoTextStyle::new(&SPLEEN_6X12, ACCENT);
        Text::new("TELETYPE", Point::new(x, 16), MonoTextStyle::new(&SPLEEN_6X12, BRIGHT)).draw(f).ok();
        let name = self.scenes.get(self.scene).map_or("", |s| s.0.as_str());
        Text::new(&name.chars().take(22).collect::<String>(), Point::new(x + 56, 16), acc).draw(f).ok();
        for (i, n) in ["A", "B", "C", "D", "X", "Y", "Z", "T"].iter().enumerate() {
            let (cx, cy) = (x + (i as i32 % 4) * 50, 34 + (i as i32 / 4) * 14);
            Text::new(&format!("{n} {}", view.vars[i]), Point::new(cx, cy), ink).draw(f).ok();
        }
        for k in 0..4 {
            let y = 70 + k as i32 * 14;
            Text::new(&format!("CV{}", k + 1), Point::new(x, y), dim).draw(f).ok();
            let w = (view.cv[k] as i32 * 110 / CV_MAX as i32).clamp(0, 110) as u32;
            Rectangle::new(Point::new(x + 26, y - 8), Size::new(110, 8)).into_styled(PrimitiveStyle::with_stroke(FAINT, 1)).draw(f).ok();
            Rectangle::new(Point::new(x + 26, y - 8), Size::new(w, 8)).into_styled(PrimitiveStyle::with_fill(INK)).draw(f).ok();
            Text::new(&format!("{}", view.cv[k]), Point::new(x + 142, y), ink).draw(f).ok();
        }
        Text::new("TR", Point::new(x, 136), dim).draw(f).ok();
        for k in 0..4 {
            let cx = x + 26 + k as i32 * 28;
            let style = if view.tr[k] { PrimitiveStyle::with_fill(BRIGHT) } else { PrimitiveStyle::with_stroke(DIM, 1) };
            Rectangle::new(Point::new(cx, 127), Size::new(18, 11)).into_styled(style).draw(f).ok();
        }
        let metro = if !self.p.running.load(Ordering::Relaxed) { "stopped (F3)" } else if view.m_act { "running" } else { "M.ACT 0" };
        Text::new(&format!("M {}ms {metro}  P.N {}", view.m, view.p_n), Point::new(x, 154), dim).draw(f).ok();
        Text::new(&format!("TR plays: {}", self.route.label().chars().take(20).collect::<String>()), Point::new(x, 168), dim).draw(f).ok();
        let Some(page) = page else { return };
        let (title, pads) = PAGES[page.min(PAGES.len() - 1)];
        Text::new(&format!("pads: {title}"), Point::new(x, 188), MonoTextStyle::new(&SPLEEN_6X12, BRIGHT)).draw(f).ok();
        for (i, p) in pads.iter().enumerate() {
            let (cx, cy) = (x + (i as i32 % 4) * 50, 196 + (i as i32 / 4) * 34);
            Rectangle::new(Point::new(cx, cy), Size::new(46, 30)).into_styled(PrimitiveStyle::with_stroke(DIM, 1)).draw(f).ok();
            let label = pad_label(*p);
            let style = if matches!(p, Pad::Word(_)) { ink } else { acc };
            let tx = cx + 23 - (label.len() as i32 * 3).min(22);
            Text::new(&label.chars().take(7).collect::<String>(), Point::new(tx, cy + 19), style).draw(f).ok();
        }
    }
}

const VOICES: usize = 4;

/// The built-in voice: one per TR/CV pair, so a scene can be heard with
/// nothing routed.
#[derive(Clone, Copy, Default)]
struct Voice {
    freq: f32,
    phase: f32,
    amp: f32,
    gate: bool,
    active: bool,
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
        } else {
            self.amp *= release;
            if self.amp < 1e-4 {
                self.active = false;
            }
        }
        s * self.amp
    }
}

struct Processor {
    p: Arc<Shared>,
    mods: Arc<ModBus>,
    eng: Engine,
    notes: NoteOut,
    voices: [Voice; VOICES],
    sounding: [Option<u8>; VOICES],
    prev_tr: [bool; 4],
    trig_prev: [bool; 8],
    /// Samples since the last millisecond tick.
    ms_acc: f64,
}

impl Processor {
    fn apply_edits(&mut self) {
        let shared = Arc::clone(&self.p);
        let Ok(mut q) = shared.edits.try_lock() else { return };
        while let Some(e) = q.pop_front() {
            match e {
                Edit::Line(s, l, line) => {
                    if s < SCRIPTS && l < LINES {
                        self.eng.scripts[s][l] = line;
                    }
                }
                Edit::Fire(s) => self.eng.run_script(s, None),
                Edit::Run(line) => self.eng.run_live(&line),
                Edit::Cell(p, r, v) => self.eng.pats[p.min(3)].v[r.min(PAT_LEN - 1)] = v,
                Edit::PatLen(p, n) => self.eng.pats[p.min(3)].len = n.clamp(0, PAT_LEN as i16),
                Edit::PatIns(p, r) | Edit::PatRm(p, r) => {
                    let mut st = Stack::new();
                    let ins = matches!(e, Edit::PatIns(..));
                    if ins {
                        st.push(0);
                    }
                    st.push(r as i16);
                    self.eng.pattern_op(p.min(3), if ins { PatOp::Ins } else { PatOp::Rm }, false, &mut st);
                }
                Edit::Load(scene) => {
                    self.release_all();
                    self.eng.load(&scene);
                }
                Edit::Metro(m) => self.eng.vars.m = m.max(METRO_MIN),
                Edit::MetroAct(on) => {
                    self.eng.vars.m_act = on;
                    self.eng.metro_acc = 0;
                }
                Edit::GridKey(x, y, z) => {
                    let fired = self.eng.grid.key(x, y, z);
                    self.eng.run_mask(fired);
                }
            }
        }
    }

    fn release_all(&mut self) {
        self.notes.all_off();
        for v in self.voices.iter_mut() {
            v.gate = false;
        }
        self.sounding = [None; VOICES];
        self.prev_tr = [false; 4];
    }

    /// TR n going high plays CV n's pitch until it goes low again. The
    /// built-in voice follows its CV while it sounds, so CV.SLEW glides.
    fn edges(&mut self) {
        for k in 0..4 {
            let on = self.eng.tr[k].on;
            if on == self.prev_tr[k] {
                if on && self.voices[k].gate {
                    let n = self.eng.cv[k].out() as f32 * 120.0 / 16384.0;
                    self.voices[k].freq = 440.0 * 2f32.powf((n - 69.0) / 12.0);
                }
                continue;
            }
            self.prev_tr[k] = on;
            if let Some(n) = self.sounding[k].take() {
                if self.notes.external() {
                    self.notes.note_off(n);
                }
            }
            self.voices[k].gate = false;
            if on {
                let note = cv_note(self.eng.cv[k].destination());
                if self.notes.internal() {
                    self.voices[k] = Voice { freq: 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0), phase: 0.0, amp: 0.0, gate: true, active: true };
                } else if self.notes.external() {
                    self.notes.note_on(note, 100);
                }
                self.sounding[k] = Some(note);
                self.p.notes_played.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn publish(&mut self) {
        if self.eng.grid.dirty {
            if let Ok(mut g) = self.p.grid_out.try_lock() {
                self.eng.grid.render(&mut g.0);
                g.1 += 1;
                self.eng.grid.dirty = false;
            }
        }
        if let Ok(mut v) = self.p.view.try_lock() {
            let e = &self.eng;
            v.vars = e.vars.v;
            v.m = e.vars.m;
            v.m_act = e.vars.m_act;
            v.time = e.vars.time;
            v.p_n = e.vars.p_n;
            v.pats = e.pats;
            v.cv = std::array::from_fn(|k| e.cv[k].out());
            v.tr = std::array::from_fn(|k| e.tr[k].on);
            v.fired = e.fired;
            v.delays = e.delays.iter().filter(|d| d.ms > 0).count();
            v.stack = e.stack_n;
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
        self.eng.grid.set_size(self.p.grid_size[0].load(Ordering::Relaxed), self.p.grid_size[1].load(Ordering::Relaxed));
        self.apply_edits();
        self.eng.transport = self.p.running.load(Ordering::Relaxed);
        let raw = |v: f32| (v.clamp(0.0, 1.0) * CV_MAX as f32) as i16;
        self.eng.in_raw = raw(self.p.in_cv.get() + self.p.hands[0].get());
        self.eng.param_raw = raw(self.p.param.get() + self.p.param_cv.get() + self.p.hands[1].get());
        for k in 0..8 {
            let high = self.p.trig_in[k].get() > 0.5;
            if high && !self.trig_prev[k] {
                self.eng.run_script(k, None);
            }
            self.trig_prev[k] = high;
        }
        self.edges();

        let wave = self.p.wave.load(Ordering::Relaxed).min(3);
        let release_s = release_ms(self.p.release.get()) / 1000.0;
        let release = (-1.0 / (release_s * rate).max(1.0)).exp();
        let attack = 1.0 / (0.003 * rate);
        let level = self.p.level.get();
        let master = (self.p.mix_level.get() + self.p.ext_mix_level.get()).clamp(0.0, 2.0) * level * level * 0.4;
        let samples_per_ms = rate as f64 / 1000.0;
        let shared = Arc::clone(&self.p);
        let mut bus = shared.output.try_lock().ok();
        if let Some(b) = bus.as_mut() {
            b.clear();
        }
        let mut done = 0usize;
        while done < frames {
            let chunk = ((samples_per_ms - self.ms_acc).ceil().max(1.0) as usize).min(frames - done);
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
            self.ms_acc += chunk as f64;
            while self.ms_acc >= samples_per_ms {
                self.ms_acc -= samples_per_ms;
                self.eng.ms();
                self.edges();
            }
        }
        for k in 0..N_OUTS {
            let target = self.p.outs[k].load(Ordering::Relaxed);
            if target > 0 {
                let value = if k < 4 { self.eng.cv[k].out() as f32 / CV_MAX as f32 } else { self.eng.tr[k - 4].on as u8 as f32 };
                if let Some(h) = self.mods.get(target - 1) {
                    h.set(value);
                }
            }
        }
        self.publish();
    }
}

pub fn create(ctx: &crate::app::AppContext, _id: &str) -> Box<dyn crate::app::App> {
    Box::new(TeletypeApp::new(ctx.named("sensitivity"), ctx.named("nav_speed"), ctx.get(), ctx.get(), ctx.get()).with_notes(ctx.try_get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str) -> Line {
        let t = parse_line(s).unwrap_or_else(|e| panic!("{s}: {e}"));
        let l = Line::from(&t);
        assert!(l.ok, "{s}: {:?}", validate(&t));
        l
    }

    /// An engine with `lines` as script 1 (and `metro`/`init` if given).
    fn engine(lines: &[&str]) -> Engine {
        let mut e = Engine::new();
        for (i, s) in lines.iter().enumerate() {
            e.scripts[0][i] = line(s);
        }
        e.vars.m_act = false;
        e
    }

    fn live(e: &mut Engine, s: &str) {
        e.run_live(&line(s));
    }

    fn get(e: &mut Engine, expr: &str) -> i16 {
        live(e, &format!("Z {expr}"));
        e.vars.v[6]
    }

    #[test]
    fn prefix_evaluation_and_the_set_rule() {
        let mut e = engine(&[]);
        live(&mut e, "X ADD 1 2");
        assert_eq!(e.vars.v[4], 3);
        // Right to left: SUB 10 MUL 2 3 = 10 - 6.
        assert_eq!(get(&mut e, "SUB 10 MUL 2 3"), 4);
        // X first with an extra value sets it; X elsewhere reads it.
        live(&mut e, "Y ADD X X");
        assert_eq!(e.vars.v[5], 6);
        assert_eq!(get(&mut e, "A"), 1, "A defaults to 1");
        live(&mut e, "X 5; Y 7");
        assert_eq!((e.vars.v[4], e.vars.v[5]), (5, 7), "; runs both sub-commands");
        live(&mut e, "CV 1 N 60");
        assert_eq!(e.cv[0].target, 8192, "N 60 is 5 V");
    }

    #[test]
    fn maths_follows_the_module() {
        let mut e = engine(&[]);
        assert_eq!(get(&mut e, "DIV 7 0"), 0, "divide by zero is 0");
        assert_eq!(get(&mut e, "MOD 7 0"), 0);
        assert_eq!(get(&mut e, "ADD 32767 1"), -32768, "16-bit wrap");
        assert_eq!(get(&mut e, "QT 14 5"), 15);
        assert_eq!(get(&mut e, "QT 12 5"), 10);
        assert_eq!(get(&mut e, "LIM 20 0 10"), 10);
        assert_eq!(get(&mut e, "WRAP 11 0 10"), 0);
        assert_eq!(get(&mut e, "WRAP -1 0 10"), 10);
        assert_eq!(get(&mut e, "SCALE 0 100 0 1000 50"), 500);
        assert_eq!(get(&mut e, "AVG 1 2"), 2, "rounds half up");
        assert_eq!(get(&mut e, "V 1"), 1638);
        assert_eq!(get(&mut e, "V 10"), 16384);
        assert_eq!(get(&mut e, "VV 150"), 1638 + 819);
        assert_eq!(get(&mut e, "N 12"), 1638);
        assert_eq!(get(&mut e, "N -12"), -1638);
        assert_eq!(get(&mut e, "BPM 120"), 500);
        assert_eq!(get(&mut e, "? 0 3 4"), 4);
        assert_eq!(get(&mut e, "LSH 1 4"), 16);
        assert_eq!(get(&mut e, "RSH 16 -1"), 32, "a negative shift goes the other way");
        for _ in 0..200 {
            let r = get(&mut e, "RRAND 3 5");
            assert!((3..=5).contains(&r));
            let r = get(&mut e, "RAND 2");
            assert!((0..=2).contains(&r));
        }
        // E(3,8): x..x..x.
        let hits: Vec<i16> = (0..8).map(|i| get(&mut e, &format!("ER 3 8 {i}"))).collect();
        assert_eq!(hits, [1, 0, 0, 1, 0, 0, 1, 0]);
    }

    #[test]
    fn if_elif_else_and_loops() {
        let mut e = engine(&["IF EQ X 1: Y 10", "ELIF EQ X 2: Y 20", "ELSE: Y 30", "L 1 4: Z ADD Z I"]);
        for (x, y) in [(1, 10), (2, 20), (7, 30)] {
            e.vars.v[4] = x;
            e.vars.v[6] = 0;
            e.run_script(0, None);
            assert_eq!(e.vars.v[5], y, "X = {x}");
            assert_eq!(e.vars.v[6], 10, "L 1 4 sums 1+2+3+4");
        }
        let mut e = engine(&["W LT X 5: X ADD X 1"]);
        e.run_script(0, None);
        assert_eq!(e.vars.v[4], 5, "W repeats until its condition fails");
        let mut e = engine(&["X 1", "BREAK", "X 2"]);
        e.run_script(0, None);
        assert_eq!(e.vars.v[4], 1, "BREAK stops the script");
    }

    #[test]
    fn every_skip_and_other_count_per_line() {
        let mut e = engine(&["EVERY 3: X ADD X 1", "OTHER: Y ADD Y 1", "SKIP 3: Z ADD Z 1"]);
        for _ in 0..6 {
            e.run_script(0, None);
        }
        // The 3rd and 6th runs.
        assert_eq!(e.vars.v[4], 2);
        // Z counts on its own line: all but every 3rd.
        assert_eq!(e.vars.v[6], 4);
        // OTHER follows the EVERY on the line above it... except the SKIP below
        // runs after it, so OTHER sees whatever that left from the run before.
        assert!(e.vars.v[5] > 0);
    }

    #[test]
    fn delays_the_stack_and_the_metro_run_on_time() {
        let mut e = engine(&["DEL 50: X 7", "S: Y 3"]);
        e.run_script(0, None);
        for _ in 0..49 {
            e.ms();
        }
        assert_eq!(e.vars.v[4], 0, "not yet");
        e.ms();
        assert_eq!(e.vars.v[4], 7, "after 50 ms");
        assert_eq!(e.stack_n, 1);
        live(&mut e, "S.ALL");
        assert_eq!((e.vars.v[5], e.stack_n), (3, 0));

        let mut e = Engine::new();
        e.scripts[METRO][0] = line("X ADD X 1");
        e.scripts[INIT][0] = line("M 100");
        let scene = Scene { scripts: e.scripts, pats: e.pats };
        e.load(&scene);
        for _ in 0..1000 {
            e.ms();
        }
        assert_eq!(e.vars.v[4], 10, "M 100 runs the metro script 10 times a second");
        live(&mut e, "M 10");
        assert_eq!(e.vars.m, 25, "M has a 25 ms minimum");
    }

    #[test]
    fn triggers_pulse_for_tr_time_and_cv_slews() {
        let mut e = engine(&["TR.TIME 1 20", "TR.P 1"]);
        e.run_script(0, None);
        assert!(e.tr[0].on);
        for _ in 0..19 {
            e.ms();
        }
        assert!(e.tr[0].on);
        e.ms();
        assert!(!e.tr[0].on, "low again after 20 ms");
        live(&mut e, "TR.POL 2 0");
        live(&mut e, "TR 2 1");
        live(&mut e, "TR.P 2");
        assert!(!e.tr[1].on, "a pulse on an inverted output goes low");
        live(&mut e, "CV.SLEW 1 100");
        live(&mut e, "CV 1 V 10");
        for _ in 0..50 {
            e.ms();
        }
        let half = e.cv[0].out();
        assert!((8000..8400).contains(&half), "halfway through the slew: {half}");
        for _ in 0..50 {
            e.ms();
        }
        assert_eq!(e.cv[0].out(), CV_MAX);
    }

    #[test]
    fn patterns_next_wrap_insert_and_remove() {
        let mut e = engine(&[]);
        for v in [10, 20, 30] {
            live(&mut e, &format!("P.PUSH {v}"));
        }
        assert_eq!(e.pats[0].len, 3);
        let seen: Vec<i16> = (0..5).map(|_| get(&mut e, "P.NEXT")).collect();
        assert_eq!(seen, [20, 30, 10, 20, 30], "P.NEXT wraps at the length");
        live(&mut e, "P.WRAP 0");
        assert_eq!(get(&mut e, "P.NEXT"), 30, "without wrap it stops at the end");
        live(&mut e, "P.INS 1 15");
        assert_eq!(&e.pats[0].v[..4], &[10, 15, 20, 30]);
        live(&mut e, "P.RM 0");
        assert_eq!(&e.pats[0].v[..3], &[15, 20, 30]);
        assert_eq!(get(&mut e, "P -1"), 30, "a negative index counts back from the length");
        assert_eq!(get(&mut e, "P.POP"), 30);
        live(&mut e, "PN 2 5 99");
        assert_eq!(e.pats[2].v[5], 99);
    }

    #[test]
    fn scripts_call_scripts_but_not_forever() {
        let mut e = engine(&["X ADD X 1", "$ 1"]);
        e.run_script(0, None);
        assert_eq!(e.vars.v[4], EXEC_DEPTH as i16, "recursion stops at 8 deep");
    }

    #[test]
    fn validation_rejects_what_the_module_rejects() {
        for bad in ["ADD 1", "IF 1 TR.P 1", "ADD 1 2 3", "X: Y 1", "L 1: X 1", "IF X:", "TR.P IF 1: X"] {
            let t = parse_line(bad).unwrap();
            assert!(validate(&t).is_err(), "{bad} should not be accepted");
        }
        for good in ["X", "IF X: TR.P 1; CV 1 0", "L 1 4: TR.P I", "P.NEXT", "EVERY 2: $ 3", "CV 1 N ADD 48 P.NEXT"] {
            let t = parse_line(good).unwrap();
            assert!(validate(&t).is_ok(), "{good}: {:?}", validate(&t));
        }
        assert!(parse_line("FOO 1").is_err());
        assert_eq!(line_text(&parse_line("if x : tr.p 1 ; cv 1 0").unwrap()), "IF X: TR.P 1; CV 1 0");
    }

    #[test]
    fn scene_files_round_trip_and_the_bundled_scenes_load() {
        let text = "a scene\n\n#1\nTR.P 1\nCV 1 N 60\n\n#M\nX ADD X 1\n\n#I\nM 250\n\n#P\n3\t0\t0\t0\n1\t1\t1\t1\n0\t0\t0\t0\n63\t63\t63\t63\n\n5\t0\t0\t0\n6\t0\t0\t0\n7\t0\t0\t0\n";
        let s = SceneText::parse(text);
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        assert_eq!(line_text(&s.scripts[0][1]), "CV 1 N 60");
        assert_eq!(line_text(&s.scripts[INIT][0]), "M 250");
        assert_eq!((s.pats[0].len, s.pats[0].v[2]), (3, 7));
        let again = SceneText::parse(&s.write("a scene"));
        assert_eq!(again, s);
        let dir = Path::new(TT_DIR).join("scenes");
        let mut n = 0;
        for f in std::fs::read_dir(&dir).unwrap().flatten() {
            let t = SceneText::parse(&std::fs::read_to_string(f.path()).unwrap());
            assert!(t.errors.is_empty(), "{:?}: {:?}", f.path(), t.errors);
            n += 1;
        }
        assert!(n >= 4, "{n} bundled scenes");
    }

    // ---- the app

    fn app(dir: &Path) -> (TeletypeApp, Arc<ModBus>) {
        let mods = Arc::new(ModBus::new());
        let a = TeletypeApp::with_dir(dir, Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), Arc::clone(&mods), Arc::new(AudioBus::new()), Arc::new(MixerBus::new()));
        (a, mods)
    }

    fn empty_dir() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("portamax-tt-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(d.join("scenes")).unwrap();
        d
    }

    fn run(p: &mut Box<dyn AudioProcessor>, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for _ in 0..blocks {
            let mut buf = vec![0.0f32; 480 * 2];
            p.process(&mut buf, 2, 48_000.0);
            assert!(buf.iter().all(|v| v.is_finite()));
            all.extend(buf.iter().step_by(2));
        }
        all
    }

    fn press(a: &mut TeletypeApp, pad: usize) {
        a.tick(&Input { grid: std::array::from_fn(|i| i == pad), ..Default::default() });
        a.tick(&Input::default());
    }

    fn page(a: &mut TeletypeApp, name: &str) {
        for _ in 0..20 {
            if a.kit.layer_label() == name {
                return;
            }
            a.toggle_grid_mode();
        }
        panic!("no page {name}");
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn hz(x: &[f32]) -> f32 {
        x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f32 / (x.len() as f32 / 48_000.0)
    }

    #[test]
    fn typing_a_line_with_pads_and_firing_it_plays_a_note_at_its_pitch() {
        let dir = empty_dir();
        let (mut a, _) = app(&dir);
        a.p.wave.store(0, Ordering::Relaxed);
        let mut p = a.audio_processor().unwrap();
        // Script 1, line 1: CV 1 N 69 (A4) ; line 2: TR.P 1.
        page(&mut a, "I/O");
        press(&mut a, 0); // CV
        page(&mut a, "NUMBERS");
        press(&mut a, 0); // 1
        page(&mut a, "I/O");
        press(&mut a, 12); // N
        page(&mut a, "NUMBERS");
        press(&mut a, 6); // 6
        press(&mut a, 9); // 8 -> 68? no: 6 then 9 -> pad 10 is 9
        assert_eq!(line_text(a.cur()), "CV 1 N 68");
        press(&mut a, 12); // backspace removes the whole number
        press(&mut a, 6);
        press(&mut a, 10);
        assert_eq!(line_text(a.cur()), "CV 1 N 69");
        a.tick(&Input { navigation_steps: 1, ..Default::default() });
        page(&mut a, "I/O");
        press(&mut a, 2); // TR.P
        page(&mut a, "NUMBERS");
        press(&mut a, 0);
        assert_eq!(line_text(&a.text.scripts[0][1]), "TR.P 1");
        // TR.TIME 1 is 100 ms: fire script 1 from the SCRIPTS page.
        page(&mut a, "SCRIPTS");
        press(&mut a, 0);
        let out = run(&mut p, 8);
        assert!(rms(&out) > 0.02, "it sounds ({})", rms(&out));
        assert!((hz(&out[960..3840]) - 440.0).abs() < 15.0, "A4, got {}", hz(&out[960..3840]));
        let after = run(&mut p, 60);
        assert!(rms(&after[after.len() - 4800..]) < 0.001, "the gate closed after TR.TIME");
    }

    #[test]
    fn mod_inputs_fire_scripts_and_outputs_reach_other_apps() {
        let dir = empty_dir();
        std::fs::write(dir.join("scenes/t.txt"), "#1\nX ADD X 1\nCV 2 V 5\nTR 1 1\n\n#I\nM.ACT 0\n").unwrap();
        let (mut a, mods) = app(&dir);
        assert_eq!(a.scenes[a.scene].0, "t");
        for name in ["Teletype: Trig 1", "Teletype: Trig 8", "Teletype: IN", "Teletype: PARAM", "Mixer: Teletype Level"] {
            assert!(mods.index_of(name).is_some(), "{name}");
        }
        let target = mods.register("Other: Cutoff");
        let idx = mods.index_of("Other: Cutoff").unwrap();
        a.p.outs[1].store(idx + 1, Ordering::Relaxed); // CV 2
        let mut p = a.audio_processor().unwrap();
        run(&mut p, 2);
        let trig = mods.get(mods.index_of("Teletype: Trig 1").unwrap()).unwrap();
        trig.set(1.0);
        run(&mut p, 2);
        trig.set(0.0);
        run(&mut p, 2);
        trig.set(1.0);
        run(&mut p, 2);
        assert_eq!(a.view().vars[4], 2, "two rising edges ran script 1 twice");
        assert!((target.get() - 0.5).abs() < 0.01, "CV 2 at 5 V reached the routed input: {}", target.get());
    }

    #[test]
    fn saving_writes_a_scene_that_loads_back() {
        let dir = empty_dir();
        let (mut a, _) = app(&dir);
        a.text.scripts[0][0] = parse_line("TR.P 1").unwrap();
        a.text.scripts[METRO][0] = parse_line("CV 1 N P.NEXT").unwrap();
        a.save();
        let path = dir.join("saved/01.txt");
        assert!(path.exists(), "{}", a.status);
        let back = SceneText::parse(&std::fs::read_to_string(path).unwrap());
        assert_eq!(line_text(&back.scripts[METRO][0]), "CV 1 N P.NEXT");
        assert!(a.scenes.iter().any(|(n, _)| n == "saved/01"));
    }

    #[test]
    fn the_bundled_scenes_play() {
        let (mut a, _) = app(Path::new(TT_DIR));
        for i in 1..a.scenes.len() {
            a.load_scene(i);
            assert!(a.status.is_empty(), "{}: {}", a.scenes[i].0, a.status);
        }
        a.load_scene(1);
        let mut p = a.audio_processor().unwrap();
        assert!(rms(&run(&mut p, 100)) < 1e-6, "silent until started");
        a.toggle_running();
        let out = run(&mut p, 300);
        assert!(rms(&out) > 0.005, "{} makes sound on its own ({})", a.scenes[1].0, rms(&out));
    }

    #[test]
    fn a_grid_scene_takes_the_grid_lights_it_and_plays_from_it() {
        let (mut a, _) = app(Path::new(TT_DIR));
        let g = grid_kit::grid();
        g.register("Other");
        g.set_focus("Other");
        let i = a.scenes.iter().position(|(n, _)| n.contains("grid")).expect("the grid scene");
        a.load_scene(i);
        assert!(a.status.is_empty(), "{}", a.status);
        assert_eq!(g.focus().as_deref(), Some(APP_NAME), "a scene with G ops takes the grid");
        assert!(a.needs_background_audio(), "it plays the grid while another app is on screen");
        let mut p = a.audio_processor().unwrap();
        let frame = |a: &mut TeletypeApp, p: &mut Box<dyn AudioProcessor>| {
            a.background_tick();
            run(p, 2);
            a.background_tick();
        };
        frame(&mut a, &mut p);
        let led = |x: usize, y: usize| {
            let s = g.snapshot();
            s.leds[y * s.cols + x]
        };
        // Steps 1 and 9 of track 1 are on, the rest dim; the speed fader is
        // half way.
        assert_eq!((led(0, 0), led(1, 0), led(8, 0), led(0, 4)), (13, 3, 13, 0));
        assert_eq!((led(8, 6), led(9, 6)), (13, 3));
        // A key turns a step on.
        g.press(1, 0, true);
        frame(&mut a, &mut p);
        g.press(1, 0, false);
        frame(&mut a, &mut p);
        assert_eq!(led(1, 0), 13);
        // The fader's far end is the fastest speed, through script 1.
        g.press(15, 6, true);
        frame(&mut a, &mut p);
        g.press(15, 6, false);
        frame(&mut a, &mut p);
        assert_eq!(a.view().m, 60);
        assert!(rms(&run(&mut p, 50)) < 1e-6, "silent until started");
        a.toggle_running();
        let out = run(&mut p, 100);
        assert!(rms(&out) > 0.005, "plays ({})", rms(&out));
        // The playhead brightens its column.
        a.background_tick();
        let s = g.snapshot();
        let bright = (0..16).filter(|&x| (0..4).all(|y| s.leds[y * s.cols + x] >= 6)).count();
        assert_eq!(bright, 1, "{:?}", &s.leds[..16]);
    }
}

#[cfg(test)]
mod screenshot {
    use super::*;

    /// Writes the editor (and the tracker) as PPM files to the folder in
    /// PORTAMAX_TT_SHOT, after playing the first bundled scene for a moment.
    #[test]
    #[ignore = "writes screenshots to the folder in PORTAMAX_TT_SHOT"]
    fn screenshot() {
        let Ok(dir) = std::env::var("PORTAMAX_TT_SHOT") else { return };
        let mods = Arc::new(ModBus::new());
        let mut a = TeletypeApp::new(Arc::new(AtomicF32::new(0.1)), Arc::new(AtomicF32::new(3.0)), mods, Arc::new(AudioBus::new()), Arc::new(MixerBus::new()));
        let mut p = a.audio_processor().unwrap();
        a.toggle_running();
        // Show the metro script, caret mid-line, on the I/O page.
        for _ in 0..8 {
            a.kit_edit(0, 0);
        }
        a.script = METRO;
        a.line = 0;
        a.caret = 3;
        for _ in 0..3 {
            a.toggle_grid_mode();
        }
        for _ in 0..80 {
            let mut buf = vec![0.0f32; 480 * 2];
            p.process(&mut buf, 2, 48_000.0);
        }
        let shoot = |a: &mut TeletypeApp, name: &str| {
            let mut fb = FrameBuffer::new();
            a.draw(&mut fb);
            let mut out = b"P6\n640 360\n255\n".to_vec();
            out.extend(fb.buffer().iter().flat_map(|px| [(px >> 16) as u8, (px >> 8) as u8, *px as u8]));
            std::fs::write(Path::new(&dir).join(name), out).unwrap();
        };
        shoot(&mut a, "tt_editor.ppm");
        for _ in 0..8 {
            a.toggle_grid_mode();
        }
        shoot(&mut a, "tt_tracker.ppm");
        // The grid scene, on the GRID pad page and on the Grid app's screen.
        let i = a.scenes.iter().position(|(n, _)| n.contains("grid")).unwrap();
        a.load_scene(i);
        a.script = INIT;
        for _ in 0..12 {
            a.toggle_grid_mode();
        }
        let mut screen = crate::apps::grid::GridApp::new(grid_kit::grid(), Arc::new(AtomicF32::new(3.0)));
        for _ in 0..45 {
            a.background_tick();
            let mut buf = vec![0.0f32; 480 * 2];
            p.process(&mut buf, 2, 48_000.0);
        }
        a.background_tick();
        shoot(&mut a, "tt_grid_page.ppm");
        let mut fb = FrameBuffer::new();
        screen.draw(&mut fb);
        let mut out = b"P6\n640 360\n255\n".to_vec();
        out.extend(fb.buffer().iter().flat_map(|px| [(px >> 16) as u8, (px >> 8) as u8, *px as u8]));
        std::fs::write(Path::new(&dir).join("tt_grid.ppm"), out).unwrap();
    }
}
