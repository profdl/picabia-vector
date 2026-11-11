//! PICABIA: a [`LaserSheet`] as GRBL 1.1 G-code for a **drag knife** held on the Z axis.
//!
//! The machine is the laser's (same bed, origin and coordinates as
//! [`laser_gcode`](crate::laser_gcode)); the tool is a swivel blade in the Z clamp. Work Z 0 is
//! the sheet's surface: the person lowers the blade until it touches the sheet at its bottom-left
//! and presses Set origin, which zeroes X, Y and Z.
//!
//! - The same `;` comment header (ASCII, at most 70 characters a line), then `G21`, `G90`, `M5`
//!   and a lift to the travel height (`G0 Z…`) before any move across the sheet.
//! - Score lines (shallow), then the cut in the sheet's planned order (inner contours first, the
//!   outline last). Engraving is left out with a warning: a blade can't engrave.
//! - Per path and per pass: `G0` to its start at the travel height, `G1 Z…` down at the plunge
//!   feed, the cut, `G0 Z…` back up. Closed contours run on past their start by the overcut, so
//!   the blade's first, still-turning millimetres are cut again.
//! - **Blade offset.** A drag knife's tip trails its axis by the offset: moving along `d`, the tip
//!   is at `axis − offset·d`. So the axis runs `offset` ahead of every point of the path, and at a
//!   corner that turns more than the swivel angle it swings round the corner on an arc of radius
//!   `offset` (`G2`/`G3`), which turns the blade on the spot; gentler corners get a straight
//!   chord. Offset 0: the path as it is (a fixed blade, a pen).
//! - **No spindle, no beam.** Nothing here turns the spindle output on: no `M3`, `M4` or `S`, so a
//!   laser left wired to the spindle output stays off. `M5` before the first move and at the end.
//! - The end: up to the travel height, `G0` to the origin (park), `M5`.
//!
//! The only words written are `G0 G1 G2 G3 G21 G90 M5 F X Y Z I J`.

use serde_json::{Map, Value};
use vectorcraft_geom::kurbo::{Point, Rect, Vec2};

use crate::laser::LaserSheet;
use crate::laser_gcode::{Gcode, GcodeParams, MAX_GCODE_LINES, MAX_HEADER_LINES, comment, q, valid};
use crate::laser_svg::num;

/// What is in the machine's tool clamp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Head {
    /// A laser on the spindle output, in laser mode: [`laser_gcode`](crate::laser_gcode).
    #[default]
    Laser,
    /// A drag knife on the Z axis: [`knife_gcode`].
    Knife,
}

impl Head {
    /// `laser` or `knife`.
    pub fn id(self) -> &'static str {
        match self {
            Head::Laser => "laser",
            Head::Knife => "knife",
        }
    }

    /// `laser` or `knife` (any case, trimmed); anything else is `None`.
    pub fn from_id(s: &str) -> Option<Head> {
        match s.trim().to_ascii_lowercase().as_str() {
            "laser" => Some(Head::Laser),
            "knife" => Some(Head::Knife),
            _ => None,
        }
    }
}

/// The drag knife's settings. Work Z 0 is the sheet's surface; Z up is positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnifeParams {
    /// Height the blade travels at between paths, mm above the surface, 0.5–50.
    pub safe_z_mm: f64,
    /// Z while cutting, mm (0 the surface, negative into it), −10–0.
    pub cut_z_mm: f64,
    /// Score lines are cut (shallow); off: left out.
    pub score: bool,
    /// Z while scoring, mm, −10–0.
    pub score_z_mm: f64,
    /// Feed while the blade is down, mm/min, 1–20000.
    pub feed_mm_min: f64,
    /// Feed going down into the sheet, mm/min, 1–5000.
    pub plunge_mm_min: f64,
    /// Times each cut path is run, 1–20 (score lines once).
    pub passes: u32,
    /// How far the blade's tip trails its axis, mm, 0–2 (0: no compensation).
    pub offset_mm: f64,
    /// Corners turning by more than this get a swivel arc, degrees, 0–180.
    pub swivel_deg: f64,
    /// How far a closed contour is cut again past its start, mm, 0–10.
    pub overcut_mm: f64,
}

