//! Laser export: a stencil's cut as laser jobs, in millimetres.
//!
//! [`laser_job`] takes the drawn cut ([`LaserInput`], layer units) and returns one or more
//! [`LaserSheet`]s ready for [`laser_svg`](crate::laser_svg):
//!
//! 1. Millimetres, the sheet's top-left at 0,0.
//! 2. The bed split ([`crate::laser_bed`]): when the sheet doesn't fit the bed either way round, a
//!    grid of overlapping pieces, split where the least cut crosses (or on the repeat cell's
//!    edges), with registration holes in each overlap band and score lines where a neighbour's
//!    edge lands.
//! 3. Each piece's paper (`piece − cut`), then the **kerf**: the paper grows by `kerf / 2` (round
//!    joins) — the same as shrinking every cut-out by `kerf / 2` while the outer outline grows by
//!    it, so the beam's centre runs half a kerf into the removed side everywhere. Cut-outs
//!    narrower than the kerf vanish (warning).
//! 4. Clean geometry: closed contours without repeated points, short segments merged, collinear
//!    points dropped, duplicate and empty contours removed.
//! 5. Cut order: deepest contours first, the sheet's outline last; within a depth nearest
//!    neighbour from the previous contour's start (from the top-left), each contour started at
//!    its vertex nearest the head.
//! 6. Pieces the cut frees on a sheet (paper not joined to its outline) are counted.
//! 7. The engraved label in the bottom margin.

use crate::ops::{BoolOp, StrokeJoin};
use serde_json::{Map, Value};
use vectorcraft_geom::kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use vectorcraft_modifiers::{hex_colour, parse_hex_colour};

use crate::index::SegIndex;
use crate::islands::{Tree, ring_area};
use crate::laser_font::{DESCENT, font_has, text_polylines};
use crate::{hoshi, laser_bed, rings};

/// A closed contour (the last point joins the first), mm.
pub type Contour = Vec<Point>;
/// An open polyline, mm.
pub type Polyline = Vec<Point>;

/// The most sheets a job is split into; a job needing more is refused (no sheets, a warning).
pub const MAX_SHEETS: usize = 64;
/// The largest sheet side, mm (10 m).
pub const MAX_SHEET_MM: f64 = 10_000.0;
/// The most contour points one sheet takes (beyond it the sheet is left out with a warning).
pub const MAX_SHEET_POINTS: usize = 4_000_000;
/// The most label lines drawn.
pub const MAX_LABEL_LINES: usize = 8;

/// Laser export settings (all mm).
#[derive(Clone, Debug, PartialEq)]
pub struct LaserParams {
    /// The beam's cut width, 0–2 (0: no compensation).
    pub kerf_mm: f64,
    /// The bed, 10–5000 each way; 0 = no limit that way.
    pub bed_w_mm: f64,
    pub bed_h_mm: f64,
    /// How far neighbouring sheets of a split job overlap, 0–200.
    pub overlap_mm: f64,
    /// Stroke colours of the three operations, `#rrggbb`.
    pub cut: String,
    pub engrave: String,
    pub score: String,
    /// Engrave the label (name, sheet number, "this side up").
    pub label: bool,
}

impl LaserParams {
    pub const DEFAULT_KERF_MM: f64 = 0.10;
    pub const DEFAULT_BED_W_MM: f64 = 600.0;
    pub const DEFAULT_BED_H_MM: f64 = 400.0;
    pub const DEFAULT_OVERLAP_MM: f64 = 10.0;
    pub const DEFAULT_CUT: &'static str = "#ff0000";
    pub const DEFAULT_ENGRAVE: &'static str = "#000000";
    pub const DEFAULT_SCORE: &'static str = "#0000ff";
    pub const MAX_KERF_MM: f64 = 2.0;
    pub const MIN_BED_MM: f64 = 10.0;
    pub const MAX_BED_MM: f64 = 5000.0;
    pub const MAX_OVERLAP_MM: f64 = 200.0;

