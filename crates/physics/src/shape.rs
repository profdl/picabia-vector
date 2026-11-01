//! An object's collision outline in document space: the pieces its colliders are built from.
//!
//! Paths are flattened to polylines (each subpath one ring, open or closed); a near-circular
//! live ellipse is a ball; type, images, symbol instances and other kinds are their geometric
//! bounding boxes. A group is all its visible members (a clip group: its clipping path only).
//! Strokes are ignored (geometric outlines, as the selection box measures them).
//!
//! Caps: at most [`MAX_POINTS_PER_BODY`] points per object (rings are simplified with
//! Ramer–Douglas–Peucker, as tlprocedural's `simplifyVertices`) and [`MAX_PIECES_PER_BODY`]
//! pieces (more: the object is its convex hull). Geometry that isn't finite, is empty, too large
//! or has no size is refused with the reason.

use vectorcraft_doc::{LiveShape, Node, NodeKind, SymbolBoxes};
use vectorcraft_geom::{BezPath, PathEl, Point, Rect};

/// The most outline points one object keeps (more are simplified away).
pub const MAX_POINTS_PER_BODY: usize = 2_000;
/// The most pieces (rings, balls, boxes) one object keeps; more: its convex hull alone.
pub const MAX_PIECES_PER_BODY: usize = 512;
/// The most points one ring flattens to before simplification (more are skipped evenly).
const MAX_RAW_POINTS: usize = 100_000;
/// Coordinates beyond ±this (points) are refused: f32 maths loses the shape out there.
pub const MAX_COORD: f64 = 1.0e6;
/// An object smaller than this (points) both ways has no size to collide with.
pub const MIN_SIZE: f64 = 0.01;
/// Groups nested deeper than this are not looked into.
const MAX_DEPTH: usize = 64;

/// One piece of an outline, in document space.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
    /// A flattened subpath.
    Ring { pts: Vec<Point>, closed: bool },
    /// A circle (a near-circular live ellipse).
    Ball { center: Point, radius: f64 },
    /// A box (type, images, symbol instances, other kinds).
    Box(Rect),
}

impl Piece {
    fn points(&self) -> Vec<Point> {
        match self {
            Piece::Ring { pts, .. } => pts.clone(),
            Piece::Ball { center, radius } => (0..16)
                .map(|i| f64::from(i) * std::f64::consts::TAU / 16.0)
                .map(|a| Point::new(center.x + radius * a.cos(), center.y + radius * a.sin()))
                .collect(),
            Piece::Box(r) => vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)],
        }
    }
}

/// An object's outline: its pieces, its geometric bounds (the body's origin is their centre),
/// and what was simplified.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyShape {
    pub pieces: Vec<Piece>,
    pub bounds: Rect,
    pub notes: Vec<String>,
}

impl BodyShape {
    /// Every point of every piece (balls as 16 points, boxes as their corners).
    pub fn all_points(&self) -> Vec<Point> {
        self.pieces.iter().flat_map(Piece::points).collect()
    }
}

/// `node`'s outline (see the module docs), or why it has none to collide with.
pub fn body_shape(node: &Node, symbols: Option<&SymbolBoxes>) -> Result<BodyShape, String> {
    let bounds = match symbols {
        Some(s) => node.geometric_bounds_with(s),
        None => node.geometric_bounds(),
    };
    let bounds = bounds.ok_or("it has no geometry")?;
    check_rect(bounds)?;
    let tol = (bounds.width().hypot(bounds.height()) * 0.002).clamp(0.05, 5.0);
    let mut pieces = Vec::new();
    collect(node, symbols, tol, 0, &mut pieces);
    pieces.retain(|p| match p {
        Piece::Ring { pts, .. } => pts.len() >= 2 && pts.iter().all(|q| finite_in_range(*q)),
        Piece::Ball { center, radius } => finite_in_range(*center) && radius.is_finite() && *radius > 0.0,
        Piece::Box(r) => check_rect(*r).is_ok(),
    });
    if pieces.is_empty() {
        return Err("it has no outline to collide with".into());
    }
    let mut notes = Vec::new();
    if pieces.len() > MAX_PIECES_PER_BODY {
        notes.push(format!("{} pieces (at most {MAX_PIECES_PER_BODY}): collides as its convex hull", pieces.len()));
        let all: Vec<Point> = pieces.iter().flat_map(Piece::points).collect();
        let hull = convex_hull(&all);
        let hull = if hull.len() > MAX_POINTS_PER_BODY { simplify(&hull, MAX_POINTS_PER_BODY, true) } else { hull };
        pieces = vec![Piece::Ring { pts: hull, closed: true }];
    }
    let total: usize = pieces.iter().map(|p| if let Piece::Ring { pts, .. } = p { pts.len() } else { 4 }).sum();
    if total > MAX_POINTS_PER_BODY {
        notes.push(format!("{total} outline points (at most {MAX_POINTS_PER_BODY}): simplified"));
        for p in &mut pieces {
            if let Piece::Ring { pts, closed } = p {
                let share = (MAX_POINTS_PER_BODY * pts.len() / total.max(1)).max(3);
                *pts = simplify(pts, share, *closed);
            }
        }
    }
    Ok(BodyShape { pieces, bounds, notes })
}