impl KnifeParams {
    pub const MIN_SAFE_Z: f64 = 0.5;
    pub const MAX_SAFE_Z: f64 = 50.0;
    pub const MIN_DEPTH_Z: f64 = -10.0;
    pub const MAX_FEED: f64 = 20_000.0;
    pub const MAX_PLUNGE: f64 = 5_000.0;
    pub const MAX_PASSES: u32 = 20;
    pub const MAX_OFFSET: f64 = 2.0;
    pub const MAX_OVERCUT: f64 = 10.0;

    /// Every number in its range (a non-finite one is the default).
    pub fn clamped(&self) -> KnifeParams {
        let d = KnifeParams::default();
        KnifeParams {
            safe_z_mm: clamp_or(self.safe_z_mm, Self::MIN_SAFE_Z, Self::MAX_SAFE_Z, d.safe_z_mm),
            cut_z_mm: clamp_or(self.cut_z_mm, Self::MIN_DEPTH_Z, 0.0, d.cut_z_mm),
            score: self.score,
            score_z_mm: clamp_or(self.score_z_mm, Self::MIN_DEPTH_Z, 0.0, d.score_z_mm),
            feed_mm_min: clamp_or(self.feed_mm_min, 1.0, Self::MAX_FEED, d.feed_mm_min),
            plunge_mm_min: clamp_or(self.plunge_mm_min, 1.0, Self::MAX_PLUNGE, d.plunge_mm_min),
            passes: self.passes.clamp(1, Self::MAX_PASSES),
            offset_mm: clamp_or(self.offset_mm, 0.0, Self::MAX_OFFSET, d.offset_mm),
            swivel_deg: clamp_or(self.swivel_deg, 0.0, 180.0, d.swivel_deg),
            overcut_mm: clamp_or(self.overcut_mm, 0.0, Self::MAX_OVERCUT, d.overcut_mm),
        }
    }

    /// Overwrite the fields `v` sets validly (camelCase: `safeZMm`, `cutZMm`, `score`,
    /// `scoreZMm`, `feedMmMin`, `plungeMmMin`, `passes`, `offsetMm`, `swivelDeg`, `overcutMm`);
    /// the rest stay, numbers are clamped.
    pub fn apply_json(&mut self, v: &Value) {
        let Some(o) = v.as_object() else { return };
        let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
        for (k, f) in [
            ("safeZMm", &mut self.safe_z_mm),
            ("cutZMm", &mut self.cut_z_mm),
            ("scoreZMm", &mut self.score_z_mm),
            ("feedMmMin", &mut self.feed_mm_min),
            ("plungeMmMin", &mut self.plunge_mm_min),
            ("offsetMm", &mut self.offset_mm),
            ("swivelDeg", &mut self.swivel_deg),
            ("overcutMm", &mut self.overcut_mm),
        ] {
            if let Some(x) = num(k) {
                *f = x;
            }
        }
        if let Some(b) = o.get("score").and_then(Value::as_bool) {
            self.score = b;
        }
        if let Some(x) = num("passes") {
            self.passes = x.round().clamp(1.0, f64::from(Self::MAX_PASSES)) as u32;
        }
        *self = self.clamped();
    }

    /// The settings as JSON (the [`apply_json`](Self::apply_json) names), clamped.
    pub fn to_json(&self) -> Value {
        let k = self.clamped();
        let mut o = Map::new();
        o.insert("safeZMm".into(), Value::from(k.safe_z_mm));
        o.insert("cutZMm".into(), Value::from(k.cut_z_mm));
        o.insert("score".into(), Value::from(k.score));
        o.insert("scoreZMm".into(), Value::from(k.score_z_mm));
        o.insert("feedMmMin".into(), Value::from(k.feed_mm_min));
        o.insert("plungeMmMin".into(), Value::from(k.plunge_mm_min));
        o.insert("passes".into(), Value::from(k.passes));
        o.insert("offsetMm".into(), Value::from(k.offset_mm));
        o.insert("swivelDeg".into(), Value::from(k.swivel_deg));
        o.insert("overcutMm".into(), Value::from(k.overcut_mm));
        Value::Object(o)
    }
}

