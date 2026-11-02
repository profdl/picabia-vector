//! Canvas handles of a modifier row: where each one sits for an object, what a drag of it to a
//! point sets, the guide lines drawn with it and its live readout. Pure geometry in document space
//! (the space the params are in, [`crate::node_bounds`]); the engine decides which rows show
//! handles and turns a drag into one `modifier.set` undo step.
//!
//! | Row | Handle | Sets |
//! |---|---|---|
//! | linear array | the last copy's centre | `offsetX`, `offsetY` (Shift: 45° steps) |
//! | circular array | the ring's centre | `centerX`, `centerY` (Shift: one axis) |
//! | circular array | a knob on the ring opposite the original | `radius` |
//! | circular array | a knob just outside the ring where the sweep ends | `sweep` (Shift: 15° steps) |
//! | grid array | the left edge of the last column / top of the last row | `gapX` / `gapY` (Shift: both) |
//! | mirror | a knob at the end of each axis line | `offset` |
//! | wallpaper | the symmetry origin (not p1) | `originX`, `originY` (Shift: one axis) |
//! | wallpaper | the end of the cell's first side | `cellWidth` |
//! | wallpaper | the end of its second side (groups whose lattice it shapes) | `cellHeight`, and `angle` for p1/p2 (Shift keeps the angle) |
//! | deform taper | knobs past each end of the axis | `startScale`, `endScale` |
//! | deform twist | a knob past the end of the axis, turned with it | `angle` (Shift: 15° steps) |
//! | deform bend | the bend point on the axis | `bendPoint` |
//! | deform bulge | a knob on the bulge's reach | `radius` |
//! | deform shear | knobs above (beside) the box, slanted with it | `shearX`, `shearY` |
//! | trim | the window's start and end on the first subpath | `start`, `end` |
//! | displace | a knob above the object, its height the amplitude | `amplitude` |
//!
//! Knobs that would sit on a bounding-box handle at the default params sit [`HANDLE_GAP_PX`]
//! screen pixels outside the box instead. Transform each, randomize, copy to shape, resample,
//! tidy, screen and the stencil have no handles. Every value is clamped to its field's range, and
//! a drag to a point that isn't finite (or that would give a value that isn't) sets nothing.

use serde_json::{Map, Value, json};
use vectorcraft_doc::Node;
use vectorcraft_geom::{Point, Rect, Vec2};

use crate::eval::Guide;
use crate::model::{
    CircularArray, Deform, DeformAxis, DeformMode, LinearArray, Mirror, MirrorAxis, ModifierKind, ShearOrigin, Trim, Wallpaper, WallpaperGroup,
};
use crate::shape::handle_geom::{TrimPath, deform_box};
use crate::{MAX_COUNT, MAX_LENGTH};

/// How far (screen pixels) the knobs that would sit on the bounding box sit outside it.
pub const HANDLE_GAP_PX: f64 = 18.0;

/// The direction (degrees, clockwise from the right) of Bulge's radius knob from its centre: up
/// and to the right, clear of the box's side handles for a circle and of its corners for a square.
const BULGE_KNOB_ANGLE: f64 = -30.0;

/// Lengths and boxes smaller than this (points) give no handle that divides by them.
const TINY: f64 = 1e-3;

/// One kind of handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HandleKind {
    LinearEnd,
    RingCenter,
    RingRadius,
    RingSweep,
    GridGapX,
    GridGapY,
    MirrorVertical,
    MirrorHorizontal,
    CellOrigin,
    CellA,
    CellB,
    TaperStart,
    TaperEnd,
    TwistEnd,
    BendPoint,
    BulgeRadius,
    ShearX,
    ShearY,
    TrimStart,
    TrimEnd,
    DisplaceAmplitude,
}

/// How a handle moves under the pointer (what the cursor shows).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    /// Anywhere.
    Free,
    /// Along this document-space direction.
    Along(Vec2),
    /// Round a centre.
    Turn,
}

/// A handle and where it is (document space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub kind: HandleKind,
    pub at: Point,
}

/// The object a row's handles belong to: the node, its bounds (whose centre is the pivot, as
/// [`crate::node_bounds_in_doc`] gives them) and the view's zoom (screen pixels per point).
#[derive(Clone, Copy)]
pub struct Frame<'a> {
    pub node: &'a Node,
    pub bounds: Rect,
    pub zoom: f64,
}

impl Frame<'_> {
    fn gap(&self) -> f64 {
        let z = if self.zoom.is_finite() && self.zoom > 0.0 { self.zoom } else { 1.0 };
        HANDLE_GAP_PX / z
    }

    fn pivot(&self) -> Point {
        self.bounds.center()
    }

    fn ok(&self) -> bool {
        let b = self.bounds;
        [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite())
    }
}

