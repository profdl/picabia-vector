//! Per-copy expressions: a small, bounded arithmetic language for the numeric params of
//! `transformEach` and `randomize`, evaluated once per instance.
//!
//! ```text
//! rotate      = i * 15                 0°, 15°, 30°… per copy
//! offsetY     = sin(t * 360) * 40      one sine wave over the whole set
//! scaleX      = value * (1 - t / 2)    the field's own value, shrinking to half
//! ```
//!
//! # Syntax
//! - Numbers: `12`, `0.5`, `.5`, `1e3`.
//! - Variables: `i` (the copy's index, 0-based), `n` (the number of copies), `t` (`i / (n - 1)`,
//!   0 when `n` = 1), `value` (the field's own value), `pi`.
//! - Operators, loosest first: `+ -`, `* / %`, unary `- +`, `^` (power, right-associative, so
//!   `-2^2` = -4 and `2^3^2` = 512). Parentheses group.
//! - Functions: `sin cos tan` (degrees, like every angle in Picabia), `asin acos atan` (to
//!   degrees), `atan2(y, x)`, `abs floor ceil round sqrt sign exp ln`, `min max` (1 to 8
//!   arguments), `mod(a, b)` (always ≥ 0 for b > 0), `clamp(x, lo, hi)`, `lerp(a, b, t)`,
//!   `rad(deg)`, `deg(rad)`, `rand(k)` (a number in 0..1 from `k`, the same everywhere).
//!
//! # Bounds
//! Texts over [`MAX_EXPR_LEN`] bytes and nesting deeper than [`MAX_EXPR_DEPTH`] are refused when
//! parsing, so evaluation is a bounded walk with no allocation. Evaluation never panics; a result
//! that isn't a finite number (`1/0`, `sqrt(-1)`) is replaced by a fallback (the field's value)
//! by [`Expr::eval_or`].

use std::fmt;

/// The longest expression text (bytes).
pub const MAX_EXPR_LEN: usize = 256;
/// The deepest nesting of parentheses, function calls and unary operators.
pub const MAX_EXPR_DEPTH: usize = 32;
/// The most arguments `min`/`max` take.
const MAX_ARGS: usize = 8;

/// The per-copy values an expression can read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Env {
    /// The copy's index (0-based).
    pub i: f64,
    /// The number of copies.
    pub n: f64,
    /// `i / (n - 1)`, 0 when `n` ≤ 1.
    pub t: f64,
    /// The field's own value.
    pub value: f64,
}

impl Env {
    /// The values for copy `i` of `n`, for a field whose own value is `value`.
    pub fn new(i: usize, n: usize, value: f64) -> Env {
        let t = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.0 };
        Env { i: i as f64, n: n as f64, t, value }
    }
}

/// Why an expression didn't parse: a message and the byte offset it refers to.
#[derive(Clone, Debug, PartialEq)]
pub struct ExprError {
    pub message: String,
    pub at: usize,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at character {})", self.message, self.at + 1)
    }
}

impl std::error::Error for ExprError {}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Var {
    I,
    N,
    T,
    Value,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Bin {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Abs,
    Floor,
    Ceil,
    Round,
    Sqrt,
    Sign,
    Exp,
    Ln,
    Min,
    Max,
    Mod,
    Clamp,
    Lerp,
    Rad,
    Deg,
    Rand,
}

impl Func {
    fn of(name: &str) -> Option<Func> {
        Some(match name {
            "sin" => Func::Sin,
            "cos" => Func::Cos,
            "tan" => Func::Tan,
            "asin" => Func::Asin,
            "acos" => Func::Acos,
            "atan" => Func::Atan,
            "atan2" => Func::Atan2,
            "abs" => Func::Abs,
            "floor" => Func::Floor,
            "ceil" => Func::Ceil,
            "round" => Func::Round,
            "sqrt" => Func::Sqrt,
            "sign" => Func::Sign,
            "exp" => Func::Exp,
            "ln" => Func::Ln,
            "min" => Func::Min,
            "max" => Func::Max,
            "mod" => Func::Mod,
            "clamp" => Func::Clamp,
            "lerp" => Func::Lerp,
            "rad" => Func::Rad,
            "deg" => Func::Deg,
            "rand" => Func::Rand,
            _ => return None,
        })
    }