    /// The settings with every number clamped to its range and every colour a valid `#rrggbb`
    /// (an invalid one is the default).
    pub fn clamped(&self) -> LaserParams {
        let d = LaserParams::default();
        LaserParams {
            kerf_mm: clamp_or(self.kerf_mm, 0.0, Self::MAX_KERF_MM, d.kerf_mm),
            bed_w_mm: bed(self.bed_w_mm),
            bed_h_mm: bed(self.bed_h_mm),
            overlap_mm: clamp_or(self.overlap_mm, 0.0, Self::MAX_OVERLAP_MM, d.overlap_mm),
            cut: colour_or(&self.cut, &d.cut),
            engrave: colour_or(&self.engrave, &d.engrave),
            score: colour_or(&self.score, &d.score),
            label: self.label,
        }
    }

    /// Settings from a JSON object (camelCase: `kerfMm`, `bedWidthMm`, `bedHeightMm`, `overlapMm`,
    /// `cutColour`, `engraveColour`, `scoreColour`, `label`), leniently: missing or wrong-typed
    /// fields keep the default, numbers are clamped, a bad colour keeps the default.
    pub fn from_json(v: &Value) -> LaserParams {
        let mut p = LaserParams::default();
        p.apply_json(v);
        p
    }

    /// Overwrite the fields `v` (a JSON object, as for [`from_json`](Self::from_json)) sets
    /// validly; the rest stay. For layering remembered settings, a material's kerf and command
    /// params.
    pub fn apply_json(&mut self, v: &Value) {
        let Some(o) = v.as_object() else { return };
        let num = |k: &str| o.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
        if let Some(x) = num("kerfMm") {
            self.kerf_mm = x;
        }
        if let Some(x) = num("bedWidthMm") {
            self.bed_w_mm = x;
        }
        if let Some(x) = num("bedHeightMm") {
            self.bed_h_mm = x;
        }
        if let Some(x) = num("overlapMm") {
            self.overlap_mm = x;
        }
        let colour = |k: &str| o.get(k).and_then(Value::as_str).and_then(parse_hex_colour).map(hex_colour);
        if let Some(c) = colour("cutColour") {
            self.cut = c;
        }
        if let Some(c) = colour("engraveColour") {
            self.engrave = c;
        }
        if let Some(c) = colour("scoreColour") {
            self.score = c;
        }
        if let Some(b) = o.get("label").and_then(Value::as_bool) {
            self.label = b;
        }
        *self = self.clamped();
    }

    /// The settings as JSON (the [`from_json`](Self::from_json) names), clamped.
    pub fn to_json(&self) -> Value {
        let p = self.clamped();
        let mut o = Map::new();
        o.insert("kerfMm".into(), Value::from(p.kerf_mm));
        o.insert("bedWidthMm".into(), Value::from(p.bed_w_mm));
        o.insert("bedHeightMm".into(), Value::from(p.bed_h_mm));
        o.insert("overlapMm".into(), Value::from(p.overlap_mm));
        o.insert("cutColour".into(), Value::from(p.cut));
        o.insert("engraveColour".into(), Value::from(p.engrave));
        o.insert("scoreColour".into(), Value::from(p.score));
        o.insert("label".into(), Value::from(p.label));
        Value::Object(o)
    }
}

impl Default for LaserParams {
    fn default() -> Self {
        LaserParams {
            kerf_mm: Self::DEFAULT_KERF_MM,
            bed_w_mm: Self::DEFAULT_BED_W_MM,
            bed_h_mm: Self::DEFAULT_BED_H_MM,
            overlap_mm: Self::DEFAULT_OVERLAP_MM,
            cut: Self::DEFAULT_CUT.into(),
            engrave: Self::DEFAULT_ENGRAVE.into(),
            score: Self::DEFAULT_SCORE.into(),
            label: true,
        }
    }
}

fn clamp_or(v: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { default }
}

/// A bed side: 0 (or less, or not a number) = no limit, else 10–5000.
fn bed(v: f64) -> f64 {
    if v.is_finite() && v > 0.0 { v.clamp(LaserParams::MIN_BED_MM, LaserParams::MAX_BED_MM) } else { 0.0 }
}

fn colour_or(c: &str, default: &str) -> String {
    parse_hex_colour(c).map_or_else(|| default.to_string(), hex_colour)
}

