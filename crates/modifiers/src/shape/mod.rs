//! Geometry rows: modifiers that change the object's paths instead of placing copies
//! (`resample`, `trim`, `tidy`, `deform`, `displace`, `screen`).
//!
//! # Where they run
//! Rows run top to bottom. A geometry row above every copy row (arrays, mirror, wallpaper, copy
//! to shape, transform each, randomize) changes the object itself before it is copied
//! ([`RowAt::Source`]): every copy is a copy of the changed object. A geometry row below a copy
//! row runs on each finished copy, after its instance transform, in document space
//! ([`RowAt::Copies`]), in stack order. [`crate::evaluate`] lists them in [`crate::Output::geometry`];
//! whoever builds the copies ([`Shaper`]) runs them: the renderer's art (canvas, bounds, hit
//! testing, every export), `modifier.apply`, the stencil and the laser.
//!
//! # What they change
//! Paths, the members of compound paths and the paths inside groups (each subpath on its own); a
//! live shape (rectangle, ellipse, star…) becomes a plain path in the copy. Guides are left alone.
//! Type, images, symbol instances and live objects (blends, envelopes, meshes, repeats…) pass
//! through as they are, with a note.
//!
//! # Caps
//! The anchors the rows make for one object, all its copies together, stay within
//! [`MAX_GEOMETRY_ANCHORS`]: each copy may hold its share. A row that would make more (and more
//! than it was given) is skipped for that copy, with a note; so is one that gives a point that
//! isn't finite. Nothing here panics.

mod deform;
#[cfg(test)]
mod deform_tests;
mod displace;
#[cfg(test)]
mod displace_pattern_tests;
#[cfg(test)]
mod displace_tests;
mod fractal;
mod halftone;
#[cfg(test)]
mod halftone_tests;
pub(crate) mod handle_geom;
mod pieces;
mod resample;
#[cfg(test)]
mod tests;
mod tidy;
mod trim;

use std::sync::Arc;

use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{PathData, Point, SubPath, Vec2};

use crate::eval::Output;
use crate::kinds::Clamp;
use crate::model::{Deform, Displace, ModifierKind, Resample, Tidy, Trim};

pub use halftone::{MAX_BAND_ENTRIES, MAX_CLIP_SEGMENTS, MAX_CLIP_WORK, MAX_EDGE_CONTACTS, MAX_OUTLINE_PIECES, NEAR_WHITE, SPOT_ANCHORS, ScreenRow};

/// The most anchors the geometry rows make for one object, all its copies together.
pub const MAX_GEOMETRY_ANCHORS: usize = 200_000;

/// A geometry row's operation, its params clamped.
#[derive(Clone, Debug, PartialEq)]
pub enum GeometryOp {
    Resample(Resample),
    Trim(Trim),
    Tidy(Tidy),
    Deform(Deform),
    Displace(Displace),
    Screen(ScreenRow),
}

impl GeometryOp {
    /// The row's name, for notes.
    pub fn label(&self) -> &'static str {
        match self {
            GeometryOp::Resample(_) => "Resample",
            GeometryOp::Trim(_) => "Trim",
            GeometryOp::Tidy(_) => "Tidy",
            GeometryOp::Deform(_) => "Deform",
            GeometryOp::Displace(_) => "Displace",
            GeometryOp::Screen(_) => "Screen",
        }
    }
}

/// Where a geometry row runs (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAt {
    /// On the object, before it is copied (no copy row above it).
    Source,
    /// On every copy, after its transform (a copy row above it).
    Copies,
}

/// One enabled geometry row of an evaluated stack.
#[derive(Clone, Debug, PartialEq)]
pub struct GeometryRow {
    /// The modifier's id.
    pub modifier: String,
    pub op: GeometryOp,
    pub at: RowAt,
}

