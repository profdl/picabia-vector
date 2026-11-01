//! Offsetting filled regions (Picabia): grow or shrink a closed region, holes included, by a
//! distance, and the morphological opening and closing built on it.
//!
//! The region is first reduced to plain contours ([`simplify`], non-zero), so self-intersections,
//! overlapping subpaths and zero-area pieces are resolved before anything is measured. The band of
//! points within `d` of the boundary is then built exactly: the union of one rectangle per boundary
//! segment (the segment swept `±d` along its normal) and, at each vertex, a join wedge on the
//! outside of the turn (a circular fan for round joins, the mitre or bevel otherwise), written as
//! two offset loops per contour whose winding adds up to that union (see [`band`]). `linesweeper`
//! reads it non-zero in the same sweep that adds it to (grow) or removes it from (shrink) the
//! region.
//!
//! A plain stroke outline is no substitute: wherever the region is narrower than the stroke its
//! inner offset turns inside out and cancels the cover, which is exactly the case opening and
//! closing exist for. Curves are flattened (see [`tolerance_for`]) and the result is polygonal,
//! even-odd.
//!
//! Ported from picabia-craft's `pathops` (PIC8.1) into the stencil crate rather than VectorCraft's
//! `pathops`: VectorCraft's `offset_path` (Object › Path › Offset Path) strokes the outline and
//! then drops spurious contours by measuring a deep point of each against the whole source, which
//! costs a pass over the source per contour on traced art, and returns refitted curves; the
//! stencil's clean-up runs three offsets per sheet and its tests pin this band construction.

use crate::ops::StrokeJoin;
use vectorcraft_geom::kurbo::{BezPath, PathEl, Point, Shape, Vec2};

use crate::ops::{PathOpsError, binary, finite, simplify};

/// The most path elements [`offset`] accepts (traced art runs to tens of thousands; past this the
/// sweep's cost is no longer interactive). Larger input is refused with [`PathOpsError::BadInput`].
pub const MAX_OFFSET_ELEMENTS: usize = 500_000;

/// Mitre limit used for [`StrokeJoin::Miter`] (the SVG default, as a ratio of the distance): a
/// sharper corner is bevelled, so a grown spike can't reach far past the shape.
pub const OFFSET_MITER_LIMIT: f64 = 4.0;

/// The largest offset distance (points); larger magnitudes are clamped.
pub const MAX_OFFSET_DISTANCE: f64 = 5.0e5;

/// Most chords in one round-join fan (a half turn), whatever the distance and tolerance.
const MAX_FAN_STEPS: usize = 32;

/// Grow (`distance > 0`) or shrink (`distance < 0`) the region `path` fills (non-zero, open
/// subpaths closed) by `|distance|`, with `join` shaping the corners the offset goes round (outer
/// corners when growing, inner ones when shrinking). Holes shrink when the region grows and grow
/// when it shrinks; a hole narrower than `2·distance` closes, a part narrower than `2·|distance|`
/// vanishes. `distance == 0` returns the region simplified. The result is even-odd.
///
/// Errors: [`PathOpsError::BadInput`] for non-finite coordinates or distance, or more than
/// [`MAX_OFFSET_ELEMENTS`] elements.
pub fn offset(path: &BezPath, distance: f64, join: StrokeJoin) -> Result<BezPath, PathOpsError> {
    if !distance.is_finite() || !finite(path) || path.elements().len() > MAX_OFFSET_ELEMENTS {
        return Err(PathOpsError::BadInput);
    }
    let region = simplify(path)?;
    if distance == 0.0 || region.elements().is_empty() {
        return Ok(region);
    }
    let d = distance.abs().min(MAX_OFFSET_DISTANCE);
    let tolerance = tolerance_for(d);
    let op = if distance > 0.0 { linesweeper::BinaryOp::Union } else { linesweeper::BinaryOp::Difference };
    let band = band(&region, d, tolerance, join);
    if band.elements().is_empty() {
        return Ok(region);
    }
    // linesweeper 0.4 can trip on a near-degenerate meeting of edges (ends at almost the same
    // height), and many edges meet at every boundary vertex here. Snapping every coordinate to a
    // grid far under the tolerance turns "almost" into "exactly"; if the sweep still fails, coarser
    // grids follow (tolerance / 1024, / 128, / 16: never more than 1/32 of the tolerance off).
    // On wasm32 a failing sweep aborts instead (see `sweep`), so the first grid is the one that
    // must hold: the seeded sweep in the tests found no input it failed on.
    let mut last = Err(PathOpsError::BadInput);
    for attempt in 0..ATTEMPTS {
        let (mut base, mut band) = (region.clone(), band.clone());
        let q = tolerance / f64::from(1024u32 >> (3 * attempt));
        snap(&mut base, q);
        snap(&mut band, q);
        last = binary(&base, &band, op);
        if let Ok(out) = &last {
            return Ok(drop_slivers(out, tolerance * tolerance * 0.01));
        }
    }
    last
}