/// What to export: the drawn stencil (the tile when tile output) in layer units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaserInput {
    /// The cut, even-odd polygons, layer units.
    pub cut: BezPath,
    /// The sheet (design bounds plus margin), layer units.
    pub sheet: Rect,
    pub mm_per_unit: f64,
    /// Registration hole centres (already part of `cut`), layer units.
    pub hoshi: Vec<Point>,
    /// Their diameter; holes between split sheets get the same (at least 1 mm).
    pub hoshi_d_mm: f64,
    /// Label text, one line each (the engine builds them: name, size, material).
    pub label_lines: Vec<String>,
    /// The repeat cell's corners (layer units): split lines go on its edges when it is
    /// axis-aligned and they fit.
    pub repeat_cell: Option<[Point; 4]>,
}

/// One sheet to cut, mm, the origin at its top-left (the kerf-grown outline's corner).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaserSheet {
    /// The page: the outline's size (the sheet plus the kerf).
    pub w_mm: f64,
    pub h_mm: f64,
    /// Closed contours in cut order, the outline last.
    pub cut: Vec<Contour>,
    pub score: Vec<Polyline>,
    pub engrave: Vec<Polyline>,
    /// "1 of 3" (also "1 of 1" for a single sheet).
    pub name: String,
    /// Paper pieces on this sheet not joined to its outline (they would fall out).
    pub loose_pieces: usize,
}

/// A laser job: its sheets and what to know about them. No sheets means it was refused (the
/// warnings say why).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LaserJob {
    pub sheets: Vec<LaserSheet>,
    pub warnings: Vec<String>,
}

/// Merge segments shorter than this, mm.
const MIN_SEGMENT_MM: f64 = 0.01;
/// Drop points closer than this to the line through their neighbours, mm.
const COLLINEAR_MM: f64 = 0.002;
/// Contours whose points all match within this are duplicates, mm.
const DUPLICATE_MM: f64 = 0.001;
/// Contours smaller than this are dropped, mm².
const MIN_AREA_MM2: f64 = 1e-6;
/// Curve flattening, mm.
const FLATTEN_MM: f64 = 0.002;

/// One piece's geometry, sheet-local mm.
pub(crate) struct Piece {
    pub contours: Vec<Contour>,
    /// Cut-outs the kerf closed.
    pub vanished: usize,
    pub loose: usize,
}

/// The paper of `rect` (job mm) minus `cut` (plain contours, job mm), kerf-grown by `kerf / 2`,
/// cleaned and ordered, moved so the outline's top-left is the origin.
pub(crate) fn piece(rect: Rect, cut: &BezPath, kerf: f64) -> Result<Piece, String> {
    let paper = crate::ops::boolean_step(&rect.to_path(0.01), cut, BoolOp::Subtract).map_err(|_| "couldn't cut the sheet out".to_string())?;
    let before = subpaths(&paper);
    let paper = if kerf > 0.0 {
        crate::offset::offset(&paper, kerf / 2.0, StrokeJoin::Round).map_err(|_| "couldn't apply the kerf".to_string())?
    } else {
        paper
    };
    let after = subpaths(&paper);
    let rs = rings::rings(&paper, FLATTEN_MM, MAX_SHEET_POINTS).ok_or_else(|| "the sheet has too many points".to_string())?;
    let shift = Vec2::new(kerf / 2.0 - rect.x0, kerf / 2.0 - rect.y0);
    let rs: Vec<Contour> = rs.into_iter().map(|r| r.into_iter().map(|p| p + shift).collect()).collect();
    let rs = dedup(rs.into_iter().filter_map(|r| clean_contour(&r)).collect());
    let (contours, loose) = order(rs);
    Ok(Piece { contours, vanished: before.saturating_sub(after), loose })
}

fn subpaths(p: &BezPath) -> usize {
    p.elements().iter().filter(|e| matches!(e, vectorcraft_geom::kurbo::PathEl::MoveTo(_))).count()
}

