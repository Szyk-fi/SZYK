//! Oracle's expression language -- the "codebox" inside every patch.
//!
//! Every node input and output formula in a patch is a small expression
//! (`"pitch * 2^(detune/12)"`, `"tanh(filt * drive) * env"`). They are
//! parsed and constant-folded once, on the UI/worker thread, into a flat
//! stack bytecode. Evaluation on the audio thread is allocation-free,
//! bounded (stack depth checked at compile time), and cannot panic: every
//! register read is bounds-checked and every math function is total
//! (no NaN from `log(0)` or `sqrt(-1)`).
//!
//! Grammar, loosest to tightest binding:
//!   ternary  a ? b : c
//!   ||  &&  (comparisons return 1.0 / 0.0; nonzero is true)
//!   == != < > <= >=
//!   + -
//!   * / %
//!   unary - !
//!   ^ (power, right-associative)
//!   number | name | name(args) | ( expr )

use std::fmt;

pub const MAX_STACK: usize = 32;
const MAX_EXPR_LEN: usize = 600;

/// Where a name resolves to.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)] // Const: resolvers may inline named constants
pub enum Sym {
    Const(f32),
    Global(u16),
    Voice(u16),
}

/// What the resolver says about a name: where it lives, and whether it can
/// change within an audio block (anything that isn't a parameter, macro or
/// constant). Expressions touching only static names are evaluated once
/// per block instead of per sample.
#[derive(Clone, Copy, Debug)]
pub struct Resolved {
    pub sym: Sym,
    pub dynamic: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum Fn1 {
    Sin, Cos, Tan, Tanh, Abs, Sqrt, Exp, Log, Log2, Floor, Ceil, Fract, Round, Sign,
    Fold, Wrap, Sat, Mtof, Ftom, Db, Tri, Saw, Sqr,
}

#[derive(Clone, Copy, Debug)]
pub enum Fn2 {
    Min, Max, Pow, Step, Crush,
}

#[derive(Clone, Copy, Debug)]
pub enum Fn3 {
    Clamp, Mix, Smoothstep,
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Const(f32),
    LoadG(u16),
    LoadV(u16),
    Add, Sub, Mul, Div, Mod, Pow,
    Neg, Not,
    Lt, Gt, Le, Ge, Eq, Ne, And, Or,
    Select,
    F1(Fn1),
    F2(Fn2),
    F3(Fn3),
    Noise,
}

/// A compiled expression.
#[derive(Clone, Debug)]
pub struct Expr {
    ops: Vec<Op>,
    /// True if it reads anything that can change within a block.
    pub dynamic: bool,
    /// Every name it referenced (for dependency ordering and the graph view).
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExprError(pub String);

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Expr {
    pub fn constant(v: f32) -> Self {
        Self { ops: vec![Op::Const(v)], dynamic: false, names: Vec::new() }
    }

    /// If this expression folded down to a single constant, its value.
    pub fn as_const(&self) -> Option<f32> {
        match self.ops.as_slice() {
            [Op::Const(v)] => Some(*v),
            _ => None,
        }
    }

    pub fn compile(src: &str, resolve: &dyn Fn(&str) -> Option<Resolved>) -> Result<Self, ExprError> {
        if src.len() > MAX_EXPR_LEN {
            return Err(ExprError(format!("expression longer than {MAX_EXPR_LEN} characters")));
        }
        let tokens = tokenize(src)?;
        let mut p = Parser { tokens, pos: 0, resolve, names: Vec::new(), dynamic: false };
        let ast = p.ternary()?;
        if p.pos != p.tokens.len() {
            return Err(ExprError(format!("unexpected '{}' in \"{src}\"", p.tokens[p.pos])));
        }
        let ast = fold(ast);
        let mut ops = Vec::new();
        let depth = emit(&ast, &mut ops);
        if depth > MAX_STACK {
            return Err(ExprError(format!("expression too deeply nested: \"{src}\"")));
        }
        Ok(Self { ops, dynamic: p.dynamic, names: p.names })
    }

    /// Evaluate. `g` = global registers, `v` = this voice's registers (empty
    /// for global scope). Never allocates, never panics.
    #[inline]
    pub fn eval(&self, g: &[f32], v: &[f32], rng: &mut u32) -> f32 {
        let mut stack = [0.0f32; MAX_STACK];
        let mut sp = 0usize;
        macro_rules! pop {
            () => {{
                sp = sp.saturating_sub(1);
                stack[sp]
            }};
        }
        macro_rules! push {
            ($x:expr) => {{
                if sp < MAX_STACK {
                    stack[sp] = $x;
                    sp += 1;
                }
            }};
        }
        for op in &self.ops {
            match *op {
                Op::Const(c) => push!(c),
                Op::LoadG(i) => push!(g.get(i as usize).copied().unwrap_or(0.0)),
                Op::LoadV(i) => push!(v.get(i as usize).copied().unwrap_or(0.0)),
                Op::Neg => {
                    let a = pop!();
                    push!(-a)
                }
                Op::Not => {
                    let a = pop!();
                    push!(if a != 0.0 { 0.0 } else { 1.0 })
                }
                Op::F1(f) => {
                    let a = pop!();
                    push!(call1(f, a))
                }
                Op::F2(f) => {
                    let b = pop!();
                    let a = pop!();
                    push!(call2(f, a, b))
                }
                Op::F3(f) => {
                    let c = pop!();
                    let b = pop!();
                    let a = pop!();
                    push!(call3(f, a, b, c))
                }
                Op::Select => {
                    let c = pop!();
                    let b = pop!();
                    let a = pop!();
                    push!(if a != 0.0 { b } else { c })
                }
                Op::Noise => push!(white(rng)),
                bin => {
                    let b = pop!();
                    let a = pop!();
                    push!(binop(bin, a, b))
                }
            }
        }
        if sp == 0 {
            0.0
        } else {
            stack[sp - 1]
        }
    }
}

/// xorshift32 white noise in -1..1.
#[inline]
pub fn white(rng: &mut u32) -> f32 {
    let mut x = *rng;
    if x == 0 {
        x = 0x9E37_79B9;
    }
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *rng = x;
    (x as f32 / u32::MAX as f32) * 2.0 - 1.0
}

#[inline]
fn binop(op: Op, a: f32, b: f32) -> f32 {
    let t = |c: bool| if c { 1.0 } else { 0.0 };
    match op {
        Op::Add => a + b,
        Op::Sub => a - b,
        Op::Mul => a * b,
        Op::Div => {
            if b.abs() < 1e-12 {
                0.0
            } else {
                a / b
            }
        }
        Op::Mod => {
            if b.abs() < 1e-12 {
                0.0
            } else {
                a.rem_euclid(b)
            }
        }
        Op::Pow => safe_pow(a, b),
        Op::Lt => t(a < b),
        Op::Gt => t(a > b),
        Op::Le => t(a <= b),
        Op::Ge => t(a >= b),
        Op::Eq => t((a - b).abs() < 1e-6),
        Op::Ne => t((a - b).abs() >= 1e-6),
        Op::And => t(a != 0.0 && b != 0.0),
        Op::Or => t(a != 0.0 || b != 0.0),
        _ => 0.0,
    }
}

#[inline]
fn safe_pow(a: f32, b: f32) -> f32 {
    let r = if a < 0.0 && b.fract() != 0.0 { -(-a).powf(b) } else { a.powf(b) };
    if r.is_finite() {
        r
    } else {
        0.0
    }
}

#[inline]
fn call1(f: Fn1, a: f32) -> f32 {
    match f {
        Fn1::Sin => a.sin(),
        Fn1::Cos => a.cos(),
        Fn1::Tan => a.tan().clamp(-1e4, 1e4),
        Fn1::Tanh => a.tanh(),
        Fn1::Abs => a.abs(),
        Fn1::Sqrt => a.max(0.0).sqrt(),
        Fn1::Exp => a.min(80.0).exp(),
        Fn1::Log => a.max(1e-12).ln(),
        Fn1::Log2 => a.max(1e-12).log2(),
        Fn1::Floor => a.floor(),
        Fn1::Ceil => a.ceil(),
        Fn1::Fract => a - a.floor(),
        Fn1::Round => a.round(),
        Fn1::Sign => {
            if a > 0.0 {
                1.0
            } else if a < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        // Triangle wavefolder: folds anything back into -1..1.
        Fn1::Fold => {
            let x = (a + 1.0) * 0.25;
            let x = x - x.floor();
            1.0 - 4.0 * (x - 0.5).abs()
        }
        Fn1::Wrap => {
            let x = (a + 1.0) * 0.5;
            (x - x.floor()) * 2.0 - 1.0
        }
        Fn1::Sat => a / (1.0 + a.abs()),
        Fn1::Mtof => 440.0 * 2f32.powf((a.clamp(-100.0, 200.0) - 69.0) / 12.0),
        Fn1::Ftom => 69.0 + 12.0 * (a.max(1e-3) / 440.0).log2(),
        Fn1::Db => 10f32.powf(a.clamp(-200.0, 60.0) / 20.0),
        // Phase-driven shapes, `a` in cycles.
        Fn1::Tri => {
            let p = a - a.floor();
            1.0 - 4.0 * (p - 0.5).abs()
        }
        Fn1::Saw => {
            let p = a - a.floor();
            2.0 * p - 1.0
        }
        Fn1::Sqr => {
            let p = a - a.floor();
            if p < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
    }
    .clamp(-1e6, 1e6)
}

#[inline]
fn call2(f: Fn2, a: f32, b: f32) -> f32 {
    match f {
        Fn2::Min => a.min(b),
        Fn2::Max => a.max(b),
        Fn2::Pow => safe_pow(a, b),
        Fn2::Step => {
            if b >= a {
                1.0
            } else {
                0.0
            }
        }
        Fn2::Crush => {
            let levels = 2f32.powf(b.clamp(1.0, 24.0));
            (a * levels).round() / levels
        }
    }
}

#[inline]
fn call3(f: Fn3, a: f32, b: f32, c: f32) -> f32 {
    match f {
        Fn3::Clamp => {
            let (lo, hi) = if b <= c { (b, c) } else { (c, b) };
            a.clamp(lo, hi)
        }
        Fn3::Mix => a + (b - a) * c,
        Fn3::Smoothstep => {
            if (b - a).abs() < 1e-12 {
                return if c >= b { 1.0 } else { 0.0 };
            }
            let t = ((c - a) / (b - a)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        }
    }
}

// ---------------------------------------------------------------- lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f32),
    Name(String),
    Sym(&'static str),
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::Num(n) => write!(f, "{n}"),
            Tok::Name(s) => f.write_str(s),
            Tok::Sym(s) => f.write_str(s),
        }
    }
}

const SYMS: [&str; 22] = [
    "<=", ">=", "==", "!=", "&&", "||", "+", "-", "*", "/", "%", "^", "(", ")", ",", "<", ">", "!", "?", ":", "=", ";",
];

fn tokenize(src: &str) -> Result<Vec<Tok>, ExprError> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let save = i;
                i += 1;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    i += 1;
                }
                if i < chars.len() && chars[i].is_ascii_digit() {
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    i = save;
                }
            }
            let text: String = chars[start..i].iter().collect();
            let n: f32 = text.parse().map_err(|_| ExprError(format!("bad number '{text}'")))?;
            out.push(Tok::Num(n));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Name(chars[start..i].iter().collect()));
            continue;
        }
        let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
        if let Some(s) = SYMS.iter().find(|s| rest.starts_with(**s)) {
            if *s == "=" || *s == ";" {
                return Err(ExprError(format!(
                    "'{s}' is not allowed: an expression is a single formula (use == to compare)"
                )));
            }
            out.push(Tok::Sym(s));
            i += s.len();
            continue;
        }
        return Err(ExprError(format!("unexpected character '{c}'")));
    }
    Ok(out)
}

