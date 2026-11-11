//! A [`LaserSheet`] as GRBL 1.1 G-code for a laser in laser mode (`$32=1`).
//!
//! The text is plain and short-lined so any GRBL sender (Picabia's own, LightBurn's Run GCode)
//! can stream it:
//!
//! - a `;` comment header (ASCII, at most 70 characters a line): the sheet, its size, the origin
//!   and every operation's settings;
//! - `G21` (mm), `G90` (absolute), `M5` (beam off), `M8` when air assist is on;
//! - the operations in the SVG's layer order, engrave → score → cut (parts don't shift before
//!   they're engraved); within cut the sheet's planned order is kept (inner contours first, the
//!   outline last). Each operation: `M4 S…` once (dynamic power: in laser mode `G0` never fires
//!   the beam), per path a `G0` to its start and `G1` moves (`F` on the operation's first `G1`,
//!   modal after), closed contours back to their start, every pass of a path in place before the
//!   next path, then `M5`;
//! - the end: `M5`, `M9` when air assist is on, `G0` to the origin (park), `M5`.
//!
//! Coordinates: the sheet is mm with its origin top-left and y down; the machine's is front-left
//! and y up, so `X = ox + x`, `Y = oy + (h − y)` with `(ox, oy)` where the sheet's bottom-left
//! sits in work coordinates. Numbers have at most 3 decimals and never an exponent. The only
//! words written are `G0 G1 G21 G90 M4 M5 M8 M9 S F X Y`.

use std::fmt::Write as _;

use serde_json::{Map, Value};
use vectorcraft_geom::kurbo::{Point, Rect};

use crate::knife_gcode::{Head, KnifeParams};
use crate::laser::LaserSheet;
use crate::laser_svg::num;

/// The most lines a G-code file may have; a sheet needing more is refused.
pub const MAX_GCODE_LINES: usize = 2_000_000;
/// The most caller header lines written (the rest are left out).
pub const MAX_HEADER_LINES: usize = 16;
/// The longest header comment line, characters (`; ` included).
const HEADER_CHARS: usize = 70;
/// Points farther than this from the sheet's origin are treated as invalid, mm.
const MAX_COORD_MM: f64 = 20_000.0;

/// One operation's settings (cut, score or engrave).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpSettings {
    /// Off: the operation is left out of the G-code.
    pub enabled: bool,
    /// Feed while burning, mm/min, 1–20000.
    pub speed_mm_min: f64,
    /// Beam power, % of the machine's `S` max, 0–100.
    pub power_pct: f64,
    /// Times each path is run, in place, before the next, 1–20.
    pub passes: u32,
}

impl OpSettings {
    pub const MIN_SPEED: f64 = 1.0;
    pub const MAX_SPEED: f64 = 20_000.0;
    pub const MAX_PASSES: u32 = 20;

    const fn new(speed_mm_min: f64, power_pct: f64, passes: u32) -> OpSettings {
        OpSettings { enabled: true, speed_mm_min, power_pct, passes }
    }

    /// Every number in its range; a non-finite one is `default`'s.
    pub fn clamped_or(&self, default: &OpSettings) -> OpSettings {
        OpSettings {
            enabled: self.enabled,
            speed_mm_min: clamp_or(self.speed_mm_min, Self::MIN_SPEED, Self::MAX_SPEED, default.speed_mm_min),
            power_pct: clamp_or(self.power_pct, 0.0, 100.0, default.power_pct),
            passes: self.passes.clamp(1, Self::MAX_PASSES),
        }
    }

    /// Overwrite the fields `v` (`{enabled, speedMmMin, powerPct, passes}`) sets validly.
    fn apply_json(&mut self, v: &Value) {
        let Some(o) = v.as_object() else { return };
        if let Some(b) = o.get("enabled").and_then(Value::as_bool) {
            self.enabled = b;
        }
        let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
        if let Some(x) = num("speedMmMin") {
            self.speed_mm_min = x;
        }
        if let Some(x) = num("powerPct") {
            self.power_pct = x;
        }
        if let Some(x) = num("passes") {
            self.passes = x.round().clamp(1.0, f64::from(Self::MAX_PASSES)) as u32;
        }
    }

    fn to_json(self) -> Value {
        let mut o = Map::new();
        o.insert("enabled".into(), Value::from(self.enabled));
        o.insert("speedMmMin".into(), Value::from(self.speed_mm_min));
        o.insert("powerPct".into(), Value::from(self.power_pct));
        o.insert("passes".into(), Value::from(self.passes));
        Value::Object(o)
    }
}

