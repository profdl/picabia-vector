//! Deform: Taper, Twist, Bend, Shear and Bulge/Pinch, one row with a `mode`.
//!
//! Ported from tlprocedural's Deform category (`TaperModifierProcessor`, `TwistModifierProcessor`,
//! `BendModifierProcessor`, `ShearModifierProcessor`, `BulgeModifierProcessor` and
//! `utils/progressiveScale.ts`): the same point maps, ranges, defaults and per-copy ramp.
//! Differences, on purpose (`docs/picabia/modifiers.md` › Deform): curves stay curves (segments
//! are split into short pieces whose control points are mapped, like VectorCraft's own distort
//! effects, instead of flattening to a polyline), the box is the curve's true bounds, Bulge's
//! centre is the box's centre (tlprocedural's average of the polygon's vertices moves with how
//! densely each side is sampled), one falloff is shared by the modes that use it, and Bulge's
//! radius is a factor (1 = 100%).

use vectorcraft_doc::live::map_nonlinear_within;
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{Affine, ParamCurve, PathData, Point, Rect, Vec2};

use crate::model::{Deform, DeformAxis, DeformMode, Falloff, ShearOrigin};

/// Boxes smaller than this (points) both ways are left alone, as are axes shorter than it
/// (tlprocedural's `0.001`).
const TINY: f64 = 1e-3;
/// Pieces are at most the box's diagonal over this.
const PIECES_PER_DIAGONAL: f64 = 32.0;
/// Points sampled per segment when measuring Bulge's extent.
const EXTENT_SAMPLES: usize = 16;

/// How much of the effect the copy at `index` of `count` gets (tlprocedural's
/// `computeProgressiveScale`): all of it on the object itself (`index: None`), with the ramp off,
/// or with one copy; otherwise `index / (count − 1)` (reversed: from the other end) times
/// `progressiveAmount`.
pub(super) fn ramp(p: &Deform, index: Option<(usize, usize)>) -> f64 {
    let Some((i, count)) = index else { return 1.0 };
    if !p.progressive || count <= 1 {
        return 1.0;
    }
    let t = (i as f64 / (count - 1) as f64).clamp(0.0, 1.0);
    let directed = if p.progressive_reverse { 1.0 - t } else { t };
    directed * p.progressive_amount
}

/// `p` with its main amount scaled by `s` (the ramp): Taper's end scale moves towards its start
/// scale, the angles, shears and strength scale.
pub(super) fn scaled(p: &Deform, s: f64) -> Deform {
    if s == 1.0 {
        return p.clone();
    }
    let mut q = p.clone();
    q.end_scale = p.start_scale + (p.end_scale - p.start_scale) * s;
    q.angle = p.angle * s;
    q.bend_angle = p.bend_angle * s;
    q.shear_x = p.shear_x * s;
    q.shear_y = p.shear_y * s;
    q.strength = p.strength * s;
    q
}

/// Whether `p` leaves every point where it is (a copy the ramp gives nothing keeps its anchors).
fn identity(p: &Deform) -> bool {
    match p.mode {
        DeformMode::Taper => p.start_scale == 1.0 && p.end_scale == 1.0,
        DeformMode::Twist => p.angle == 0.0,
        DeformMode::Bend => p.bend_angle == 0.0,
        DeformMode::Shear => p.shear_x == 0.0 && p.shear_y == 0.0,
        DeformMode::Bulge => p.strength == 0.0,
    }
}

fn ease(t: f64, f: Falloff) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match f {
        Falloff::Linear => t,
        Falloff::EaseIn => t * t,
        Falloff::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        Falloff::Smooth => t * t * (3.0 - 2.0 * t),
    }
}

/// A deformation of the points within one box.
#[derive(Clone, Debug)]
pub(super) enum Map {
    /// An affine map (Shear): applied to anchors and handles exactly, no anchors added.
    Affine(Affine),
    /// A point map; `piece` is the longest piece a segment is split into.
    Field { f: Field, piece: f64 },
}

#[derive(Clone, Debug)]
pub(super) struct Field {
    p: Deform,
    b: Rect,
    /// Bulge: the farthest the shape reaches from the box's centre.
    extent: f64,
}

