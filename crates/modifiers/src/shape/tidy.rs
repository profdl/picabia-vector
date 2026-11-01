//! Tidy: close open subpaths, then sort or reverse each subpath's anchors.
//!
//! Ported from tlprocedural's Close Path (`closeRing`) and Sort Points (`sortRing`). Differences,
//! on purpose: ends within the tolerance become one anchor (tlprocedural snapped the last point
//! onto the first and kept both); a smooth close is a curve tangent to both ends (tlprocedural
//! added a nudged midpoint as a polygon vertex); a random sort is a seeded shuffle (tlprocedural
//! sorted with a random comparator, whose result depends on the browser); and `sort: none` with
//! `reverse` reverses the subpath's direction, keeping its curves.

use vectorcraft_geom::{Anchor, Point, SubPath, Vec2};

use crate::kinds::rng_stream;
use crate::model::{CloseMethod, SortBy, Tidy};

/// Ends this close count as the same point when anchors are read as a polygon (tlprocedural's
/// ring tolerance).
const SAME_POINT: f64 = 0.5;

/// `sp` tidied by `p` (clamped); `None`: left as it is.
pub(super) fn tidy(sp: &SubPath, p: &Tidy) -> Option<SubPath> {
    let closed = close(sp, p.close, p.tolerance);
    let cur = closed.as_ref().unwrap_or(sp);
    let sorted = order(cur, p.sort, p.reverse, p.seed);
    sorted.or(closed)
}

/// `sp` closed by `method`; `None` when it is closed already, has fewer than two anchors, or
/// `method` is off.
fn close(sp: &SubPath, method: CloseMethod, tolerance: f64) -> Option<SubPath> {
    if sp.closed || method == CloseMethod::Off || sp.anchors.len() < 2 {
        return None;
    }
    let mut anchors = sp.anchors.clone();
    let (first, last) = (*anchors.first()?, *anchors.last()?);
    if first.p.distance(last.p) <= tolerance {
        // The ends are one point: the last anchor goes, its in handle moving to the first.
        if anchors.len() < 3 {
            return None;
        }
        anchors.pop();
        if let Some(a) = anchors.first_mut() {
            a.h_in = last.h_in + (first.p - last.p);
        }
        return Some(SubPath::new(anchors, true));
    }
    let n = anchors.len();
    let (h_out, h_in) = if method == CloseMethod::Smooth && n >= 3 {
        // A curve carrying on from the last anchor's direction into the first's, a third of the
        // gap long at each end.
        let reach = first.p.distance(last.p) / 3.0;
        let into_last = direction(if last.has_in() { last.h_in } else { anchors.get(n - 2).map_or(last.p, |a| a.p) }, last.p);
        let out_of_first = direction(first.p, if first.has_out() { first.h_out } else { anchors.get(1).map_or(first.p, |a| a.p) });
        (last.p + into_last * reach, first.p - out_of_first * reach)
    } else {
        (last.p, first.p)
    };
    if let Some(a) = anchors.last_mut() {
        a.h_out = h_out;
    }
    if let Some(a) = anchors.first_mut() {
        a.h_in = h_in;
    }
    Some(SubPath::new(anchors, true))
}

/// The unit direction from `a` to `b` (zero when they meet).
fn direction(a: Point, b: Point) -> Vec2 {
    let d = b - a;
    let len = d.hypot();
    if len > 1e-12 && len.is_finite() { d / len } else { Vec2::ZERO }
}

/// `sp`'s anchors sorted by `by` (then reversed with `reverse`), joined by straight segments; with
/// `by: none`, `reverse` reverses its direction. `None`: left as it is.
fn order(sp: &SubPath, by: SortBy, reverse: bool, seed: u32) -> Option<SubPath> {
    if sp.anchors.len() < 2 {
        return None;
    }
    if by == SortBy::None {
        return reverse.then(|| reversed(sp));
    }
    let mut pts: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
    if sp.closed && pts.len() >= 3 && pts.first().zip(pts.last()).is_some_and(|(a, b)| a.distance(*b) <= SAME_POINT) {
        pts.pop();
    }
    let n = pts.len() as f64;
    let c = pts.iter().fold(Point::ZERO, |acc, p| acc + p.to_vec2() / n);
    let key = |p: &Point| -> f64 {
        match by {
            SortBy::Angle => (p.y - c.y).atan2(p.x - c.x),
            SortBy::Distance => p.distance(c),
            SortBy::X => p.x,
            SortBy::Y => p.y,
            SortBy::None | SortBy::Random => 0.0,
        }
    };
    if by == SortBy::Random {
        shuffle(&mut pts, seed);
    } else {
        // Stable: equal keys keep their order (tlprocedural's tie-break on the index).
        pts.sort_by(|a, b| key(a).total_cmp(&key(b)));
    }
    if reverse {
        pts.reverse();
    }
    Some(SubPath::polyline(&pts, sp.closed))
}

/// `sp` running the other way, its curves kept.
pub(super) fn reversed(sp: &SubPath) -> SubPath {
    let anchors = sp.anchors.iter().rev().map(|a| Anchor { p: a.p, h_in: a.h_out, h_out: a.h_in, kind: a.kind }).collect();
    SubPath::new(anchors, sp.closed)
}

/// A seeded Fisher-Yates shuffle (the crate's deterministic stream: the same on every platform).
fn shuffle(pts: &mut [Point], seed: u32) {
    let mut rng = rng_stream(seed, 0);
    for i in (1..pts.len()).rev() {
        let unit = (rng.signed() + 1.0) / 2.0;
        let j = ((unit * (i + 1) as f64) as usize).min(i);
        pts.swap(i, j);
    }
}