impl HandleKind {
    /// The handle's name in JSON (`linearEnd`, `ringCenter`…).
    pub fn id(self) -> &'static str {
        match self {
            Self::LinearEnd => "linearEnd",
            Self::RingCenter => "ringCenter",
            Self::RingRadius => "ringRadius",
            Self::RingSweep => "ringSweep",
            Self::GridGapX => "gridGapX",
            Self::GridGapY => "gridGapY",
            Self::MirrorVertical => "mirrorVertical",
            Self::MirrorHorizontal => "mirrorHorizontal",
            Self::CellOrigin => "cellOrigin",
            Self::CellA => "cellA",
            Self::CellB => "cellB",
            Self::TaperStart => "taperStart",
            Self::TaperEnd => "taperEnd",
            Self::TwistEnd => "twistEnd",
            Self::BendPoint => "bendPoint",
            Self::BulgeRadius => "bulgeRadius",
            Self::ShearX => "shearX",
            Self::ShearY => "shearY",
            Self::TrimStart => "trimStart",
            Self::TrimEnd => "trimEnd",
            Self::DisplaceAmplitude => "displaceAmplitude",
        }
    }

    /// The fields a drag of the handle on row `kind` sets (a field bound to a parameter or an
    /// expression makes the handle read-only).
    pub fn fields(self, kind: &ModifierKind) -> &'static [&'static str] {
        match self {
            Self::LinearEnd => &["offsetX", "offsetY"],
            Self::RingCenter => &["centerX", "centerY"],
            Self::RingRadius | Self::BulgeRadius => &["radius"],
            Self::RingSweep => &["sweep"],
            Self::GridGapX => &["gapX"],
            Self::GridGapY => &["gapY"],
            Self::MirrorVertical | Self::MirrorHorizontal => &["offset"],
            Self::CellOrigin => &["originX", "originY"],
            Self::CellA => &["cellWidth"],
            Self::CellB => match kind {
                ModifierKind::Wallpaper(w) if w.group.uses_angle() => &["cellHeight", "angle"],
                _ => &["cellHeight"],
            },
            Self::TaperStart => &["startScale"],
            Self::TaperEnd => &["endScale"],
            Self::TwistEnd => &["angle"],
            Self::BendPoint => &["bendPoint"],
            Self::ShearX => &["shearX"],
            Self::ShearY => &["shearY"],
            Self::TrimStart => &["start"],
            Self::TrimEnd => &["end"],
            Self::DisplaceAmplitude => &["amplitude"],
        }
    }

    /// How the handle moves on row `kind`.
    pub fn motion(self, kind: &ModifierKind) -> Motion {
        let x = Motion::Along(Vec2::new(1.0, 0.0));
        let y = Motion::Along(Vec2::new(0.0, 1.0));
        let deform_across = |d: &Deform| if d.axis == DeformAxis::Horizontal { y } else { x };
        let deform_along = |d: &Deform| if d.axis == DeformAxis::Horizontal { x } else { y };
        match (self, kind) {
            (Self::RingRadius, ModifierKind::CircularArray(c)) => Motion::Along(dir(c.start_angle)),
            (Self::RingSweep | Self::TwistEnd, _) => Motion::Turn,
            // Out from the bulge's centre, at the knob's fixed angle.
            (Self::BulgeRadius, _) => Motion::Along(dir(BULGE_KNOB_ANGLE)),
            (Self::GridGapX | Self::MirrorVertical | Self::CellA | Self::ShearX, _) => x,
            (Self::GridGapY | Self::MirrorHorizontal | Self::ShearY | Self::DisplaceAmplitude, _) => y,
            (Self::CellB, ModifierKind::Wallpaper(w)) if !w.group.uses_angle() => y,
            (Self::TaperStart | Self::TaperEnd, ModifierKind::Deform(d)) => deform_across(d),
            (Self::BendPoint, ModifierKind::Deform(d)) => deform_along(d),
            _ => Motion::Free,
        }
    }
}

/// `v` if finite, within ±[`MAX_LENGTH`] (non-finite: 0).
fn len(v: f64) -> f64 {
    if v.is_finite() { v.clamp(-MAX_LENGTH, MAX_LENGTH) } else { 0.0 }
}

/// `v` within `lo..=hi`, `default` when not finite (as the evaluator reads it).
fn within(v: f64, lo: f64, hi: f64, default: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { default }
}

fn count(c: u32) -> u32 {
    c.clamp(1, MAX_COUNT)
}

/// The unit vector at `deg` degrees (clockwise on screen from the right).
fn dir(deg: f64) -> Vec2 {
    let r = within(deg, -1.0e6, 1.0e6, 0.0).to_radians();
    Vec2::new(r.cos(), r.sin())
}

/// The angle of `v` in degrees (clockwise on screen from the right).
fn angle_of(v: Vec2) -> f64 {
    v.y.atan2(v.x).to_degrees()
}