    /// The argument counts the function takes, `(min, max)`.
    fn arity(self) -> (usize, usize) {
        match self {
            Func::Min | Func::Max => (1, MAX_ARGS),
            Func::Atan2 | Func::Mod => (2, 2),
            Func::Clamp | Func::Lerp => (3, 3),
            _ => (1, 1),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Num(f64),
    Var(Var),
    Neg(Box<Node>),
    Bin(Bin, Box<Node>, Box<Node>),
    Call(Func, Vec<Node>),
}

/// A parsed expression, ready to evaluate per copy.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    root: Node,
}

impl Expr {
    /// Parse `text`. Refused: empty text, more than [`MAX_EXPR_LEN`] bytes, nesting deeper than
    /// [`MAX_EXPR_DEPTH`], unknown names, wrong argument counts, stray characters.
    pub fn parse(text: &str) -> Result<Expr, ExprError> {
        if text.len() > MAX_EXPR_LEN {
            return Err(ExprError { message: format!("keep expressions under {MAX_EXPR_LEN} characters"), at: 0 });
        }
        let tokens = lex(text)?;
        let mut p = Parser { tokens: &tokens, pos: 0, end: text.len() };
        if tokens.is_empty() {
            return Err(ExprError { message: "the expression is empty".into(), at: 0 });
        }
        let root = p.expr(0, 0)?;
        match p.peek() {
            None => Ok(Expr { root }),
            Some((_, at)) => Err(ExprError { message: "unexpected text after the expression".into(), at }),
        }
    }

    /// The value for `env`; may be NaN or infinite (see [`Expr::eval_or`]).
    pub fn eval(&self, env: &Env) -> f64 {
        eval(&self.root, env)
    }

    /// The value for `env`, or `fallback` when it isn't a finite number.
    pub fn eval_or(&self, env: &Env, fallback: f64) -> f64 {
        let v = self.eval(env);
        if v.is_finite() { v } else { fallback }
    }

    /// Whether the expression reads `i`, `n` or `t` (its value differs per copy).
    pub fn varies(&self) -> bool {
        fn walk(n: &Node) -> bool {
            match n {
                Node::Num(_) => false,
                Node::Var(v) => *v != Var::Value,
                Node::Neg(a) => walk(a),
                Node::Bin(_, a, b) => walk(a) || walk(b),
                Node::Call(Func::Rand, _) => true,
                Node::Call(_, args) => args.iter().any(walk),
            }
        }
        walk(&self.root)
    }
}

/// Check `text` parses (for the panel's expression editor and `modifier.set`).
pub fn check(text: &str) -> Result<(), ExprError> {
    Expr::parse(text).map(|_| ())
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Name(String),
    Op(char),
    Open,
    Close,
    Comma,
}

fn lex(text: &str) -> Result<Vec<(Tok, usize)>, ExprError> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(&b) = bytes.get(at) {
        let start = at;
        match b {
            b' ' | b'\t' | b'\n' | b'\r' => at += 1,
            b'0'..=b'9' | b'.' => {
                while bytes.get(at).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
                    at += 1;
                }
                // An exponent: `e`/`E`, an optional sign, at least one digit.
                if bytes.get(at).is_some_and(|c| *c == b'e' || *c == b'E') {
                    let mut k = at + 1;
                    if bytes.get(k).is_some_and(|c| *c == b'+' || *c == b'-') {
                        k += 1;
                    }
                    if bytes.get(k).is_some_and(u8::is_ascii_digit) {
                        at = k;
                        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                            at += 1;
                        }
                    }
                }
                let s = text.get(start..at).unwrap_or_default();
                let v = s.parse::<f64>().map_err(|_| ExprError { message: format!("`{s}` isn't a number"), at: start })?;
                out.push((Tok::Num(v), start));
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                while bytes.get(at).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                    at += 1;
                }
                out.push((Tok::Name(text.get(start..at).unwrap_or_default().to_ascii_lowercase()), start));
            }
            b'+' | b'-' | b'*' | b'/' | b'%' | b'^' => {
                out.push((Tok::Op(char::from(b)), start));
                at += 1;
            }
            b'(' => {
                out.push((Tok::Open, start));
                at += 1;
            }
            b')' => {
                out.push((Tok::Close, start));
                at += 1;
            }
            b',' => {
                out.push((Tok::Comma, start));
                at += 1;
            }
            _ => {
                let c = text.get(start..).and_then(|s| s.chars().next()).unwrap_or('?');
                return Err(ExprError { message: format!("unexpected `{c}`"), at: start });
            }
        }
    }
    Ok(out)
}

