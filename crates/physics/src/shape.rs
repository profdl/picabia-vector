//! An object's collision outline in document space: the pieces its colliders are built from.
//!
//! Paths are flattened to polylines (each subpath one ring, open or closed); a near-circular
//! live ellipse is a ball; type, images, symbol instances and other kinds are their geometric
//! bounding boxes. A group is all its visible members (a clip group: its clipping path only).
//! Strokes are ignored (geometric outlines, as the selection box measures them).
//!
//! Caps, applied while the outline is read so a huge path costs a bounded amount of memory: at
//! most [`MAX_RAW_POINTS`] flattened points per object (skipped evenly across all its outlines),
//! [`MAX_COLLECTED_PIECES`] outlines read (the rest left out, with a note), then at most
//! [`MAX_POINTS_PER_BODY`] points (rings simplified with Ramer–Douglas–Peucker, as tlprocedural's
//! `simplifyVertices`) and [`MAX_PIECES_PER_BODY`] pieces (more: the object is its convex hull).
//! Geometry that isn't finite, is empty, too large or has no size is refused with the reason.

use vectorcraft_doc::{LiveShape, Node, NodeKind, SymbolBoxes};
use vectorcraft_geom::{BezPath, PathEl, Point, Rect};

/// The most outline points one object keeps (more are simplified away).
pub const MAX_POINTS_PER_BODY: usize = 2_000;
/// The most pieces (rings, balls, boxes) one object keeps; more: its convex hull alone.
pub const MAX_PIECES_PER_BODY: usize = 512;
/// The most flattened points one object keeps before simplification (more are skipped evenly).
pub const MAX_RAW_POINTS: usize = 100_000;
/// The most outlines (subpaths, boxes, balls) read from one object; the rest are left out.
pub const MAX_COLLECTED_PIECES: usize = MAX_PIECES_PER_BODY * 4;
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
        self.outline(16)
    }

    /// The piece's points: a ring's own, a ball as `ball_points` points around it, a box's corners.
    pub fn outline(&self, ball_points: u32) -> Vec<Point> {
        match self {
            Piece::Ring { pts, .. } => pts.clone(),
            Piece::Ball { center, radius } => (0..ball_points.max(3))
                .map(|i| f64::from(i) * std::f64::consts::TAU / f64::from(ball_points.max(3)))
                .map(|a| Point::new(center.x + radius * a.cos(), center.y + radius * a.sin()))
                .collect(),
            Piece::Box(r) => vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)],
        }
    }

    /// Whether the piece is a closed outline (a closed ring, a ball, a box).
    pub fn is_closed(&self) -> bool {
        !matches!(self, Piece::Ring { closed: false, .. })
    }
}

/// An object's outline: its pieces, its geometric bounds (the body's origin is their centre),
/// and what was simplified.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyShape {
    pub pieces: Vec<Piece>,
    pub bounds: Rect,
    pub notes: Vec<String>,
    /// For each piece, the outline group it belongs to: a path's subpaths (or a compound path's
    /// members) share one, so even-odd nesting makes holes only within it; a group's members each
    /// have their own. Empty: all one.
    pub owners: Vec<usize>,
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
    // First count the flattened points, then keep every `stride`-th of them.
    let mut counter = Collector { symbols, tol, stride: 1, seen: 0, pieces: Vec::new(), read: 0, left_out: 0, counting: true, owner: 0, fixed: None };
    counter.node(node, 0);
    let stride = counter.seen.div_ceil(MAX_RAW_POINTS).max(1);
    let mut c = Collector { symbols, tol, stride, seen: 0, pieces: Vec::new(), read: 0, left_out: 0, counting: false, owner: 0, fixed: None };
    c.node(node, 0);
    let mut owned = c.pieces;
    let mut notes = Vec::new();
    if c.left_out > 0 {
        notes.push(format!("more than {MAX_COLLECTED_PIECES} outlines: {} left out", c.left_out));
    }
    if stride > 1 {
        notes.push(format!("{} outline points: every {stride}th kept", counter.seen));
    }
    owned.retain(|(p, _)| match p {
        Piece::Ring { pts, .. } => pts.len() >= 2 && pts.iter().all(|q| finite_in_range(*q)),
        Piece::Ball { center, radius } => finite_in_range(*center) && radius.is_finite() && *radius > 0.0,
        Piece::Box(r) => check_rect(*r).is_ok(),
    });
    let (mut pieces, mut owners): (Vec<Piece>, Vec<usize>) = owned.into_iter().unzip();
    if pieces.is_empty() {
        return Err("it has no outline to collide with".into());
    }
    if pieces.len() > MAX_PIECES_PER_BODY {
        notes.push(format!("{} pieces (at most {MAX_PIECES_PER_BODY}): collides as its convex hull", pieces.len()));
        let all: Vec<Point> = pieces.iter().flat_map(Piece::points).collect();
        let hull = convex_hull(&all);
        let hull = if hull.len() > MAX_POINTS_PER_BODY { simplify(&hull, MAX_POINTS_PER_BODY, true) } else { hull };
        pieces = vec![Piece::Ring { pts: hull, closed: true }];
        owners = vec![0];
    }
    let rings = pieces.iter().filter(|p| matches!(p, Piece::Ring { .. })).count();
    let total: usize = pieces.iter().map(|p| if let Piece::Ring { pts, .. } = p { pts.len() } else { 0 }).sum();
    if total > MAX_POINTS_PER_BODY {
        notes.push(format!("{total} outline points (at most {MAX_POINTS_PER_BODY}): simplified"));
        // Each ring keeps at least 3 points and a share of the rest by its size, so the total stays
        // within the cap (at most 512 rings: 1,536 points of floors).
        let spare = MAX_POINTS_PER_BODY.saturating_sub(3 * rings);
        for p in &mut pieces {
            if let Piece::Ring { pts, closed } = p {
                let share = 3 + spare * pts.len() / total.max(1);
                *pts = simplify(pts, share, *closed);
            }
        }
    }
    Ok(BodyShape { pieces, bounds, notes, owners })
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