/// G-code settings: the machine's `S` max, the three operations, where the sheet sits, air assist
/// and extra header comment lines.
#[derive(Clone, Debug, PartialEq)]
pub struct GcodeParams {
    /// The `S` value for full power (GRBL's `$30`), 1–100000.
    pub s_max: f64,
    pub cut: OpSettings,
    pub score: OpSettings,
    pub engrave: OpSettings,
    /// Where the sheet's bottom-left sits in work coordinates, mm, ±5000 each way.
    pub origin_mm: Point,
    /// Air assist: `M8` before the job, `M9` after.
    pub air: bool,
    /// Extra comment lines for the header (made ASCII and cut to fit; at most
    /// [`MAX_HEADER_LINES`]).
    pub header: Vec<String>,
    /// PICABIA: what is in the tool clamp. [`laser_gcode`] writes the laser's G-code whatever
    /// this says; [`knife_gcode`](crate::knife_gcode) the knife's.
    pub head: Head,
    /// PICABIA: the drag knife's settings (used when `head` is the knife).
    pub knife: KnifeParams,
}

impl GcodeParams {
    pub const DEFAULT_S_MAX: f64 = 1000.0;
    pub const MAX_S_MAX: f64 = 100_000.0;
    pub const MAX_ORIGIN_MM: f64 = 5000.0;

    /// Paper. **Guesses** for a 2.8 W blue diode until the test card and the power ladder are
    /// cut on the machine: cut 300 mm/min, 100 %, 2 passes; score 1500, 30 %, 1; engrave 2000,
    /// 20 %, 1.
    const PAPER: [OpSettings; 3] = [OpSettings::new(300.0, 100.0, 2), OpSettings::new(1500.0, 30.0, 1), OpSettings::new(2000.0, 20.0, 1)];
    /// Film (polyester stencil film). **Guesses**: clear film takes a blue beam poorly, so slower
    /// and more passes than paper: cut 200, 100 %, 3; score 1000, 25 %, 1; engrave 1500, 15 %, 1.
    const FILM: [OpSettings; 3] = [OpSettings::new(200.0, 100.0, 3), OpSettings::new(1000.0, 25.0, 1), OpSettings::new(1500.0, 15.0, 1)];
    /// Card. **Guesses**: thicker than paper: cut 150, 100 %, 3; score 1000, 40 %, 1; engrave
    /// 1500, 25 %, 1.
    const CARD: [OpSettings; 3] = [OpSettings::new(150.0, 100.0, 3), OpSettings::new(1000.0, 40.0, 1), OpSettings::new(1500.0, 25.0, 1)];

    fn with_ops(ops: [OpSettings; 3]) -> GcodeParams {
        let [cut, score, engrave] = ops;
        GcodeParams {
            s_max: Self::DEFAULT_S_MAX,
            cut,
            score,
            engrave,
            origin_mm: Point::ZERO,
            air: false,
            header: Vec::new(),
            head: Head::Laser,
            knife: KnifeParams::default(),
        }
    }

    /// The starting settings for a stencil material (`paper`, `film`, `card`; `custom` and
    /// anything else get paper's). Every value is a guess for a 2.8 W diode until measured.
    pub fn for_material(id: &str) -> GcodeParams {
        match id.trim().to_ascii_lowercase().as_str() {
            "film" => Self::with_ops(Self::FILM),
            "card" => Self::with_ops(Self::CARD),
            _ => Self::with_ops(Self::PAPER),
        }
    }

    /// The settings with every number in its range (a non-finite one is the default).
    pub fn clamped(&self) -> GcodeParams {
        let d = GcodeParams::default();
        let origin = |v: f64| clamp_or(v, -Self::MAX_ORIGIN_MM, Self::MAX_ORIGIN_MM, 0.0);
        GcodeParams {
            s_max: clamp_or(self.s_max, 1.0, Self::MAX_S_MAX, d.s_max),
            cut: self.cut.clamped_or(&d.cut),
            score: self.score.clamped_or(&d.score),
            engrave: self.engrave.clamped_or(&d.engrave),
            origin_mm: Point::new(origin(self.origin_mm.x), origin(self.origin_mm.y)),
            air: self.air,
            header: self.header.clone(),
            head: self.head,
            knife: self.knife.clamped(),
        }
    }

    /// Settings from a JSON object, leniently (see [`apply_json`](Self::apply_json)).
    pub fn from_json(v: &Value) -> GcodeParams {
        let mut p = GcodeParams::default();
        p.apply_json(v);
        p
    }