/// `kind`'s geometry operation with its params clamped to their ranges, and what the clamping
/// says; `None` for the other kinds.
pub(crate) fn clamped(kind: &ModifierKind) -> Option<(GeometryOp, Vec<String>)> {
    let mut warnings = Vec::new();
    let mut c = Clamp { warnings: &mut warnings };
    let op = match kind {
        ModifierKind::Resample(p) => {
            let mut p = p.clone();
            p.iterations = whole(&mut c, "iterations", p.iterations, 1, 5);
            p.count = whole(&mut c, "count", p.count, 2, 1024);
            p.spacing = c.within("spacing", p.spacing, 1.0, 5000.0, 24.0);
            GeometryOp::Resample(p)
        }
        ModifierKind::Trim(p) => GeometryOp::Trim(Trim {
            start: c.within("start", p.start, 0.0, 1.0, 0.0),
            end: c.within("end", p.end, 0.0, 1.0, 1.0),
            offset: c.within("offset", p.offset, 0.0, 1.0, 0.0),
        }),
        ModifierKind::Tidy(p) => {
            let mut p = p.clone();
            p.tolerance = c.within("tolerance", p.tolerance, 0.0, 500.0, 1.0);
            p.seed = whole(&mut c, "seed", p.seed, 0, 999_999);
            GeometryOp::Tidy(p)
        }
        ModifierKind::Deform(p) => GeometryOp::Deform(Deform {
            start_scale: c.within("startScale", p.start_scale, 0.1, 3.0, 1.0),
            end_scale: c.within("endScale", p.end_scale, 0.1, 3.0, 0.2),
            angle: c.within("angle", p.angle, -720.0, 720.0, 90.0),
            bend_angle: c.within("bendAngle", p.bend_angle, -360.0, 360.0, 45.0),
            bend_point: c.within("bendPoint", p.bend_point, 0.0, 1.0, 0.5),
            shear_x: c.within("shearX", p.shear_x, -2.0, 2.0, 0.3),
            shear_y: c.within("shearY", p.shear_y, -2.0, 2.0, 0.0),
            strength: c.within("strength", p.strength, -3.0, 3.0, 0.35),
            radius: c.within("radius", p.radius, 0.01, 2.0, 1.0),
            progressive_amount: c.within("progressiveAmount", p.progressive_amount, 0.0, 1.0, 1.0),
            ..p.clone()
        }),
        ModifierKind::Displace(p) => {
            let mut p = p.clone();
            // Subdivision Growth's displacement reaches 200; the other patterns' amplitude 100.
            let most = if p.mode == crate::model::DisplaceMode::Fractal { 200.0 } else { 100.0 };
            p.amplitude = c.within("amplitude", p.amplitude, 0.0, most, 10.0);
            p.angle = c.within("angle", p.angle, -360.0, 360.0, 0.0);
            p.scale = c.within("scale", p.scale, 0.01, 1.0, 0.05);
            p.octaves = whole(&mut c, "octaves", p.octaves, 1, 6);
            p.ripple_count = whole(&mut c, "rippleCount", p.ripple_count, 2, 12);
            p.frequency = c.within("frequency", p.frequency, 0.1, 10.0, 2.0);
            p.phase = c.within("phase", p.phase, 0.0, 360.0, 0.0);
            p.jitter_min = c.within("jitterMin", p.jitter_min, -1.0, 1.0, 0.0);
            p.steps = whole(&mut c, "steps", p.steps, 1, fractal::MAX_STEPS);
            p.decay = c.within("decay", p.decay, 0.0, 0.5, 0.45);
            p.wander = c.within("wander", p.wander, 0.0, 1.0, 0.15);
            p.edge_variance = c.within("edgeVariance", p.edge_variance, 0.0, 1.0, 0.0);
            p.normal_bias = c.within("normalBias", p.normal_bias, 0.0, 1.0, 0.5);
            p.seed = whole(&mut c, "seed", p.seed, 0, 999_999);
            p.progressive_amount = c.within("progressiveAmount", p.progressive_amount, 0.0, 1.0, 1.0);
            GeometryOp::Displace(p)
        }
        ModifierKind::Screen(p) => {
            let mut p = p.clone();
            p.lpi = c.within("lpi", p.lpi, crate::screen::MIN_LPI, crate::screen::MAX_LPI, crate::screen::DEFAULT_LPI);
            p.angle = c.within("angle", p.angle, -360.0, 360.0, 45.0);
            p.quality = whole(&mut c, "quality", p.quality, crate::screen::MIN_QUALITY, crate::screen::MAX_QUALITY);
            p.amount = c.within("amount", p.amount, 0.0, 100.0, 50.0);
            // The image is resolved by the evaluator, which has the document.
            GeometryOp::Screen(ScreenRow { params: p, image: None })
        }
        _ => return None,
    };
    Some((op, warnings))
}