/// Round every coordinate of `path` to a multiple of `q`.
fn snap(path: &mut BezPath, q: f64) {
    let s = |p: &mut Point| {
        p.x = (p.x / q).round() * q;
        p.y = (p.y / q).round() * q;
    };
    for el in path.elements_mut() {
        match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => s(p),
            PathEl::QuadTo(a, b) => {
                s(a);
                s(b);
            }
            PathEl::CurveTo(a, b, c) => {
                s(a);
                s(b);
                s(c);
            }
            PathEl::ClosePath => {}
        }
    }
}

/// Sweeps tried before [`offset`] gives up.
const ATTEMPTS: u32 = 3;

/// Flattening tolerance for distance `d`: 0.5% of it, between 1e-4 and 0.05 units.
fn tolerance_for(d: f64) -> f64 {
    (d * 0.005).clamp(1e-4, 0.05)
}

/// `path` without the subpaths enclosing less than `min_area`. Band edges meet the region's own
/// at every boundary vertex and the sweep can leave zero-width slivers there; nothing that small can contain
/// another contour, so the even-odd nesting of the rest is unchanged.
fn drop_slivers(path: &BezPath, min_area: f64) -> BezPath {
    let mut out = BezPath::new();
    let mut sub = BezPath::new();
    let mut flush = |sub: &mut BezPath| {
        if sub.area().abs() >= min_area {
            out.extend(sub.elements().iter().copied());
        }
        *sub = BezPath::new();
    };
    for el in path.elements() {
        if matches!(el, PathEl::MoveTo(_)) {
            flush(&mut sub);
        }
        sub.push(*el);
    }
    flush(&mut sub);
    out
}

/// Morphological opening: shrink by `r`, then grow by `r` (round joins). Parts and spurs narrower
/// than `2·r` disappear; wider parts keep their shape (convex corners rounded to radius `r`).
/// `r ≤ 0` returns the region simplified.
pub fn open(path: &BezPath, r: f64) -> Result<BezPath, PathOpsError> {
    morph(path, r, true)
}

/// Morphological closing: grow by `r`, then shrink by `r` (round joins). Gaps, notches and holes
/// narrower than `2·r` fill in; the outline elsewhere stays put (concave corners rounded to radius
/// `r`). `r ≤ 0` returns the region simplified.
pub fn close(path: &BezPath, r: f64) -> Result<BezPath, PathOpsError> {
    morph(path, r, false)
}

fn morph(path: &BezPath, r: f64, opening: bool) -> Result<BezPath, PathOpsError> {
    if !r.is_finite() {
        return Err(PathOpsError::BadInput);
    }
    let r = r.max(0.0);
    let first = if opening { -r } else { r };
    let mid = offset(path, first, StrokeJoin::Round)?;
    offset(&mid, -first, StrokeJoin::Round)
}