    /// Overwrite the fields `v` sets validly (camelCase: `sMax`, `originXMm`, `originYMm`, `air`,
    /// `cut`/`score`/`engrave`: `{enabled, speedMmMin, powerPct, passes}` overlaid field by field,
    /// `header`: an array of strings); missing or wrong-typed fields keep their value, numbers are
    /// clamped. PICABIA: `head` (`laser`|`knife`) and `knife` (see
    /// [`KnifeParams::apply_json`]) too.
    pub fn apply_json(&mut self, v: &Value) {
        let Some(o) = v.as_object() else { return };
        let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
        if let Some(x) = num("sMax") {
            self.s_max = x;
        }
        if let Some(x) = num("originXMm") {
            self.origin_mm.x = x;
        }
        if let Some(x) = num("originYMm") {
            self.origin_mm.y = x;
        }
        if let Some(b) = o.get("air").and_then(Value::as_bool) {
            self.air = b;
        }
        for (k, op) in [("cut", &mut self.cut), ("score", &mut self.score), ("engrave", &mut self.engrave)] {
            if let Some(v) = o.get(k) {
                op.apply_json(v);
            }
        }
        if let Some(h) = o.get("head").and_then(Value::as_str).and_then(Head::from_id) {
            self.head = h;
        }
        if let Some(k) = o.get("knife") {
            self.knife.apply_json(k);
        }
        if let Some(a) = o.get("header").and_then(Value::as_array) {
            self.header = a.iter().filter_map(Value::as_str).take(MAX_HEADER_LINES).map(str::to_string).collect();
        }
        *self = self.clamped();
    }

    /// The settings as JSON (the [`apply_json`](Self::apply_json) names), clamped. The header is
    /// not written: it belongs to one export, not to the remembered settings.
    pub fn to_json(&self) -> Value {
        let p = self.clamped();
        let mut o = Map::new();
        o.insert("sMax".into(), Value::from(p.s_max));
        o.insert("originXMm".into(), Value::from(p.origin_mm.x));
        o.insert("originYMm".into(), Value::from(p.origin_mm.y));
        o.insert("air".into(), Value::from(p.air));
        o.insert("cut".into(), p.cut.to_json());
        o.insert("score".into(), p.score.to_json());
        o.insert("engrave".into(), p.engrave.to_json());
        o.insert("head".into(), Value::from(p.head.id()));
        o.insert("knife".into(), p.knife.to_json());
        Value::Object(o)
    }
}

impl Default for GcodeParams {
    fn default() -> Self {
        Self::with_ops(Self::PAPER)
    }
}

fn clamp_or(v: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { default }
}

/// G-code for one sheet and what to know about it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gcode {
    /// The file, LF line endings. Empty means refused (the warnings say why).
    pub text: String,
    /// Lines in `text`, comments included.
    pub lines: usize,
    pub warnings: Vec<String>,
    /// Work coordinates of every `G0`/`G1` target, the park included, mm.
    pub bounds_mm: Rect,
    /// Beam-on (`G1`) length, every pass, mm.
    pub burn_mm: f64,
    /// Beam-off (`G0`) length, from and back to the origin, mm.
    pub travel_mm: f64,
    /// Σ length / feed over every `G1`, seconds.
    pub burn_sec: f64,
}

impl Gcode {
    fn refused(why: impl Into<String>) -> Gcode {
        Gcode { warnings: vec![why.into()], ..Gcode::default() }
    }
}

/// How long `g` takes, seconds: its burn time plus its travel at `rapid_mm_min` (at least 1).
/// Acceleration is ignored, so short segments run longer than this.
pub fn estimate_sec(g: &Gcode, rapid_mm_min: f64) -> f64 {
    let rapid = if rapid_mm_min.is_nan() { 1.0 } else { rapid_mm_min.max(1.0) };
    g.burn_sec + g.travel_mm / rapid * 60.0
}

/// One run of the beam at one setting over some paths.
pub(crate) struct Block<'a> {
    pub op: OpSettings,
    pub paths: Vec<&'a [Point]>,
    /// Closed contours return to their start.
    pub closed: bool,
}

/// `sheet` as G-code with `params` (clamped first). See the module docs.
pub fn laser_gcode(sheet: &LaserSheet, params: &GcodeParams) -> Gcode {
    let p = params.clamped();
    let blocks = [
        Block { op: p.engrave, paths: slices(&sheet.engrave), closed: false },
        Block { op: p.score, paths: slices(&sheet.score), closed: false },
        Block { op: p.cut, paths: slices(&sheet.cut), closed: true },
    ];
    let mut info = vec![format!("S max {}, M4 dynamic power", num(p.s_max))];
    for (name, op) in [("engrave", p.engrave), ("score", p.score), ("cut", p.cut)] {
        info.push(op_line(name, op, p.s_max));
    }
    write(sheet, &blocks, &p, &info)
}