fn whole(c: &mut Clamp<'_>, name: &str, v: u32, lo: u32, hi: u32) -> u32 {
    if v < lo || v > hi {
        c.warnings.push(format!("`{name}` is limited to {lo}..{hi}"));
    }
    v.clamp(lo, hi)
}

/// Runs an evaluation's geometry rows on the object and its copies, and keeps what it has to say.
///
/// ```ignore
/// let mut shaper = Shaper::new(&out);
/// shaper.source(&mut object);          // before copying
/// for (k, i) in out.instances.iter().enumerate(): copy.transform(i.transform); shaper.copy_at(&mut copy, k);
/// ```
pub struct Shaper<'a> {
    rows: &'a [GeometryRow],
    /// The evaluation: its copies, for a row that varies by copy (Deform's ramp and frame).
    out: &'a Output,
    /// The anchors one copy may hold after a row that adds anchors.
    budget: usize,
    /// What went wrong, each said once.
    pub notes: Vec<String>,
}

impl<'a> Shaper<'a> {
    /// A shaper for `out`'s rows, the budget shared by its copies.
    pub fn new(out: &'a Output) -> Shaper<'a> {
        let copies = out.instances.len().max(1);
        Shaper { rows: &out.geometry, out, budget: MAX_GEOMETRY_ANCHORS / copies, notes: Vec::new() }
    }

    /// Whether there is anything to run.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Run the rows above the copy rows on `n`, the object before it is copied.
    pub fn source(&mut self, n: &mut Node) {
        self.run(n, RowAt::Source, None);
    }

    /// Run the rows below a copy row on `n`, one copy after its transform, as if it were the only
    /// copy (no ramp, deformed in document axes); [`Shaper::copy_at`] knows which copy it is.
    pub fn copy(&mut self, n: &mut Node) {
        self.run(n, RowAt::Copies, None);
    }

    /// Run the rows below a copy row on `n`, copy `index` of [`Output::instances`] after its
    /// transform: a Deform ramps across the array copies ([`Output::ramp_position`]) and bends each
    /// copy in its own frame. `index` is the copy's place in `instances`, also when callers skip
    /// some (a transform that isn't finite).
    pub fn copy_at(&mut self, n: &mut Node, index: usize) {
        self.run(n, RowAt::Copies, Some(index));
    }

    fn run(&mut self, n: &mut Node, at: RowAt, copy: Option<usize>) {
        let rows = self.rows;
        for row in rows.iter().filter(|r| r.at == at) {
            match &row.op {
                GeometryOp::Deform(p) => self.deform(n, p, copy),
                GeometryOp::Screen(p) => self.screen(n, p),
                _ => self.row(n, row, copy),
            }
        }
    }

    /// A Deform row on `n`: copy `copy` (ramped, in its frame) or the object. It fits its pieces
    /// in the budget itself, so it is never skipped for anchors.
    fn deform(&mut self, n: &mut Node, p: &Deform, copy: Option<usize>) {
        let instance = copy.and_then(|i| self.out.instances.get(i));
        let p = deform::scaled(p, deform::ramp(p, instance.and_then(|i| self.out.ramp_position(i))));
        let frame = instance.map_or(vectorcraft_geom::Affine::IDENTITY, |i| deform::frame_of(i.transform));
        let Some((b, extent)) = deform::measure(n, &p, frame) else { return };
        let Some(m) = deform::map(&p, b, extent) else { return };
        let room = deform::Room { room: self.budget.saturating_sub(anchors(n, 0)), segments: deform::segments(n, 0) };
        let mut changed = n.clone();
        let mut skipped = false;
        deform::apply_node(&mut changed, &m, frame, &room, 0, &mut skipped);
        if skipped {
            self.note("Deform changes paths only: type, images, symbols and live objects are drawn as they are".into());
        }
        if !finite(&changed, 0) {
            self.note("Deform gave points that aren't finite numbers; it is skipped".into());
            return;
        }
        *n = changed;
    }

    /// A Screen row on `n` (the object, or a copy where it sits: the screen is fixed to the page).
    /// It checks the anchors against the budget itself, path by path.
    fn screen(&mut self, n: &mut Node, p: &ScreenRow) {
        let mut changed = n.clone();
        let mut run = halftone::Run { room: self.budget, notes: &mut self.notes, skipped: false };
        if !halftone::screen_node(&mut changed, p, &mut run) {
            return;
        }
        if run.skipped {
            self.note("Screen changes paths only: type, images, symbols and live objects are drawn as they are".into());
        }
        if !finite(&changed, 0) {
            self.note("Screen gave points that aren't finite numbers; it is skipped".into());
            return;
        }
        *n = changed;
    }

    fn note(&mut self, s: String) {
        if !self.notes.contains(&s) {
            self.notes.push(s);
        }
    }

    /// One row on `n`, or none of it (over the budget, or a point that isn't finite).
    fn row(&mut self, n: &mut Node, row: &GeometryRow, copy: Option<usize>) {
        let label = row.op.label();
        let (before, after) = (anchors(n, 0), projected(n, &row.op, 0));
        if after > self.budget && after > before {
            let total = MAX_GEOMETRY_ANCHORS;
            self.note(format!("{label} would make more than {total} anchors for this object and its copies; it is skipped"));
            return;
        }
        // A displace ramps and seeds its radial phase by the copy's place among the array
        // copies (`Output::ramp_position`): a mirror pair shares it; no array position, the whole
        // effect and the phase of position 0.
        let instance = copy.and_then(|i| self.out.instances.get(i));
        let position = instance.and_then(|i| self.out.ramp_position(i));
        let frame = instance.map_or(displace::Frame::IDENTITY, |i| displace::Frame::of(i.transform));
        let field = field(n, &row.op, position, frame);
        let mut changed = n.clone();
        let mut skipped = false;
        apply(&mut changed, &row.op, field.as_ref(), 0, &mut skipped);
        if skipped {
            self.note(format!("{label} changes paths only: type, images, symbols and live objects are drawn as they are"));
        }
        if !finite(&changed, 0) {
            self.note(format!("{label} gave points that aren't finite numbers; it is skipped"));
            return;
        }
        *n = changed;
    }
}

/// The anchors of the paths in `n`.
fn anchors(n: &Node, depth: usize) -> usize {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return 0;
    }
    match &n.kind {
        NodeKind::Path { path, .. } => path.anchor_count(),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            children.iter().fold(0usize, |a, c| a.saturating_add(anchors(c, depth + 1)))
        }
        _ => 0,
    }
}