/// A contour cleaned: repeated points and segments shorter than [`MIN_SEGMENT_MM`] merged,
/// points within [`COLLINEAR_MM`] of the outline through the rest dropped. `None` when fewer
/// than 3 points or no area are left.
pub(crate) fn clean_contour(ring: &[Point]) -> Option<Contour> {
    let mut pts: Vec<Point> = Vec::with_capacity(ring.len());
    for &p in ring {
        if !(p.x.is_finite() && p.y.is_finite()) {
            continue;
        }
        if pts.last().is_none_or(|&q| (p - q).hypot() >= MIN_SEGMENT_MM) {
            pts.push(p);
        }
    }
    while pts.len() > 1 && pts.first().zip(pts.last()).is_some_and(|(&a, &b)| (a - b).hypot() < MIN_SEGMENT_MM) {
        pts.pop();
    }
    let pts = rings::simplify_ring(&pts, COLLINEAR_MM);
    if pts.len() < 3 || ring_area(&pts).abs() < MIN_AREA_MM2 { None } else { Some(pts) }
}

/// Whether `a` and `b` have the same points within [`DUPLICATE_MM`], in either direction and from
/// any start.
fn same_contour(a: &[Point], b: &[Point]) -> bool {
    let n = a.len();
    if n != b.len() || n == 0 {
        return false;
    }
    let near = |p: Point, q: Point| (p - q).hypot() <= DUPLICATE_MM;
    let Some(&a0) = a.first() else { return false };
    (0..n).filter(|&j| b.get(j).is_some_and(|&q| near(a0, q))).any(|j| {
        let fwd = (0..n).all(|i| matches!((a.get(i), b.get((j + i) % n)), (Some(&p), Some(&q)) if near(p, q)));
        fwd || (0..n).all(|i| matches!((a.get(i), b.get((j + n - i) % n)), (Some(&p), Some(&q)) if near(p, q)))
    })
}

/// `contours` without duplicates (the first of each kept).
pub(crate) fn dedup(contours: Vec<Contour>) -> Vec<Contour> {
    use std::collections::HashMap;
    // Bucket by point count and rounded first-point-independent bounds.
    let key = |c: &[Point]| {
        let bb = bounds(c);
        let q = |v: f64| (v / 0.01).round() as i64;
        (c.len(), q(bb.x0), q(bb.y0), q(bb.x1), q(bb.y1))
    };
    let mut seen: HashMap<(usize, i64, i64, i64, i64), Vec<usize>> = HashMap::new();
    let mut out: Vec<Contour> = Vec::with_capacity(contours.len());
    for c in contours {
        let k = key(&c);
        // Neighbouring buckets too: rounding can put two near-equal bounds either side of a step.
        let mut dup = false;
        'probe: for dx0 in -1..=1 {
            for dy0 in -1..=1 {
                if let Some(list) = seen.get(&(k.0, k.1 + dx0, k.2 + dy0, k.3 + dx0, k.4 + dy0))
                    && list.iter().any(|&i| out.get(i).is_some_and(|o| same_contour(o, &c)))
                {
                    dup = true;
                    break 'probe;
                }
            }
        }
        if !dup {
            seen.entry(k).or_default().push(out.len());
            out.push(c);
        }
    }
    out
}

pub(crate) fn bounds(c: &[Point]) -> Rect {
    let Some(&first) = c.first() else { return Rect::ZERO };
    c.iter().fold(Rect::from_points(first, first), |b, &p| b.union_pt(p))
}

/// Closed outlines (mm, any order) ready to cut: each cleaned (see [`clean_contour`]; one with
/// no area is dropped), duplicates dropped, then put in cut order: the deepest contours first,
/// nearest neighbour within each depth, the largest outermost contour last. For laser-cutting a
/// design that isn't a stencil (PICABIA: `laser.gcode`).
pub fn order_contours(contours: Vec<Contour>) -> Vec<Contour> {
    let cleaned = dedup(contours.into_iter().filter_map(|c| clean_contour(&c)).collect());
    order(cleaned).0
}

/// Greedy nearest-neighbour order above this many contours per depth is replaced by rows.
const MAX_GREEDY: usize = 5000;