/// `a` (degrees) wrapped into `-180..180`.
fn wrap180(a: f64) -> f64 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

/// `v` turned to the nearest 45° step (same length).
fn at_45(v: Vec2) -> Vec2 {
    let step = std::f64::consts::FRAC_PI_4;
    let a = (v.y.atan2(v.x) / step).round() * step;
    Vec2::new(a.cos(), a.sin()) * v.hypot()
}

/// `q` kept on the axis (through `from`) along which it moved most.
fn one_axis(q: Point, from: Point) -> Point {
    let m = q - from;
    if m.x.abs() >= m.y.abs() { Point::new(q.x, from.y) } else { Point::new(from.x, q.y) }
}

/// `v` rounded to steps of `step`.
fn snapped(v: f64, step: f64) -> f64 {
    (v / step).round() * step
}

fn rotate(v: Vec2, deg: f64) -> Vec2 {
    let (s, c) = deg.to_radians().sin_cos();
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

fn finite(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

/// A trim window end (`x` plus `offset`) as a fraction of the subpath, as Trim reads it.
fn trim_place(x: f64, offset: f64, closed: bool) -> f64 {
    let v = within(x, 0.0, 1.0, 0.0) + within(offset, 0.0, 1.0, 0.0);
    if closed {
        let w = v.rem_euclid(1.0);
        if w >= 1.0 { 0.0 } else { w }
    } else {
        v.clamp(0.0, 1.0)
    }
}

// ---- Geometry shared by the spots, the drags and the guides --------------------------------------

struct Ring {
    radius: f64,
    start: f64,
    sweep: f64,
    offset: Vec2,
    centre: Point,
}

fn ring(c: &CircularArray, f: &Frame) -> Ring {
    let radius = within(c.radius, 0.0, MAX_LENGTH, 0.0);
    let start = within(c.start_angle, -1.0e6, 1.0e6, 0.0);
    let sweep = within(c.sweep, -360.0, 360.0, 360.0);
    let offset = Vec2::new(len(c.center_x), len(c.center_y));
    Ring { radius, start, sweep, offset, centre: f.pivot() - dir(start) * radius + offset }
}

impl Ring {
    fn sweep_knob(&self, f: &Frame) -> Point {
        self.centre + dir(self.start + self.sweep) * (self.radius + f.gap())
    }
}

struct Cell {
    angle: f64,
    origin: Point,
    a: Vec2,
    b: Vec2,
}

fn cell(p: &Wallpaper, f: &Frame) -> Cell {
    let w = within(p.cell_width, 0.001, MAX_LENGTH, 100.0);
    let h = within(p.cell_height, 0.001, MAX_LENGTH, 100.0);
    let angle = within(p.angle, 1.0, 179.0, 90.0);
    let origin = Point::new(f.bounds.x0 + len(p.origin_x), f.bounds.y0 + len(p.origin_y));
    let (a, b) = crate::kinds::wallpaper_lattice(p.group, w, h, angle);
    Cell { angle, origin, a: Vec2::new(a[0], a[1]), b: Vec2::new(b[0], b[1]) }
}

fn mirror_reach(b: Rect) -> f64 {
    (b.width().max(b.height()) * 1.5).max(100.0)
}

/// Deform's box, its centre and its axis: `(box, centre, horizontal, axis length, extent)`.
struct DeformBox {
    b: Rect,
    c: Point,
    horizontal: bool,
    extent: f64,
}

fn deform_frame(d: &Deform, f: &Frame) -> Option<DeformBox> {
    let (b, extent) = deform_box(f.node, d)?;
    if ![b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(DeformBox { b, c: b.center(), horizontal: d.axis == DeformAxis::Horizontal, extent })
}

impl DeformBox {
    fn axis_len(&self) -> f64 {
        if self.horizontal { self.b.width() } else { self.b.height() }
    }

    /// Half the box across the axis (what Taper scales).
    fn half_across(&self) -> f64 {
        (if self.horizontal { self.b.height() } else { self.b.width() }) / 2.0
    }

    fn taper(&self, f: &Frame, end: bool, scale: f64) -> Point {
        let (g, half, s) = (f.gap(), self.half_across(), within(scale, 0.1, 3.0, 1.0));
        if self.horizontal {
            Point::new(if end { self.b.x1 + g } else { self.b.x0 - g }, self.c.y - s * half)
        } else {
            Point::new(self.c.x + s * half, if end { self.b.y1 + g } else { self.b.y0 - g })
        }
    }

    /// The twist knob's arm before turning, and the angle it points at.
    fn twist_arm(&self, f: &Frame) -> (Vec2, f64) {
        if self.horizontal { (Vec2::new(self.b.width() / 2.0 + f.gap(), 0.0), 0.0) } else { (Vec2::new(0.0, self.b.height() / 2.0 + f.gap()), 90.0) }
    }

    fn shear_origin(&self, o: ShearOrigin) -> Point {
        let b = self.b;
        match o {
            ShearOrigin::Center => b.center(),
            ShearOrigin::TopLeft => Point::new(b.x0, b.y0),
            ShearOrigin::BottomLeft => Point::new(b.x0, b.y1),
            ShearOrigin::TopRight => Point::new(b.x1, b.y0),
            ShearOrigin::BottomRight => Point::new(b.x1, b.y1),
        }
    }

    /// The unsheared point the shear knob `h` slants, relative to the origin: above the box
    /// (below it when the origin is on the top edge) for `ShearX`, right of it (left of it when
    /// the origin is on the right edge) for `ShearY`.
    fn shear_lever(&self, f: &Frame, d: &Deform, h: HandleKind) -> Vec2 {
        let (b, o, g) = (self.b, self.shear_origin(d.origin), f.gap());
        let r = if h == HandleKind::ShearX {
            Point::new(self.c.x, if o.y > b.y0 + 1e-9 { b.y0 - g } else { b.y1 + g })
        } else {
            Point::new(if o.x < b.x1 - 1e-9 { b.x1 + g } else { b.x0 - g }, self.c.y)
        };
        r - o
    }

    fn shear_knob(&self, f: &Frame, d: &Deform, h: HandleKind) -> Point {
        let o = self.shear_origin(d.origin);
        let l = self.shear_lever(f, d, h);
        let (sx, sy) = (within(d.shear_x, -2.0, 2.0, 0.0), within(d.shear_y, -2.0, 2.0, 0.0));
        o + Vec2::new(l.x + sx * l.y, l.y + sy * l.x)
    }

    fn bulge_knob(&self, d: &Deform) -> Point {
        self.c + dir(BULGE_KNOB_ANGLE) * within(d.radius, 0.01, 2.0, 1.0) * self.extent
    }
}

fn displace_base(f: &Frame) -> Point {
    Point::new(f.bounds.center().x, f.bounds.y0 - f.gap())
}

// ---- Spots ---------------------------------------------------------------------------------------

/// The handles of row `kind` on the object in `f` (empty: the row has none, or none fit the
/// object), in hit order: the first wins where two coincide.
pub fn spots(kind: &ModifierKind, f: &Frame) -> Vec<Spot> {
    if !f.ok() {
        return Vec::new();
    }
    let s = |kind, at| Spot { kind, at };
    let pv = f.pivot();
    let b = f.bounds;
    let v = match kind {
        ModifierKind::LinearArray(LinearArray { count: n, offset_x, offset_y, .. }) => {
            let n = count(*n);
            if n < 2 {
                return Vec::new();
            }
            vec![s(HandleKind::LinearEnd, pv + Vec2::new(len(*offset_x), len(*offset_y)) * f64::from(n - 1))]
        }
        ModifierKind::CircularArray(c) => {
            let r = ring(c, f);
            let mut v = vec![s(HandleKind::RingRadius, r.centre - dir(r.start) * r.radius), s(HandleKind::RingCenter, r.centre)];
            if r.radius > TINY {
                v.push(s(HandleKind::RingSweep, r.sweep_knob(f)));
            }
            v
        }
        ModifierKind::GridArray(g) => {
            let (cols, rows) = (count(g.cols), count(g.rows));
            let mut v = Vec::new();
            if cols > 1 {
                v.push(s(HandleKind::GridGapX, Point::new(b.x0 + (b.width() + len(g.gap_x)) * f64::from(cols - 1), pv.y)));
            }
            if rows > 1 {
                v.push(s(HandleKind::GridGapY, Point::new(pv.x, b.y0 + (b.height() + len(g.gap_y)) * f64::from(rows - 1))));
            }
            v
        }
        ModifierKind::Mirror(Mirror { axis, offset, .. }) => {
            let (o, reach) = (len(*offset), mirror_reach(b));
            let vert = s(HandleKind::MirrorVertical, Point::new(pv.x + o, pv.y - reach));
            let hor = s(HandleKind::MirrorHorizontal, Point::new(pv.x - reach, pv.y + o));
            match axis {
                MirrorAxis::Vertical => vec![vert],
                MirrorAxis::Horizontal => vec![hor],
                MirrorAxis::Both => vec![vert, hor],
            }
        }
        ModifierKind::Wallpaper(p) => {
            let c = cell(p, f);
            let mut v = vec![s(HandleKind::CellA, c.origin + c.a)];
            if p.group.uses_cell_height() {
                v.push(s(HandleKind::CellB, c.origin + c.b));
            }
            if p.group.uses_origin() {
                v.push(s(HandleKind::CellOrigin, c.origin));
            }
            v
        }
        ModifierKind::Deform(d) => deform_spots(d, f),
        ModifierKind::Trim(t) => trim_spots(t, f),
        ModifierKind::Displace(d) => vec![s(HandleKind::DisplaceAmplitude, displace_base(f) - Vec2::new(0.0, within(d.amplitude, 0.0, 100.0, 0.0)))],
        _ => Vec::new(),
    };
    v.into_iter().filter(|s| finite(s.at)).collect()
}

fn deform_spots(d: &Deform, f: &Frame) -> Vec<Spot> {
    let Some(x) = deform_frame(d, f) else { return Vec::new() };
    let s = |kind, at| Spot { kind, at };
    match d.mode {
        DeformMode::Taper if x.half_across() > TINY && x.axis_len() > TINY => {
            vec![s(HandleKind::TaperStart, x.taper(f, false, d.start_scale)), s(HandleKind::TaperEnd, x.taper(f, true, d.end_scale))]
        }
        DeformMode::Twist if x.axis_len() > TINY => {
            let (arm, _) = x.twist_arm(f);
            vec![s(HandleKind::TwistEnd, x.c + rotate(arm, within(d.angle, -720.0, 720.0, 0.0) / 2.0))]
        }
        DeformMode::Bend if x.axis_len() > TINY => {
            let t = within(d.bend_point, 0.0, 1.0, 0.5);
            let at = if x.horizontal { Point::new(x.b.x0 + t * x.b.width(), x.c.y) } else { Point::new(x.c.x, x.b.y0 + t * x.b.height()) };
            vec![s(HandleKind::BendPoint, at)]
        }
        DeformMode::Bulge if x.extent.is_finite() && x.extent > TINY => vec![s(HandleKind::BulgeRadius, x.bulge_knob(d))],
        DeformMode::Shear => {
            let mut v = Vec::new();
            for h in [HandleKind::ShearX, HandleKind::ShearY] {
                let l = x.shear_lever(f, d, h);
                if (if h == HandleKind::ShearX { l.y } else { l.x }).abs() > TINY {
                    v.push(s(h, x.shear_knob(f, d, h)));
                }
            }
            v
        }
        _ => Vec::new(),
    }
}

fn trim_spots(t: &Trim, f: &Frame) -> Vec<Spot> {
    let Some(tp) = TrimPath::of(f.node) else { return Vec::new() };
    let mut v = Vec::new();
    // The end first: at start = end (the whole path) the end knob drags out the window.
    for (h, x) in [(HandleKind::TrimEnd, t.end), (HandleKind::TrimStart, t.start)] {
        if let Some(at) = tp.at(trim_place(x, t.offset, tp.closed)) {
            v.push(Spot { kind: h, at });
        }
    }
    v
}

// ---- Drags ---------------------------------------------------------------------------------------

fn num(v: f64) -> Value {
    json!(if v == 0.0 { 0.0 } else { v })
}

fn props<const N: usize>(kv: [(&str, f64); N]) -> Option<Map<String, Value>> {
    kv.iter().all(|(_, v)| v.is_finite()).then(|| kv.iter().map(|(k, v)| ((*k).to_string(), num(*v))).collect())
}

/// The params dragging handle `h` of row `kind` (its params when the drag began) to `to` gives,
/// with `from` where the handle was then (Shift's one-axis lock measures from it); `None` when
/// `to` isn't finite, the handle isn't the row's, or the result wouldn't be finite. Values are
/// within each field's range.
pub fn drag(kind: &ModifierKind, h: HandleKind, f: &Frame, to: Point, from: Point, shift: bool) -> Option<Map<String, Value>> {
    if !finite(to) || !finite(from) || !f.ok() {
        return None;
    }
    let pv = f.pivot();
    let b = f.bounds;
    match (kind, h) {
        (ModifierKind::LinearArray(l), HandleKind::LinearEnd) => {
            let steps = f64::from(count(l.count).saturating_sub(1).max(1));
            let v = if shift { at_45(to - pv) } else { to - pv };
            props([("offsetX", len(v.x / steps)), ("offsetY", len(v.y / steps))])
        }
        (ModifierKind::CircularArray(c), _) => {
            let r = ring(c, f);
            match h {
                HandleKind::RingCenter => {
                    let q = if shift { one_axis(to, from) } else { to };
                    let o = q - (pv - dir(r.start) * r.radius);
                    props([("centerX", len(o.x)), ("centerY", len(o.y))])
                }
                // The knob sits at pivot + offset − 2r·dir: r from the pointer's place along dir.
                HandleKind::RingRadius => props([("radius", ((pv + r.offset - to).dot(dir(r.start)) / 2.0).clamp(0.0, MAX_LENGTH))]),
                HandleKind::RingSweep => {
                    let v = to - r.centre;
                    if v.hypot() < TINY {
                        return None;
                    }
                    let s = r.sweep + wrap180(angle_of(v) - (r.start + r.sweep));
                    let s = if shift { snapped(s, 15.0) } else { s };
                    props([("sweep", s.clamp(-360.0, 360.0))])
                }
                _ => None,
            }
        }
        (ModifierKind::GridArray(g), HandleKind::GridGapX | HandleKind::GridGapY) => {
            let x = h == HandleKind::GridGapX;
            let n = f64::from(if x { count(g.cols) } else { count(g.rows) }.saturating_sub(1).max(1));
            let gap = if x { (to.x - b.x0) / n - b.width() } else { (to.y - b.y0) / n - b.height() };
            let gap = len(gap);
            match (x, shift) {
                (true, false) => props([("gapX", gap)]),
                (false, false) => props([("gapY", gap)]),
                (_, true) => props([("gapX", gap), ("gapY", gap)]),
            }
        }
        (ModifierKind::Mirror(_), HandleKind::MirrorVertical) => props([("offset", len(to.x - pv.x))]),
        (ModifierKind::Mirror(_), HandleKind::MirrorHorizontal) => props([("offset", len(to.y - pv.y))]),
        (ModifierKind::Wallpaper(p), _) => wallpaper_drag(p, h, f, to, from, shift),
        (ModifierKind::Deform(d), _) => deform_drag(d, h, f, to, shift),
        (ModifierKind::Trim(t), HandleKind::TrimStart | HandleKind::TrimEnd) => {
            let tp = TrimPath::of(f.node)?;
            let at = tp.nearest(to)? - within(t.offset, 0.0, 1.0, 0.0);
            let v = if tp.closed { trim_place(at.rem_euclid(1.0), 0.0, true) } else { at.clamp(0.0, 1.0) };
            props([(if h == HandleKind::TrimStart { "start" } else { "end" }, v)])
        }
        (ModifierKind::Displace(_), HandleKind::DisplaceAmplitude) => props([("amplitude", (displace_base(f).y - to.y).clamp(0.0, 100.0))]),
        _ => None,
    }
}

fn wallpaper_drag(p: &Wallpaper, h: HandleKind, f: &Frame, to: Point, from: Point, shift: bool) -> Option<Map<String, Value>> {
    let c = cell(p, f);
    let b = f.bounds;
    match h {
        HandleKind::CellOrigin => {
            let q = if shift { one_axis(to, from) } else { to };
            props([("originX", len(q.x - b.x0)), ("originY", len(q.y - b.y0))])
        }
        HandleKind::CellA => props([("cellWidth", (to.x - c.origin.x).clamp(0.001, MAX_LENGTH))]),
        HandleKind::CellB => {
            let v = to - c.origin;
            use WallpaperGroup::*;
            match p.group {
                Pm | Pg | Pmm | Pmg | Pgg => props([("cellHeight", v.y.clamp(0.001, MAX_LENGTH))]),
                Cm | Cmm => props([("cellHeight", (2.0 * v.y).clamp(0.001, MAX_LENGTH))]),
                P1 | P2 if shift => props([("cellHeight", v.dot(dir(c.angle)).clamp(0.001, MAX_LENGTH))]),
                P1 | P2 => props([("cellHeight", v.hypot().clamp(0.001, MAX_LENGTH)), ("angle", angle_of(v).clamp(1.0, 179.0))]),
                // The square and hexagonal lattices have no second side to drag.
                _ => None,
            }
        }
        _ => None,
    }
}

fn deform_drag(d: &Deform, h: HandleKind, f: &Frame, to: Point, shift: bool) -> Option<Map<String, Value>> {
    let x = deform_frame(d, f)?;
    match (d.mode, h) {
        (DeformMode::Taper, HandleKind::TaperStart | HandleKind::TaperEnd) => {
            let half = x.half_across();
            if half <= TINY {
                return None;
            }
            let s = if x.horizontal { (x.c.y - to.y) / half } else { (to.x - x.c.x) / half };
            props([(if h == HandleKind::TaperStart { "startScale" } else { "endScale" }, s.clamp(0.1, 3.0))])
        }
        (DeformMode::Twist, HandleKind::TwistEnd) => {
            let v = to - x.c;
            if v.hypot() < TINY {
                return None;
            }
            let (_, base) = x.twist_arm(f);
            let a0 = within(d.angle, -720.0, 720.0, 0.0);
            let a = a0 + 2.0 * wrap180(angle_of(v) - base - a0 / 2.0);
            let a = if shift { snapped(a, 15.0) } else { a };
            props([("angle", a.clamp(-720.0, 720.0))])
        }
        (DeformMode::Bend, HandleKind::BendPoint) => {
            let l = x.axis_len();
            if l <= TINY {
                return None;
            }
            let t = if x.horizontal { (to.x - x.b.x0) / l } else { (to.y - x.b.y0) / l };
            props([("bendPoint", t.clamp(0.0, 1.0))])
        }
        (DeformMode::Bulge, HandleKind::BulgeRadius) => {
            if !(x.extent.is_finite() && x.extent > TINY) {
                return None;
            }
            props([("radius", ((to - x.c).dot(dir(BULGE_KNOB_ANGLE)) / x.extent).clamp(0.01, 2.0))])
        }
        (DeformMode::Shear, HandleKind::ShearX | HandleKind::ShearY) => {
            let o = x.shear_origin(d.origin);
            let l = x.shear_lever(f, d, h);
            if h == HandleKind::ShearX {
                (l.y.abs() > TINY).then_some(())?;
                props([("shearX", ((to.x - o.x - l.x) / l.y).clamp(-2.0, 2.0))])
            } else {
                (l.x.abs() > TINY).then_some(())?;
                props([("shearY", ((to.y - o.y - l.y) / l.x).clamp(-2.0, 2.0))])
            }
        }
        _ => None,
    }
}

// ---- Guides and readouts -------------------------------------------------------------------------

/// The guide lines drawn with row `kind`'s handles on the object in `f`.
pub fn guides(kind: &ModifierKind, f: &Frame) -> Vec<Guide> {
    if !f.ok() {
        return Vec::new();
    }
    let line = |from: Point, to: Point| Guide::Line { from, to };
    let pv = f.pivot();
    let b = f.bounds;
    let at = |h: HandleKind| spots(kind, f).into_iter().find(|s| s.kind == h).map(|s| s.at);
    let mut v = match kind {
        ModifierKind::LinearArray(_) => at(HandleKind::LinearEnd).map(|e| vec![line(pv, e)]).unwrap_or_default(),
        ModifierKind::CircularArray(c) => {
            let r = ring(c, f);
            let mut v = vec![Guide::Circle { center: r.centre, radius: r.radius }];
            v.extend(at(HandleKind::RingSweep).map(|e| line(r.centre, e)));
            v
        }
        ModifierKind::GridArray(g) => {
            let (cols, rows) = (f64::from(count(g.cols)), f64::from(count(g.rows)));
            let x1 = b.x1 + (b.width() + len(g.gap_x)) * (cols - 1.0);
            let y1 = b.y1 + (b.height() + len(g.gap_y)) * (rows - 1.0);
            let (p0, p1, p2, p3) = (Point::new(b.x0, b.y0), Point::new(x1, b.y0), Point::new(x1, y1), Point::new(b.x0, y1));
            vec![line(p0, p1), line(p1, p2), line(p2, p3), line(p3, p0)]
        }
        ModifierKind::Mirror(m) => {
            let (o, r) = (len(m.offset), mirror_reach(b));
            let vert = line(Point::new(pv.x + o, pv.y - r), Point::new(pv.x + o, pv.y + r));
            let hor = line(Point::new(pv.x - r, pv.y + o), Point::new(pv.x + r, pv.y + o));
            match m.axis {
                MirrorAxis::Vertical => vec![vert],
                MirrorAxis::Horizontal => vec![hor],
                MirrorAxis::Both => vec![vert, hor],
            }
        }
        ModifierKind::Wallpaper(p) => {
            let c = cell(p, f);
            let (o, a, bb) = (c.origin, c.origin + c.a, c.origin + c.b);
            let ab = c.origin + c.a + c.b;
            vec![line(o, a), line(a, ab), line(ab, bb), line(bb, o)]
        }
        ModifierKind::Deform(d) => deform_guides(d, f),
        ModifierKind::Displace(_) => at(HandleKind::DisplaceAmplitude).map(|e| vec![line(Point::new(pv.x, b.y0), e)]).unwrap_or_default(),
        _ => Vec::new(),
    };
    v.retain(|g| match g {
        Guide::Line { from, to } => finite(*from) && finite(*to),
        Guide::Circle { center, radius } => finite(*center) && radius.is_finite(),
        Guide::Point(p) => finite(*p),
    });
    v
}

fn deform_guides(d: &Deform, f: &Frame) -> Vec<Guide> {
    let Some(x) = deform_frame(d, f) else { return Vec::new() };
    let line = |from: Point, to: Point| Guide::Line { from, to };
    let b = x.b;
    let axis =
        if x.horizontal { line(Point::new(b.x0, x.c.y), Point::new(b.x1, x.c.y)) } else { line(Point::new(x.c.x, b.y0), Point::new(x.c.x, b.y1)) };
    let knobs: Vec<Point> = deform_spots(d, f).iter().map(|s| s.at).collect();
    match d.mode {
        DeformMode::Taper => {
            let mut v = vec![axis];
            if let [a, e] = knobs.as_slice() {
                v.push(line(*a, *e));
            }
            v
        }
        DeformMode::Twist => {
            let mut v = vec![axis];
            v.extend(knobs.first().map(|k| line(x.c, *k)));
            v
        }
        DeformMode::Bend => vec![axis],
        DeformMode::Bulge => vec![Guide::Circle { center: x.c, radius: within(d.radius, 0.01, 2.0, 1.0) * x.extent }, Guide::Point(x.c)],
        DeformMode::Shear => {
            let o = x.shear_origin(d.origin);
            let mut v = vec![Guide::Point(o)];
            v.extend(knobs.iter().map(|k| line(o, *k)));
            v
        }
    }
}

/// `v` rounded to one decimal, without a trailing `.0` (or a minus on zero).
fn one(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    let r = if r == 0.0 { 0.0 } else { r };
    if r.fract() == 0.0 { format!("{r:.0}") } else { format!("{r:.1}") }
}

fn pct(v: f64) -> String {
    format!("{}%", one(v * 100.0))
}

/// The live readout beside handle `h` of row `kind`: `Offset 120, 0`, `Radius 80`, `Sweep 270°`…
/// (lengths in points).
pub fn label(kind: &ModifierKind, h: HandleKind) -> Option<String> {
    let s = match (kind, h) {
        (ModifierKind::LinearArray(l), _) => format!("Offset {}, {}", one(len(l.offset_x)), one(len(l.offset_y))),
        (ModifierKind::CircularArray(c), HandleKind::RingCenter) => format!("Center {}, {}", one(len(c.center_x)), one(len(c.center_y))),
        (ModifierKind::CircularArray(c), HandleKind::RingRadius) => format!("Radius {}", one(within(c.radius, 0.0, MAX_LENGTH, 0.0))),
        (ModifierKind::CircularArray(c), _) => format!("Sweep {}°", one(within(c.sweep, -360.0, 360.0, 360.0))),
        (ModifierKind::GridArray(g), HandleKind::GridGapY) => format!("Gap Y {}", one(len(g.gap_y))),
        (ModifierKind::GridArray(g), _) => format!("Gap X {}", one(len(g.gap_x))),
        (ModifierKind::Mirror(m), _) => format!("Axis {}", one(len(m.offset))),
        (ModifierKind::Wallpaper(p), HandleKind::CellOrigin) => format!("Origin {}, {}", one(len(p.origin_x)), one(len(p.origin_y))),
        (ModifierKind::Wallpaper(p), HandleKind::CellA) => format!("Cell width {}", one(within(p.cell_width, 0.001, MAX_LENGTH, 100.0))),
        (ModifierKind::Wallpaper(p), _) if p.group.uses_angle() => {
            format!("Cell height {}, {}°", one(within(p.cell_height, 0.001, MAX_LENGTH, 100.0)), one(within(p.angle, 1.0, 179.0, 90.0)))
        }
        (ModifierKind::Wallpaper(p), _) => format!("Cell height {}", one(within(p.cell_height, 0.001, MAX_LENGTH, 100.0))),
        (ModifierKind::Deform(d), HandleKind::TaperStart) => format!("Start scale {}", pct(within(d.start_scale, 0.1, 3.0, 1.0))),
        (ModifierKind::Deform(d), HandleKind::TaperEnd) => format!("End scale {}", pct(within(d.end_scale, 0.1, 3.0, 1.0))),
        (ModifierKind::Deform(d), HandleKind::TwistEnd) => format!("Twist {}°", one(within(d.angle, -720.0, 720.0, 0.0))),
        (ModifierKind::Deform(d), HandleKind::BendPoint) => format!("Bend point {}", pct(within(d.bend_point, 0.0, 1.0, 0.5))),
        (ModifierKind::Deform(d), HandleKind::BulgeRadius) => format!("Bulge radius {}", pct(within(d.radius, 0.01, 2.0, 1.0))),
        (ModifierKind::Deform(d), HandleKind::ShearX) => format!("Shear X {}%", one(within(d.shear_x, -2.0, 2.0, 0.0) * 100.0)),
        (ModifierKind::Deform(d), HandleKind::ShearY) => format!("Shear Y {}%", one(within(d.shear_y, -2.0, 2.0, 0.0) * 100.0)),
        (ModifierKind::Trim(t), HandleKind::TrimStart) => format!("Start {}", pct(within(t.start, 0.0, 1.0, 0.0))),
        (ModifierKind::Trim(t), _) => format!("End {}", pct(within(t.end, 0.0, 1.0, 0.0))),
        (ModifierKind::Displace(d), _) => format!("Amplitude {}", one(within(d.amplitude, 0.0, 100.0, 0.0))),
        _ => return None,
    };
    Some(s)
}

/// Whether row `kind` can have handles at all (whatever the object): what decides which row's
/// handles show when no row is open in the panel.
pub fn has_handles(kind: &ModifierKind) -> bool {
    matches!(
        kind,
        ModifierKind::LinearArray(_)
            | ModifierKind::CircularArray(_)
            | ModifierKind::GridArray(_)
            | ModifierKind::Mirror(_)
            | ModifierKind::Wallpaper(_)
            | ModifierKind::Deform(_)
            | ModifierKind::Trim(_)
            | ModifierKind::Displace(_)
    )
}