impl Default for KnifeParams {
    /// **Guesses** for a 0.25 mm-offset swivel blade in paper until cut on the machine: travel at
    /// 5 mm, cut at −0.5 (the holder's spring takes the rest), score at −0.2, 1500 mm/min,
    /// plunge 300, 1 pass, swivel over 15°, overcut 1 mm.
    fn default() -> Self {
        KnifeParams {
            safe_z_mm: 5.0,
            cut_z_mm: -0.5,
            score: true,
            score_z_mm: -0.2,
            feed_mm_min: 1500.0,
            plunge_mm_min: 300.0,
            passes: 1,
            offset_mm: 0.25,
            swivel_deg: 15.0,
            overcut_mm: 1.0,
        }
    }
}

fn clamp_or(v: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { default }
}

/// One move of the knife's axis with the blade down.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Line(Point),
    /// Round `centre` to `to`, counter-clockwise (`G3`) or clockwise (`G2`), Y up.
    Arc {
        to: Point,
        centre: Point,
        ccw: bool,
    },
}

/// Segments shorter than this are dropped (mm).
const MIN_SEG: f64 = 1e-6;

/// The axis path that drags the tip along `tip` (work coordinates): where it starts and its moves.
/// `None` when `tip` has no length.
fn axis_path(tip: &[Point], offset: f64, cos_swivel: f64) -> Option<(Point, Vec<Step>)> {
    let segs: Vec<(Point, Vec2)> = tip
        .windows(2)
        .filter_map(|w| {
            let &[a, b] = w else { return None };
            let v = b - a;
            let len = v.hypot();
            (len > MIN_SEG).then(|| (b, v / len))
        })
        .collect();
    let (&(_, d0), _) = segs.split_first()?;
    let start = *tip.first()? + d0 * offset;
    let mut steps = Vec::with_capacity(segs.len() * 2);
    for (i, &(b, d)) in segs.iter().enumerate() {
        steps.push(Step::Line(b + d * offset));
        let Some(&(_, d2)) = segs.get(i + 1) else { continue };
        if offset <= 0.0 {
            continue;
        }
        let to = b + d2 * offset;
        if d.dot(d2) < cos_swivel {
            steps.push(Step::Arc { to, centre: b, ccw: d.cross(d2) > 0.0 });
        } else {
            steps.push(Step::Line(to));
        }
    }
    Some((start, steps))
}

/// `ring` closed and run on past its start for `overcut` mm along itself.
fn closed_with_overcut(ring: &[Point], overcut: f64) -> Vec<Point> {
    let mut out = Vec::with_capacity(ring.len() * 2 + 1);
    out.extend_from_slice(ring);
    let Some(&first) = ring.first() else { return out };
    out.push(first);
    let mut left = overcut;
    let mut at = first;
    for &next in ring.iter().skip(1).chain(std::iter::once(&first)) {
        if left <= MIN_SEG {
            break;
        }
        let len = (next - at).hypot();
        if len <= MIN_SEG {
            continue;
        }
        if len >= left {
            out.push(at + (next - at) * (left / len));
            break;
        }
        out.push(next);
        left -= len;
        at = next;
    }
    out
}

struct Writer {
    text: String,
    lines: usize,
    head: Option<Point>,
    feed: Option<f64>,
    bounds: Option<Rect>,
    cut_mm: f64,
    travel_mm: f64,
    cut_sec: f64,
}

impl Writer {
    fn line(&mut self, s: &str) {
        self.text.push_str(s);
        self.text.push('\n');
        self.lines += 1;
    }

    fn mark(&mut self, r: Rect) {
        self.bounds = Some(self.bounds.map_or(r, |b| b.union(r)));
    }

    /// ` F…` when the modal feed isn't `f` already.
    fn feed_word(&mut self, f: f64) -> String {
        if self.feed == Some(f) {
            return String::new();
        }
        self.feed = Some(f);
        format!(" F{}", num(f))
    }

    /// `G0` across to `t` (rounded) at the travel height, unless the head is there.
    fn travel(&mut self, t: Point, origin: Point) {
        let from = self.head.unwrap_or(origin);
        if self.head == Some(t) {
            return;
        }
        self.line(&format!("G0 X{} Y{}", num(t.x), num(t.y)));
        self.travel_mm += (t - from).hypot();
        self.head = Some(t);
        self.mark(Rect::from_points(t, t));
    }

