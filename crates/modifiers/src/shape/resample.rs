//! Resample: subdivide every segment, or rebuild a subpath with points spaced evenly along it.
//!
//! Ported from tlprocedural's Subdivide (`subdivideRing`) and Rebuild Points (`resamplePolyline`,
//! count and spacing modes). Differences, on purpose: linear subdivision splits curves at their
//! middle instead of flattening them (the shape stays exactly the same), Catmull-Rom subdivision
//! keeps the anchors and adds the spline's midpoints between them (tlprocedural's replaced the
//! anchors with the midpoints, halving nothing and doubling nothing), and count and spacing
//! measure the true curve rather than a flattened outline.

use vectorcraft_geom::{Point, SubPath};

use super::pieces::{Measured, from_pieces, pieces};
use crate::model::{Resample, ResampleMode, SubdivideMethod};

/// Ends this close (points) count as one point when a closed subpath's anchors are read as a
/// polygon (tlprocedural's tolerance for a ring's repeated first point).
const SAME_POINT: f64 = 0.5;
/// A subpath shorter than this (points) is left as it is.
const TOO_SHORT: f64 = 1e-6;

/// How many anchors resampling `sp` with `p` (clamped) makes, without making them; `None` when it
/// is left as it is.
pub(super) fn count(sp: &SubPath, p: &Resample) -> Option<usize> {
    let n = sp.anchors.len();
    if n < 2 {
        return None;
    }
    match p.mode {
        ResampleMode::Subdivide => {
            let n = polygon_len(sp, p.method);
            let mut k = n;
            for _ in 0..p.iterations {
                k = if sp.closed { k.saturating_mul(2) } else { k.saturating_mul(2).saturating_sub(1) };
            }
            Some(k)
        }
        ResampleMode::Count => {
            let total = Measured::new(pieces(sp)).total;
            (total > TOO_SHORT).then(|| (p.count as usize).max(if sp.closed { 3 } else { 2 }))
        }
        ResampleMode::Spacing => {
            let total = Measured::new(pieces(sp)).total;
            if total <= TOO_SHORT {
                return None;
            }
            let steps = (total / p.spacing.max(1.0)).ceil();
            // Huge counts are refused by the caller's budget; a float that big just saturates.
            let steps = if steps.is_finite() && steps < 1e15 { steps as usize } else { usize::MAX };
            Some(if sp.closed { steps.max(3) } else { steps.saturating_add(1) })
        }
    }
}

/// How many anchors the subdivision starts from (a closed subpath's repeated end dropped for the
/// polygon methods).
fn polygon_len(sp: &SubPath, method: SubdivideMethod) -> usize {
    match method {
        SubdivideMethod::Linear => sp.anchors.len(),
        _ => polygon(sp).len(),
    }
}

/// `sp` resampled by `p` (clamped); `None`: left as it is.
pub(super) fn resample(sp: &SubPath, p: &Resample) -> Option<SubPath> {
    if sp.anchors.len() < 2 {
        return None;
    }
    match p.mode {
        ResampleMode::Subdivide => Some(subdivide(sp, p.method, p.iterations)),
        ResampleMode::Count => by_count(sp, p.count as usize),
        ResampleMode::Spacing => by_spacing(sp, p.spacing),
    }
}

fn subdivide(sp: &SubPath, method: SubdivideMethod, iterations: u32) -> SubPath {
    match method {
        SubdivideMethod::Linear => {
            let mut ps = pieces(sp);
            for _ in 0..iterations {
                ps = ps
                    .iter()
                    .flat_map(|p| {
                        let (a, b) = p.halves();
                        [a, b]
                    })
                    .collect();
            }
            from_pieces(&ps, sp.closed)
        }
        SubdivideMethod::CatmullRom | SubdivideMethod::Chaikin => {
            let mut pts = polygon(sp);
            for _ in 0..iterations {
                pts = if method == SubdivideMethod::Chaikin { chaikin(&pts, sp.closed) } else { catmull_rom(&pts, sp.closed) };
            }
            SubPath::polyline(&pts, sp.closed)
        }
    }
}

/// `sp`'s anchor points as a polygon (a closed subpath's last point dropped when it repeats the
/// first).
fn polygon(sp: &SubPath) -> Vec<Point> {
    let mut pts: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
    if sp.closed && pts.len() >= 2 && pts.first().zip(pts.last()).is_some_and(|(a, b)| a.distance(*b) <= SAME_POINT) {
        pts.pop();
    }
    pts
}