/// Reads an object's outlines: counting the flattened points (`counting`), or keeping every
/// `stride`-th of them (each outline's first point always), up to [`MAX_COLLECTED_PIECES`].
struct Collector<'a, 'b> {
    symbols: Option<&'a SymbolBoxes<'b>>,
    tol: f64,
    stride: usize,
    /// Flattened points seen so far (all outlines).
    seen: usize,
    /// The pieces read, each with its outline group ([`BodyShape::owners`]).
    pieces: Vec<(Piece, usize)>,
    /// The next outline group.
    owner: usize,
    /// Inside a compound path: its outline group, shared by its members.
    fixed: Option<usize>,
    /// Outlines read (subpaths, boxes, balls): both passes read the same ones.
    read: usize,
    /// Outlines past the cap.
    left_out: usize,
    counting: bool,
}

impl Collector<'_, '_> {
    /// Take one more outline: false (and counted as left out) past the cap.
    fn take(&mut self) -> bool {
        if self.read >= MAX_COLLECTED_PIECES {
            self.left_out += 1;
            return false;
        }
        self.read += 1;
        true
    }

    /// The outline group of the next object's outlines: the compound path's when inside one.
    fn group(&mut self) -> usize {
        self.fixed.unwrap_or_else(|| {
            self.owner += 1;
            self.owner
        })
    }

    fn push(&mut self, p: Piece) {
        if self.take() && !self.counting {
            let g = self.group();
            self.pieces.push((p, g));
        }
    }

    fn node(&mut self, n: &Node, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        match &n.kind {
            NodeKind::Path { path, guide, live, .. } => {
                if *guide {
                    return;
                }
                if let Some(ball) = live.as_deref().and_then(|l| ball_of(l, path.bounds())) {
                    self.push(ball);
                    return;
                }
                let g = self.group();
                for sp in &path.subpaths {
                    if !self.take() {
                        continue;
                    }
                    let mut bez = BezPath::new();
                    sp.to_bezpath_into(&mut bez);
                    self.flatten(&bez, g);
                }
            }
            NodeKind::Compound { children, .. } => {
                // A compound path's members make holes in each other: one outline group.
                let outer = self.fixed;
                let g = self.group();
                self.fixed = Some(g);
                for c in children.iter().filter(|c| c.visible) {
                    self.node(c, depth + 1);
                }
                self.fixed = outer;
            }
            NodeKind::Layer { children, .. } => {
                for c in children.iter().filter(|c| c.visible) {
                    self.node(c, depth + 1);
                }
            }
            NodeKind::Group { children, clip } => {
                if *clip {
                    if let Some(c) = children.first() {
                        self.node(c, depth + 1);
                    }
                } else {
                    for c in children.iter().filter(|c| c.visible) {
                        self.node(c, depth + 1);
                    }
                }
            }
            _ => {
                let b = match self.symbols {
                    Some(s) => n.geometric_bounds_with(s),
                    None => n.geometric_bounds(),
                };
                if let Some(b) = b {
                    self.push(Piece::Box(b));
                }
            }
        }
    }

