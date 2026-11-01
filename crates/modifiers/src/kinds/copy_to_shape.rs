//! `copyToShape`.
//!
//! Places copies of the previous set at points of the target's outline (in the layer's parent
//! space, from a [`TargetResolver`]). Copy `k` at point `P_k` with direction `θ_k` is
//! `T_k = translate(P_k − pivot) · rotate_about(align ? θ_k : 0, pivot) · scale_about(scale, pivot)`:
//! the pivot lands on the point. The original isn't kept (no identity copy).
//!
//! - `points`: one copy per anchor (segment end points; a closed subpath's closing point once),
//!   turned to the outgoing segment (an open subpath's last anchor: the incoming one).
//! - `alongPath`: `count` copies evenly spaced by arc length, over all subpaths in order. When
//!   every subpath is closed the spacing is `length / count` (no copy doubles the first);
//!   otherwise `length / (count − 1)`, so both ends get one.
//! - `populate`: `count` copies at seeded pseudo-random points inside the outline (nonzero
//!   winding), by rejection sampling in its bounding box; `align` doesn't apply.

use vectorcraft_geom::kurbo::{ParamCurve, ParamCurveArclen, ParamCurveDeriv, PathEl, PathSeg};
use vectorcraft_geom::{Affine, BezPath, Point, Shape, Vec2};

use super::{Clamp, Ctx, Stage, StageResult, rng};
use crate::MAX_COUNT;
use crate::eval::Guide;
use crate::model::{CopyMode, CopyToShape};
use crate::target::TargetResolver;

/// Arc-length accuracy, in points.
const ARCLEN_ACCURACY: f64 = 1e-3;
/// The most `contains` tests × path elements populate spends.
const POPULATE_WORK: usize = 20_000_000;
/// The most guide points shown.
const MAX_GUIDE_POINTS: usize = 256;

pub(crate) fn stage(p: &CopyToShape, ctx: &Ctx, targets: &dyn TargetResolver) -> StageResult {
    let mut warnings = Vec::new();
    if p.target.is_empty() {
        warnings.push("choose a target layer to copy onto".to_string());
        return StageResult { stage: Stage::Unchanged, guides: Vec::new(), warnings };
    }
    let shape = match targets.resolve(&p.target) {
        Ok(s) => s,
        Err(e) => {
            warnings.push(e);
            return StageResult { stage: Stage::Unchanged, guides: Vec::new(), warnings };
        }
    };
    let mut c = Clamp { warnings: &mut warnings };
    let scale = c.scale("scale", p.scale);
    let mut places = match p.mode {
        CopyMode::Points => anchors(&shape.outline),
        CopyMode::AlongPath => along(&shape.outline, c.count("count", p.count)),
        CopyMode::Populate => {
            let count = c.count("count", p.count);
            populate(&shape.outline, count, p.seed, &mut warnings)
        }
    };
    let cap = usize::try_from(MAX_COUNT).unwrap_or(usize::MAX);
    if places.len() > cap {
        warnings.push(format!("the target has {} points; copies are limited to {MAX_COUNT}", places.len()));
        places.truncate(cap);
    }
    places.retain(|(pt, a)| pt.x.is_finite() && pt.y.is_finite() && a.is_finite());
    if places.is_empty() {
        warnings.push("the target has no outline to place copies on".to_string());
        return StageResult { stage: Stage::Unchanged, guides: Vec::new(), warnings };
    }
    let align = p.align && p.mode != CopyMode::Populate;
    let copies = places
        .iter()
        .map(|(pt, a)| {
            let turn = if align { *a } else { 0.0 };
            Affine::translate(*pt - ctx.pivot) * Affine::rotate_about(turn, ctx.pivot) * Affine::scale_about(scale, ctx.pivot)
        })
        .collect();
    let guides = places.iter().take(MAX_GUIDE_POINTS).map(|(pt, _)| Guide::Point(*pt)).collect();
    StageResult { stage: Stage::Array(copies), guides, warnings }
}

/// A subpath as segments, and whether it is closed.
struct Sub {
    segs: Vec<PathSeg>,
    closed: bool,
}