impl Field {
    pub(super) fn at(&self, pt: Point) -> Point {
        let (p, b) = (&self.p, self.b);
        let horizontal = p.axis == DeformAxis::Horizontal;
        let (axis_min, axis_len) = if horizontal { (b.x0, b.width()) } else { (b.y0, b.height()) };
        let c = b.center();
        match p.mode {
            DeformMode::Taper => {
                let t = ((if horizontal { pt.x } else { pt.y } - axis_min) / axis_len).clamp(0.0, 1.0);
                let scale = p.start_scale + (p.end_scale - p.start_scale) * ease(t, p.falloff);
                if horizontal { Point::new(pt.x, c.y + (pt.y - c.y) * scale) } else { Point::new(c.x + (pt.x - c.x) * scale, pt.y) }
            }
            DeformMode::Twist => {
                // Centred: one end turns −angle/2, the other +angle/2, the middle not at all.
                let t = ((if horizontal { pt.x } else { pt.y } - axis_min) / axis_len).clamp(0.0, 1.0);
                let tc = t - 0.5;
                let f = if tc < 0.0 { -ease(-tc * 2.0, p.falloff) * 0.5 } else { ease(tc * 2.0, p.falloff) * 0.5 };
                rotate(pt, c, p.angle.to_radians() * f)
            }
            DeformMode::Bend => {
                let pivot = axis_min + p.bend_point * axis_len;
                let a = p.bend_angle.to_radians();
                if horizontal {
                    let theta = ((pt.x - b.x0) / axis_len - p.bend_point) * a;
                    let (rel, d) = (pt.x - pivot, pt.y - c.y);
                    let (s, co) = theta.sin_cos();
                    Point::new(pivot + rel * co - d * s, c.y + rel * s + d * co)
                } else {
                    let theta = ((pt.y - b.y0) / axis_len - p.bend_point) * a;
                    let (rel, d) = (pt.y - pivot, pt.x - c.x);
                    let (s, co) = theta.sin_cos();
                    Point::new(c.x + rel * s + d * co, pivot + rel * co - d * s)
                }
            }
            DeformMode::Bulge => {
                let radius = p.radius * self.extent;
                let v = pt - c;
                let dist = v.hypot();
                if dist > radius || dist < TINY {
                    return pt;
                }
                // The size of the push comes from the extent only; the radius sets its reach.
                let push = p.strength * self.extent * 0.5 * ease(1.0 - dist / radius, p.falloff);
                pt + v * (push / dist)
            }
            // Affine: never a field (see [`map`]).
            DeformMode::Shear => pt,
        }
    }
}

fn rotate(pt: Point, c: Point, a: f64) -> Point {
    let (s, co) = a.sin_cos();
    let v = pt - c;
    c + Vec2::new(v.x * co - v.y * s, v.x * s + v.y * co)
}

/// The deformation `p` makes in box `b` (the paths' bounds) of paths reaching `extent` from its
/// centre; `None`: nothing changes (the effect is nothing, or the box or its axis is too small).
pub(super) fn map(p: &Deform, b: Rect, extent: f64) -> Option<Map> {
    if identity(p) || !(b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()) {
        return None;
    }
    let (w, h) = (b.width(), b.height());
    let axis_len = if p.axis == DeformAxis::Horizontal { w } else { h };
    match p.mode {
        DeformMode::Shear => {
            let o = match p.origin {
                ShearOrigin::Center => b.center(),
                ShearOrigin::TopLeft => Point::new(b.x0, b.y0),
                ShearOrigin::BottomLeft => Point::new(b.x0, b.y1),
                ShearOrigin::TopRight => Point::new(b.x1, b.y0),
                ShearOrigin::BottomRight => Point::new(b.x1, b.y1),
            };
            // x' = ox + lx + shearX·ly, y' = oy + ly + shearY·lx about the origin.
            let shear = Affine::new([1.0, p.shear_y, p.shear_x, 1.0, 0.0, 0.0]);
            return Some(Map::Affine(Affine::translate(o.to_vec2()) * shear * Affine::translate(-o.to_vec2())));
        }
        DeformMode::Taper | DeformMode::Twist if w < TINY && h < TINY => return None,
        DeformMode::Taper | DeformMode::Twist | DeformMode::Bend if axis_len < TINY => return None,
        DeformMode::Bulge if !(extent.is_finite() && extent >= TINY) => return None,
        _ => {}
    }
    let diag = w.hypot(h);
    if !diag.is_finite() {
        return None;
    }
    Some(Map::Field { f: Field { p: p.clone(), b, extent }, piece: (diag / PIECES_PER_DIAGONAL).max(TINY) })
}

/// `path` through `m` worked out in `frame` (the map is read in the frame's coordinates: `path`
/// is taken into them, mapped and brought back), with at most `max` anchors (or its own count
/// when that is more).
pub(super) fn apply_map(path: &PathData, m: &Map, frame: Affine, max: usize) -> PathData {
    let inv = frame.inverse();
    match m {
        Map::Affine(a) => path.transformed(frame * *a * inv),
        Map::Field { f, piece } => map_nonlinear_within(path, *piece, max, |pt| frame * f.at(inv * pt)),
    }
}