fn finite_in_range(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.x.abs() <= MAX_COORD && p.y.abs() <= MAX_COORD
}

fn check_rect(r: Rect) -> Result<(), String> {
    if ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) {
        return Err("its geometry is not finite".into());
    }
    if !finite_in_range(Point::new(r.x0, r.y0)) || !finite_in_range(Point::new(r.x1, r.y1)) {
        return Err(format!("it reaches beyond ±{MAX_COORD} pt"));
    }
    if r.width() < MIN_SIZE && r.height() < MIN_SIZE {
        return Err("it has no size".into());
    }
    Ok(())
}

fn collect(n: &Node, symbols: Option<&SymbolBoxes>, tol: f64, depth: usize, out: &mut Vec<Piece>) {
    if depth > MAX_DEPTH || out.len() > MAX_PIECES_PER_BODY * 4 {
        return;
    }
    match &n.kind {
        NodeKind::Path { path, guide, live, .. } => {
            if *guide {
                return;
            }
            if let Some(ball) = live.as_deref().and_then(|l| ball_of(l, path.bounds())) {
                out.push(ball);
                return;
            }
            for sp in &path.subpaths {
                let mut bez = BezPath::new();
                sp.to_bezpath_into(&mut bez);
                out.extend(flatten_rings(&bez, tol));
            }
        }
        NodeKind::Compound { children, .. } | NodeKind::Layer { children, .. } => {
            for c in children.iter().filter(|c| c.visible) {
                collect(c, symbols, tol, depth + 1, out);
            }
        }
        NodeKind::Group { children, clip } => {
            if *clip {
                if let Some(c) = children.first() {
                    collect(c, symbols, tol, depth + 1, out);
                }
            } else {
                for c in children.iter().filter(|c| c.visible) {
                    collect(c, symbols, tol, depth + 1, out);
                }
            }
        }
        _ => {
            let b = match symbols {
                Some(s) => n.geometric_bounds_with(s),
                None => n.geometric_bounds(),
            };
            if let Some(b) = b {
                out.push(Piece::Box(b));
            }
        }
    }
}

/// A whole live ellipse whose bounds are within 5% of square is a ball (tlprocedural's circle
/// rule: a ball when the aspect ratio is under 1.05).
fn ball_of(live: &LiveShape, bounds: Option<Rect>) -> Option<Piece> {
    let LiveShape::Ellipse { pie, .. } = live else { return None };
    let whole = (pie.1 - pie.0).abs() >= 360.0 - 1e-9 || (pie.0 == 0.0 && (pie.1 == 0.0 || pie.1 == 360.0));
    let b = bounds?;
    let (rx, ry) = (b.width() / 2.0, b.height() / 2.0);
    if !whole || rx.min(ry) <= 0.0 || rx.max(ry) / rx.min(ry) >= 1.05 {
        return None;
    }
    Some(Piece::Ball { center: b.center(), radius: (rx + ry) / 2.0 })
}