fn slices(v: &[Vec<Point>]) -> Vec<&[Point]> {
    v.iter().map(Vec::as_slice).collect()
}

/// The header line for an operation.
pub(crate) fn op_line(name: &str, op: OpSettings, s_max: f64) -> String {
    if op.enabled {
        let passes = if op.passes == 1 { "1 pass".to_string() } else { format!("{} passes", op.passes) };
        format!("{name}: {} mm/min, {}% (S{}), {passes}", num(op.speed_mm_min), num(op.power_pct), s_value(op.power_pct, s_max))
    } else {
        format!("{name}: off")
    }
}

/// `S` for `power_pct` of `s_max`, rounded.
pub(crate) fn s_value(power_pct: f64, s_max: f64) -> u64 {
    let s = (power_pct / 100.0 * s_max).round();
    if s.is_finite() && s > 0.0 { s as u64 } else { 0 }
}

/// A header comment: printable ASCII only (anything else becomes `?`), at most 70 characters.
pub(crate) fn comment(s: &str) -> String {
    let body: String = s.chars().map(|c| if c == ' ' || c.is_ascii_graphic() { c } else { '?' }).take(HEADER_CHARS - 2).collect();
    let body = body.trim_end();
    if body.is_empty() { ";".into() } else { format!("; {body}") }
}

/// Round to the 3 decimals the file carries (no `-0`).
pub(crate) fn q(v: f64) -> f64 {
    let r = (v * 1000.0).round() / 1000.0;
    if r == 0.0 { 0.0 } else { r }
}

pub(crate) fn valid(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.x.abs() <= MAX_COORD_MM && p.y.abs() <= MAX_COORD_MM
}

/// An upper bound on the motion and power lines `blocks` take.
fn line_bound(blocks: &[Block]) -> usize {
    let mut n: usize = 0;
    for b in blocks.iter().filter(|b| b.op.enabled) {
        let passes = b.op.passes as usize;
        n = n.saturating_add(2);
        for path in &b.paths {
            let per = path.len().saturating_add(usize::from(b.closed));
            let back = if b.closed { 0 } else { passes.saturating_sub(1) };
            n = n.saturating_add(1).saturating_add(per.saturating_mul(passes)).saturating_add(back);
        }
    }
    n
}

struct Writer {
    text: String,
    lines: usize,
    ox: f64,
    oy: f64,
    h: f64,
    /// The head's place in work coordinates, rounded; `None` before the first move.
    head: Option<Point>,
    bounds: Option<Rect>,
    burn_mm: f64,
    travel_mm: f64,
    burn_sec: f64,
}

impl Writer {
    fn line(&mut self, s: &str) {
        self.text.push_str(s);
        self.text.push('\n');
        self.lines += 1;
    }

    fn work(&self, p: Point) -> Point {
        Point::new(q(self.ox + p.x), q(self.oy + (self.h - p.y)))
    }

    fn mark(&mut self, t: Point) {
        self.bounds = Some(self.bounds.map_or(Rect::from_points(t, t), |b| b.union_pt(t)));
    }

    /// `G0` to `t` (work coordinates, rounded) unless the head is there already. The first move
    /// is always written: the head's start is unknown.
    fn rapid(&mut self, t: Point) {
        let from = self.head.unwrap_or(Point::new(q(self.ox), q(self.oy)));
        if self.head == Some(t) {
            return;
        }
        let mut s = String::with_capacity(32);
        let _ = write!(s, "G0 X{} Y{}", num(t.x), num(t.y));
        self.line(&s);
        self.travel_mm += (t - from).hypot();
        self.head = Some(t);
        self.mark(t);
    }

    /// `G1` to `t` at `feed` (`F` written while `f_due`), unless the head is there already.
    fn burn(&mut self, t: Point, feed: f64, f_due: &mut bool) {
        let Some(from) = self.head else { return };
        if from == t {
            return;
        }
        let mut s = String::with_capacity(40);
        let _ = write!(s, "G1 X{} Y{}", num(t.x), num(t.y));
        if *f_due {
            let _ = write!(s, " F{}", num(feed));
            *f_due = false;
        }
        self.line(&s);
        let len = (t - from).hypot();
        self.burn_mm += len;
        self.burn_sec += len / feed * 60.0;
        self.head = Some(t);
        self.mark(t);
    }
}

