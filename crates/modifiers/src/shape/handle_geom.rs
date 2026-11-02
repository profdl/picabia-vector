//! What the canvas handles of the geometry rows need to know about the object they reshape
//! ([`crate::handles`]): Trim's first subpath measured by arc length, and Deform's box and Bulge's
//! extent, read exactly as the rows read them.

use vectorcraft_doc::Node;
use vectorcraft_geom::{Affine, Point, Rect, SubPath};

use super::pieces::{Measured, pieces};
use crate::model::Deform;

/// A subpath with more anchors than this gets no Trim handles (measuring it on every pointer move
/// would be slow).
pub const MAX_TRIM_HANDLE_ANCHORS: usize = 20_000;
/// Points sampled along the subpath when finding the fraction nearest the pointer.
const SAMPLES: usize = 512;
/// Refinement steps around the nearest sample.
const REFINE: usize = 24;

/// The first subpath Trim cuts that has a length: the first of the object's plain paths' subpaths
/// with two anchors or more.
pub(crate) struct TrimPath {
    m: Measured,
    pub closed: bool,
}

impl TrimPath {
    pub(crate) fn of(n: &Node) -> Option<TrimPath> {
        let mut paths = Vec::new();
        super::deform::paths_of(n, 0, &mut paths);
        let sp: &SubPath = paths.iter().flat_map(|p| &p.subpaths).find(|s| s.anchors.len() >= 2)?;
        if sp.anchors.len() > MAX_TRIM_HANDLE_ANCHORS {
            return None;
        }
        let m = Measured::new(pieces(sp));
        (m.total.is_finite() && m.total > 1e-9).then_some(TrimPath { m, closed: sp.closed })
    }

    /// The point at fraction `f` (0..=1) of the length.
    pub(crate) fn at(&self, f: f64) -> Option<Point> {
        let f = if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 };
        self.m.point_at(f * self.m.total).filter(|p| p.x.is_finite() && p.y.is_finite())
    }

    /// The fraction of the length nearest `q`.
    pub(crate) fn nearest(&self, q: Point) -> Option<f64> {
        if !(q.x.is_finite() && q.y.is_finite()) {
            return None;
        }
        let dist = |f: f64| self.at(f).map_or(f64::INFINITY, |p| p.distance(q));
        let (mut best, mut best_d) = (0.0, f64::INFINITY);
        for k in 0..=SAMPLES {
            let f = k as f64 / SAMPLES as f64;
            let d = dist(f);
            if d < best_d {
                (best, best_d) = (f, d);
            }
        }
        // Ternary search in the samples on either side.
        let step = 1.0 / SAMPLES as f64;
        let (mut lo, mut hi) = ((best - step).max(0.0), (best + step).min(1.0));
        for _ in 0..REFINE {
            let (a, b) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
            if dist(a) <= dist(b) {
                hi = b;
            } else {
                lo = a;
            }
        }
        let f = (lo + hi) / 2.0;
        Some(if dist(f) <= best_d { f } else { best })
    }
}

/// Deform's box (the paths' bounds) and, in Bulge mode, how far the paths reach from its centre:
/// what the row reads for the object itself.
pub(crate) fn deform_box(n: &Node, p: &Deform) -> Option<(Rect, f64)> {
    super::deform::measure(n, p, Affine::IDENTITY)
}