    fn z(&mut self, z: f64, rapid: bool, feed: f64) {
        if rapid {
            self.line(&format!("G0 Z{}", num(z)));
        } else {
            let f = self.feed_word(feed);
            self.line(&format!("G1 Z{}{f}", num(z)));
        }
    }

    fn step(&mut self, s: Step, feed: f64) {
        let Some(from) = self.head else { return };
        match s {
            Step::Line(to) => {
                let to = round(to);
                if to == from {
                    return;
                }
                let f = self.feed_word(feed);
                self.line(&format!("G1 X{} Y{}{f}", num(to.x), num(to.y)));
                let len = (to - from).hypot();
                self.cut_mm += len;
                self.cut_sec += len / feed * 60.0;
                self.head = Some(to);
                self.mark(Rect::from_points(to, to));
            }
            Step::Arc { to, centre, ccw } => {
                let to = round(to);
                let (i, j) = (q(centre.x - from.x), q(centre.y - from.y));
                let r = i.hypot(j);
                // Rounded onto its start, an arc would be a whole circle to GRBL: a chord instead.
                if to == from || r < 0.002 {
                    return self.step(Step::Line(to), feed);
                }
                let f = self.feed_word(feed);
                let g = if ccw { 3 } else { 2 };
                self.line(&format!("G{g} X{} Y{} I{} J{}{f}", num(to.x), num(to.y), num(i), num(j)));
                let c = Point::new(from.x + i, from.y + j);
                let cos = ((from - c).dot(to - c) / (r * r)).clamp(-1.0, 1.0);
                let len = r * cos.acos();
                self.cut_mm += len;
                self.cut_sec += len / feed * 60.0;
                self.head = Some(to);
                // The whole circle: never smaller than the arc, as the link's own box counts it.
                self.mark(Rect::new(c.x - r, c.y - r, c.x + r, c.y + r));
            }
        }
    }
}

fn round(p: Point) -> Point {
    Point::new(q(p.x), q(p.y))
}

/// One run of the blade at one depth.
struct Run {
    z: f64,
    passes: u32,
    closed: bool,
    /// Work coordinates, rounded, without repeated points.
    paths: Vec<Vec<Point>>,
}