/// The polylines `bez` flattens to (one per subpath, with its closed flag).
fn flatten_rings(bez: &BezPath, tol: f64) -> Vec<Piece> {
    let mut rings = Vec::new();
    let mut cur: Vec<Point> = Vec::new();
    let mut raw = 0usize;
    let finish = |cur: &mut Vec<Point>, closed: bool, rings: &mut Vec<Piece>| {
        if closed && cur.len() > 2 && cur.first() == cur.last() {
            cur.pop();
        }
        if cur.len() >= 2 {
            let closed = closed && cur.len() >= 3;
            rings.push(Piece::Ring { pts: std::mem::take(cur), closed });
        }
        cur.clear();
    };
    vectorcraft_geom::flatten(bez.iter(), tol, |el| match el {
        PathEl::MoveTo(p) => {
            finish(&mut cur, false, &mut rings);
            cur.push(p);
        }
        PathEl::LineTo(p) => {
            raw += 1;
            if raw <= MAX_RAW_POINTS && cur.last() != Some(&p) {
                cur.push(p);
            }
        }
        PathEl::ClosePath => finish(&mut cur, true, &mut rings),
        PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
            if cur.last() != Some(&p) {
                cur.push(p);
            }
        }
    });
    finish(&mut cur, false, &mut rings);
    rings
}