/// Contours in cut order (deepest first, the outline last) and the loose paper pieces.
pub(crate) fn order(contours: Vec<Contour>) -> (Vec<Contour>, usize) {
    if contours.is_empty() {
        return (contours, 0);
    }
    let tree = Tree::of_rings(contours);
    let depth = tree.depth;
    let rings = tree.rings;
    let areas: Vec<f64> = rings.iter().map(|r| ring_area(r).abs()).collect();
    // The outline: the largest outermost contour.
    let outline = (0..rings.len())
        .filter(|&i| depth.get(i) == Some(&0))
        .max_by(|&a, &b| areas.get(a).partial_cmp(&areas.get(b)).unwrap_or(std::cmp::Ordering::Equal).then(b.cmp(&a)));
    // Paper pieces are the even-depth contours (outlines of paper); all but the sheet are loose.
    let loose = (0..rings.len()).filter(|&i| depth.get(i).is_some_and(|d| d % 2 == 0)).count().saturating_sub(1);
    let max_depth = depth.iter().copied().max().unwrap_or(0);
    let mut slots: Vec<Option<Contour>> = rings.into_iter().map(Some).collect();
    let mut out: Vec<Contour> = Vec::with_capacity(slots.len());
    let mut head = Point::ZERO;
    for d in (0..=max_depth).rev() {
        let group: Vec<usize> = (0..slots.len()).filter(|&i| depth.get(i) == Some(&d) && Some(i) != outline).collect();
        let picked = if group.len() > MAX_GREEDY { rows(&slots, group) } else { greedy(&slots, group, head) };
        for (i, start) in picked {
            if let Some(c) = slots.get_mut(i).and_then(Option::take) {
                let c = rotate(c, start);
                if let Some(&p) = c.first() {
                    head = p;
                }
                out.push(c);
            }
        }
    }
    if let Some(c) = outline.and_then(|i| slots.get_mut(i)).and_then(Option::take) {
        let start = nearest_vertex(&c, head).map_or(0, |(i, _)| i);
        out.push(rotate(c, start));
    }
    (out, loose)
}

fn nearest_vertex(c: &[Point], p: Point) -> Option<(usize, f64)> {
    c.iter()
        .enumerate()
        .map(|(i, &q)| (i, (q - p).hypot2()))
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)))
}

fn rect_dist2(r: Rect, p: Point) -> f64 {
    let dx = (r.x0 - p.x).max(0.0).max(p.x - r.x1);
    let dy = (r.y0 - p.y).max(0.0).max(p.y - r.y1);
    dx * dx + dy * dy
}

/// `group` ordered greedily from `head`: each time the contour with the vertex nearest the head,
/// started there. Returns (contour, start vertex) pairs.
fn greedy(slots: &[Option<Contour>], mut group: Vec<usize>, mut head: Point) -> Vec<(usize, usize)> {
    let bbs: std::collections::HashMap<usize, Rect> =
        group.iter().map(|&i| (i, slots.get(i).and_then(Option::as_ref).map_or(Rect::ZERO, |c| bounds(c)))).collect();
    let mut out = Vec::with_capacity(group.len());
    while !group.is_empty() {
        // (distance², place in group, contour, vertex)
        let mut best: Option<(f64, usize, usize, usize)> = None;
        for (gi, &i) in group.iter().enumerate() {
            let bb = bbs.get(&i).copied().unwrap_or(Rect::ZERO);
            if best.is_some_and(|b| rect_dist2(bb, head) > b.0) {
                continue;
            }
            let Some(c) = slots.get(i).and_then(Option::as_ref) else { continue };
            if let Some((v, d)) = nearest_vertex(c, head)
                && best.is_none_or(|b| d < b.0 || (d == b.0 && i < b.2))
            {
                best = Some((d, gi, i, v));
            }
        }
        let Some((_, gi, i, v)) = best else { break };
        group.swap_remove(gi);
        if let Some(&p) = slots.get(i).and_then(Option::as_ref).and_then(|c| c.get(v)) {
            head = p;
        }
        out.push((i, v));
    }
    out
}

/// A cheap order for very many contours: rows top to bottom, alternating direction.
fn rows(slots: &[Option<Contour>], group: Vec<usize>) -> Vec<(usize, usize)> {
    let mut keyed: Vec<(i64, f64, usize)> = group
        .into_iter()
        .map(|i| {
            let c = slots.get(i).and_then(Option::as_ref).map_or(Point::ZERO, |c| bounds(c).center());
            ((c.y / 10.0).floor() as i64, c.x, i)
        })
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(if a.0 % 2 == 0 { a.1.total_cmp(&b.1) } else { b.1.total_cmp(&a.1) }).then(a.2.cmp(&b.2)));
    keyed.into_iter().map(|k| (k.2, 0)).collect()
}