fn subpaths(path: &BezPath) -> Vec<Sub> {
    let mut subs: Vec<Sub> = Vec::new();
    let mut cur: Option<Sub> = None;
    let (mut start, mut at) = (Point::ZERO, Point::ZERO);
    let push = |s: Option<Sub>, subs: &mut Vec<Sub>| {
        if let Some(s) = s.filter(|s| !s.segs.is_empty()) {
            subs.push(s);
        }
    };
    for e in path.elements() {
        match *e {
            PathEl::MoveTo(p) => {
                push(cur.take(), &mut subs);
                cur = Some(Sub { segs: Vec::new(), closed: false });
                start = p;
                at = p;
            }
            PathEl::LineTo(p) => {
                cur.get_or_insert_with(|| Sub { segs: Vec::new(), closed: false })
                    .segs
                    .push(PathSeg::Line(vectorcraft_geom::kurbo::Line::new(at, p)));
                at = p;
            }
            PathEl::QuadTo(a, b) => {
                cur.get_or_insert_with(|| Sub { segs: Vec::new(), closed: false })
                    .segs
                    .push(PathSeg::Quad(vectorcraft_geom::kurbo::QuadBez::new(at, a, b)));
                at = b;
            }
            PathEl::CurveTo(a, b, c) => {
                cur.get_or_insert_with(|| Sub { segs: Vec::new(), closed: false })
                    .segs
                    .push(PathSeg::Cubic(vectorcraft_geom::kurbo::CubicBez::new(at, a, b, c)));
                at = c;
            }
            PathEl::ClosePath => {
                if let Some(mut s) = cur.take() {
                    if (at - start).hypot() > 1e-9 {
                        s.segs.push(PathSeg::Line(vectorcraft_geom::kurbo::Line::new(at, start)));
                    }
                    s.closed = true;
                    push(Some(s), &mut subs);
                }
                at = start;
            }
        }
    }
    push(cur, &mut subs);
    subs
}

fn start_of(s: &PathSeg) -> Point {
    match s {
        PathSeg::Line(l) => l.p0,
        PathSeg::Quad(q) => q.p0,
        PathSeg::Cubic(c) => c.p0,
    }
}

fn end_of(s: &PathSeg) -> Point {
    match s {
        PathSeg::Line(l) => l.p1,
        PathSeg::Quad(q) => q.p2,
        PathSeg::Cubic(c) => c.p3,
    }
}

/// The direction (radians) of `s` at `t`, falling back to its chord, then 0.
fn angle_at(s: &PathSeg, t: f64) -> f64 {
    let d: Vec2 = match s {
        PathSeg::Line(l) => l.p1 - l.p0,
        PathSeg::Quad(q) => q.deriv().eval(t).to_vec2(),
        PathSeg::Cubic(c) => c.deriv().eval(t).to_vec2(),
    };
    let d = if d.hypot() > 1e-12 { d } else { end_of(s) - start_of(s) };
    if d.hypot() > 1e-12 { d.y.atan2(d.x) } else { 0.0 }
}

fn anchors(path: &BezPath) -> Vec<(Point, f64)> {
    let mut out = Vec::new();
    for sub in subpaths(path) {
        for s in &sub.segs {
            out.push((start_of(s), angle_at(s, 0.0)));
        }
        if !sub.closed
            && let Some(last) = sub.segs.last()
        {
            out.push((end_of(last), angle_at(last, 1.0)));
        }
    }
    out
}

fn along(path: &BezPath, count: usize) -> Vec<(Point, f64)> {
    let subs = subpaths(path);
    let segs: Vec<(PathSeg, f64)> = subs.iter().flat_map(|s| s.segs.iter().map(|g| (*g, g.arclen(ARCLEN_ACCURACY)))).collect();
    let total: f64 = segs.iter().map(|(_, l)| l).sum();
    if segs.is_empty() || !total.is_finite() {
        return Vec::new();
    }
    let closed = subs.iter().all(|s| s.closed);
    let step = if closed {
        total / count as f64
    } else if count > 1 {
        total / (count - 1) as f64
    } else {
        0.0
    };
    let mut out = Vec::with_capacity(count);
    let (mut i, mut before) = (0usize, 0.0f64);
    for k in 0..count {
        let s = (k as f64 * step).min(total);
        // Walk forward to the segment holding `s` (the last one at the very end).
        while let Some((_, len)) = segs.get(i)
            && before + len < s
            && i + 1 < segs.len()
        {
            before += len;
            i += 1;
        }
        let Some((seg, len)) = segs.get(i) else { break };
        let local = (s - before).clamp(0.0, *len);
        let t = if *len > 0.0 { seg.inv_arclen(local, ARCLEN_ACCURACY).clamp(0.0, 1.0) } else { 0.0 };
        out.push((seg.eval(t), angle_at(seg, t)));
    }
    out
}

fn populate(path: &BezPath, count: usize, seed: u32, warnings: &mut Vec<String>) -> Vec<(Point, f64)> {
    let b = path.bounding_box();
    if !(b.width() > 0.0 && b.height() > 0.0 && b.is_finite()) {
        warnings.push("the target has no area to populate".to_string());
        return Vec::new();
    }
    let per_test = path.elements().len().max(1);
    let attempts = count.saturating_mul(64).min(POPULATE_WORK / per_test).max(count.min(POPULATE_WORK / per_test));
    let mut r = rng::Stream::new(seed, 0);
    let mut out = Vec::with_capacity(count);
    for _ in 0..attempts {
        if out.len() >= count {
            break;
        }
        let x = b.x0 + (r.signed() + 1.0) * 0.5 * b.width();
        let y = b.y0 + (r.signed() + 1.0) * 0.5 * b.height();
        let pt = Point::new(x, y);
        if path.winding(pt) != 0 {
            out.push((pt, 0.0));
        }
    }
    if out.len() < count {
        warnings.push(format!("found room for {} of {count} copies inside the target", out.len()));
    }
    out
}