/// Ramer–Douglas–Peucker with an explicit stack (tlprocedural's `rdpSimplify`).
fn rdp(v: &[Point], eps: f64) -> Vec<Point> {
    let n = v.len();
    if n <= 2 {
        return v.to_vec();
    }
    let mut keep = vec![false; n];
    if let Some(k) = keep.first_mut() {
        *k = true;
    }
    if let Some(k) = keep.last_mut() {
        *k = true;
    }
    let mut stack = vec![(0usize, n - 1)];
    while let Some((s, e)) = stack.pop() {
        if e < s + 2 {
            continue;
        }
        let (Some(&a), Some(&b)) = (v.get(s), v.get(e)) else { continue };
        let d = b - a;
        let len2 = d.hypot2();
        let (mut best, mut at) = (0.0, s);
        for i in s + 1..e {
            let Some(&p) = v.get(i) else { continue };
            let dist = if len2 < 1e-20 {
                (p - a).hypot()
            } else {
                let t = ((p - a).dot(d) / len2).clamp(0.0, 1.0);
                (p - (a + d * t)).hypot()
            };
            if dist > best {
                best = dist;
                at = i;
            }
        }
        if best > eps {
            if let Some(k) = keep.get_mut(at) {
                *k = true;
            }
            stack.push((s, at));
            stack.push((at, e));
        }
    }
    v.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// `v` simplified to at most `max` points (at least 3 for a closed ring, 2 open): RDP with a
/// bisected tolerance (tlprocedural's `simplifyVertices`), then evenly spaced points if RDP
/// still keeps too many.
pub fn simplify(v: &[Point], max: usize, closed: bool) -> Vec<Point> {
    let max = max.max(if closed { 3 } else { 2 });
    if v.len() <= max {
        return v.to_vec();
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in v {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    let (mut lo, mut hi) = (0.0, (x1 - x0).hypot(y1 - y0) * 0.5);
    let mut best: Option<Vec<Point>> = None;
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        let r = rdp(v, mid);
        if r.len() > max {
            lo = mid;
        } else {
            hi = mid;
            let good = r.len() * 10 >= max * 8;
            best = Some(r);
            if good {
                break;
            }
        }
    }
    let r = best.unwrap_or_else(|| rdp(v, hi));
    if r.len() <= max && r.len() >= if closed { 3 } else { 2 } {
        return r;
    }
    // Evenly spaced points (RDP collapsed the ring, or couldn't get under the cap).
    let step = v.len() as f64 / max as f64;
    (0..max).filter_map(|i| v.get((i as f64 * step) as usize).copied()).collect()
}

/// The convex hull of `pts` (Andrew's monotone chain), counter-clockwise in a y-up frame.
pub fn convex_hull(pts: &[Point]) -> Vec<Point> {
    let mut p: Vec<Point> = pts.iter().copied().filter(|q| q.x.is_finite() && q.y.is_finite()).collect();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: Point, a: Point, b: Point| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    let mut lower: Vec<Point> = Vec::new();
    for &q in &p {
        while let [.., a, b] = lower.as_slice() {
            if cross(*a, *b, q) <= 0.0 {
                lower.pop();
            } else {
                break;
            }
        }
        lower.push(q);
    }
    let mut upper: Vec<Point> = Vec::new();
    for &q in p.iter().rev() {
        while let [.., a, b] = upper.as_slice() {
            if cross(*a, *b, q) <= 0.0 {
                upper.pop();
            } else {
                break;
            }
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Twice the signed area of ring `v` (positive: counter-clockwise in a y-up frame).
pub fn signed_area2(v: &[Point]) -> f64 {
    let n = v.len();
    (0..n).filter_map(|i| Some((v.get(i)?, v.get((i + 1) % n)?))).map(|(a, b)| a.x * b.y - b.x * a.y).sum()
}

/// Convex parts covering closed ring `v` (a simple polygon): ear-clipped triangles, each merged
/// into the part before it while that part stays convex. `None` when the ring isn't simple
/// enough to clip (self-intersecting): use its hull.
pub fn convex_parts(v: &[Point]) -> Option<Vec<Vec<Point>>> {
    let area = signed_area2(v);
    if v.len() < 3 || !area.is_finite() || area.abs() < 1e-12 {
        return None;
    }
    // Work counter-clockwise.
    let ring: Vec<Point> = if area > 0.0 { v.to_vec() } else { v.iter().rev().copied().collect() };
    let cross = |o: Point, a: Point, b: Point| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    let pt = |i: usize| ring.get(i).copied().unwrap_or(Point::ZERO);
    let mut idx: Vec<usize> = (0..ring.len()).collect();
    let mut tris: Vec<[usize; 3]> = Vec::new();
    let mut guard = ring.len() * ring.len() + 16;
    while idx.len() > 3 {
        guard = guard.checked_sub(1)?;
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (Some(&a), Some(&b), Some(&c)) = (idx.get((k + m - 1) % m), idx.get(k), idx.get((k + 1) % m)) else { return None };
            if cross(pt(a), pt(b), pt(c)) <= 0.0 {
                continue;
            }
            let inside = idx.iter().any(|&o| {
                o != a && o != b && o != c && {
                    let p = pt(o);
                    cross(pt(a), pt(b), p) >= 0.0 && cross(pt(b), pt(c), p) >= 0.0 && cross(pt(c), pt(a), p) >= 0.0
                }
            });
            if inside {
                continue;
            }
            tris.push([a, b, c]);
            idx.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            return None;
        }
    }
    if let [a, b, c] = idx.as_slice() {
        tris.push([*a, *b, *c]);
    }
    // Merge each triangle into the part before it while it stays convex.
    let convex = |poly: &[usize]| {
        let n = poly.len();
        (0..n).all(|i| {
            let (Some(&a), Some(&b), Some(&c)) = (poly.get(i), poly.get((i + 1) % n), poly.get((i + 2) % n)) else { return false };
            cross(pt(a), pt(b), pt(c)) >= -1e-12
        })
    };
    let mut parts: Vec<Vec<usize>> = Vec::new();
    for t in tris {
        let merged = parts.last().and_then(|cur| {
            let n = cur.len();
            (0..n).find_map(|i| {
                let (u, w) = (*cur.get(i)?, *cur.get((i + 1) % n)?);
                // The triangle has the edge w→u: insert its third vertex between u and w.
                let third = (0..3).find_map(|j| {
                    let (p, q, r) = (t.get(j)?, t.get((j + 1) % 3)?, t.get((j + 2) % 3)?);
                    (*p == w && *q == u).then_some(*r)
                })?;
                let mut next = cur.clone();
                next.insert(i + 1, third);
                convex(&next).then_some(next)
            })
        });
        match merged {
            Some(next) => {
                if let Some(last) = parts.last_mut() {
                    *last = next;
                }
            }
            None => parts.push(t.to_vec()),
        }
    }
    Some(parts.into_iter().map(|p| p.into_iter().map(pt).collect()).collect())
}