/// The rotation part of copy transform `t` (its x axis's direction), with its reflection when it
/// has one (a negative determinant: the y axis flipped after the turn): the frame a copy is
/// deformed in, so each copy bends along its own axis as tlprocedural's do (it strips a copy's
/// rotation before deforming), and a mirrored copy's deform is the mirror image of the object's.
/// Identity when `t` has no direction.
pub(super) fn frame_of(t: Affine) -> Affine {
    let [a, b, c, d, ..] = t.as_coeffs();
    if !(a.is_finite() && b.is_finite()) || a.hypot(b) < 1e-12 {
        return Affine::IDENTITY;
    }
    let turn = Affine::rotate(b.atan2(a));
    let det = a * d - b * c;
    if det.is_finite() && det < 0.0 { turn * Affine::scale_non_uniform(1.0, -1.0) } else { turn }
}

/// What a deform needs to know about the node it reshapes: its paths' box and Bulge's extent,
/// read in `frame`.
pub(super) fn measure(n: &Node, p: &Deform, frame: Affine) -> Option<(Rect, f64)> {
    let mut paths = Vec::new();
    paths_of(n, 0, &mut paths);
    if frame != Affine::IDENTITY {
        let inv = frame.inverse();
        for path in &mut paths {
            path.transform(inv);
        }
    }
    let b = bounds(&paths)?;
    let e = if p.mode == DeformMode::Bulge { extent(&paths, b.center()) } else { 0.0 };
    Some((b, e))
}

/// The segments of the plain paths of `n`.
pub(super) fn segments(n: &Node, depth: usize) -> usize {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return 0;
    }
    match &n.kind {
        NodeKind::Path { guide: true, .. } => 0,
        NodeKind::Path { path, .. } => path.subpaths.iter().map(|s| s.segment_count()).fold(0, usize::saturating_add),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            children.iter().fold(0usize, |a, c| a.saturating_add(segments(c, depth + 1)))
        }
        _ => 0,
    }
}

/// Room for anchors shared out among the paths of a node, by their segments.
pub(super) struct Room {
    pub room: usize,
    pub segments: usize,
}

impl Room {
    /// The most anchors `path` may hold after the deform: its own and its share of the room.
    fn max_for(&self, path: &PathData) -> usize {
        let segs = path.subpaths.iter().map(|s| s.segment_count()).fold(0, usize::saturating_add);
        let share = if self.segments == 0 { 0.0 } else { (self.room as f64) * (segs as f64 / self.segments as f64) };
        let share = if share.is_finite() && share > 0.0 { share.floor().min(self.room as f64) as usize } else { 0 };
        path.anchor_count().saturating_add(share)
    }
}

/// `m` (worked out in `frame`) on every plain path in `n`, each within its share of `room`;
/// `skipped` is set when something it can't change is passed over.
pub(super) fn apply_node(n: &mut Node, m: &Map, frame: Affine, room: &Room, depth: usize, skipped: &mut bool) {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return;
    }
    match &mut n.kind {
        // Ruler guides draw on screen only: passed over silently, as path guides are.
        NodeKind::Path { guide: true, .. } | NodeKind::RulerGuide { .. } => {}
        NodeKind::Path { path, live, .. } => {
            *path = apply_map(path, m, frame, room.max_for(path));
            // The copy is a plain path now: a live shape would redraw its old outline.
            *live = None;
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            for c in children.iter_mut() {
                apply_node(std::sync::Arc::make_mut(c), m, frame, room, depth + 1, skipped);
            }
        }
        _ => *skipped = true,
    }
}

/// The farthest the paths of `paths` reach from `c` (anchors and points along each segment).
pub(super) fn extent(paths: &[PathData], c: Point) -> f64 {
    let mut far: f64 = 0.0;
    for sp in paths.iter().flat_map(|p| &p.subpaths) {
        for a in &sp.anchors {
            far = far.max(a.p.distance(c));
        }
        for i in 0..sp.segment_count() {
            let seg = sp.segment(i);
            for k in 1..EXTENT_SAMPLES {
                far = far.max(seg.eval(k as f64 / EXTENT_SAMPLES as f64).distance(c));
            }
        }
    }
    far
}

/// The plain paths of `n` (guides left out), for reading.
pub(super) fn paths_of(n: &Node, depth: usize, out: &mut Vec<PathData>) {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return;
    }
    match &n.kind {
        NodeKind::Path { guide: true, .. } => {}
        NodeKind::Path { path, .. } => out.push(path.clone()),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            for c in children {
                paths_of(c, depth + 1, out);
            }
        }
        _ => {}
    }
}

/// The union of `paths`' bounds.
pub(super) fn bounds(paths: &[PathData]) -> Option<Rect> {
    paths.iter().filter_map(PathData::bounds).reduce(|a, b| a.union(b))
}

/// `path` deformed by `p` in its own box, without a frame, ramp or cap (bare geometry).
pub(super) fn deform_path(path: &PathData, p: &Deform) -> Option<PathData> {
    let paths = [path.clone()];
    let b = bounds(&paths)?;
    let m = map(p, b, extent(&paths, b.center()))?;
    Some(apply_map(path, &m, Affine::IDENTITY, usize::MAX))
}