/// How many anchors the paths in `n` hold after `op` (a trim and a tidy: as many as before),
/// found without changing them.
fn projected(n: &Node, op: &GeometryOp, depth: usize) -> usize {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return 0;
    }
    match &n.kind {
        NodeKind::Path { guide: true, path, .. } => path.anchor_count(),
        NodeKind::Path { path, .. } => path
            .subpaths
            .iter()
            .map(|sp| match op {
                GeometryOp::Resample(p) => resample::count(sp, p).unwrap_or(sp.anchors.len()),
                // A trim makes at most two more anchors than it was given (an anchor at each cut;
                // through a closed path's start, its first anchor once more) and a tidy none:
                // counted as what they were given, so they are never refused.
                GeometryOp::Trim(_) | GeometryOp::Tidy(_) => sp.anchors.len(),
                // A deform fits its pieces in the room it is given (`Shaper::deform`).
                GeometryOp::Deform(_) => sp.anchors.len(),
                // A fractal displace splits every segment `steps` times; the others move anchors,
                // never add them.
                GeometryOp::Displace(p) if p.mode == crate::model::DisplaceMode::Fractal => fractal::count(sp, p.steps),
                GeometryOp::Displace(_) => sp.anchors.len(),
                // A screen checks its dots against the budget itself (`Shaper::screen`).
                GeometryOp::Screen(_) => sp.anchors.len(),
            })
            .fold(0usize, usize::saturating_add),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            children.iter().fold(0usize, |a, c| a.saturating_add(projected(c, op, depth + 1)))
        }
        _ => 0,
    }
}