fn rotate(mut c: Contour, start: usize) -> Contour {
    if start < c.len() {
        c.rotate_left(start);
    }
    c
}

/// How far the label keeps from the sheet's left and bottom edges (beyond the room's padding),
/// widest first: the first that fits wins.
const LABEL_INSETS_MM: [f64; 5] = [6.0, 4.0, 2.5, 1.0, 0.0];

/// Lay the label out on a sheet: `lines` engraved in the free bottom margin from the left (box
/// inside `room`), clear of every contour by `clear`. Tries 2.5 mm caps on one line, then
/// stacked, then smaller, down to 1.5 mm; at each size it keeps away from the sheet's edge
/// (handled paper edges fray) by the widest of [`LABEL_INSETS_MM`] that fits. `None` when nothing
/// fits.
pub(crate) fn label(lines: &[String], room: Rect, contours: &[Contour], clear: f64) -> Option<Vec<Polyline>> {
    if lines.is_empty() || room.width() <= 0.0 || room.height() <= 0.0 {
        return None;
    }
    let mut index = SegIndex::new(contours);
    let one = vec![lines.join("   ")];
    for cap in [2.5, 2.0, 1.5] {
        for layout in [&one, &lines.to_vec()] {
            let pitch = cap * 1.8;
            let n = layout.len() as f64;
            let height = cap + (n - 1.0) * pitch + cap * DESCENT;
            let width = layout.iter().map(|l| text_polylines(l, Point::ZERO, cap).1).fold(0.0, f64::max);
            for inset in LABEL_INSETS_MM {
                let r = Rect::new(room.x0 + inset, room.y0, room.x1, room.y1 - inset);
                if width > r.width() || height > r.height() {
                    continue;
                }
                let y0 = r.y1 - height;
                let step = (r.width() / 400.0).max(0.5);
                let mut x0 = r.x0;
                while x0 + width <= r.x1 {
                    let b = Rect::new(x0, y0, x0 + width, r.y1).inflate(clear, clear);
                    let quad = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)];
                    // On paper (inside an odd number of contours: the outline, not a cut-out in it).
                    if !index.quad_hits(&quad, [usize::MAX; 2]) && index.containing(b.center()).len() % 2 == 1 {
                        let mut out = Vec::new();
                        for (k, l) in layout.iter().enumerate() {
                            out.extend(text_polylines(l, Point::new(x0, y0 + cap + k as f64 * pitch), cap).0);
                        }
                        return Some(out);
                    }
                    x0 += step;
                }
            }
        }
    }
    None
}