// --------------------------------------------------------------- parser

#[derive(Clone, Debug)]
enum Ast {
    Num(f32),
    Load(Sym),
    Un(Op, Box<Ast>),
    Bin(Op, Box<Ast>, Box<Ast>),
    Sel(Box<Ast>, Box<Ast>, Box<Ast>),
    Call(Op, Vec<Ast>),
}

struct Parser<'a> {
    tokens: Vec<Tok>,
    pos: usize,
    resolve: &'a dyn Fn(&str) -> Option<Resolved>,
    names: Vec<String>,
    dynamic: bool,
}

impl Parser<'_> {
    fn peek_sym(&self, s: &str) -> bool {
        matches!(self.tokens.get(self.pos), Some(Tok::Sym(t)) if *t == s)
    }

    fn eat(&mut self, s: &str) -> bool {
        if self.peek_sym(s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &str) -> Result<(), ExprError> {
        if self.eat(s) {
            Ok(())
        } else {
            Err(ExprError(format!(
                "expected '{s}' but found {}",
                self.tokens.get(self.pos).map(|t| format!("'{t}'")).unwrap_or_else(|| "end of expression".into())
            )))
        }
    }

    fn ternary(&mut self) -> Result<Ast, ExprError> {
        let cond = self.or()?;
        if self.eat("?") {
            let a = self.ternary()?;
            self.expect(":")?;
            let b = self.ternary()?;
            return Ok(Ast::Sel(Box::new(cond), Box::new(a), Box::new(b)));
        }
        Ok(cond)
    }

    fn or(&mut self) -> Result<Ast, ExprError> {
        let mut l = self.and()?;
        while self.eat("||") {
            let r = self.and()?;
            l = Ast::Bin(Op::Or, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn and(&mut self) -> Result<Ast, ExprError> {
        let mut l = self.cmp()?;
        while self.eat("&&") {
            let r = self.cmp()?;
            l = Ast::Bin(Op::And, Box::new(l), Box::new(r));
        }
        Ok(l)
    }

    fn cmp(&mut self) -> Result<Ast, ExprError> {
        let mut l = self.add()?;
        loop {
            let op = if self.eat("==") {
                Op::Eq
            } else if self.eat("!=") {
                Op::Ne
            } else if self.eat("<=") {
                Op::Le
            } else if self.eat(">=") {
                Op::Ge
            } else if self.eat("<") {
                Op::Lt
            } else if self.eat(">") {
                Op::Gt
            } else {
                return Ok(l);
            };
            let r = self.add()?;
            l = Ast::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn add(&mut self) -> Result<Ast, ExprError> {
        let mut l = self.mul()?;
        loop {
            let op = if self.eat("+") {
                Op::Add
            } else if self.eat("-") {
                Op::Sub
            } else {
                return Ok(l);
            };
            let r = self.mul()?;
            l = Ast::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn mul(&mut self) -> Result<Ast, ExprError> {
        let mut l = self.unary()?;
        loop {
            let op = if self.eat("*") {
                Op::Mul
            } else if self.eat("/") {
                Op::Div
            } else if self.eat("%") {
                Op::Mod
            } else {
                return Ok(l);
            };
            let r = self.unary()?;
            l = Ast::Bin(op, Box::new(l), Box::new(r));
        }
    }

    fn unary(&mut self) -> Result<Ast, ExprError> {
        if self.eat("-") {
            return Ok(Ast::Un(Op::Neg, Box::new(self.unary()?)));
        }
        if self.eat("+") {
            return self.unary();
        }
        if self.eat("!") {
            return Ok(Ast::Un(Op::Not, Box::new(self.unary()?)));
        }
        self.power()
    }

    fn power(&mut self) -> Result<Ast, ExprError> {
        let base = self.primary()?;
        if self.eat("^") {
            let exp = self.unary()?; // right-assoc, allows 2^-x
            return Ok(Ast::Bin(Op::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Ast, ExprError> {
        let tok = self.tokens.get(self.pos).cloned();
        match tok {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(Ast::Num(n))
            }
            Some(Tok::Sym("(")) => {
                self.pos += 1;
                let e = self.ternary()?;
                self.expect(")")?;
                Ok(e)
            }
            Some(Tok::Name(name)) => {
                self.pos += 1;
                if self.eat("(") {
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            args.push(self.ternary()?);
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                    return self.call(&name, args);
                }
                match name.as_str() {
                    "pi" => return Ok(Ast::Num(std::f32::consts::PI)),
                    "tau" => return Ok(Ast::Num(std::f32::consts::TAU)),
                    "e" => return Ok(Ast::Num(std::f32::consts::E)),
                    _ => {}
                }
                match (self.resolve)(&name) {
                    Some(r) => {
                        if !self.names.contains(&name) {
                            self.names.push(name.clone());
                        }
                        self.dynamic |= r.dynamic;
                        Ok(match r.sym {
                            Sym::Const(c) => Ast::Num(c),
                            s => Ast::Load(s),
                        })
                    }
                    None => Err(ExprError(format!("unknown name '{name}'"))),
                }
            }
            Some(t) => Err(ExprError(format!("unexpected '{t}'"))),
            None => Err(ExprError("expression ended unexpectedly".into())),
        }
    }

    fn call(&mut self, name: &str, args: Vec<Ast>) -> Result<Ast, ExprError> {
        let n = args.len();
        let want = |k: usize| -> Result<(), ExprError> {
            if n == k {
                Ok(())
            } else {
                Err(ExprError(format!("{name}() takes {k} argument(s), got {n}")))
            }
        };
        let op = match name {
            "noise" => {
                want(0)?;
                self.dynamic = true;
                Op::Noise
            }
            "sin" | "cos" | "tan" | "tanh" | "abs" | "sqrt" | "exp" | "log" | "log2" | "floor" | "ceil" | "fract"
            | "round" | "sign" | "fold" | "wrap" | "sat" | "mtof" | "ftom" | "db" | "tri" | "saw" | "sqr" => {
                want(1)?;
                Op::F1(match name {
                    "sin" => Fn1::Sin,
                    "cos" => Fn1::Cos,
                    "tan" => Fn1::Tan,
                    "tanh" => Fn1::Tanh,
                    "abs" => Fn1::Abs,
                    "sqrt" => Fn1::Sqrt,
                    "exp" => Fn1::Exp,
                    "log" => Fn1::Log,
                    "log2" => Fn1::Log2,
                    "floor" => Fn1::Floor,
                    "ceil" => Fn1::Ceil,
                    "fract" => Fn1::Fract,
                    "round" => Fn1::Round,
                    "sign" => Fn1::Sign,
                    "fold" => Fn1::Fold,
                    "wrap" => Fn1::Wrap,
                    "sat" => Fn1::Sat,
                    "mtof" => Fn1::Mtof,
                    "ftom" => Fn1::Ftom,
                    "db" => Fn1::Db,
                    "tri" => Fn1::Tri,
                    "saw" => Fn1::Saw,
                    _ => Fn1::Sqr,
                })
            }
            "min" | "max" | "pow" | "step" | "crush" => {
                want(2)?;
                Op::F2(match name {
                    "min" => Fn2::Min,
                    "max" => Fn2::Max,
                    "pow" => Fn2::Pow,
                    "step" => Fn2::Step,
                    _ => Fn2::Crush,
                })
            }
            "clamp" | "mix" | "smoothstep" => {
                want(3)?;
                Op::F3(match name {
                    "clamp" => Fn3::Clamp,
                    "mix" => Fn3::Mix,
                    _ => Fn3::Smoothstep,
                })
            }
            _ => return Err(ExprError(format!("unknown function '{name}()'"))),
        };
        Ok(Ast::Call(op, args))
    }
}

// ---------------------------------------------------- folding + emitting

fn fold(ast: Ast) -> Ast {
    match ast {
        Ast::Un(op, a) => {
            let a = fold(*a);
            if let Ast::Num(x) = a {
                return Ast::Num(match op {
                    Op::Neg => -x,
                    _ => {
                        if x != 0.0 {
                            0.0
                        } else {
                            1.0
                        }
                    }
                });
            }
            Ast::Un(op, Box::new(a))
        }
        Ast::Bin(op, a, b) => {
            let (a, b) = (fold(*a), fold(*b));
            if let (Ast::Num(x), Ast::Num(y)) = (&a, &b) {
                return Ast::Num(binop(op, *x, *y));
            }
            // x*1, x+0 and friends
            match (&op, &a, &b) {
                (Op::Mul, Ast::Num(o), e) | (Op::Mul, e, Ast::Num(o)) if *o == 1.0 => return e.clone(),
                (Op::Add, Ast::Num(z), e) | (Op::Add, e, Ast::Num(z)) if *z == 0.0 => return e.clone(),
                (Op::Sub, e, Ast::Num(z)) if *z == 0.0 => return e.clone(),
                _ => {}
            }
            Ast::Bin(op, Box::new(a), Box::new(b))
        }
        Ast::Sel(c, a, b) => {
            let (c, a, b) = (fold(*c), fold(*a), fold(*b));
            if let Ast::Num(x) = c {
                return if x != 0.0 { a } else { b };
            }
            Ast::Sel(Box::new(c), Box::new(a), Box::new(b))
        }
        Ast::Call(op, args) => {
            let args: Vec<Ast> = args.into_iter().map(fold).collect();
            let consts: Option<Vec<f32>> =
                args.iter().map(|a| if let Ast::Num(x) = a { Some(*x) } else { None }).collect();
            if let Some(c) = consts {
                match op {
                    Op::F1(f) => return Ast::Num(call1(f, c[0])),
                    Op::F2(f) => return Ast::Num(call2(f, c[0], c[1])),
                    Op::F3(f) => return Ast::Num(call3(f, c[0], c[1], c[2])),
                    _ => {}
                }
            }
            Ast::Call(op, args)
        }
        other => other,
    }
}

/// Emits postfix ops, returns the max stack depth needed.
fn emit(ast: &Ast, ops: &mut Vec<Op>) -> usize {
    match ast {
        Ast::Num(n) => {
            ops.push(Op::Const(*n));
            1
        }
        Ast::Load(Sym::Global(i)) => {
            ops.push(Op::LoadG(*i));
            1
        }
        Ast::Load(Sym::Voice(i)) => {
            ops.push(Op::LoadV(*i));
            1
        }
        Ast::Load(Sym::Const(c)) => {
            ops.push(Op::Const(*c));
            1
        }
        Ast::Un(op, a) => {
            let d = emit(a, ops);
            ops.push(*op);
            d
        }
        Ast::Bin(op, a, b) => {
            let da = emit(a, ops);
            let db = emit(b, ops);
            ops.push(*op);
            da.max(db + 1)
        }
        Ast::Sel(c, a, b) => {
            let dc = emit(c, ops);
            let da = emit(a, ops);
            let db = emit(b, ops);
            ops.push(Op::Select);
            dc.max(da + 1).max(db + 2)
        }
        Ast::Call(op, args) => {
            let mut d = 0;
            for (k, a) in args.iter().enumerate() {
                d = d.max(emit(a, ops) + k);
            }
            ops.push(*op);
            d.max(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver(name: &str) -> Option<Resolved> {
        match name {
            "x" => Some(Resolved { sym: Sym::Global(0), dynamic: true }),
            "p" => Some(Resolved { sym: Sym::Global(1), dynamic: false }),
            "pitch" => Some(Resolved { sym: Sym::Voice(0), dynamic: true }),
            _ => None,
        }
    }

    fn ev(src: &str) -> f32 {
        let e = Expr::compile(src, &resolver).unwrap_or_else(|err| panic!("{src}: {err}"));
        let mut rng = 1;
        e.eval(&[2.0, 10.0], &[440.0], &mut rng)
    }

    #[test]
    fn arithmetic_precedence_and_associativity() {
        assert_eq!(ev("1 + 2 * 3"), 7.0);
        assert_eq!(ev("(1 + 2) * 3"), 9.0);
        assert_eq!(ev("2 ^ 3 ^ 2"), 512.0);
        assert_eq!(ev("-2 ^ 2"), -4.0);
        assert_eq!(ev("2 ^ -1"), 0.5);
        assert_eq!(ev("7 % 4"), 3.0);
    }

    #[test]
    fn names_functions_and_ternary() {
        assert_eq!(ev("x * p"), 20.0);
        assert_eq!(ev("pitch / 2"), 220.0);
        assert_eq!(ev("x > 1 ? p : 0"), 10.0);
        assert_eq!(ev("clamp(x * 100, 0, 5)"), 5.0);
        assert_eq!(ev("mix(0, p, 0.5)"), 5.0);
        assert!((ev("mtof(69)") - 440.0).abs() < 1e-3);
        assert!((ev("fold(3)") - (-1.0)).abs() < 1e-5);
    }

    #[test]
    fn division_by_zero_and_bad_math_never_produce_nan() {
        for src in ["1 / 0", "5 % 0", "log(0)", "sqrt(-1)", "(-8) ^ 0.5", "exp(1000)", "tan(1.5707964)"] {
            let v = ev(src);
            assert!(v.is_finite(), "{src} gave {v}");
        }
    }

    #[test]
    fn constant_folding_and_rate_detection() {
        let e = Expr::compile("2 * 3 + 1", &resolver).unwrap();
        assert_eq!(e.as_const(), Some(7.0));
        let e = Expr::compile("p * 2", &resolver).unwrap();
        assert!(!e.dynamic, "params only -> block rate");
        let e = Expr::compile("p * x", &resolver).unwrap();
        assert!(e.dynamic);
        let e = Expr::compile("noise() * p", &resolver).unwrap();
        assert!(e.dynamic, "noise is always per-sample");
    }

    #[test]
    fn helpful_errors() {
        let err = |s: &str| Expr::compile(s, &resolver).unwrap_err().0;
        assert!(err("y + 1").contains("unknown name 'y'"));
        assert!(err("sin(1, 2)").contains("takes 1"));
        assert!(err("x = 1").contains("single formula"));
        assert!(err("(x + 1").contains("expected ')'"));
        assert!(err("wobble(x)").contains("unknown function"));
    }

    #[test]
    fn deep_nesting_is_rejected_not_overflowed() {
        let src = format!("{}x{}", "(x+".repeat(40), ")".repeat(40));
        assert!(Expr::compile(&src, &resolver).is_err());
    }
}