/// `op` on every path in `n`; `skipped` is set when something it can't change is passed over.
fn apply(n: &mut Node, op: &GeometryOp, field: Option<&displace::Field>, depth: usize, skipped: &mut bool) {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return;
    }
    match &mut n.kind {
        // Ruler guides draw on screen only: passed over silently, as path guides are.
        NodeKind::Path { guide: true, .. } | NodeKind::RulerGuide { .. } => {}
        NodeKind::Path { path, live, .. } => {
            if let Some(new) = shaped(path, op, field) {
                *path = new;
                // The copy is a plain path now: a live shape would redraw its old outline.
                *live = None;
            }
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            for c in children.iter_mut() {
                apply(Arc::make_mut(c), op, field, depth + 1, skipped);
            }
        }
        _ => *skipped = true,
    }
}

/// A displace's field for `n` (the radial and jitter centre is the mean of `n`'s anchors, a wave
/// runs across their extent) at ramp position `copy` (position, total); `None` for the other rows.
fn field(n: &Node, op: &GeometryOp, copy: Option<(usize, usize)>, frame: displace::Frame) -> Option<displace::Field> {
    field_from(Object { sum: || anchor_sum(n, 0), span: |along| anchor_span(n, along, 0) }, op, copy, frame)
}

/// What a displace reads from the whole object, each only when its pattern needs it: the anchors'
/// sum and count, and their least and greatest projection on a direction.
struct Object<S, P> {
    sum: S,
    span: P,
}

/// The field for `op` (a displace).
fn field_from<S, P>(object: Object<S, P>, op: &GeometryOp, copy: Option<(usize, usize)>, frame: displace::Frame) -> Option<displace::Field>
where
    S: FnOnce() -> (Vec2, usize),
    P: FnOnce(Vec2) -> (f64, f64),
{
    let GeometryOp::Displace(p) = op else { return None };
    let centre = if displace::needs_centre(p) {
        match (object.sum)() {
            (s, count) if count > 0 => (s / count as f64).to_point(),
            _ => Point::ZERO,
        }
    } else {
        Point::ZERO
    };
    let field = displace::Field::new(p, centre, copy, frame);
    Some(match displace::wave_along(p, frame) {
        Some(along) => {
            let (lo, hi) = (object.span)(along);
            field.spanning(lo, hi)
        }
        None => field,
    })
}

/// The least and greatest projection on `along` of the finite anchor points of the plain paths in
/// `n` (`(∞, -∞)` when there are none).
fn anchor_span(n: &Node, along: Vec2, depth: usize) -> (f64, f64) {
    let none = (f64::INFINITY, f64::NEG_INFINITY);
    if depth > vectorcraft_doc::MAX_DEPTH {
        return none;
    }
    match &n.kind {
        NodeKind::Path { guide: true, .. } => none,
        NodeKind::Path { path, .. } => path_span(path, along),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            children.iter().map(|c| anchor_span(c, along, depth + 1)).fold(none, |(lo, hi), (l, h)| (lo.min(l), hi.max(h)))
        }
        _ => none,
    }
}

fn path_span(path: &PathData, along: Vec2) -> (f64, f64) {
    path.subpaths
        .iter()
        .flat_map(|s| &s.anchors)
        .filter(|a| a.p.is_finite())
        .map(|a| along.dot(a.p.to_vec2()))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), s| (lo.min(s), hi.max(s)))
}