/// `sheet` as drag-knife G-code with `params` (clamped first; `params.knife`, the origin and the
/// header are used, the laser's operations and `S` max aren't). See the module docs.
pub fn knife_gcode(sheet: &LaserSheet, params: &GcodeParams) -> Gcode {
    let p = params.clamped();
    let k = p.knife;
    let (w, h) = (sheet.w_mm, sheet.h_mm);
    if !(w.is_finite() && h.is_finite() && w >= 0.0 && h >= 0.0 && w <= 20_000.0 && h <= 20_000.0) {
        return Gcode { warnings: vec!["the sheet has no valid size".into()], ..Gcode::default() };
    }
    let (ox, oy) = (p.origin_mm.x, p.origin_mm.y);
    let work = |pt: Point| round(Point::new(ox + pt.x, oy + (h - pt.y)));
    let mut warnings = Vec::new();
    let mut invalid = 0usize;
    let mut clean = |paths: &[Vec<Point>], closed: bool| -> Vec<Vec<Point>> {
        paths
            .iter()
            .filter_map(|path| {
                let mut pts: Vec<Point> = Vec::with_capacity(path.len());
                for &pt in path {
                    if !valid(pt) {
                        invalid += 1;
                        continue;
                    }
                    let t = work(pt);
                    if pts.last() != Some(&t) {
                        pts.push(t);
                    }
                }
                if closed && pts.len() > 1 && pts.first() == pts.last() {
                    pts.pop();
                }
                (pts.len() >= 2).then_some(pts)
            })
            .collect()
    };
    let runs: Vec<Run> = [
        Run { z: k.score_z_mm, passes: 1, closed: false, paths: if k.score { clean(&sheet.score, false) } else { Vec::new() } },
        Run { z: k.cut_z_mm, passes: k.passes, closed: true, paths: clean(&sheet.cut, true) },
    ]
    .into_iter()
    .filter(|r| !r.paths.is_empty())
    .collect();
    if invalid > 0 {
        warnings.push(if invalid == 1 {
            "1 point with invalid coordinates was left out".to_string()
        } else {
            format!("{invalid} points with invalid coordinates were left out")
        });
    }
    let engraved = sheet.engrave.iter().filter(|e| e.len() >= 2).count();
    if engraved > 0 {
        warnings.push(format!("the knife doesn't engrave: {engraved} engraved paths (the label) were left out"));
    }
    if runs.is_empty() {
        warnings.push("nothing to cut: the sheet has no cut or score lines".into());
        return Gcode { warnings, ..Gcode::default() };
    }
    // Per pass: travel, plunge, up to 2 moves a point (a corner's arc or chord), the overcut, lift.
    let header: Vec<String> = p.header.iter().take(MAX_HEADER_LINES).cloned().collect();
    let mut bound: usize = header.len() + 24;
    for r in &runs {
        for path in &r.paths {
            let per = path.len().saturating_mul(4).saturating_add(8);
            bound = bound.saturating_add(per.saturating_mul(r.passes as usize));
        }
    }
    if bound > MAX_GCODE_LINES {
        warnings.push(format!("the G-code would take more than {MAX_GCODE_LINES} lines; simplify the design or split the sheet"));
        return Gcode { warnings, ..Gcode::default() };
    }

    let origin = round(Point::new(ox, oy));
    let cos_swivel = k.swivel_deg.to_radians().cos();
    let mut wr = Writer {
        text: String::with_capacity(bound.saturating_mul(24).min(64 << 20)),
        lines: 0,
        head: None,
        feed: None,
        bounds: None,
        cut_mm: 0.0,
        travel_mm: 0.0,
        cut_sec: 0.0,
    };
    wr.line(&comment("Picabia drag knife G-code (GRBL 1.1)"));
    wr.line(&comment(&format!("sheet: {}", sheet.name)));
    wr.line(&comment(&format!("size: {} x {} mm", num(w), num(h))));
    wr.line(&comment(&format!("origin: X{} Y{} is the sheet's bottom-left; mm, Y up", num(ox), num(oy))));
    wr.line(&comment(&format!("Z0 is the sheet's surface; travel at Z{}", num(k.safe_z_mm))));
    let passes = if k.passes == 1 { "1 pass".to_string() } else { format!("{} passes", k.passes) };
    wr.line(&comment(&format!("cut: Z{}, {} mm/min, {passes}; plunge {} mm/min", num(k.cut_z_mm), num(k.feed_mm_min), num(k.plunge_mm_min))));
    wr.line(&comment(&if k.score { format!("score: Z{}, 1 pass", num(k.score_z_mm)) } else { "score: off".into() }));
    wr.line(&comment(&format!("blade offset {} mm, swivel over {} deg, overcut {} mm", num(k.offset_mm), num(k.swivel_deg), num(k.overcut_mm))));
    wr.line(&comment("no spindle or laser: M5 only"));
    for l in &header {
        wr.line(&comment(l));
    }
    wr.line("G21");
    wr.line("G90");
    wr.line("M5");
    let safe = q(k.safe_z_mm);
    wr.z(safe, true, k.plunge_mm_min);
    let mut z_travel = 0.0;
    for r in &runs {
        let z = q(r.z);
        for path in &r.paths {
            let tip = if r.closed { closed_with_overcut(path, k.overcut_mm) } else { path.clone() };
            let Some((start, steps)) = axis_path(&tip, k.offset_mm, cos_swivel) else { continue };
            for _ in 0..r.passes {
                wr.travel(round(start), origin);
                wr.z(z, false, k.plunge_mm_min);
                wr.cut_sec += (safe - z).abs() / k.plunge_mm_min * 60.0;
                for &s in &steps {
                    wr.step(s, k.feed_mm_min);
                }
                wr.z(safe, true, k.plunge_mm_min);
                z_travel += (safe - z).abs();
            }
        }
    }
    // Park at the origin, at the travel height, written even when the last move ended there.
    if wr.head == Some(origin) {
        wr.head = None;
    }
    wr.travel(origin, origin);
    wr.line("M5");
    Gcode {
        text: wr.text,
        lines: wr.lines,
        warnings,
        bounds_mm: wr.bounds.unwrap_or(Rect::ZERO),
        burn_mm: wr.cut_mm,
        travel_mm: wr.travel_mm + z_travel,
        burn_sec: wr.cut_sec,
    }
}