    /// The polylines `bez` flattens to (one per subpath, with its closed flag), every
    /// `stride`-th point kept (each subpath's first always), so a long path is thinned evenly;
    /// each in outline group `group`.
    fn flatten(&mut self, bez: &BezPath, group: usize) {
        let mut cur: Vec<Point> = Vec::new();
        let (stride, counting) = (self.stride.max(1), self.counting);
        let mut rings: Vec<Piece> = Vec::new();
        let mut seen = self.seen;
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
        let mut point = |p: Point, first: bool, cur: &mut Vec<Point>| {
            seen += 1;
            if !counting && (first || seen.is_multiple_of(stride)) && cur.last() != Some(&p) {
                cur.push(p);
            }
        };
        vectorcraft_geom::flatten(bez.iter(), self.tol, |el| match el {
            PathEl::MoveTo(p) => {
                finish(&mut cur, false, &mut rings);
                point(p, true, &mut cur);
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => point(p, false, &mut cur),
            PathEl::ClosePath => finish(&mut cur, true, &mut rings),
        });
        finish(&mut cur, false, &mut rings);
        self.seen = seen;
        if !counting {
            self.pieces.extend(rings.into_iter().map(|r| (r, group)));
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
    // RDP is quadratic at worst: thin a long ring evenly to twice the target first.
    let thinned: Vec<Point>;
    let v = if v.len() > 2 * max {
        let step = v.len() as f64 / (2 * max) as f64;
        thinned = (0..2 * max).filter_map(|i| v.get((i as f64 * step) as usize).copied()).collect();
        thinned.as_slice()
    } else {
        v
    };
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in v {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    let (mut lo, mut hi) = (0.0, (x1 - x0).hypot(y1 - y0) * 0.5);
    let mut best: Option<Vec<Point>> = None;
    for _ in 0..16 {
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
    let v = &clean_ring(v);
    if !is_simple(v) {
        return None;
    }
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

/// `v` without repeated points (the closing one too) and without points on a straight line
/// between their neighbours.
pub fn clean_ring(v: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(v.len());
    for &p in v {
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    while out.len() > 1 && out.first() == out.last() {
        out.pop();
    }
    // Drop collinear points until none is left (each pass removes at least one or stops).
    loop {
        let n = out.len();
        if n < 3 {
            return out;
        }
        let keep: Vec<bool> = (0..n)
            .map(|i| {
                let (Some(&a), Some(&b), Some(&c)) = (out.get((i + n - 1) % n), out.get(i), out.get((i + 1) % n)) else { return true };
                let cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
                let scale = ((b - a).hypot() * (c - a).hypot()).max(f64::MIN_POSITIVE);
                cross.abs() / scale > 1e-12
            })
            .collect();
        if keep.iter().all(|k| *k) {
            return out;
        }
        // Remove one point per pass at a time from each run, keeping the polygon connected.
        let mut next = Vec::with_capacity(n);
        let mut dropped_prev = false;
        for (p, k) in out.iter().zip(&keep) {
            if !*k && !dropped_prev {
                dropped_prev = true;
            } else {
                next.push(*p);
                dropped_prev = false;
            }
        }
        if next.len() == n {
            return out;
        }
        out = next;
    }
}

/// Whether closed ring `v` is simple: no two edges that aren't neighbours touch or cross.
pub fn is_simple(v: &[Point]) -> bool {
    let n = v.len();
    if n < 3 {
        return false;
    }
    let touch = segments_touch;
    for i in 0..n {
        let (Some(&a), Some(&b)) = (v.get(i), v.get((i + 1) % n)) else { return false };
        for j in i + 1..n {
            // Neighbouring edges share a point: skip them.
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            let (Some(&c), Some(&d)) = (v.get(j), v.get((j + 1) % n)) else { return false };
            if touch(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

/// Whether segments `a`–`b` and `c`–`d` cross or touch.
pub fn segments_touch(a: Point, b: Point, c: Point, d: Point) -> bool {
    let orient = |a: Point, b: Point, c: Point| {
        let x = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        if x > 0.0 {
            1
        } else if x < 0.0 {
            -1
        } else {
            0
        }
    };
    let on = |a: Point, b: Point, p: Point| p.x >= a.x.min(b.x) && p.x <= a.x.max(b.x) && p.y >= a.y.min(b.y) && p.y <= a.y.max(b.y);
    let (o1, o2, o3, o4) = (orient(a, b, c), orient(a, b, d), orient(c, d, a), orient(c, d, b));
    (o1 != o2 && o3 != o4) || (o1 == 0 && on(a, b, c)) || (o2 == 0 && on(a, b, d)) || (o3 == 0 && on(c, d, a)) || (o4 == 0 && on(c, d, b))
}