struct Parser<'a> {
    tokens: &'a [(Tok, usize)],
    pos: usize,
    end: usize,
}

fn binding(op: char) -> Option<(u8, Bin)> {
    Some(match op {
        '+' => (1, Bin::Add),
        '-' => (1, Bin::Sub),
        '*' => (2, Bin::Mul),
        '/' => (2, Bin::Div),
        '%' => (2, Bin::Rem),
        '^' => (4, Bin::Pow),
        _ => return None,
    })
}

/// Unary minus binds tighter than `* /` and looser than `^`.
const UNARY: u8 = 3;

impl Parser<'_> {
    fn peek(&self) -> Option<(&Tok, usize)> {
        self.tokens.get(self.pos).map(|(t, at)| (t, *at))
    }

    fn at(&self) -> usize {
        self.peek().map_or(self.end, |(_, at)| at)
    }

    fn err(&self, message: impl Into<String>) -> ExprError {
        ExprError { message: message.into(), at: self.at() }
    }

    fn deeper(&self, depth: usize) -> Result<usize, ExprError> {
        if depth >= MAX_EXPR_DEPTH { Err(self.err("the expression is nested too deeply")) } else { Ok(depth + 1) }
    }

    /// Precedence climbing: an operand, then operators binding at least `min`.
    fn expr(&mut self, min: u8, depth: usize) -> Result<Node, ExprError> {
        let mut lhs = self.unary(depth)?;
        while let Some((Tok::Op(op), _)) = self.peek() {
            let Some((prec, bin)) = binding(*op) else { break };
            if prec < min {
                break;
            }
            self.pos += 1;
            // `^` is right-associative; the rest are left-associative.
            let next = if bin == Bin::Pow { prec } else { prec + 1 };
            let rhs = self.expr(next, self.deeper(depth)?)?;
            lhs = Node::Bin(bin, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self, depth: usize) -> Result<Node, ExprError> {
        match self.peek() {
            Some((Tok::Op('-'), _)) => {
                self.pos += 1;
                let d = self.deeper(depth)?;
                Ok(Node::Neg(Box::new(self.expr(UNARY, d)?)))
            }
            Some((Tok::Op('+'), _)) => {
                self.pos += 1;
                let d = self.deeper(depth)?;
                self.expr(UNARY, d)
            }
            _ => self.primary(depth),
        }
    }

    fn primary(&mut self, depth: usize) -> Result<Node, ExprError> {
        let Some((tok, at)) = self.peek() else {
            return Err(self.err("the expression ends too soon"));
        };
        let tok = tok.clone();
        self.pos += 1;
        match tok {
            Tok::Num(v) => Ok(Node::Num(v)),
            Tok::Open => {
                let inner = self.expr(0, self.deeper(depth)?)?;
                self.close()?;
                Ok(inner)
            }
            Tok::Name(name) => {
                if let Some((Tok::Open, _)) = self.peek() {
                    let f = Func::of(&name).ok_or_else(|| ExprError { message: format!("unknown function `{name}`"), at })?;
                    self.pos += 1;
                    let args = self.args(depth)?;
                    let (lo, hi) = f.arity();
                    if args.len() < lo || args.len() > hi {
                        let want = if lo == hi { format!("{lo}") } else { format!("{lo} to {hi}") };
                        return Err(ExprError { message: format!("`{name}` takes {want} argument(s), got {}", args.len()), at });
                    }
                    return Ok(Node::Call(f, args));
                }
                Ok(match name.as_str() {
                    "i" => Node::Var(Var::I),
                    "n" => Node::Var(Var::N),
                    "t" => Node::Var(Var::T),
                    "value" => Node::Var(Var::Value),
                    "pi" => Node::Num(std::f64::consts::PI),
                    _ if Func::of(&name).is_some() => return Err(ExprError { message: format!("`{name}` needs parentheses: {name}(…)"), at }),
                    _ => return Err(ExprError { message: format!("unknown name `{name}` (use i, n, t, value or pi)"), at }),
                })
            }
            Tok::Op(c) => Err(ExprError { message: format!("unexpected `{c}`"), at }),
            Tok::Close => Err(ExprError { message: "unexpected `)`".into(), at }),
            Tok::Comma => Err(ExprError { message: "unexpected `,`".into(), at }),
        }
    }

    fn close(&mut self) -> Result<(), ExprError> {
        match self.peek() {
            Some((Tok::Close, _)) => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(self.err("missing `)`")),
        }
    }

    /// Call arguments after `(`, through the closing `)`.
    fn args(&mut self, depth: usize) -> Result<Vec<Node>, ExprError> {
        let d = self.deeper(depth)?;
        let mut args = Vec::new();
        if let Some((Tok::Close, _)) = self.peek() {
            self.pos += 1;
            return Ok(args);
        }
        loop {
            if args.len() >= MAX_ARGS {
                return Err(self.err(format!("at most {MAX_ARGS} arguments")));
            }
            args.push(self.expr(0, d)?);
            match self.peek() {
                Some((Tok::Comma, _)) => self.pos += 1,
                _ => break,
            }
        }
        self.close()?;
        Ok(args)
    }
}

fn arg(args: &[Node], k: usize, env: &Env) -> f64 {
    args.get(k).map_or(f64::NAN, |a| eval(a, env))
}

fn eval(n: &Node, env: &Env) -> f64 {
    match n {
        Node::Num(v) => *v,
        Node::Var(Var::I) => env.i,
        Node::Var(Var::N) => env.n,
        Node::Var(Var::T) => env.t,
        Node::Var(Var::Value) => env.value,
        Node::Neg(a) => -eval(a, env),
        Node::Bin(op, a, b) => {
            let (a, b) = (eval(a, env), eval(b, env));
            match op {
                Bin::Add => a + b,
                Bin::Sub => a - b,
                Bin::Mul => a * b,
                Bin::Div => a / b,
                Bin::Rem => a % b,
                Bin::Pow => a.powf(b),
            }
        }
        Node::Call(f, args) => {
            let x = arg(args, 0, env);
            match f {
                Func::Sin => x.to_radians().sin(),
                Func::Cos => x.to_radians().cos(),
                Func::Tan => x.to_radians().tan(),
                Func::Asin => x.asin().to_degrees(),
                Func::Acos => x.acos().to_degrees(),
                Func::Atan => x.atan().to_degrees(),
                Func::Atan2 => x.atan2(arg(args, 1, env)).to_degrees(),
                Func::Abs => x.abs(),
                Func::Floor => x.floor(),
                Func::Ceil => x.ceil(),
                Func::Round => x.round(),
                Func::Sqrt => x.sqrt(),
                Func::Sign => {
                    if x == 0.0 || x.is_nan() {
                        x
                    } else {
                        x.signum()
                    }
                }
                Func::Exp => x.exp(),
                Func::Ln => x.ln(),
                Func::Min => args.iter().map(|a| eval(a, env)).fold(f64::INFINITY, f64::min),
                Func::Max => args.iter().map(|a| eval(a, env)).fold(f64::NEG_INFINITY, f64::max),
                Func::Mod => x.rem_euclid(arg(args, 1, env)),
                Func::Clamp => {
                    let (lo, hi) = (arg(args, 1, env), arg(args, 2, env));
                    if lo.is_nan() || hi.is_nan() || lo > hi { f64::NAN } else { x.clamp(lo, hi) }
                }
                Func::Lerp => {
                    let (b, t) = (arg(args, 1, env), arg(args, 2, env));
                    x + (b - x) * t
                }
                Func::Rad => x.to_radians(),
                Func::Deg => x.to_degrees(),
                Func::Rand => {
                    if !x.is_finite() {
                        return f64::NAN;
                    }
                    // `+ 0.0` makes -0 and 0 the same.
                    let bits = crate::splitmix64((x + 0.0).to_bits() ^ 0x5049_4341_4249_4121);
                    // The top 53 bits as a fraction in 0..1.
                    (bits >> 11) as f64 / (1u64 << 53) as f64
                }
            }
        }
    }
}

/// The expression text driving `field` of `kind` (`"rotate"` → its `rotateExpr`), if any: what the
/// panel shows as the field's expression chip.
pub fn field_expr(kind: &crate::ModifierKind, field: &str) -> Option<String> {
    let params = kind.params();
    params.get(&crate::expr_key(field))?.as_str().map(str::to_string)
}

/// Parse a stored expression for evaluation; one that doesn't parse (a hand-edited file) is
/// ignored with a warning, and the field's own value is used.
pub(crate) fn compile(text: Option<&str>, key: &str, warnings: &mut Vec<String>) -> Option<Expr> {
    let text = text?;
    match Expr::parse(text) {
        Ok(e) => Some(e),
        Err(e) => {
            warnings.push(format!("`{key}` is ignored: {e}"));
            None
        }
    }
}