/// The points within `d` of the boundary of the simple region `region`, as a non-zero fill.
///
/// Each ring gives two loops, its right-hand offset traced forwards and its left-hand offset
/// traced backwards. On the inside of a turn a loop passes through the vertex itself; on the
/// outside it goes round the join (fan, mitre or bevel). Summed, the two loops wind exactly like
/// the sum of every segment's rectangle (the segment swept `±d` along its normal) and every
/// join's wedge, all wound positively, so the band covers their union and nothing else, however
/// far the offsets fold over. (A stroker that cuts inner corners short, as kurbo's does, adds
/// negatively wound triangles there instead, which cancel the cover wherever the region is
/// narrower than the stroke.)
fn band(region: &BezPath, d: f64, tolerance: f64, join: StrokeJoin) -> BezPath {
    // Chord angle whose sagitta on a radius-`d` arc is `tolerance`.
    let step = (2.0 * (1.0 - tolerance / d).clamp(-1.0, 1.0).acos()).max(std::f64::consts::PI / MAX_FAN_STEPS as f64);
    let rot = |v: Vec2, t: f64| {
        let (s, c) = t.sin_cos();
        Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
    };
    let mut out = BezPath::new();
    let mut right: Vec<Point> = Vec::new();
    let mut left: Vec<Point> = Vec::new();
    for ring in rings(region, tolerance) {
        let n = ring.len();
        // Unit directions of the segments ring[i] → ring[i + 1].
        let dirs: Vec<Vec2> = (0..n).filter_map(|i| Some((*ring.get((i + 1) % n)? - *ring.get(i)?).normalize())).collect();
        if dirs.len() != n || dirs.iter().any(|u| !u.is_finite()) {
            continue;
        }
        right.clear();
        left.clear();
        for i in 0..n {
            let (Some(&u), Some(&v), Some(&b)) = (dirs.get(i), dirs.get((i + 1) % n), ring.get((i + 1) % n)) else { continue };
            // Left normals of the segment ending at b and the one leaving it.
            let (nu, nv) = (Vec2::new(-u.y, u.x) * d, Vec2::new(-v.y, v.x) * d);
            right.push(b - nu);
            left.push(b + nu);
            let turn = u.cross(v).atan2(u.dot(v));
            if turn.abs() >= 1e-9 {
                // The outside of a left turn is on the right.
                let (outer, inner, o) = if turn > 0.0 { (&mut right, &mut left, -nu) } else { (&mut left, &mut right, nu) };
                inner.push(b);
                match join {
                    StrokeJoin::Round => {
                        let steps = ((turn.abs() / step).ceil() as usize).clamp(1, MAX_FAN_STEPS);
                        outer.extend((1..steps).map(|k| b + rot(o, turn * k as f64 / steps as f64)));
                    }
                    StrokeJoin::Miter if 1.0 / (turn / 2.0).cos() <= OFFSET_MITER_LIMIT => {
                        outer.push(b + rot(o, turn / 2.0) / (turn / 2.0).cos());
                    }
                    StrokeJoin::Miter | StrokeJoin::Bevel => {}
                }
            }
            right.push(b - nv);
            left.push(b + nv);
        }
        push_loop(&mut out, right.iter());
        push_loop(&mut out, left.iter().rev());
    }
    out
}

fn push_loop<'a>(out: &mut BezPath, mut pts: impl Iterator<Item = &'a Point>) {
    let Some(first) = pts.next() else { return };
    out.move_to(*first);
    for p in pts {
        out.line_to(*p);
    }
    out.close_path();
}

/// Each subpath of `path` flattened to within `tolerance`, as a ring of distinct points (closing
/// duplicate and repeated points dropped). Rings of fewer than two points are skipped.
fn rings(path: &BezPath, tolerance: f64) -> Vec<Vec<Point>> {
    let mut out: Vec<Vec<Point>> = Vec::new();
    let mut cur: Vec<Point> = Vec::new();
    let mut finish = |cur: &mut Vec<Point>| {
        while cur.len() > 1 && cur.first() == cur.last() {
            cur.pop();
        }
        if cur.len() >= 2 {
            out.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    vectorcraft_geom::kurbo::flatten(path.iter(), tolerance, |el| match el {
        PathEl::MoveTo(p) => {
            finish(&mut cur);
            cur.push(p);
        }
        PathEl::LineTo(p) => {
            if cur.last() != Some(&p) {
                cur.push(p);
            }
        }
        PathEl::ClosePath => finish(&mut cur),
        PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
    });
    finish(&mut cur);
    out
}
