//! Stage 6: paper islands. The cut is plain contours (outers and holes, none crossing), so the
//! paper's topology is the cut's contour nesting: a contour's parent is the smallest contour
//! containing it. Even depths (0, 2, ...) are cut outlines; odd depths are holes in the cut, and
//! every hole in the cut is the outline of a paper piece the cut separates from the sheet, an
//! **island**. Its nesting depth counts the islands around it.
//!
//! Containment is a ray cast per contour through a grid of the edges ([`SegIndex::containing`]),
//! so the tree costs about one ray per contour, not one point-in-polygon test per pair.

use vectorcraft_geom::kurbo::{BezPath, Point, Rect};

use crate::index::SegIndex;
use crate::rings;

/// The most contour points the island tree takes (beyond it no islands are reported).
pub(crate) const MAX_TREE_POINTS: usize = 4_000_000;

/// The cut's contours with their nesting.
pub(crate) struct Tree {
    pub rings: Vec<Vec<Point>>,
    pub bounds: Vec<Rect>,
    /// The smallest contour containing each one.
    pub parent: Vec<Option<usize>>,
    /// 0 for outermost cut outlines; odd = a hole in the cut (an island's outline).
    pub depth: Vec<u32>,
    pub index: SegIndex,
}

/// Signed shoelace area of a ring.
pub(crate) fn ring_area(ring: &[Point]) -> f64 {
    let n = ring.len();
    let mut a = 0.0;
    for i in 0..n {
        if let (Some(p), Some(q)) = (ring.get(i), ring.get((i + 1) % n)) {
            a += p.x * q.y - q.x * p.y;
        }
    }
    a * 0.5
}

/// The midpoint of the longest edge of `ring`: a point on the ring away from its corners (where
/// another contour may touch it).
fn probe(ring: &[Point]) -> Option<Point> {
    let n = ring.len();
    let mut best: Option<(f64, Point)> = None;
    for i in 0..n {
        let (Some(&a), Some(&b)) = (ring.get(i), ring.get((i + 1) % n)) else { continue };
        let l = (b - a).hypot2();
        if best.is_none_or(|(bl, _)| l > bl) {
            best = Some((l, a.midpoint(b)));
        }
    }
    best.map(|(_, p)| p)
}

impl Tree {
    /// The tree of `cut`'s contours (polygonal, plain). `None` when it has more than
    /// [`MAX_TREE_POINTS`] points.
    pub fn new(cut: &BezPath) -> Option<Tree> {
        let rings = rings::rings(cut, 1e-3, MAX_TREE_POINTS)?;
        Some(Self::of_rings(rings))
    }

    pub fn of_rings(rings: Vec<Vec<Point>>) -> Tree {
        let n = rings.len();
        let areas: Vec<f64> = rings.iter().map(|r| ring_area(r).abs()).collect();
        let bounds: Vec<Rect> =
            rings.iter().map(|r| r.iter().skip(1).fold(r.first().map_or(Rect::ZERO, |&p| Rect::from_points(p, p)), |b, &p| b.union_pt(p))).collect();
        let mut index = SegIndex::new(&rings);
        let mut parent = vec![None; n];
        for i in 0..n {
            let Some(p) = rings.get(i).and_then(|r| probe(r)) else { continue };
            let own = areas.get(i).copied().unwrap_or(0.0);
            let mut best: Option<(f64, usize)> = None;
            for r in index.containing(p) {
                let j = r as usize;
                let a = areas.get(j).copied().unwrap_or(0.0);
                // Larger than this one (equal areas: the earlier contour, so no cycles).
                if j != i && (a > own || (a == own && j < i)) && best.is_none_or(|(ba, bj)| a < ba || (a == ba && j > bj)) {
                    best = Some((a, j));
                }
            }
            if let Some(slot) = parent.get_mut(i) {
                *slot = best.map(|(_, j)| j);
            }
        }
        // Depths, parents first (a parent is always larger).
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| areas.get(b).partial_cmp(&areas.get(a)).unwrap_or(std::cmp::Ordering::Equal).then(a.cmp(&b)));
        let mut depth = vec![0u32; n];
        for &i in &order {
            let d = parent.get(i).copied().flatten().and_then(|p| depth.get(p).copied()).map_or(0, |d| d + 1);
            if let Some(slot) = depth.get_mut(i) {
                *slot = d;
            }
        }
        Tree { rings, bounds, parent, depth, index }
    }

    /// The islands: holes in the cut (odd depth), outermost first (then by contour order).
    pub fn islands(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.rings.len()).filter(|&i| self.depth.get(i).is_some_and(|d| d % 2 == 1)).collect();
        v.sort_by_key(|&i| (self.depth.get(i).copied().unwrap_or(0), i));
        v
    }

    /// An island's nesting depth: 0 inside a cut-out of the sheet, 1 inside one of an island...
    pub fn nested_depth(&self, ring: usize) -> u32 {
        self.depth.get(ring).copied().unwrap_or(0) / 2
    }

    /// The deepest contour containing `p` (`None`: on the sheet's paper or outside every contour).
    pub fn deepest_containing(&mut self, p: Point) -> Option<usize> {
        let depth = &self.depth;
        self.index.containing(p).into_iter().map(|r| r as usize).max_by_key(|&r| (depth.get(r).copied().unwrap_or(0), r))
    }

    /// A point inside island `ring`'s paper, just inside its outline (off the middle of its
    /// longest edge).
    pub fn inside_point(&mut self, ring: usize) -> Option<Point> {
        let r = self.rings.get(ring)?;
        let n = r.len();
        let (mut best_len, mut at) = (0.0, 0);
        for i in 0..n {
            let (Some(&a), Some(&b)) = (r.get(i), r.get((i + 1) % n)) else { continue };
            let l = (b - a).hypot2();
            if l > best_len {
                (best_len, at) = (l, i);
            }
        }
        let (&a, &b) = (r.get(at)?, r.get((at + 1) % n)?);
        let m = a.midpoint(b);
        let d = b - a;
        let len = d.hypot();
        if len.is_nan() || len <= 0.0 {
            return None;
        }
        // A thousandth of the edge's length to its side.
        let normal = vectorcraft_geom::kurbo::Vec2::new(-d.y, d.x) * 1e-3;
        for cand in [m + normal, m - normal] {
            if self.index.containing(cand).contains(&u32::try_from(ring).ok()?) {
                return Some(cand);
            }
        }
        None
    }
}
