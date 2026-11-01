//! A uniform grid over the edges of a set of closed rings: ray casts (which rings contain a point),
//! nearest points on one ring, and "does this convex quad touch any ring but these" queries.
//!
//! Plain arithmetic only (no trig), so results are the same on every platform.

use vectorcraft_geom::kurbo::{Point, Rect, Vec2};

/// One ring edge `a`–`b`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Seg {
    pub ring: u32,
    pub a: Point,
    pub b: Point,
}

/// The most grid cells (memory bound).
const MAX_CELLS: usize = 1 << 20;

pub(crate) struct SegIndex {
    segs: Vec<Seg>,
    origin: Point,
    cell: f64,
    nx: usize,
    ny: usize,
    cells: Vec<Vec<u32>>,
    /// Per-edge query stamp (an edge in several cells is visited once per query).
    stamp: Vec<u32>,
    query: u32,
}

impl SegIndex {
    /// The index of `rings` (each closed, at least 3 points).
    pub fn new(rings: &[Vec<Point>]) -> SegIndex {
        let mut segs = Vec::new();
        let mut bounds: Option<Rect> = None;
        for (r, ring) in rings.iter().enumerate() {
            let n = ring.len();
            for i in 0..n {
                let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) else { continue };
                let rid = u32::try_from(r).unwrap_or(u32::MAX);
                segs.push(Seg { ring: rid, a, b });
                let bb = Rect::from_points(a, b);
                bounds = Some(bounds.map_or(bb, |x| x.union(bb)));
            }
        }
        let bounds = bounds.unwrap_or(Rect::ZERO);
        let (w, h) = (bounds.width().max(1e-9), bounds.height().max(1e-9));
        // About two edges per cell.
        let target = (segs.len() / 2).clamp(1, MAX_CELLS) as f64;
        let mut cell = (w * h / target).sqrt().max(w.max(h) / 4096.0).max(1e-9);
        let dims = |cell: f64| (((w / cell).ceil() as usize).max(1), ((h / cell).ceil() as usize).max(1));
        let (mut nx, mut ny) = dims(cell);
        while nx.saturating_mul(ny) > MAX_CELLS {
            cell *= 1.5;
            (nx, ny) = dims(cell);
        }
        let mut idx = SegIndex {
            stamp: vec![0; segs.len()],
            segs,
            origin: Point::new(bounds.x0, bounds.y0),
            cell,
            nx,
            ny,
            cells: vec![Vec::new(); nx * ny],
            query: 0,
        };
        for (i, s) in idx.segs.iter().enumerate() {
            let (x0, y0, x1, y1) = idx.cell_range(Rect::from_points(s.a, s.b));
            let id = u32::try_from(i).unwrap_or(u32::MAX);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    if let Some(c) = idx.cells.get_mut(y * nx + x) {
                        c.push(id);
                    }
                }
            }
        }
        idx
    }

    fn cell_of(&self, v: f64, origin: f64, n: usize) -> usize {
        let c = ((v - origin) / self.cell).floor();
        if c.is_nan() || c < 0.0 { 0 } else { (c as usize).min(n - 1) }
    }

    /// Cells `(x0, y0, x1, y1)` (inclusive) overlapping `r`.
    fn cell_range(&self, r: Rect) -> (usize, usize, usize, usize) {
        (
            self.cell_of(r.x0, self.origin.x, self.nx),
            self.cell_of(r.y0, self.origin.y, self.ny),
            self.cell_of(r.x1, self.origin.x, self.nx),
            self.cell_of(r.y1, self.origin.y, self.ny),
        )
    }

    fn next_query(&mut self) -> u32 {
        self.query = self.query.wrapping_add(1);
        if self.query == 0 {
            self.stamp.iter_mut().for_each(|s| *s = 0);
            self.query = 1;
        }
        self.query
    }

    /// Every edge in the cells overlapping `r`, once each.
    fn edges_near(&mut self, r: Rect, mut f: impl FnMut(&Seg)) {
        let q = self.next_query();
        let (x0, y0, x1, y1) = self.cell_range(r);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let Some(cell) = self.cells.get(y * self.nx + x) else { continue };
                for &id in cell {
                    let i = id as usize;
                    let Some(st) = self.stamp.get_mut(i) else { continue };
                    if *st == q {
                        continue;
                    }
                    *st = q;
                    if let Some(s) = self.segs.get(i) {
                        f(s);
                    }
                }
            }
        }
    }

    /// How many times a ray from `p` towards +x crosses each ring, as `(ring, crossings)` for the
    /// rings it crosses an odd number of times: the rings containing `p` (even-odd), except ones
    /// `p` lies on.
    pub fn containing(&mut self, p: Point) -> Vec<u32> {
        let mut x1 = self.origin.x + self.cell * self.nx as f64;
        if x1.is_nan() || x1 <= p.x {
            x1 = p.x;
        }
        let mut odd: Vec<u32> = Vec::new();
        self.edges_near(Rect::new(p.x, p.y, x1, p.y), |s| {
            let (a, b) = (s.a, s.b);
            if (a.y > p.y) != (b.y > p.y) {
                let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
                if x > p.x {
                    match odd.iter().position(|&r| r == s.ring) {
                        Some(i) => {
                            odd.swap_remove(i);
                        }
                        None => odd.push(s.ring),
                    }
                }
            }
        });
        odd
    }

    /// The nearest point to `p` on the rings `take` accepts, with its distance and ring.
    pub fn nearest_in(&mut self, p: Point, take: impl Fn(usize) -> bool) -> Option<(Point, f64, usize)> {
        let span = self.cell * self.nx.max(self.ny) as f64;
        let mut best: Option<(Point, f64, usize)> = None;
        let mut r = self.cell;
        loop {
            let mut found = best;
            self.edges_near(Rect::new(p.x - r, p.y - r, p.x + r, p.y + r), |s| {
                let ring = s.ring as usize;
                if !take(ring) {
                    return;
                }
                let q = nearest_on_segment(p, s.a, s.b);
                let d = (q - p).hypot();
                // Ties in distance: the lower ring, so the answer doesn't depend on visiting order.
                if found.is_none_or(|(_, bd, br)| d < bd || (d == bd && ring < br)) {
                    found = Some((q, d, ring));
                }
            });
            best = found;
            if best.is_some_and(|(_, d, _)| d <= r) || r > span * 2.0 {
                return best;
            }
            r *= 2.0;
        }
    }

    /// Whether the convex quad `quad` (in order) touches an edge of a ring other than `skip`.
    pub fn quad_hits(&mut self, quad: &[Point; 4], skip: [usize; 2]) -> bool {
        let bb = quad.iter().skip(1).fold(Rect::from_points(quad[0], quad[0]), |b, &p| b.union_pt(p));
        let mut hit = false;
        self.edges_near(bb, |s| {
            if hit || skip.contains(&(s.ring as usize)) {
                return;
            }
            if in_convex(quad, s.a) || in_convex(quad, s.b) {
                hit = true;
                return;
            }
            for i in 0..4 {
                if crosses(quad[i], quad[(i + 1) % 4], s.a, s.b).is_some() {
                    hit = true;
                    return;
                }
            }
        });
        hit
    }

    /// The rings (each once, ascending) with an edge touching the convex quad `quad`.
    pub fn rings_hit(&mut self, quad: &[Point; 4]) -> Vec<u32> {
        let bb = quad.iter().skip(1).fold(Rect::from_points(quad[0], quad[0]), |b, &p| b.union_pt(p));
        let mut out: Vec<u32> = Vec::new();
        self.edges_near(bb, |s| {
            if out.contains(&s.ring) {
                return;
            }
            if in_convex(quad, s.a) || in_convex(quad, s.b) || (0..4).any(|i| crosses(quad[i], quad[(i + 1) % 4], s.a, s.b).is_some()) {
                out.push(s.ring);
            }
        });
        out.sort_unstable();
        out
    }

    /// Whether the open segment `a`–`b` (its ends left out by a margin) crosses an edge of a ring
    /// in `rings`.
    pub fn segment_crosses(&mut self, a: Point, b: Point, rings: [usize; 2]) -> bool {
        let mut hit = false;
        self.edges_near(Rect::from_points(a, b), |s| {
            if hit || !rings.contains(&(s.ring as usize)) {
                return;
            }
            if let Some(t) = crosses(a, b, s.a, s.b) {
                hit = t > 1e-6 && t < 1.0 - 1e-6;
            }
        });
        hit
    }
}

/// The point of segment `a`–`b` nearest to `p`.
pub(crate) fn nearest_on_segment(p: Point, a: Point, b: Point) -> Point {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 <= 0.0 {
        return a;
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    a + ab * t
}

fn cross(u: Vec2, v: Vec2) -> f64 {
    u.x * v.y - u.y * v.x
}

/// Where segment `a`–`b` meets segment `c`–`d`, as the parameter along `a`–`b`; `None` when they
/// don't meet or are parallel.
pub(crate) fn crosses(a: Point, b: Point, c: Point, d: Point) -> Option<f64> {
    let r = b - a;
    let s = d - c;
    let den = cross(r, s);
    if den == 0.0 {
        return None;
    }
    let t = cross(c - a, s) / den;
    let u = cross(c - a, r) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

/// Whether `p` is inside (or on) the convex quad `q`, either orientation.
pub(crate) fn in_convex(q: &[Point; 4], p: Point) -> bool {
    let mut sign = 0.0f64;
    for i in 0..4 {
        let c = cross(q[(i + 1) % 4] - q[i], p - q[i]);
        if c != 0.0 {
            if sign != 0.0 && c.signum() != sign {
                return false;
            }
            sign = c.signum();
        }
    }
    true
}