/// The G-code of `blocks` (in order) on `sheet`, `p` clamped, `info` the settings lines of the
/// header.
pub(crate) fn write(sheet: &LaserSheet, blocks: &[Block], p: &GcodeParams, info: &[String]) -> Gcode {
    let (w, h) = (sheet.w_mm, sheet.h_mm);
    if !(w.is_finite() && h.is_finite() && w >= 0.0 && h >= 0.0 && w <= MAX_COORD_MM && h <= MAX_COORD_MM) {
        return Gcode::refused("the sheet has no valid size");
    }
    let mut warnings = Vec::new();
    // Keep the valid points of every path; drop paths left with fewer than 2.
    let mut invalid = 0usize;
    let cleaned: Vec<(OpSettings, bool, Vec<Vec<Point>>)> = blocks
        .iter()
        .filter(|b| b.op.enabled)
        .map(|b| {
            let paths = b
                .paths
                .iter()
                .filter_map(|path| {
                    let pts: Vec<Point> = path.iter().copied().filter(|&pt| valid(pt)).collect();
                    invalid += path.len() - pts.len();
                    (pts.len() >= 2).then_some(pts)
                })
                .collect();
            (b.op, b.closed, paths)
        })
        .filter(|(_, _, paths): &(OpSettings, bool, Vec<Vec<Point>>)| !paths.is_empty())
        .collect();
    if invalid > 0 {
        warnings.push(if invalid == 1 {
            "1 point with invalid coordinates was left out".to_string()
        } else {
            format!("{invalid} points with invalid coordinates were left out")
        });
    }
    if cleaned.is_empty() {
        warnings.push("nothing to run: every operation is off or has nothing to draw".into());
        return Gcode { warnings, ..Gcode::default() };
    }
    let header: Vec<String> = p.header.iter().take(MAX_HEADER_LINES).cloned().collect();
    let bound_blocks: Vec<Block> =
        cleaned.iter().map(|(op, closed, paths)| Block { op: *op, paths: paths.iter().map(Vec::as_slice).collect(), closed: *closed }).collect();
    let estimate = line_bound(&bound_blocks).saturating_add(info.len() + header.len() + 16);
    if estimate > MAX_GCODE_LINES {
        warnings.push(format!("the G-code would take more than {MAX_GCODE_LINES} lines; simplify the design or split the sheet"));
        return Gcode { warnings, ..Gcode::default() };
    }

    let (ox, oy) = (p.origin_mm.x, p.origin_mm.y);
    let mut wr = Writer {
        text: String::with_capacity(estimate.saturating_mul(24).min(64 << 20)),
        lines: 0,
        ox,
        oy,
        h,
        head: None,
        bounds: None,
        burn_mm: 0.0,
        travel_mm: 0.0,
        burn_sec: 0.0,
    };
    wr.line(&comment("Picabia laser G-code (GRBL 1.1, laser mode $32=1)"));
    wr.line(&comment(&format!("sheet: {}", sheet.name)));
    wr.line(&comment(&format!("size: {} x {} mm", num(w), num(h))));
    wr.line(&comment(&format!("origin: X{} Y{} is the sheet's bottom-left; mm, Y up", num(ox), num(oy))));
    for l in info {
        wr.line(&comment(l));
    }
    if p.air {
        wr.line(&comment("air assist: M8 on, M9 off"));
    }
    for l in &header {
        wr.line(&comment(l));
    }
    wr.line("G21");
    wr.line("G90");
    wr.line("M5");
    if p.air {
        wr.line("M8");
    }
    for (op, closed, paths) in &cleaned {
        wr.line(&format!("M4 S{}", s_value(op.power_pct, p.s_max)));
        let feed = op.speed_mm_min;
        let mut f_due = true;
        for path in paths {
            let pts: Vec<Point> = path.iter().map(|&pt| wr.work(pt)).collect();
            let (Some(&first), Some(rest)) = (pts.first(), pts.get(1..)) else { continue };
            for _ in 0..op.passes {
                // A closed contour is back at its start after each pass; an open one returns.
                wr.rapid(first);
                for &t in rest {
                    wr.burn(t, feed, &mut f_due);
                }
                if *closed {
                    wr.burn(first, feed, &mut f_due);
                }
            }
        }
        wr.line("M5");
    }
    wr.line("M5");
    if p.air {
        wr.line("M9");
    }
    // Park at the origin, written even when the last move ended there.
    let park = Point::new(q(ox), q(oy));
    if wr.head == Some(park) {
        wr.head = None;
    }
    wr.rapid(park);
    wr.line("M5");
    Gcode {
        text: wr.text,
        lines: wr.lines,
        warnings,
        bounds_mm: wr.bounds.unwrap_or(Rect::ZERO),
        burn_mm: wr.burn_mm,
        travel_mm: wr.travel_mm,
        burn_sec: wr.burn_sec,
    }
}