/// The segments of polygon `pts`: `(from, to)` index pairs.
fn segments(n: usize, closed: bool) -> impl Iterator<Item = (usize, usize)> {
    let count = if n < 2 {
        0
    } else if closed {
        n
    } else {
        n - 1
    };
    (0..count).map(move |i| (i, (i + 1) % n))
}

/// Chaikin's corner cutting, once: each segment's quarter and three-quarter points (an open
/// polygon keeps its last point, as tlprocedural's).
fn chaikin(pts: &[Point], closed: bool) -> Vec<Point> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for (i, j) in segments(pts.len(), closed) {
        let (Some(a), Some(b)) = (pts.get(i), pts.get(j)) else { continue };
        out.push(a.lerp(*b, 0.25));
        out.push(a.lerp(*b, 0.75));
    }
    if !closed && let Some(last) = pts.last() {
        out.push(*last);
    }
    out
}

/// Each anchor, then the Catmull-Rom spline's point halfway to the next (an open polygon's ends
/// repeat themselves as the phantom neighbours, as tlprocedural's).
fn catmull_rom(pts: &[Point], closed: bool) -> Vec<Point> {
    let n = pts.len();
    let mut out = Vec::with_capacity(n * 2);
    let at = |i: usize| pts.get(i).copied().unwrap_or_default();
    for (i, j) in segments(n, closed) {
        let (p0, p1) = (at(i), at(j));
        let prev = if closed { at((i + n - 1) % n) } else { at(i.saturating_sub(1)) };
        let next = if closed { at((i + 2) % n) } else { at((i + 2).min(n - 1)) };
        out.push(p0);
        out.push(catmull_mid(prev, p0, p1, next));
    }
    if !closed && let Some(last) = pts.last() {
        out.push(*last);
    }
    out
}

/// The uniform Catmull-Rom spline through `p0` and `p1` at `t = 0.5`.
fn catmull_mid(prev: Point, p0: Point, p1: Point, next: Point) -> Point {
    let (t, t2, t3) = (0.5, 0.25, 0.125);
    let f =
        |a: f64, b: f64, c: f64, d: f64| 0.5 * (2.0 * b + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3);
    Point::new(f(prev.x, p0.x, p1.x, next.x), f(prev.y, p0.y, p1.y, next.y))
}

/// `count` points evenly spaced along `sp` (a closed one: at least 3, the gap back to the first
/// as long as the others; an open one: both ends kept), joined by straight segments.
fn by_count(sp: &SubPath, count: usize) -> Option<SubPath> {
    let m = Measured::new(pieces(sp));
    if m.total <= TOO_SHORT {
        return None;
    }
    let pts: Vec<Point> = if sp.closed {
        let n = count.max(3);
        let step = m.total / n as f64;
        (0..n).filter_map(|i| m.point_at(i as f64 * step)).collect()
    } else {
        let n = count.max(2);
        let step = m.total / (n - 1) as f64;
        (0..n).filter_map(|i| m.point_at(i as f64 * step)).collect()
    };
    Some(SubPath::polyline(&pts, sp.closed))
}

/// A point every `spacing` along `sp` from its start (a closed one: at least 3, the last gap
/// shorter; an open one ends at its end), joined by straight segments.
fn by_spacing(sp: &SubPath, spacing: f64) -> Option<SubPath> {
    let m = Measured::new(pieces(sp));
    if m.total <= TOO_SHORT {
        return None;
    }
    let step = spacing.max(1.0);
    let mut pts = Vec::new();
    if sp.closed {
        // The caller's budget refuses more beforehand ([`count`]); the cap only keeps this bounded.
        let n = ((m.total / step).ceil().min(super::MAX_GEOMETRY_ANCHORS as f64) as usize).max(3);
        // Fewer than three steps fit: they spread evenly instead.
        let step = if (n as f64) * step > m.total + step { m.total / n as f64 } else { step };
        pts.extend((0..n).filter_map(|i| m.point_at(i as f64 * step)));
    } else {
        let mut i = 0usize;
        while (i as f64) * step < m.total && i < super::MAX_GEOMETRY_ANCHORS {
            pts.extend(m.point_at(i as f64 * step));
            i += 1;
        }
        if let Some(end) = m.point_at(m.total)
            && pts.last().is_none_or(|l| l.distance(end) > 1e-6)
        {
            pts.push(end);
        }
    }
    Some(SubPath::polyline(&pts, sp.closed))
}