/// The sum of the anchor points of the plain paths in `n`, and how many.
fn anchor_sum(n: &Node, depth: usize) -> (Vec2, usize) {
    let zero = (Vec2::ZERO, 0);
    if depth > vectorcraft_doc::MAX_DEPTH {
        return zero;
    }
    match &n.kind {
        NodeKind::Path { guide: true, .. } => zero,
        NodeKind::Path { path, .. } => path_sum(path),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            children.iter().map(|c| anchor_sum(c, depth + 1)).fold(zero, |(s, k), (s2, k2)| (s + s2, k.saturating_add(k2)))
        }
        _ => zero,
    }
}

fn path_sum(path: &PathData) -> (Vec2, usize) {
    path.subpaths.iter().flat_map(|s| &s.anchors).filter(|a| a.p.is_finite()).fold((Vec2::ZERO, 0), |(s, k), a| (s + a.p.to_vec2(), k + 1))
}

/// `path` after `op`, `None` when nothing changes.
fn shaped(path: &PathData, op: &GeometryOp, field: Option<&displace::Field>) -> Option<PathData> {
    // A deform reads the whole path's box, not each subpath's.
    if let GeometryOp::Deform(p) = op {
        return deform::deform_path(path, p);
    }
    let mut changed = false;
    let mut subpaths: Vec<SubPath> = Vec::with_capacity(path.subpaths.len());
    for sp in &path.subpaths {
        let new = match op {
            GeometryOp::Resample(p) => resample::resample(sp, p),
            GeometryOp::Tidy(p) => tidy::tidy(sp, p),
            GeometryOp::Deform(_) | GeometryOp::Screen(_) => None,
            GeometryOp::Displace(_) => field.and_then(|f| f.displace(sp)),
            GeometryOp::Trim(p) => match trim::trim(sp, p) {
                trim::Trimmed::Whole => None,
                trim::Trimmed::Part(t) => Some(t),
                trim::Trimmed::Gone => {
                    changed = true;
                    continue;
                }
            },
        };
        match new {
            Some(new) => {
                changed = true;
                subpaths.push(new);
            }
            None => subpaths.push(sp.clone()),
        }
    }
    changed.then(|| PathData::new(subpaths))
}

/// Whether every anchor and handle in `n` is finite.
fn finite(n: &Node, depth: usize) -> bool {
    if depth > vectorcraft_doc::MAX_DEPTH {
        return true;
    }
    match &n.kind {
        NodeKind::Path { path, .. } => {
            path.subpaths.iter().flat_map(|s| &s.anchors).all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite())
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } | NodeKind::Compound { children, .. } => {
            children.iter().all(|c| finite(c, depth + 1))
        }
        _ => true,
    }
}

/// `n`'s copies for `out`, reshaped by its geometry rows, without its stack (each copy is `n`
/// under one instance transform, stroke weights scaling with it), and the shaper's notes: for
/// whoever reads the copies' geometry rather than drawing them (the stencil, the laser). The
/// renderer builds its art the same way, with effects scaling too.
pub fn shaped_copies(n: &Node, out: &Output) -> (Vec<Node>, Vec<String>) {
    let mut object = n.clone();
    crate::set_node_source(&mut object, None);
    let mut shaper = Shaper::new(out);
    shaper.source(&mut object);
    let copies = out
        .instances
        .iter()
        .enumerate()
        .filter(|(_, i)| i.transform.is_finite())
        .map(|(k, i)| {
            let mut c = object.clone();
            if i.transform != vectorcraft_geom::Affine::IDENTITY {
                c.transform(i.transform, true);
            }
            shaper.copy_at(&mut c, k);
            c
        })
        .collect();
    (copies, shaper.notes)
}

/// `path` after `op` (as it is when `op` leaves it alone): bare geometry, without the caps (a
/// displace as on the object: its radial centre the mean of `path`'s anchors, no copy ramp). A
/// screen needs the path's paint and returns `path` as it is: it runs on nodes ([`Shaper`]).
pub fn shape_path(path: &PathData, op: &GeometryOp) -> PathData {
    let field = field_from(Object { sum: || path_sum(path), span: |along| path_span(path, along) }, op, None, displace::Frame::IDENTITY);
    shaped(path, op, field.as_ref()).unwrap_or_else(|| path.clone())
}