/// The laser job for `input` with `params` (clamped first). See the module docs.
pub fn laser_job(input: &LaserInput, params: &LaserParams) -> LaserJob {
    let p = params.clamped();
    let mut warnings = Vec::new();
    let refuse = |w: &str| LaserJob { sheets: Vec::new(), warnings: vec![w.to_string()] };
    let mpu = input.mm_per_unit;
    let sheet = input.sheet.abs();
    let finite_rect = |r: Rect| r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite();
    if !(mpu.is_finite() && mpu > 0.0 && finite_rect(sheet)) {
        return refuse("nothing to export: the sheet has no physical size");
    }
    let (w, h) = (sheet.width() * mpu, sheet.height() * mpu);
    if !(w > 0.0 && h > 0.0) {
        return refuse("nothing to export: the sheet is empty");
    }
    if w > MAX_SHEET_MM || h > MAX_SHEET_MM {
        return refuse(&format!("the sheet ({} × {} mm) is larger than {} mm", fmt_mm(w), fmt_mm(h), fmt_mm(MAX_SHEET_MM)));
    }
    let to_mm = Affine::scale(mpu) * Affine::translate(-sheet.origin().to_vec2());
    let cut = to_mm * input.cut.clone();
    if !crate::ops::finite(&cut) {
        return refuse("the cut has invalid coordinates");
    }
    if cut.elements().len() > crate::offset::MAX_OFFSET_ELEMENTS {
        return refuse("the cut has too many points to export");
    }
    let cut = if cut.elements().is_empty() {
        cut
    } else {
        match crate::ops::simplify_with_rule(&cut, true) {
            Ok(c) => c,
            Err(_) => return refuse("the cut has invalid coordinates"),
        }
    };
    let Some(cut_rings) = rings::rings(&cut, FLATTEN_MM, MAX_SHEET_POINTS) else { return refuse("the cut has too many points to export") };
    let cell = input.repeat_cell.map(|c| c.map(|q| to_mm * q));
    let hole_d = if input.hoshi_d_mm.is_finite() { input.hoshi_d_mm.clamp(1.0, 20.0) } else { 1.5 };
    let Some(plan) = laser_bed::plan(w, h, &p, &cut_rings, cell, hole_d, &mut warnings) else {
        return LaserJob { sheets: Vec::new(), warnings };
    };
    let total = plan.pieces.len();
    // `input.hoshi` needs no handling of its own: the design's holes are part of the cut, so the
    // label and the holes between sheets clear them as they clear every cut-out.
    let label_lines: Vec<String> = input.label_lines.iter().filter(|l| !l.trim().is_empty()).take(MAX_LABEL_LINES).cloned().collect();
    if p.label && label_lines.iter().any(|l| l.chars().any(|c| !font_has(c))) {
        warnings.push("the label has characters the laser font lacks; they engrave as boxes".into());
    }
    let mut vanished = 0;
    let mut sheets = Vec::with_capacity(total);
    let k = p.kerf_mm;
    let tolerance = 0.005;
    for (n, pc) in plan.pieces.iter().enumerate() {
        let name = format!("{} of {}", n + 1, total);
        // This piece's registration holes join the cut (they're placed clear of it).
        let mut piece_cut = cut.clone();
        for &c in &plan.holes {
            if pc.rect.contains(c) {
                piece_cut.extend(rings::to_path(&[hoshi::disc(c, hole_d / 2.0, tolerance)]).iter());
            }
        }
        let geom = match piece(pc.rect, &piece_cut, k) {
            Ok(g) => g,
            Err(e) => {
                warnings.push(format!("sheet {name}: {e}; it was left out"));
                continue;
            }
        };
        vanished += geom.vanished;
        let shift = Vec2::new(k / 2.0 - pc.rect.x0, k / 2.0 - pc.rect.y0);
        let score: Vec<Polyline> = pc.score.iter().map(|l| l.iter().map(|&q| q + shift).collect()).collect();
        if geom.loose > 0 {
            let what = if geom.loose == 1 { "1 piece would fall out".to_string() } else { format!("{} pieces would fall out", geom.loose) };
            warnings.push(if total > 1 {
                format!("sheet {name}: {what}; move the split or add ties")
            } else {
                format!("{what} of the sheet; add ties")
            });
        }
        let mut engrave = Vec::new();
        if p.label {
            let mut lines = label_lines.clone();
            if total > 1 {
                lines.push(name.clone());
            }
            lines.push("THIS SIDE UP ↑".into());
            let pad = 1.5 + k / 2.0;
            let room = Rect::new(pc.free.x0 + shift.x + pad, pc.free.y0 + shift.y + pad, pc.free.x1 + shift.x - pad, pc.free.y1 + shift.y - pad);
            match label(&lines, room, &geom.contours, 0.5) {
                Some(l) => engrave = l,
                None => warnings.push(if total > 1 {
                    format!("sheet {name}: no room for the label clear of the cut; it was left out")
                } else {
                    "no room for the label clear of the cut; it was left out".into()
                }),
            }
        }
        sheets.push(LaserSheet {
            w_mm: pc.rect.width() + k,
            h_mm: pc.rect.height() + k,
            cut: geom.contours,
            score,
            engrave,
            name,
            loose_pieces: geom.loose,
        });
    }
    if vanished > 0 {
        warnings.insert(
            0,
            if vanished == 1 {
                "1 cut-out is narrower than the kerf and was left out".into()
            } else {
                format!("{vanished} cut-outs are narrower than the kerf and were left out")
            },
        );
    }
    LaserJob { sheets, warnings }
}

/// A length for messages: up to 2 decimals.
pub(crate) fn fmt_mm(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}
